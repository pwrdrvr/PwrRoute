//! What the menu bar dropdown and the tray menu show, as plain data.
//!
//! The recents, the held links and the count come from the router's
//! dispatch records ([`Recent::from_record`], [`HeldLink::from_record`],
//! [`TrayModel::sync`]), what incoming links do (rules, send, hold, pause)
//! from the router's override slot (`tray::sync_override`), and whether
//! PwrRoute is the default browser from the OS
//! (`tray::follow_default_browser`). [`TrayModel::starting`] is the model
//! before any of them has reported; [`TrayModel::fixture`] is the whole
//! design example, for screenshots and tests. Everything here is testable
//! without a window.
//!
//! Recent links keep a host, never a full URL: the dropdown is visible to
//! whoever is looking at the screen. A held link is the exception (design:
//! Incoming Links 2c): the user held it to see it, so a record kept in full
//! shows its path too.

use std::time::{Duration, SystemTime};

use pwrroute_router::{
    Destination, DispatchId, DispatchRecord, MAX_NEXT, Matched, NotOpenedReason, Outcome, Owner,
    Privacy, SendScope, Sent, Step,
};

use crate::activity::{Feed, thousands};
use crate::activity_actions::BrowserChoice;
use crate::destinations::{self, Choice, Lane};
use crate::overview_model::Installed;
use crate::widgets::{AppBadge, badge_for};

/// How many recent links the dropdown lists.
pub const RECENT_LIMIT: usize = 3;

/// How many held links the dropdown lists; the rest wait in Activity.
pub const HELD_LIMIT: usize = 4;

/// What incoming links do: the router's override slot, as the tray last
/// read it (`tray::sync_override`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Routing {
    #[default]
    Active,
    /// Every link goes to [`TrayModel::pause_target`]; rewrites still run.
    Paused,
    /// The user sent links to [`TrayModel::destination`]: the next few, or
    /// all of them for a while. The router counts.
    Sending(SendScope),
    /// Links are recorded and not opened, for the next few or for a while
    /// (design: Incoming Links). They wait in [`TrayModel::held`].
    Holding(SendScope),
}

/// The Incoming links switch (design: Incoming Links 1a): what the next
/// links do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Rules,
    Send,
    Hold,
    Pause,
}

impl Mode {
    pub const ALL: [Self; 4] = [Self::Rules, Self::Send, Self::Hold, Self::Pause];

    pub fn label(self) -> &'static str {
        match self {
            Self::Rules => "Rules",
            Self::Send => "Send",
            Self::Hold => "Hold",
            Self::Pause => "Pause",
        }
    }

    /// Send and Hold say for how long; Rules and Pause last until changed.
    pub fn has_scope(self) -> bool {
        matches!(self, Self::Send | Self::Hold)
    }
}

/// The For row: how many links, or how long, Send and Hold last.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ForKind {
    /// The next [`TrayModel::next_count`] links.
    #[default]
    Links,
    /// [`TIME_STEPS`]`[`[`TrayModel::time_step`]`]` from when it starts.
    Time,
    /// Until the user goes back to the rules.
    UntilOff,
}

impl ForKind {
    pub const ALL: [Self; 3] = [Self::Links, Self::Time, Self::UntilOff];

    pub fn label(self) -> &'static str {
        match self {
            Self::Links => "Links",
            Self::Time => "Time",
            Self::UntilOff => "Until off",
        }
    }

    /// Which choice a running scope is.
    pub fn of(scope: SendScope) -> Self {
        match scope {
            SendScope::Next { .. } => Self::Links,
            SendScope::All { until: Some(_) } => Self::Time,
            SendScope::All { until: None } => Self::UntilOff,
        }
    }
}

/// The times For › Time steps through (design: Incoming Links 5).
pub const TIME_STEPS: [Duration; 6] = [
    Duration::from_secs(15 * 60),
    Duration::from_secs(30 * 60),
    Duration::from_secs(60 * 60),
    Duration::from_secs(2 * 60 * 60),
    Duration::from_secs(4 * 60 * 60),
    Duration::from_secs(8 * 60 * 60),
];

/// One hour: where Time starts.
pub const DEFAULT_TIME_STEP: usize = 2;

/// "15m", "1h": the stepper's words for a time, short enough for its box.
pub fn time_step_label(step: usize) -> String {
    let minutes = TIME_STEPS[step.min(TIME_STEPS.len() - 1)].as_secs() / 60;
    if minutes < 60 {
        format!("{minutes}m")
    } else {
        format!("{}h", minutes / 60)
    }
}

/// The tone a mode is drawn in: Send the accent, Hold info blue, Pause the
/// warning (design: Incoming Links 1b, Tones).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Send,
    Hold,
    Pause,
}

/// A run of the live line or the hint; `strong` runs are emphasized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub strong: bool,
}

impl Run {
    fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            strong: false,
        }
    }

    fn strong(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            strong: true,
        }
    }
}

/// The text of a list of runs.
pub fn runs_text(runs: &[Run]) -> String {
    runs.iter().map(|run| run.text.as_str()).collect()
}

/// The line that says what is on, and its one-click way back (design:
/// Incoming Links 2b-2f).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Live {
    pub tone: Tone,
    pub runs: Vec<Run>,
    /// "Back to rules", or "Resume" while paused.
    pub button: &'static str,
}

/// Where Send links sends links: a browser, a profile or none (the
/// browser's own), or the browser's private window by its word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendDestination {
    pub browser: BrowserLabel,
    /// The browser's word for its private window ("InPrivate") when the
    /// destination is one; `browser.profile` is then `None`.
    pub private: Option<String>,
}

impl SendDestination {
    pub fn profile(browser: BrowserLabel) -> Self {
        Self {
            browser,
            private: None,
        }
    }

    /// "Microsoft Edge · Work", "Microsoft Edge · InPrivate", or the name.
    pub fn display(&self) -> String {
        match &self.private {
            Some(word) => format!("{} \u{00b7} {word}", self.browser.name),
            None => self.browser.display(),
        }
    }

    /// What follows the name: the profile or the private window's word.
    pub fn detail(&self) -> Option<&str> {
        self.private.as_deref().or(self.browser.profile.as_deref())
    }

    /// The destination as the lanes light it: the browser by the name the
    /// scan knows it by.
    pub fn choice(&self) -> Choice {
        Choice {
            browser: self.browser.name.clone(),
            profile: if self.private.is_some() {
                String::new()
            } else {
                self.browser.profile.clone().unwrap_or_default()
            },
            private: self.private.is_some(),
        }
    }

    /// The destination as the router is told it.
    pub fn target(&self) -> Destination {
        Destination {
            browser: self.browser.app().to_owned(),
            profile: if self.private.is_some() {
                String::new()
            } else {
                self.browser.profile.clone().unwrap_or_default()
            },
            private: self.private.is_some(),
        }
    }
}

/// What the destination lanes are drawn from (design: Destination and
/// App Pickers, as the rule editor draws them): the scan less the hidden
/// apps and PwrRoute, the whole scan, and whether it ran.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DestinationChoices {
    pub browsers: Vec<BrowserChoice>,
    pub installed: Vec<Installed>,
    pub scanned: bool,
}

/// A browser and profile, as the dropdown names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserLabel {
    /// The `.bi` badge: the app's icon, or two letters.
    pub badge: AppBadge,
    /// The name the reader knows ("Microsoft Edge"), never a bundle id when
    /// a name is known.
    pub name: String,
    pub profile: Option<String>,
    /// The app as `rules.json` or the router names it, when that is not
    /// `name` (a bundle id, a path). What a pause tells the router.
    pub app: Option<String>,
}

impl BrowserLabel {
    /// A badge of bare letters names the app by `name`, so it draws the
    /// app's icon too.
    pub fn new(badge: impl Into<AppBadge>, name: &str, profile: Option<&str>) -> Self {
        Self {
            badge: badge.into().or_app(name),
            name: name.to_owned(),
            profile: profile.map(str::to_owned),
            app: None,
        }
    }

