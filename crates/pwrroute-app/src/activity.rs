//! Activity (design option 2b): the router's recent dispatches, newest
//! first, one expandable to its trace.
//!
//! [`Feed`] is the dispatch history as the window sees it: a GPUI entity
//! that [`install`] fills from the channel `router::start` feeds on every
//! dispatch, and keeps current from it. The channel is the feed's only
//! source. The router's own buffer holds the same records, but every one of
//! them is also on the channel (the sender exists before the router starts),
//! so reading both would show each record dispatched before `install` twice.
//! The window's [`Activity`] observes the feed and draws one [`ActivityRow`]
//! per `DispatchRecord`; the tray observes it for its recents and count. Full
//! URLs stay in memory, as the footer says: nothing here writes one anywhere.

use std::time::SystemTime;

use chrono::{DateTime, Local, NaiveDate};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Div, Entity, Global, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, div, px,
};
use pwrroute_router::{DispatchId, DispatchRecord, Matched, Options, Outcome, RouterHandle, Step};

use crate::app::{PwrRoute, Screen};
use crate::fixtures;
use crate::rules_view::{card, pane_frame, pane_heading};
use crate::text_field::{self, Edit, Look, TextField};
use crate::theme::{self, icon, mono, text, tracked};
use crate::widgets::{
    ButtonKind, NOT_OPENED, NOT_OPENED_BADGE, badge, badge_for, button, note, one_line, paragraph,
    segment, segmented, status, tag,
};

/// The element id of the empty state, for tests.
pub const EMPTY_ID: &str = "activity-empty";

/// The From column when the OS did not say which app sent the link.
const NO_APP: &str = "\u{2014}";

/// How many links were routed on one local day.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Every dispatch, whatever came of it.
    pub routed: u64,
    /// Rejected before routing, or the launch failed.
    pub failed: u64,
}

impl Counts {
    /// The strip chip: "1,412 today · 2 failed".
    pub fn label(&self) -> String {
        let mut label = format!("{} today", thousands(self.routed));
        if self.failed > 0 {
            label.push_str(&format!(" \u{00b7} {} failed", self.failed));
        }
        label
    }

    /// The rail badge: "31", "1.4k", "12k"; nothing before the first link.
    pub fn compact(&self) -> Option<String> {
        Some(match self.routed {
            0 => return None,
            n @ ..1_000 => n.to_string(),
            n @ ..10_000 => {
                let tenths = n / 100;
                if tenths.is_multiple_of(10) {
                    format!("{}k", tenths / 10)
                } else {
                    format!("{}.{}k", tenths / 10, tenths % 10)
                }
            }
            n => format!("{}k", n / 1_000),
        })
    }
}

/// "1,412": a count with thousands separators.
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// The dispatch history behind Activity, the tray's recents and the counts.
pub struct Feed {
    /// Newest first, at most `capacity`: the records the channel delivered,
    /// bounded like the router's buffer.
    records: Vec<DispatchRecord>,
    capacity: usize,
    /// The local day `counts` is for.
    day: Option<NaiveDate>,
    counts: Counts,
    handle: Option<RouterHandle>,
}

impl Feed {
    /// Empty. [`install`] fills it from the channel; the handle is for
    /// [`Feed::clear`]. `None` runs without a router, as the window harness
    /// does.
    pub fn new(handle: Option<RouterHandle>) -> Self {
        Self {
            records: Vec::new(),
            capacity: Options::default().capacity,
            day: None,
            counts: Counts::default(),
            handle,
        }
    }

    /// A feed over `records` (newest first) and no router: tests and
    /// screenshots.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "the window harness and the screens example")
    )]
    pub fn from_records(records: Vec<DispatchRecord>) -> Self {
        let mut feed = Self::new(None);
        for record in records.into_iter().rev() {
            feed.push(record);
        }
        feed
    }

    /// A dispatch just landed.
    pub fn push(&mut self, record: DispatchRecord) {
        let day = local_day(record.received_at);
        if self.day != Some(day) {
            self.day = Some(day);
            self.counts = Counts::default();
        }
        self.counts.routed += 1;
        if record.outcome == Outcome::Failed {
            self.counts.failed += 1;
        }
        self.records.insert(0, record);
        self.records.truncate(self.capacity);
    }

    /// Every record, newest first.
    pub fn records(&self) -> &[DispatchRecord] {
        &self.records
    }

    /// How many the router keeps.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Today's counts, by the local calendar; zero once the day has turned.
    pub fn today(&self, now: SystemTime) -> Counts {
        if self.day == Some(local_day(now)) {
            self.counts
        } else {
            Counts::default()
        }
    }

    /// Forget every record, here and in the router. Today's counts stay:
    /// clearing the list does not un-route the links.
    pub fn clear(&mut self) {
        self.records.clear();
        if let Some(handle) = &self.handle {
            handle.clear_recent();
        }
    }
}

