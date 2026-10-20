//! Contrived data for the window to draw until it reads `rules.json` and
//! the router's recent dispatches.
//!
//! These are the examples from the design review (`design/PwrRoute App
//! Baseline.dc.html`): the rules of option 1a and the Activity rows of 2b.
//! (Option 1b's Finicky config is `fixtures/home/.finicky.ts`, which the
//! engine describes like any other.) None of it comes from anyone's browsing: the
//! hosts, hit counts, times, profiles and bundle ids are made up for the mock.
//! `fixtures/rules.json` is the same rules in the engine's format, so Test a
//! link resolves against what the Rules screen shows.

use gpui_kit::SharedString;
use pwrroute_engine::RulesFile;

use crate::activity::{ActivityRow, ActivityStep, Opened, StepMark};

/// One piece of a rule's match column.
#[derive(Clone, Debug)]
pub enum Matcher {
    /// A wildcard URL pattern, drawn as a mono chip.
    Pattern(SharedString),
    /// A `/regex/` pattern, drawn in the info color.
    Regex(SharedString),
    /// Connective text: "with query", "sent from", "and".
    Label(&'static str),
    /// The app that handed the link to the OS.
    App {
        badge: &'static str,
        name: &'static str,
    },
    /// A JS function, by its source: only in a Finicky config.
    Function(SharedString),
    /// The rule is switched off.
    Off,
}

/// Where a rule sends a link.
#[derive(Clone, Debug)]
pub enum Destination {
    /// A rewrite: `kind` is host, drop or scheme.
    Rewrite {
        kind: &'static str,
        value: SharedString,
    },
    /// A browser, with its two-letter badge and an optional profile.
    Browser {
        badge: SharedString,
        name: SharedString,
        profile: Option<SharedString>,
    },
    /// A JS function that picks the browser or URL per link, by its source:
    /// only in a Finicky config.
    Function(SharedString),
}

/// One row of the rewrites or handlers card.
#[derive(Clone, Debug)]
pub struct Rule {
    pub matchers: Vec<Matcher>,
    pub destination: Destination,
    pub hits: &'static str,
    pub enabled: bool,
}

/// The app a handler requires the link to come from.
#[derive(Clone, Debug)]
pub struct SourceApp {
    pub badge: &'static str,
    pub name: &'static str,
    pub bundle_id: &'static str,
}

/// A handler row plus what its editor shows.
#[derive(Clone, Debug)]
pub struct Handler {
    pub rule: Rule,
    pub title: &'static str,
    pub patterns: Vec<&'static str>,
    pub sent_from: Option<SourceApp>,
}

/// A rewrite row plus what its editor shows.
#[derive(Clone, Debug)]
pub struct Rewrite {
    pub rule: Rule,
    pub title: &'static str,
    pub patterns: Vec<&'static str>,
    /// Query keys the link must carry, for a rewrite that drops them.
    pub query: Vec<&'static str>,
    /// The editor's Try it link, and what the rewrite makes of it.
    pub sample: (&'static str, &'static str),
}

/// An app a test link can come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opener {
    pub badge: &'static str,
    pub name: &'static str,
    pub bundle_id: &'static str,
}

fn browser(badge: &'static str, name: &'static str, profile: Option<&'static str>) -> Destination {
    Destination::Browser {
        badge: badge.into(),
        name: name.into(),
        profile: profile.map(SharedString::from),
    }
}

fn rewrite(kind: &'static str, value: &'static str) -> Destination {
    Destination::Rewrite {
        kind,
        value: value.into(),
    }
}

pub fn rewrites() -> Vec<Rewrite> {
    use Matcher::{Label, Pattern};
    let rewrite = |title, matchers, destination, hits, patterns, query, sample| Rewrite {
        rule: Rule {
            matchers,
            destination,
            hits,
            enabled: true,
        },
        title,
        patterns,
        query,
        sample,
    };
    vec![
        rewrite(
            "X to xcancel",
            vec![Pattern("x.com/*".into()), Pattern("twitter.com/*".into())],
            self::rewrite("host", "xcancel.com"),
            "12 today",
            vec!["x.com/*", "twitter.com/*"],
            Vec::new(),
            (
                "https://x.com/pwrdrvr/status/1842",
                "https://xcancel.com/pwrdrvr/status/1842",
            ),
        ),
        rewrite(
            "Strip tracking",
            vec![
                Pattern("*".into()),
                Label("with query"),
                Pattern("utm_*".into()),
                Pattern("fbclid".into()),
            ],
            self::rewrite("drop", "utm_* fbclid"),
            "1.1k today",
            vec!["*"],
            vec!["utm_*", "fbclid"],
            (
                "https://example.com/post?utm_source=mail&id=7",
                "https://example.com/post?id=7",
            ),
        ),
        rewrite(
            "Internal over HTTPS",
            vec![Pattern("http://*.internal/*".into())],
            self::rewrite("scheme", "https"),
            "\u{2014}",
            vec!["http://*.internal/*"],
            Vec::new(),
            ("http://wiki.internal/start", "https://wiki.internal/start"),
        ),
    ]
}