    /// The label for `app`, shown as `name`.
    pub fn for_app(mut self, app: &str) -> Self {
        self.app = (app != self.name).then(|| app.to_owned());
        self
    }

    /// The app as the router is told it: `app`, else the name.
    pub fn app(&self) -> &str {
        self.app.as_deref().unwrap_or(&self.name)
    }

    /// "Google Chrome · Personal", or the name alone.
    pub fn display(&self) -> String {
        match &self.profile {
            Some(profile) => format!("{} \u{00b7} {profile}", self.name),
            None => self.name.clone(),
        }
    }
}

/// Which rule sent a recent link where it went, in Activity's words:
/// "rule 3" is `rules.json`'s, "config handler 2" the JS config's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecentRoute {
    /// The JS config's handler matched; its 1-based number in the
    /// config's `handlers`.
    Handler(usize),
    /// `rules.json`'s rule matched; its 1-based number in the file.
    Rule(usize),
    /// A rule matched that the record does not number (a router that did
    /// not say which).
    Unnumbered,
    /// Nothing matched: PwrRoute's default browser (`rules.json`'s), or a
    /// record that did not say whose default it was.
    Default,
    /// Nothing matched: the JS config's own `defaultBrowser`.
    ConfigDefault,
    /// No config, or the config pointed back at PwrRoute: the router's
    /// fallback browser.
    Fallback,
    /// Routing was paused. `skipped` is what matched and was passed over,
    /// when the record numbers it.
    Paused { skipped: Option<Skipped> },
    /// The user sent it ("next link", "all links"), passing over `skipped`.
    Sent {
        sent: Sent,
        skipped: Option<Skipped>,
    },
}

/// What a paused link would have gone to: a rule or handler, numbered
/// from 1 as [`RecentRoute`] numbers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skipped {
    Handler(usize),
    Rule(usize),
}

/// One recent link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recent {
    /// Host, with the port when it is not the scheme's default.
    pub host: String,
    /// The host before a rewrite changed it.
    pub rewritten_from: Option<String>,
    /// A rewrite ran, whether or not it changed the host.
    pub rewritten: bool,
    pub browser: BrowserLabel,
    pub route: RecentRoute,
    pub at: SystemTime,
}

impl Recent {
    /// A recent entry from the URL the browser opened and, when a rewrite
    /// ran, the URL as received. Only hosts are kept. `None` when the URL has
    /// no host (a `file:` or `mailto:` link), which the dropdown skips.
    pub fn from_urls(
        opened: &str,
        received: Option<&str>,
        browser: BrowserLabel,
        route: RecentRoute,
        at: SystemTime,
    ) -> Option<Self> {
        let host = host_of(opened)?;
        let before = received.and_then(host_of);
        Some(Self {
            rewritten: received.is_some_and(|received| received != opened),
            rewritten_from: before.filter(|before| *before != host),
            host,
            browser,
            route,
            at,
        })
    }

    /// A recent entry from a dispatch record. `None` when the link was
    /// rejected, failed to launch or has no host. A link a rule or default
    /// kept from opening (Don't open, `browser: null`) is listed as "not
    /// opened" with who decided (design: Not Opened 2d).
    pub fn from_record(record: &DispatchRecord) -> Option<Self> {
        // A held link is listed under Held, not here.
        if record.not_opened_reason() == Some(NotOpenedReason::Hold) {
            return None;
        }
        let not_opened = record.outcome == Outcome::NotOpened;
        if !not_opened && !matches!(record.outcome, Outcome::Opened | Outcome::DryRun) {
            return None;
        }
        let browser = record.browser.as_ref();
        if browser.is_none() && !not_opened {
            return None;
        }
        let mut received = None;
        let mut rewritten = false;
        let mut route = RecentRoute::Unnumbered;
        for step in &record.steps {
            match step {
                Step::Received { url } => received = Some(url.as_str()),
                Step::Rewrite { .. } => rewritten = true,
                Step::Matched { by } => {
                    route = match by {
                        Matched::Handler { index } => RecentRoute::Handler(index + 1),
                        Matched::Rule { index } => RecentRoute::Rule(index + 1),
                        Matched::Default {
                            owner: Some(Owner::Config),
                        } => RecentRoute::ConfigDefault,
                        Matched::Default { .. } => RecentRoute::Default,
                        Matched::Unreported => RecentRoute::Unnumbered,
                    };
                }
                Step::Fallback { .. } | Step::RefusedLoop { .. } => route = RecentRoute::Fallback,
                // After the step that chose the browser: the pause took
                // it. That step says which list the skipped rule is in;
                // the pause's own number is the config's.
                Step::Paused { skipped } => {
                    route = RecentRoute::Paused {
                        skipped: skipped_by(route, *skipped),
                    };
                }
                Step::Sent { sent, skipped } => {
                    route = RecentRoute::Sent {
                        sent: *sent,
                        skipped: skipped_by(route, *skipped),
                    };
                }
                _ => {}
            }
        }
        let (label, opened) = match browser {
            // A rule may name the browser by bundle id: show its name.
            Some(browser) => (
                BrowserLabel::new(
                    badge_for(&browser.name),
                    &crate::default_browser::browser_name(&browser.name),
                    (!browser.profile.is_empty()).then_some(browser.profile.as_str()),
                )
                .for_app(&browser.name),
                browser.url.as_str(),
            ),
            None => (
                BrowserLabel::new(
                    crate::widgets::AppBadge::from(crate::widgets::NOT_OPENED_BADGE),
                    "not opened",
                    None,
                ),
                record.final_url.as_deref()?,
            ),
        };
        Self::from_urls(
            opened,
            received.filter(|_| rewritten),
            label,
            route,
            record.received_at,
        )
    }

    /// The second line: "Firefox · rewritten · rule 3".
    pub fn detail(&self) -> String {
        let mut parts = vec![self.browser.name.clone()];
        if let Some(profile) = &self.browser.profile {
            parts.push(profile.clone());
        }
        if self.rewritten {
            parts.push("rewritten".to_owned());
        }
        match self.route {
            RecentRoute::Handler(n) => parts.push(format!("config handler {n}")),
            RecentRoute::Rule(n) => parts.push(format!("rule {n}")),
            RecentRoute::Unnumbered => {}
            RecentRoute::Default => parts.push("default".to_owned()),
            RecentRoute::ConfigDefault => parts.push("config default".to_owned()),
            RecentRoute::Fallback => parts.push("fallback".to_owned()),
            RecentRoute::Paused { skipped } => {
                parts.push("paused".to_owned());
                parts.extend(skipped_part(skipped));
            }
            RecentRoute::Sent { sent, skipped } => {
                parts.push(sent.short().to_owned());
                parts.extend(skipped_part(skipped));
            }
        }
        parts.join(" \u{00b7} ")
    }

    /// "now", "4m", "2h", "3d".
    pub fn age(&self, now: SystemTime) -> String {
        age(now.duration_since(self.at).unwrap_or_default())
    }
}

/// What a pause or a send passed over: the step that chose the browser
/// says which list; else the config's handler number the step carries.
fn skipped_by(route: RecentRoute, skipped: Option<usize>) -> Option<Skipped> {
    match route {
        RecentRoute::Rule(n) => Some(Skipped::Rule(n)),
        RecentRoute::Handler(n) => Some(Skipped::Handler(n)),
        _ => skipped.map(|index| Skipped::Handler(index + 1)),
    }
}

fn skipped_part(skipped: Option<Skipped>) -> Option<String> {
    match skipped {
        Some(Skipped::Handler(n)) => Some(format!("skipped config handler {n}")),
        Some(Skipped::Rule(n)) => Some(format!("skipped rule {n}")),
        None => None,
    }
}

/// The host of a URL, plus a non-default port. Never the path, query,
/// fragment or credentials.
pub fn host_of(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    let host = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    })
}

