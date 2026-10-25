//! Help › Logs: `main.log`, live, in its own window. The family's viewer
//! (PwrAgent's "Log Viewer Workflows UX Review" in Claude Design): search
//! that filters (with context and gap rows) or highlights, previous and
//! next match, the level toggles with Error and Warning counts, Wrap with a
//! hanging indent, Mark and Select since mark, Live / Paused with a "new
//! lines" pill, line selection in the number gutter with Copy (and the
//! diagnostics header), and the file's Copy path and Reveal. Wrap, the
//! search mode, Context, Include diagnostics and the levels but Debug are
//! remembered in `settings.json` (`settings_store::LogsSettings`).
//!
//! The rows are a GPUI `list`, measured row by row: a wrapped line is as
//! tall as its text. GPUI's text has no hanging indent, so a wrapped line
//! is cut into rows here ([`LogsWindow::segments`]) and each continuation
//! is drawn 2ch in. Unwrapped, every row is one line high and the text
//! scrolls sideways under a fixed gutter.
//!
//! The log never carries a URL (`diag_log`), so nothing here filters one,
//! and nothing here writes a line anywhere but the clipboard. The model,
//! parsing and the file reads are `log_view.rs`.

use std::ops::Range;
use std::time::Duration;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AnyView, App, ClipboardItem, Context, FocusHandle, Font, HighlightStyle, Hsla,
    InteractiveElement as _, IntoElement, KeyDownEvent, LineFragment, ListAlignment, ListOffset,
    ListState, MouseButton, MouseDownEvent, ParentElement as _, Render, ScrollWheelEvent,
    SharedString, StatefulInteractiveElement as _, Styled as _, StyledText, Window, WindowHandle,
    deferred, div, list, px,
};

use crate::aux_window::{self, Frame};
use crate::log_view::{
    CONTEXT_CHOICES, Level, LogView, Read, Row, RowKey, SearchMode, Shown, Tail, file_name,
    thousands,
};
use crate::settings_store::{LogLevels, LogsSettings};
use crate::text_field::{self, Edit, Look, TextField};
use crate::theme::{self, icon, mono, text};
use crate::widgets::{ButtonKind, button, menu_card, menu_row, segment, segmented};

/// How often the file is read for new lines.
const POLL_EVERY: Duration = Duration::from_millis(500);
/// How long a status note ("Copied 3 lines") stays.
const NOTE_FOR: Duration = Duration::from_secs(3);
/// A row: 12px mono at the design's 1.55.
const ROW_HEIGHT: f32 = 19.0;
/// The text's size.
const TEXT_SIZE: f32 = 12.0;
/// The number gutter.
const GUTTER: f32 = 56.0;
/// The text's padding inside a row, left and right.
const TEXT_LEFT: f32 = 12.0;
const TEXT_RIGHT: f32 = 14.0;
/// The window body's side padding and the viewport's border: the list is
/// the window's width less these.
const BODY_PAD: f32 = 18.0;
const FRAME_BORDER: f32 = 1.0;
/// A wrapped line's continuation rows start this many characters in.
const HANGING_INDENT: f32 = 2.0;
/// A gap row and the mark's row.
const GAP_HEIGHT: f32 = 22.0;
const MARK_HEIGHT: f32 = 26.0;
/// Rows kept above a line the window scrolls to (a match, the line Esc
/// returns to), so what led to it shows.
const REVEAL_ABOVE: usize = 2;
/// Below this width Wrap and Mark keep their icons and tooltips only.
const NARROW_BELOW: f32 = 900.0;

pub const SEARCH_ID: &str = "logs-search";
pub const LIVE_ID: &str = "logs-live";
pub const COPY_PATH_ID: &str = "logs-copy-path";
pub const REVEAL_ID: &str = "logs-reveal";
pub const COPY_ID: &str = "logs-copy";
pub const PILL_ID: &str = "logs-new-lines";
pub const FILTER_ID: &str = "logs-mode-filter";
pub const HIGHLIGHT_ID: &str = "logs-mode-highlight";
pub const CONTEXT_ID: &str = "logs-context";
pub const WRAP_ID: &str = "logs-wrap";
pub const MARK_ID: &str = "logs-mark";
pub const PREV_ID: &str = "logs-prev-match";
pub const NEXT_ID: &str = "logs-next-match";
pub const SINCE_MARK_ID: &str = "logs-select-since-mark";
pub const CLEAR_MARK_ID: &str = "logs-clear-mark";

/// The minimum width holds the toolbar's one row with Context showing and
/// the labels dropped: the search at its least, every control, and room
/// for four-digit level counts.
const FRAME: Frame = Frame {
    title: "PwrRoute Logs",
    size: (1040.0, 760.0),
    min: (880.0, 500.0),
};

/// The status bar's hint while a search is active.
const SEARCH_HINT: &str = "Enter next \u{00b7} Shift-Enter previous \u{00b7} Esc clear";

/// Opens the Logs window, or brings it forward.
pub fn open(cx: &mut App) {
    let existing = cx
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<LogsWindow>());
    if aux_window::activate(existing, cx) {
        return;
    }
    let path = crate::diag_log::path().map(std::path::Path::to_path_buf);
    open_window(path, cx);
}

/// A new Logs window over `path` (`None`: there is no log file).
pub fn open_window(
    path: Option<std::path::PathBuf>,
    cx: &mut App,
) -> Option<WindowHandle<LogsWindow>> {
    let handle = aux_window::open(&FRAME, cx, |_, cx| LogsWindow::new(path, cx))?;
    let _ = handle.update(cx, |view, window, cx| {
        window.focus(&view.search.focus, cx);
        view.follow(cx);
    });
    Some(handle)
}

/// The Logs window.
pub struct LogsWindow {
    pub view: LogView,
    tail: Option<Tail>,
    /// Why there are no lines to show, when there are none.
    pub problem: Option<String>,
    search: TextField,
    focus: FocusHandle,
    list: ListState,
    /// The rows the list was last told about, and the model's generation
    /// they came from.
    list_keys: Vec<RowKey>,
    list_generation: Option<u64>,
    /// New lines scroll into view.
    pub live: bool,
    /// Lines and errors that came in while paused.
    pub new_lines: usize,
    new_errors: usize,
    /// Selected lines, by number (`LogView::first_number` counting): the
    /// anchor and the other end.
    pub selection: Option<(usize, usize)>,
    /// The selection's far end moves with new lines: Select since mark,
    /// or a Shift-click that reached the newest line. A plain click or ⌘A
    /// stays put.
    follows_tail: bool,
    pub include_diagnostics: bool,
    /// Long lines wrap; off, they scroll sideways.
    pub wrap: bool,
    /// With Wrap off: how far the text is scrolled left, and how far it can
    /// go.
    pub h_offset: f32,
    h_max: f32,
    /// The mono face's advance and the room a line's text has, from the
    /// last frame: what [`Self::segments`] wraps to.
    char_width: f32,
    text_width: f32,
    context_open: bool,
    /// Whether the Context list was open when the press that may close it
    /// began, so the press on its button does not open it again.
    context_was_open: bool,
    note: Option<(SharedString, u64)>,
    note_generation: u64,
}

impl LogsWindow {
    pub fn new(path: Option<std::path::PathBuf>, cx: &mut Context<'_, Self>) -> Self {
        let prefs = crate::settings_store::store(cx)
            .map(|store| store.read(cx).settings().logs.clone())
            .unwrap_or_default();
        // Debug is never remembered: every window opens without it.
        let view = LogView::with(
            [
                prefs.levels.error,
                prefs.levels.warning,
                prefs.levels.info,
                false,
            ],
            prefs.search_mode,
            prefs.context(),
        );
        let mut this = Self {
            view,
            tail: path.map(Tail::new),
            problem: None,
            search: TextField::new("", cx),
            focus: cx.focus_handle(),
            list: ListState::new(0, ListAlignment::Top, px(240.0)),
            list_keys: Vec::new(),
            list_generation: None,
            live: true,
            new_lines: 0,
            new_errors: 0,
            selection: None,
            follows_tail: false,
            include_diagnostics: prefs.include_diagnostics,
            wrap: prefs.wrap,
            h_offset: 0.0,
            h_max: 0.0,
            char_width: 7.2,
            text_width: 800.0,
            context_open: false,
            context_was_open: false,
            note: None,
            note_generation: 0,
        };
        this.reload();
        this
    }

    /// Reads everything again.
    fn reload(&mut self) {
        let Some(tail) = &mut self.tail else {
            self.problem = Some(
                "No log file: the OS gave PwrRoute no log folder. Messages go to the terminal \
                 that launched it, if any."
                    .to_owned(),
            );
            return;
        };
        match tail.read_all() {
            Ok(text) => {
                self.view.replace(&text);
                self.problem = None;
            }
            Err(error) => {
                self.problem = Some(format!("Could not read the log: {error}."));
            }
        }
        self.sync_list();
        self.list.scroll_to_end();
    }