/// The one feed, set by [`install`].
struct FeedGlobal(Entity<Feed>);

impl Global for FeedGlobal {}

/// The feed, once [`install`] has run.
pub fn feed(cx: &App) -> Option<Entity<Feed>> {
    cx.try_global::<FeedGlobal>().map(|feed| feed.0.clone())
}

/// Builds the feed every surface observes from `dispatched`, which
/// `router::start` sends every recorded dispatch down, and keeps it current
/// from there. The router has usually routed links before GPUI gets here
/// (the command line's, and any a second launch forwarded): those are
/// already on the channel, oldest first, so they are taken now and the first
/// frame shows them, each once.
pub fn install(
    handle: RouterHandle,
    dispatched: async_channel::Receiver<DispatchRecord>,
    cx: &mut App,
) {
    let mut feed = Feed::new(Some(handle));
    while let Ok(record) = dispatched.try_recv() {
        feed.push(record);
    }
    let feed = install_feed(feed, cx);
    cx.spawn(async move |cx| {
        while let Ok(record) = dispatched.recv().await {
            cx.update(|cx| {
                feed.update(cx, |feed, cx| {
                    feed.push(record);
                    cx.notify();
                });
            });
        }
    })
    .detach();
}

/// Makes `feed` the one the window and tray observe. Tests feed it by hand.
pub fn install_feed(feed: Feed, cx: &mut App) -> Entity<Feed> {
    let feed = cx.new(|_| feed);
    cx.set_global(FeedGlobal(feed.clone()));
    feed
}

fn local_day(at: SystemTime) -> NaiveDate {
    DateTime::<Local>::from(at).date_naive()
}

/// "09:41:08", local time.
fn clock(at: SystemTime) -> String {
    DateTime::<Local>::from(at).format("%H:%M:%S").to_string()
}

/// One dispatch.
#[derive(Clone, Debug)]
pub struct ActivityRow {
    pub id: DispatchId,
    pub time: SharedString,
    /// The link as received.
    pub url: SharedString,
    /// The app it came from.
    pub from: SharedString,
    /// The rules that fired, short: "rw 1, 2 · h 3".
    pub rule: SharedString,
    pub opened: Opened,
    pub took: SharedString,
    /// A rewrite ran.
    pub rewritten: bool,
    /// No handler matched.
    pub default: bool,
    /// Resolved and recorded, nothing opened (`--dry-run`).
    pub dry_run: bool,
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
    /// The config chose `browser: null`.
    Nowhere,
    /// Rejected, or the launch failed, with why.
    Failed(SharedString),
}

/// A line of an expanded row's trace.
#[derive(Clone, Debug)]
pub struct ActivityStep {
    pub mark: StepMark,
    pub text: SharedString,
}

/// `.ax .l .n`: the line's numbered circle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepMark {
    /// A rule fired.
    Hit(usize),
    /// Nothing matched, or a fallback stood in.
    Miss(usize),
    /// The link opened.
    Done,
    /// It did not.
    Fail,
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
    /// One record as the screen draws it.
    pub fn from_record(record: &DispatchRecord) -> Self {
        let received = received_url(record);
        let from = record
            .link
            .opener
            .as_ref()
            .map(|opener| opener.name.as_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(NO_APP);
        let matched = record.steps.iter().find_map(|step| match step {
            Step::Matched { by } => Some(*by),
            _ => None,
        });
        Self {
            id: record.id,
            time: clock(record.received_at).into(),
            url: received.into(),
            from: from.into(),
            rule: rule_label(&record.steps).into(),
            opened: opened(record),
            took: took_label(record.timings.total_us).into(),
            rewritten: record
                .steps
                .iter()
                .any(|step| matches!(step, Step::Rewrite { .. })),
            default: matched == Some(Matched::Default),
            dry_run: record
                .steps
                .iter()
                .any(|step| matches!(step, Step::Launched { dry_run: true })),
            steps: trace_lines(record),
            final_url: record.final_url.as_deref().unwrap_or(received).into(),
        }
    }

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
            Opened::Nowhere => NOT_OPENED.to_owned(),
            Opened::Failed(why) => why.to_string(),
        };
        [self.url.as_ref(), self.from.as_ref(), opened.as_str()]
            .iter()
            .any(|field| field.to_lowercase().contains(&needle))
    }
}