/// "41:52", "1:41:52": the time left, to the second, rounded up.
pub fn countdown(left: Duration) -> String {
    let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
    let (hours, minutes, seconds) = (secs / 3600, secs / 60 % 60, secs % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// A link that was held and still waits to be opened (design: Incoming
/// Links 2c). Only a hold's links: a rule that opens nothing is Activity's
/// to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldLink {
    pub id: DispatchId,
    /// Host, with the port when it is not the scheme's default.
    pub host: String,
    /// The path, query and fragment, when the record keeps the whole link;
    /// empty for a record kept as host only, or a bare host.
    pub rest: String,
    /// The app that sent it, by name.
    pub from: Option<String>,
    /// Where the rules would have opened it.
    pub rules: Option<BrowserLabel>,
    pub at: SystemTime,
}

impl HeldLink {
    /// The held link a record is. `None` unless a hold kept it, it keeps
    /// at least the host, and it has one.
    pub fn from_record(record: &DispatchRecord) -> Option<Self> {
        if record.not_opened_reason() != Some(NotOpenedReason::Hold)
            || record.privacy == Privacy::None
        {
            return None;
        }
        let url = url::Url::parse(&record.link.url).ok()?;
        let host = host_of(url.as_str())?;
        let rest = if record.privacy == Privacy::Full {
            let mut rest = url.path().to_owned();
            if let Some(query) = url.query() {
                rest.push('?');
                rest.push_str(query);
            }
            if let Some(fragment) = url.fragment() {
                rest.push('#');
                rest.push_str(fragment);
            }
            if rest == "/" { String::new() } else { rest }
        } else {
            String::new()
        };
        let rules = record.steps.iter().find_map(|step| match step {
            Step::Held {
                browser: Some(browser),
                ..
            } => Some(
                BrowserLabel::new(
                    badge_for(&browser.name),
                    &crate::default_browser::browser_name(&browser.name),
                    (!browser.profile.is_empty()).then_some(browser.profile.as_str()),
                )
                .for_app(&browser.name),
            ),
            _ => None,
        });
        Some(Self {
            id: record.id,
            host,
            rest,
            from: record
                .link
                .opener
                .as_ref()
                .map(|opener| opener.name.clone())
                .filter(|name| !name.is_empty()),
            rules,
            at: record.received_at,
        })
    }

    /// The second line: "from Chat · rules:" before the rules' browser,
    /// or "from Chat · the rules open nothing".
    pub fn detail(&self) -> String {
        let mut parts = Vec::new();
        if let Some(from) = &self.from {
            parts.push(format!("from {from}"));
        }
        parts.push(
            if self.rules.is_some() {
                "rules:"
            } else {
                "the rules open nothing"
            }
            .to_owned(),
        );
        parts.join(" \u{00b7} ")
    }
}

fn age(elapsed: Duration) -> String {
    let minutes = elapsed.as_secs() / 60;
    match minutes {
        0 => "now".to_owned(),
        1..60 => format!("{minutes}m"),
        60..1440 => format!("{}h", minutes / 60),
        _ => format!("{}d", minutes / 1440),
    }
}

/// What the tray takes from the dispatch feed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dispatched {
    /// The newest [`RECENT_LIMIT`] links that opened.
    pub recents: Vec<Recent>,
    /// Every held link that still waits, newest first.
    pub held: Vec<HeldLink>,
    pub links_today: u64,
}

impl Dispatched {
    /// With a router, only what its buffer still holds is held: it drops a
    /// record by age, and the link with it, while the feed keeps the record
    /// until a later push.
    pub fn of(feed: &Feed, now: SystemTime) -> Self {
        let mut dispatched = Self::of_records(feed.records(), feed.today(now).routed);
        if feed.router().is_some() {
            let pending = feed.pending();
            dispatched.held.retain(|held| pending.contains(&held.id));
        }
        dispatched
    }

    /// From records, newest first. A held link that was opened since has a
    /// newer record naming it ([`DispatchRecord::released_from`]), and no
    /// longer waits; one whose record left the feed left the router's
    /// buffer with it.
    pub fn of_records(records: &[DispatchRecord], links_today: u64) -> Self {
        let released: std::collections::HashSet<DispatchId> = records
            .iter()
            .filter_map(DispatchRecord::released_from)
            .collect();
        Self {
            recents: records
                .iter()
                .filter_map(Recent::from_record)
                .take(RECENT_LIMIT)
                .collect(),
            held: records
                .iter()
                .filter(|record| !released.contains(&record.id))
                .filter_map(HeldLink::from_record)
                .collect(),
            links_today,
        }
    }
}

/// The header chip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderChip {
    /// `.chip.ok` "default".
    Default,
    /// `.chip.warn` "paused".
    Paused,
    /// `.chip` "not default": links do not reach PwrRoute yet.
    NotDefault,
    /// `.chip.acc` "next link", "next 3" or "all links": the user sent
    /// links.
    Sending(SendScope),
    /// `.chip.info` "holding".
    Holding,
}

impl HeaderChip {
    pub fn label(self) -> String {
        match self {
            Self::Default => "default".to_owned(),
            Self::Paused => "paused".to_owned(),
            Self::NotDefault => "not default".to_owned(),
            Self::Sending(SendScope::Next { of: 1, .. }) => "next link".to_owned(),
            Self::Sending(SendScope::Next { of, .. }) => format!("next {of}"),
            Self::Sending(SendScope::All { .. }) => "all links".to_owned(),
            Self::Holding => "holding".to_owned(),
        }
    }
}

/// The sync row's color (design: Synced Rules 3c's `.mi-row.sync`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncTone {
    /// Synced: the icon in the ok color.
    Ok,
    /// Syncing: the accent.
    Busy,
    /// Paused, or some folder unreachable.
    Warn,
    /// A conflict.
    Bad,
}

/// Sync's one row in the dropdown, and the native menu's first row:
/// "Rules synced 2 min ago" with "Dropbox, iCloud" beside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncRow {
    pub title: String,
    pub sub: Option<String>,
    pub tone: SyncTone,
}

impl SyncRow {
    /// The native menu's words: the title and the sub on one line.
    pub fn menu_label(&self) -> String {
        match &self.sub {
            Some(sub) => format!("{} \u{00b7} {sub}", self.title),
            None => self.title.clone(),
        }
    }

    /// Only a conflict's row acts (it opens Settings › Cloud Sync) in the native
    /// menu; the others only say how sync stands.
    pub fn actionable(&self) -> bool {
        self.tone == SyncTone::Bad
    }
}

/// Everything the tray surfaces draw.
#[derive(Clone, Debug)]
pub struct TrayModel {
    pub routing: Routing,
    /// PwrRoute is the OS default browser.
    pub is_default_browser: bool,
    /// Where links go while paused: the target in force, or where a pause
    /// would send them now (`tray::pause_target`).
    pub pause_target: BrowserLabel,
    /// Newest first. The dropdown shows the first [`RECENT_LIMIT`].
    pub recents: Vec<Recent>,
    /// Held links that still wait, newest first. The dropdown shows the
    /// first [`HELD_LIMIT`].
    pub held: Vec<HeldLink>,
    pub links_today: u64,
    /// Where Send sends links: the one in force, else the last one chosen
    /// this session, else the Everything else browser.
    pub destination: SendDestination,
    /// The user picked [`destination`](Self::destination); until then it
    /// follows the pause target.
    pub destination_chosen: bool,
    /// The For row's choice when Send or Hold turns on: the last chosen
    /// this session, else the next link.
    pub for_kind: ForKind,
    /// How many links For › Links takes when turned on (the stepper).
    pub next_count: u32,
    /// For › Time's length, an index into [`TIME_STEPS`].
    pub time_step: usize,
    /// Hold can turn on: Activity keeps links in memory, so a held link
    /// has somewhere to wait (design: Incoming Links 5).
    pub can_hold: bool,
    /// What the destination lanes offer, from the browser scan; read when
    /// the dropdown opens.
    pub choices: DestinationChoices,
    /// Sync's row, while rules sync with a folder (`sync::tray_row_of`).
    pub sync: Option<SyncRow>,
}