    /// Reads the file every [`POLL_EVERY`] while the window is open.
    fn follow(&self, cx: &mut Context<'_, Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_EVERY).await;
                if this.update(cx, |this, cx| this.poll(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn poll(&mut self, cx: &mut Context<'_, Self>) {
        let Some(tail) = &mut self.tail else {
            return;
        };
        let read = match tail.poll() {
            Ok(read) => read,
            // Not there yet (nothing logged since a rotation): keep what
            // is shown.
            Err(_) => return,
        };
        match read {
            Read::Nothing => return,
            Read::All(text) => {
                self.view.replace(&text);
                self.problem = None;
                self.selection = None;
            }
            Read::More(text) => {
                let newest = self.view.newest();
                let before = self.view.lines.len() + self.view.first_number;
                let errors = self.view.append(&text);
                if !self.live {
                    let added = (self.view.lines.len() + self.view.first_number) - before;
                    self.new_lines += added;
                    self.new_errors += errors;
                }
                self.follow_selection(newest);
            }
        }
        self.sync_list();
        cx.notify();
    }

    /// A following selection that ended on the newest line grows with the
    /// lines that came after it, so Copy at the end of a repro takes them
    /// too.
    fn follow_selection(&mut self, newest_before: Option<usize>) {
        let (true, Some((anchor, end)), Some(before), Some(newest)) = (
            self.follows_tail,
            self.selection,
            newest_before,
            self.view.newest(),
        ) else {
            return;
        };
        if end == before {
            self.selection = Some((anchor, newest));
        }
    }

    /// Tells the list which rows changed since it last drew: lines dropped
    /// from the front, then what differs between the rows both share at
    /// the start and the end. Rows it already measured keep their height.
    fn sync_list(&mut self) {
        let generation = self.view.generation();
        if self.list_generation == Some(generation) {
            return;
        }
        self.list_generation = Some(generation);
        let keys: Vec<RowKey> = self
            .view
            .rows()
            .iter()
            .map(|row| self.view.row_key(*row))
            .collect();
        if keys == self.list_keys {
            return;
        }
        let mut old = std::mem::take(&mut self.list_keys);
        // Paused, the reader stays on the line at the top (or the next one
        // still shown), whatever was spliced around or under it.
        let scroll_top = self.list.logical_scroll_top();
        let top = (!self.live)
            .then(|| {
                old.get(scroll_top.item_ix.min(old.len())..)?
                    .iter()
                    .find_map(|key| match key {
                        RowKey::Line { number, .. } => Some(*number),
                        _ => None,
                    })
            })
            .flatten();
        if let (Some(first), Some(old_first)) = (keys.first(), old.first())
            && first != old_first
            && let Some(dropped) = old.iter().position(|key| key == first)
        {
            self.list.splice(0..dropped, 0);
            old.drain(..dropped);
        }
        let prefix = old.iter().zip(&keys).take_while(|(a, b)| a == b).count();
        let room = old.len().min(keys.len()) - prefix;
        let suffix = old
            .iter()
            .rev()
            .zip(keys.iter().rev())
            .take(room)
            .take_while(|(a, b)| a == b)
            .count();
        if prefix == 0 && suffix == 0 {
            self.list
                .reset_with_uniform_height(keys.len(), px(ROW_HEIGHT));
        } else {
            self.list
                .splice(prefix..old.len() - suffix, keys.len() - prefix - suffix);
        }
        if let Some(top) = top
            && let Some(row) = keys
                .iter()
                .position(|key| matches!(key, RowKey::Line { number, .. } if *number >= top))
        {
            let same = matches!(keys[row], RowKey::Line { number, .. } if number == top);
            self.list.scroll_to(ListOffset {
                item_ix: row,
                offset_in_item: if same {
                    scroll_top.offset_in_item
                } else {
                    px(0.0)
                },
            });
        }
        self.list_keys = keys;
        if self.live {
            self.list.scroll_to_end();
        }
    }

    /// Scrolls so row `row` is near the top, with a little above it.
    fn reveal_row(&self, row: usize) {
        self.list.scroll_to(ListOffset {
            item_ix: row.saturating_sub(REVEAL_ABOVE),
            offset_in_item: px(0.0),
        });
    }

    /// Scrolls to the current match.
    fn reveal_active(&self) {
        if let Some(row) = self
            .view
            .active()
            .and_then(|number| self.view.row_of(number))
        {
            self.reveal_row(row);
        }
    }

    pub fn set_live(&mut self, live: bool, cx: &mut Context<'_, Self>) {
        self.live = live;
        if live {
            self.new_lines = 0;
            self.new_errors = 0;
            self.list.scroll_to_end();
        }
        cx.notify();
    }

    /// Whether the list shows its last row's bottom.
    fn at_end(&self) -> bool {
        let count = self.list.item_count();
        if count == 0 {
            return true;
        }
        if let Some(end) = self.list.is_scrolled_to_end() {
            return end;
        }
        // Not scrollable, or some heights are not known yet.
        match self.list.bounds_for_item(count - 1) {
            Some(last) => last.bottom() <= self.list.viewport_bounds().bottom() + px(1.0),
            None => self.list.logical_scroll_top().item_ix >= count,
        }
    }

    // ---- The view's settings -----------------------------------------------

    /// What `settings.json` keeps of the view: never Debug, never the
    /// search.
    fn prefs(&self) -> LogsSettings {
        LogsSettings {
            wrap: self.wrap,
            search_mode: self.view.mode,
            context_lines: self.view.context,
            include_diagnostics: self.include_diagnostics,
            levels: LogLevels {
                error: self.view.shows(Level::Error),
                warning: self.view.shows(Level::Warn),
                info: self.view.shows(Level::Info),
            },
        }
    }

    fn save_prefs(&self, cx: &mut Context<'_, Self>) {
        let Some(store) = crate::settings_store::store(cx) else {
            return;
        };
        let logs = self.prefs();
        store.update(cx, |store, _| {
            if store.settings().logs == logs {
                return;
            }
            if let Err(error) = store.update(|settings| settings.logs = logs) {
                log::warn!("cannot save the Logs window's view: {error:#}");
            }
        });
    }

    pub fn set_wrap(&mut self, wrap: bool, cx: &mut Context<'_, Self>) {
        self.wrap = wrap;
        self.h_offset = 0.0;
        // Every row's height changes.
        self.list.remeasure();
        if self.live {
            self.list.scroll_to_end();
        }
        self.save_prefs(cx);
        cx.notify();
    }

    pub fn set_mode(&mut self, mode: SearchMode, cx: &mut Context<'_, Self>) {
        self.view.set_mode(mode);
        self.sync_list();
        self.reveal_active();
        self.save_prefs(cx);
        cx.notify();
    }

    pub fn set_context(&mut self, lines: usize, cx: &mut Context<'_, Self>) {
        self.context_open = false;
        self.view.set_context(lines);
        self.sync_list();
        self.reveal_active();
        self.save_prefs(cx);
        cx.notify();
    }

    pub fn toggle_level(&mut self, level: Level, cx: &mut Context<'_, Self>) {
        self.view.toggle_level(level);
        self.sync_list();
        self.save_prefs(cx);
        cx.notify();
    }

    fn toggle_diagnostics(&mut self, cx: &mut Context<'_, Self>) {
        self.include_diagnostics = !self.include_diagnostics;
        self.save_prefs(cx);
        cx.notify();
    }

    // ---- Search -------------------------------------------------------------

    fn query_changed(&mut self) {
        self.view.set_query(&self.search.text);
        // Reading a search's results, not the tail.
        if self.view.searching() {
            self.live = false;
        }
        self.sync_list();
        self.reveal_active();
    }

    /// Esc: the search goes and the window stays on the line that was the
    /// current match, now among all the others.
    fn clear_search(&mut self) {
        let back = self.view.active();
        self.search.set("");
        self.view.set_query("");
        self.sync_list();
        if !self.live
            && let Some(row) = back.and_then(|number| self.view.row_of(number))
        {
            self.reveal_row(row);
        }
    }

    /// Enter and Shift-Enter, and the field's chevrons.
    pub fn step(&mut self, forward: bool, cx: &mut Context<'_, Self>) {
        if self.view.step(forward).is_none() {
            return;
        }
        self.live = false;
        self.reveal_active();
        cx.notify();
    }

    fn open_gap(&mut self, first: usize, cx: &mut Context<'_, Self>) {
        self.view.open_gap(first);
        self.sync_list();
        cx.notify();
    }

    // ---- Mark ---------------------------------------------------------------

    /// Drops the mark after the newest line, labelled with the time.
    pub fn mark(&mut self, cx: &mut Context<'_, Self>) {
        let now = chrono::Local::now().format("%H:%M:%S").to_string();
        self.view.set_mark(now);
        self.sync_list();
        cx.notify();
    }

    fn clear_mark(&mut self, cx: &mut Context<'_, Self>) {
        self.view.clear_mark();
        self.sync_list();
        cx.notify();
    }

    /// Selects every line after the mark; the selection then grows as new
    /// lines come in.
    pub fn select_since_mark(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(range) = self.view.since_mark() {
            self.selection = Some(range);
            self.follows_tail = true;
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    // ---- Selection and copy ---------------------------------------------------

    fn set_note(&mut self, note: impl Into<SharedString>, cx: &mut Context<'_, Self>) {
        self.note_generation += 1;
        let generation = self.note_generation;
        self.note = Some((note.into(), generation));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(NOTE_FOR).await;
            let _ = this.update(cx, |this, cx| {
                if this.note.as_ref().is_some_and(|(_, g)| *g == generation) {
                    this.note = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// The selected range, low to high, by number.
    fn selected(&self) -> Option<(usize, usize)> {
        self.selection.map(|(a, b)| (a.min(b), a.max(b)))
    }

    /// The shown lines inside the selection, as indexes into the view.
    fn selected_indexes(&self) -> Vec<usize> {
        let Some((low, high)) = self.selected() else {
            return Vec::new();
        };
        let first = self.view.first_number;
        self.view
            .visible()
            .iter()
            .copied()
            .filter(|index| (low..=high).contains(&(first + index)))
            .collect()
    }

    fn click_number(&mut self, number: usize, extend: bool, cx: &mut Context<'_, Self>) {
        self.selection = match (self.selection, extend) {
            (Some((anchor, _)), true) => Some((anchor, number)),
            _ => Some((number, number)),
        };
        self.follows_tail = extend && self.view.newest() == Some(number);
        cx.notify();
    }

    fn select_all(&mut self, cx: &mut Context<'_, Self>) {
        let visible = self.view.visible();
        if let (Some(first), Some(last)) = (visible.first(), visible.last()) {
            let base = self.view.first_number;
            self.selection = Some((base + first, base + last));
            self.follows_tail = false;
            cx.notify();
        }
    }

    /// The selection on the clipboard, after the diagnostics lines when
    /// asked: what a bug report wants.
    fn copy_selection(&mut self, cx: &mut Context<'_, Self>) {
        let indexes = self.selected_indexes();
        if indexes.is_empty() {
            return;
        }
        let mut out = String::new();
        if self.include_diagnostics {
            out.push_str(&crate::help::diagnostics_report(cx));
            out.push_str(&self.copy_header(&indexes));
            out.push('\n');
        }
        for &index in &indexes {
            out.push_str(self.view.lines[index].raw());
            out.push('\n');
        }
        cx.write_to_clipboard(ClipboardItem::new_string(out));
        let count = indexes.len();
        let lines = if count == 1 { "line" } else { "lines" };
        let with = if self.include_diagnostics {
            " with diagnostics"
        } else {
            ""
        };
        self.set_note(format!("Copied {count} {lines}{with}"), cx);
    }

    /// "Log lines: 215–219 (5 of 1,204; levels: Error, Warning, Info)",
    /// and the search with its mode while one is active.
    fn copy_header(&self, indexes: &[usize]) -> String {
        let first = self.view.first_number;
        let low = indexes.first().map_or(0, |index| first + index);
        let high = indexes.last().map_or(0, |index| first + index);
        let levels: Vec<&str> = Level::ALL
            .into_iter()
            .filter(|level| self.view.shows(*level))
            .map(Level::label)
            .collect();
        let mut header = format!(
            "Log lines: {low}\u{2013}{high} ({} of {}; levels: {})\n",
            thousands(indexes.len()),
            thousands(self.view.lines.len()),
            levels.join(", ")
        );
        if self.view.searching() {
            let context = if self.view.mode == SearchMode::Filter && self.view.context > 0 {
                format!(", context \u{00b1}{}", self.view.context)
            } else {
                String::new()
            };
            header.push_str(&format!(
                "Log search: \"{}\" ({}{context})\n",
                self.search.text.trim(),
                self.view.mode.label()
            ));
        }
        header
    }

    fn copy_path(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(tail) = &self.tail {
            cx.write_to_clipboard(ClipboardItem::new_string(
                tail.path.to_string_lossy().into_owned(),
            ));
            self.set_note("Copied the log file path", cx);
        }
    }

    fn reveal(&self, cx: &mut Context<'_, Self>) {
        if let Some(tail) = &self.tail {
            cx.reveal_path(&tail.path);
        }
    }

    // ---- Keys and scrolling ----------------------------------------------------

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        let keystroke = &event.keystroke;
        let command = keystroke.modifiers.platform || keystroke.modifiers.control;
        if self.search.focus.is_focused(window) {
            if keystroke.key == "escape" && !self.search.text.is_empty() {
                self.clear_search();
                cx.stop_propagation();
                cx.notify();
                return;
            }
            match self.search.key(keystroke, cx) {
                Edit::Ignored => {}
                Edit::Changed => {
                    self.query_changed();
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                Edit::Submit => {
                    self.step(!keystroke.modifiers.shift, cx);
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                Edit::Handled => {
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
            }
        }
        match (command, keystroke.key.as_str()) {
            (true, "f") => window.focus(&self.search.focus, cx),
            (true, "a") => self.select_all(cx),
            (true, "c") => self.copy_selection(cx),
            (false, "escape") => {
                self.selection = None;
                self.context_open = false;
                cx.notify();
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// Types `query` into the search field: the `screens` example's way to
    /// draw a search.
    #[allow(dead_code, reason = "only the screens example searches this way")]
    pub fn set_query_for_screens(&mut self, query: &str) {
        self.search.set(query);
        self.query_changed();
    }

    /// Drops the mark with a fixed label: the `screens` example's clock.
    #[allow(dead_code, reason = "only the screens example marks this way")]
    pub fn set_mark_for_screens(&mut self, label: &str) {
        self.view.set_mark(label);
        self.sync_list();
    }

    /// The list scrolled first (its handler runs before this one); this
    /// pauses when that left the end, resumes when it reached it, and moves
    /// unwrapped text sideways.
    fn scrolled(&mut self, event: &ScrollWheelEvent, cx: &mut Context<'_, Self>) {
        let delta = event.delta.pixel_delta(px(ROW_HEIGHT));
        let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
        if !self.wrap && dx != 0.0 {
            let offset = (self.h_offset - dx).clamp(0.0, self.h_max);
            if offset != self.h_offset {
                self.h_offset = offset;
                cx.notify();
            }
        }
        if dy > 0.0 && self.live && !self.at_end() {
            // Scrolled up: stop following.
            self.live = false;
            cx.notify();
        } else if dy < 0.0
            && !self.live
            && !self.view.searching()
            && self.scrollable()
            && self.at_end()
        {
            // Scrolled back down to the end of a list that scrolls; one that
            // fits stays paused until Live or the pill.
            self.set_live(true, cx);
        }
    }

    /// Whether the rows are taller than the viewport.
    fn scrollable(&self) -> bool {
        self.list.max_offset_for_scrollbar().y > px(0.0)
    }

    /// The face the lines are drawn in, for measuring them.
    fn mono_font() -> Font {
        Font {
            weight: theme::weight(400.0),
            ..gpui_kit::font(theme::mono_family(400.0))
        }
    }

    /// Where `raw` breaks into rows when wrapped: byte ranges, one per row.
    /// Every row wraps to the room left after the hanging indent, so a
    /// continuation fits beside it.
    fn segments(&self, raw: &str, window: &Window) -> Vec<Range<usize>> {
        let width = self.text_width - self.char_width * (HANGING_INDENT + 1.0);
        if raw.is_empty() || width < self.char_width * 8.0 {
            return std::iter::once(0..raw.len()).collect();
        }
        let mut wrapper = window
            .text_system()
            .line_wrapper(Self::mono_font(), px(TEXT_SIZE));
        let fragments = [LineFragment::text(raw)];
        let mut starts = vec![0];
        starts.extend(
            wrapper
                .wrap_line(&fragments, px(width))
                .map(|boundary| boundary.ix),
        );
        starts.push(raw.len());
        starts.dedup();
        starts.windows(2).map(|pair| pair[0]..pair[1]).collect()
    }
}

// ---- Drawing --------------------------------------------------------------

fn level_color(level: Level) -> Hsla {
    match level {
        Level::Error => theme::danger_text(),
        Level::Warn => theme::status_warning(),
        Level::Info => theme::text_primary(),
        Level::Debug => theme::text_muted(),
    }
}

/// The app's tooltip, one line of text.
fn tip(label: &'static str) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    crate::overview::tip(label, None)
}

impl Render for LogsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        // A test or the screens example may change the view directly.
        self.sync_list();
        // The Context list closes with its select.
        if self.view.mode != SearchMode::Filter || !self.view.searching() {
            self.context_open = false;
        }
        let list_width = f32::from(window.viewport_size().width) - 2.0 * (BODY_PAD + FRAME_BORDER);
        self.text_width = (list_width - GUTTER - TEXT_LEFT - TEXT_RIGHT).max(0.0);
        let font_id = window.text_system().resolve_font(&Self::mono_font());
        if let Ok(advance) = window.text_system().advance(font_id, px(TEXT_SIZE), 'm') {
            self.char_width = f32::from(advance.width);
        }
        self.h_max = (self.view.widest() as f32 * self.char_width - self.text_width).max(0.0);
        self.h_offset = self.h_offset.min(self.h_max);
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.key(event, window, cx);
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::bg_app())
            .text_color(theme::text_primary())
            .font_family(theme::SANS)
            .child(aux_window::strip("Logs"))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .pt(px(14.0))
                    .px(px(BODY_PAD))
                    .child(self.toolbar(window, cx))
                    .child(self.viewport(cx)),
            )
            .child(self.status_bar(cx))
    }
}

impl LogsWindow {
    fn toolbar(&self, window: &Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let narrow = f32::from(window.viewport_size().width) < NARROW_BELOW;
        let search = text_field::field_with(
            SEARCH_ID,
            &self.search,
            Look {
                mono: false,
                placeholder: "Search logs",
            },
            Some(icon("pwrroute/search.svg", 13.0, theme::text_subtle()).into_any_element()),
            Some(self.search_trailing(narrow, cx)),
            window,
        )
        .on_click(cx.listener(|this, _, window, cx| {
            window.focus(&this.search.focus, cx);
            cx.notify();
        }));
        let mode = [
            (
                SearchMode::Filter,
                FILTER_ID,
                "Filter",
                "Show only matching lines",
            ),
            (
                SearchMode::Highlight,
                HIGHLIGHT_ID,
                "Highlight",
                "Show every line and highlight matches",
            ),
        ]
        .into_iter()
        .fold(segmented(), |seg, (mode, id, label, hint)| {
            seg.child(
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .child(segment(label, self.view.mode == mode))
                    .tooltip(tip(hint))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_mode(mode, cx))),
            )
        });
        let levels = Level::ALL.into_iter().fold(segmented(), |seg, level| {
            let on = self.view.shows(level);
            let count = match level {
                Level::Error | Level::Warn => Some(self.view.count(level)),
                _ => None,
            };
            let label = match count {
                Some(count) if count > 0 => format!("{} {}", level.label(), thousands(count)),
                _ => level.label().to_owned(),
            };
            seg.child(
                div()
                    .id(SharedString::from(format!("logs-level-{}", level.label())))
                    .debug_selector(move || format!("logs-level-{}", level.label()))
                    .child(segment(label, on))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_level(level, cx))),
            )
        });
        let (dot, label) = if self.live {
            (theme::success_text(), "Live")
        } else {
            (theme::text_muted(), "Paused")
        };
        let live = div()
            .id(LIVE_ID)
            .debug_selector(|| LIVE_ID.into())
            .flex()
            .flex_none()
            .items_center()
            .gap(px(7.0))
            .min_w(px(78.0))
            .h(px(32.0))
            .px(px(10.0))
            .border_1()
            .border_color(theme::border_default())
            .rounded(px(7.0))
            .bg(theme::bg_elevated())
            .cursor_pointer()
            .child(div().size(px(7.0)).rounded_full().bg(dot))
            .child(text(label, 11.5, 600.0, theme::text_secondary()))
            .tooltip(tip(if self.live {
                "Pause the live log"
            } else {
                "Resume the live log"
            }))
            .on_click(cx.listener(|this, _, _, cx| {
                let live = !this.live;
                this.set_live(live, cx);
            }));
        let wrap = tool_button(WRAP_ID, "pwrroute/wrap.svg", "Wrap", self.wrap, narrow)
            .tooltip(tip("Wrap long lines"))
            .on_click(cx.listener(|this, _, _, cx| {
                let wrap = !this.wrap;
                this.set_wrap(wrap, cx);
            }));
        let mark = tool_button(MARK_ID, "pwrroute/bookmark.svg", "Mark", false, narrow)
            .tooltip(tip("Drop a mark after the newest line"))
            .on_click(cx.listener(|this, _, _, cx| this.mark(cx)));
        let show_context = self.view.mode == SearchMode::Filter && self.view.searching();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .child(div().flex_1().min_w(px(200.0)).child(search))
            .child(mode)
            .when(show_context, |bar| {
                bar.child(self.context_select(narrow, cx))
            })
            .child(levels)
            .child(
                div()
                    .flex_none()
                    .w(px(1.0))
                    .h(px(20.0))
                    .bg(theme::border_subtle()),
            )
            .child(wrap)
            .child(mark)
            .child(live)
    }

    /// Inside the search field: "⌘F" while empty; the current match as "1
    /// of 4 lines" ("1/4" when narrow) and the previous and next chevrons
    /// while searching.
    fn search_trailing(&self, narrow: bool, cx: &mut Context<'_, Self>) -> AnyElement {
        if !self.view.searching() {
            let hint = if cfg!(target_os = "macos") {
                "\u{2318}F"
            } else {
                "Ctrl+F"
            };
            return mono(hint, 11.0, 400.0, theme::text_muted()).into_any_element();
        }
        let count = self.view.match_count();
        let label = match self.view.active_place() {
            Some(place) if narrow => format!("{place}/{}", thousands(count)),
            None if narrow => "0".to_owned(),
            Some(place) => format!(
                "{place} of {} {}",
                thousands(count),
                if count == 1 { "line" } else { "lines" }
            ),
            None => "No matches".to_owned(),
        };
        let step = |id: &'static str, path: &'static str, hint: &'static str, forward: bool| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .size(px(22.0))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .cursor_pointer()
                .hover(|step| step.bg(theme::bg_hover_strong()))
                .child(icon(path, 14.0, theme::text_secondary()))
                .tooltip(tip(hint))
                .on_click(cx.listener(move |this, _, _, cx| this.step(forward, cx)))
        };
        div()
            .flex()
            .items_center()
            .gap(px(2.0))
            .mr(px(-6.0))
            .child(
                div()
                    .mr(px(4.0))
                    .child(mono(label, 11.0, 400.0, theme::text_muted())),
            )
            .child(step(
                PREV_ID,
                "pwrroute/chevron-up.svg",
                "Previous match (Shift-Enter)",
                false,
            ))
            .child(step(
                NEXT_ID,
                "pwrroute/chevron-down.svg",
                "Next match (Enter)",
                true,
            ))
            .into_any_element()
    }

    /// Context: lines kept around each match, in Filter mode with a query.
    /// Narrow, the button reads "±1" alone.
    fn context_select(&self, narrow: bool, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let label = if narrow {
            format!("\u{00b1}{}", self.view.context)
        } else {
            context_label(self.view.context)
        };
        let menu = CONTEXT_CHOICES.into_iter().fold(
            menu_card("logs-context-menu")
                .absolute()
                .top(px(36.0))
                .left_0()
                .w(px(150.0))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.context_open = false;
                    cx.notify();
                })),
            |menu, lines| {
                menu.child(
                    menu_row(
                        SharedString::from(format!("logs-context-{lines}")),
                        None,
                        context_label(lines),
                        (lines == self.view.context).then(|| SharedString::from("\u{2713}")),
                    )
                    .debug_selector(move || format!("logs-context-{lines}"))
                    .hover(|row| row.bg(theme::bg_hover()))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_context(lines, cx))),
                )
            },
        );
        div()
            .relative()
            .flex_none()
            .child(
                div()
                    .id(CONTEXT_ID)
                    .debug_selector(|| CONTEXT_ID.into())
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .h(px(32.0))
                    .px(px(10.0))
                    .border_1()
                    .border_color(theme::border_default())
                    .rounded(px(7.0))
                    .bg(theme::bg_elevated())
                    .cursor_pointer()
                    .child(text(label, 11.5, 600.0, theme::text_secondary()))
                    .child(text("\u{25be}", 10.0, 400.0, theme::text_subtle()))
                    .tooltip(tip("Lines kept around each match"))
                    // Capture runs before the open list's press-outside
                    // handler closes it.
                    .capture_any_mouse_down(cx.listener(|this, _, _, _| {
                        this.context_was_open = this.context_open;
                    }))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.context_open = !this.context_was_open;
                        cx.notify();
                    })),
            )
            .when(self.context_open, |select| select.child(deferred(menu)))
    }

    fn viewport(&self, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let frame = div()
            .relative()
            .flex_1()
            .min_h_0()
            .border_1()
            .border_color(theme::border_subtle())
            .rounded(px(8.0))
            .bg(theme::bg_sidebar())
            .overflow_hidden();
        if let Some(problem) = &self.problem {
            return frame.child(centered(problem.clone()));
        }
        if self.view.lines.is_empty() {
            return frame.child(centered("No log output yet."));
        }
        if self.view.rows().is_empty() {
            return frame.child(centered("No lines match."));
        }
        let rows = list(
            self.list.clone(),
            cx.processor(|this, row: usize, window, cx| this.row(row, window, cx)),
        )
        .size_full()
        .py(px(6.0));
        frame
            .child(
                div()
                    .id("logs-viewport")
                    .size_full()
                    .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                        this.scrolled(event, cx);
                    }))
                    .child(rows),
            )
            .when(!self.live && self.new_lines > 0, |frame| {
                frame.child(self.pill(cx))
            })
            .when(self.selection.is_some(), |frame| {
                frame.child(self.selection_bar(cx))
            })
    }

    fn row(&self, row: usize, window: &mut Window, cx: &mut Context<'_, Self>) -> AnyElement {
        match self.view.rows().get(row).copied() {
            Some(Row::Line { index, shown }) => self.line_row(index, shown, window, cx),
            Some(Row::Gap { first, count }) => gap_row(first, count, cx),
            Some(Row::Mark) => self.mark_row(cx),
            None => div().into_any_element(),
        }
    }

    fn line_row(
        &self,
        index: usize,
        shown: Shown,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let line = &self.view.lines[index];
        let number = self.view.first_number + index;
        let selected = self
            .selected()
            .is_some_and(|(low, high)| (low..=high).contains(&number));
        let context = shown == Shown::Context;
        let background = if selected {
            Some(theme::log_selected())
        } else if context {
            None
        } else {
            match line.level {
                Level::Error => Some(theme::log_error_row()),
                Level::Warn => Some(theme::log_warn_row()),
                _ => None,
            }
        };
        let raw = line.raw().clone();
        let mut highlights = Vec::new();
        // Context lines are muted whole: no token colors.
        if !line.time.is_empty() && !context {
            highlights.push((
                0..line.time.len(),
                HighlightStyle {
                    color: Some(theme::info_text()),
                    ..Default::default()
                },
            ));
            highlights.push((
                line.level_range.clone(),
                HighlightStyle {
                    color: Some(level_color(line.level)),
                    font_weight: matches!(line.level, Level::Error | Level::Warn)
                        .then(|| theme::weight(700.0)),
                    ..Default::default()
                },
            ));
            if let Some(target) = line.target_range.clone() {
                highlights.push((
                    target,
                    HighlightStyle {
                        color: Some(theme::log_target()),
                        ..Default::default()
                    },
                ));
            }
        }
        // Search hits on top, never overlapping the styles above: GPUI
        // wants the ranges in order and apart, so hits replace them. The
        // current match's first hit is solid.
        if shown == Shown::Match {
            let active = self.view.active() == Some(number);
            let hits = search_hits(&raw, &self.search.text);
            if !hits.is_empty() {
                let styled: Vec<_> = hits
                    .into_iter()
                    .enumerate()
                    .map(|(at, hit)| (hit, hit_style(active && at == 0)))
                    .collect();
                highlights = merge_hits(highlights, &styled);
            }
        }
        let number_color = if selected {
            theme::accent_bright()
        } else {
            theme::text_muted()
        };
        let text_color = if context {
            theme::text_muted()
        } else {
            theme::text_secondary()
        };
        let body = if self.wrap {
            let segments = self.segments(&raw, window);
            let indent = self.char_width * HANGING_INDENT;
            div()
                .flex()
                .flex_col()
                .children(segments.into_iter().enumerate().map(|(at, range)| {
                    let part = clip_highlights(&highlights, &range);
                    line_text(raw[range].to_owned().into(), part, text_color)
                        .when(at > 0, |row| row.pl(px(indent)))
                }))
        } else {
            div().overflow_hidden().child(
                line_text(raw, highlights, text_color)
                    .ml(px(-self.h_offset))
                    .debug_selector(move || format!("log-text-{number}")),
            )
        };
        div()
            .id(("log-row", number))
            .debug_selector(move || format!("log-row-{number}"))
            .w_full()
            .flex()
            .when_some(background, |row, color| row.bg(color))
            .child(
                div()
                    .id(("log-number", number))
                    .debug_selector(move || format!("log-number-{number}"))
                    .w(px(GUTTER))
                    .flex()
                    .flex_none()
                    .justify_end()
                    .pr(px(8.0))
                    .border_r_1()
                    .border_color(theme::border_faint())
                    .when(selected, |gutter| {
                        gutter.border_l_3().border_color(theme::accent())
                    })
                    .cursor_pointer()
                    .hover(|gutter| gutter.bg(theme::bg_hover()))
                    .child(div().h(px(ROW_HEIGHT)).flex().items_center().child(mono(
                        number.to_string(),
                        11.0,
                        400.0,
                        number_color,
                    )))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                            // The lines take the keys now: ⌘C, ⌘A and Esc
                            // are the selection's, not the search field's.
                            window.focus(&this.focus, cx);
                            this.click_number(number, event.modifiers.shift, cx);
                        }),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .pl(px(TEXT_LEFT))
                    .pr(px(TEXT_RIGHT))
                    .child(body),
            )
            .into_any_element()
    }

    /// "Mark · 12:39:00", the rule, and Select since mark.
    fn mark_row(&self, cx: &mut Context<'_, Self>) -> AnyElement {
        let label = self
            .view
            .mark()
            .map(|mark| format!("Mark \u{00b7} {}", mark.label))
            .unwrap_or_default();
        let since = self.view.since_mark().is_some();
        let action_color = if since {
            theme::accent_bright()
        } else {
            theme::text_muted()
        };
        div()
            .id("log-mark")
            .debug_selector(|| "log-mark".into())
            .h(px(MARK_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .gap(px(8.0))
            .pl(px(GUTTER + TEXT_LEFT))
            .pr(px(TEXT_RIGHT))
            .child(icon("pwrroute/bookmark.svg", 13.0, theme::accent_bright()))
            .child(text(label, 11.0, 600.0, theme::accent_bright()))
            .child(div().flex_1().h(px(1.5)).bg(theme::accent_border()))
            .child(
                div()
                    .id(SINCE_MARK_ID)
                    .debug_selector(|| SINCE_MARK_ID.into())
                    .px(px(8.0))
                    .py(px(3.0))
                    .border_1()
                    .border_color(theme::accent_border())
                    .rounded(px(5.0))
                    .bg(theme::bg_elevated())
                    .when(since, |action| action.cursor_pointer())
                    .child(text("Select since mark", 11.0, 600.0, action_color))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.select_since_mark(window, cx);
                    })),
            )
            .child(
                div()
                    .id(CLEAR_MARK_ID)
                    .debug_selector(|| CLEAR_MARK_ID.into())
                    .size(px(20.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.0))
                    .cursor_pointer()
                    .hover(|clear| clear.bg(theme::bg_hover_strong()))
                    .child(icon("pwrroute/close.svg", 12.0, theme::text_muted()))
                    .tooltip(tip("Remove the mark"))
                    .on_click(cx.listener(|this, _, _, cx| this.clear_mark(cx))),
            )
            .into_any_element()
    }

    fn pill(&self, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let lines = if self.new_lines == 1 { "line" } else { "lines" };
        let mut label = format!("\u{2193} {} new {lines}", thousands(self.new_lines));
        if self.new_errors > 0 {
            let errors = if self.new_errors == 1 {
                "error"
            } else {
                "errors"
            };
            label.push_str(&format!(" \u{00b7} {} {errors}", self.new_errors));
        }
        let bottom = if self.selection.is_some() { 62.0 } else { 14.0 };
        div()
            .absolute()
            .bottom(px(bottom))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .id(PILL_ID)
                    .debug_selector(|| PILL_ID.into())
                    .h(px(30.0))
                    .px(px(14.0))
                    .flex()
                    .items_center()
                    .rounded_full()
                    .border_1()
                    .border_color(theme::accent_border())
                    .bg(theme::bg_elevated())
                    .cursor_pointer()
                    .child(text(label, 12.0, 600.0, theme::accent_bright()))
                    .on_click(cx.listener(|this, _, _, cx| this.set_live(true, cx))),
            )
    }

    fn selection_bar(&self, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let indexes = self.selected_indexes();
        let first = self.view.first_number;
        let since_mark =
            self.view.since_mark().is_some() && self.selected() == self.view.since_mark();
        let lines = |count: usize| if count == 1 { "line" } else { "lines" };
        let label = match indexes.as_slice() {
            [] => "No shown lines selected".to_owned(),
            many if since_mark => {
                format!("{} {} since mark", thousands(many.len()), lines(many.len()))
            }
            [only] => format!("Line {}", first + only),
            many => format!("{} lines selected", thousands(many.len())),
        };
        let range = match (indexes.first(), indexes.last()) {
            (Some(low), Some(high)) if low != high => {
                Some(format!("{}\u{2013}{}", first + low, first + high))
            }
            _ => None,
        };
        let check = if self.include_diagnostics {
            "\u{2611}"
        } else {
            "\u{2610}"
        };
        div()
            .absolute()
            .bottom(px(14.0))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .py(px(6.0))
                    .pl(px(14.0))
                    .pr(px(8.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(theme::border_strong())
                    .bg(theme::bg_elevated())
                    .shadow_lg()
                    .child(text(label, 12.0, 500.0, theme::text_primary()))
                    .when_some(range, |bar, range| {
                        bar.child(mono(range, 11.0, 400.0, theme::text_muted()))
                    })
                    .child(
                        div()
                            .id("logs-include-diagnostics")
                            .debug_selector(|| "logs-include-diagnostics".into())
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .cursor_pointer()
                            .child(text(check, 13.0, 500.0, theme::text_secondary()))
                            .child(text(
                                "Include diagnostics",
                                12.0,
                                500.0,
                                theme::text_secondary(),
                            ))
                            .tooltip(tip(
                                "Start the copy with the version, platform and log path",
                            ))
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_diagnostics(cx))),
                    )
                    .child(
                        div()
                            .id(COPY_ID)
                            .debug_selector(|| COPY_ID.into())
                            .child(button("Copy", ButtonKind::Primary, true))
                            .on_click(cx.listener(|this, _, _, cx| this.copy_selection(cx))),
                    )
                    .child(
                        div()
                            .id("logs-clear-selection")
                            .px(px(6.0))
                            .cursor_pointer()
                            .child(text("\u{00d7}", 15.0, 500.0, theme::text_muted()))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.selection = None;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn status_bar(&self, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let mut summary = if !self.live && self.new_lines > 0 && !self.view.searching() {
            format!(
                "Paused \u{00b7} {} new since you scrolled",
                thousands(self.new_lines)
            )
        } else {
            self.view.summary()
        };
        if self.view.truncated {
            summary.push_str(&format!(
                " \u{00b7} the newest {}",
                thousands(crate::log_view::MAX_LINES)
            ));
        }
        let searching = self.view.searching();
        let file = self
            .tail
            .as_ref()
            .filter(|_| !searching)
            .map(|tail| file_name(&tail.path));
        let has_file = self.tail.is_some() && !searching;
        div()
            .h(px(34.0))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(14.0))
            .px(px(BODY_PAD))
            .child(text(summary, 11.5, 500.0, theme::text_muted()))
            .when_some(file, |bar, file| {
                bar.child(mono(file, 11.0, 500.0, theme::text_subtle()))
            })
            .child(div().flex_1())
            .when_some(self.note.as_ref(), |bar, (note, _)| {
                bar.child(text(
                    format!("\u{2713} {note}"),
                    11.5,
                    500.0,
                    theme::success_text(),
                ))
            })
            .when(searching, |bar| {
                bar.child(text(SEARCH_HINT, 11.5, 500.0, theme::text_muted()))
            })
            .when(has_file, |bar| {
                bar.child(
                    div()
                        .id(COPY_PATH_ID)
                        .debug_selector(|| COPY_PATH_ID.into())
                        .child(button("Copy path", ButtonKind::Quiet, true))
                        .on_click(cx.listener(|this, _, _, cx| this.copy_path(cx))),
                )
                .child(
                    div()
                        .id(REVEAL_ID)
                        .debug_selector(|| REVEAL_ID.into())
                        .child(button("Reveal", ButtonKind::Quiet, true))
                        .on_click(cx.listener(|this, _, _, cx| this.reveal(cx))),
                )
            })
    }
}

