//! The Logs window's model: `main.log`'s lines, parsed, filtered and
//! followed. Plain data and file reads; `logs_window.rs` draws it.
//!
//! The file is the source, so the window shows lines from earlier launches
//! and from the other processes that write it (`diag_log`), and the log
//! never carries a URL to begin with. On open the window reads
//! `main.old.log` and `main.log`, keeps the last [`MAX_LINES`], then reads
//! only what was appended; a file that shrank was rotated, and is read
//! again from the top.

use gpui_kit::SharedString;
use std::{
    fs::File,
    io::{self, Read as _, Seek as _, SeekFrom},
    path::{Path, PathBuf},
};

/// Lines kept in the window, as the family's 5000-entry buffer.
pub const MAX_LINES: usize = 5000;

/// A line's level, as `diag_log` writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warn,
    Info,
    /// Debug and trace: shown together.
    Debug,
}

impl Level {
    pub const ALL: [Self; 4] = [Self::Error, Self::Warn, Self::Info, Self::Debug];

    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "Error",
            Self::Warn => "Warning",
            Self::Info => "Info",
            Self::Debug => "Debug",
        }
    }

    fn parse(word: &str) -> Option<Self> {
        match word {
            "ERROR" => Some(Self::Error),
            "WARN" => Some(Self::Warn),
            "INFO" => Some(Self::Info),
            "DEBUG" | "TRACE" => Some(Self::Debug),
            _ => None,
        }
    }
}

/// One line of the log: `TIME LEVEL target: message`. A line in another
/// shape (a panic's continuation, a hand edit) is all message, at Info.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    pub time: String,
    pub level: Level,
    /// Where the level word is in the line, in bytes.
    pub level_range: std::ops::Range<usize>,
    /// Where the target is, when the line names one.
    pub target_range: Option<std::ops::Range<usize>>,
    pub target: String,
    pub message: String,
    /// The line as the file has it, kept so Copy is byte-exact and a row
    /// draws without rebuilding it.
    raw: SharedString,
    /// `raw` in lowercase, for the search.
    lower: String,
}

impl LogLine {
    pub fn parse(line: &str) -> Self {
        let parsed = (|| {
            let (time, rest) = line.split_once(' ')?;
            if !time.as_bytes().first()?.is_ascii_digit() {
                return None;
            }
            // Byte offsets into `line` of a slice of it.
            let at = |part: &str| part.as_ptr() as usize - line.as_ptr() as usize;
            let rest_trimmed = rest.trim_start();
            let (word, after) = rest_trimmed.split_once(' ')?;
            let level = Level::parse(word)?;
            let after = after.trim_start();
            let (target, message) = after.split_once(": ").unwrap_or(("", after));
            let level_start = at(word);
            let target_start = at(after);
            Some(Self {
                time: time.to_owned(),
                level,
                level_range: level_start..level_start + word.len(),
                target_range: (!target.is_empty())
                    .then(|| target_start..target_start + target.len()),
                target: target.to_owned(),
                message: message.to_owned(),
                raw: SharedString::default(),
                lower: String::new(),
            })
        })();
        let mut parsed = parsed.unwrap_or_else(|| Self {
            time: String::new(),
            level: Level::Info,
            level_range: 0..0,
            target_range: None,
            target: String::new(),
            message: line.to_owned(),
            raw: SharedString::default(),
            lower: String::new(),
        });
        parsed.lower = line.to_lowercase();
        parsed.raw = SharedString::from(line.to_owned());
        parsed
    }

    /// The line as the file has it.
    pub fn raw(&self) -> &SharedString {
        &self.raw
    }

    /// Case-insensitive: every word of `query` is somewhere in the line.
    pub fn matches(&self, words: &[String]) -> bool {
        words.iter().all(|word| self.lower.contains(word.as_str()))
    }
}

