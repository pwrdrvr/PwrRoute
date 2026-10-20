//! The opt-in activity journal: one JSON line per dispatch, a file per day.
//!
//! Off unless [`Options::journal`](crate::Options::journal) is set. When on,
//! the dispatch thread appends each recorded dispatch to
//! `<dir>/activity-YYYY-MM-DD.jsonl` (the UTC day it was received; `0600`
//! on Unix, in a `0700` directory it creates), as a [`DispatchRecord`]
//! reduced to the lesser of the record's own [`Privacy`] and the journal's
//! [`JournalMode`]. Error text is withheld at every level: a config or
//! launcher message can quote the URL. Files older than
//! [`JournalConfig::days`] are deleted when a new day's file opens.
//!
//! A write that fails is reported once, without link data, and never stops
//! a dispatch.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};

use crate::trace::{DispatchRecord, Privacy, Step};

/// The most a journal line keeps of any dispatch; a record's own
/// [`Privacy`] can keep less.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JournalMode {
    /// Scheme and host only, for every link.
    #[default]
    Hosts,
    /// Full URLs, except for links whose rule keeps less.
    Full,
}

impl JournalMode {
    fn cap(self) -> Privacy {
        match self {
            Self::Hosts => Privacy::Host,
            Self::Full => Privacy::Full,
        }
    }
}

/// Where and how much to journal. See the [module docs](self).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalConfig {
    pub dir: PathBuf,
    #[serde(default)]
    pub mode: JournalMode,
    /// How many days of files to keep, today included. At least one.
    #[serde(default = "default_days")]
    pub days: u32,
}

const DEFAULT_DAYS: u32 = 7;

fn default_days() -> u32 {
    DEFAULT_DAYS
}

impl JournalConfig {
    /// A journal in `dir` kept for the default seven days.
    pub fn new(dir: impl Into<PathBuf>, mode: JournalMode) -> Self {
        Self {
            dir: dir.into(),
            mode,
            days: DEFAULT_DAYS,
        }
    }

    /// The files it writes, as a pattern for a description.
    pub fn file_pattern(&self) -> PathBuf {
        self.dir.join("activity-YYYY-MM-DD.jsonl")
    }

    /// Everything journaled so far, newest first, at most `limit` records.
    /// A line that does not parse (a write cut short) is skipped. An absent
    /// directory is an empty journal.
    pub fn read(&self, limit: usize) -> io::Result<Vec<DispatchRecord>> {
        read_recent(&self.dir, limit)
    }
}

/// Everything journaled under `dir`, newest first, at most `limit` records.
/// See [`JournalConfig::read`].
pub fn read_recent(dir: &Path, limit: usize) -> io::Result<Vec<DispatchRecord>> {
    let mut records = Vec::new();
    if limit == 0 {
        return Ok(records);
    }
    let mut files = journal_files(dir)?;
    files.sort_unstable_by_key(|(day, _)| std::cmp::Reverse(*day));
    for (_, path) in files {
        let text = fs::read_to_string(&path)?;
        for line in text.lines().rev() {
            if let Ok(record) = serde_json::from_str::<DispatchRecord>(line) {
                records.push(record);
                if records.len() == limit {
                    return Ok(records);
                }
            }
        }
    }
    Ok(records)
}

/// The journal's files under `dir`, each with its day.
fn journal_files(dir: &Path) -> io::Result<Vec<(Day, PathBuf)>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        if let Some(day) = name.to_str().and_then(Day::from_file_name) {
            files.push((day, entry.path()));
        }
    }
    Ok(files)
}

/// The writer, owned by the dispatch thread.
pub(crate) struct Journal {
    config: JournalConfig,
    open: Option<(Day, fs::File)>,
    /// The last write failure, reported once; cleared by the next success.
    error: Option<String>,
    /// The last pruning failure, reported once. Pruning is best effort:
    /// it never costs a line.
    prune_error: Option<String>,
}

