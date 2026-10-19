//! Activity (design option 2b): the router's recent dispatches, newest
//! first, one expandable to its trace.
//!
//! [`ActivityRow`] is the screen's model. Fixtures fill it for now; the
//! router's in-memory buffer of `DispatchRecord`s will fill the same rows.
//! Full URLs stay in memory, as the footer says.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Div, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};

use crate::app::{PwrRoute, Screen};
use crate::fixtures;
use crate::rules_view::{card, pane_frame, pane_heading};
use crate::text_field::{self, Edit, Look, TextField};
use crate::theme::{self, icon, mono, text, tracked};
use crate::widgets::{ButtonKind, badge, button, note, segment, segmented, status, tag};

/// One dispatch.
#[derive(Clone, Debug)]
pub struct ActivityRow {
    pub time: SharedString,
    /// The link as received.
    pub url: SharedString,
    /// The app it came from.
    pub from: SharedString,
    /// The rules that fired, short: "rw 1, 2 · h 3".
    pub rule: SharedString,
    pub opened: Opened,
    pub took: SharedString,
    /// A rewrite changed the URL.
    pub rewritten: bool,
    /// No handler matched.
    pub default: bool,
    /// The trace, for the expanded row.
    pub steps: Vec<ActivityStep>,
    /// The URL the browser was asked to open.
    pub final_url: SharedString,
}

/// Where a dispatch went.
#[derive(Clone, Debug)]
pub enum Opened {
    Browser {
        badge: SharedString,
        name: SharedString,
        profile: Option<SharedString>,
    },
    /// The launch failed, with why.
    Failed(SharedString),
}

/// A line of an expanded row's trace.
#[derive(Clone, Debug)]
pub struct ActivityStep {
    pub mark: StepMark,
    pub text: SharedString,
}

/// `.ax .l .n`: the line's numbered circle.
#[derive(Clone, Copy, Debug)]
pub enum StepMark {
    Hit(&'static str),
    Miss(&'static str),
    Done,
}

/// The `.seg` filters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Rewritten,
    Default,
    Failed,
}

impl Filter {
    const ALL: [(Self, &'static str); 4] = [
        (Self::All, "All"),
        (Self::Rewritten, "Rewritten"),
        (Self::Default, "Default"),
        (Self::Failed, "Failed"),
    ];
}

impl ActivityRow {
    pub fn failed(&self) -> bool {
        matches!(self.opened, Opened::Failed(_))
    }

    fn passes(&self, filter: Filter, search: &str) -> bool {
        let kind = match filter {
            Filter::All => true,
            Filter::Rewritten => self.rewritten,
            Filter::Default => self.default,
            Filter::Failed => self.failed(),
        };
        if !kind {
            return false;
        }
        if search.is_empty() {
            return true;
        }
        let needle = search.to_lowercase();
        let opened = match &self.opened {
            Opened::Browser { name, profile, .. } => {
                format!("{name} {}", profile.as_deref().unwrap_or_default())
            }
            Opened::Failed(why) => why.to_string(),
        };
        [self.url.as_ref(), self.from.as_ref(), opened.as_str()]
            .iter()
            .any(|field| field.to_lowercase().contains(&needle))
    }
}

/// The screen's state.
pub struct Activity {
    pub rows: Vec<ActivityRow>,
    /// Index into `rows` of the expanded row.
    pub expanded: Option<usize>,
    pub filter: Filter,
    pub search: TextField,
}

impl Activity {
    pub fn new(cx: &mut gpui_kit::App) -> Self {
        Self {
            rows: fixtures::activity(),
            expanded: Some(1),
            filter: Filter::All,
            search: TextField::new("", cx),
        }
    }

    /// The rows the filter and search let through, with their indexes.
    pub fn visible(&self) -> Vec<(usize, &ActivityRow)> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.passes(self.filter, &self.search.text))
            .collect()
    }

    pub fn toggle(&mut self, index: usize) {
        self.expanded = if self.expanded == Some(index) {
            None
        } else {
            Some(index)
        };
    }

    pub fn clear(&mut self) {
        self.rows.clear();
        self.expanded = None;
    }
}

/// Column widths: the design's grid `54px minmax(0,1fr) 84px 118px 160px
/// 52px` with a 12px gap.
mod column {
    pub const TIME: f32 = 54.0;
    pub const FROM: f32 = 84.0;
    pub const RULE: f32 = 118.0;
    pub const OPENED: f32 = 160.0;
    pub const TOOK: f32 = 52.0;
    pub const GAP: f32 = 12.0;
    /// Where the expanded trace starts: the link column.
    pub const TRACE_INDENT: f32 = 14.0 + TIME + GAP;
}

