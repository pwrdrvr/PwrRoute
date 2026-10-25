//! Help › Logs: `main.log`, live, in its own window. The family's viewer
//! (PwrAgent's "Log Viewer Workflows UX Review" in Claude Design), cut to
//! what PwrRoute's one log needs: search that filters, the level toggles
//! with Error and Warning counts, Live / Paused with a "new lines" pill,
//! line selection in the number gutter with Copy (and the diagnostics
//! header), and the file's Copy path and Reveal.
//!
//! The log never carries a URL (`diag_log`), so nothing here filters one.
//! The model, parsing and the file reads are `log_view.rs`.

use std::time::Duration;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClipboardItem, Context, FocusHandle, HighlightStyle, Hsla, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, ParentElement as _, Render,
    ScrollWheelEvent, SharedString, StatefulInteractiveElement as _, Styled as _, StyledText,
    UniformListScrollHandle, Window, WindowHandle, div, px, rgb, uniform_list,
};

use crate::aux_window::{self, Frame};
use crate::log_view::{Level, LogView, Read, Tail, file_name, thousands};
use crate::text_field::{self, Edit, Look, TextField};
use crate::theme::{self, icon, mono, text};
use crate::widgets::{ButtonKind, button, segment, segmented};

/// How often the file is read for new lines.
const POLL_EVERY: Duration = Duration::from_millis(500);
/// How long a status note ("Copied 3 lines") stays.
const NOTE_FOR: Duration = Duration::from_secs(3);
/// A row: 12px mono at the design's 1.55.
const ROW_HEIGHT: f32 = 19.0;
/// The number gutter.
const GUTTER: f32 = 56.0;

pub const SEARCH_ID: &str = "logs-search";
pub const LIVE_ID: &str = "logs-live";
pub const COPY_PATH_ID: &str = "logs-copy-path";
pub const REVEAL_ID: &str = "logs-reveal";
pub const COPY_ID: &str = "logs-copy";
pub const PILL_ID: &str = "logs-new-lines";

const FRAME: Frame = Frame {
    title: "PwrRoute Logs",
    size: (1040.0, 760.0),
    min: (700.0, 500.0),
};

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
    scroll: UniformListScrollHandle,
    /// New lines scroll into view.
    pub live: bool,
    /// Lines and errors that came in while paused.
    pub new_lines: usize,
    new_errors: usize,
    /// Selected lines, by number (`LogView::first_number` counting): the
    /// anchor and the other end.
    pub selection: Option<(usize, usize)>,
    include_diagnostics: bool,
    note: Option<(SharedString, u64)>,
    note_generation: u64,
}