impl Journal {
    pub(crate) fn new(config: JournalConfig) -> Self {
        Self {
            config,
            open: None,
            error: None,
            prune_error: None,
        }
    }

    /// Why the last write, or the last pruning, failed, if it did.
    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref().or(self.prune_error.as_deref())
    }

    /// Appends `record`, reduced to what the journal may keep, to the file
    /// for the day it was received. A failure is printed once, without
    /// link data, and otherwise ignored.
    pub(crate) fn append(&mut self, record: &DispatchRecord) {
        match self.try_append(record) {
            Ok(()) => self.error = None,
            Err(e) => {
                let message = format!(
                    "the activity journal in {} cannot be written: {e}",
                    self.config.dir.display()
                );
                if self.error.is_none() {
                    eprintln!("pwrroute: {message}; links are still routed");
                }
                self.error = Some(message);
            }
        }
    }

    fn try_append(&mut self, record: &DispatchRecord) -> io::Result<()> {
        let mut line = serde_json::to_string(&journal_entry(record, self.config.mode))
            .map_err(io::Error::other)?;
        line.push('\n');
        let file = self.file_for(Day::from(record.received_at))?;
        file.write_all(line.as_bytes())
    }

    /// The open file for `day`, opening (and pruning) when the day moves.
    fn file_for(&mut self, day: Day) -> io::Result<&mut fs::File> {
        if self.open.as_ref().is_none_or(|(open, _)| *open != day) {
            create_private_dir(&self.config.dir)?;
            let file = open_private(&self.config.dir.join(day.file_name()))?;
            self.open = Some((day, file));
            if let Err(e) = self.prune(day) {
                let message = format!(
                    "old files in the activity journal in {} cannot be deleted: {e}",
                    self.config.dir.display()
                );
                if self.prune_error.is_none() {
                    eprintln!("pwrroute: {message}");
                }
                self.prune_error = Some(message);
            } else {
                self.prune_error = None;
            }
        }
        Ok(&mut self.open.as_mut().expect("just opened").1)
    }

    /// Deletes files older than `days`, counting `today` as the first.
    fn prune(&self, today: Day) -> io::Result<()> {
        let keep_from = today.minus_days(self.config.days.max(1) - 1);
        for (day, path) in journal_files(&self.config.dir)? {
            if day < keep_from {
                match fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(())
    }
}

/// `record` as the journal keeps it: capped by the mode, and with error
/// text withheld at every level.
fn journal_entry(record: &DispatchRecord, mode: JournalMode) -> DispatchRecord {
    let mut entry = record.reduced(mode.cap());
    for step in &mut entry.steps {
        if let Step::ConfigError { message } | Step::LaunchFailed { message } = step {
            message.clear();
        }
    }
    entry
}

fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Opens the day's file for appending: created fresh as `0600`, or, when
/// it already exists, reopened only if it is a plain file nobody else can
/// read. A symlink in its place is never followed, and a file with a wider
/// mode is refused rather than re-moded: someone set it that way, and the
/// status should say so instead of hiding it.
fn open_private(path: &Path) -> io::Result<fs::File> {
    let mut create = fs::OpenOptions::new();
    create.create_new(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        create.mode(0o600);
    }
    match create.open(path) {
        Ok(file) => return Ok(file),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    open_existing(path)
}

#[cfg(unix)]
fn open_existing(path: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let refuse = |why: &str| {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("the day's file {why}; move it aside"),
        ))
    };
    let file = match fs::OpenOptions::new()
        .append(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => return refuse("is a symbolic link"),
        Err(e) => return Err(e),
    };
    // Checked on the open descriptor, so nothing can be swapped in between.
    let meta = file.metadata()?;
    if !meta.file_type().is_file() {
        return refuse("is not a regular file");
    }
    // Owner-only: a file we could open for writing and that only its owner
    // can read is ours (short of running as root), with no `getuid` call,
    // which the workspace's `unsafe` ban keeps out.
    if meta.mode() & 0o077 != 0 {
        return refuse("is readable by other users");
    }
    Ok(file)
}

