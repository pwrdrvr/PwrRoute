//! Contrived data for the window to draw until it reads `rules.json` and
//! the router's recent dispatches.
//!
//! These are the examples from the design review (`design/PwrRoute App
//! Baseline.dc.html`): the rules of option 1a, the Finicky config of 1b and
//! the Activity rows of 2b. None of it comes from anyone's browsing: the
//! hosts, hit counts, times, profiles and bundle ids are made up for the mock.
//! `fixtures/rules.json` is the same rules in the engine's format, so Test a
//! link resolves against what the Rules screen shows.

use pwrroute_engine::RulesFile;

use crate::activity::{ActivityRow, ActivityStep, Opened, StepMark};
use crate::config_view::ConfigInCharge;

/// One piece of a rule's match column.
#[derive(Clone, Debug)]
pub enum Matcher {
    /// A wildcard URL pattern, drawn as a mono chip.
    Pattern(&'static str),
    /// A `/regex/` pattern, drawn in the info color.
    Regex(&'static str),
    /// Connective text: "with query", "sent from", "and".
    Label(&'static str),
    /// The app that handed the link to the OS.
    App {
        badge: &'static str,
        name: &'static str,
    },
    /// A JS function, by its source: only in a Finicky config.
    Function(&'static str),
    /// The rule is switched off.
    Off,
}

/// Where a rule sends a link.
#[derive(Clone, Debug)]
pub enum Destination {
    /// A rewrite: `kind` is host, drop or scheme.
    Rewrite {
        kind: &'static str,
        value: &'static str,
    },
    /// A browser, with its two-letter badge and an optional profile.
    Browser {
        badge: &'static str,
        name: &'static str,
        profile: Option<&'static str>,
    },
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

const fn browser(
    badge: &'static str,
    name: &'static str,
    profile: Option<&'static str>,
) -> Destination {
    Destination::Browser {
        badge,
        name,
        profile,
    }
}

const fn rewrite(kind: &'static str, value: &'static str) -> Destination {
    Destination::Rewrite { kind, value }
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
            vec![Pattern("x.com/*"), Pattern("twitter.com/*")],
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
                Pattern("*"),
                Label("with query"),
                Pattern("utm_*"),
                Pattern("fbclid"),
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
            vec![Pattern("http://*.internal/*")],
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
            vec![Pattern("meet.google.com/*"), Pattern("*.google.com/*")],
            chrome_work(),
            "38 today",
            vec!["meet.google.com/*", "*.google.com/*"],
        ),
        handler(
            "Work tools",
            vec![
                Pattern("*.atlassian.net/*"),
                Pattern("github.com/pwrdrvr/*"),
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
                Pattern("*"),
            ],
            browser("Ff", "Firefox", Some("default-release")),
            "57 today",
            vec!["*"],
        ),
        handler(
            "Social",
            vec![
                Pattern("bsky.app/*"),
                Pattern("*.reddit.com/*"),
                Regex("/^https:\\/\\/news\\.ycombinator/"),
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
                Pattern("localhost:*"),
                Pattern("127.0.0.1:*"),
                Pattern("*.localhost/*"),
            ],
            browser("Ar", "Arc", None),
            "402 today",
            vec!["localhost:*", "127.0.0.1:*", "*.localhost/*"],
        ),
        handler(
            "Zoom links",
            vec![Pattern("zoom.us/j/*"), Pattern("*.zoom.us/j/*")],
            browser("Zm", "zoom.us", Some("us.zoom.xos")),
            "6 today",
            vec!["zoom.us/j/*", "*.zoom.us/j/*"],
        ),
        handler(
            "Figma",
            vec![Pattern("figma.com/*"), Off],
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

/// The apps Test a link offers as the opener, the first being none.
pub fn openers() -> Vec<Option<Opener>> {
    let app = |badge, name, bundle_id| {
        Some(Opener {
            badge,
            name,
            bundle_id,
        })
    };
    vec![
        None,
        app("Sl", "Slack", "com.tinyspeck.slackmacgap"),
        app("Ml", "Mail", "com.apple.mail"),
        app("Ca", "Calendar", "com.apple.iCal"),
        app("Gh", "Ghostty", "com.mitchellh.ghostty"),
    ]
}

/// The link Test a link starts with, from Slack: two rewrites apply and
/// handler 3 wins.
pub const TEST_URL: &str = "https://x.com/pwrdrvr/status/1842?utm_source=slack&s=20";

/// The index into [`openers`] Test a link starts with: Slack.
pub const TEST_OPENER: usize = 1;

/// The Finicky config of option 1b, `~/.finicky.ts`, whose source is
/// `fixtures/home/.finicky.ts`.
pub fn config_in_charge(path: String) -> ConfigInCharge {
    use Matcher::{Function, Pattern};
    let rule = |matchers, destination| Rule {
        matchers,
        destination,
        hits: "",
        enabled: true,
    };
    let firefox = || browser("Ff", "Firefox", None);
    ConfigInCharge {
        path,
        edited: "Edited externally 3 min ago; reloaded.",
        handlers: vec![
            rule(
                vec![Function(
                    "(url, { opener }) => url.host.includes(\"workplace\")",
                )],
                firefox(),
            ),
            rule(
                vec![Function(
                    "(url, { opener }) => opener?.name.includes(\"Slack\") || false",
                )],
                firefox(),
            ),
            rule(vec![Pattern("bsky.app/*")], firefox()),
            rule(
                vec![Pattern("google.com/*"), Pattern("*.google.com*")],
                browser("Ch", "Google Chrome", None),
            ),
        ],
        default_browser: browser("Ch", "Google Chrome", None),
    }
}

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