/// The URL as received, once trimmed; the raw link when it was rejected
/// before that.
fn received_url(record: &DispatchRecord) -> &str {
    record
        .steps
        .iter()
        .find_map(|step| match step {
            Step::Received { url } => Some(url.as_str()),
            _ => None,
        })
        .unwrap_or(&record.link.url)
}

/// The Rule column: "rw 1, 2 · h 3", "handler 2", "default", "fallback".
fn rule_label(steps: &[Step]) -> String {
    let mut numbered = Vec::new();
    let mut unnumbered = false;
    for step in steps {
        match step {
            Step::Rewrite {
                rule: Some(rule), ..
            } => numbered.push((rule + 1).to_string()),
            Step::Rewrite { rule: None, .. } => unnumbered = true,
            _ => {}
        }
    }
    let mut parts: Vec<String> = Vec::new();
    if !numbered.is_empty() {
        parts.push(format!("rw {}", numbered.join(", ")));
    } else if unnumbered {
        parts.push("rw".into());
    }
    for step in steps {
        match step {
            Step::Matched {
                by: Matched::Handler { index },
            } => parts.push(if parts.is_empty() {
                format!("handler {}", index + 1)
            } else {
                format!("h {}", index + 1)
            }),
            Step::Matched {
                by: Matched::Default,
            } => parts.push("default".into()),
            // A rules.json handler: the router does not number those.
            Step::Matched {
                by: Matched::Unreported,
            } => parts.push("handler".into()),
            Step::Fallback { .. } | Step::RefusedLoop { .. } => parts.push("fallback".into()),
            Step::Rejected { .. } => parts.push("rejected".into()),
            _ => {}
        }
    }
    if parts.is_empty() {
        NO_APP.into()
    } else {
        parts.join(" \u{00b7} ")
    }
}

fn opened(record: &DispatchRecord) -> Opened {
    for step in &record.steps {
        match step {
            Step::Rejected { reason } => return Opened::Failed(reason.clone().into()),
            Step::LaunchFailed { message } => return Opened::Failed(message.clone().into()),
            Step::NotOpened => return Opened::Nowhere,
            _ => {}
        }
    }
    match &record.browser {
        Some(browser) => Opened::Browser {
            badge: badge_for(&browser.name),
            name: browser.name.clone().into(),
            profile: (!browser.profile.is_empty()).then(|| browser.profile.clone().into()),
        },
        None => Opened::Failed("not routed".into()),
    }
}

/// "4 ms": the whole dispatch, to the millisecond.
fn took_label(total_us: u64) -> String {
    format!("{} ms", (total_us + 500) / 1_000)
}

/// "0.6 ms", or microseconds under a tenth of a millisecond.
fn millis(us: u64) -> String {
    if us < 100 {
        format!("{us} \u{00b5}s")
    } else {
        format!("{:.1} ms", us as f64 / 1_000.0)
    }
}

/// "Firefox · default-release", or the name alone.
fn browser_label(record: &DispatchRecord) -> String {
    match &record.browser {
        Some(browser) if browser.profile.is_empty() => browser.name.clone(),
        Some(browser) => format!("{} \u{00b7} {}", browser.name, browser.profile),
        None => NOT_OPENED.to_owned(),
    }
}