/// `.ar`'s cells around their contents.
fn grid_row(cells: [Div; 6]) -> Div {
    let [time, link, from, rule, opened, took] = cells;
    div()
        .flex()
        .items_center()
        .gap(px(column::GAP))
        .px(px(14.0))
        .child(time.w(px(column::TIME)).flex_none())
        .child(link.flex_1().min_w_0())
        .child(from.w(px(column::FROM)).flex_none().min_w_0())
        .child(rule.w(px(column::RULE)).flex_none().min_w_0())
        .child(opened.w(px(column::OPENED)).flex_none().min_w_0())
        .child(took.w(px(column::TOOK)).flex_none().flex().justify_end())
}

/// `.ar.head`
fn head_row() -> Div {
    let cell = |label: &str| div().child(tracked(label, 9.5, 600.0, 0.14, theme::text_muted()));
    grid_row([
        cell("Time"),
        cell("Link"),
        cell("From"),
        cell("Rule"),
        cell("Opened in"),
        cell("Took"),
    ])
    .py(px(7.0))
    .bg(theme::bg_elevated())
    .border_b_1()
    .border_color(theme::border_faint())
}

/// A one-line cell that ends in an ellipsis when it does not fit.
fn clipped(content: SharedString, mono_font: bool, size: f32, color: gpui_kit::Hsla) -> Div {
    let (family, weight) = if mono_font {
        (theme::mono_family(500.0), 500.0)
    } else {
        (theme::SANS, 400.0)
    };
    div()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .font_family(family)
        .font_weight(theme::weight(weight))
        .text_size(px(size))
        .line_height(px(size))
        .text_color(color)
        .child(content)
}

fn opened_cell(opened: &Opened) -> Div {
    match opened {
        Opened::Browser {
            badge: badge_label,
            name,
            profile,
        } => div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .overflow_hidden()
            .child(badge(badge_label.clone()))
            .child(text(name.clone(), 11.5, 400.0, theme::text_secondary()))
            .when_some(profile.clone(), |cell, profile| {
                cell.child(mono(profile, 10.5, 500.0, theme::text_muted()))
            }),
        Opened::Failed(why) => div().child(status(why.clone(), theme::status_error())),
    }
}

/// `.ar`
fn row(
    index: usize,
    row: &ActivityRow,
    expanded: bool,
    last: bool,
    cx: &mut Context<'_, PwrRoute>,
) -> impl IntoElement + use<> {
    let link_color = if row.failed() {
        theme::danger_text()
    } else {
        theme::text_primary()
    };
    grid_row([
        div().child(mono(row.time.clone(), 10.5, 500.0, theme::text_subtle())),
        clipped(row.url.clone(), true, 11.5, link_color),
        clipped(row.from.clone(), false, 11.5, theme::text_secondary()),
        div().flex().child(tag(row.rule.clone())),
        opened_cell(&row.opened),
        div().child(mono(row.took.clone(), 10.5, 500.0, theme::text_subtle())),
    ])
    .py(px(8.0))
    .when(expanded, |row| row.bg(theme::bg_row_active()))
    .when(!expanded && !last, |row| {
        row.border_b_1().border_color(theme::border_faint())
    })
    .when(!expanded, |row| {
        row.hover(|style| style.bg(theme::bg_hover()))
    })
    .id(("activity-row", index))
    .debug_selector(move || format!("activity-row-{index}"))
    .cursor_pointer()
    .on_click(cx.listener(move |this, _, _, cx| {
        this.activity.toggle(index);
        cx.notify();
    }))
}

