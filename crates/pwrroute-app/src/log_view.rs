//! The Logs window's model: `main.log`'s lines, parsed, filtered and
//! followed. Plain data and file reads; `logs_window.rs` draws it.
//!
//! The file is the source, so the window shows lines from earlier launches
//! and from the other processes that write it (`diag_log`), and the log
//! never carries a URL to begin with. On open the window reads
//! `main.old.log` and `main.log`, keeps the last [`MAX_LINES`], then reads
//! only what was appended; a file that shrank was rotated, and is read
//! again from the top.
//!
//! What the window shows is [`LogView::rows`]: the lines the levels let
//! through, and with a search in Filter mode only the matches, the context
//! around them and a gap row for each run hidden between; the mark, when
//! there is one, is a row of its own after the line it was dropped on.

use std::collections::BTreeSet;

use gpui_kit::SharedString;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{self, Read as _, Seek as _, SeekFrom},
    path::{Path, PathBuf},
};

/// Lines kept in the window, as the family's 5000-entry buffer.
pub const MAX_LINES: usize = 5000;

/// The Context choices: lines kept on each side of a match.
pub const CONTEXT_CHOICES: [usize; 4] = [0, 1, 3, 5];

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

/// What a search does to the lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchMode {
    /// Only the matching lines stay, with their context.
    #[default]
    Filter,
    /// Every line stays and the matches are marked.
    Highlight,
}

impl SearchMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Filter => "filter",
            Self::Highlight => "highlight",
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
    /// Characters in `raw`: how wide the line is unwrapped, in a mono face.
    chars: usize,
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
                chars: 0,
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
            chars: 0,
        });
        parsed.lower = line.to_lowercase();
        parsed.chars = line.chars().count();
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

/// How a line row is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shown {
    /// No search, or a line that does not match in Highlight mode.
    Plain,
    /// It matches the search.
    Match,
    /// Kept around a match, or shown from an opened gap.
    Context,
}

/// One row of the viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// A line, by index into [`LogView::lines`].
    Line { index: usize, shown: Shown },
    /// Lines hidden between matches: the first one's number and how many.
    Gap { first: usize, count: usize },
    /// The mark.
    Mark,
}

/// A row by what it shows, not where: the same key on two reads is the same
/// row, drawn the same height. Line numbers outlive the lines dropped from
/// the front, so the list keeps its measurements across a poll.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RowKey {
    Line { number: usize, shown: Shown },
    Gap { first: usize, count: usize },
    Mark { after: usize },
}

/// A mark in the view, never in the file: after line `after`, dropped at
/// `label` ("12:39:00").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub after: usize,
    pub label: String,
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
    pub mode: SearchMode,
    /// Lines kept on each side of a match in Filter mode.
    pub context: usize,
    query_words: Vec<String>,
    /// Indexes into `lines` that pass the levels and match the search.
    matches: Vec<usize>,
    /// The current match, by number.
    active: Option<usize>,
    /// Gaps opened by a click, by their first line's number.
    opened: BTreeSet<usize>,
    mark: Option<Mark>,
    rows: Vec<Row>,
    /// Indexes into `lines` of the line rows, in order.
    visible: Vec<usize>,
    /// Characters in the longest line shown.
    widest: usize,
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
            mode: SearchMode::Filter,
            context: 0,
            query_words: Vec::new(),
            matches: Vec::new(),
            active: None,
            opened: BTreeSet::new(),
            mark: None,
            rows: Vec::new(),
            visible: Vec::new(),
            widest: 0,
            counts: [0; 4],
        }
    }
}

impl LogView {
    /// An empty view with remembered settings: the levels shown, the
    /// search mode and the context.
    pub fn with(levels: [bool; 4], mode: SearchMode, context: usize) -> Self {
        Self {
            levels,
            mode,
            context,
            ..Self::default()
        }
    }