/// The expanded row's lines: one per step that did something.
fn trace_lines(record: &DispatchRecord) -> Vec<ActivityStep> {
    let mut lines: Vec<ActivityStep> = Vec::new();
    let mut url = received_url(record);
    for step in &record.steps {
        let number = lines.len() + 1;
        let (mark, line) = match step {
            // The row itself shows the link and where it went.
            Step::Received { .. } | Step::Browser { .. } => continue,
            Step::Rewrite { rule, url: after } => {
                let name = match rule {
                    Some(rule) => format!("rewrite {}", rule + 1),
                    None => "rewrite".to_owned(),
                };
                let line = format!("{name} \u{00b7} {}", describe_rewrite(url, after));
                url = after;
                (StepMark::Hit(number), line)
            }
            Step::Matched { by } => (
                StepMark::Hit(number),
                match by {
                    Matched::Handler { index } => format!("handler {}", index + 1),
                    Matched::Default => "default \u{00b7} no handler matched".to_owned(),
                    Matched::Unreported => "a handler matched".to_owned(),
                },
            ),
            Step::ConfigError { message } => {
                (StepMark::Fail, format!("config error \u{00b7} {message}"))
            }
            Step::Fallback { reason } => (
                StepMark::Miss(number),
                format!("fallback \u{00b7} {reason}"),
            ),
            Step::RefusedLoop { browser } => (
                StepMark::Miss(number),
                format!("{browser} is PwrRoute \u{00b7} fallback instead"),
            ),
            Step::NotOpened => (
                StepMark::Miss(number),
                "not opened \u{00b7} the rule says browser: null".to_owned(),
            ),
            Step::Launched { dry_run } => {
                let mut line = format!(
                    "{} \u{00b7} resolved {} \u{00b7} launched {}",
                    browser_label(record),
                    millis(record.timings.resolve_us),
                    millis(record.timings.launch_us)
                );
                if *dry_run {
                    line.push_str(" \u{00b7} dry run");
                }
                (StepMark::Done, line)
            }
            Step::LaunchFailed { message } => {
                (StepMark::Fail, format!("launch failed \u{00b7} {message}"))
            }
            Step::Rejected { reason } => (StepMark::Fail, format!("rejected \u{00b7} {reason}")),
        };
        lines.push(ActivityStep {
            mark,
            text: line.into(),
        });
    }
    lines
}

/// What a rewrite changed: "host → xcancel.com", "dropped utm_source",
/// "scheme → https", "path → /new".
fn describe_rewrite(before: &str, after: &str) -> String {
    let (Ok(a), Ok(b)) = (url::Url::parse(before), url::Url::parse(after)) else {
        return format!("url \u{2192} {after}");
    };
    let mut changes = Vec::new();
    if a.scheme() != b.scheme() {
        changes.push(format!("scheme \u{2192} {}", b.scheme()));
    }
    let host = |u: &url::Url| match (u.host_str(), u.port()) {
        (Some(host), Some(port)) => Some(format!("{host}:{port}")),
        (Some(host), None) => Some(host.to_owned()),
        (None, _) => None,
    };
    if host(&a) != host(&b) {
        changes.push(format!("host \u{2192} {}", host(&b).unwrap_or_default()));
    }
    if a.path() != b.path() {
        changes.push(format!("path \u{2192} {}", b.path()));
    }
    if a.query() != b.query() {
        let kept: Vec<String> = b.query_pairs().map(|(k, _)| k.into_owned()).collect();
        let dropped: Vec<String> = a
            .query_pairs()
            .map(|(k, _)| k.into_owned())
            .filter(|k| !kept.contains(k))
            .collect();
        if dropped.is_empty() {
            changes.push("query changed".to_owned());
        } else {
            changes.push(format!("dropped {}", dropped.join(", ")));
        }
    }
    if changes.is_empty() {
        "matched, unchanged".to_owned()
    } else {
        changes.join(" \u{00b7} ")
    }
}

/// The screen's state.
pub struct Activity {
    pub rows: Vec<ActivityRow>,
    pub expanded: Option<DispatchId>,
    pub filter: Filter,
    pub search: TextField,
    pub today: Counts,
    /// How many the router keeps, for the toolbar's note.
    pub capacity: usize,
    _feed: Option<Subscription>,
}

impl Activity {
    /// Follows the feed when one is installed; empty otherwise.
    pub fn new(cx: &mut Context<'_, PwrRoute>) -> Self {
        let mut activity = Self {
            rows: Vec::new(),
            expanded: None,
            filter: Filter::All,
            search: TextField::new("", cx),
            today: Counts::default(),
            capacity: Options::default().capacity,
            _feed: None,
        };
        if let Some(feed) = feed(cx) {
            activity.sync(feed.read(cx));
            activity._feed = Some(cx.observe(&feed, |this, feed, cx| {
                this.activity.sync(feed.read(cx));
                cx.notify();
            }));
        }
        activity
    }

