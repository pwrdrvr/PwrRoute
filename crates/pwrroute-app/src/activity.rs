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
//! per `DispatchRecord` kept at `Full` or `Host`; a record kept at `None`
//! is a count, never a row. The tray observes it for its recents and count.
//! The footer and the toolbar's note are the router's own words about
//! where links are kept (`RouterHandle::retention`), never a literal here.
//! When the journal is on, journaled records that are no longer in memory
//! are listed below the in-memory ones, under a divider. Nothing here
//! writes a URL anywhere: Export… hands over to Settings' producer, which
//! writes only to the file the reader picks. The journal is read from disk
//! only when its files changed since the last read ([`JournalCache`]): a
//! sync happens on every dispatch and every Settings change, and most of
//! them find the files as they were.
//!
//! The row's "⋯" opens [`row_menu`] (`activity_actions.rs`): Add a rule
//! for this link, Save as example or Save as test, over the record the
//! [`RowKey`] names; the expanded row's "Add a rule…" is
//! [`PwrRoute::add_rule`], the same sheet.

use std::cell::RefCell;
use std::collections::HashSet;
use std::io;
use std::time::SystemTime;

use chrono::{DateTime, Local, NaiveDate};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, Entity, Global, InteractiveElement as _,
    IntoElement, KeyDownEvent, ParentElement as _, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, px,
};
use pwrroute_router::{
    DispatchId, DispatchRecord, JournalConfig, JournalStamp, Matched, Options, Outcome, Privacy,
    RetentionInfo, RouterHandle, Step,
};

use crate::app::{PwrRoute, Screen};
use crate::default_browser::browser_name;
use crate::fixtures;
use crate::rules_view::{card, pane_frame, pane_heading};
use crate::text_field::{self, Edit, Look, TextField};
use crate::theme::{self, icon, mono, text, tracked};
use crate::widgets::{
    ButtonKind, NOT_OPENED, NOT_OPENED_BADGE, Tone, badge, badge_for, button, chip, note, one_line,
    paragraph, segment, segmented, status, tag, toned_chip,
};

/// The element id of the empty state, for tests.
pub const EMPTY_ID: &str = "activity-empty";

/// The From column when the OS did not say which app sent the link.
const NO_APP: &str = "\u{2014}";

/// How many journaled records the screen reads back, at most: the design's
/// list is one screen of recent links, not the whole week.
const JOURNAL_ROWS: usize = 500;

/// How many links were routed on one local day.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Every dispatch, whatever came of it.
    pub routed: u64,
    /// Rejected before routing, or the launch failed.
    pub failed: u64,
    /// No handler matched: the default or the fallback browser took it.
    pub unrouted: u64,
    /// A rewrite ran.
    pub rewritten: u64,
    /// Kept at host only or nothing: the rule that took it keeps less.
    pub private: u64,
}

impl Counts {
    /// The strip chip: "1,412 today · 2 failed · 14 private".
    pub fn label(&self) -> String {
        let mut label = format!("{} today", thousands(self.routed));
        if self.failed > 0 {
            label.push_str(&format!(" \u{00b7} {} failed", self.failed));
        }
        if self.private > 0 {
            label.push_str(&format!(" \u{00b7} {} private", self.private));
        }
        label
    }

    /// Adds one record to the day.
    fn count(&mut self, record: &DispatchRecord) {
        self.routed += 1;
        if record.outcome == Outcome::Failed {
            self.failed += 1;
        }
        if is_unrouted(record) {
            self.unrouted += 1;
        }
        if is_rewritten(record) {
            self.rewritten += 1;
        }
        if record.privacy != Privacy::Full {
            self.private += 1;
        }
    }

    /// The count a filter shows beside its label.
    pub fn of(&self, filter: Filter) -> u64 {
        match filter {
            Filter::All => self.routed,
            Filter::Unrouted => self.unrouted,
            Filter::Failed => self.failed,
            Filter::Rewritten => self.rewritten,
            Filter::Private => self.private,
        }
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
    /// Newest first, at most [`capacity`](Self::capacity): the records the
    /// channel delivered, bounded like the router's buffer.
    records: Vec<DispatchRecord>,
    /// The bound without a router; with one, the router's retention count.
    capacity: usize,
    /// The local day `counts` is for.
    day: Option<NaiveDate>,
    counts: Counts,
    handle: Option<RouterHandle>,
    /// The journal as last read, so a sync that finds its files unchanged
    /// lists the same records without reading them again.
    journal_cache: RefCell<JournalCache>,
}

/// The journal's records as last read, and the [`JournalStamp`] its files
/// had then. A read takes the stamp first and reads the files only when it
/// differs from the last one (or the journal, or the limit, is not what
/// was read), so a week of journal is read once, not on every sync.
#[derive(Default)]
struct JournalCache {
    /// The journal read, how many records were asked for, and its files'
    /// stamp before the read.
    read_as: Option<(JournalConfig, usize, JournalStamp)>,
    /// Newest first, at most the limit in `read_as`.
    records: Vec<DispatchRecord>,
}

impl JournalCache {
    /// Brings `records` up to date with `config`, reading its files only
    /// when their stamp changed since the last read. `true` when they were
    /// read. The stamp is taken before the read, so a line appended during
    /// the read is caught by the next stamp.
    fn refresh(&mut self, config: &JournalConfig, limit: usize) -> io::Result<bool> {
        let stamp = config.stamp()?;
        if self
            .read_as
            .as_ref()
            .is_some_and(|(read, n, was)| read == config && *n == limit && *was == stamp)
        {
            return Ok(false);
        }
        self.records = config.read(limit)?;
        self.read_as = Some((config.clone(), limit, stamp));
        Ok(true)
    }

    fn records(&self) -> &[DispatchRecord] {
        &self.records
    }