/// "No context", "Context ±3".
fn context_label(lines: usize) -> String {
    if lines == 0 {
        "No context".to_owned()
    } else {
        format!("Context \u{00b1}{lines}")
    }
}

/// A toolbar toggle or action: its icon, and its label unless `narrow`.
fn tool_button(
    id: &'static str,
    path: &'static str,
    label: &'static str,
    on: bool,
    narrow: bool,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let foreground = if on {
        theme::accent_bright()
    } else {
        theme::text_secondary()
    };
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .h(px(32.0))
        .px(px(if narrow { 9.0 } else { 10.0 }))
        .border_1()
        .rounded(px(7.0))
        .border_color(if on {
            theme::accent_border()
        } else {
            theme::border_default()
        })
        .bg(if on {
            theme::accent_soft()
        } else {
            theme::bg_elevated()
        })
        .cursor_pointer()
        .child(icon(path, 14.0, foreground))
        .when(!narrow, |button| {
            button.child(text(label, 11.5, 600.0, foreground))
        })
}

/// "214 lines hidden" between dashed rules; a click shows them.
fn gap_row(first: usize, count: usize, cx: &mut Context<'_, LogsWindow>) -> AnyElement {
    let dashed = || {
        div()
            .h(px(1.0))
            .border_t_1()
            .border_dashed()
            .border_color(theme::border_strong())
    };
    let lines = if count == 1 { "line" } else { "lines" };
    div()
        .id(("log-gap", first))
        .debug_selector(move || format!("log-gap-{first}"))
        .h(px(GAP_HEIGHT))
        .w_full()
        .flex()
        .items_center()
        .gap(px(10.0))
        .pl(px(GUTTER + TEXT_LEFT))
        .pr(px(TEXT_RIGHT))
        .cursor_pointer()
        .hover(|gap| gap.bg(theme::bg_hover()))
        .child(dashed().w(px(18.0)).flex_none())
        .child(text(
            format!("{} {lines} hidden", thousands(count)),
            11.0,
            500.0,
            theme::text_muted(),
        ))
        .child(dashed().flex_1())
        .tooltip(tip("Show these lines"))
        .on_click(cx.listener(move |this, _, _, cx| this.open_gap(first, cx)))
        .into_any_element()
}

