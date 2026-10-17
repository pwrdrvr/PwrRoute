//! One router per user. The first launch takes an exclusive lock on a file
//! in the router's directory and listens on a local socket (a Unix domain
//! socket beside the lock, or a Windows named pipe named after that
//! directory); every later launch finds the lock taken, forwards its links
//! to that socket, and exits.
//!
//! The lock, not the socket, decides who is primary: the OS releases it when
//! the holder exits, however it exits, so a crashed router never blocks the
//! next launch, and a stale Unix socket file can be replaced safely.

use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::{self, BufReader},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use interprocess::local_socket::{ConnectOptions, ListenerOptions, Name, Stream, prelude::*};

use crate::{
    ipc::{self, Request, Response},
    link::Link,
};

/// How long a connection may take to send its request or read the answer.
const IO_TIMEOUT: Duration = Duration::from_secs(2);

/// Where the router's lock and socket live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    dir: PathBuf,
}

impl Endpoint {
    /// An endpoint in `dir`, which is created private to the user. Keep the
    /// path short: a Unix socket path is limited to about 100 bytes.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn lock_path(&self) -> PathBuf {
        self.dir.join("router.lock")
    }

    /// The Unix socket's path. On Windows the endpoint is a named pipe, and
    /// this file is never created.
    pub fn socket_path(&self) -> PathBuf {
        self.dir.join("router.sock")
    }

    fn name(&self) -> io::Result<Name<'static>> {
        #[cfg(windows)]
        {
            use interprocess::local_socket::GenericNamespaced;
            // Named pipes share one machine-wide namespace. The directory is
            // per user, so a name derived from it is too.
            let dir = self.dir.to_string_lossy().to_lowercase();
            format!("pwrroute-router-{:016x}", fnv1a(dir.as_bytes()))
                .to_ns_name::<GenericNamespaced>()
        }
        #[cfg(not(windows))]
        {
            use interprocess::local_socket::GenericFilePath;
            self.socket_path().to_fs_name::<GenericFilePath>()
        }
    }
}