impl LogsWindow {
    pub fn new(path: Option<std::path::PathBuf>, cx: &mut Context<'_, Self>) -> Self {
        let mut this = Self {
            view: LogView::default(),
            tail: path.map(Tail::new),
            problem: None,
            search: TextField::new("", cx),
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            live: true,
            new_lines: 0,
            new_errors: 0,
            selection: None,
            include_diagnostics: true,
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
        self.scroll.scroll_to_bottom();
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
                let before = self.view.lines.len() + self.view.first_number;
                self.view.append(&text);
                if !self.live {
                    let added = (self.view.lines.len() + self.view.first_number) - before;
                    self.new_lines += added;
                    self.new_errors += text
                        .lines()
                        .filter(|line| crate::log_view::LogLine::parse(line).level == Level::Error)
                        .count();
                }
            }
        }
        if self.live {
            self.scroll.scroll_to_bottom();
        }
        cx.notify();
    }

    pub fn set_live(&mut self, live: bool, cx: &mut Context<'_, Self>) {
        self.live = live;
        if live {
            self.new_lines = 0;
            self.new_errors = 0;
            self.scroll.scroll_to_bottom();
        }
        cx.notify();
    }

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
        cx.notify();
    }

    fn select_all(&mut self, cx: &mut Context<'_, Self>) {
        let visible = self.view.visible();
        if let (Some(first), Some(last)) = (visible.first(), visible.last()) {
            let base = self.view.first_number;
            self.selection = Some((base + first, base + last));
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
            out.push_str(&self.view.lines[index].raw());
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

    /// "Log lines: 215–219 (5 of 1,204; levels: Error, Warning, Info)".
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
            header.push_str(&format!("Log search: \"{}\"\n", self.search.text));
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

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        let keystroke = &event.keystroke;
        let command = keystroke.modifiers.platform || keystroke.modifiers.control;
        if self.search.focus.is_focused(window) {
            if keystroke.key == "escape" && !self.search.text.is_empty() {
                self.search.set("");
                self.view.set_query("");
                cx.stop_propagation();
                cx.notify();
                return;
            }
            match self.search.key(keystroke, cx) {
                Edit::Ignored => {}
                Edit::Changed => {
                    self.view.set_query(&self.search.text);
                    // Reading a search's results, not the tail.
                    if self.view.searching() && self.live {
                        self.live = false;
                    }
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                Edit::Submit | Edit::Handled => {
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
        self.view.set_query(query);
    }

    fn scrolled(&mut self, event: &ScrollWheelEvent, cx: &mut Context<'_, Self>) {
        let delta = event.delta.pixel_delta(px(ROW_HEIGHT)).y;
        if delta > px(0.0) && self.live {
            // Scrolled up: stop following.
            self.live = false;
            cx.notify();
        } else if delta < px(0.0)
            && !self.live
            && !self.view.searching()
            && self.scroll.is_scrolled_to_end() == Some(true)
        {
            self.set_live(true, cx);
        }
    }
}

// ---- Drawing --------------------------------------------------------------

fn teal() -> Hsla {
    rgb(0x7fcfc4).into()
}

fn wash(color: u32, alpha: f32) -> Hsla {
    let mut color: Hsla = rgb(color).into();
    color.a = alpha;
    color
}

fn level_color(level: Level) -> Hsla {
    match level {
        Level::Error => theme::danger_text(),
        Level::Warn => theme::status_warning(),
        Level::Info => theme::text_primary(),
        Level::Debug => theme::text_muted(),
    }
}

impl Render for LogsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
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
                    .px(px(18.0))
                    .child(self.toolbar(window, cx))
                    .child(self.viewport(cx)),
            )
            .child(self.status_bar(cx))
    }
}

impl LogsWindow {
    fn toolbar(&self, window: &Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let search = text_field::field(
            SEARCH_ID,
            &self.search,
            Look {
                mono: false,
                placeholder: "Search logs",
            },
            Some(icon("pwrroute/search.svg", 13.0, theme::text_subtle()).into_any_element()),
            window,
        )
        .on_click(cx.listener(|this, _, window, cx| {
            window.focus(&this.search.focus, cx);
            cx.notify();
        }));
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
                    .child(segment(label, on))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.view.toggle_level(level);
                        cx.notify();
                    })),
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
            .on_click(cx.listener(|this, _, _, cx| {
                let live = !this.live;
                this.set_live(live, cx);
            }));
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .child(div().flex_1().min_w(px(200.0)).child(search))
            .child(levels)
            .child(live)
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
        if self.view.visible().is_empty() {
            return frame.child(centered("No lines match."));
        }
        let count = self.view.visible().len();
        let list = uniform_list(
            "logs-lines",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range.map(|row| this.row(row, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
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
                    .child(list),
            )
            .when(!self.live && self.new_lines > 0, |frame| {
                frame.child(self.pill(cx))
            })
            .when(self.selection.is_some(), |frame| {
                frame.child(self.selection_bar(cx))
            })
    }

    fn row(&self, row: usize, cx: &mut Context<'_, Self>) -> gpui_kit::AnyElement {
        let index = self.view.visible()[row];
        let line = &self.view.lines[index];
        let number = self.view.first_number + index;
        let selected = self
            .selected()
            .is_some_and(|(low, high)| (low..=high).contains(&number));
        let background = if selected {
            Some(wash(0xff8a1f, 0.10))
        } else {
            match line.level {
                Level::Error => Some(wash(0xb94232, 0.075)),
                Level::Warn => Some(wash(0xd99a3d, 0.045)),
                _ => None,
            }
        };
        let raw = line.raw();
        let mut highlights = Vec::new();
        if !line.time.is_empty() {
            let time_end = line.time.len();
            highlights.push((
                0..time_end,
                HighlightStyle {
                    color: Some(theme::info_text()),
                    ..Default::default()
                },
            ));
            let level_start = time_end + 1;
            let level_end = level_start + line.level_text.len();
            highlights.push((
                level_start..level_end,
                HighlightStyle {
                    color: Some(level_color(line.level)),
                    font_weight: matches!(line.level, Level::Error | Level::Warn)
                        .then(|| theme::weight(700.0)),
                    ..Default::default()
                },
            ));
            if !line.target.is_empty() {
                let target_start = level_end + 1;
                highlights.push((
                    target_start..target_start + line.target.len(),
                    HighlightStyle {
                        color: Some(teal()),
                        ..Default::default()
                    },
                ));
            }
        }
        // Search hits on top, never overlapping the styles above: GPUI
        // wants the ranges in order and apart, so hits replace them.
        let hits = search_hits(&raw, &self.search.text);
        if !hits.is_empty() {
            highlights = merge_hits(highlights, &hits);
        }
        let number_color = if selected {
            theme::accent_bright()
        } else {
            theme::text_muted()
        };
        div()
            .id(("log-row", number))
            .h(px(ROW_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .when_some(background, |row, color| row.bg(color))
            .child(
                div()
                    .id(("log-number", number))
                    .w(px(GUTTER))
                    .h_full()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_end()
                    .pr(px(8.0))
                    .border_r_1()
                    .border_color(theme::border_faint())
                    .when(selected, |gutter| {
                        gutter.border_l_3().border_color(theme::accent())
                    })
                    .cursor_pointer()
                    .hover(|gutter| gutter.bg(theme::bg_hover()))
                    .child(mono(number.to_string(), 11.0, 400.0, number_color))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            this.click_number(number, event.modifiers.shift, cx);
                        }),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .pl(px(12.0))
                    .pr(px(14.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .font_family(theme::mono_family(400.0))
                    .font_weight(theme::weight(400.0))
                    .text_size(px(12.0))
                    .line_height(px(ROW_HEIGHT))
                    .text_color(theme::text_secondary())
                    .child(StyledText::new(raw).with_highlights(highlights)),
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
        let label = match indexes.as_slice() {
            [] => "No shown lines selected".to_owned(),
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
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.include_diagnostics = !this.include_diagnostics;
                                cx.notify();
                            })),
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
        let mut summary = if !self.live && self.new_lines > 0 {
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
        let file = self.tail.as_ref().map(|tail| file_name(&tail.path));
        let has_file = self.tail.is_some();
        div()
            .h(px(34.0))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(14.0))
            .px(px(18.0))
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

fn centered(message: impl Into<SharedString>) -> gpui_kit::Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(text(message, 12.5, 500.0, theme::text_muted()))
}

/// Byte ranges of each search word in `line`, ignoring case, sorted and
/// merged.
pub fn search_hits(line: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    let lower = line.to_lowercase();
    // Lowercasing can change byte lengths outside ASCII; then skip
    // highlighting rather than cut a character.
    if lower.len() != line.len() {
        return Vec::new();
    }
    let mut hits: Vec<std::ops::Range<usize>> = Vec::new();
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
    let mut merged: Vec<std::ops::Range<usize>> = Vec::new();
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
    styles: Vec<(std::ops::Range<usize>, HighlightStyle)>,
    hits: &[std::ops::Range<usize>],
) -> Vec<(std::ops::Range<usize>, HighlightStyle)> {
    let hit_style = HighlightStyle {
        color: Some(theme::accent_on()),
        background_color: Some(theme::accent_bright()),
        ..Default::default()
    };
    let mut out = Vec::new();
    for (range, style) in styles {
        let mut start = range.start;
        for hit in hits {
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
    out.extend(hits.iter().map(|hit| (hit.clone(), hit_style)));
    out.sort_by_key(|(range, _)| range.start);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{Modifiers, TestAppContext, VisualTestContext};

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

    #[gpui_kit::test]
    fn without_a_log_file_the_window_says_why(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| LogsWindow::new(None, cx));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(
            view.read_with(cx, |logs, _| logs.problem.clone())
                .is_some_and(|p| p.starts_with("No log file"))
        );
    }

    #[test]
    fn hits_are_found_ignoring_case_and_merged() {
        assert_eq!(search_hits("Link 42 link", "LINK"), [0..4, 8..12]);
        assert_eq!(
            search_hits("abcdef", "abc cd"),
            [std::ops::Range { start: 0, end: 4 }]
        );
        assert!(search_hits("abc", "").is_empty());
    }

    #[test]
    fn hits_cut_the_token_styles_they_cross() {
        let plain = HighlightStyle::default();
        let hit = std::ops::Range { start: 2, end: 4 };
        let merged = merge_hits(vec![(0..10, plain)], std::slice::from_ref(&hit));
        let ranges: Vec<_> = merged.iter().map(|(range, _)| range.clone()).collect();
        assert_eq!(ranges, [0..2, 2..4, 4..10]);
    }
}