/// One visual row of a line's text, unwrapped.
fn line_text(
    content: SharedString,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    color: Hsla,
) -> gpui_kit::Div {
    div()
        .h(px(ROW_HEIGHT))
        .whitespace_nowrap()
        .font_family(theme::mono_family(400.0))
        .font_weight(theme::weight(400.0))
        .text_size(px(TEXT_SIZE))
        .line_height(px(ROW_HEIGHT))
        .text_color(color)
        .child(StyledText::new(content).with_highlights(highlights))
}

fn centered(message: impl Into<SharedString>) -> gpui_kit::Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(text(message, 12.5, 500.0, theme::text_muted()))
}

/// A search hit: the soft accent, or the current match's solid fill.
fn hit_style(current: bool) -> HighlightStyle {
    if current {
        HighlightStyle {
            color: Some(theme::accent_on()),
            background_color: Some(theme::accent()),
            ..Default::default()
        }
    } else {
        let mut soft = theme::accent_bright();
        soft.a = 0.28;
        HighlightStyle {
            color: Some(theme::text_primary()),
            background_color: Some(soft),
            ..Default::default()
        }
    }
}

/// Byte ranges of each search word in `line`, ignoring case, sorted and
/// merged.
pub fn search_hits(line: &str, query: &str) -> Vec<Range<usize>> {
    let lower = line.to_lowercase();
    // Lowercasing can change byte lengths outside ASCII; then skip
    // highlighting rather than cut a character.
    if lower.len() != line.len() {
        return Vec::new();
    }
    let mut hits: Vec<Range<usize>> = Vec::new();
    for word in query.split_whitespace() {
        let word = word.to_lowercase();
        let mut from = 0;
        while let Some(at) = lower[from..].find(&word) {
            let start = from + at;
            hits.push(start..start + word.len());
            from = start + word.len().max(1);
        }
    }
    hits.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for hit in hits {
        match merged.last_mut() {
            Some(last) if hit.start <= last.end => last.end = last.end.max(hit.end),
            _ => merged.push(hit),
        }
    }
    merged
}