impl TrayModel {
    /// The design's 1a: routing, three recent links. Made up for the mock.
    pub fn fixture(now: SystemTime) -> Self {
        let ago = |minutes: u64| {
            now.checked_sub(Duration::from_secs(minutes * 60 + 5))
                .unwrap_or(now)
        };
        let chrome = |profile: Option<&str>| BrowserLabel::new("Ch", "Google Chrome", profile);
        let recents = [
            Recent::from_urls(
                "https://tickets.example.com/browse/DEMO-1",
                None,
                chrome(Some("Work")),
                RecentRoute::Handler(2),
                ago(1),
            ),
            Recent::from_urls(
                "https://xcancel.com/example/status/1",
                Some("https://x.com/example/status/1"),
                BrowserLabel::new("Ff", "Firefox", None),
                RecentRoute::Handler(3),
                ago(2),
            ),
            Recent::from_urls(
                "http://localhost:5173/",
                None,
                BrowserLabel::new("Ar", "Arc", None),
                RecentRoute::Handler(5),
                ago(4),
            ),
        ];
        let edge = |profile: &str| BrowserLabel::new("Ed", "Microsoft Edge", Some(profile));
        Self {
            routing: Routing::Active,
            is_default_browser: true,
            pause_target: chrome(Some("Personal")),
            recents: recents.into_iter().flatten().collect(),
            held: Vec::new(),
            links_today: 1412,
            destination: SendDestination::profile(edge("Work")),
            destination_chosen: true,
            for_kind: ForKind::Links,
            next_count: 1,
            time_step: DEFAULT_TIME_STEP,
            can_hold: true,
            choices: DestinationChoices {
                browsers: vec![
                    BrowserChoice {
                        name: "Microsoft Edge".into(),
                        profiles: vec!["Personal".into(), "Work".into(), "Travel".into()],
                    },
                    BrowserChoice {
                        name: "Google Chrome".into(),
                        profiles: vec!["Personal".into(), "Work".into()],
                    },
                    BrowserChoice {
                        name: "Firefox".into(),
                        profiles: Vec::new(),
                    },
                ],
                installed: Vec::new(),
                scanned: true,
            },
            sync: None,
        }
    }

    /// The model at launch: routing, nothing routed yet, not the default
    /// browser until the OS says so (`tray::follow_default_browser`), and
    /// `pause_target` as the files name it (`tray::pause_target`).
    pub fn starting(pause_target: BrowserLabel) -> Self {
        Self {
            routing: Routing::Active,
            is_default_browser: false,
            destination: SendDestination::profile(pause_target.clone()),
            destination_chosen: false,
            pause_target,
            recents: Vec::new(),
            held: Vec::new(),
            links_today: 0,
            for_kind: ForKind::Links,
            next_count: 1,
            time_step: DEFAULT_TIME_STEP,
            can_hold: true,
            choices: DestinationChoices::default(),
            sync: None,
        }
    }

    /// The recents, the held links and the count from the dispatch feed.
    pub fn sync(&mut self, dispatched: Dispatched) {
        self.recents = dispatched.recents;
        self.held = dispatched.held;
        self.links_today = dispatched.links_today;
    }

    /// Incoming Links 2c: holding for an hour, two links held. Made up for
    /// the mock.
    pub fn holding_fixture(now: SystemTime) -> Self {
        let mut model = Self::fixture(now);
        model.routing = Routing::Holding(SendScope::All {
            until: Some(now + Duration::from_secs(41 * 60 + 52)),
        });
        model.for_kind = ForKind::Time;
        model.recents.truncate(1);
        let held = |id: u64, host: &str, rest: &str, from: &str, rules, minutes: u64| HeldLink {
            id: DispatchId(id),
            host: host.to_owned(),
            rest: rest.to_owned(),
            from: Some(from.to_owned()),
            rules: Some(rules),
            at: now - Duration::from_secs(minutes * 60),
        };
        model.held = vec![
            held(
                9002,
                "chat.example.com",
                "/r/team-7?ref=bot",
                "Chat",
                BrowserLabel::new("Ch", "Google Chrome", Some("Work")),
                1,
            ),
            held(
                9001,
                "l.example.net",
                "/t/9f2c1a?u=docs",
                "Mail",
                BrowserLabel::new("Sa", "Safari", None),
                3,
            ),
        ];
        model
    }

    /// The design's 1b: paused, with a link that skipped its handler.
    pub fn paused_fixture(now: SystemTime) -> Self {
        let mut model = Self::fixture(now);
        model.routing = Routing::Paused;
        model.recents.truncate(1);
        model.recents.insert(
            0,
            Recent::from_urls(
                "https://bsky.app/profile/example",
                None,
                BrowserLabel::new("Ch", "Google Chrome", None),
                RecentRoute::Paused {
                    skipped: Some(Skipped::Handler(4)),
                },
                now,
            )
            .expect("a fixture URL with a host"),
        );
        model
    }

    pub fn is_paused(&self) -> bool {
        self.routing == Routing::Paused
    }

    #[cfg(test)]
    pub fn set_paused(&mut self, paused: bool) {
        self.routing = if paused {
            Routing::Paused
        } else {
            Routing::Active
        };
    }

    /// The destination lanes, the destination's pill lit. A destination
    /// the scan lacks (a default browser since removed) keeps a lane of
    /// its own, first.
    pub fn lanes(&self) -> Vec<Lane> {
        let mut current = self.destination.choice();
        // The name the reader sees may not be the scan's (an app named by
        // bundle id or path): then let the lanes find it by what the
        // router is told.
        if !self
            .choices
            .browsers
            .iter()
            .any(|browser| browser.name.eq_ignore_ascii_case(&current.browser))
        {
            self.destination
                .browser
                .app()
                .clone_into(&mut current.browser);
        }
        destinations::lanes(
            &self.choices.browsers,
            &self.choices.installed,
            &current,
            None,
            self.choices.scanned,
            // Sending links nowhere is Hold, not a destination.
            false,
        )
    }

    /// The send in force, when the user sent links.
    pub fn sending(&self) -> Option<SendScope> {
        match self.routing {
            Routing::Sending(scope) => Some(scope),
            _ => None,
        }
    }

    /// The hold in force.
    pub fn holding(&self) -> Option<SendScope> {
        match self.routing {
            Routing::Holding(scope) => Some(scope),
            _ => None,
        }
    }

    /// The send's or the hold's scope, whichever is on.
    pub fn scope(&self) -> Option<SendScope> {
        self.sending().or(self.holding())
    }

    /// Which segment of the Incoming links switch is lit.
    pub fn mode(&self) -> Mode {
        match self.routing {
            Routing::Active => Mode::Rules,
            Routing::Paused => Mode::Pause,
            Routing::Sending(_) => Mode::Send,
            Routing::Holding(_) => Mode::Hold,
        }
    }

    /// Which For choice is lit: the running scope's, else the one Send or
    /// Hold will start with.
    pub fn for_shown(&self) -> ForKind {
        self.scope().map_or(self.for_kind, ForKind::of)
    }

    /// The scope Send or Hold starts with, as the For row stands: the
    /// stepper's count, the time from `now`, or until switched back.
    pub fn scope_for(&self, kind: ForKind, now: SystemTime) -> SendScope {
        match kind {
            ForKind::Links => {
                let n = self.next_count.clamp(1, MAX_NEXT);
                SendScope::Next { left: n, of: n }
            }
            ForKind::Time => SendScope::All {
                until: Some(now + TIME_STEPS[self.time_step.min(TIME_STEPS.len() - 1)]),
            },
            ForKind::UntilOff => SendScope::All { until: None },
        }
    }

