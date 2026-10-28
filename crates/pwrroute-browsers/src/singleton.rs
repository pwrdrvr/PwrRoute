//! Handing a command line to a running Chromium browser over its process
//! singleton socket, the channel its own second instance uses
//! (`chrome/browser/process_singleton_posix.cc`).
//!
//! A running Chromium browser keeps three symlinks in its user data dir:
//! `SingletonLock`, `SingletonSocket`, pointing at a Unix socket in a
//! directory of its own under the user's temp dir, and `SingletonCookie`,
//! whose target is a random number. The socket's directory holds a
//! `SingletonCookie` link with the same number. A client checks the two
//! cookies match, connects, checks them again (POSIX has no `connectat`),
//! writes `START` NUL `<cwd>` NUL `<argv0>` NUL `<argv1>`… (at most 32 KiB),
//! shuts down its write side, and reads `ACK` (handled) or `SHUTDOWN` (the
//! browser is quitting).
//!
//! `open -n --args` gets the same message there by starting a whole second
//! copy of the browser, which costs a second or more when it is not in the
//! page cache; writing it here takes milliseconds. Any failure leaves the
//! caller to run `open` as before.
//!
//! That is the upstream Chromium format, not a contract for its forks, and
//! the catalog's `singleton_protocol` names the one each browser speaks.
//! Microsoft Edge (156, captured from its own sender: the fixture
//! `tests/fixtures/edge-156-singleton-message.bin`) puts two numbers between
//! the cwd and argv: the wall-clock time in microseconds since 1601
//! (Chromium's `base::Time`) and the system's uptime in microseconds, sleep
//! excluded (`base::TimeTicks` on macOS, `CLOCK_UPTIME_RAW`). Sent the
//! upstream message, Edge still answers `ACK` but reads the program and the
//! profile flag as those numbers, losing the profile and the URL: an ACK
//! alone does not establish that a header was understood.
//!
//! Beyond Chromium's own checks, the socket and its directory must be ours
//! (owner uid, no group or other write), the directory a real one under the
//! user's temp dir, and the socket a socket. Nothing here logs, and no
//! [`SocketError`] carries the command line: it holds the URL.

// The socket is Unix-only; the encoding and checks still build (and are
// tested) everywhere.
#![cfg_attr(not(unix), allow(dead_code))]

use std::{
    fmt, io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::catalog::SingletonProtocol;

#[cfg(unix)]
use std::time::Instant;

/// How long a hand-off may take, connect to `ACK`, before the caller falls
/// back. Chromium's own client waits 20 s; a browser that is that slow to
/// answer is better reached by `open`.
pub(crate) const SOCKET_BUDGET: Duration = Duration::from_secs(1);

/// The longest message Chromium reads (`kMaxMessageLength`); it drops the
/// rest. Ours must be shorter.
const MAX_MESSAGE: usize = 32 * 1024;

const START: &[u8] = b"START";
const ACK: &[u8] = b"ACK";
const SHUTDOWN: &[u8] = b"SHUTDOWN";

/// The bytes Chromium trims from each token it reads (`kWhitespaceASCII`).
const CHROMIUM_WHITESPACE: [u8; 6] = [b'\t', b'\n', 0x0B, 0x0C, b'\r', b' '];

/// A hand-off over the singleton socket: where to find it, and what to say.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SocketHandOff {
    /// The browser's user data dir, which holds `SingletonSocket`.
    pub user_data_dir: PathBuf,
    /// The command line, program first. Holds the URL: never log it.
    pub argv: Vec<String>,
    /// The message format the browser reads.
    pub(crate) protocol: SingletonProtocol,
    /// Where the user's temp dir is; the socket must be under it.
    pub(crate) temp_dir: PathBuf,
    /// The uid the socket and its directory must belong to.
    pub(crate) uid: u32,
}