#[cfg(windows)]
fn open_existing(path: &Path) -> io::Result<fs::File> {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    // A symlink or junction in the file's place is refused before the open,
    // which would follow it; the opened handle is checked again after.
    let meta = fs::symlink_metadata(path)?;
    if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 || !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the day's file is not a regular file; move it aside",
        ));
    }
    let file = fs::OpenOptions::new().append(true).open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the day's file is not a regular file; move it aside",
        ));
    }
    Ok(file)
}

/// A UTC calendar day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Day {
    year: i64,
    month: u8,
    day: u8,
}

impl Day {
    fn file_name(self) -> String {
        format!(
            "activity-{:04}-{:02}-{:02}.jsonl",
            self.year, self.month, self.day
        )
    }

    fn from_file_name(name: &str) -> Option<Self> {
        let date = name
            .strip_prefix("activity-")?
            .strip_suffix(".jsonl")?
            .as_bytes();
        if date.len() != 10 || date[4] != b'-' || date[7] != b'-' {
            return None;
        }
        let digits = |range: std::ops::Range<usize>| -> Option<i64> {
            let text = std::str::from_utf8(&date[range]).ok()?;
            if !text.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            text.parse().ok()
        };
        let (year, month, day) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        Some(Self {
            year,
            month: u8::try_from(month).ok()?,
            day: u8::try_from(day).ok()?,
        })
    }

    fn minus_days(self, days: u32) -> Self {
        Self::from_days(self.to_days() - i64::from(days))
    }

    /// Days since 1970-01-01, by Howard Hinnant's date algorithms.
    fn to_days(self) -> i64 {
        let (y, m, d) = (self.year, i64::from(self.month), i64::from(self.day));
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    fn from_days(days: i64) -> Self {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        Self {
            year,
            month: u8::try_from(month).unwrap_or(1),
            day: u8::try_from(day).unwrap_or(1),
        }
    }
}

impl From<SystemTime> for Day {
    fn from(t: SystemTime) -> Self {
        let secs = match t.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
            Err(e) => -i64::try_from(e.duration().as_secs()).unwrap_or(i64::MAX),
        };
        Self::from_days(secs.div_euclid(86_400))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use pwrroute_engine::{AppType, Browser};

    use super::*;
    use crate::{
        link::{Link, OpenerInfo, Source},
        trace::{DispatchId, Matched, Outcome, Timings},
    };

    const URL: &str = "https://bank.example.com/account?token=hunter2";
    /// 2026-10-20T12:00:00Z.
    const NOON: u64 = 1_792_497_600;

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn record(id: u64, received_at: SystemTime, privacy: Privacy) -> DispatchRecord {
        let browser = Browser {
            name: "Firefox".into(),
            app_type: AppType::AppName,
            url: URL.into(),
            profile: String::new(),
            args: Vec::new(),
            open_in_background: None,
        };
        let mut record = DispatchRecord {
            id: DispatchId(id),
            received_at,
            privacy: Privacy::Full,
            link: Link::new(URL, Source::AppleEvent).with_opener(Some(OpenerInfo {
                name: "Mail".into(),
                ..OpenerInfo::default()
            })),
            forwarded: false,
            final_url: Some(URL.into()),
            browser: Some(browser.clone()),
            steps: vec![
                Step::Received { url: URL.into() },
                Step::ConfigError {
                    message: format!("threw on {URL}"),
                },
                Step::Matched {
                    by: Matched::Handler { index: 1 },
                },
                Step::Browser { browser },
                Step::Launched { dry_run: true },
            ],
            outcome: Outcome::DryRun,
            timings: Timings {
                total_us: 7,
                ..Timings::default()
            },
        };
        record.reduce(privacy);
        record
    }