pub fn handlers() -> Vec<Handler> {
    use Matcher::{App, Label, Off, Pattern, Regex};
    let handler = |title, matchers, destination, hits, patterns| Handler {
        rule: Rule {
            matchers,
            destination,
            hits,
            enabled: true,
        },
        title,
        patterns,
        sent_from: None,
    };
    let chrome_work = || browser("Ch", "Google Chrome", Some("Work"));
    let mut handlers = vec![
        handler(
            "Google, at work",
            vec![
                Pattern("meet.google.com/*".into()),
                Pattern("*.google.com/*".into()),
            ],
            chrome_work(),
            "38 today",
            vec!["meet.google.com/*", "*.google.com/*"],
        ),
        handler(
            "Work tools",
            vec![
                Pattern("*.atlassian.net/*".into()),
                Pattern("github.com/pwrdrvr/*".into()),
            ],
            chrome_work(),
            "214 today",
            vec!["*.atlassian.net/*", "github.com/pwrdrvr/*"],
        ),
        handler(
            "Sent from Slack",
            vec![
                Label("sent from"),
                App {
                    badge: "Sl",
                    name: "Slack",
                },
                Label("and"),
                Pattern("*".into()),
            ],
            browser("Ff", "Firefox", Some("default-release")),
            "57 today",
            vec!["*"],
        ),
        handler(
            "Social",
            vec![
                Pattern("bsky.app/*".into()),
                Pattern("*.reddit.com/*".into()),
                Regex("/^https:\\/\\/news\\.ycombinator/".into()),
            ],
            browser("Ff", "Firefox", None),
            "9 today",
            vec![
                "bsky.app/*",
                "*.reddit.com/*",
                "/^https:\\/\\/news\\.ycombinator/",
            ],
        ),
        handler(
            "Local dev",
            vec![
                Pattern("localhost:*".into()),
                Pattern("127.0.0.1:*".into()),
                Pattern("*.localhost/*".into()),
            ],
            browser("Ar", "Arc", None),
            "402 today",
            vec!["localhost:*", "127.0.0.1:*", "*.localhost/*"],
        ),
        handler(
            "Zoom links",
            vec![
                Pattern("zoom.us/j/*".into()),
                Pattern("*.zoom.us/j/*".into()),
            ],
            browser("Zm", "zoom.us", Some("us.zoom.xos")),
            "6 today",
            vec!["zoom.us/j/*", "*.zoom.us/j/*"],
        ),
        handler(
            "Figma",
            vec![Pattern("figma.com/*".into()), Off],
            browser("Fg", "Figma", Some("com.figma.Desktop")),
            "\u{2014}",
            vec!["figma.com/*"],
        ),
    ];
    handlers[2].sent_from = Some(SourceApp {
        badge: "Sl",
        name: "Slack",
        bundle_id: "com.tinyspeck.slackmacgap",
    });
    handlers[6].rule.enabled = false;
    handlers
}

/// The catch-all row under the handlers.
pub fn everything_else() -> Rule {
    Rule {
        matchers: Vec::new(),
        destination: browser("Ch", "Google Chrome", Some("Personal")),
        hits: "690 today",
        enabled: true,
    }
}

/// The design's rules in the engine's `rules.json` format.
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

/// The apps Test a link offers as the opener, the first being none.
pub const OPENERS: &[Option<Opener>] = &[
    None,
    app("Sl", "Slack", "com.tinyspeck.slackmacgap"),
    app("Ml", "Mail", "com.apple.mail"),
    app("Ca", "Calendar", "com.apple.iCal"),
    app("Gh", "Ghostty", "com.mitchellh.ghostty"),
];

/// The link Test a link starts with, from Slack: two rewrites apply and
/// handler 3 wins.
pub const TEST_URL: &str = "https://x.com/pwrdrvr/status/1842?utm_source=slack&s=20";

/// The index into [`OPENERS`] Test a link starts with: Slack.
pub const TEST_OPENER: usize = 1;

/// How many links the router handled today, for the rail and strip.
pub const ACTIVITY_TODAY: &str = "1,412 today \u{00b7} 2 failed";
pub const ACTIVITY_COUNT: &str = "1.4k";