/// Why a hand-off did not happen. Never carries the command line or URL.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SocketError {
    /// No `SingletonSocket` link: the browser is not running from this
    /// user data dir, or is between runs.
    NoSocket,
    /// `SingletonSocket` is there but not a symlink (Chromium before 2010)
    /// or names a relative path.
    NotALink,
    /// The socket's directory is not under the user's temp dir.
    OutsideTempDir,
    /// The socket or its directory belongs to someone else, may be written
    /// by others, or is not what it should be (a socket, a directory).
    NotOurs(&'static str),
    /// The cookie beside the socket is missing or does not match the user
    /// data dir's: the socket is stale or someone else's.
    CookieMismatch,
    /// An argument has a NUL (the message's separator) or whitespace at
    /// either end (Chromium trims it), so the socket cannot carry it as is.
    Unrepresentable,
    /// The message is longer than Chromium reads.
    TooLong,
    Connect(io::ErrorKind),
    Write(io::ErrorKind),
    Read(io::ErrorKind),
    /// The time budget ran out, writing the message or waiting for the
    /// reply.
    NoReply,
    /// The browser closed the socket without a reply.
    Closed,
    /// The browser is shutting down.
    ShuttingDown,
    /// A reply that is neither `ACK` nor `SHUTDOWN`.
    Unexpected,
    /// The socket hand-off is not built for this platform.
    Unsupported,
    /// Edge's header needs the system's uptime and there was none to read.
    NoClock,
}

impl fmt::Display for SocketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSocket => f.write_str("no SingletonSocket"),
            Self::NotALink => f.write_str("SingletonSocket is not an absolute symlink"),
            Self::OutsideTempDir => f.write_str("the socket is outside the user's temp dir"),
            Self::NotOurs(what) => write!(f, "refused the socket: {what}"),
            Self::CookieMismatch => f.write_str("the singleton cookies do not match"),
            Self::Unrepresentable => {
                f.write_str("an argument cannot be sent as is (NUL or edge whitespace)")
            }
            Self::TooLong => write!(f, "the command line is over {MAX_MESSAGE} bytes"),
            Self::Connect(kind) => write!(f, "connect: {kind}"),
            Self::Write(kind) => write!(f, "write: {kind}"),
            Self::Read(kind) => write!(f, "read: {kind}"),
            Self::NoReply => f.write_str("the browser did not answer in time"),
            Self::Closed => f.write_str("the browser closed the socket without a reply"),
            Self::ShuttingDown => f.write_str("the browser is shutting down"),
            Self::Unexpected => f.write_str("an unexpected reply"),
            Self::Unsupported => f.write_str("not supported on this platform"),
            Self::NoClock => f.write_str("no uptime clock for Edge's header"),
        }
    }
}

impl std::error::Error for SocketError {}

/// Microseconds from 1601-01-01 to 1970-01-01, as Chromium's `base::Time`
/// counts from 1601 (`kTimeTToMicrosecondsOffset`).
const UNIX_EPOCH_SINCE_1601_US: u128 = 11_644_473_600_000_000;

/// The fields `protocol` puts between the cwd and argv, read now: none for
/// upstream Chromium; for Edge the wall clock (microseconds since 1601) and
/// `uptime` (microseconds), which Edge's own sender reads as it sends.
/// `None` when Edge's header needs an uptime there is none of.
pub(crate) fn header(protocol: SingletonProtocol, uptime: Option<Duration>) -> Option<Vec<String>> {
    match protocol {
        SingletonProtocol::Chromium => Some(Vec::new()),
        SingletonProtocol::Edge => {
            let since_unix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_micros();
            Some(vec![
                (since_unix + UNIX_EPOCH_SINCE_1601_US).to_string(),
                uptime?.as_micros().to_string(),
            ])
        }
    }
}

/// The message for `argv`, run from `cwd`: `START`, `cwd`, the protocol's
/// `header` fields (see [`header`]) and each argument, NUL-separated, as the
/// browser's own client writes it.
pub(crate) fn encode(
    cwd: &str,
    header: &[String],
    argv: &[String],
) -> Result<Vec<u8>, SocketError> {
    let mut message = START.to_vec();
    let tokens = std::iter::once(cwd)
        .chain(header.iter().map(String::as_str))
        .chain(argv.iter().map(String::as_str));
    for token in tokens {
        let bytes = token.as_bytes();
        let trimmed = bytes
            .first()
            .is_some_and(|b| CHROMIUM_WHITESPACE.contains(b))
            || bytes
                .last()
                .is_some_and(|b| CHROMIUM_WHITESPACE.contains(b));
        if bytes.contains(&0) || trimmed {
            return Err(SocketError::Unrepresentable);
        }
        message.push(0);
        message.extend_from_slice(bytes);
    }
    if message.len() >= MAX_MESSAGE {
        return Err(SocketError::TooLong);
    }
    Ok(message)
}