    fn lines_of(path: &Path) -> Vec<String> {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn days_round_trip_and_name_files() {
        let day = Day::from(at(NOON));
        assert_eq!(day.file_name(), "activity-2026-10-20.jsonl");
        assert_eq!(Day::from_file_name("activity-2026-10-20.jsonl"), Some(day));
        assert_eq!(Day::from_file_name("activity-2026-10-2.jsonl"), None);
        assert_eq!(Day::from_file_name("notes-2026-10-20.jsonl"), None);
        assert_eq!(day.minus_days(20).file_name(), "activity-2026-09-30.jsonl");
        assert_eq!(Day::from(at(0)).file_name(), "activity-1970-01-01.jsonl");
        for days in [-1_000_000, -1, 0, 1, 59, 60, 20_746, 1_000_000] {
            assert_eq!(Day::from_days(days).to_days(), days, "{days}");
        }
    }

    #[test]
    fn each_level_writes_what_it_may_and_the_mode_caps_it() {
        let parent = tempfile::tempdir().unwrap();
        // The journal creates its own directory, privately.
        let dir = parent.path().join("activity");
        let mut journal = Journal::new(JournalConfig::new(&dir, JournalMode::Full));
        journal.append(&record(1, at(NOON), Privacy::Full));
        journal.append(&record(2, at(NOON), Privacy::Host));
        journal.append(&record(3, at(NOON), Privacy::None));
        assert_eq!(journal.error(), None);

        let path = dir.join("activity-2026-10-20.jsonl");
        let lines = lines_of(&path);
        assert_eq!(lines.len(), 3);
        let full: DispatchRecord = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(full.privacy, Privacy::Full);
        assert_eq!(full.link.url, URL);
        assert_eq!(full.final_url.as_deref(), Some(URL));
        assert_eq!(full.link.opener.as_ref().unwrap().name, "Mail");
        // Error text never reaches the journal, even in full mode.
        assert!(!lines[0].contains("threw on"));
        assert!(full.steps.contains(&Step::ConfigError {
            message: String::new()
        }));

        let host: DispatchRecord = serde_json::from_str(&lines[1]).unwrap();
        assert_eq!(host.privacy, Privacy::Host);
        assert_eq!(host.link.url, "https://bank.example.com");
        assert!(!lines[1].contains("hunter2") && !lines[1].contains("/account"));
        assert!(!lines[1].contains("withheld"));

        let none: DispatchRecord = serde_json::from_str(&lines[2]).unwrap();
        assert_eq!(none.privacy, Privacy::None);
        assert_eq!(none.outcome, Outcome::DryRun);
        assert_eq!(none.timings.total_us, 7);
        assert!(none.steps.is_empty() && none.browser.is_none());
        for private in ["example.com", "hunter2", "Mail", "Firefox"] {
            assert!(!lines[2].contains(private), "{private} in {}", lines[2]);
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }

        // Hosts mode caps a full record at host; a none record stays none.
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::new(JournalConfig::new(dir.path(), JournalMode::Hosts));
        journal.append(&record(4, at(NOON), Privacy::Full));
        journal.append(&record(5, at(NOON), Privacy::None));
        let lines = lines_of(&dir.path().join("activity-2026-10-20.jsonl"));
        let capped: DispatchRecord = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(capped.privacy, Privacy::Host);
        assert_eq!(capped.link.url, "https://bank.example.com");
        assert!(!lines[0].contains("hunter2"));
        let none: DispatchRecord = serde_json::from_str(&lines[1]).unwrap();
        assert_eq!(none.privacy, Privacy::None);
        assert!(!lines[1].contains("example.com"));
    }

    #[test]
    fn files_rotate_by_day_and_old_ones_are_pruned() {
        let dir = tempfile::tempdir().unwrap();
        let config = JournalConfig {
            days: 3,
            ..JournalConfig::new(dir.path(), JournalMode::Hosts)
        };
        let mut journal = Journal::new(config.clone());
        let day = 86_400;
        for days_ago in [10, 2, 1, 0] {
            journal.append(&record(days_ago, at(NOON - days_ago * day), Privacy::Full));
        }
        // Something that is not the journal's is left alone.
        fs::write(dir.path().join("notes.txt"), "mine").unwrap();
        // The newest day's open pruned everything before today - 2.
        let mut names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "activity-2026-10-18.jsonl",
                "activity-2026-10-19.jsonl",
                "activity-2026-10-20.jsonl",
                "notes.txt",
            ]
        );

