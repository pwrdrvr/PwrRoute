//! What the menu bar dropdown and the tray menu show, as plain data.
//!
//! The router fills this in (recent links, pause state, whether PwrRoute is
//! the default browser); until it does, [`TrayModel::fixture`] draws the
//! design's example. Everything here is testable without a window.
//!
//! Recent links keep a host, never a full URL: the dropdown is visible to
//! whoever is looking at the screen.

use std::time::{Duration, SystemTime};

/// How many recent links the dropdown lists.
pub const RECENT_LIMIT: usize = 3;

/// Whether links follow the rules.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Routing {
    #[default]
    Active,
    /// Every link goes to [`TrayModel::pause_target`]; rewrites still run.
    Paused,
}

/// A browser and profile, as the dropdown names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserLabel {
    /// The two-letter `.bi` badge.
    pub badge: String,
    pub name: String,
    pub profile: Option<String>,
}

impl BrowserLabel {
    pub fn new(badge: &str, name: &str, profile: Option<&str>) -> Self {
        Self {
            badge: badge.to_owned(),
            name: name.to_owned(),
            profile: profile.map(str::to_owned),
        }
    }

    /// "Google Chrome · Personal", or the name alone.
    pub fn display(&self) -> String {
        match &self.profile {
            Some(profile) => format!("{} \u{00b7} {profile}", self.name),
            None => self.name.clone(),
        }
    }
}

/// Which rule sent a recent link where it went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecentRoute {
    /// A handler matched; its 1-based number in the rules list.
    Handler(usize),
    /// No handler matched: the fallback browser.
    #[allow(dead_code, reason = "the router reports it; no fixture shows one")]
    Default,
    /// Routing was paused. `skipped` is the handler that would have matched.
    Paused { skipped: Option<usize> },
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

    /// The second line: "Firefox · rewritten · handler 3".
    pub fn detail(&self) -> String {
        let mut parts = vec![self.browser.name.clone()];
        if let Some(profile) = &self.browser.profile {
            parts.push(profile.clone());
        }
        if self.rewritten {
            parts.push("rewritten".to_owned());
        }
        match self.route {
            RecentRoute::Handler(n) => parts.push(format!("handler {n}")),
            RecentRoute::Default => parts.push("default".to_owned()),
            RecentRoute::Paused { skipped } => {
                parts.push("paused".to_owned());
                if let Some(n) = skipped {
                    parts.push(format!("skipped handler {n}"));
                }
            }
        }
        parts.join(" \u{00b7} ")
    }

    /// "now", "4m", "2h", "3d".
    pub fn age(&self, now: SystemTime) -> String {
        age(now.duration_since(self.at).unwrap_or_default())
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

fn age(elapsed: Duration) -> String {
    let minutes = elapsed.as_secs() / 60;
    match minutes {
        0 => "now".to_owned(),
        1..60 => format!("{minutes}m"),
        60..1440 => format!("{}h", minutes / 60),
        _ => format!("{}d", minutes / 1440),
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
}

impl HeaderChip {
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Paused => "paused",
            Self::NotDefault => "not default",
        }
    }
}

/// Everything the tray surfaces draw.
#[derive(Clone, Debug)]
pub struct TrayModel {
    pub routing: Routing,
    /// PwrRoute is the OS default browser.
    pub is_default_browser: bool,
    /// Where links go while paused: the fallback browser.
    pub pause_target: BrowserLabel,
    /// Newest first. The dropdown shows the first [`RECENT_LIMIT`].
    pub recents: Vec<Recent>,
    pub links_today: u64,
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
        Self {
            routing: Routing::Active,
            is_default_browser: true,
            pause_target: chrome(Some("Personal")),
            recents: recents.into_iter().flatten().collect(),
            links_today: 1412,
        }
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
                RecentRoute::Paused { skipped: Some(4) },
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

    pub fn chip(&self) -> HeaderChip {
        if self.is_paused() {
            HeaderChip::Paused
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

fn thousands(n: u64) -> String {
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
            "Firefox \u{00b7} rewritten \u{00b7} handler 3"
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
                "Google Chrome \u{00b7} Work \u{00b7} handler 2",
                "Firefox \u{00b7} rewritten \u{00b7} handler 3",
                "Arc \u{00b7} handler 5",
            ]
        );
        let paused = TrayModel::paused_fixture(now());
        assert_eq!(
            paused.recents[0].detail(),
            "Google Chrome \u{00b7} paused \u{00b7} skipped handler 4"
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
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1412), "1,412");
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(TrayModel::fixture(now()).today_label(), "1,412 today");
    }
}