/// The Activity rows of option 2b, newest first. The second is the design's
/// expanded row, and the fourth a failed launch.
pub fn activity() -> Vec<ActivityRow> {
    let row =
        |time: &str, url: &str, from: &str, rule: &str, opened: Opened, took: &str| ActivityRow {
            time: time.into(),
            url: url.into(),
            from: from.into(),
            rule: rule.into(),
            opened,
            took: took.into(),
            rewritten: false,
            default: rule == "default",
            steps: Vec::new(),
            final_url: url.into(),
        };
    let browser = |badge: &str, name: &str, profile: Option<&str>| Opened::Browser {
        badge: badge.into(),
        name: name.into(),
        profile: profile.map(Into::into),
    };
    let step = |mark, text: &str| ActivityStep {
        mark,
        text: text.into(),
    };
    let mut rows = vec![
        row(
            "09:41:08",
            "https://example.atlassian.net/browse/PG-12",
            "Slack",
            "handler 2",
            browser("Ch", "Google Chrome", Some("Work")),
            "4 ms",
        ),
        row(
            "09:40:51",
            TEST_URL,
            "Slack",
            "rw 1, 2 \u{00b7} h 3",
            browser("Ff", "Firefox", Some("default-release")),
            "6 ms",
        ),
        row(
            "09:38:12",
            "http://localhost:5173/library",
            "Ghostty",
            "handler 5",
            browser("Ar", "Arc", None),
            "3 ms",
        ),
        row(
            "09:31:40",
            "https://zoom.us/j/8142201137?pwd=\u{2026}",
            "Calendar",
            "handler 6",
            Opened::Failed("zoom.us not found".into()),
            "41 ms",
        ),
        row(
            "09:30:02",
            "https://github.com/pwrdrvr/PwrGit/pull/361",
            "PwrAgnt",
            "handler 2",
            browser("Ch", "Google Chrome", Some("Work")),
            "5 ms",
        ),
        row(
            "09:27:55",
            "https://news.ycombinator.com/item?id=41",
            "Mail",
            "handler 4",
            browser("Ff", "Firefox", None),
            "4 ms",
        ),
        row(
            "09:26:10",
            "https://www.apple.com/macos/",
            "Safari",
            "default",
            browser("Ch", "Google Chrome", Some("Personal")),
            "4 ms",
        ),
        row(
            "09:22:48",
            "https://meet.google.com/abc-defg-hij",
            "Calendar",
            "handler 1",
            browser("Ch", "Google Chrome", Some("Work")),
            "7 ms",
        ),
    ];
    let expanded = &mut rows[1];
    expanded.rewritten = true;
    expanded.final_url = "https://xcancel.com/pwrdrvr/status/1842?s=20".into();
    expanded.steps = vec![
        step(
            StepMark::Hit("1"),
            "rewrite 1 \u{00b7} host \u{2192} xcancel.com",
        ),
        step(StepMark::Hit("2"), "rewrite 2 \u{00b7} dropped utm_source"),
        step(
            StepMark::Miss("3"),
            "rewrite 3, handlers 1\u{2013}2 \u{00b7} no match",
        ),
        step(StepMark::Hit("5"), "handler 3 \u{00b7} sent from Slack"),
        step(
            StepMark::Done,
            "Firefox \u{00b7} default-release \u{00b7} resolved 0.6 ms \u{00b7} launched 5.1 ms",
        ),
    ];
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Rules screen draws `rewrites()` and `handlers()`; Test a link
    /// resolves `fixtures/rules.json`. They must be the same rules.
    #[test]
    fn rules_json_is_the_rules_screen() {
        let file = rules_file();

        let rewrites = rewrites();
        assert_eq!(file.rewrites.len(), rewrites.len());
        for (json, row) in file.rewrites.iter().zip(&rewrites) {
            assert_eq!(json.patterns, row.patterns, "{}", row.title);
            assert_eq!(json.enabled, row.rule.enabled, "{}", row.title);
        }

        let handlers = handlers();
        assert_eq!(file.rules.len(), handlers.len());
        for (json, row) in file.rules.iter().zip(&handlers) {
            // rules.json has no regex matchers: a `/regex/` row stands in
            // as a wildcard there, so only the wildcards must agree.
            let wildcards: Vec<&str> = row
                .patterns
                .iter()
                .copied()
                .filter(|p| !p.starts_with('/'))
                .collect();
            assert_eq!(
                json.patterns
                    .iter()
                    .take(wildcards.len())
                    .collect::<Vec<_>>(),
                wildcards,
                "{}",
                row.title
            );
            assert_eq!(json.enabled, row.rule.enabled, "{}", row.title);
            let Destination::Browser { name, profile, .. } = &row.rule.destination else {
                panic!("{} opens a browser", row.title);
            };
            // An app row shows its bundle id where a browser shows the
            // profile; rules.json names that app by the bundle id.
            if json.browser == *name {
                assert_eq!(
                    json.profile,
                    profile.as_deref().unwrap_or_default(),
                    "{}",
                    row.title
                );
            } else {
                assert_eq!(
                    Some(json.browser.as_str()),
                    profile.as_deref(),
                    "{}",
                    row.title
                );
            }
            let opener: Vec<&str> = row.sent_from.iter().map(|app| app.bundle_id).collect();
            assert_eq!(json.opener, opener, "{}", row.title);
        }

        let Destination::Browser { name, profile, .. } = everything_else().destination else {
            panic!("everything else opens a browser");
        };
        assert_eq!(file.default_browser, name);
        assert_eq!(file.default_profile, profile.unwrap_or_default());
    }
}