    /// Starts over with `text`, as read from the files. Numbers start
    /// again at 1, so the mark and the opened gaps go.
    pub fn replace(&mut self, text: &str) {
        self.lines.clear();
        self.counts = [0; 4];
        self.first_number = 1;
        self.truncated = false;
        self.mark = None;
        self.opened.clear();
        self.active = None;
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
        self.opened.clear();
        // A new search starts at its first match.
        self.active = None;
        self.refilter();
    }

    pub fn searching(&self) -> bool {
        !self.query_words.is_empty()
    }

    pub fn set_mode(&mut self, mode: SearchMode) {
        self.mode = mode;
        self.opened.clear();
        self.refilter();
    }

    pub fn set_context(&mut self, lines: usize) {
        self.context = lines;
        self.opened.clear();
        self.refilter();
    }

    pub fn toggle_level(&mut self, level: Level) {
        let slot = &mut self.levels[level as usize];
        *slot = !*slot;
        self.refilter();
    }

    pub fn shows(&self, level: Level) -> bool {
        self.levels[level as usize]
    }

    /// Shows the lines of the gap that starts at line `first`.
    pub fn open_gap(&mut self, first: usize) {
        self.opened.insert(first);
        self.refilter();
    }

    /// The newest line's number, when there are lines.
    pub fn newest(&self) -> Option<usize> {
        (!self.lines.is_empty()).then(|| self.first_number + self.lines.len() - 1)
    }

    /// Drops the mark after the newest line, replacing any other.
    pub fn set_mark(&mut self, label: impl Into<String>) {
        let after = self.newest().unwrap_or(self.first_number - 1);
        self.mark = Some(Mark {
            after,
            label: label.into(),
        });
        self.refilter();
    }

    pub fn clear_mark(&mut self) {
        self.mark = None;
        self.refilter();
    }

    pub fn mark(&self) -> Option<&Mark> {
        self.mark.as_ref()
    }

    /// The lines after the mark, by number, when any came in.
    pub fn since_mark(&self) -> Option<(usize, usize)> {
        let mark = self.mark.as_ref()?;
        let newest = self.newest()?;
        (newest > mark.after).then(|| ((mark.after + 1).max(self.first_number), newest))
    }

    fn refilter(&mut self) {
        let levels = self.levels;
        let shown: Vec<usize> = (0..self.lines.len())
            .filter(|&index| levels[self.lines[index].level as usize])
            .collect();
        let searching = self.searching();
        self.matches = if searching {
            shown
                .iter()
                .copied()
                .filter(|&index| self.lines[index].matches(&self.query_words))
                .collect()
        } else {
            Vec::new()
        };
        let mut rows = Vec::with_capacity(shown.len() + 2);
        if !searching {
            rows.extend(shown.iter().map(|&index| Row::Line {
                index,
                shown: Shown::Plain,
            }));
        } else if self.mode == SearchMode::Highlight {
            let mut matches = self.matches.iter().peekable();
            for &index in &shown {
                let matched = matches.next_if(|&&m| m == index).is_some();
                rows.push(Row::Line {
                    index,
                    shown: if matched { Shown::Match } else { Shown::Plain },
                });
            }
        } else {
            self.filter_rows(&shown, &mut rows);
        }
        if let Some(mark) = &self.mark {
            let first = self.first_number;
            let at = rows
                .iter()
                .position(|row| match row {
                    Row::Line { index, .. } => first + index > mark.after,
                    Row::Gap { first, .. } => *first > mark.after,
                    Row::Mark => false,
                })
                .unwrap_or(rows.len());
            rows.insert(at, Row::Mark);
        }
        self.visible = rows
            .iter()
            .filter_map(|row| match row {
                Row::Line { index, .. } => Some(*index),
                _ => None,
            })
            .collect();
        self.widest = self
            .visible
            .iter()
            .map(|&index| self.lines[index].chars)
            .max()
            .unwrap_or(0);
        self.rows = rows;
        self.keep_active();
    }