/// The lines in the window and how they are filtered.
#[derive(Clone, Debug)]
pub struct LogView {
    /// Oldest first. `number` of `lines[i]` is `first_number + i`.
    pub lines: Vec<LogLine>,
    /// The number of the first line kept, counting from 1 since the window
    /// opened its files.
    pub first_number: usize,
    /// Older lines were dropped past [`MAX_LINES`].
    pub truncated: bool,
    /// The levels shown.
    pub levels: [bool; 4],
    query_words: Vec<String>,
    /// Indexes into `lines` that pass the filter.
    visible: Vec<usize>,
    /// Lines kept at each level, kept as lines come and go.
    counts: [usize; 4],
}

impl Default for LogView {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            first_number: 1,
            truncated: false,
            // The family's default: Error, Warning and Info.
            levels: [true, true, true, false],
            query_words: Vec::new(),
            visible: Vec::new(),
            counts: [0; 4],
        }
    }
}

impl LogView {
    /// Starts over with `text`, as read from the files.
    pub fn replace(&mut self, text: &str) {
        self.lines.clear();
        self.counts = [0; 4];
        self.first_number = 1;
        self.truncated = false;
        self.append(text);
    }

    /// Adds complete lines, dropping the oldest past [`MAX_LINES`].
    /// Returns how many of the added lines are errors.
    pub fn append(&mut self, text: &str) -> usize {
        let mut errors = 0;
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let line = LogLine::parse(line);
            self.counts[line.level as usize] += 1;
            errors += usize::from(line.level == Level::Error);
            self.lines.push(line);
        }
        if self.lines.len() > MAX_LINES {
            let drop = self.lines.len() - MAX_LINES;
            for line in self.lines.drain(..drop) {
                self.counts[line.level as usize] -= 1;
            }
            self.first_number += drop;
            self.truncated = true;
        }
        self.refilter();
        errors
    }

    pub fn set_query(&mut self, query: &str) {
        self.query_words = query.split_whitespace().map(str::to_lowercase).collect();
        self.refilter();
    }

    pub fn searching(&self) -> bool {
        !self.query_words.is_empty()
    }

    pub fn toggle_level(&mut self, level: Level) {
        let slot = &mut self.levels[level as usize];
        *slot = !*slot;
        self.refilter();
    }

    pub fn shows(&self, level: Level) -> bool {
        self.levels[level as usize]
    }

    fn refilter(&mut self) {
        self.visible = self
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| self.levels[line.level as usize] && line.matches(&self.query_words))
            .map(|(index, _)| index)
            .collect();
    }

    /// The shown lines' indexes into `lines`.
    pub fn visible(&self) -> &[usize] {
        &self.visible
    }

    /// Lines at `level` among all kept, whatever the filter.
    pub fn count(&self, level: Level) -> usize {
        self.counts[level as usize]
    }

    /// "1,204 lines", or "12 of 1,204 lines shown" while filtered.
    pub fn summary(&self) -> String {
        let total = self.lines.len();
        let shown = self.visible.len();
        let lines = plural(total, "line", "lines");
        if shown == total {
            format!("{} {lines}", thousands(total))
        } else {
            format!("{} of {} {lines} shown", thousands(shown), thousands(total))
        }
    }
}

fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}

/// 12345 as "12,345".
pub fn thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Follows `main.log`: what was read so far, and where.
#[derive(Debug)]
pub struct Tail {
    pub path: PathBuf,
    /// Bytes of `main.log` already read.
    offset: u64,
    /// A partial last line, kept until its newline arrives.
    pending: String,
    /// The file read, so a rotation is noticed even after the new file grew
    /// past the old offset.
    identity: Option<u64>,
}

/// What a poll found.
#[derive(Debug, PartialEq, Eq)]
pub enum Read {
    /// Everything again: the first read, or the file was rotated.
    All(String),
    /// Lines appended since the last read.
    More(String),
    Nothing,
}