        // The reader walks newest first, across files, up to the limit.
        let ids: Vec<_> = config.read(10).unwrap().iter().map(|r| r.id.0).collect();
        assert_eq!(ids, vec![0, 1, 2]);
        let ids: Vec<_> = config.read(2).unwrap().iter().map(|r| r.id.0).collect();
        assert_eq!(ids, vec![0, 1]);
        assert!(config.read(0).unwrap().is_empty());

        // A write cut short is skipped, and an absent journal is empty.
        let today = dir.path().join("activity-2026-10-20.jsonl");
        let mut partial = fs::read_to_string(&today).unwrap();
        partial.push_str(r#"{"id":99,"receivedAt":1,"privac"#);
        fs::write(&today, partial).unwrap();
        let ids: Vec<_> = config.read(10).unwrap().iter().map(|r| r.id.0).collect();
        assert_eq!(ids, vec![0, 1, 2]);
        assert!(
            read_recent(&dir.path().join("nowhere"), 10)
                .unwrap()
                .is_empty()
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_day_file_is_refused_and_nothing_follows_it() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("elsewhere.txt");
        fs::write(&target, "theirs\n").unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join("activity-2026-10-20.jsonl")).unwrap();
        let mut journal = Journal::new(JournalConfig::new(dir.path(), JournalMode::Full));
        journal.append(&record(1, at(NOON), Privacy::Full));
        let error = journal.error().expect("the symlink is refused");
        assert!(error.contains("is a symbolic link"), "{error}");
        assert!(!error.contains("example.com") && !error.contains("hunter2"));
        assert_eq!(fs::read_to_string(&target).unwrap(), "theirs\n");
        // The next day, a fresh file, is fine.
        journal.append(&record(2, at(NOON + 86_400), Privacy::Full));
        assert_eq!(journal.error(), None);
        assert_eq!(
            lines_of(&dir.path().join("activity-2026-10-21.jsonl")).len(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_day_file_readable_by_others_is_refused_not_remoded() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("activity-2026-10-20.jsonl");
        fs::write(&path, "").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let mut journal = Journal::new(JournalConfig::new(dir.path(), JournalMode::Full));
        journal.append(&record(1, at(NOON), Privacy::Full));
        let error = journal.error().expect("a wider mode is refused");
        assert!(error.contains("readable by other users"), "{error}");
        assert!(!error.contains("example.com") && !error.contains("hunter2"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );

        // Made private again, the same file takes the next line.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        journal.append(&record(2, at(NOON), Privacy::Full));
        assert_eq!(journal.error(), None);
        assert_eq!(lines_of(&path).len(), 1);
    }

    #[test]
    fn a_write_failure_is_kept_as_a_status_and_never_quotes_the_link() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the directory should be: nothing can be created in it.
        let blocked = dir.path().join("journal");
        fs::write(&blocked, "").unwrap();
        let mut journal = Journal::new(JournalConfig::new(&blocked, JournalMode::Full));
        journal.append(&record(1, at(NOON), Privacy::Full));
        let error = journal.error().expect("the write failed");
        assert!(error.contains("cannot be written"), "{error}");
        assert!(!error.contains("example.com") && !error.contains("hunter2"));
        assert!(fs::read_dir(dir.path()).unwrap().count() == 1);
    }
}