/// What a reply so far says: `Some` once it is decided.
fn decide(reply: &[u8]) -> Option<Result<(), SocketError>> {
    if reply.starts_with(ACK) {
        Some(Ok(()))
    } else if reply.starts_with(SHUTDOWN) {
        Some(Err(SocketError::ShuttingDown))
    } else if ACK.starts_with(reply) || SHUTDOWN.starts_with(reply) {
        None
    } else {
        Some(Err(SocketError::Unexpected))
    }
}

/// The uid this process runs as.
#[cfg(unix)]
pub(crate) fn current_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

#[cfg(not(unix))]
pub(crate) fn current_uid() -> u32 {
    0
}

/// The socket a hand-off would connect to, and the cookie it must carry,
/// after every check that does not connect. The planner calls it so the dry
/// run says which way a link would go; [`send`] calls it again.
#[cfg(unix)]
pub(crate) fn locate(handoff: &SocketHandOff) -> Result<(PathBuf, PathBuf), SocketError> {
    use std::{
        fs,
        os::unix::fs::{FileTypeExt, MetadataExt},
    };

    let target = match fs::read_link(handoff.user_data_dir.join("SingletonSocket")) {
        Ok(target) => target,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(SocketError::NoSocket),
        Err(_) => return Err(SocketError::NotALink),
    };
    if !target.is_absolute() {
        return Err(SocketError::NotALink);
    }
    let (Some(dir), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(SocketError::NotALink);
    };
    // Compared resolved (macOS's `/var` is `/private/var`); connected to as
    // the link names it, as Chromium does, which keeps under the path limit.
    let resolved_dir = fs::canonicalize(dir).map_err(|_| SocketError::NoSocket)?;
    let temp = fs::canonicalize(&handoff.temp_dir).map_err(|_| SocketError::OutsideTempDir)?;
    if resolved_dir == temp || !resolved_dir.starts_with(&temp) {
        return Err(SocketError::OutsideTempDir);
    }
    let ours = |meta: &fs::Metadata, what: &'static str| {
        if meta.uid() != handoff.uid {
            Err(SocketError::NotOurs(what))
        } else {
            Ok(())
        }
    };
    let dir_meta = fs::symlink_metadata(&resolved_dir).map_err(|_| SocketError::NoSocket)?;
    if !dir_meta.is_dir() {
        return Err(SocketError::NotOurs("its directory is not a directory"));
    }
    ours(&dir_meta, "its directory belongs to another user")?;
    if dir_meta.mode() & 0o022 != 0 {
        return Err(SocketError::NotOurs("others may write its directory"));
    }
    let socket_meta =
        fs::symlink_metadata(resolved_dir.join(name)).map_err(|_| SocketError::NoSocket)?;
    if !socket_meta.file_type().is_socket() {
        return Err(SocketError::NotOurs("it is not a socket"));
    }
    ours(&socket_meta, "it belongs to another user")?;
    let remote_cookie = dir.join("SingletonCookie");
    check_cookie(&handoff.user_data_dir, &remote_cookie)?;
    Ok((target, remote_cookie))
}

#[cfg(not(unix))]
pub(crate) fn locate(_handoff: &SocketHandOff) -> Result<(PathBuf, PathBuf), SocketError> {
    Err(SocketError::Unsupported)
}

/// The user data dir's cookie, which the one beside the socket must match.
fn check_cookie(user_data_dir: &Path, remote_cookie: &Path) -> Result<(), SocketError> {
    let ours = std::fs::read_link(user_data_dir.join("SingletonCookie"))
        .map_err(|_| SocketError::CookieMismatch)?;
    let theirs = std::fs::read_link(remote_cookie).map_err(|_| SocketError::CookieMismatch)?;
    if ours.as_os_str().is_empty() || ours != theirs {
        return Err(SocketError::CookieMismatch);
    }
    Ok(())
}