    /// The router's send as the switch stands.
    #[cfg(test)]
    pub fn send_for(&self, now: SystemTime) -> pwrroute_router::SendTo {
        pwrroute_router::SendTo {
            destination: self.destination.target(),
            scope: self.scope_for(self.for_kind, now),
        }
    }

    /// The stepper's value: the count (of the links a running Links scope
    /// takes in all) or the time; `None` for Until off.
    pub fn stepper(&self) -> Option<String> {
        match self.for_shown() {
            ForKind::Links => Some(match self.scope() {
                Some(SendScope::Next { of, .. }) => of.to_string(),
                _ => self.next_count.to_string(),
            }),
            ForKind::Time => Some(time_step_label(self.time_step)),
            ForKind::UntilOff => None,
        }
    }

    /// The hint under the switch at rest (design: Incoming Links 2a): what
    /// Send and Hold would do, so the click is never a surprise.
    pub fn hint(&self) -> Option<Vec<Run>> {
        (self.mode() == Mode::Rules).then(|| {
            vec![
                Run::plain("Send goes to "),
                Run::strong(self.destination.display()),
                Run::plain(". Hold keeps links here, unopened, until you open them."),
            ]
        })
    }

    /// The live line while Send, Hold or Pause is on (design: Incoming
    /// Links 2b-2f): what happens, what is left, and the way back.
    pub fn live(&self, now: SystemTime) -> Option<Live> {
        let (tone, scope) = match self.routing {
            Routing::Active => return None,
            Routing::Paused => {
                let target = &self.pause_target;
                let mut runs = vec![
                    Run::plain("Every link to "),
                    Run::strong(target.name.clone()),
                ];
                if let Some(profile) = &target.profile {
                    runs.push(Run::plain(format!(" \u{00b7} {profile}")));
                }
                runs.push(Run::plain(". Rewrites still run."));
                return Some(Live {
                    tone: Tone::Pause,
                    runs,
                    button: "Resume",
                });
            }
            Routing::Sending(scope) => (Tone::Send, scope),
            Routing::Holding(scope) => (Tone::Hold, scope),
        };
        let held = (tone == Tone::Hold && !self.held.is_empty())
            .then(|| Run::plain(format!(" \u{00b7} {} held", self.held.len())));
        let then = Run::plain(", then your rules decide again");
        let mut runs = match scope {
            SendScope::Next { of: 1, .. } => vec![Run::strong("Next link")],
            SendScope::Next { left, of } => {
                vec![Run::strong(format!("{left} of {of}")), Run::plain(" left")]
            }
            SendScope::All { until: Some(until) } => vec![
                Run::strong(countdown(until.duration_since(now).unwrap_or_default())),
                Run::plain(" left"),
            ],
            SendScope::All { until: None } => {
                let mut runs = vec![
                    Run::strong("Every link"),
                    Run::plain(" until you go back to rules"),
                ];
                runs.extend(held);
                return Some(Live {
                    tone,
                    runs,
                    button: "Back to rules",
                });
            }
        };
        runs.extend(held);
        runs.push(then);
        Some(Live {
            tone,
            runs,
            button: "Back to rules",
        })
    }

    /// The held links the dropdown lists.
    pub fn shown_held(&self) -> &[HeldLink] {
        &self.held[..self.held.len().min(HELD_LIMIT)]
    }

    /// "2 held links will be lost", beside Quit while links wait: they
    /// live in memory only.
    pub fn quit_warning(&self) -> Option<String> {
        match self.held.len() {
            0 => None,
            1 => Some("1 held link will be lost".to_owned()),
            n => Some(format!("{n} held links will be lost")),
        }
    }

    pub fn chip(&self) -> HeaderChip {
        if self.is_paused() {
            HeaderChip::Paused
        } else if let Some(scope) = self.sending() {
            HeaderChip::Sending(scope)
        } else if self.holding().is_some() {
            HeaderChip::Holding
        } else if self.is_default_browser {
            HeaderChip::Default
        } else {
            HeaderChip::NotDefault
        }
    }

    /// The recent links the dropdown lists.
    pub fn shown_recents(&self) -> &[Recent] {
        &self.recents[..self.recents.len().min(RECENT_LIMIT)]
    }

    /// The design shows "Open Activity" under the recents only while routing;
    /// paused, the skipped-rule recents say enough.
    pub fn shows_activity_row(&self) -> bool {
        !self.is_paused()
    }

    /// "Make default browser…" sits under the commands while another app
    /// has the default: links are not reaching PwrRoute, and this is the
    /// surface a windowless router shows. Pausing does not hide it.
    pub fn shows_make_default_row(&self) -> bool {
        !self.is_default_browser
    }

    /// "1,412 today".
    pub fn today_label(&self) -> String {
        format!("{} today", thousands(self.links_today))
    }