    /// Filter mode's rows: the matches, `context` lines around each, and a
    /// gap for every run between, unless that gap was opened.
    fn filter_rows(&self, shown: &[usize], rows: &mut Vec<Row>) {
        let mut keep = vec![false; shown.len()];
        let mut matched = vec![false; shown.len()];
        let mut matches = self.matches.iter().peekable();
        for (at, &index) in shown.iter().enumerate() {
            if matches.next_if(|&&m| m == index).is_some() {
                matched[at] = true;
                let from = at.saturating_sub(self.context);
                let to = (at + self.context).min(shown.len() - 1);
                keep[from..=to].iter_mut().for_each(|k| *k = true);
            }
        }
        if self.matches.is_empty() {
            // Nothing matched: no rows, and the window says so.
            return;
        }
        let mut at = 0;
        while at < shown.len() {
            if keep[at] {
                rows.push(Row::Line {
                    index: shown[at],
                    shown: if matched[at] {
                        Shown::Match
                    } else {
                        Shown::Context
                    },
                });
                at += 1;
                continue;
            }
            let start = at;
            while at < shown.len() && !keep[at] {
                at += 1;
            }
            let first = self.first_number + shown[start];
            if self.opened.contains(&first) {
                rows.extend(shown[start..at].iter().map(|&index| Row::Line {
                    index,
                    shown: Shown::Context,
                }));
            } else {
                rows.push(Row::Gap {
                    first,
                    count: at - start,
                });
            }
        }
    }

    /// The current match stays on its line while it matches; else it moves
    /// to the next match after it, or the last.
    fn keep_active(&mut self) {
        let first = self.first_number;
        let numbers = || self.matches.iter().map(|index| first + index);
        self.active = match self.active {
            _ if self.matches.is_empty() => None,
            None => numbers().next(),
            Some(active) => numbers()
                .find(|&number| number >= active)
                .or_else(|| numbers().next_back()),
        };
    }