/// Hands `handoff`'s command line to the browser and waits up to `budget`
/// for its `ACK`. `uptime` reads the clock Edge's header carries; see
/// [`header`].
#[cfg(unix)]
pub(crate) fn send(
    handoff: &SocketHandOff,
    uptime: Option<&dyn Fn() -> Option<Duration>>,
    budget: Duration,
) -> Result<(), SocketError> {
    use std::{
        io::{Read as _, Write as _},
        net::Shutdown,
        os::unix::net::UnixStream,
    };

    let deadline = Instant::now() + budget;
    let left = || {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or(SocketError::NoReply)
    };
    let (socket, remote_cookie) = locate(handoff)?;
    // Read as late as possible: these are "now" in the browser's eyes.
    let header =
        header(handoff.protocol, uptime.and_then(|clock| clock())).ok_or(SocketError::NoClock)?;
    let message = encode("/", &header, &handoff.argv)?;
    // A Unix socket's connect does not wait on macOS: a full backlog is
    // refused at once.
    let mut stream = UnixStream::connect(&socket).map_err(|e| SocketError::Connect(e.kind()))?;
    check_cookie(&handoff.user_data_dir, &remote_cookie)?;
    // A timeout reads as WouldBlock on Unix.
    let or_timeout = |e: io::Error, other: fn(io::ErrorKind) -> SocketError| match e.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => SocketError::NoReply,
        kind => other(kind),
    };
    stream
        .set_write_timeout(Some(left()?))
        .map_err(|e| SocketError::Write(e.kind()))?;
    stream
        .write_all(&message)
        .map_err(|e| or_timeout(e, SocketError::Write))?;
    // Chromium reads to end of stream before it acts.
    stream
        .shutdown(Shutdown::Write)
        .map_err(|e| SocketError::Write(e.kind()))?;
    let mut reply = Vec::with_capacity(SHUTDOWN.len());
    let mut buf = [0u8; 16];
    loop {
        stream
            .set_read_timeout(Some(left()?))
            .map_err(|e| SocketError::Read(e.kind()))?;
        match stream.read(&mut buf) {
            // Anything decided returned already.
            Ok(0) => return Err(SocketError::Closed),
            Ok(n) => {
                reply.extend_from_slice(&buf[..n]);
                if let Some(decided) = decide(&reply) {
                    return decided;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(or_timeout(e, SocketError::Read)),
        }
    }
}

#[cfg(not(unix))]
pub(crate) fn send(
    _handoff: &SocketHandOff,
    _uptime: Option<&dyn Fn() -> Option<Duration>>,
    _budget: Duration,
) -> Result<(), SocketError> {
    Err(SocketError::Unsupported)
}

/// A fake running browser for this crate's tests.
#[cfg(all(test, unix))]
pub(crate) mod fake {
    use std::{
        fs,
        io::{Read as _, Write as _},
        os::unix::{fs::PermissionsExt as _, net::UnixListener},
        path::PathBuf,
        sync::mpsc,
        thread,
        time::Duration,
    };

    /// A running Chromium browser as the singleton sees it: a user data
    /// dir with the two links, and a socket in a temp dir. Short names:
    /// the socket's path must stay under the Unix socket path limit.
    pub(crate) struct Fake {
        root: tempfile::TempDir,
        pub(crate) data: PathBuf,
        pub(crate) temp: PathBuf,
        pub(crate) sock_dir: PathBuf,
        pub(crate) listener: Option<UnixListener>,
    }

    impl Fake {
        pub(crate) fn new(data: &str) -> Self {
            let root = tempfile::Builder::new().prefix("s").tempdir().unwrap();
            let data = root.path().join(data);
            let temp = root.path().join("t");
            let sock_dir = temp.join("s");
            fs::create_dir_all(&data).unwrap();
            fs::create_dir_all(&sock_dir).unwrap();
            fs::set_permissions(&sock_dir, fs::Permissions::from_mode(0o700)).unwrap();
            let socket = sock_dir.join("SingletonSocket");
            let listener = UnixListener::bind(&socket).unwrap();
            std::os::unix::fs::symlink(&socket, data.join("SingletonSocket")).unwrap();
            std::os::unix::fs::symlink("4242", data.join("SingletonCookie")).unwrap();
            std::os::unix::fs::symlink("4242", sock_dir.join("SingletonCookie")).unwrap();
            Self {
                root,
                data,
                temp,
                sock_dir,
                listener: Some(listener),
            }
        }

        /// The temp dir holding the user data dir and the socket's.
        pub(crate) fn root(&self) -> &std::path::Path {
            self.root.path()
        }

        /// Serves one connection: reads to end of stream, sends it back
        /// on the channel, then replies `reply` (nothing for `None`,
        /// after a pause longer than the test's budget).
        pub(crate) fn serve(&mut self, reply: Option<&'static [u8]>) -> mpsc::Receiver<Vec<u8>> {
            let listener = self.listener.take().unwrap();
            let (tx, rx) = mpsc::channel();
            thread::spawn(move || {
                let (mut conn, _) = listener.accept().unwrap();
                let mut got = Vec::new();
                conn.read_to_end(&mut got).unwrap();
                let _ = tx.send(got);
                match reply {
                    Some(reply) => {
                        let _ = conn.write_all(reply);
                    }
                    None => thread::sleep(Duration::from_millis(500)),
                }
            });
            rx
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_owned()).collect()
    }

    #[test]
    fn the_message_is_nul_separated_like_chromiums() {
        let message = encode(
            "/",
            &[],
            &argv(&[
                "/Applications/Example.app/Contents/MacOS/Example",
                "--profile-directory=Profile 1",
                "https://example.com/a?b=c d",
            ]),
        )
        .unwrap();
        assert_eq!(
            message,
            b"START\0/\0/Applications/Example.app/Contents/MacOS/Example\0--profile-directory=Profile 1\0https://example.com/a?b=c d"
        );
    }

    #[test]
    fn what_chromium_would_change_is_refused() {
        for bad in ["a\0b", " leading", "trailing\n", "\ttab"] {
            assert_eq!(
                encode("/", &[], &argv(&["x", bad])),
                Err(SocketError::Unrepresentable),
                "{bad:?}"
            );
        }
        let long = "x".repeat(MAX_MESSAGE);
        assert_eq!(
            encode("/", &[], &argv(&["x", &long])),
            Err(SocketError::TooLong)
        );
        // Just under the limit is fine.
        let fits = "x".repeat(MAX_MESSAGE - b"START\0/\0x\0".len() - 1);
        assert!(encode("/", &[], &argv(&["x", &fits])).is_ok());
    }

    /// The tokens of a captured message, split as the browser splits it.
    fn tokens(message: &[u8]) -> Vec<String> {
        message
            .split(|b| *b == 0)
            .map(|t| String::from_utf8(t.to_vec()).unwrap())
            .collect()
    }

    /// Chrome's and Edge 156's own senders, captured against a fake
    /// receiver (an isolated `--user-data-dir`): ours writes the same bytes.
    #[test]
    fn messages_match_each_browsers_own_sender() {
        let chrome = include_bytes!("../tests/fixtures/chrome-singleton-message.bin");
        let t = tokens(chrome);
        assert_eq!(t[..2], ["START", "/"]);
        assert!(t[2].ends_with("/Google Chrome"), "{t:?}");
        assert_eq!(encode("/", &[], &t[2..]).unwrap(), chrome);

        let edge = include_bytes!("../tests/fixtures/edge-156-singleton-message.bin");
        let t = tokens(edge);
        assert_eq!(t[..2], ["START", "/"]);
        // The two numbers, then the program: Edge's header.
        assert!(
            t[2..4]
                .iter()
                .all(|n| n.bytes().all(|b| b.is_ascii_digit()))
        );
        assert!(t[4].ends_with("/Microsoft Edge"), "{t:?}");
        assert_eq!(
            t[t.len() - 2..],
            [
                "--profile-directory=Profile 1",
                "https://example.com/edge-capture"
            ]
        );
        assert_eq!(encode("/", &t[2..4], &t[4..]).unwrap(), edge);
    }

    #[test]
    fn edges_header_is_the_wall_clock_since_1601_and_the_uptime() {
        assert_eq!(header(SingletonProtocol::Chromium, None), Some(Vec::new()));
        assert_eq!(header(SingletonProtocol::Edge, None), None);
        let before = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let fields = header(
            SingletonProtocol::Edge,
            Some(Duration::from_micros(206_565_059_142)),
        )
        .unwrap();
        let after = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let wall: u128 = fields[0].parse().unwrap();
        let since_1601 = |d: Duration| d.as_micros() + UNIX_EPOCH_SINCE_1601_US;
        assert!((since_1601(before)..=since_1601(after)).contains(&wall));
        assert_eq!(fields[1], "206565059142");
        // The captured message's wall clock decodes to its day, 2026-10-28.
        let captured: u128 = 13_437_698_950_766_736;
        let unix_secs = (captured - UNIX_EPOCH_SINCE_1601_US) / 1_000_000;
        assert_eq!(unix_secs / 86_400, 20_754);
    }

    #[test]
    fn replies_are_decided_as_soon_as_they_can_be() {
        assert_eq!(decide(b""), None);
        assert_eq!(decide(b"AC"), None);
        assert_eq!(decide(b"ACK"), Some(Ok(())));
        assert_eq!(decide(b"ACKextra"), Some(Ok(())));
        assert_eq!(decide(b"SHUT"), None);
        assert_eq!(decide(b"SHUTDOWN"), Some(Err(SocketError::ShuttingDown)));
        assert_eq!(decide(b"NOPE"), Some(Err(SocketError::Unexpected)));
    }

    #[test]
    fn errors_never_carry_the_command_line() {
        let all = [
            SocketError::NoSocket,
            SocketError::NotALink,
            SocketError::OutsideTempDir,
            SocketError::NotOurs("it is not a socket"),
            SocketError::CookieMismatch,
            SocketError::Unrepresentable,
            SocketError::TooLong,
            SocketError::Connect(io::ErrorKind::ConnectionRefused),
            SocketError::Write(io::ErrorKind::BrokenPipe),
            SocketError::Read(io::ErrorKind::ConnectionReset),
            SocketError::NoReply,
            SocketError::Closed,
            SocketError::ShuttingDown,
            SocketError::Unexpected,
            SocketError::Unsupported,
            SocketError::NoClock,
        ];
        for e in all {
            let text = e.to_string();
            assert!(!text.contains("example.com"), "{text}");
            assert!(!text.is_empty());
        }
    }

    #[cfg(unix)]
    mod fake_browser {
        use std::{fs, os::unix::fs::PermissionsExt as _};

        use super::*;
        use crate::singleton::fake::Fake;

        fn handoff(fake: &Fake) -> SocketHandOff {
            SocketHandOff {
                user_data_dir: fake.data.clone(),
                argv: argv(&["/A/Example", "--profile-directory=Profile 1", URL]),
                protocol: SingletonProtocol::Chromium,
                temp_dir: fake.temp.clone(),
                uid: current_uid(),
            }
        }

        const URL: &str = "https://example.com/singleton";

        #[test]
        fn an_ack_is_a_hand_off() {
            let mut fake = Fake::new("d");
            let got = fake.serve(Some(b"ACK"));
            let handoff = handoff(&fake);
            send(&handoff, None, SOCKET_BUDGET).unwrap();
            assert_eq!(
                got.recv().unwrap(),
                encode("/", &[], &handoff.argv).unwrap()
            );
        }

        #[test]
        fn edge_gets_its_header_read_at_send_time() {
            let mut fake = Fake::new("d");
            let got = fake.serve(Some(b"ACK"));
            let mut edge = handoff(&fake);
            edge.protocol = SingletonProtocol::Edge;
            let uptime = || Some(Duration::from_micros(42_000_123));
            send(&edge, Some(&uptime), SOCKET_BUDGET).unwrap();
            let t = tokens(&got.recv().unwrap());
            assert_eq!(t[..2], ["START", "/"]);
            assert!(t[2].parse::<u128>().unwrap() > UNIX_EPOCH_SINCE_1601_US);
            assert_eq!(t[3], "42000123");
            assert_eq!(t[4..], edge.argv);

            // No clock, no Edge message: the caller falls back.
            let fake = Fake::new("d");
            let mut edge = handoff(&fake);
            edge.protocol = SingletonProtocol::Edge;
            assert_eq!(send(&edge, None, SOCKET_BUDGET), Err(SocketError::NoClock));
        }

        #[test]
        fn shutdown_silence_and_garbage_fall_back() {
            let mut fake = Fake::new("d");
            let _got = fake.serve(Some(b"SHUTDOWN"));
            assert_eq!(
                send(&handoff(&fake), None, SOCKET_BUDGET),
                Err(SocketError::ShuttingDown)
            );

            let mut fake = Fake::new("d");
            let _got = fake.serve(None);
            let started = Instant::now();
            assert_eq!(
                send(&handoff(&fake), None, Duration::from_millis(100)),
                Err(SocketError::NoReply)
            );
            assert!(started.elapsed() < Duration::from_millis(450));

            let mut fake = Fake::new("d");
            let _got = fake.serve(Some(b"WHAT"));
            assert_eq!(
                send(&handoff(&fake), None, SOCKET_BUDGET),
                Err(SocketError::Unexpected)
            );

            let mut fake = Fake::new("d");
            let _got = fake.serve(Some(b""));
            assert_eq!(
                send(&handoff(&fake), None, SOCKET_BUDGET),
                Err(SocketError::Closed)
            );
        }

        #[test]
        fn nobody_listening_is_a_connect_error() {
            let mut fake = Fake::new("d");
            drop(fake.listener.take());
            assert!(matches!(
                send(&handoff(&fake), None, SOCKET_BUDGET),
                Err(SocketError::Connect(_))
            ));
        }

        #[test]
        fn no_socket_link_is_no_socket() {
            let fake = Fake::new("d");
            fs::remove_file(fake.data.join("SingletonSocket")).unwrap();
            assert_eq!(locate(&handoff(&fake)), Err(SocketError::NoSocket));
        }

        #[test]
        fn cookies_must_match() {
            let fake = Fake::new("d");
            fs::remove_file(fake.sock_dir.join("SingletonCookie")).unwrap();
            std::os::unix::fs::symlink("9999", fake.sock_dir.join("SingletonCookie")).unwrap();
            assert_eq!(locate(&handoff(&fake)), Err(SocketError::CookieMismatch));

            let fake = Fake::new("d");
            fs::remove_file(fake.sock_dir.join("SingletonCookie")).unwrap();
            assert_eq!(locate(&handoff(&fake)), Err(SocketError::CookieMismatch));

            let fake = Fake::new("d");
            fs::remove_file(fake.data.join("SingletonCookie")).unwrap();
            assert_eq!(locate(&handoff(&fake)), Err(SocketError::CookieMismatch));
        }

        #[test]
        fn the_socket_must_be_ours_and_under_the_temp_dir() {
            let fake = Fake::new("d");
            let mut elsewhere = handoff(&fake);
            elsewhere.temp_dir = fake.data.clone();
            assert_eq!(locate(&elsewhere), Err(SocketError::OutsideTempDir));

            let mut someone_else = handoff(&fake);
            someone_else.uid = current_uid().wrapping_add(1);
            assert!(matches!(
                locate(&someone_else),
                Err(SocketError::NotOurs(_))
            ));

            fs::set_permissions(&fake.sock_dir, fs::Permissions::from_mode(0o777)).unwrap();
            assert_eq!(
                locate(&handoff(&fake)),
                Err(SocketError::NotOurs("others may write its directory"))
            );
        }

        #[test]
        fn the_link_must_be_absolute_and_name_a_socket() {
            let fake = Fake::new("d");
            let link = fake.data.join("SingletonSocket");
            fs::remove_file(&link).unwrap();
            std::os::unix::fs::symlink("t/s/SingletonSocket", &link).unwrap();
            assert_eq!(locate(&handoff(&fake)), Err(SocketError::NotALink));

            fs::remove_file(&link).unwrap();
            fs::write(&link, "").unwrap();
            assert_eq!(locate(&handoff(&fake)), Err(SocketError::NotALink));

            fs::remove_file(&link).unwrap();
            let file = fake.sock_dir.join("plain");
            fs::write(&file, "").unwrap();
            std::os::unix::fs::symlink(&file, &link).unwrap();
            assert_eq!(
                locate(&handoff(&fake)),
                Err(SocketError::NotOurs("it is not a socket"))
            );
        }
    }
}
