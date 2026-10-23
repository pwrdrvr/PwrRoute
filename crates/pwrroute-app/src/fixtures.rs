//! Contrived data for tests and screenshots: the sample rules and the
//! Activity rows. The window draws the real `rules.json` and the router's
//! real records.
//!
//! These are the examples from the design review (`design/PwrRoute App
//! Baseline.dc.html`): the Activity rows of 2b and the apps Test a link
//! offers. (Option 1b's Finicky config is `fixtures/home/.finicky.ts`, which
//! the engine describes like any other.) None of it comes from anyone's
//! browsing: the hosts, hit counts, times, profiles and bundle ids are made
//! up for the mock. `fixtures/rules.json` holds the design's rules in the
//! engine's format: the Rules screen reads the real `rules.json`, and only
//! tests and the `screens` example open this one.

use std::time::{Duration, SystemTime};

use pwrroute_engine::RulesFile;
use pwrroute_router::{
    AppType, Browser, DispatchId, DispatchRecord, Link, Matched, OpenerInfo, Outcome, Privacy,
    Source, Step, Timings,
};

/// An app a test link can come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opener {
    pub badge: &'static str,
    pub name: &'static str,
    pub bundle_id: &'static str,
}

/// The design's rules in the engine's `rules.json` format, for tests and
/// screenshots. The window reads the real file.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "the screens example draws these rules")
)]
pub fn rules_file() -> RulesFile {
    serde_json::from_str(include_str!("../fixtures/rules.json"))
        .expect("fixtures/rules.json is a valid rules.json")
}

const fn app(badge: &'static str, name: &'static str, bundle_id: &'static str) -> Option<Opener> {
    Some(Opener {
        badge,
        name,
        bundle_id,
    })
}

/// The apps Test a link offers as examples of a sender, the first being
/// none. They keep this order: Activity's re-test picks a record's app by
/// its position here.
pub const OPENERS: &[Option<Opener>] = &[
    None,
    app("Sl", "Slack", "com.tinyspeck.slackmacgap"),
    app("Ml", "Mail", "com.apple.mail"),
    app("Ca", "Calendar", "com.apple.iCal"),
    app("Gh", "Ghostty", "com.mitchellh.ghostty"),
];

/// The design's test link, from Slack: two rewrites apply and handler 3
/// wins. The window's Test a link starts empty; tests and the `screens`
/// example type this one.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "the window harness and the screens example")
)]
pub const TEST_URL: &str = "https://x.com/pwrdrvr/status/1842?utm_source=slack&s=20";

/// The index into [`OPENERS`] of the design's sender: Slack.
#[allow(dead_code, reason = "the screens example")]
pub const TEST_OPENER: usize = 1;

/// `now` less `secs`, kept on today's local calendar day.
///
/// The records below are minutes old, so in the minutes after local
/// midnight the plain subtraction would put most of them on yesterday and
/// the "today" counts the tests assert would stop adding up (seen on CI at
/// 00:03 UTC). When less time than the longest offset has passed since
/// midnight, the offsets are compressed into that window, in order.
fn ago(now: SystemTime, secs: u64) -> SystemTime {
    use chrono::Timelike;
    /// Longer than any offset the fixtures use.
    const SPAN: u64 = 20 * 60;
    let since_midnight = u64::from(
        chrono::DateTime::<chrono::Local>::from(now)
            .time()
            .num_seconds_from_midnight(),
    );
    let secs = if since_midnight > SPAN {
        secs
    } else {
        secs * since_midnight / (SPAN + 1)
    };
    now - Duration::from_secs(secs)
}