/// Stable across builds, unlike `DefaultHasher`: an updated binary must find
/// the pipe an older router made.
#[cfg_attr(not(windows), allow(dead_code))]
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, b| {
        (hash ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The outcome of [`claim`].
#[derive(Debug)]
pub enum Claim {
    /// This process is the router. Call [`Primary::serve`] to start listening.
    Primary(Primary),
    /// A router is already running; send it the links.
    Running(Client),
}

/// Takes the router lock if no other process holds it.
pub fn claim(endpoint: &Endpoint) -> io::Result<Claim> {
    create_private_dir(&endpoint.dir)?;
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(endpoint.lock_path())?;
    match lock.try_lock() {
        Ok(()) => Ok(Claim::Primary(Primary {
            endpoint: endpoint.clone(),
            lock,
        })),
        Err(TryLockError::WouldBlock) => Ok(Claim::Running(Client {
            endpoint: endpoint.clone(),
        })),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

fn create_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// The router lock, held. The listener is not open yet.
#[derive(Debug)]
pub struct Primary {
    endpoint: Endpoint,
    lock: File,
}

impl Primary {
    /// Listens on the endpoint and answers each request with `handler`, on a
    /// thread per connection. Returns once the socket accepts connections.
    pub fn serve(
        self,
        handler: impl Fn(Request) -> Response + Send + Sync + 'static,
    ) -> io::Result<Server> {
        let listener = ListenerOptions::new()
            .name(self.endpoint.name()?)
            // Holding the lock means any socket file left there is stale.
            .try_overwrite(true)
            .create_sync()?;
        let stop = Arc::new(AtomicBool::new(false));
        let handler = Arc::new(handler);
        let flag = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("pwrroute-ipc".into())
            .spawn(move || {
                for conn in listener.incoming() {
                    if flag.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(conn) = conn else {
                        // A failed accept is one client's problem. Back off
                        // briefly so a persistent error can't spin a core.
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    };
                    let handler = Arc::clone(&handler);
                    let _ = thread::Builder::new()
                        .name("pwrroute-ipc-conn".into())
                        .spawn(move || answer(conn, &*handler));
                }
            })?;
        Ok(Server {
            endpoint: self.endpoint,
            stop,
            thread: Some(thread),
            _lock: self.lock,
        })
    }
}

fn answer(conn: Stream, handler: &dyn Fn(Request) -> Response) {
    let _ = conn.set_recv_timeout(Some(IO_TIMEOUT));
    let _ = conn.set_send_timeout(Some(IO_TIMEOUT));
    let mut conn = BufReader::new(conn);
    let response = match ipc::read_message::<Request>(&mut conn) {
        Ok(request) => handler(request),
        Err(e) => Response::error(e.to_string()),
    };
    let _ = ipc::write_message(conn.get_mut(), &response);
}

/// The running IPC server. Dropping it stops listening, removes the socket
/// and releases the lock.
#[derive(Debug)]
pub struct Server {
    endpoint: Endpoint,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    _lock: File,
}

impl Server {
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Wake the blocking accept so the thread sees the flag. If the wake
        // can't connect, the accept may never return: leave the thread to the
        // process exit rather than hang here.
        let woke = self.endpoint.name().and_then(Stream::connect).is_ok();
        if let Some(thread) = self.thread.take()
            && woke
        {
            let _ = thread.join();
        }
    }
}

/// Talks to the running router.
#[derive(Debug, Clone)]
pub struct Client {
    endpoint: Endpoint,
}

impl Client {
    pub fn new(endpoint: Endpoint) -> Self {
        Self { endpoint }
    }

    /// Sends one request and reads the answer, retrying the connection until
    /// `timeout` passes: the router holds the lock a moment before it
    /// listens.
    pub fn send(&self, request: &Request, timeout: Duration) -> io::Result<Response> {
        let deadline = Instant::now() + timeout;
        let conn = loop {
            match ConnectOptions::new()
                .name(self.endpoint.name()?)
                .connect_sync()
            {
                Ok(conn) => break conn,
                Err(e) if Instant::now() >= deadline => return Err(e),
                Err(_) => thread::sleep(Duration::from_millis(25)),
            }
        };
        conn.set_recv_timeout(Some(IO_TIMEOUT))?;
        conn.set_send_timeout(Some(IO_TIMEOUT))?;
        let mut conn = BufReader::new(conn);
        ipc::write_message(conn.get_mut(), request)?;
        ipc::read_message(&mut conn)
    }

    /// Forwards links to the router. Fails unless the router accepted them.
    pub fn open(&self, links: Vec<Link>, timeout: Duration) -> io::Result<Response> {
        let response = self.send(&Request::Open { links }, timeout)?;
        if response.ok {
            Ok(response)
        } else {
            Err(io::Error::other(
                response
                    .error
                    .unwrap_or_else(|| "the router refused the links".into()),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::link::Source;

    fn endpoint() -> (tempfile::TempDir, Endpoint) {
        let dir = tempfile::Builder::new()
            .prefix("pr")
            .tempdir()
            .expect("temp dir");
        let endpoint = Endpoint::new(dir.path().join("r"));
        (dir, endpoint)
    }

    #[test]
    fn the_first_claim_is_primary_and_the_second_is_not() {
        let (_dir, endpoint) = endpoint();
        let Claim::Primary(primary) = claim(&endpoint).unwrap() else {
            panic!("first claim should be primary");
        };
        assert!(matches!(claim(&endpoint).unwrap(), Claim::Running(_)));
        drop(primary);
        assert!(matches!(claim(&endpoint).unwrap(), Claim::Primary(_)));
    }

    #[test]
    fn a_second_launch_forwards_links_to_the_primary() {
        let (_dir, endpoint) = endpoint();
        let Claim::Primary(primary) = claim(&endpoint).unwrap() else {
            panic!("first claim should be primary");
        };
        let (tx, rx) = mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        let server = primary
            .serve(move |request| {
                tx.lock().unwrap().send(request).unwrap();
                Response::ok()
            })
            .unwrap();

        let Claim::Running(client) = claim(&endpoint).unwrap() else {
            panic!("second claim should find the router");
        };
        let links = vec![
            Link::new("https://example.com/one", Source::CommandLine),
            Link::new("https://example.com/two", Source::CommandLine),
        ];
        let response = client.open(links.clone(), Duration::from_secs(2)).unwrap();
        assert!(response.ok);
        assert_eq!(response.pid, std::process::id());
        assert_eq!(rx.recv().unwrap(), Request::Open { links });

        assert!(
            client
                .send(&Request::Ping, Duration::from_secs(2))
                .unwrap()
                .ok
        );
        assert_eq!(rx.recv().unwrap(), Request::Ping);

        drop(server);
        // The lock is free again, and nobody answers.
        assert!(
            Client::new(endpoint.clone())
                .send(&Request::Ping, Duration::from_millis(100))
                .is_err()
        );
        assert!(matches!(claim(&endpoint).unwrap(), Claim::Primary(_)));
    }

    #[test]
    fn a_refused_open_is_an_error() {
        let (_dir, endpoint) = endpoint();
        let Claim::Primary(primary) = claim(&endpoint).unwrap() else {
            panic!("first claim should be primary");
        };
        let _server = primary.serve(|_| Response::error("busy")).unwrap();
        let err = Client::new(endpoint)
            .open(Vec::new(), Duration::from_secs(2))
            .unwrap_err();
        assert_eq!(err.to_string(), "busy");
    }

    #[cfg(unix)]
    #[test]
    fn a_stale_socket_file_is_replaced() {
        let (_dir, endpoint) = endpoint();
        create_private_dir(endpoint.dir()).unwrap();
        fs::write(endpoint.socket_path(), b"left over from a crash").unwrap();
        let Claim::Primary(primary) = claim(&endpoint).unwrap() else {
            panic!("first claim should be primary");
        };
        let _server = primary.serve(|_| Response::ok()).unwrap();
        assert!(
            Client::new(endpoint)
                .send(&Request::Ping, Duration::from_secs(2))
                .unwrap()
                .ok
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_directory_is_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let (_dir, endpoint) = endpoint();
        let _claim = claim(&endpoint).unwrap();
        let mode = fs::metadata(endpoint.dir()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    fn pipe_names_are_stable() {
        // FNV-1a test vectors: an updated build must derive the same name.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