impl Tail {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            offset: 0,
            pending: String::new(),
            identity: None,
        }
    }

    /// `main.old.log` beside it.
    fn old_path(&self) -> PathBuf {
        self.path.with_file_name("main.old.log")
    }

    /// The whole history: `main.old.log`, then `main.log`. A `main.log`
    /// not there yet (just rotated, nothing written since) is empty; the
    /// next poll that finds it reads everything again.
    pub fn read_all(&mut self) -> io::Result<String> {
        let old = std::fs::read(self.old_path()).unwrap_or_default();
        let mut current = Vec::new();
        match File::open(&self.path) {
            Ok(mut file) => {
                self.identity = identity(&file.metadata()?);
                file.read_to_end(&mut current)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound && !old.is_empty() => {
                self.identity = None;
            }
            Err(error) => return Err(error),
        }
        self.offset = current.len() as u64;
        let mut text = String::from_utf8_lossy(&old).into_owned();
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(&current));
        self.pending.clear();
        Ok(self.complete_lines(text))
    }

    /// What changed since the last read.
    pub fn poll(&mut self) -> io::Result<Read> {
        let metadata = std::fs::metadata(&self.path)?;
        let len = metadata.len();
        if len < self.offset || identity(&metadata) != self.identity {
            return self.read_all().map(Read::All);
        }
        if len == self.offset {
            return Ok(Read::Nothing);
        }
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(self.offset))?;
        let mut bytes = Vec::new();
        file.take(len - self.offset).read_to_end(&mut bytes)?;
        self.offset += bytes.len() as u64;
        let text = format!(
            "{}{}",
            std::mem::take(&mut self.pending),
            String::from_utf8_lossy(&bytes)
        );
        let text = self.complete_lines(text);
        Ok(if text.is_empty() {
            Read::Nothing
        } else {
            Read::More(text)
        })
    }

    /// `text` up to its last newline; the rest waits in `pending`.
    fn complete_lines(&mut self, mut text: String) -> String {
        match text.rfind('\n') {
            Some(end) => {
                self.pending = text.split_off(end + 1);
                text
            }
            None => {
                self.pending = text;
                String::new()
            }
        }
    }
}

/// Which file a path names now: its inode where there is one.
#[cfg(unix)]
fn identity(metadata: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt as _;
    Some(metadata.ino())
}

#[cfg(not(unix))]
fn identity(_: &std::fs::Metadata) -> Option<u64> {
    None
}