    /// The rows and counts as the feed has them now. The expanded row stays
    /// expanded while it is still listed.
    pub fn sync(&mut self, feed: &Feed) {
        self.rows = feed
            .records()
            .iter()
            .map(ActivityRow::from_record)
            .collect();
        self.today = feed.today(SystemTime::now());
        self.capacity = feed.capacity();
        if self
            .expanded
            .is_some_and(|id| !self.rows.iter().any(|row| row.id == id))
        {
            self.expanded = None;
        }
    }

    /// Rows from records outside any feed: tests and screenshots.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "the window harness and the screens example")
    )]
    pub fn load(&mut self, records: Vec<DispatchRecord>) {
        self.sync(&Feed::from_records(records));
    }

    /// The rows the filter and search let through, with their indexes.
    pub fn visible(&self) -> Vec<(usize, &ActivityRow)> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.passes(self.filter, &self.search.text))
            .collect()
    }

    pub fn toggle(&mut self, id: DispatchId) {
        self.expanded = if self.expanded == Some(id) {
            None
        } else {
            Some(id)
        };
    }

    /// Clear: here, in the feed and in the router.
    pub fn clear(&mut self, cx: &mut App) {
        self.rows.clear();
        self.expanded = None;
        if let Some(feed) = feed(cx) {
            feed.update(cx, |feed, cx| {
                feed.clear();
                cx.notify();
            });
        }
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
    one_line(content, family, weight, size, size, color)
}

fn opened_cell(opened: &Opened, dry_run: bool) -> Div {
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
            })
            .when(dry_run, |cell| cell.child(tag("dry run"))),
        Opened::Nowhere => div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(badge(NOT_OPENED_BADGE))
            .child(text(NOT_OPENED, 11.5, 400.0, theme::text_muted())),
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
    let id = row.id;
    grid_row([
        div().child(mono(row.time.clone(), 10.5, 500.0, theme::text_subtle())),
        clipped(row.url.clone(), true, 11.5, link_color),
        clipped(row.from.clone(), false, 11.5, theme::text_secondary()),
        div().flex().child(tag(row.rule.clone())),
        opened_cell(&row.opened, row.dry_run),
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
        this.activity.toggle(id);
        cx.notify();
    }))
}

/// `.ax`: the expanded row's trace and actions.
fn expansion(index: usize, row: &ActivityRow, last: bool, cx: &mut Context<'_, PwrRoute>) -> Div {
    let line = |mark: StepMark, content: SharedString| {
        let (label, fill, color) = match mark {
            StepMark::Hit(n) => (n.to_string(), theme::accent_soft(), theme::accent_bright()),
            StepMark::Miss(n) => (n.to_string(), theme::bg_elevated(), theme::text_subtle()),
            StepMark::Done => (
                "\u{2713}".to_owned(),
                theme::success_soft(),
                theme::success_text(),
            ),
            StepMark::Fail => ("!".to_owned(), theme::danger_soft(), theme::danger_text()),
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
        .child(div().ml_auto().child(note(format!(
            "last {} \u{00b7} in memory",
            app.activity.capacity
        ))))
}

/// `.empty`: nothing has been routed yet (`design/PwrRoute Empty and Error
/// States.dc.html`'s empty block, with Activity's words).
fn empty_state(cx: &mut Context<'_, PwrRoute>) -> gpui_kit::Stateful<Div> {
    div()
        .id(EMPTY_ID)
        .debug_selector(|| EMPTY_ID.into())
        .flex()
        .items_start()
        .gap(px(14.0))
        .pt(px(16.0))
        .px(px(16.0))
        .pb(px(18.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    text("No links yet", 12.5, 600.0, theme::text_primary()).line_height(px(16.25)),
                )
                .child(
                    paragraph(
                        "Every link the router handles lands here, newest first: the rule that \
                         fired, where it opened and how long it took. Full URLs stay in memory \
                         for this session.",
                        11.5,
                        17.8,
                        theme::text_muted(),
                    )
                    .max_w(px(520.0)),
                ),
        )
        .child(
            button("Test a link\u{2026}", ButtonKind::Default, true)
                .id("activity-empty-test")
                .debug_selector(|| "activity-empty-test".into())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.navigate(Screen::TestLink, window, cx);
                })),
        )
}

