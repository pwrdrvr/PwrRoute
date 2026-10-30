//! What the menu bar dropdown and the tray menu show, as plain data.
//!
//! The recents and the count come from the router's dispatch records
//! ([`Recent::from_record`], [`TrayModel::sync`]), the pause state from the
//! tray's own toggle (`tray::toggle_pause`, which pauses the router), and
//! whether PwrRoute is the default browser from the OS
//! (`tray::follow_default_browser`). [`TrayModel::starting`] is the model
//! before any of them has reported; [`TrayModel::fixture`] is the whole
//! design example, for screenshots and tests. Everything here is testable
//! without a window.
//!
//! Recent links keep a host, never a full URL: the dropdown is visible to
//! whoever is looking at the screen.

use std::time::{Duration, SystemTime};

use pwrroute_router::{
    Destination, DispatchRecord, MAX_NEXT, Matched, Outcome, Owner, SendScope, SendTo, Sent, Step,
};

use crate::activity::{Feed, thousands};
use crate::activity_actions::BrowserChoice;
use crate::destinations::{self, Choice, Lane};
use crate::overview_model::Installed;
use crate::widgets::{AppBadge, badge_for};

/// How many recent links the dropdown lists.
pub const RECENT_LIMIT: usize = 3;

/// Whether links follow the rules.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Routing {
    #[default]
    Active,
    /// Every link goes to [`TrayModel::pause_target`]; rewrites still run.
    Paused,
    /// The user sent links to [`TrayModel::destination`]: the next few, or
    /// all of them for a while (design: Send Links 1c-1e). The router
    /// counts; the tray follows it (`tray::sync_send`).
    Sending(SendScope),
}

/// The Send links switch: who picks the browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendMode {
    Rules,
    Next,
    All,
}

/// How long All links lasts.
pub const ALL_FOR: Duration = Duration::from_secs(60 * 60);

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