/// The file name alone, for the footer.
pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: &str = "2026-10-24T07:31:02.123-04:00 INFO  pwrroute::link: link 42 outcome=opened";
    const WARN: &str =
        "2026-10-24T07:31:03.000-04:00 WARN  pwrroute::default_browser: reading took 68ms";

    #[test]
    fn a_line_parses_into_time_level_target_and_message() {
        let line = LogLine::parse(INFO);
        assert_eq!(line.time, "2026-10-24T07:31:02.123-04:00");
        assert_eq!(line.level, Level::Info);
        assert_eq!(&INFO[line.level_range.clone()], "INFO");
        assert_eq!(
            line.target_range.clone().map(|range| &INFO[range]),
            Some("pwrroute::link")
        );
        assert_eq!(line.target, "pwrroute::link");
        assert_eq!(line.message, "link 42 outcome=opened");
        assert_eq!(line.raw().as_ref(), INFO);
        assert_eq!(LogLine::parse(WARN).level, Level::Warn);
        assert_eq!(LogLine::parse(WARN).raw().as_ref(), WARN);

        let odd = LogLine::parse("  at some continuation");
        assert_eq!(odd.level, Level::Info);
        assert_eq!(odd.message, "  at some continuation");
        assert_eq!(odd.raw().as_ref(), "  at some continuation");
        assert_eq!(
            LogLine::parse("2026-10-24T07:31:02 TRACE gpui: x").level,
            Level::Debug
        );
    }

    #[test]
    fn levels_and_words_filter_the_lines() {
        let mut view = LogView::default();
        view.replace(&format!(
            "{INFO}\n{WARN}\n2026-10-24T07:31:04.000-04:00 DEBUG pwrroute::router: reload\n"
        ));
        assert_eq!(view.lines.len(), 3);
        // Debug is off by default.
        assert_eq!(view.visible(), [0, 1]);
        assert_eq!(view.summary(), "2 of 3 lines shown");
        view.toggle_level(Level::Debug);
        assert_eq!(view.visible(), [0, 1, 2]);
        assert_eq!(view.summary(), "3 lines");
        view.set_query("LINK 42");
        assert_eq!(view.visible(), [0]);
        view.set_query("");
        view.toggle_level(Level::Info);
        assert_eq!(view.visible(), [1, 2]);
        assert_eq!(view.count(Level::Warn), 1);
    }

    #[test]
    fn the_view_keeps_the_newest_lines() {
        let mut view = LogView::default();
        let text: String = (0..MAX_LINES + 10)
            .map(|n| format!("2026-10-24T07:31:02 INFO  pwrroute: line {n}\n"))
            .collect();
        view.replace(&text);
        assert_eq!(view.lines.len(), MAX_LINES);
        assert!(view.truncated);
        assert_eq!(view.first_number, 11);
        assert_eq!(view.lines[0].message, "line 10");
    }

    #[test]
    fn the_tail_follows_appends_partial_lines_and_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.log");
        std::fs::write(dir.path().join("main.old.log"), "old\n").unwrap();
        std::fs::write(&path, "one\ntw").unwrap();
        let mut tail = Tail::new(path.clone());
        assert_eq!(tail.read_all().unwrap(), "old\none\n");
        assert_eq!(tail.poll().unwrap(), Read::Nothing);

        let append = |text: &str| {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            file.write_all(text.as_bytes()).unwrap();
        };
        append("o\nthree\n");
        assert_eq!(tail.poll().unwrap(), Read::More("two\nthree\n".to_owned()));

        // Rotated: the old file is what main.log was, main.log starts over.
        std::fs::rename(&path, dir.path().join("main.old.log")).unwrap();
        std::fs::write(&path, "four\n").unwrap();
        assert_eq!(
            tail.poll().unwrap(),
            Read::All("one\ntwo\nthree\nfour\n".to_owned())
        );
    }

    #[test]
    fn counts_follow_the_lines_kept() {
        let mut view = LogView::default();
        assert_eq!(view.append(&format!("{INFO}\n{WARN}\n")), 0);
        let error = "2026-10-24T07:31:05.000-04:00 ERROR pwrroute::link: failed";
        assert_eq!(view.append(&format!("{error}\n")), 1);
        assert_eq!(view.count(Level::Error), 1);
        assert_eq!(view.count(Level::Warn), 1);
        let more: String = (0..MAX_LINES)
            .map(|n| format!("2026-10-24T07:31:06 INFO  pwrroute: line {n}\n"))
            .collect();
        view.append(&more);
        assert_eq!(view.count(Level::Error), 0);
        assert_eq!(view.count(Level::Warn), 0);
        assert_eq!(view.count(Level::Info), MAX_LINES);
    }

    #[test]
    fn just_after_a_rotation_the_old_file_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.log");
        std::fs::write(dir.path().join("main.old.log"), "old\n").unwrap();
        let mut tail = Tail::new(path.clone());
        assert_eq!(tail.read_all().unwrap(), "old\n");
        std::fs::write(&path, "new\n").unwrap();
        assert_eq!(tail.poll().unwrap(), Read::All("old\nnew\n".to_owned()));
        // With neither file, it is an error the window shows.
        let mut none = Tail::new(dir.path().join("nothing/main.log"));
        assert!(none.read_all().is_err());
    }

    #[test]
    fn numbers_read_with_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
