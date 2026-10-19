//! The local link between `pwrroute mcp` and the running app: a per-user
//! Unix socket on macOS and Linux, a named pipe on Windows.
//!
//! The link carries MCP itself, newline-delimited JSON-RPC, so the proxy is
//! a byte pipe and the app runs the one MCP server. Only this user can reach
//! it: the socket is `0600` and every connection's peer uid is checked; the
//! pipe refuses remote clients, and the default pipe ACL leaves other local
//! users read access only, which cannot send a request.

use std::{
    io,
    path::{Path, PathBuf},
};

use tokio::io::{AsyncRead, AsyncWrite};

/// Names the link, for tests and for a second PwrRoute that should not meet
/// the first.
pub const ENDPOINT_ENV: &str = "PWRROUTE_MCP_ENDPOINT";

/// One end of a link connection.
pub trait LinkStream: AsyncRead + AsyncWrite + Unpin + Send + 'static {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> LinkStream for T {}

pub type Stream = Box<dyn LinkStream>;

/// Where the app listens: a socket path on Unix, a pipe name on Windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint(PathBuf);

impl Endpoint {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// `$PWRROUTE_MCP_ENDPOINT` if set, else this user's default.
    #[must_use]
    pub fn for_user() -> Self {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
        Self::choose(
            var(ENDPOINT_ENV).map(PathBuf::from),
            pwrroute_os::router_dir(),
            var(if cfg!(windows) { "USERNAME" } else { "USER" })
                .map(|u| u.to_string_lossy().into_owned()),
        )
    }

    /// [`Self::for_user`] over given values, so the choice is testable. On
    /// Unix the socket sits beside the router's, in a directory that is the
    /// user's own; only without one does it go in the shared `/tmp`, named
    /// for the user, where the peer check still turns others away.
    #[must_use]
    pub fn choose(
        explicit: Option<PathBuf>,
        router_dir: Option<PathBuf>,
        user: Option<String>,
    ) -> Self {
        if let Some(path) = explicit {
            return Self(path);
        }
        let user = user
            .map(|u| sanitize(&u))
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| "user".to_owned());
        if cfg!(windows) {
            return Self(PathBuf::from(format!(r"\\.\pipe\pwrroute-mcp-{user}")));
        }
        match router_dir {
            Some(dir) => Self(dir.join("mcp.sock")),
            None => Self(PathBuf::from(format!("/tmp/pwrroute-mcp-{user}.sock"))),
        }
    }
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.display().fmt(f)
    }
}

fn sanitize(user: &str) -> String {
    user.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect()
}

/// Connects to the app's end of the link.
pub async fn connect(endpoint: &Endpoint) -> io::Result<Stream> {
    imp::connect(endpoint.path()).await
}

pub(crate) use imp::Listener;

#[cfg(unix)]
mod imp {
    use std::{
        fs, io,
        os::unix::fs::{MetadataExt as _, PermissionsExt as _},
        path::{Path, PathBuf},
    };

    use tokio::net::{UnixListener, UnixStream};

    use super::Stream;

    /// The app's end: owns the socket file and removes it on drop.
    #[derive(Debug)]
    pub(crate) struct Listener {
        inner: UnixListener,
        path: PathBuf,
        uid: u32,
    }

    impl Listener {
        /// Listens at `path`, replacing a socket file nothing answers on
        /// (a PwrRoute that did not exit cleanly). One something answers on
        /// belongs to a running PwrRoute and is left alone.
        pub(crate) fn bind(path: &Path) -> io::Result<Self> {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let inner = match UnixListener::bind(path) {
                Ok(listener) => listener,
                Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
                    if std::os::unix::net::UnixStream::connect(path).is_ok() {
                        return Err(io::Error::new(
                            io::ErrorKind::AddrInUse,
                            "another PwrRoute is already serving local agents",
                        ));
                    }
                    fs::remove_file(path)?;
                    UnixListener::bind(path)?
                }
                Err(e) => return Err(e),
            };
            // The runtime directory is the user's already; `/tmp` is not.
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            let uid = fs::metadata(path)?.uid();
            Ok(Self {
                inner,
                path: path.to_path_buf(),
                uid,
            })
        }