/// The token styles with the search hits laid over them: each hit wins
/// where it falls, and what is left of a token keeps its style.
fn merge_hits(
    styles: Vec<(Range<usize>, HighlightStyle)>,
    hits: &[(Range<usize>, HighlightStyle)],
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut out = Vec::new();
    for (range, style) in styles {
        let mut start = range.start;
        for (hit, _) in hits {
            if hit.end <= start || hit.start >= range.end {
                continue;
            }
            if hit.start > start {
                out.push((start..hit.start, style));
            }
            start = hit.end.min(range.end);
        }
        if start < range.end {
            out.push((start..range.end, style));
        }
    }
    out.extend(hits.iter().cloned());
    out.sort_by_key(|(range, _)| range.start);
    out
}

/// The styles that fall inside `segment`, moved to start at its start.
fn clip_highlights(
    highlights: &[(Range<usize>, HighlightStyle)],
    segment: &Range<usize>,
) -> Vec<(Range<usize>, HighlightStyle)> {
    highlights
        .iter()
        .filter_map(|(range, style)| {
            let start = range.start.max(segment.start);
            let end = range.end.min(segment.end);
            (start < end).then(|| (start - segment.start..end - segment.start, *style))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{
        AppContext as _, Modifiers, ScrollDelta, TestAppContext, VisualTestContext, point, size,
    };

    const LINE: &str = "2026-10-24T09:14:37.880-04:00 INFO  pwrroute::link: link 1 outcome=opened";
    const ERROR: &str = "2026-10-24T09:14:38.000-04:00 ERROR pwrroute::link: link 2 failed";

    fn append(path: &std::path::Path, text: &str) {
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        file.write_all(text.as_bytes()).unwrap();
    }

    fn open<'a>(
        cx: &'a mut TestAppContext,
        text: &str,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        gpui_kit::Entity<LogsWindow>,
        &'a mut VisualTestContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.log");
        std::fs::write(&path, text).unwrap();
        cx.update(|cx| crate::assets::load_fonts(cx));
        let log = path.clone();
        let (view, vcx) = cx.add_window_view(move |_, cx| {
            let view = LogsWindow::new(Some(log), cx);
            view.follow(cx);
            view
        });
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        (dir, path, view, vcx)
    }

    fn tick(cx: &mut VisualTestContext) {
        cx.executor().advance_clock(POLL_EVERY);
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    #[gpui_kit::test]
    fn the_window_follows_the_file_and_counts_while_paused(cx: &mut TestAppContext) {
        let (_dir, path, view, cx) = open(cx, &format!("{LINE}\n"));
        assert_eq!(view.read_with(cx, |logs, _| logs.view.lines.len()), 1);

        append(&path, &format!("{LINE}\n"));
        tick(cx);
        assert_eq!(view.read_with(cx, |logs, _| logs.view.lines.len()), 2);
        assert!(view.read_with(cx, |logs, _| logs.live));

        let live = cx.debug_bounds(LIVE_ID).expect("the Live button");
        cx.simulate_click(live.center(), Modifiers::default());
        assert!(!view.read_with(cx, |logs, _| logs.live));
        append(&path, &format!("{LINE}\n{ERROR}\n"));
        tick(cx);
        assert_eq!(view.read_with(cx, |logs, _| logs.new_lines), 2);
        assert!(view.read_with(cx, |logs, _| logs.new_errors) == 1);

        // The pill resumes.
        let pill = cx.debug_bounds(PILL_ID).expect("the new-lines pill");
        cx.simulate_click(pill.center(), Modifiers::default());
        assert!(view.read_with(cx, |logs, _| logs.live && logs.new_lines == 0));
    }

    #[gpui_kit::test]
    fn selected_lines_copy_as_the_file_has_them(cx: &mut TestAppContext) {
        let (_dir, _path, view, cx) = open(cx, &format!("{LINE}\n{ERROR}\n{LINE}\n"));
        view.update(cx, |logs, cx| {
            logs.include_diagnostics = false;
            logs.click_number(1, false, cx);
            logs.click_number(2, true, cx);
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let copy = cx.debug_bounds(COPY_ID).expect("the selection bar's Copy");
        cx.simulate_click(copy.center(), Modifiers::default());
        let copied = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()));
        assert_eq!(
            copied.as_deref(),
            Some(format!("{LINE}\n{ERROR}\n").as_str())
        );

        // Filtered out, a selected line is not copied.
        view.update(cx, |logs, cx| {
            logs.view.toggle_level(Level::Error);
            logs.copy_selection(cx);
        });
        let copied = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()));
        assert_eq!(copied.as_deref(), Some(format!("{LINE}\n").as_str()));
    }

    /// The search field has focus when the window opens; a click in the
    /// gutter hands the keys to the lines, so ⌘A and ⌘C copy them.
    #[gpui_kit::test]
    fn keys_reach_the_selection_after_a_gutter_click(cx: &mut TestAppContext) {
        let (_dir, _path, view, cx) = open(cx, &format!("{LINE}\n{ERROR}\n"));
        view.update_in(cx, |logs, window, cx| {
            logs.include_diagnostics = false;
            window.focus(&logs.search.focus, cx);
        });
        let gutter = cx.debug_bounds("log-number-2").expect("line 2's number");
        cx.simulate_click(gutter.center(), Modifiers::default());
        cx.simulate_keystrokes("cmd-c");
        let copied = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()));
        assert_eq!(copied.as_deref(), Some(format!("{ERROR}\n").as_str()));
        cx.simulate_keystrokes("cmd-a cmd-c");
        let copied = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()));
        assert_eq!(
            copied.as_deref(),
            Some(format!("{LINE}\n{ERROR}\n").as_str())
        );
        cx.simulate_keystrokes("escape");
        assert_eq!(view.read_with(cx, |logs, _| logs.selection), None);
    }

    #[gpui_kit::test]
    fn without_a_log_file_the_window_says_why(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| LogsWindow::new(None, cx));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(
            view.read_with(cx, |logs, _| logs.problem.clone())
                .is_some_and(|p| p.starts_with("No log file"))
        );
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        draw(cx);
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"));
        cx.simulate_click(bounds.center(), Modifiers::default());
        draw(cx);
    }

    /// Twelve lines; every fourth is an error that "failed".
    fn twelve() -> String {
        (1..=12)
            .map(|n| {
                if n % 4 == 0 {
                    format!("2026-10-24T09:14:38.000-04:00 ERROR pwrroute::link: link {n} failed\n")
                } else {
                    format!("2026-10-24T09:14:37.880-04:00 INFO  pwrroute::link: link {n} opened\n")
                }
            })
            .collect()
    }

    #[gpui_kit::test]
    fn long_lines_wrap_with_a_hanging_indent_or_scroll_sideways(cx: &mut TestAppContext) {
        let long = format!(
            "2026-10-24T09:14:37.880-04:00 INFO  pwrroute::link: {}",
            "word ".repeat(80)
        );
        let (_dir, _path, view, cx) = open(cx, &format!("{long}\n{LINE}\n"));
        cx.simulate_resize(size(px(1040.0), px(760.0)));
        draw(cx);
        // Wrap is on by default: the long line takes several rows, the
        // short one one.
        assert!(view.read_with(cx, |logs, _| logs.wrap));
        let tall = cx.debug_bounds("log-row-1").expect("line 1");
        assert!(tall.size.height >= px(ROW_HEIGHT * 3.0), "{tall:?}");
        let short = cx.debug_bounds("log-row-2").expect("line 2");
        assert_eq!(short.size.height, px(ROW_HEIGHT));
        // Cut at word boundaries into rows that cover the line, each short
        // enough to sit beside the 2ch indent.
        let (segments, len, room) = view.update_in(cx, |logs, window, _| {
            let raw = logs.view.lines[0].raw().clone();
            let room = ((logs.text_width / logs.char_width) - HANGING_INDENT) as usize;
            (logs.segments(&raw, window), raw.len(), room)
        });
        assert!(segments.len() >= 3, "{segments:?}");
        assert_eq!(segments.first().map(|s| s.start), Some(0));
        assert_eq!(segments.last().map(|s| s.end), Some(len));
        assert!(segments.windows(2).all(|pair| pair[0].end == pair[1].start));
        assert!(
            segments.iter().all(|s| s.len() <= room),
            "{segments:?} {room}"
        );

        // Off: one row each, and the text scrolls sideways under the gutter.
        click(cx, WRAP_ID);
        assert!(!view.read_with(cx, |logs, _| logs.wrap));
        let row = cx.debug_bounds("log-row-1").expect("line 1");
        assert_eq!(row.size.height, px(ROW_HEIGHT));
        let before = cx
            .debug_bounds("log-text-1")
            .expect("line 1's text")
            .origin
            .x;
        cx.simulate_event(ScrollWheelEvent {
            position: row.center(),
            delta: ScrollDelta::Pixels(point(px(-120.0), px(0.0))),
            ..Default::default()
        });
        draw(cx);
        assert_eq!(view.read_with(cx, |logs, _| logs.h_offset), 120.0);
        let after = cx
            .debug_bounds("log-text-1")
            .expect("line 1's text")
            .origin
            .x;
        assert_eq!(before - after, px(120.0));
        let gutter = cx.debug_bounds("log-number-1").expect("line 1's number");
        assert!(gutter.origin.x < after + px(120.0));
        // It stops at the longest line's end.
        cx.simulate_event(ScrollWheelEvent {
            position: row.center(),
            delta: ScrollDelta::Pixels(point(px(-100_000.0), px(0.0))),
            ..Default::default()
        });
        let (offset, max) = view.read_with(cx, |logs, _| (logs.h_offset, logs.h_max));
        assert!(max > 120.0 && offset == max, "{offset} {max}");
    }

    #[gpui_kit::test]
    fn narrow_windows_keep_only_the_icons(cx: &mut TestAppContext) {
        let (_dir, _path, view, cx) = open(cx, &format!("{LINE}\n"));
        cx.simulate_resize(size(px(1040.0), px(760.0)));
        draw(cx);
        let wide = cx.debug_bounds(WRAP_ID).expect("Wrap").size.width;
        cx.simulate_resize(size(px(860.0), px(640.0)));
        draw(cx);
        let narrow = cx.debug_bounds(WRAP_ID).expect("Wrap").size.width;
        assert!(narrow < wide, "{narrow:?} {wide:?}");
        assert!(narrow <= px(36.0), "{narrow:?}");
        let mark = cx.debug_bounds(MARK_ID).expect("Mark").size.width;
        assert_eq!(mark, narrow);

        // At the least width, searching, with Context showing: one row
        // that fits, the chevrons inside the field and room for the query.
        let least = px(FRAME.min.0);
        cx.simulate_resize(size(least, px(640.0)));
        view.update(cx, |logs, _| logs.set_query_for_screens("outcome"));
        draw(cx);
        let field = cx.debug_bounds(SEARCH_ID).expect("the search field");
        let next = cx.debug_bounds(NEXT_ID).expect("the next chevron");
        assert!(next.right() <= field.right(), "{next:?} in {field:?}");
        assert!(field.size.width >= px(200.0), "{field:?}");
        assert!(cx.debug_bounds(CONTEXT_ID).is_some());
        // Live's place depends on the face: the test text system is wider
        // than Geist, so the screens example's `help-logs-narrow` checks it.
    }

    #[gpui_kit::test]
    fn search_filters_steps_through_matches_and_opens_gaps(cx: &mut TestAppContext) {
        let (_dir, _path, view, cx) = open(cx, &twelve());
        view.update_in(cx, |logs, window, cx| window.focus(&logs.search.focus, cx));
        cx.simulate_keystrokes("f a i l e d");
        draw(cx);
        view.read_with(cx, |logs, _| {
            assert!(!logs.live, "a search stops following");
            assert_eq!(logs.view.match_count(), 3);
            assert_eq!(logs.view.active(), Some(4));
        });
        assert!(cx.debug_bounds("log-gap-1").is_some(), "lines 1-3 hidden");
        assert!(cx.debug_bounds("log-row-1").is_none());

        let active = |cx: &mut VisualTestContext| view.read_with(cx, |logs, _| logs.view.active());
        cx.simulate_keystrokes("enter");
        assert_eq!(active(cx), Some(8));
        cx.simulate_keystrokes("shift-enter shift-enter");
        assert_eq!(active(cx), Some(12));
        click(cx, NEXT_ID);
        assert_eq!(active(cx), Some(4));
        click(cx, PREV_ID);
        assert_eq!(active(cx), Some(12));

        // Context ±1 from the select.
        click(cx, CONTEXT_ID);
        click(cx, "logs-context-1");
        assert_eq!(view.read_with(cx, |logs, _| logs.view.context), 1);
        assert!(cx.debug_bounds("log-row-3").is_some(), "line 3 is context");
        assert!(cx.debug_bounds("log-gap-1").is_some(), "lines 1-2 hidden");

        // A gap row shows its lines.
        click(cx, "log-gap-1");
        assert!(cx.debug_bounds("log-row-1").is_some());

        // Highlight keeps every line.
        click(cx, HIGHLIGHT_ID);
        assert_eq!(view.read_with(cx, |logs, _| logs.view.rows().len()), 12);
        assert!(cx.debug_bounds(CONTEXT_ID).is_none(), "Context is Filter's");

        // Esc clears the search; every line is back.
        view.update_in(cx, |logs, window, cx| window.focus(&logs.search.focus, cx));
        cx.simulate_keystrokes("escape");
        draw(cx);
        view.read_with(cx, |logs, _| {
            assert!(!logs.view.searching());
            assert_eq!(logs.view.rows().len(), 12);
        });
    }

    #[gpui_kit::test]
    fn the_mark_selects_what_came_after_it(cx: &mut TestAppContext) {
        let (_dir, path, view, cx) = open(cx, &format!("{LINE}\n{ERROR}\n"));
        click(cx, MARK_ID);
        assert_eq!(
            view.read_with(cx, |logs, _| logs.view.mark().map(|mark| mark.after)),
            Some(2)
        );
        assert!(cx.debug_bounds("log-mark").is_some());
        // Nothing after it yet: Select since mark does nothing.
        click(cx, SINCE_MARK_ID);
        assert_eq!(view.read_with(cx, |logs, _| logs.selection), None);

        append(&path, &format!("{ERROR}\n{LINE}\n"));
        tick(cx);
        click(cx, SINCE_MARK_ID);
        assert_eq!(view.read_with(cx, |logs, _| logs.selection), Some((3, 4)));
        // The selection ends on the newest line, so it grows with the log.
        append(&path, &format!("{LINE}\n"));
        tick(cx);
        assert_eq!(view.read_with(cx, |logs, _| logs.selection), Some((3, 5)));

        view.update(cx, |logs, _| logs.include_diagnostics = false);
        cx.simulate_keystrokes("cmd-c");
        let copied = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()));
        assert_eq!(
            copied.as_deref(),
            Some(format!("{ERROR}\n{LINE}\n{LINE}\n").as_str())
        );

        click(cx, CLEAR_MARK_ID);
        assert!(view.read_with(cx, |logs, _| logs.view.mark().is_none()));
    }

    #[gpui_kit::test]
    fn the_view_is_remembered_but_never_debug(cx: &mut TestAppContext) {
        let data = tempfile::tempdir().unwrap();
        let store = crate::settings_store::SettingsStore::load(Some(data.path().to_path_buf()));
        cx.update(|cx| crate::settings_store::install(store, None, cx));
        let (_dir, path, view, cx) = open(cx, &format!("{LINE}\n{ERROR}\n"));
        click(cx, WRAP_ID);
        click(cx, HIGHLIGHT_ID);
        click(cx, "logs-level-Info");
        click(cx, "logs-level-Debug");
        view.update(cx, |logs, cx| logs.click_number(2, false, cx));
        click(cx, "logs-include-diagnostics");
        view.update(cx, |logs, cx| logs.set_context(5, cx));

        let text = std::fs::read_to_string(data.path().join("settings.json")).unwrap();
        let file: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            file["logs"],
            serde_json::json!({
                "wrap": false,
                "searchMode": "highlight",
                "contextLines": 5,
                "includeDiagnostics": false,
                "levels": { "error": true, "warning": true, "info": false },
            })
        );
        assert!(!text.to_lowercase().contains("debug"), "{text}");
        assert!(!text.contains("pwrroute::link"), "{text}");

        // The next window opens the same way, with Debug off again.
        let again = cx.update(|_, cx| cx.new(|cx| LogsWindow::new(Some(path), cx)));
        again.read_with(cx, |logs, _| {
            assert!(!logs.wrap);
            assert!(!logs.include_diagnostics);
            assert_eq!(logs.view.mode, SearchMode::Highlight);
            assert_eq!(logs.view.context, 5);
            assert!(!logs.view.shows(Level::Info));
            assert!(!logs.view.shows(Level::Debug));
            assert!(logs.view.shows(Level::Error));
        });
    }

    #[test]
    fn hits_are_found_ignoring_case_and_merged() {
        assert_eq!(search_hits("Link 42 link", "LINK"), [0..4, 8..12]);
        assert_eq!(
            search_hits("abcdef", "abc cd"),
            [Range { start: 0, end: 4 }]
        );
        assert!(search_hits("abc", "").is_empty());
    }

    #[test]
    fn hits_cut_the_token_styles_they_cross() {
        let plain = HighlightStyle::default();
        let hit = (Range { start: 2, end: 4 }, hit_style(true));
        let merged = merge_hits(vec![(0..10, plain)], std::slice::from_ref(&hit));
        let ranges: Vec<_> = merged.iter().map(|(range, _)| range.clone()).collect();
        assert_eq!(ranges, [0..2, 2..4, 4..10]);
        assert_eq!(merged[1].1, hit_style(true));
    }

    #[test]
    fn a_wrapped_rows_styles_are_clipped_to_it() {
        let style = HighlightStyle::default();
        let clipped = clip_highlights(&[(0..4, style), (6..12, style)], &(5..10));
        let ranges: Vec<_> = clipped.iter().map(|(range, _)| range.clone()).collect();
        assert_eq!(ranges, [Range { start: 1, end: 5 }]);
    }

    /// Only Select since mark and a Shift-click that reaches the newest
    /// line follow new lines; a plain click on the newest line stays put.
    #[gpui_kit::test]
    fn only_a_following_selection_grows(cx: &mut TestAppContext) {
        let (_dir, path, view, cx) = open(cx, &format!("{LINE}\n{ERROR}\n"));
        view.update(cx, |logs, cx| logs.click_number(2, false, cx));
        append(&path, &format!("{LINE}\n"));
        tick(cx);
        assert_eq!(view.read_with(cx, |logs, _| logs.selection), Some((2, 2)));
        view.update(cx, |logs, cx| logs.select_all(cx));
        append(&path, &format!("{LINE}\n"));
        tick(cx);
        assert_eq!(view.read_with(cx, |logs, _| logs.selection), Some((1, 3)));
        view.update(cx, |logs, cx| {
            logs.click_number(1, false, cx);
            logs.click_number(4, true, cx);
        });
        append(&path, &format!("{LINE}\n"));
        tick(cx);
        assert_eq!(view.read_with(cx, |logs, _| logs.selection), Some((1, 5)));
    }

    /// Paused, a change that rebuilds every row keeps the reader on the
    /// line they were reading.
    #[gpui_kit::test]
    fn a_level_toggle_keeps_the_place_while_paused(cx: &mut TestAppContext) {
        let text: String = (1..=400)
            .map(|n| {
                if n % 2 == 0 {
                    format!("2026-10-24T09:14:38.000-04:00 WARN  pwrroute::link: link {n} slow\n")
                } else {
                    format!("2026-10-24T09:14:37.880-04:00 INFO  pwrroute::link: link {n} opened\n")
                }
            })
            .collect();
        let (_dir, _path, view, cx) = open(cx, &text);
        view.update(cx, |logs, cx| {
            logs.set_live(false, cx);
            let row = logs.view.row_of(201).unwrap();
            logs.list.scroll_to(ListOffset {
                item_ix: row,
                offset_in_item: px(0.0),
            });
        });
        draw(cx);
        // Info off: line 201 goes; the top is the next line still shown.
        click(cx, "logs-level-Info");
        let top = view.read_with(cx, |logs, _| {
            let row = logs.list.logical_scroll_top().item_ix;
            logs.view.row_key(logs.view.rows()[row])
        });
        assert_eq!(
            top,
            RowKey::Line {
                number: 202,
                shown: Shown::Plain
            }
        );
    }

    /// A list that fits has nowhere to scroll: a scroll down does not undo
    /// Pause.
    #[gpui_kit::test]
    fn scrolling_a_short_list_keeps_it_paused(cx: &mut TestAppContext) {
        let (_dir, _path, view, cx) = open(cx, &format!("{LINE}\n{ERROR}\n"));
        click(cx, LIVE_ID);
        assert!(!view.read_with(cx, |logs, _| logs.live));
        let row = cx.debug_bounds("log-row-1").expect("line 1");
        cx.simulate_event(ScrollWheelEvent {
            position: row.center(),
            delta: ScrollDelta::Pixels(point(px(0.0), px(-60.0))),
            ..Default::default()
        });
        assert!(!view.read_with(cx, |logs, _| logs.live));
    }

    /// The Context list does not come back open with its select.
    #[gpui_kit::test]
    fn the_context_list_closes_with_its_select(cx: &mut TestAppContext) {
        let (_dir, _path, view, cx) = open(cx, &twelve());
        view.update(cx, |logs, _| logs.set_query_for_screens("failed"));
        click(cx, CONTEXT_ID);
        assert!(cx.debug_bounds("logs-context-3").is_some());
        click(cx, HIGHLIGHT_ID);
        click(cx, FILTER_ID);
        assert!(cx.debug_bounds(CONTEXT_ID).is_some());
        assert!(cx.debug_bounds("logs-context-3").is_none());
    }
}