    /// The tray icon's tooltip.
    pub fn tooltip(&self) -> String {
        match self.chip() {
            chip @ HeaderChip::Sending(_) => {
                return format!(
                    "PwrRoute \u{00b7} {} \u{00b7} {}",
                    chip.label(),
                    self.destination.display()
                );
            }
            HeaderChip::Holding if !self.held.is_empty() => {
                return format!(
                    "PwrRoute \u{00b7} holding \u{00b7} {} held",
                    self.held.len()
                );
            }
            HeaderChip::Holding => return "PwrRoute \u{00b7} holding".to_owned(),
            _ => {}
        }
        if self.is_paused() {
            format!(
                "PwrRoute \u{00b7} paused \u{00b7} {}",
                self.pause_target.display()
            )
        } else {
            "PwrRoute".to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
    }

    #[test]
    fn recents_keep_the_host_and_nothing_else() {
        let cases = [
            ("https://example.com/a/b?q=secret#frag", "example.com"),
            ("https://user:pw@example.com/", "example.com"),
            ("http://localhost:5173/app", "localhost:5173"),
            ("https://example.com:443/", "example.com"),
            ("http://[::1]:8080/", "[::1]:8080"),
        ];
        for (url, host) in cases {
            assert_eq!(host_of(url).as_deref(), Some(host), "{url}");
        }
        assert_eq!(host_of("mailto:someone@example.com"), None);
        assert_eq!(host_of("not a url"), None);
    }

    #[test]
    fn a_rewrite_that_changes_the_host_is_shown_as_before() {
        let recent = Recent::from_urls(
            "https://xcancel.com/a",
            Some("https://x.com/a"),
            BrowserLabel::new("Ff", "Firefox", None),
            RecentRoute::Handler(3),
            now(),
        )
        .unwrap();
        assert_eq!(recent.host, "xcancel.com");
        assert_eq!(recent.rewritten_from.as_deref(), Some("x.com"));
        assert_eq!(
            recent.detail(),
            "Firefox \u{00b7} rewritten \u{00b7} config handler 3"
        );

        // A rewrite that keeps the host (dropped tracking parameters) is
        // still a rewrite, with no "before" host.
        let same_host = Recent::from_urls(
            "https://example.com/a",
            Some("https://example.com/a?utm_source=x"),
            BrowserLabel::new("Ff", "Firefox", None),
            RecentRoute::Default,
            now(),
        )
        .unwrap();
        assert!(same_host.rewritten);
        assert_eq!(same_host.rewritten_from, None);
    }

    #[test]
    fn details_match_the_design() {
        let model = TrayModel::fixture(now());
        let details: Vec<String> = model.recents.iter().map(Recent::detail).collect();
        assert_eq!(
            details,
            [
                "Google Chrome \u{00b7} Work \u{00b7} config handler 2",
                "Firefox \u{00b7} rewritten \u{00b7} config handler 3",
                "Arc \u{00b7} config handler 5",
            ]
        );
        let paused = TrayModel::paused_fixture(now());
        assert_eq!(
            paused.recents[0].detail(),
            "Google Chrome \u{00b7} paused \u{00b7} skipped config handler 4"
        );
        assert_eq!(paused.recents.len(), 2);
    }

    #[test]
    fn ages_are_short() {
        let at = now();
        let recent = |secs: u64| Recent {
            at: at - Duration::from_secs(secs),
            ..TrayModel::fixture(at).recents[0].clone()
        };
        assert_eq!(recent(5).age(at), "now");
        assert_eq!(recent(65).age(at), "1m");
        assert_eq!(recent(59 * 60).age(at), "59m");
        assert_eq!(recent(2 * 3600).age(at), "2h");
        assert_eq!(recent(3 * 86_400).age(at), "3d");
        // A clock that went backwards reads as now, not a panic.
        assert_eq!(
            Recent {
                at: at + Duration::from_secs(60),
                ..recent(0)
            }
            .age(at),
            "now"
        );
    }

    #[test]
    fn pausing_changes_the_chip_and_the_copy() {
        let mut model = TrayModel::fixture(now());
        assert_eq!(model.chip(), HeaderChip::Default);
        assert!(model.shows_activity_row());
        assert_eq!(model.live(now()), None);

        model.set_paused(true);
        assert_eq!(model.chip(), HeaderChip::Paused);
        assert_eq!(model.mode(), Mode::Pause);
        assert!(!model.shows_activity_row());
        let live = model.live(now()).expect("a pause says so");
        assert_eq!(live.tone, Tone::Pause);
        assert_eq!(live.button, "Resume");
        assert_eq!(
            runs_text(&live.runs),
            "Every link to Google Chrome \u{00b7} Personal. Rewrites still run."
        );
        assert_eq!(model.hint(), None);

        model.set_paused(false);
        model.is_default_browser = false;
        assert_eq!(model.chip(), HeaderChip::NotDefault);
    }

    #[test]
    fn losing_the_default_flips_the_chip_and_offers_the_row() {
        // The registrar's re-read says Chrome took the default: the tray
        // follows it with no window involved.
        let mut model = TrayModel::fixture(now());
        assert_eq!(model.chip(), HeaderChip::Default);
        assert!(!model.shows_make_default_row());

        model.is_default_browser = false;
        assert_eq!(model.chip(), HeaderChip::NotDefault);
        assert!(model.shows_make_default_row());
        // Paused takes the chip, and the row stays: links still do not
        // reach PwrRoute.
        model.set_paused(true);
        assert_eq!(model.chip(), HeaderChip::Paused);
        assert!(model.shows_make_default_row());

        model.set_paused(false);
        model.is_default_browser = true;
        assert_eq!(model.chip(), HeaderChip::Default);
        assert!(!model.shows_make_default_row());
    }

    #[test]
    fn at_most_three_recents_show() {
        let mut model = TrayModel::fixture(now());
        let extra = model.recents[0].clone();
        model.recents.push(extra);
        assert_eq!(model.recents.len(), 4);
        assert_eq!(model.shown_recents().len(), RECENT_LIMIT);
        model.recents.clear();
        assert!(model.shown_recents().is_empty());
    }

    #[test]
    fn counts_get_separators() {
        assert_eq!(TrayModel::fixture(now()).today_label(), "1,412 today");
        assert_eq!(TrayModel::starting(chrome()).today_label(), "0 today");
    }

    fn chrome() -> BrowserLabel {
        BrowserLabel::new("Ch", "Google Chrome", Some("Personal"))
    }

    #[test]
    fn the_model_at_launch_claims_nothing() {
        let model = TrayModel::starting(chrome());
        assert_eq!(model.chip(), HeaderChip::NotDefault);
        assert!(!model.is_paused());
        assert_eq!(model.mode(), Mode::Rules);
        assert!(model.recents.is_empty());
        assert!(model.held.is_empty());
        assert_eq!(model.quit_warning(), None);
        // Send starts where a pause would go, for the next link.
        assert_eq!(
            runs_text(&model.hint().expect("a hint at rest")),
            "Send goes to Google Chrome \u{00b7} Personal. Hold keeps links here, unopened, \
             until you open them."
        );
        assert_eq!(model.for_shown(), ForKind::Links);
        assert_eq!(model.stepper().as_deref(), Some("1"));
    }

    #[test]
    fn a_paused_record_names_the_skipped_handler() {
        use pwrroute_router::{AppType, Browser, DispatchId, DispatchRecord, Privacy, Timings};
        let url = "https://bsky.app/profile/example";
        let chrome = Browser {
            name: "Google Chrome".into(),
            app_type: AppType::AppName,
            url: url.into(),
            profile: String::new(),
            args: Vec::new(),
            open_in_background: None,
            private: false,
            privacy: Privacy::Full,
        };
        let record = |steps: Vec<Step>| DispatchRecord {
            id: DispatchId(1),
            received_at: now(),
            privacy: Privacy::Full,
            link: pwrroute_router::Link::new(url, pwrroute_router::Source::CommandLine),
            forwarded: false,
            final_url: Some(url.into()),
            browser: Some(chrome.clone()),
            steps,
            outcome: Outcome::Opened,
            timings: Timings::default(),
        };
        let skipped = record(vec![
            Step::Received { url: url.into() },
            Step::Matched {
                by: Matched::Handler { index: 3 },
            },
            Step::Paused { skipped: Some(3) },
            Step::Browser {
                browser: chrome.clone(),
            },
            Step::Launched { dry_run: false },
        ]);
        let recent = Recent::from_record(&skipped).expect("it opened");
        assert_eq!(
            recent.route,
            RecentRoute::Paused {
                skipped: Some(Skipped::Handler(4))
            }
        );
        assert_eq!(recent.host, "bsky.app");
        assert_eq!(
            recent.detail(),
            "Google Chrome \u{00b7} paused \u{00b7} skipped config handler 4"
        );
        // The design's paused fixture is this same entry.
        assert_eq!(
            recent.detail(),
            TrayModel::paused_fixture(now()).recents[0].detail()
        );

        // No handler to skip: the default would have had it anyway.
        let default = record(vec![
            Step::Received { url: url.into() },
            Step::Matched {
                by: Matched::Default { owner: None },
            },
            Step::Paused { skipped: None },
            Step::Browser {
                browser: chrome.clone(),
            },
            Step::Launched { dry_run: false },
        ]);
        let recent = Recent::from_record(&default).unwrap();
        assert_eq!(recent.route, RecentRoute::Paused { skipped: None });
        assert_eq!(recent.detail(), "Google Chrome \u{00b7} paused");
    }

    /// A record whose browser a rule named by bundle id: the recent names
    /// the app, and keeps the id it was opened with.
    #[test]
    fn a_recent_names_a_bundle_id_browser() {
        use pwrroute_router::{AppType, Browser, DispatchId, DispatchRecord, Privacy, Timings};
        let url = "https://example.com/";
        let edge = Browser {
            name: "com.microsoft.edgemac".into(),
            app_type: AppType::BundleId,
            url: url.into(),
            profile: "Work".into(),
            args: Vec::new(),
            open_in_background: None,
            private: false,
            privacy: Privacy::Full,
        };
        let record = DispatchRecord {
            id: DispatchId(1),
            received_at: now(),
            privacy: Privacy::Full,
            link: pwrroute_router::Link::new(url, pwrroute_router::Source::CommandLine),
            forwarded: false,
            final_url: Some(url.into()),
            browser: Some(edge.clone()),
            steps: vec![
                Step::Received { url: url.into() },
                Step::Matched {
                    by: Matched::Handler { index: 0 },
                },
                Step::Browser { browser: edge },
                Step::Launched { dry_run: false },
            ],
            outcome: Outcome::Opened,
            timings: Timings::default(),
        };
        let recent = Recent::from_record(&record).expect("it opened");
        assert_eq!(recent.browser.name, "Microsoft Edge");
        assert_eq!(recent.browser.app(), "com.microsoft.edgemac");
        assert_eq!(
            recent.detail(),
            "Microsoft Edge \u{00b7} Work \u{00b7} config handler 1"
        );
    }

    #[test]
    fn the_model_follows_the_feed() {
        let records = crate::fixtures::activity();
        let feed = Feed::from_records(records.clone());
        let mut model = TrayModel::starting(chrome());
        assert!(model.recents.is_empty());
        model.sync(Dispatched::of(&feed, SystemTime::now()));
        // The failed launch is skipped; the three newest that opened show.
        let details: Vec<String> = model.recents.iter().map(Recent::detail).collect();
        assert_eq!(
            details,
            [
                "Google Chrome \u{00b7} Work \u{00b7} config handler 2",
                "Firefox \u{00b7} default-release \u{00b7} rewritten \u{00b7} config handler 3",
                "Arc \u{00b7} config handler 5",
            ]
        );
        assert_eq!(model.recents[1].host, "xcancel.com");
        assert_eq!(model.recents[1].rewritten_from.as_deref(), Some("x.com"));
        assert_eq!(model.links_today, records.len() as u64);
        assert!(records.iter().all(|r| Recent::from_record(r).is_none()
            || matches!(
                r.outcome,
                Outcome::Opened | Outcome::DryRun | Outcome::NotOpened
            )));

        let default = Recent::from_record(&records[6]).expect("the default opened");
        assert_eq!(default.route, RecentRoute::Default);
        assert_eq!(default.host, "www.apple.com");
        assert_eq!(
            default.detail(),
            "Google Chrome \u{00b7} Personal \u{00b7} default"
        );
    }

    /// A link a rule kept from opening is listed, as "not opened" with
    /// who decided (design: Not Opened 2d).
    #[test]
    fn a_not_opened_link_is_a_recent_that_says_so() {
        use pwrroute_router::{DispatchId, DispatchRecord, Privacy, Timings};
        let url = "https://ads.example.net/click?id=7";
        let record = |by| DispatchRecord {
            id: DispatchId(1),
            received_at: now(),
            privacy: Privacy::Full,
            link: pwrroute_router::Link::new(url, pwrroute_router::Source::CommandLine),
            forwarded: false,
            final_url: Some(url.into()),
            browser: None,
            steps: vec![
                Step::Received { url: url.into() },
                Step::Matched { by },
                Step::NotOpened,
            ],
            outcome: Outcome::NotOpened,
            timings: Timings::default(),
        };
        let rule = Recent::from_record(&record(Matched::Rule { index: 3 })).unwrap();
        assert_eq!(rule.host, "ads.example.net");
        assert_eq!(rule.detail(), "not opened \u{00b7} rule 4");
        let default = Recent::from_record(&record(Matched::Default {
            owner: Some(Owner::PwrRoute),
        }))
        .unwrap();
        assert_eq!(default.detail(), "not opened \u{00b7} default");
    }

    /// Attribution (WP-B): a recent names who routed it, `rules.json`'s
    /// rules as "rule N", the config's handlers as "handler N", and whose
    /// default took an unclaimed link.
    #[test]
    fn a_recent_names_the_rule_or_default_that_routed_it() {
        use pwrroute_router::{AppType, Browser, DispatchId, DispatchRecord, Privacy, Timings};
        let url = "https://docs.example.com/a";
        let firefox = Browser {
            name: "Firefox".into(),
            app_type: AppType::AppName,
            url: url.into(),
            profile: String::new(),
            args: Vec::new(),
            open_in_background: None,
            private: false,
            privacy: Privacy::Full,
        };
        let detail = |steps: Vec<Step>| {
            let mut all = vec![Step::Received { url: url.into() }];
            all.extend(steps);
            all.push(Step::Browser {
                browser: firefox.clone(),
            });
            all.push(Step::Launched { dry_run: false });
            let record = DispatchRecord {
                id: DispatchId(1),
                received_at: now(),
                privacy: Privacy::Full,
                link: pwrroute_router::Link::new(url, pwrroute_router::Source::CommandLine),
                forwarded: false,
                final_url: Some(url.into()),
                browser: Some(firefox.clone()),
                steps: all,
                outcome: Outcome::Opened,
                timings: Timings::default(),
            };
            Recent::from_record(&record).expect("it opened").detail()
        };
        let matched = |by| Step::Matched { by };
        assert_eq!(
            detail(vec![matched(Matched::Rule { index: 2 })]),
            "Firefox \u{00b7} rule 3"
        );
        assert_eq!(
            detail(vec![matched(Matched::Handler { index: 1 })]),
            "Firefox \u{00b7} config handler 2"
        );
        assert_eq!(
            detail(vec![matched(Matched::Default {
                owner: Some(Owner::PwrRoute)
            })]),
            "Firefox \u{00b7} default"
        );
        assert_eq!(
            detail(vec![matched(Matched::Default {
                owner: Some(Owner::Config)
            })]),
            "Firefox \u{00b7} config default"
        );
        assert_eq!(
            detail(vec![matched(Matched::Unreported)]),
            "Firefox",
            "a rule the record does not number is not named"
        );
        // Paused over a rules.json rule: the record's Matched step says
        // which list, whatever the pause step numbers.
        assert_eq!(
            detail(vec![
                matched(Matched::Rule { index: 1 }),
                Step::Paused { skipped: None },
            ]),
            "Firefox \u{00b7} paused \u{00b7} skipped rule 2"
        );
        assert_eq!(
            detail(vec![
                matched(Matched::Rule { index: 1 }),
                Step::Paused { skipped: Some(1) },
            ]),
            "Firefox \u{00b7} paused \u{00b7} skipped rule 2"
        );
    }

    fn with(routing: Routing) -> TrayModel {
        let mut model = TrayModel::fixture(now());
        model.routing = routing;
        model
    }

    fn live_text(model: &TrayModel) -> String {
        runs_text(&model.live(now()).expect("something is on").runs)
    }

    #[test]
    fn the_switch_says_what_is_on_and_what_is_left() {
        let model = TrayModel::fixture(now());
        assert_eq!(model.mode(), Mode::Rules);
        assert_eq!(model.chip(), HeaderChip::Default);

        let next = with(Routing::Sending(SendScope::Next { left: 1, of: 1 }));
        assert_eq!(next.mode(), Mode::Send);
        assert_eq!(next.chip().label(), "next link");
        assert_eq!(next.live(now()).unwrap().tone, Tone::Send);
        assert_eq!(live_text(&next), "Next link, then your rules decide again");
        assert_eq!(next.live(now()).unwrap().button, "Back to rules");
        assert_eq!(
            next.tooltip(),
            "PwrRoute \u{00b7} next link \u{00b7} Microsoft Edge \u{00b7} Work"
        );
        assert_eq!(next.hint(), None);

        // Design 2b: one of three used. The stepper shows the three.
        let counting = with(Routing::Sending(SendScope::Next { left: 2, of: 3 }));
        assert_eq!(counting.chip().label(), "next 3");
        assert_eq!(
            live_text(&counting),
            "2 of 3 left, then your rules decide again"
        );
        assert_eq!(counting.stepper().as_deref(), Some("3"));
        assert_eq!(counting.for_shown(), ForKind::Links);

        let timed = with(Routing::Sending(SendScope::All {
            until: Some(now() + Duration::from_secs(41 * 60 + 52)),
        }));
        assert_eq!(timed.chip().label(), "all links");
        assert_eq!(
            live_text(&timed),
            "41:52 left, then your rules decide again"
        );
        assert_eq!(timed.for_shown(), ForKind::Time);
        assert_eq!(timed.stepper().as_deref(), Some("1h"));
        let until_off = with(Routing::Sending(SendScope::All { until: None }));
        assert_eq!(
            live_text(&until_off),
            "Every link until you go back to rules"
        );
        assert_eq!(until_off.for_shown(), ForKind::UntilOff);
        assert_eq!(until_off.stepper(), None);

        // Hold (design 2c, 2f): info blue, the held count in the line.
        let hold = TrayModel::holding_fixture(now());
        assert_eq!(hold.mode(), Mode::Hold);
        assert_eq!(hold.chip(), HeaderChip::Holding);
        assert_eq!(hold.chip().label(), "holding");
        assert_eq!(hold.live(now()).unwrap().tone, Tone::Hold);
        assert_eq!(
            live_text(&hold),
            "41:52 left \u{00b7} 2 held, then your rules decide again"
        );
        assert_eq!(hold.tooltip(), "PwrRoute \u{00b7} holding \u{00b7} 2 held");
        assert_eq!(
            hold.quit_warning().as_deref(),
            Some("2 held links will be lost")
        );
        let mut next_hold = with(Routing::Holding(SendScope::Next { left: 1, of: 1 }));
        assert_eq!(
            live_text(&next_hold),
            "Next link, then your rules decide again"
        );
        next_hold.held = hold.held.clone();
        next_hold.routing = Routing::Holding(SendScope::All { until: None });
        assert_eq!(
            live_text(&next_hold),
            "Every link until you go back to rules \u{00b7} 2 held"
        );
        // Held links outlive the hold.
        next_hold.routing = Routing::Active;
        assert_eq!(next_hold.live(now()), None);
        assert_eq!(next_hold.shown_held().len(), 2);
        assert!(next_hold.quit_warning().is_some());

        // Paused wins the chip.
        let mut paused = TrayModel::fixture(now());
        paused.set_paused(true);
        assert_eq!(paused.chip(), HeaderChip::Paused);
    }

    #[test]
    fn countdowns_round_up_to_the_second() {
        assert_eq!(countdown(Duration::from_secs(41 * 60 + 52)), "41:52");
        assert_eq!(countdown(Duration::from_millis(59_001)), "1:00");
        assert_eq!(countdown(Duration::ZERO), "0:00");
        assert_eq!(countdown(Duration::from_secs(3 * 3600 + 5)), "3:00:05");
        assert_eq!(time_step_label(0), "15m");
        assert_eq!(time_step_label(DEFAULT_TIME_STEP), "1h");
        assert_eq!(time_step_label(TIME_STEPS.len() - 1), "8h");
        assert_eq!(time_step_label(99), "8h");
    }

    #[test]
    fn the_for_row_asks_the_router_for_the_count_the_time_or_until_off() {
        let mut model = TrayModel::fixture(now());
        model.next_count = 3;
        let next = model.send_for(now());
        assert_eq!(next.scope, SendScope::Next { left: 3, of: 3 });
        assert_eq!(
            next.destination,
            Destination {
                browser: "Microsoft Edge".into(),
                profile: "Work".into(),
                private: false,
            }
        );
        model.for_kind = ForKind::Time;
        model.time_step = 0;
        assert_eq!(
            model.send_for(now()).scope,
            SendScope::All {
                until: Some(now() + Duration::from_secs(15 * 60))
            }
        );
        model.for_kind = ForKind::UntilOff;
        assert_eq!(
            model.scope_for(model.for_kind, now()),
            SendScope::All { until: None }
        );
        model.next_count = 500;
        assert_eq!(
            model.scope_for(ForKind::Links, now()),
            SendScope::Next {
                left: MAX_NEXT,
                of: MAX_NEXT
            }
        );
        // A private window is the browser's, with no profile.
        model.destination = crate::tray::destination("Microsoft Edge", "", true);
        assert_eq!(
            model.destination.display(),
            "Microsoft Edge \u{00b7} InPrivate"
        );
        assert_eq!(
            model.destination.target(),
            Destination {
                browser: "Microsoft Edge".into(),
                profile: String::new(),
                private: true,
            }
        );
    }

    #[test]
    fn a_held_link_shows_its_path_only_when_its_record_keeps_it() {
        use pwrroute_router::{AppType, Browser, DispatchId, Link, OpenerInfo, Source, Timings};
        let url = "https://chat.example.com/r/team-7?ref=bot#top";
        let chrome = Browser {
            name: "com.google.Chrome".into(),
            app_type: AppType::BundleId,
            url: url.into(),
            profile: "Work".into(),
            args: Vec::new(),
            open_in_background: None,
            private: false,
            privacy: Privacy::Full,
        };
        let held = |id: u64, privacy: Privacy| DispatchRecord {
            id: DispatchId(id),
            received_at: now(),
            privacy,
            link: Link::new(url, Source::AppleEvent).with_opener(Some(OpenerInfo {
                name: "Chat".into(),
                ..OpenerInfo::default()
            })),
            forwarded: false,
            final_url: Some(url.into()),
            browser: None,
            steps: vec![
                Step::Received { url: url.into() },
                Step::Matched {
                    by: Matched::Rule { index: 1 },
                },
                Step::Held {
                    held: Sent::Next { nth: 1, of: 2 },
                    skipped: None,
                    browser: Some(chrome.clone()),
                },
                Step::NotOpened,
            ],
            outcome: Outcome::NotOpened,
            timings: Timings::default(),
        };
        let full = HeldLink::from_record(&held(1, Privacy::Full)).expect("a held link");
        assert_eq!(full.host, "chat.example.com");
        assert_eq!(full.rest, "/r/team-7?ref=bot#top");
        assert_eq!(full.from.as_deref(), Some("Chat"));
        let rules = full.rules.clone().expect("where the rules would open it");
        assert_eq!(rules.name, "Google Chrome");
        assert_eq!(rules.app(), "com.google.Chrome");
        assert_eq!(rules.profile.as_deref(), Some("Work"));
        assert_eq!(full.detail(), "from Chat \u{00b7} rules:");

        let mut host_only = held(2, Privacy::Host);
        host_only.link.url = "https://chat.example.com".into();
        let host = HeldLink::from_record(&host_only).expect("a host-only held link");
        assert_eq!(host.rest, "");
        assert_eq!(HeldLink::from_record(&held(3, Privacy::None)), None);

        // A rule that opens nothing is not a hold's: Activity shows it.
        let mut by_rule = held(4, Privacy::Full);
        by_rule
            .steps
            .retain(|step| !matches!(step, Step::Held { .. }));
        assert_eq!(HeldLink::from_record(&by_rule), None);

        // Opened later: the newer record names it, and it waits no more.
        let mut released = held(5, Privacy::Full);
        released.steps = vec![
            Step::Received { url: url.into() },
            Step::Released {
                from: DispatchId(1),
                chosen: false,
            },
            Step::Launched { dry_run: false },
        ];
        released.outcome = Outcome::Opened;
        released.browser = Some(chrome.clone());
        let dispatched = Dispatched::of_records(
            &[released, held(2, Privacy::Host), held(1, Privacy::Full)],
            3,
        );
        assert_eq!(
            dispatched
                .held
                .iter()
                .map(|held| held.id)
                .collect::<Vec<_>>(),
            [DispatchId(2)]
        );
        assert_eq!(dispatched.recents.len(), 1);
    }

    #[test]
    fn a_sent_record_says_so_in_the_recents() {
        let records = crate::fixtures::sent_activity();
        let next = Recent::from_record(&records[0]).unwrap();
        assert!(matches!(
            next.route,
            RecentRoute::Sent {
                sent: Sent::Next { nth: 1, of: 3 },
                ..
            }
        ));
        assert!(
            next.detail().starts_with("Microsoft Edge \u{00b7} Work"),
            "{}",
            next.detail()
        );
        assert!(next.detail().contains("next link"), "{}", next.detail());
        let all = Recent::from_record(&records[1]).unwrap();
        assert!(all.detail().contains("all links"), "{}", all.detail());
        // Over a rules.json rule, the rule is named from its own list.
        let recent = Recent::from_record(&DispatchRecord {
            steps: vec![
                Step::Received {
                    url: "https://docs.example.com/".into(),
                },
                Step::Matched {
                    by: Matched::Rule { index: 1 },
                },
                Step::Sent {
                    sent: Sent::Next { nth: 1, of: 1 },
                    skipped: None,
                },
            ],
            ..records[0].clone()
        })
        .unwrap();
        assert!(
            recent
                .detail()
                .ends_with("next link \u{00b7} skipped rule 2"),
            "{}",
            recent.detail()
        );
    }
}
