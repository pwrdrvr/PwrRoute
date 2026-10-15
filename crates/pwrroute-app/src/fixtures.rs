//! Contrived rules for the window to draw until it reads `rules.json`.
//!
//! These are the example rules from the design review (`design/PwrRoute App
//! Baseline.dc.html`, option 1a). None of it comes from anyone's browsing:
//! the hosts, hit counts, profiles and bundle ids are made up for the mock.

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

pub fn rewrites() -> Vec<Rule> {
    use Matcher::{Label, Pattern};
    let rule = |matchers, destination, hits| Rule {
        matchers,
        destination,
        hits,
        enabled: true,
    };
    vec![
        rule(
            vec![Pattern("x.com/*"), Pattern("twitter.com/*")],
            rewrite("host", "xcancel.com"),
            "12 today",
        ),
        rule(
            vec![
                Pattern("*"),
                Label("with query"),
                Pattern("utm_*"),
                Pattern("fbclid"),
            ],
            rewrite("drop", "utm_* fbclid"),
            "1.1k today",
        ),
        rule(
            vec![Pattern("http://*.internal/*")],
            rewrite("scheme", "https"),
            "\u{2014}",
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
