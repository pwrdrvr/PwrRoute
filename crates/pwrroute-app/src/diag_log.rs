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
        log::error!(target: "pwrroute", "panic: {info}");
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
        if let Some(file) = self
            .file
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            file.write(&line);
        }
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
struct LogFile {
    path: PathBuf,
    file: File,
    len: u64,
}

impl LogFile {
    fn open(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let path = dir.join(FILE);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let len = file.metadata()?.len();
        let mut log = Self { path, file, len };
        if log.len >= MAX_BYTES {
            log.rotate()?;
        }
        Ok(log)
    }

    fn write(&mut self, line: &str) {
        if self.len + line.len() as u64 > MAX_BYTES && self.rotate().is_err() {
            return;
        }
        if self.file.write_all(line.as_bytes()).is_ok() {
            self.len += line.len() as u64;
        }
    }

    fn rotate(&mut self) -> io::Result<()> {
        fs::rename(&self.path, self.path.with_file_name(OLD_FILE))?;
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.len = 0;
        Ok(())
    }
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
}
