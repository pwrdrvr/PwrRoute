//! The diagnostic log: `main.log` in [`pwrroute_os::log_dir`]
//! (`~/Library/Logs/PwrRoute` on macOS), beside the other Pwr apps' logs.
//!
//! One logger for the `log` facade, so PwrRoute's own messages, Velopack's
//! updater and GPUI's warnings land in one file. A Finder or LaunchServices
//! launch has no terminal, and stderr goes nowhere; this is what is left
//! when a link was slow or an update failed. When stderr is a terminal (a
//! dev run), each line is echoed there too.
//!
//! Never a URL: the dispatch thread's lines come from
//! `DispatchRecord::log_line`, and per-link lines are written only while
//! links are kept at all (see `pwrroute_router::Options::diagnostic_log`).
//!
//! `main.log` is rotated to `main.old.log` past [`MAX_BYTES`], so the two
//! together stay under twice that.

use std::{
    cell::Cell,
    fs::{self, File, OpenOptions},
    io::{self, IsTerminal as _, Write as _},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, PoisonError},
};

use log::{Level, LevelFilter, Log, Metadata, Record};

/// When `main.log` is moved aside to `main.old.log`.
pub const MAX_BYTES: u64 = 1024 * 1024;

const FILE: &str = "main.log";
const OLD_FILE: &str = "main.old.log";

static PATH: OnceLock<PathBuf> = OnceLock::new();

thread_local! {
    /// This thread is inside [`Logger::log`], holding its lock: a panic
    /// there must not log again (the panic hook would wait on that lock).
    static LOGGING: Cell<bool> = const { Cell::new(false) };
}

/// Where the log is being written, once [`install`] found a directory.
pub fn path() -> Option<&'static Path> {
    PATH.get().map(PathBuf::as_path)
}

/// Installs the logger and a panic hook that logs the panic. Call once, at
/// the top of `main`. `PWRROUTE_LOG=debug` (or `trace`, `warn`, ...) sets
/// PwrRoute's own level; Velopack logs at info, everything else at warn.
pub fn install() {
    let level = std::env::var("PWRROUTE_LOG")
        .ok()
        .and_then(|value| value.parse::<LevelFilter>().ok())
        .unwrap_or(LevelFilter::Info);
    let file = pwrroute_os::log_dir().and_then(|dir| {
        let file = LogFile::open(&dir).map_err(|error| {
            eprintln!("pwrroute: no diagnostic log in {}: {error}", dir.display());
        });
        file.ok()
    });
    if let Some(file) = &file {
        let _ = PATH.set(file.path.clone());
    }
    let logger = Logger {
        own: level,
        file: Mutex::new(file),
        echo: io::stderr().is_terminal(),
    };
    if log::set_boxed_logger(Box::new(logger)).is_err() {
        return;
    }
    log::set_max_level(level.max(LevelFilter::Info));
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !LOGGING.get() {
            log::error!(target: "pwrroute", "panic: {info}");
        }
        previous(info);
    }));
}

struct Logger {
    /// PwrRoute's own level.
    own: LevelFilter,
    file: Mutex<Option<LogFile>>,
    echo: bool,
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= level_for(metadata.target(), self.own)
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format_line(
            &chrono::Local::now()
                .format("%Y-%m-%dT%H:%M:%S%.3f%:z")
                .to_string(),
            record.level(),
            record.target(),
            &record.args().to_string(),
        );
        if self.echo {
            let _ = io::stderr().write_all(line.as_bytes());
        }
        LOGGING.set(true);
        if let Some(file) = self
            .file
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            file.write(&line);
        }
        LOGGING.set(false);
    }

    fn flush(&self) {}
}

/// The most verbose level kept for `target`.
fn level_for(target: &str, own: LevelFilter) -> LevelFilter {
    if target.starts_with("pwrroute") {
        own
    } else if target.starts_with("velopack") {
        LevelFilter::Info
    } else {
        LevelFilter::Warn
    }
}

/// `2026-10-24T07:31:02.123-04:00 INFO  pwrroute::link: link 42 ...`, one
/// line: a message's own newlines are escaped, so each record stays one line
/// for grep.
fn format_line(time: &str, level: Level, target: &str, message: &str) -> String {
    let message = message.trim_end().replace('\n', "\\n");
    format!("{time} {level:<5} {target}: {message}\n")
}