        /// The next connection from this user. Anyone else is hung up on.
        pub(crate) async fn accept(&mut self) -> io::Result<Stream> {
            loop {
                let (stream, _) = self.inner.accept().await?;
                if stream.peer_cred().is_ok_and(|c| c.uid() == self.uid) {
                    return Ok(Box::new(stream));
                }
            }
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    pub(super) async fn connect(path: &Path) -> io::Result<Stream> {
        Ok(Box::new(UnixStream::connect(path).await?))
    }
}

#[cfg(windows)]
mod imp {
    use std::{
        io,
        path::{Path, PathBuf},
        time::Duration,
    };

    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};

    use super::Stream;

    /// `ERROR_PIPE_BUSY`: every instance is taken; wait for the next.
    const PIPE_BUSY: i32 = 231;
    /// `ERROR_ACCESS_DENIED`: `first_pipe_instance` found the name taken.
    const ACCESS_DENIED: i32 = 5;

    /// The app's end. A named pipe serves one client per instance, so the
    /// next instance is created before a connected one is handed out.
    #[derive(Debug)]
    pub(crate) struct Listener {
        name: PathBuf,
        next: NamedPipeServer,
    }

    impl Listener {
        /// Creates the first instance. If the name is taken, another
        /// PwrRoute (or a squatter) holds it, and this one does not listen.
        pub(crate) fn bind(name: &Path) -> io::Result<Self> {
            let next = ServerOptions::new()
                .first_pipe_instance(true)
                .reject_remote_clients(true)
                .create(name)
                .map_err(|e| {
                    if e.raw_os_error() == Some(ACCESS_DENIED) {
                        io::Error::new(
                            io::ErrorKind::AddrInUse,
                            "another PwrRoute is already serving local agents",
                        )
                    } else {
                        e
                    }
                })?;
            Ok(Self {
                name: name.to_path_buf(),
                next,
            })
        }

        pub(crate) async fn accept(&mut self) -> io::Result<Stream> {
            if let Err(e) = self.next.connect().await {
                // A client that left before the connect (ERROR_NO_DATA)
                // leaves the instance unusable until it is disconnected.
                let _ = self.next.disconnect();
                return Err(e);
            }
            let following = ServerOptions::new()
                .reject_remote_clients(true)
                .create(&self.name)?;
            let connected = std::mem::replace(&mut self.next, following);
            Ok(Box::new(connected))
        }
    }

    pub(super) async fn connect(name: &Path) -> io::Result<Stream> {
        let mut attempts = 0;
        loop {
            match ClientOptions::new().open(name) {
                Ok(client) => return Ok(Box::new(client)),
                Err(e) if e.raw_os_error() == Some(PIPE_BUSY) && attempts < 100 => {
                    attempts += 1;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_endpoint_wins() {
        let e = Endpoint::choose(Some("/x/y.sock".into()), Some("/run".into()), None);
        assert_eq!(e.path(), Path::new("/x/y.sock"));
    }

    #[cfg(unix)]
    #[test]
    fn unix_sits_beside_the_router_socket_else_a_named_tmp_socket() {
        let e = Endpoint::choose(None, Some("/run/user/501/pwrroute".into()), None);
        assert_eq!(e.path(), Path::new("/run/user/501/pwrroute/mcp.sock"));
        let e = Endpoint::choose(None, None, Some("ann/../x".into()));
        assert_eq!(e.path(), Path::new("/tmp/pwrroute-mcp-ann..x.sock"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_names_a_pipe_per_user() {
        let e = Endpoint::choose(None, None, Some("Ann Lee".into()));
        assert_eq!(e.path(), Path::new(r"\\.\pipe\pwrroute-mcp-AnnLee"));
    }
}