    /// The rows the viewport shows, in order.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn row_key(&self, row: Row) -> RowKey {
        match row {
            Row::Line { index, shown } => RowKey::Line {
                number: self.first_number + index,
                shown,
            },
            Row::Gap { first, count } => RowKey::Gap { first, count },
            Row::Mark => RowKey::Mark {
                after: self.mark.as_ref().map_or(0, |mark| mark.after),
            },
        }
    }

    /// The row that shows line `number`, when one does.
    pub fn row_of(&self, number: usize) -> Option<usize> {
        let index = number.checked_sub(self.first_number)?;
        self.rows
            .iter()
            .position(|row| matches!(row, Row::Line { index: i, .. } if *i == index))
    }

    /// The shown lines' indexes into `lines`.
    pub fn visible(&self) -> &[usize] {
        &self.visible
    }

    /// Characters in the longest line shown.
    pub fn widest(&self) -> usize {
        self.widest
    }

    /// Lines that match the search.
    pub fn match_count(&self) -> usize {
        self.matches.len()
    }

    /// The current match's number.
    pub fn active(&self) -> Option<usize> {
        self.active
    }

    /// The current match as "n of N": its place among the matches, from 1.
    pub fn active_place(&self) -> Option<usize> {
        let index = self.active?.checked_sub(self.first_number)?;
        self.matches.binary_search(&index).ok().map(|at| at + 1)
    }

    /// Moves to the next match (or the previous), around the ends. Returns
    /// its number.
    pub fn step(&mut self, forward: bool) -> Option<usize> {
        let count = self.matches.len();
        if count == 0 {
            return None;
        }
        let at = self.active_place().map_or(0, |place| place - 1);
        let next = if forward {
            (at + 1) % count
        } else {
            (at + count - 1) % count
        };
        self.active = Some(self.first_number + self.matches[next]);
        self.active
    }

    /// Lines at `level` among all kept, whatever the filter.
    pub fn count(&self, level: Level) -> usize {
        self.counts[level as usize]
    }

    /// "1,204 lines", "12 of 1,204 lines shown" while the levels hide
    /// some, "4 of 1,204 lines match" while searching; " · 1 mark" with a
    /// mark down.
    pub fn summary(&self) -> String {
        let total = self.lines.len();
        let shown = self.visible.len();
        let lines = plural(total, "line", "lines");
        let mut summary = if self.searching() {
            format!(
                "{} of {} {lines} match",
                thousands(self.matches.len()),
                thousands(total)
            )
        } else if shown == total {
            format!("{} {lines}", thousands(total))
        } else {
            format!("{} of {} {lines} shown", thousands(shown), thousands(total))
        };
        if self.mark.is_some() {
            summary.push_str(" \u{00b7} 1 mark");
        }
        summary
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
    /// `main.log` was not there at the last full read: read everything
    /// again once it is (Windows has no inode to tell a new file by).
    missing: bool,
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
            missing: false,
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
                self.missing = false;
                file.read_to_end(&mut current)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound && !old.is_empty() => {
                self.identity = None;
                self.missing = true;
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
        if self.missing || len < self.offset || identity(&metadata) != self.identity {
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

    /// Twelve lines, every fourth an error that says "failed".
    fn twelve() -> LogView {
        let mut view = LogView::default();
        let text: String = (1..=12)
            .map(|n| {
                if n % 4 == 0 {
                    format!("2026-10-24T07:31:02 ERROR pwrroute::link: link {n} failed\n")
                } else {
                    format!("2026-10-24T07:31:02 INFO  pwrroute::link: link {n} opened\n")
                }
            })
            .collect();
        view.replace(&text);
        view
    }

    fn keys(view: &LogView) -> Vec<RowKey> {
        view.rows().iter().map(|row| view.row_key(*row)).collect()
    }

    #[test]
    fn filter_keeps_matches_and_context_and_gaps_the_rest() {
        let mut view = twelve();
        view.set_query("failed");
        assert_eq!(view.match_count(), 3);
        assert_eq!(
            keys(&view),
            [
                RowKey::Gap { first: 1, count: 3 },
                RowKey::Line {
                    number: 4,
                    shown: Shown::Match
                },
                RowKey::Gap { first: 5, count: 3 },
                RowKey::Line {
                    number: 8,
                    shown: Shown::Match
                },
                RowKey::Gap { first: 9, count: 3 },
                RowKey::Line {
                    number: 12,
                    shown: Shown::Match
                },
            ]
        );
        assert_eq!(view.summary(), "3 of 12 lines match");

        // ±1 keeps a line on each side; the gaps shrink.
        view.set_context(1);
        let line = |number, shown| RowKey::Line { number, shown };
        use Shown::{Context, Match};
        assert_eq!(
            keys(&view),
            [
                RowKey::Gap { first: 1, count: 2 },
                line(3, Context),
                line(4, Match),
                line(5, Context),
                RowKey::Gap { first: 6, count: 1 },
                line(7, Context),
                line(8, Match),
                line(9, Context),
                RowKey::Gap {
                    first: 10,
                    count: 1
                },
                line(11, Context),
                line(12, Match),
            ]
        );
        assert_eq!(view.visible().len(), 8);

        // A clicked gap shows its lines, as context.
        view.open_gap(1);
        assert_eq!(keys(&view)[..2], [line(1, Context), line(2, Context)]);
        // A new search closes it again.
        view.set_query("link 8");
        assert_eq!(view.match_count(), 1);
        assert_eq!(keys(&view)[0], RowKey::Gap { first: 1, count: 6 });

        // Nothing matches: no rows at all.
        view.set_query("nowhere");
        assert!(view.rows().is_empty());
        assert_eq!(view.summary(), "0 of 12 lines match");
    }

    #[test]
    fn highlight_keeps_every_line_and_marks_the_matches() {
        let mut view = twelve();
        view.set_mode(SearchMode::Highlight);
        view.set_query("failed");
        assert_eq!(view.rows().len(), 12);
        assert_eq!(
            keys(&view)[3],
            RowKey::Line {
                number: 4,
                shown: Shown::Match
            }
        );
        assert_eq!(
            keys(&view)[4],
            RowKey::Line {
                number: 5,
                shown: Shown::Plain
            }
        );
        // Levels still apply.
        view.toggle_level(Level::Info);
        assert_eq!(view.rows().len(), 3);
    }

    #[test]
    fn the_current_match_steps_around_and_stays_on_its_line() {
        let mut view = twelve();
        view.set_query("failed");
        assert_eq!((view.active(), view.active_place()), (Some(4), Some(1)));
        assert_eq!(view.step(true), Some(8));
        assert_eq!(view.step(true), Some(12));
        assert_eq!(view.step(true), Some(4));
        assert_eq!(view.step(false), Some(12));
        assert_eq!(view.active_place(), Some(3));
        // New lines do not move it.
        view.append("2026-10-24T07:31:03 ERROR pwrroute::link: link 13 failed\n");
        assert_eq!((view.active(), view.active_place()), (Some(12), Some(3)));
        assert_eq!(view.match_count(), 4);
        // Hidden by a level with nothing left, there is none; shown
        // again, the search starts over at its first match.
        view.toggle_level(Level::Error);
        assert_eq!(view.active(), None);
        view.toggle_level(Level::Error);
        assert_eq!(view.active(), Some(4));
        view.set_query("");
        assert_eq!(view.active(), None);
        assert_eq!(view.step(true), None);
    }

    #[test]
    fn the_mark_sits_after_the_newest_line_and_selects_what_came_after() {
        let mut view = twelve();
        assert_eq!(view.since_mark(), None);
        view.set_mark("12:39:00");
        assert_eq!(view.mark().map(|mark| mark.after), Some(12));
        assert_eq!(view.rows().last(), Some(&Row::Mark));
        assert_eq!(view.since_mark(), None);
        assert_eq!(view.summary(), "12 lines \u{00b7} 1 mark");
        view.append(
            "2026-10-24T07:31:03 INFO  pwrroute::link: link 13 opened\n\
             2026-10-24T07:31:03 INFO  pwrroute::link: link 14 opened\n",
        );
        assert_eq!(view.rows()[12], Row::Mark);
        assert_eq!(view.since_mark(), Some((13, 14)));
        // A second mark replaces the first.
        view.set_mark("12:40:00");
        assert_eq!(
            view.rows().iter().filter(|row| **row == Row::Mark).count(),
            1
        );
        assert_eq!(view.rows().last(), Some(&Row::Mark));
        // Filtered, it sits before the first row after it.
        view.set_mark("12:41:00");
        view.set_query("failed");
        assert_eq!(view.rows().last(), Some(&Row::Mark));
        view.clear_mark();
        assert_eq!(view.mark(), None);
        // A rotation starts the numbers over, and the mark goes with them.
        view.set_mark("12:42:00");
        view.replace("2026-10-24T07:31:03 INFO  pwrroute: one\n");
        assert_eq!(view.mark(), None);
    }

    #[test]
    fn row_keys_survive_lines_dropped_from_the_front() {
        let mut view = LogView::default();
        let text: String = (0..MAX_LINES)
            .map(|n| format!("2026-10-24T07:31:02 INFO  pwrroute: line {n}\n"))
            .collect();
        view.replace(&text);
        let before = keys(&view);
        view.append("2026-10-24T07:31:02 INFO  pwrroute: one more\n");
        let after = keys(&view);
        assert_eq!(after[..MAX_LINES - 1], before[1..]);
        assert_eq!(view.row_of(2), Some(0));
        assert_eq!(view.row_of(1), None);
        assert_eq!(
            view.widest(),
            "2026-10-24T07:31:02 INFO  pwrroute: line 4999".len()
        );
    }
}