/// `main.log`, appended to, and moved to `main.old.log` when it grows past
/// [`MAX_BYTES`].
///
/// Other PwrRoute processes write it too (a second launch, Velopack's hooks,
/// a copy the link helper starts), and any of them can rotate it. So nothing
/// is held open: each line opens the file the path names now, checks that
/// file's own size, appends and closes. A few lines per link make that
/// cheap, and no process can be left writing to a file another one moved.
struct LogFile {
    path: PathBuf,
}

impl LogFile {
    fn open(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let log = Self {
            path: dir.join(FILE),
        };
        // Fails here, at install, rather than silently on every line.
        drop(open_append(&log.path)?);
        Ok(log)
    }

    fn write(&mut self, line: &str) {
        let Ok(mut file) = self.rotated_if_full(line.len() as u64) else {
            return;
        };
        let _ = file.write_all(line.as_bytes());
    }

    /// The file to append `extra` bytes to: `main.log`, moved aside first
    /// when they would take it past [`MAX_BYTES`].
    fn rotated_if_full(&self, extra: u64) -> io::Result<File> {
        let file = open_append(&self.path)?;
        if file.metadata()?.len() + extra <= MAX_BYTES {
            return Ok(file);
        }
        drop(file);
        fs::rename(&self.path, self.path.with_file_name(OLD_FILE))?;
        open_append(&self.path)
    }
}

/// Appends to `path`, creating it readable by this user only, as the
/// activity journal is: it names the apps links came from and went to.
fn open_append(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_one_line_with_time_level_and_target() {
        assert_eq!(
            format_line(
                "2026-10-24T07:31:02.123-04:00",
                Level::Warn,
                "pwrroute::link",
                "first\nsecond\n"
            ),
            "2026-10-24T07:31:02.123-04:00 WARN  pwrroute::link: first\\nsecond\n"
        );
    }

    #[test]
    fn pwrroute_follows_its_level_velopack_logs_info_and_the_rest_warns() {
        assert_eq!(
            level_for("pwrroute::link", LevelFilter::Debug),
            LevelFilter::Debug
        );
        assert_eq!(
            level_for("pwrroute_router::dispatch", LevelFilter::Info),
            LevelFilter::Info
        );
        assert_eq!(
            level_for("velopack::manager", LevelFilter::Debug),
            LevelFilter::Info
        );
        assert_eq!(level_for("gpui", LevelFilter::Debug), LevelFilter::Warn);
    }

    #[test]
    fn the_log_moves_aside_past_its_limit_and_keeps_one_old_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = LogFile::open(dir.path()).unwrap();
        let line = format!("{}\n", "x".repeat(1023));
        for _ in 0..(MAX_BYTES / 1024) {
            log.write(&line);
        }
        assert_eq!(
            fs::metadata(dir.path().join(FILE)).unwrap().len(),
            MAX_BYTES
        );
        assert!(!dir.path().join(OLD_FILE).exists());

        log.write("next\n");
        assert_eq!(fs::read_to_string(dir.path().join(FILE)).unwrap(), "next\n");
        assert_eq!(
            fs::metadata(dir.path().join(OLD_FILE)).unwrap().len(),
            MAX_BYTES
        );

        // Reopened past the limit (another process wrote it), it starts fresh.
        drop(log);
        fs::write(dir.path().join(FILE), "y".repeat(MAX_BYTES as usize)).unwrap();
        let mut log = LogFile::open(dir.path()).unwrap();
        log.write("after\n");
        assert_eq!(
            fs::read_to_string(dir.path().join(FILE)).unwrap(),
            "after\n"
        );
    }

    #[test]
    fn a_rotation_by_another_process_is_followed_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let mut ours = LogFile::open(dir.path()).unwrap();
        ours.write("ours 1\n");
        // Another process fills the log and rotates it.
        let mut theirs = LogFile::open(dir.path()).unwrap();
        theirs.write(&"t".repeat(MAX_BYTES as usize - "ours 1\n".len()));
        theirs.write("theirs\n");
        assert_eq!(
            fs::read_to_string(dir.path().join(FILE)).unwrap(),
            "theirs\n"
        );

        ours.write("ours 2\n");
        assert_eq!(
            fs::read_to_string(dir.path().join(FILE)).unwrap(),
            "theirs\nours 2\n"
        );
        assert!(
            fs::read_to_string(dir.path().join(OLD_FILE))
                .unwrap()
                .starts_with("ours 1\n")
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_log_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let mut log = LogFile::open(dir.path()).unwrap();
        log.write("x\n");
        let mode = fs::metadata(dir.path().join(FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