pub fn pane(
    app: &PwrRoute,
    window: &mut Window,
    cx: &mut Context<'_, PwrRoute>,
) -> impl IntoElement + use<> {
    let visible = app.activity.visible();
    let count = visible.len();
    let list = if app.activity.rows.is_empty() {
        card().child(empty_state(cx))
    } else {
        let list = visible.into_iter().enumerate().fold(
            card().child(head_row()),
            |list, (position, (index, item))| {
                let last = position + 1 == count;
                let expanded = app.activity.expanded == Some(item.id);
                list.child(row(index, item, expanded, last, cx))
                    .when(expanded, |list| {
                        list.child(expansion(index, item, last, cx))
                    })
            },
        );
        if count == 0 {
            list.child(div().py(px(14.0)).px(px(14.0)).child(text(
                "No dispatch matches.",
                11.5,
                400.0,
                theme::text_muted(),
            )))
        } else {
            list
        }
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
                            this.activity.clear(cx);
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
        // An app the picker does not offer tests as no app, never as
        // whichever app was picked before.
        self.test.opener = fixtures::OPENERS
            .iter()
            .position(|o| o.is_some_and(|o| o.name == from))
            .unwrap_or(0);
        // Run first, so opening the screen does not run its sample too.
        self.test.run();
        self.navigate(Screen::TestLink, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use pwrroute_router::{
        AppType, Browser, DispatchId, DispatchRecord, Link, Matched, OpenerInfo, Outcome, Source,
        Step, Timings,
    };

    use super::*;

    fn browser(name: &str, profile: &str, url: &str) -> Browser {
        Browser {
            name: name.into(),
            app_type: AppType::AppName,
            url: url.into(),
            profile: profile.into(),
            args: Vec::new(),
            open_in_background: None,
        }
    }

    /// A record with these steps, as the router would fill the rest in.
    fn record(id: u64, url: &str, from: Option<&str>, steps: Vec<Step>) -> DispatchRecord {
        let browser = steps.iter().find_map(|step| match step {
            Step::Browser { browser } => Some(browser.clone()),
            _ => None,
        });
        let outcome = steps
            .iter()
            .rev()
            .find_map(|step| match step {
                Step::Launched { dry_run: true } => Some(Outcome::DryRun),
                Step::Launched { dry_run: false } => Some(Outcome::Opened),
                Step::NotOpened => Some(Outcome::NotOpened),
                Step::LaunchFailed { .. } | Step::Rejected { .. } => Some(Outcome::Failed),
                _ => None,
            })
            .unwrap_or(Outcome::Failed);
        DispatchRecord {
            id: DispatchId(id),
            received_at: SystemTime::now(),
            link: Link::new(url, Source::AppleEvent).with_opener(from.map(|name| OpenerInfo {
                name: name.into(),
                ..OpenerInfo::default()
            })),
            forwarded: false,
            final_url: browser.as_ref().map(|b| b.url.clone()),
            browser,
            steps,
            outcome,
            timings: Timings {
                queue_us: 40,
                resolve_us: 600,
                launch_us: 5_100,
                total_us: 5_740,
            },
        }
    }

    fn texts(row: &ActivityRow) -> Vec<&str> {
        row.steps.iter().map(|step| step.text.as_ref()).collect()
    }

    #[test]
    fn a_rewritten_handler_dispatch_fills_every_column() {
        let received = "https://x.com/pwrdrvr/status/1842?utm_source=slack&s=20";
        let after_host = "https://xcancel.com/pwrdrvr/status/1842?utm_source=slack&s=20";
        let opened = "https://xcancel.com/pwrdrvr/status/1842?s=20";
        let record = record(
            7,
            received,
            Some("Slack"),
            vec![
                Step::Received {
                    url: received.into(),
                },
                Step::Rewrite {
                    rule: Some(0),
                    url: after_host.into(),
                },
                Step::Rewrite {
                    rule: Some(1),
                    url: opened.into(),
                },
                Step::Matched {
                    by: Matched::Handler { index: 2 },
                },
                Step::Browser {
                    browser: browser("Firefox", "default-release", opened),
                },
                Step::Launched { dry_run: false },
            ],
        );
        let row = ActivityRow::from_record(&record);
        assert_eq!(row.id, DispatchId(7));
        assert_eq!(row.time.len(), "09:41:08".len());
        assert_eq!(row.url, received);
        assert_eq!(row.from, "Slack");
        assert_eq!(row.rule, "rw 1, 2 \u{00b7} h 3");
        assert_eq!(row.took, "6 ms");
        assert!(row.rewritten && !row.default && !row.dry_run && !row.failed());
        assert_eq!(row.final_url, opened);
        let Opened::Browser {
            badge,
            name,
            profile,
        } = &row.opened
        else {
            panic!("opened in a browser");
        };
        assert_eq!((badge.as_ref(), name.as_ref()), ("Ff", "Firefox"));
        assert_eq!(profile.as_deref(), Some("default-release"));
        assert_eq!(
            texts(&row),
            [
                "rewrite 1 \u{00b7} host \u{2192} xcancel.com",
                "rewrite 2 \u{00b7} dropped utm_source",
                "handler 3",
                "Firefox \u{00b7} default-release \u{00b7} resolved 0.6 ms \u{00b7} launched 5.1 ms",
            ]
        );
        assert_eq!(
            row.steps.iter().map(|s| s.mark).collect::<Vec<_>>(),
            [
                StepMark::Hit(1),
                StepMark::Hit(2),
                StepMark::Hit(3),
                StepMark::Done
            ]
        );
    }

    #[test]
    fn the_default_a_dry_run_and_no_app_read_as_such() {
        let url = "https://www.apple.com/macos/";
        let record = record(
            1,
            url,
            None,
            vec![
                Step::Received { url: url.into() },
                Step::Matched {
                    by: Matched::Default,
                },
                Step::Browser {
                    browser: browser("Google Chrome", "Personal", url),
                },
                Step::Launched { dry_run: true },
            ],
        );
        let row = ActivityRow::from_record(&record);
        assert_eq!(row.from, NO_APP);
        assert_eq!(row.rule, "default");
        assert!(row.default && row.dry_run && !row.rewritten);
        assert_eq!(
            texts(&row)[1],
            "Google Chrome \u{00b7} Personal \u{00b7} resolved 0.6 ms \u{00b7} launched 5.1 ms \
             \u{00b7} dry run"
        );
        assert!(row.passes(Filter::Default, "chrome"));
        assert!(!row.passes(Filter::Rewritten, ""));
    }

    #[test]
    fn failures_name_their_reason() {
        let url = "https://zoom.us/j/8142201137";
        let failed = record(
            2,
            url,
            Some("Calendar"),
            vec![
                Step::Received { url: url.into() },
                Step::Matched {
                    by: Matched::Handler { index: 5 },
                },
                Step::Browser {
                    browser: browser("zoom.us", "", url),
                },
                Step::LaunchFailed {
                    message: "zoom.us not found".into(),
                },
            ],
        );
        let row = ActivityRow::from_record(&failed);
        assert!(row.failed());
        assert!(matches!(&row.opened, Opened::Failed(why) if why == "zoom.us not found"));
        assert_eq!(row.rule, "handler 6");
        assert_eq!(
            texts(&row),
            ["handler 6", "launch failed \u{00b7} zoom.us not found"]
        );
        assert_eq!(row.steps[1].mark, StepMark::Fail);
        assert!(row.passes(Filter::Failed, "not found"));

        let rejected = record(
            3,
            "   ",
            None,
            vec![Step::Rejected {
                reason: "the link is empty".into(),
            }],
        );
        let row = ActivityRow::from_record(&rejected);
        assert_eq!(row.rule, "rejected");
        assert!(matches!(&row.opened, Opened::Failed(why) if why == "the link is empty"));

        let no_config = record(
            4,
            url,
            None,
            vec![
                Step::Received { url: url.into() },
                Step::Fallback {
                    reason: "no config is selected".into(),
                },
                Step::Browser {
                    browser: browser("Safari", "", url),
                },
                Step::Launched { dry_run: false },
            ],
        );
        let row = ActivityRow::from_record(&no_config);
        assert_eq!(row.rule, "fallback");
        assert_eq!(row.steps[0].mark, StepMark::Miss(1));
        assert_eq!(texts(&row)[0], "fallback \u{00b7} no config is selected");

        let not_opened = record(
            5,
            url,
            None,
            vec![
                Step::Received { url: url.into() },
                Step::Matched {
                    by: Matched::Unreported,
                },
                Step::NotOpened,
            ],
        );
        let row = ActivityRow::from_record(&not_opened);
        assert!(matches!(row.opened, Opened::Nowhere));
        assert_eq!(row.rule, "handler");
        assert!(!row.failed());
    }

    #[test]
    fn rewrites_say_what_changed() {
        assert_eq!(
            describe_rewrite("http://wiki.internal/start", "https://wiki.internal/start"),
            "scheme \u{2192} https"
        );
        assert_eq!(
            describe_rewrite(
                "https://a.example/old?x=1",
                "https://b.example:8443/new?x=1"
            ),
            "host \u{2192} b.example:8443 \u{00b7} path \u{2192} /new"
        );
        assert_eq!(
            describe_rewrite("https://a.example/?x=1", "https://a.example/?x=2"),
            "query changed"
        );
        assert_eq!(
            describe_rewrite("https://a.example/", "https://a.example/"),
            "matched, unchanged"
        );
        assert_eq!(
            describe_rewrite("not a url", "https://a.example/"),
            "url \u{2192} https://a.example/"
        );
    }

    #[test]
    fn the_feed_counts_today_and_keeps_the_newest() {
        let url = "https://a.example/";
        let step = |id: u64, failed: bool| {
            let mut steps = vec![
                Step::Received { url: url.into() },
                Step::Matched {
                    by: Matched::Default,
                },
                Step::Browser {
                    browser: browser("Safari", "", url),
                },
            ];
            steps.push(if failed {
                Step::LaunchFailed {
                    message: "no".into(),
                }
            } else {
                Step::Launched { dry_run: false }
            });
            record(id, url, None, steps)
        };
        let mut feed = Feed::from_records(vec![step(3, true), step(2, false), step(1, false)]);
        let now = SystemTime::now();
        assert_eq!(
            feed.today(now),
            Counts {
                routed: 3,
                failed: 1
            }
        );
        assert_eq!(feed.today(now).label(), "3 today \u{00b7} 1 failed");
        let ids: Vec<u64> = feed.records().iter().map(|r| r.id.0).collect();
        assert_eq!(ids, [3, 2, 1]);

        // Yesterday's links do not count for today.
        let mut old = step(0, false);
        old.received_at = now - Duration::from_secs(2 * 86_400);
        let yesterday = Feed::from_records(vec![old]);
        assert_eq!(yesterday.today(now), Counts::default());
        assert_eq!(yesterday.today(now).label(), "0 today");

        // The buffer is bounded like the router's.
        for id in 10..(10 + feed.capacity() as u64) {
            feed.push(step(id, false));
        }
        assert_eq!(feed.records().len(), feed.capacity());
        assert_eq!(feed.records()[0].id, DispatchId(9 + feed.capacity() as u64));
        assert_eq!(feed.today(now).routed, 3 + feed.capacity() as u64);

        // Clearing forgets the list, not the day's count.
        feed.clear();
        assert!(feed.records().is_empty());
        assert_eq!(feed.today(now).routed, 3 + feed.capacity() as u64);
    }

    #[test]
    fn counts_read_as_the_design_writes_them() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1412), "1,412");
        assert_eq!(thousands(1_234_567), "1,234,567");
        let counts = |routed| Counts { routed, failed: 0 };
        assert_eq!(counts(0).compact(), None);
        assert_eq!(counts(31).compact().as_deref(), Some("31"));
        assert_eq!(counts(999).compact().as_deref(), Some("999"));
        assert_eq!(counts(1_000).compact().as_deref(), Some("1k"));
        assert_eq!(counts(1_412).compact().as_deref(), Some("1.4k"));
        assert_eq!(counts(9_950).compact().as_deref(), Some("9.9k"));
        assert_eq!(counts(12_345).compact().as_deref(), Some("12k"));
        assert_eq!(
            Counts {
                routed: 1412,
                failed: 2
            }
            .label(),
            "1,412 today \u{00b7} 2 failed"
        );
    }
}