/// "52 min", "1 min", "under a minute".
fn time_left(left: Duration) -> String {
    let minutes = left.as_secs().div_ceil(60);
    match minutes {
        0 => "under a minute".to_owned(),
        _ => format!("{minutes} min"),
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
    pub links_today: u64,
}

impl Dispatched {
    pub fn of(feed: &Feed, now: SystemTime) -> Self {
        Self {
            recents: feed
                .records()
                .iter()
                .filter_map(Recent::from_record)
                .take(RECENT_LIMIT)
                .collect(),
            links_today: feed.today(now).routed,
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
    /// `.chip.acc` "next link" or "all links": the user sent links.
    Sending(Sent),
}

impl HeaderChip {
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Paused => "paused",
            Self::NotDefault => "not default",
            Self::Sending(sent) => sent.short(),
        }
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
    pub links_today: u64,
    /// Where Send links sends links: the one in force, else the last one
    /// chosen this session, else the Everything else browser.
    pub destination: SendDestination,
    /// The user picked [`destination`](Self::destination); until then it
    /// follows the pause target.
    pub destination_chosen: bool,
    /// How many links Next link takes when turned on (the stepper).
    pub next_count: u32,
    /// All links lasts [`ALL_FOR`] rather than until switched back.
    pub all_for_hour: bool,
    /// What the destination lanes offer, from the browser scan; read when
    /// the dropdown opens.
    pub choices: DestinationChoices,
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
                "https://pwrdrvr.atlassian.net/browse/DEMO-1",
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
            links_today: 1412,
            destination: SendDestination::profile(edge("Work")),
            destination_chosen: true,
            next_count: 1,
            all_for_hour: false,
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
            links_today: 0,
            next_count: 1,
            all_for_hour: false,
            choices: DestinationChoices::default(),
        }
    }

    /// The recents and the count from the dispatch feed.
    pub fn sync(&mut self, dispatched: Dispatched) {
        self.recents = dispatched.recents;
        self.links_today = dispatched.links_today;
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

    /// Which segment of the Send links switch is lit: none while paused.
    pub fn send_mode(&self) -> Option<SendMode> {
        match self.routing {
            Routing::Active => Some(SendMode::Rules),
            Routing::Paused => None,
            Routing::Sending(SendScope::Next { .. }) => Some(SendMode::Next),
            Routing::Sending(SendScope::All { .. }) => Some(SendMode::All),
        }
    }

    /// The router's send for `mode` as the switch stands: the stepper's
    /// count, the hour from `now`. `None` for Rules.
    pub fn send_for(&self, mode: SendMode, now: SystemTime) -> Option<SendTo> {
        let scope = match mode {
            SendMode::Rules => return None,
            SendMode::Next => {
                let n = self.next_count.clamp(1, MAX_NEXT);
                SendScope::Next { left: n, of: n }
            }
            SendMode::All => SendScope::All {
                until: self.all_for_hour.then(|| now + ALL_FOR),
            },
        };
        Some(SendTo {
            destination: self.destination.target(),
            scope,
        })
    }

    /// The line under the switch while Next link or All links is on
    /// (design 1c-1e): what happens after, or how long is left.
    pub fn send_detail(&self, now: SystemTime) -> Option<String> {
        match self.sending()? {
            SendScope::Next { left, of } if left < of => {
                Some(format!("{left} of {of} left, then the rules again"))
            }
            SendScope::Next { left: 1, .. } => Some("link, then the rules again".to_owned()),
            SendScope::Next { .. } => Some("links, then the rules again".to_owned()),
            SendScope::All { until: None } => None,
            SendScope::All { until: Some(until) } => Some(format!(
                "{} left",
                time_left(until.duration_since(now).unwrap_or_default())
            )),
        }
    }

    /// The stepper's number: what is left while Next link is on, else how
    /// many it will take.
    pub fn next_shown(&self) -> u32 {
        match self.sending() {
            Some(SendScope::Next { left, .. }) => left,
            _ => self.next_count,
        }
    }

    pub fn chip(&self) -> HeaderChip {
        if self.is_paused() {
            HeaderChip::Paused
        } else if let Some(scope) = self.sending() {
            HeaderChip::Sending(match scope {
                SendScope::Next { .. } => Sent::Next { nth: 1, of: 1 },
                SendScope::All { .. } => Sent::All,
            })
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

    /// The line under "Pause routing".
    pub fn pause_detail(&self) -> String {
        let target = self.pause_target.display();
        if self.is_paused() {
            format!("Everything goes to {target}. Rewrites still run.")
        } else {
            format!("Send every link to {target} until switched back.")
        }
    }

    /// "1,412 today".
    pub fn today_label(&self) -> String {
        format!("{} today", thousands(self.links_today))
    }

    /// The tray icon's tooltip.
    pub fn tooltip(&self) -> String {
        if let HeaderChip::Sending(sent) = self.chip() {
            return format!(
                "PwrRoute \u{00b7} {} \u{00b7} {}",
                sent.short(),
                self.destination.display()
            );
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
        assert_eq!(
            model.pause_detail(),
            "Send every link to Google Chrome \u{00b7} Personal until switched back."
        );

        model.set_paused(true);
        assert_eq!(model.chip(), HeaderChip::Paused);
        assert!(!model.shows_activity_row());
        assert_eq!(
            model.pause_detail(),
            "Everything goes to Google Chrome \u{00b7} Personal. Rewrites still run."
        );

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
        assert!(model.recents.is_empty());
        assert_eq!(
            model.pause_detail(),
            "Send every link to Google Chrome \u{00b7} Personal until switched back."
        );
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

    fn sending(scope: SendScope) -> TrayModel {
        let mut model = TrayModel::fixture(now());
        model.routing = Routing::Sending(scope);
        model
    }

    #[test]
    fn send_links_says_who_picks_the_browser() {
        let model = TrayModel::fixture(now());
        assert_eq!(model.send_mode(), Some(SendMode::Rules));
        assert_eq!(model.chip(), HeaderChip::Default);
        assert_eq!(model.send_detail(now()), None);

        let next = sending(SendScope::Next { left: 1, of: 1 });
        assert_eq!(next.send_mode(), Some(SendMode::Next));
        assert_eq!(next.chip().label(), "next link");
        assert_eq!(
            next.send_detail(now()).as_deref(),
            Some("link, then the rules again")
        );
        assert_eq!(
            next.tooltip(),
            "PwrRoute \u{00b7} next link \u{00b7} Microsoft Edge \u{00b7} Work"
        );
        let counting = sending(SendScope::Next { left: 2, of: 3 });
        assert_eq!(
            counting.send_detail(now()).as_deref(),
            Some("2 of 3 left, then the rules again")
        );
        assert_eq!(counting.next_shown(), 2);
        let three = sending(SendScope::Next { left: 3, of: 3 });
        assert_eq!(
            three.send_detail(now()).as_deref(),
            Some("links, then the rules again")
        );

        let all = sending(SendScope::All {
            until: Some(now() + Duration::from_secs(51 * 60 + 10)),
        });
        assert_eq!(all.chip().label(), "all links");
        assert_eq!(all.send_detail(now()).as_deref(), Some("52 min left"));
        assert_eq!(
            sending(SendScope::All { until: None }).send_detail(now()),
            None
        );

        // Paused: no segment is lit, and the pause wins the chip.
        let mut paused = TrayModel::fixture(now());
        paused.set_paused(true);
        assert_eq!(paused.send_mode(), None);
        assert_eq!(paused.chip(), HeaderChip::Paused);
    }

    #[test]
    fn the_switch_asks_the_router_for_the_stepper_count_or_the_hour() {
        let mut model = TrayModel::fixture(now());
        assert_eq!(model.send_for(SendMode::Rules, now()), None);
        model.next_count = 3;
        let next = model.send_for(SendMode::Next, now()).unwrap();
        assert_eq!(next.scope, SendScope::Next { left: 3, of: 3 });
        assert_eq!(
            next.destination,
            Destination {
                browser: "Microsoft Edge".into(),
                profile: "Work".into(),
                private: false,
            }
        );
        assert_eq!(
            model.send_for(SendMode::All, now()).unwrap().scope,
            SendScope::All { until: None }
        );
        model.all_for_hour = true;
        assert_eq!(
            model.send_for(SendMode::All, now()).unwrap().scope,
            SendScope::All {
                until: Some(now() + ALL_FOR)
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