    /// Forgets the last read: the next refresh reads.
    fn forget(&mut self) {
        *self = Self::default();
    }
}

impl Feed {
    /// Empty. [`install`] fills it from the channel; the handle is for
    /// [`Feed::clear`]. `None` runs without a router, as the window harness
    /// does.
    pub fn new(handle: Option<RouterHandle>) -> Self {
        Self {
            records: Vec::new(),
            capacity: Options::default().retention.count,
            day: None,
            counts: Counts::default(),
            handle,
            journal_cache: RefCell::new(JournalCache::default()),
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
        self.counts.count(&record);
        self.records.insert(0, record);
        let capacity = self.capacity();
        self.records.truncate(capacity);
    }

    /// Takes the router's buffer as the list, after its retention changed
    /// or Settings cleared it: the buffer is swept to the new retention,
    /// and every record in it was on the channel once, so nothing is lost
    /// or doubled. Today's counts stay.
    pub fn reload(&mut self) {
        if let Some(handle) = &self.handle {
            self.records = handle.recent();
        }
    }

    /// Every record, newest first.
    pub fn records(&self) -> &[DispatchRecord] {
        &self.records
    }

    /// How many the router keeps: its retention count as set now.
    pub fn capacity(&self) -> usize {
        match &self.handle {
            Some(handle) => handle.retention().memory.count,
            None => self.capacity,
        }
    }

    /// Where the router keeps dispatches and for how long, in its own
    /// words. Without a router (tests, screenshots), the defaults.
    pub fn retention(&self) -> RetentionInfo {
        match &self.handle {
            Some(handle) => handle.retention(),
            None => {
                let options = Options::default();
                RetentionInfo {
                    memory: options.retention,
                    journal: None,
                    description: options.retention_description(),
                }
            }
        }
    }

    /// Journaled dispatches, newest first, at most `limit`: empty while
    /// the journal is off. Read from disk when the journal's files changed
    /// since the last read, else answered from that read.
    pub fn journal(&self, limit: usize) -> Vec<DispatchRecord> {
        self.with_journal(limit, <[DispatchRecord]>::to_vec)
    }

    /// `f` over the journal's records, newest first, at most `limit`, read
    /// again only when the files changed (see [`JournalCache`]). An empty
    /// list while the journal is off, or when it cannot be read (reported
    /// on stderr, without link data).
    fn with_journal<T>(&self, limit: usize, f: impl FnOnce(&[DispatchRecord]) -> T) -> T {
        let Some(handle) = &self.handle else {
            return f(&[]);
        };
        let mut cache = self.journal_cache.borrow_mut();
        let Some(config) = handle.retention().journal else {
            cache.forget();
            return f(&[]);
        };
        match cache.refresh(&config, limit) {
            Ok(_) => f(cache.records()),
            Err(error) => {
                eprintln!("pwrroute: cannot read the activity journal: {error}");
                cache.forget();
                f(&[])
            }
        }
    }

    /// The record behind a row: from memory, or from the journal for a
    /// journaled row (read again only when its files changed, like
    /// [`journal`](Self::journal)).
    pub fn record(&self, key: RowKey) -> Option<DispatchRecord> {
        if key.journaled {
            self.with_journal(JOURNAL_ROWS, |records| {
                records.iter().find(|record| record.id == key.id).cloned()
            })
        } else {
            self.records
                .iter()
                .find(|record| record.id == key.id)
                .cloned()
        }
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

/// Which row: a dispatch's id, and whether it came from the journal. Ids
/// are unique within one router process, so a journaled record from an
/// earlier run can repeat an in-memory one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RowKey {
    pub id: DispatchId,
    pub journaled: bool,
}

/// One dispatch.
#[derive(Clone, Debug)]
pub struct ActivityRow {
    pub key: RowKey,
    /// The link as received: scheme and host only for a host-only record.
    pub time: SharedString,
    pub url: SharedString,
    /// The app it came from.
    pub from: SharedString,
    /// The rules that fired, short: "rw 1, 2 · h 3".
    pub rule: SharedString,
    pub opened: Opened,
    pub took: SharedString,
    /// A rewrite ran.
    pub rewritten: bool,
    /// No rule routed it: the default, the fallback or a pause took it.
    pub default: bool,
    /// Resolved and recorded, nothing opened (`--dry-run`).
    pub dry_run: bool,
    /// The trace, for the expanded row.
    pub steps: Vec<ActivityStep>,
    /// The URL the rules saw (`matched_url`): where a short link landed,
    /// else `url`. Re-test tries this one.
    pub matched_url: SharedString,
    /// The URL the browser was asked to open.
    pub final_url: SharedString,
    /// How much of the link the record keeps: `Host` draws the "host only"
    /// chip. Never `None`: those records are counted, not listed.
    pub privacy: Privacy,
    /// Listed from the journal, below the in-memory rows.
    pub journaled: bool,
}

/// Where a dispatch went.
#[derive(Clone, Debug)]
pub enum Opened {
    Browser {
        badge: SharedString,
        name: SharedString,
        profile: Option<SharedString>,
        /// The rule asked for a private window.
        private: bool,
    },
    /// The config chose `browser: null`.
    Nowhere,
    /// The chosen browser could not be opened and the fallback browser took
    /// the link: the design's "zoom.us not found ↪ opened in Chrome".
    Fallback {
        /// Why, as the router traced it: "Arc could not be opened".
        why: SharedString,
        badge: SharedString,
        name: SharedString,
        profile: Option<SharedString>,
    },
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

/// The `.seg` filters: the questions people have of the list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    /// No handler matched: the default or the fallback browser took it.
    /// The list of links to add a rule for.
    Unrouted,
    Failed,
    Rewritten,
    /// Kept at host only or nothing: counts, and the host-only rows.
    Private,
}

impl Filter {
    pub const ALL: [(Self, &'static str); 5] = [
        (Self::All, "All"),
        (Self::Unrouted, "Unrouted"),
        (Self::Failed, "Failed"),
        (Self::Rewritten, "Rewritten"),
        (Self::Private, "Private"),
    ];
}

/// No rule routed it: `defaultBrowser` applied, nothing was in charge and
/// the fallback browser took it, or routing was paused and the pause target
/// did.
fn is_unrouted(record: &DispatchRecord) -> bool {
    record.steps.iter().any(|step| {
        matches!(
            step,
            Step::Matched {
                by: Matched::Default
            } | Step::Fallback { .. }
                | Step::Paused { .. }
        )
    })
}

fn is_rewritten(record: &DispatchRecord) -> bool {
    record
        .steps
        .iter()
        .any(|step| matches!(step, Step::Rewrite { .. }))
}

impl ActivityRow {
    /// One record as the screen draws it.
    pub fn from_record(record: &DispatchRecord) -> Self {
        Self::build(record, false)
    }

    /// One journaled record, as [`from_record`](Self::from_record) draws it.
    pub fn from_journal(record: &DispatchRecord) -> Self {
        Self::build(record, true)
    }

    fn build(record: &DispatchRecord, journaled: bool) -> Self {
        let received = received_url(record);
        let from = record
            .link
            .opener
            .as_ref()
            .map(|opener| opener.name.as_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(NO_APP);
        Self {
            key: RowKey {
                id: record.id,
                journaled,
            },
            time: clock(record.received_at).into(),
            url: received.into(),
            from: from.into(),
            rule: rule_label(&record.steps).into(),
            opened: opened(record),
            took: took_label(record.timings.total_us).into(),
            rewritten: is_rewritten(record),
            default: is_unrouted(record),
            dry_run: record
                .steps
                .iter()
                .any(|step| matches!(step, Step::Launched { dry_run: true })),
            steps: trace_lines(record),
            matched_url: matched_url(record).into(),
            final_url: record.final_url.as_deref().unwrap_or(received).into(),
            privacy: record.privacy,
            journaled,
        }
    }

    pub fn failed(&self) -> bool {
        matches!(self.opened, Opened::Failed(_))
    }

    fn passes(&self, filter: Filter, search: &str) -> bool {
        let kind = match filter {
            Filter::All => true,
            Filter::Unrouted => self.default,
            Filter::Rewritten => self.rewritten,
            Filter::Failed => self.failed(),
            Filter::Private => self.privacy != Privacy::Full,
        };
        if !kind {
            return false;
        }
        if search.is_empty() {
            return true;
        }
        let needle = search.to_lowercase();
        let opened = match &self.opened {
            Opened::Browser {
                name,
                profile,
                private,
                ..
            } => format!(
                "{name} {}{}",
                profile.as_deref().unwrap_or_default(),
                if *private { " private window" } else { "" }
            ),
            Opened::Nowhere => NOT_OPENED.to_owned(),
            Opened::Fallback {
                why, name, profile, ..
            } => format!("{why} {name} {}", profile.as_deref().unwrap_or_default()),
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

/// The URL the rules saw: where a short link landed when the router
/// expanded it, else the URL as received. Whatever re-resolves a record
/// (Re-test, Save as test or example, Add a rule) starts from this one,
/// since Test a link and the saved tests do not expand short links.
pub(crate) fn matched_url(record: &DispatchRecord) -> &str {
    record
        .steps
        .iter()
        .find_map(|step| match step {
            Step::Expanded { url } => Some(url.as_str()),
            _ => None,
        })
        .unwrap_or_else(|| received_url(record))
}

/// The Rule column: "rw 1, 2 · h 3", "handler 2", "default", "fallback",
/// and while paused "paused · skipped h 3" (what would have routed it).
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
    let paused = steps.iter().any(|step| matches!(step, Step::Paused { .. }));
    if paused {
        parts.push("paused".into());
    }
    for step in steps {
        match step {
            Step::Matched {
                by: Matched::Handler { index },
            } => parts.push(if paused {
                format!("skipped h {}", index + 1)
            } else if parts.is_empty() {
                format!("handler {}", index + 1)
            } else {
                format!("h {}", index + 1)
            }),
            // A rules.json handler: the router does not number those.
            Step::Matched {
                by: Matched::Unreported,
            } => parts.push(if paused { "skipped h" } else { "handler" }.into()),
            // Paused, the pause target took what these would have sent to
            // the default or the fallback: "paused" says it all.
            Step::Matched {
                by: Matched::Default,
            } if !paused => parts.push("default".into()),
            Step::Fallback { .. } | Step::RefusedLoop { .. } if !paused => {
                parts.push("fallback".into());
            }
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
    // The first launch failure, and the router's reason for falling back
    // after it. A launch that follows them is the fallback browser's.
    let mut launch_failed: Option<&str> = None;
    let mut why: Option<&str> = None;
    for step in &record.steps {
        match step {
            Step::Rejected { reason } => return Opened::Failed(reason.clone().into()),
            Step::LaunchFailed { message } => {
                launch_failed.get_or_insert(message);
            }
            Step::Fallback { reason } if launch_failed.is_some() => why = Some(reason),
            Step::Launched { .. } if launch_failed.is_some() => {
                return match &record.browser {
                    Some(browser) => {
                        // First run saves a bundle id: show the app's name.
                        let name = browser_name(&browser.name);
                        Opened::Fallback {
                            why: why.or(launch_failed).unwrap_or_default().to_owned().into(),
                            badge: badge_for(&name),
                            name: name.into(),
                            profile: (!browser.profile.is_empty())
                                .then(|| browser.profile.clone().into()),
                        }
                    }
                    None => Opened::Failed("not routed".into()),
                };
            }
            Step::NotOpened => return Opened::Nowhere,
            _ => {}
        }
    }
    if let Some(message) = launch_failed {
        return Opened::Failed(message.to_owned().into());
    }
    match &record.browser {
        Some(browser) => Opened::Browser {
            badge: badge_for(&browser.name),
            name: browser.name.clone().into(),
            profile: (!browser.profile.is_empty()).then(|| browser.profile.clone().into()),
            private: browser.private,
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
            Step::Expanded { url: after } => {
                let line = format!(
                    "short link \u{00b7} expanded \u{00b7} {}",
                    describe_rewrite(url, after)
                );
                url = after;
                (StepMark::Hit(number), line)
            }
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
            Step::Paused { skipped } => (
                StepMark::Miss(number),
                match skipped {
                    Some(index) => format!("paused \u{00b7} skipped handler {}", index + 1),
                    None => "paused \u{00b7} every handler skipped".to_owned(),
                },
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

/// A rule that keeps less than full, and how many of today's links it
/// took: one `.cnt-card` under the Private filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivateCount {
    /// "handler 8", or "kept nothing" for the records that say nothing.
    pub rule: SharedString,
    pub count: u64,
    /// "Safari, private window. Keeps host only."
    pub detail: SharedString,
}

/// The screen's state.
pub struct Activity {
    /// The in-memory records, newest first, each kept at `Full` or
    /// `Host`.
    pub rows: Vec<ActivityRow>,
    /// Journaled records not among `rows`, newest first, while the journal
    /// is on.
    pub journal_rows: Vec<ActivityRow>,
    pub expanded: Option<RowKey>,
    pub filter: Filter,
    pub search: TextField,
    pub today: Counts,
    /// The filters' counts, over the records listed (and the ones kept at
    /// nothing, which are counted here and listed nowhere).
    pub counts: Counts,
    /// Records kept at nothing: a count, never a row.
    pub kept_nothing: u64,
    /// Where the router keeps dispatches, in its words.
    pub retention: RetentionInfo,
    _feed: Option<Subscription>,
}

impl Activity {
    /// Follows the feed when one is installed; empty otherwise.
    pub fn new(cx: &mut Context<'_, PwrRoute>) -> Self {
        let mut activity = Self {
            rows: Vec::new(),
            journal_rows: Vec::new(),
            expanded: None,
            filter: Filter::All,
            search: TextField::new("", cx),
            today: Counts::default(),
            counts: Counts::default(),
            kept_nothing: 0,
            retention: Feed::new(None).retention(),
            _feed: None,
        };
        if let Some(feed) = feed(cx) {
            activity.sync(feed.read(cx));
            activity._feed = Some(cx.observe(&feed, |this, feed, cx| {
                this.activity.sync(feed.read(cx));
                // A dispatch is a moment the file may have changed: an
                // agent wrote a rule the router just used.
                this.sync_with_disk(cx);
                cx.notify();
            }));
        }
        activity
    }

    /// The rows and counts as the feed has them now. The expanded row stays
    /// expanded while it is still listed.
    pub fn sync(&mut self, feed: &Feed) {
        let records = feed.records();
        let mut counts = Counts::default();
        for record in records {
            counts.count(record);
        }
        self.counts = counts;
        self.kept_nothing = records
            .iter()
            .filter(|record| record.privacy == Privacy::None)
            .count() as u64;
        self.rows = records
            .iter()
            .filter(|record| record.privacy != Privacy::None)
            .map(ActivityRow::from_record)
            .collect();
        self.retention = feed.retention();
        // The journal holds the same dispatches as memory, and more: only
        // what memory no longer has is listed again, below. A journal line
        // keeps the time to the millisecond, so that is what is compared.
        let stamp = |record: &DispatchRecord| {
            (
                record.id,
                record
                    .received_at
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0),
            )
        };
        let in_memory: HashSet<(DispatchId, u128)> = records.iter().map(stamp).collect();
        self.journal_rows = feed
            .journal(JOURNAL_ROWS)
            .iter()
            .filter(|record| record.privacy != Privacy::None && !in_memory.contains(&stamp(record)))
            .map(ActivityRow::from_journal)
            .collect();
        self.today = feed.today(SystemTime::now());
        if self.expanded.is_some_and(|key| {
            !self
                .rows
                .iter()
                .chain(&self.journal_rows)
                .any(|row| row.key == key)
        }) {
            self.expanded = None;
        }
    }

    /// The count a filter shows: the records it would list, plus, for
    /// Private, the ones kept at nothing.
    pub fn count(&self, filter: Filter) -> u64 {
        self.counts.of(filter)
    }

    /// Under Private: each rule that keeps less than full, with how many
    /// of the listed links it took, then the records that kept nothing.
    pub fn private_counts(&self) -> Vec<PrivateCount> {
        let mut cards: Vec<PrivateCount> = Vec::new();
        for row in self.rows.iter().filter(|row| row.privacy != Privacy::Full) {
            match cards.iter_mut().find(|card| card.rule == row.rule) {
                Some(card) => card.count += 1,
                None => cards.push(PrivateCount {
                    rule: row.rule.clone(),
                    count: 1,
                    detail: private_detail(row),
                }),
            }
        }
        if self.kept_nothing > 0 {
            cards.push(PrivateCount {
                rule: "kept nothing".into(),
                count: self.kept_nothing,
                detail: "No URL, no host, no sending app: counted, not listed.".into(),
            });
        }
        cards
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

    /// The journaled rows the filter and search let through, indexed
    /// after the in-memory rows.
    pub fn visible_journal(&self) -> Vec<(usize, &ActivityRow)> {
        let offset = self.rows.len();
        self.journal_rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.passes(self.filter, &self.search.text))
            .map(|(index, row)| (offset + index, row))
            .collect()
    }

    pub fn toggle(&mut self, key: RowKey) {
        self.expanded = if self.expanded == Some(key) {
            None
        } else {
            Some(key)
        };
    }

    /// Clear: here, in the feed and in the router. The journal is not
    /// touched: that is Settings › Privacy's Clear now.
    pub fn clear(&mut self, cx: &mut App) {
        self.rows.clear();
        self.kept_nothing = 0;
        self.counts = Counts::default();
        self.expanded = None;
        if let Some(feed) = feed(cx) {
            feed.update(cx, |feed, cx| {
                feed.clear();
                cx.notify();
            });
        }
    }
}

/// "Safari, private window. Keeps host only.": a `.cnt-card`'s line.
fn private_detail(row: &ActivityRow) -> SharedString {
    let went = match &row.opened {
        Opened::Browser {
            name,
            profile,
            private,
            ..
        } => {
            let mut went = name.to_string();
            if let Some(profile) = profile {
                went.push_str(&format!(" \u{00b7} {profile}"));
            }
            if *private {
                went.push_str(", private window");
            }
            went
        }
        Opened::Nowhere => NOT_OPENED.to_owned(),
        Opened::Fallback { name, profile, .. } => match profile {
            Some(profile) => format!("{name} \u{00b7} {profile}"),
            None => name.to_string(),
        },
        Opened::Failed(_) => "Failed".to_owned(),
    };
    let keeps = match row.privacy {
        Privacy::Full => "the full URL",
        Privacy::Host => "host only",
        Privacy::None => "nothing",
    };
    format!("{went}. Keeps {keeps}.").into()
}

// The row's "⋯" menu (design 2b) and the expanded row's "Add a rule…"
// (design 2c) live in `activity_actions.rs`: `row_menu` draws the menu
// `PwrRoute::actions.menu` holds open, `PwrRoute::add_rule` opens the sheet.
pub use crate::activity_actions::row_menu;

/// Column widths: the design's grid `54px minmax(0,1fr) 76px 110px 176px
/// 48px 18px` with a 12px gap.
mod column {
    pub const TIME: f32 = 54.0;
    pub const FROM: f32 = 76.0;
    pub const RULE: f32 = 110.0;
    pub const OPENED: f32 = 176.0;
    pub const TOOK: f32 = 48.0;
    pub const MORE: f32 = 18.0;
    pub const GAP: f32 = 12.0;
    /// Where the expanded trace starts: the link column.
    pub const TRACE_INDENT: f32 = 14.0 + TIME + GAP;
}

/// `.ar`'s cells around their contents.
fn grid_row(cells: [Div; 7]) -> Div {
    let [time, link, from, rule, opened, took, more] = cells;
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
        .child(more.w(px(column::MORE)).flex_none().flex().justify_center())
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
        div(),
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
            private,
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
            .when(*private, |cell| {
                cell.child(mono("private window", 10.5, 500.0, theme::text_muted()))
            })
            .when(dry_run, |cell| cell.child(tag("dry run"))),
        Opened::Nowhere => div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(badge(NOT_OPENED_BADGE))
            .child(text(NOT_OPENED, 11.5, 400.0, theme::text_muted())),
        // `.ar .d`: the failure, then `.sub` with where the link went.
        Opened::Fallback {
            why,
            badge: badge_label,
            name,
            profile,
        } => div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .min_w_0()
            .child(status(why.clone(), theme::status_error()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .min_w_0()
                    .child(text("\u{21aa}", 10.5, 500.0, theme::text_muted()))
                    .child(badge(badge_label.clone()))
                    .child(
                        one_line(
                            match profile {
                                Some(profile) => format!("opened in {name} \u{00b7} {profile}"),
                                None => format!("opened in {name}"),
                            },
                            theme::SANS,
                            500.0,
                            10.5,
                            10.5,
                            theme::text_muted(),
                        )
                        .min_w_0(),
                    )
                    .when(dry_run, |sub| sub.child(tag("dry run"))),
            ),
        Opened::Failed(why) => div().child(status(why.clone(), theme::status_error())),
    }
}

/// `.ar`
fn row(
    index: usize,
    row: &ActivityRow,
    expanded: bool,
    menu: Option<AnyElement>,
    last: bool,
    cx: &mut Context<'_, PwrRoute>,
) -> impl IntoElement + use<> {
    let link_color = if row.failed() {
        theme::danger_text()
    } else {
        theme::text_primary()
    };
    let key = row.key;
    // `.u`: the link, with the "host only" chip for a host-only record.
    let link = div()
        .flex()
        .items_center()
        .gap(px(7.0))
        .min_w_0()
        .child(clipped(row.url.clone(), true, 11.5, link_color).min_w_0())
        .when(row.privacy == Privacy::Host, |cell| {
            cell.child(chip("host only").flex_none())
        });
    // `.ar.unr .tag`: an unrouted row's rule chip is amber.
    let rule = if row.default {
        toned_chip(row.rule.clone(), Tone::Warn)
    } else {
        tag(row.rule.clone())
    };
    // "⋯" opens the row's menu, drawn under the button, and closes it
    // again: the menu closes itself on a mouse down outside it, which this
    // button is, so the click looks at what was open before that.
    let more = button("\u{22ef}", ButtonKind::Quiet, true)
        .id(("activity-more", index))
        .debug_selector(move || format!("activity-more-{index}"))
        .capture_any_mouse_down(cx.listener(|this, _, _, _| {
            this.actions.menu_was = this.actions.menu.as_ref().map(|menu| menu.key);
        }))
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.toggle_row_menu(key, cx);
        }));
    grid_row([
        div().child(mono(row.time.clone(), 10.5, 500.0, theme::text_subtle())),
        link,
        clipped(row.from.clone(), false, 11.5, theme::text_secondary()),
        div().flex().child(rule),
        opened_cell(&row.opened, row.dry_run),
        div().child(mono(row.took.clone(), 10.5, 500.0, theme::text_subtle())),
        div().relative().child(more).children(menu),
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
        this.activity.toggle(key);
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
    let url = row.matched_url.to_string();
    let from = row.from.to_string();
    let key = row.key;
    // A host-only record has no full URL to re-test or add a rule for.
    let whole = row.privacy == Privacy::Full;
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
                .when(row.journaled, |actions| actions.child(tag("from the journal")))
                .when(whole, |actions| {
                    actions.child(
                        button("Add a rule\u{2026}", ButtonKind::Quiet, true)
                            .id(("activity-add-rule", index))
                            .debug_selector(move || format!("activity-add-rule-{index}"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.add_rule(key, window, cx);
                            })),
                    )
                })
                // Opening again waits for the browser launcher.
                .child(button("Open again", ButtonKind::Quiet, true))
                .when(whole, |actions| {
                    actions.child(
                        button("Re-test", ButtonKind::Quiet, true)
                            .id(("activity-retest", index))
                            .debug_selector(move || format!("activity-retest-{index}"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.retest(&url, &from, window, cx);
                            })),
                    )
                }),
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
            let selected = app.activity.filter == filter;
            let count_color = if selected {
                theme::accent_bright()
            } else {
                theme::text_subtle()
            };
            seg.child(
                segment(label, selected)
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    // `.seg span i`: the count beside the label.
                    .child(mono(
                        thousands(app.activity.count(filter)),
                        10.0,
                        500.0,
                        count_color,
                    ))
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
        .child(
            div()
                .ml_auto()
                .child(note(memory_note(&app.activity.retention))),
        )
}

/// "last 200 · 24 h · in memory": the toolbar's note, from the router's
/// retention.
pub fn memory_note(retention: &RetentionInfo) -> String {
    let memory = retention.memory;
    let mut note = format!("last {}", thousands(memory.count as u64));
    if memory.hours > 0 {
        note.push_str(&format!(" \u{00b7} {} h", memory.hours));
    }
    note.push_str(" \u{00b7} in memory");
    if retention.journal.is_some() {
        note.push_str(" \u{00b7} journaled");
    }
    note
}

/// `.cnts`: under Private, one `.cnt-card` per rule that keeps less than
/// full, over the host-only rows.
fn private_cards(app: &PwrRoute) -> Option<Div> {
    let cards = app.activity.private_counts();
    if cards.is_empty() {
        return None;
    }
    let total = app.activity.count(Filter::Private);
    let heading = div()
        .flex()
        .flex_col()
        .pt(px(13.0))
        .px(px(16.0))
        .child(tracked(
            "Counts only",
            9.5,
            600.0,
            0.14,
            theme::text_muted(),
        ))
        .child(
            text(
                format!(
                    "{} private {}",
                    thousands(total),
                    if total == 1 { "link" } else { "links" }
                ),
                13.5,
                600.0,
                theme::text_primary(),
            )
            .mt(px(6.0)),
        )
        .child(
            paragraph(
                "Rules that keep host only or nothing. What they dropped was never recorded, so \
                 there is nothing more to show.",
                11.5,
                18.4,
                theme::text_muted(),
            )
            .mt(px(6.0))
            .max_w(px(520.0)),
        );
    let grid = cards.into_iter().fold(
        div()
            .flex()
            .flex_wrap()
            .gap(px(10.0))
            .py(px(14.0))
            .px(px(16.0)),
        |grid, card| {
            grid.child(
                div()
                    .w(px(220.0))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .py(px(12.0))
                    .px(px(14.0))
                    .border_1()
                    .border_color(theme::border_subtle())
                    .rounded(px(8.0))
                    .bg(theme::bg_app())
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(6.0))
                            .child(mono(
                                thousands(card.count),
                                22.0,
                                700.0,
                                theme::text_primary(),
                            ))
                            .child(text("listed", 11.0, 500.0, theme::text_muted())),
                    )
                    .child(div().flex().child(tag(card.rule)))
                    .child(paragraph(card.detail, 10.5, 15.75, theme::text_subtle())),
            )
        },
    );
    Some(
        div()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(theme::border_faint())
            .child(heading)
            .child(grid),
    )
}

/// `.ar.sum`: the records kept at nothing, as one line at the end.
fn kept_nothing_row(count: u64) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(column::GAP))
        .px(px(14.0))
        .py(px(8.0))
        .bg(theme::bg_input())
        .child(div().w(px(column::TIME)).flex_none())
        .child(text(
            format!(
                "{} {} kept nothing. Counted, not listed.",
                thousands(count),
                if count == 1 { "link" } else { "links" }
            ),
            11.0,
            400.0,
            theme::text_subtle(),
        ))
}

/// The divider over the journaled rows.
fn journal_divider() -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .px(px(14.0))
        .py(px(7.0))
        .bg(theme::bg_elevated())
        .border_b_1()
        .border_color(theme::border_faint())
        .child(tracked(
            "From the journal",
            9.5,
            600.0,
            0.14,
            theme::text_muted(),
        ))
        .child(note(
            "no longer in memory \u{00b7} read from disk, at each record\u{2019}s level",
        ))
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
    let journaled = app.activity.visible_journal();
    let private = app.activity.filter == Filter::Private;
    let kept_nothing = app.activity.kept_nothing;
    let nothing_to_list =
        app.activity.rows.is_empty() && app.activity.journal_rows.is_empty() && kept_nothing == 0;
    let list = if nothing_to_list {
        card().child(empty_state(cx))
    } else {
        let count = visible.len() + journaled.len();
        let mut list = card();
        if private && let Some(cards) = private_cards(app) {
            list = list.child(cards);
        }
        let rows = |list: Div,
                    rows: Vec<(usize, &ActivityRow)>,
                    offset: usize,
                    cx: &mut Context<'_, PwrRoute>| {
            rows.into_iter()
                .enumerate()
                .fold(list, |list, (position, (index, item))| {
                    let last = offset + position + 1 == count && kept_nothing == 0;
                    let expanded = app.activity.expanded == Some(item.key);
                    let menu = row_menu(app, index, item.key, cx);
                    list.child(row(index, item, expanded, menu, last, cx))
                        .when(expanded, |list| {
                            list.child(expansion(index, item, last, cx))
                        })
                })
        };
        let memory_rows = visible.len();
        let mut list = rows(list.child(head_row()), visible, 0, cx);
        if !journaled.is_empty() {
            list = rows(list.child(journal_divider()), journaled, memory_rows, cx);
        }
        if count == 0
            && (kept_nothing == 0 || !matches!(app.activity.filter, Filter::All | Filter::Private))
        {
            list = list.child(div().py(px(14.0)).px(px(14.0)).child(text(
                "No dispatch matches.",
                11.5,
                400.0,
                theme::text_muted(),
            )));
        }
        if kept_nothing > 0 && matches!(app.activity.filter, Filter::All | Filter::Private) {
            list = list.child(kept_nothing_row(kept_nothing));
        }
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
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            button("Export\u{2026}", ButtonKind::Quiet, true)
                                .id("activity-export")
                                .debug_selector(|| "activity-export".into())
                                .on_click(cx.listener(|_, _, _, cx| {
                                    // Writes the links in memory, each at
                                    // its level, to a file the reader picks
                                    // in the OS's dialog: the one way out of
                                    // memory from this screen.
                                    crate::settings::privacy_host::export(cx);
                                })),
                        )
                        .child(
                            button("Clear", ButtonKind::Quiet, true)
                                .id("activity-clear")
                                .debug_selector(|| "activity-clear".into())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.activity.clear(cx);
                                    cx.notify();
                                })),
                        ),
                ),
        )
        .child(toolbar(app, window, cx))
        .child(list)
        // The router's sentence, verbatim: the same one Settings › Privacy
        // shows under the journal switch.
        .child(
            note(app.activity.retention.description.clone())
                .id("activity-retention")
                .debug_selector(|| "activity-retention".into()),
        )
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
        AppType, Browser, DispatchId, DispatchRecord, DryRunLauncher, JournalConfig, JournalMode,
        Link, Matched, OpenerInfo, Outcome, Privacy, Retention, Router, Source, Step, Timings,
    };

    use super::*;

    #[test]
    fn the_journal_is_read_again_only_when_its_files_change() {
        let dir = tempfile::tempdir().unwrap();
        let config = JournalConfig::new(dir.path().join("journal"), JournalMode::Hosts);
        let router = Router::spawn(
            Options {
                journal: Some(config.clone()),
                ..Options::default()
            },
            None,
            DryRunLauncher::new(),
        )
        .unwrap();
        let handle = router.handle();
        let dispatch = |path: &str| {
            handle
                .dispatch_and_wait(Link::new(
                    format!("https://a.example/{path}?q=1"),
                    Source::CommandLine,
                ))
                .unwrap();
        };

        // Nothing journaled yet: one read of an empty journal, then none.
        let mut cache = JournalCache::default();
        assert!(cache.refresh(&config, 10).unwrap());
        assert!(cache.records().is_empty());
        assert!(!cache.refresh(&config, 10).unwrap());

        // A dispatch appends a line: read once, at the journal's level.
        dispatch("one");
        assert!(cache.refresh(&config, 10).unwrap());
        assert_eq!(cache.records().len(), 1);
        assert_eq!(cache.records()[0].link.url, "https://a.example");
        assert!(!cache.refresh(&config, 10).unwrap());
        assert!(!cache.refresh(&config, 10).unwrap());
        assert_eq!(cache.records().len(), 1);

        // Another limit, or another journal, is another read.
        assert!(cache.refresh(&config, 5).unwrap());
        assert!(!cache.refresh(&config, 5).unwrap());
        let elsewhere = JournalConfig::new(dir.path().join("elsewhere"), JournalMode::Hosts);
        assert!(cache.refresh(&elsewhere, 5).unwrap());
        assert!(cache.records().is_empty());
        assert!(cache.refresh(&config, 5).unwrap());
        assert_eq!(cache.records().len(), 1);

        dispatch("two");
        assert!(cache.refresh(&config, 5).unwrap());
        assert_eq!(cache.records().len(), 2);
        assert_eq!(cache.records()[0].id, handle.recent()[0].id);
        cache.forget();
        assert!(cache.refresh(&config, 5).unwrap());

        // The feed answers from the same cache, and a journaled row's
        // record comes back from it.
        let feed = Feed::new(Some(handle.clone()));
        let records = feed.journal(10);
        assert_eq!(records.len(), 2);
        assert_eq!(feed.journal(10).len(), 2);
        assert!(
            !feed
                .journal_cache
                .borrow_mut()
                .refresh(&config, 10)
                .unwrap()
        );
        let key = RowKey {
            id: records[1].id,
            journaled: true,
        };
        assert_eq!(feed.record(key).unwrap().id, records[1].id);
        assert_eq!(feed.journal(10).len(), 2);
        assert!(
            !feed
                .journal_cache
                .borrow_mut()
                .refresh(&config, 10)
                .unwrap()
        );
        // The journal turned off: nothing is listed, and nothing is kept.
        handle.set_journal(None).unwrap();
        assert!(feed.journal(10).is_empty());
        assert!(feed.journal_cache.borrow().read_as.is_none());
    }

    fn browser(name: &str, profile: &str, url: &str) -> Browser {
        Browser {
            name: name.into(),
            app_type: AppType::AppName,
            url: url.into(),
            profile: profile.into(),
            args: Vec::new(),
            open_in_background: None,
            private: false,
            privacy: pwrroute_engine::Privacy::Full,
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
            privacy: Privacy::Full,
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
    fn a_short_link_row_shows_the_link_and_where_it_landed() {
        let received = "https://bit.ly/gh";
        let landed = "https://github.com/pwrdrvr/x";
        let record = record(
            9,
            received,
            Some("Mail"),
            vec![
                Step::Received {
                    url: received.into(),
                },
                Step::Expanded { url: landed.into() },
                Step::Matched {
                    by: Matched::Unreported,
                },
                Step::Browser {
                    browser: browser("Firefox", "", landed),
                },
                Step::Launched { dry_run: false },
            ],
        );
        let row = ActivityRow::from_record(&record);
        assert_eq!(row.url, received, "the row is the link as clicked");
        assert_eq!(row.matched_url, landed, "Re-test tries what the rules saw");
        assert_eq!(row.final_url, landed);
        assert_eq!(row.rule, "handler");
        assert!(!row.rewritten, "an expansion is not a rewrite");
        assert_eq!(
            texts(&row)[..2],
            [
                "short link \u{00b7} expanded \u{00b7} host \u{2192} github.com \u{00b7} path \u{2192} /pwrdrvr/x",
                "a handler matched",
            ]
        );
        assert_eq!(row.steps[0].mark, StepMark::Hit(1));
        assert!(texts(&row)[2].starts_with("Firefox \u{00b7} resolved"));
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
        assert_eq!(row.key.id, DispatchId(7));
        assert!(!row.key.journaled && !row.journaled);
        assert_eq!(row.privacy, Privacy::Full);
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
            private,
        } = &row.opened
        else {
            panic!("opened in a browser");
        };
        assert_eq!((badge.as_ref(), name.as_ref()), ("Ff", "Firefox"));
        assert_eq!(profile.as_deref(), Some("default-release"));
        assert!(!private);
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
        assert!(row.passes(Filter::Unrouted, "chrome"));
        assert!(!row.passes(Filter::Private, ""));
        assert!(!row.passes(Filter::Rewritten, ""));
    }

    #[test]
    fn a_paused_dispatch_names_the_handler_it_skipped() {
        let url = "https://bsky.app/profile/example";
        let paused = record(
            4,
            url,
            None,
            vec![
                Step::Received { url: url.into() },
                Step::Matched {
                    by: Matched::Handler { index: 3 },
                },
                Step::Paused { skipped: Some(3) },
                Step::Browser {
                    browser: browser("Google Chrome", "Personal", url),
                },
                Step::Launched { dry_run: false },
            ],
        );
        let row = ActivityRow::from_record(&paused);
        assert_eq!(row.rule, "paused \u{00b7} skipped h 4");
        assert!(row.default, "a paused link is unrouted");
        assert!(row.passes(Filter::Unrouted, ""));
        assert_eq!(
            texts(&row),
            [
                "handler 4",
                "paused \u{00b7} skipped handler 4",
                "Google Chrome \u{00b7} Personal \u{00b7} resolved 0.6 ms \u{00b7} launched 5.1 ms",
            ]
        );
        assert_eq!(row.steps[1].mark, StepMark::Miss(2));

        // Rewritten first, then nothing to skip: the default would have had
        // it, and the pause target did.
        let rewritten = "https://xcancel.com/example/status/1";
        let unskipped = record(
            5,
            "https://x.com/example/status/1",
            None,
            vec![
                Step::Received {
                    url: "https://x.com/example/status/1".into(),
                },
                Step::Rewrite {
                    rule: Some(0),
                    url: rewritten.into(),
                },
                Step::Matched {
                    by: Matched::Default,
                },
                Step::Paused { skipped: None },
                Step::Browser {
                    browser: browser("Google Chrome", "Personal", rewritten),
                },
                Step::Launched { dry_run: false },
            ],
        );
        let row = ActivityRow::from_record(&unskipped);
        assert_eq!(row.rule, "rw 1 \u{00b7} paused");
        assert!(row.rewritten && row.default);
        assert_eq!(texts(&row)[2], "paused \u{00b7} every handler skipped");
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
    fn a_browser_that_could_not_open_shows_the_fallback_that_did() {
        let url = "https://zoom.us/j/8142201137";
        let mut record = record(
            6,
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
                    message: "browser \"zoom.us\" is not installed".into(),
                },
                Step::Fallback {
                    reason: "zoom.us could not be opened".into(),
                },
                Step::Launched { dry_run: false },
            ],
        );
        // The router names the browser that opened the link.
        record.browser = Some(browser("Google Chrome", "Personal", url));
        assert_eq!(record.outcome, Outcome::Opened);
        let row = ActivityRow::from_record(&record);
        assert!(!row.failed(), "{:?}", row.opened);
        let Opened::Fallback {
            why,
            badge,
            name,
            profile,
        } = &row.opened
        else {
            panic!("opened in the fallback: {:?}", row.opened);
        };
        assert_eq!(why.as_ref(), "zoom.us could not be opened");
        assert_eq!((badge.as_ref(), name.as_ref()), ("Ch", "Google Chrome"));
        assert_eq!(profile.as_deref(), Some("Personal"));
        assert_eq!(row.rule, "handler 6 \u{00b7} fallback");
        assert!(
            row.default,
            "the fallback took it: a link to add a rule for"
        );
        assert_eq!(
            texts(&row),
            [
                "handler 6",
                "launch failed \u{00b7} browser \"zoom.us\" is not installed",
                "fallback \u{00b7} zoom.us could not be opened",
                "Google Chrome \u{00b7} Personal \u{00b7} resolved 0.6 ms \u{00b7} launched 5.1 ms",
            ]
        );
        assert_eq!(row.steps[1].mark, StepMark::Fail);
        assert_eq!(row.steps[2].mark, StepMark::Miss(3));
        assert_eq!(row.steps[3].mark, StepMark::Done);
        assert!(row.passes(Filter::Unrouted, "chrome"));
        assert!(row.passes(Filter::All, "could not be opened"));
        assert!(!row.passes(Filter::Failed, ""));
        assert_eq!(
            private_detail(&row).as_ref(),
            "Google Chrome \u{00b7} Personal. Keeps the full URL."
        );

        // First run saves a bundle id; the row names the app.
        record.browser = Some(browser("com.google.Chrome", "", url));
        let row = ActivityRow::from_record(&record);
        assert!(
            matches!(&row.opened, Opened::Fallback { badge, name, profile: None, .. }
                if badge == "Ch" && name == "Google Chrome"),
            "{:?}",
            row.opened
        );

        // Both failed: the row is a failure with the first reason.
        record.steps.pop();
        record.steps.push(Step::LaunchFailed {
            message: "browser \"Google Chrome\" is not installed".into(),
        });
        record.outcome = Outcome::Failed;
        let row = ActivityRow::from_record(&record);
        assert!(row.failed());
        assert!(
            matches!(&row.opened, Opened::Failed(why) if why == "browser \"zoom.us\" is not installed")
        );
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
                failed: 1,
                unrouted: 3,
                rewritten: 0,
                private: 0,
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

    /// Records at every privacy level, as the router reduces them before
    /// the feed sees them: a full unrouted one, a host-only private-window
    /// one, and one kept at nothing.
    fn leveled_records() -> Vec<DispatchRecord> {
        let unrouted = "https://app.metrics.example/dashboard/abc?from_ts=1";
        let full = record(
            1,
            unrouted,
            Some("Slack"),
            vec![
                Step::Received {
                    url: unrouted.into(),
                },
                Step::Matched {
                    by: Matched::Default,
                },
                Step::Browser {
                    browser: browser("Google Chrome", "Personal", unrouted),
                },
                Step::Launched { dry_run: false },
            ],
        );
        let bank = "https://mybank.example/accounts/42?session=abc";
        let mut private = browser("Safari", "", bank);
        private.private = true;
        let mut host = record(
            2,
            bank,
            Some("Mail"),
            vec![
                Step::Received { url: bank.into() },
                Step::Matched {
                    by: Matched::Handler { index: 7 },
                },
                Step::Browser { browser: private },
                Step::Launched { dry_run: false },
            ],
        );
        host.privacy = Privacy::Host;
        host.reduce(Privacy::Host);
        let mut none = record(
            3,
            "https://clinic.example/visit/9",
            Some("Mail"),
            vec![
                Step::Received {
                    url: "https://clinic.example/visit/9".into(),
                },
                Step::Matched {
                    by: Matched::Handler { index: 8 },
                },
                Step::Browser {
                    browser: browser("Firefox", "", "https://clinic.example/visit/9"),
                },
                Step::Launched { dry_run: false },
            ],
        );
        none.privacy = Privacy::None;
        none.reduce(Privacy::None);
        vec![none, host, full]
    }

    #[test]
    fn a_host_only_record_keeps_the_host_and_one_kept_at_nothing_is_a_count() {
        let records = leveled_records();
        let feed = Feed::from_records(records.clone());
        let today = feed.today(SystemTime::now());
        assert_eq!((today.routed, today.unrouted, today.private), (3, 1, 2));
        assert_eq!(today.label(), "3 today \u{00b7} 2 private");

        let host = ActivityRow::from_record(&records[1]);
        assert_eq!(host.privacy, Privacy::Host);
        assert_eq!(host.url, "https://mybank.example");
        assert!(!host.url.contains("session"));
        assert_eq!(host.final_url, "https://mybank.example");
        assert_eq!(host.rule, "handler 8");
        assert!(matches!(
            &host.opened,
            Opened::Browser {
                private: true,
                profile: None,
                ..
            }
        ));
        assert!(host.passes(Filter::Private, "private window"));
        assert!(!host.passes(Filter::Unrouted, ""));
        assert_eq!(
            private_detail(&host),
            "Safari, private window. Keeps host only."
        );

        let full = ActivityRow::from_record(&records[2]);
        assert!(full.default && full.passes(Filter::Unrouted, ""));
        assert!(!full.passes(Filter::Private, ""));

        let journaled = ActivityRow::from_journal(&records[2]);
        assert!(journaled.journaled && journaled.key.journaled);
        assert_eq!(journaled.key.id, full.key.id);
        assert_ne!(journaled.key, full.key);
    }

    #[test]
    fn memory_notes_follow_the_retention() {
        let note = |count, hours, journal: Option<JournalMode>| {
            let options = Options {
                retention: Retention { count, hours },
                journal: journal.map(|mode| JournalConfig::new("/tmp/j", mode)),
                ..Options::default()
            };
            memory_note(&RetentionInfo {
                memory: options.retention,
                journal: options.journal.clone(),
                description: options.retention_description(),
            })
        };
        assert_eq!(
            note(200, 24, None),
            "last 200 \u{00b7} 24 h \u{00b7} in memory"
        );
        assert_eq!(note(2000, 0, None), "last 2,000 \u{00b7} in memory");
        assert_eq!(
            note(500, 72, Some(JournalMode::Hosts)),
            "last 500 \u{00b7} 72 h \u{00b7} in memory \u{00b7} journaled"
        );
    }

    #[test]
    fn counts_read_as_the_design_writes_them() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1412), "1,412");
        assert_eq!(thousands(1_234_567), "1,234,567");
        let counts = |routed| Counts {
            routed,
            ..Counts::default()
        };
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
                failed: 2,
                ..Counts::default()
            }
            .label(),
            "1,412 today \u{00b7} 2 failed"
        );
        assert_eq!(
            Counts {
                routed: 1412,
                failed: 2,
                private: 14,
                ..Counts::default()
            }
            .label(),
            "1,412 today \u{00b7} 2 failed \u{00b7} 14 private"
        );
    }
}