/// `.ax`: the expanded row's trace and actions.
fn expansion(index: usize, row: &ActivityRow, last: bool, cx: &mut Context<'_, PwrRoute>) -> Div {
    let line = |mark: StepMark, content: SharedString| {
        let (label, fill, color) = match mark {
            StepMark::Hit(n) => (n, theme::accent_soft(), theme::accent_bright()),
            StepMark::Miss(n) => (n, theme::bg_elevated(), theme::text_subtle()),
            StepMark::Done => ("\u{2713}", theme::success_soft(), theme::success_text()),
        };
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .size(px(16.0))
                    .flex_none()
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(fill)
                    .child(mono(label, 8.5, 600.0, color)),
            )
            .child(text(content, 11.0, 400.0, theme::text_muted()).line_height(px(16.5)))
    };
    let url = row.url.to_string();
    let from = row.from.to_string();
    row.steps
        .iter()
        .fold(
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .pt(px(4.0))
                .pr(px(14.0))
                .pb(px(12.0))
                .pl(px(column::TRACE_INDENT))
                .bg(theme::bg_row_active())
                .when(!last, |ax| {
                    ax.border_b_1().border_color(theme::border_faint())
                }),
            |ax, step| ax.child(line(step.mark, step.text.clone())),
        )
        .child(
            div()
                .mt(px(2.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(mono(
                    row.final_url.clone(),
                    11.5,
                    500.0,
                    theme::text_secondary(),
                ))
                // Opening again waits for the browser launcher.
                .child(button("Open again", ButtonKind::Quiet, true))
                .child(
                    button("Re-test", ButtonKind::Quiet, true)
                        .id(("activity-retest", index))
                        .debug_selector(move || format!("activity-retest-{index}"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.retest(&url, &from, window, cx);
                        })),
                ),
        )
}

/// `.toolbar`: search, the filters and where the list lives.
fn toolbar(app: &PwrRoute, window: &mut Window, cx: &mut Context<'_, PwrRoute>) -> Div {
    let search = text_field::field(
        "activity-search",
        &app.activity.search,
        Look {
            mono: false,
            placeholder: "Search URL, app, browser",
        },
        Some(icon("pwrroute/search.svg", 13.0, theme::text_subtle()).into_any_element()),
        window,
    )
    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
        match this.activity.search.key(&event.keystroke, cx) {
            Edit::Ignored => return,
            Edit::Changed | Edit::Submit | Edit::Handled => {}
        }
        cx.stop_propagation();
        cx.notify();
    }))
    .on_click(cx.listener(|this, _, window, cx| {
        window.focus(&this.activity.search.focus, cx);
        cx.notify();
    }));
    let filters = Filter::ALL
        .into_iter()
        .fold(segmented(), |seg, (filter, label)| {
            seg.child(
                segment(label, app.activity.filter == filter)
                    .id(label)
                    .debug_selector(move || format!("activity-filter-{label}"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.activity.filter = filter;
                        cx.notify();
                    })),
            )
        });
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .child(div().w(px(260.0)).flex_none().flex().child(search))
        .child(filters)
        .child(div().ml_auto().child(note("last 100 \u{00b7} in memory")))
}

pub fn pane(
    app: &PwrRoute,
    window: &mut Window,
    cx: &mut Context<'_, PwrRoute>,
) -> impl IntoElement + use<> {
    let visible = app.activity.visible();
    let count = visible.len();
    let list = visible.into_iter().enumerate().fold(
        card().child(head_row()),
        |list, (position, (index, item))| {
            let last = position + 1 == count;
            let expanded = app.activity.expanded == Some(index);
            list.child(row(index, item, expanded, last, cx))
                .when(expanded, |list| {
                    list.child(expansion(index, item, last, cx))
                })
        },
    );
    let list = if count == 0 {
        list.child(div().py(px(14.0)).px(px(14.0)).child(text(
            if app.activity.rows.is_empty() {
                "Nothing yet. Links the router handles show up here."
            } else {
                "No dispatch matches."
            },
            11.5,
            400.0,
            theme::text_muted(),
        )))
    } else {
        list
    };
    pane_frame("activity-pane")
        .child(
            div()
                .flex()
                .items_start()
                .gap(px(16.0))
                .child(pane_heading("Recent", "Activity", None))
                .child(
                    button("Clear", ButtonKind::Quiet, true)
                        .id("activity-clear")
                        .debug_selector(|| "activity-clear".into())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.activity.clear();
                            cx.notify();
                        })),
                ),
        )
        .child(toolbar(app, window, cx))
        .child(list)
        .child(note(
            "Full URLs are kept in memory for this session only. Logs on disk carry rule \
             numbers and timings, never URL contents.",
        ))
}

impl PwrRoute {
    /// Re-test: Test a link with this dispatch's link and app.
    pub fn retest(
        &mut self,
        url: &str,
        from: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.test.url.set(url);
        if let Some(index) = fixtures::openers()
            .iter()
            .position(|o| o.is_some_and(|o| o.name == from))
        {
            self.test.opener = index;
        }
        self.navigate(Screen::TestLink, window, cx);
        self.test.run();
        cx.notify();
    }
}