/// Contrived dispatch records in the shape of option 2b's Activity rows,
/// newest first, for tests and screenshots: the second ran two rewrites and
/// handler 3, the fourth failed to launch. The window draws the router's
/// real records; nothing here reaches it.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "the window harness and the screens example")
)]
pub fn activity() -> Vec<DispatchRecord> {
    let now = SystemTime::now();
    let link = |url: &str, from: &str| {
        Link::new(url, Source::AppleEvent).with_opener(Some(OpenerInfo {
            name: from.into(),
            ..OpenerInfo::default()
        }))
    };
    let browser = |name: &str, profile: &str, url: &str| Browser {
        name: name.into(),
        app_type: AppType::AppName,
        url: url.into(),
        profile: profile.into(),
        args: Vec::new(),
        open_in_background: None,
        private: false,
        privacy: pwrroute_engine::Privacy::Full,
    };
    let timings = |total_ms: u64| Timings {
        queue_us: 40,
        resolve_us: 600,
        launch_us: total_ms * 1_000 - 700,
        total_us: total_ms * 1_000,
    };
    // Handler numbers are 1-based in the design; `index` is 0-based.
    let opened = |id: u64,
                  secs_ago: u64,
                  url: &str,
                  from: &str,
                  handler: Option<usize>,
                  browser: Browser,
                  total_ms: u64| DispatchRecord {
        id: DispatchId(id),
        received_at: ago(now, secs_ago),
        privacy: Privacy::Full,
        link: link(url, from),
        forwarded: false,
        final_url: Some(browser.url.clone()),
        browser: Some(browser.clone()),
        steps: vec![
            Step::Received { url: url.into() },
            Step::Matched {
                by: handler.map_or(Matched::Default { owner: None }, |index| Matched::Handler {
                    index,
                }),
            },
            Step::Browser { browser },
            Step::Launched { dry_run: false },
        ],
        outcome: Outcome::Opened,
        timings: timings(total_ms),
    };
    let chrome_work = |url: &str| browser("Google Chrome", "Work", url);

    let rewritten = {
        let after_host = "https://xcancel.com/pwrdrvr/status/1842?utm_source=slack&s=20";
        let final_url = "https://xcancel.com/pwrdrvr/status/1842?s=20";
        let firefox = browser("Firefox", "default-release", final_url);
        DispatchRecord {
            id: DispatchId(7),
            received_at: ago(now, 17),
            privacy: Privacy::Full,
            link: link(TEST_URL, "Slack"),
            forwarded: false,
            final_url: Some(final_url.into()),
            browser: Some(firefox.clone()),
            steps: vec![
                Step::Received {
                    url: TEST_URL.into(),
                },
                Step::Rewrite {
                    rule: Some(0),
                    url: after_host.into(),
                },
                Step::Rewrite {
                    rule: Some(1),
                    url: final_url.into(),
                },
                Step::Matched {
                    by: Matched::Handler { index: 2 },
                },
                Step::Browser { browser: firefox },
                Step::Launched { dry_run: false },
            ],
            outcome: Outcome::Opened,
            timings: timings(6),
        }
    };
    let failed = {
        let url = "https://zoom.us/j/8142201137?pwd=\u{2026}";
        let zoom = browser("zoom.us", "", url);
        DispatchRecord {
            id: DispatchId(5),
            received_at: ago(now, 9 * 60 + 28),
            privacy: Privacy::Full,
            link: link(url, "Calendar"),
            forwarded: false,
            final_url: Some(url.into()),
            browser: Some(zoom.clone()),
            steps: vec![
                Step::Received { url: url.into() },
                Step::Matched {
                    by: Matched::Handler { index: 5 },
                },
                Step::Browser { browser: zoom },
                Step::LaunchFailed {
                    message: "zoom.us not found".into(),
                },
            ],
            outcome: Outcome::Failed,
            timings: timings(41),
        }
    };
    vec![
        opened(
            8,
            0,
            "https://example.atlassian.net/browse/PG-12",
            "Slack",
            Some(1),
            chrome_work("https://example.atlassian.net/browse/PG-12"),
            4,
        ),
        rewritten,
        opened(
            6,
            2 * 60 + 56,
            "http://localhost:5173/library",
            "Ghostty",
            Some(4),
            browser("Arc", "", "http://localhost:5173/library"),
            3,
        ),
        failed,
        opened(
            4,
            11 * 60 + 6,
            "https://github.com/pwrdrvr/PwrGit/pull/361",
            "PwrAgnt",
            Some(1),
            chrome_work("https://github.com/pwrdrvr/PwrGit/pull/361"),
            5,
        ),
        opened(
            3,
            13 * 60 + 13,
            "https://news.ycombinator.com/item?id=41",
            "Mail",
            Some(3),
            browser("Firefox", "", "https://news.ycombinator.com/item?id=41"),
            4,
        ),
        opened(
            2,
            14 * 60 + 58,
            "https://www.apple.com/macos/",
            "Safari",
            None,
            browser("Google Chrome", "Personal", "https://www.apple.com/macos/"),
            4,
        ),
        opened(
            1,
            18 * 60 + 20,
            "https://meet.google.com/abc-defg-hij",
            "Calendar",
            Some(0),
            chrome_work("https://meet.google.com/abc-defg-hij"),
            7,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone, Timelike};

    /// The sample rules are the design's: three rewrites and seven handlers,
    /// one of them off.
    #[test]
    fn the_sample_rules_are_the_designs() {
        let file = rules_file();
        assert_eq!(file.rewrites.len(), 3);
        assert_eq!(file.rules.len(), 7);
        assert_eq!(file.rules.iter().filter(|r| !r.enabled).count(), 1);
        assert_eq!(file.default_browser, "Google Chrome");
    }

    #[test]
    fn fixture_times_stay_on_today_right_after_midnight() {
        let today = Local::now().date_naive();
        let at = |h, m, sec| {
            SystemTime::from(
                Local
                    .from_local_datetime(&today.and_hms_opt(h, m, sec).expect("a valid time"))
                    .single()
                    .expect("an unambiguous local time"),
            )
        };
        let just_after = at(0, 0, 5);
        let offsets = [0, 17, 2 * 60 + 56, 9 * 60 + 28, 18 * 60 + 20];
        let times: Vec<SystemTime> = offsets.iter().map(|&s| ago(just_after, s)).collect();
        for t in &times {
            assert_eq!(chrono::DateTime::<Local>::from(*t).date_naive(), today);
        }
        assert!(
            times.windows(2).all(|w| w[0] >= w[1]),
            "older offsets stay older"
        );
        // Well into the day the offsets are used as written.
        let noon = at(12, 0, 0);
        assert_eq!(ago(noon, 17), noon - Duration::from_secs(17));
        assert_eq!(chrono::DateTime::<Local>::from(ago(noon, 17)).hour(), 11);
    }
}
