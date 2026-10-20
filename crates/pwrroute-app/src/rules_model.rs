//! What the Rules screen and its editor draw for a rule: rows derived from
//! the engine's `rules.json` types, and the handful of names the editor
//! gives each rewrite action.
//!
//! The store (`rules_store`) owns the file; these are views of it, rebuilt
//! after every change. A Finicky config's read-only rows (`config_view`) use
//! the same [`Rule`].

use gpui_kit::SharedString;
use pwrroute_engine::{self as engine, RewriteAction};

use crate::widgets::badge_for;

/// One piece of a rule's match column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Matcher {
    /// A wildcard URL pattern, drawn as a mono chip.
    Pattern(SharedString),
    /// A `/regex/` pattern, drawn in the info color.
    Regex(SharedString),
    /// Connective text: "with query", "sent from", "and".
    Label(&'static str),
    /// The app that handed the link to the OS.
    App {
        badge: SharedString,
        name: SharedString,
    },
    /// A JS function, by its source: only in a Finicky config.
    Function(SharedString),
    /// The rule is switched off.
    Off,
}

/// One change a rewrite row shows: `kind` dimmed before the value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub kind: &'static str,
    pub value: SharedString,
}

/// Where a rule sends a link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    /// A rewrite's changes, in order.
    Rewrite(Vec<Change>),
    /// A browser, with its two-letter badge and an optional profile.
    Browser {
        badge: SharedString,
        name: SharedString,
        profile: Option<SharedString>,
    },
    /// A JS function that picks the browser or URL per link, by its source:
    /// only in a Finicky config.
    Function(SharedString),
    /// A draft has none yet: `.dest .ph`.
    Unset(&'static str),
}

/// What a row says in its hits column besides a count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowBadge {
    #[default]
    None,
    /// A new rule the file does not hold yet.
    Draft,
    /// An edit with problems: the file keeps the saved version.
    NotSaved,
}

/// One row of the rewrites or handlers card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub matchers: Vec<Matcher>,
    pub destination: Destination,
    pub hits: SharedString,
    pub enabled: bool,
    pub badge: RowBadge,
}

impl Rule {
    /// A read-only row: a config's, or the catch-all.
    pub fn plain(matchers: Vec<Matcher>, destination: Destination) -> Self {
        Self {
            matchers,
            destination,
            hits: "".into(),
            enabled: true,
            badge: RowBadge::None,
        }
    }
}

/// A handler row plus what its editor shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handler {
    pub rule: Rule,
    pub title: SharedString,
    pub patterns: Vec<String>,
    /// The apps the link must come from, by name or bundle id.
    pub openers: Vec<String>,
    pub browser: String,
    pub profile: String,
    /// Why the editor cannot save it, in `RulesFile::problems`'s words.
    pub problems: Vec<String>,
}

/// A rewrite row plus what its editor shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rewrite {
    pub rule: Rule,
    pub title: SharedString,
    pub patterns: Vec<String>,
    pub actions: Vec<RewriteAction>,
    pub problems: Vec<String>,
}

/// The hits column until the router counts dispatches per rule.
const NO_HITS: &str = "\u{2014}";

/// The row and editor model of a handler at `badge` in the file.
pub fn handler(rule: &engine::Rule, badge: RowBadge, problems: Vec<String>) -> Handler {
    let mut matchers = Vec::new();
    for (i, app) in rule.opener.iter().enumerate() {
        matchers.push(Matcher::Label(if i == 0 { "sent from" } else { "or" }));
        matchers.push(Matcher::App {
            badge: badge_for(app),
            name: app.clone().into(),
        });
    }
    if !rule.opener.is_empty() && !rule.patterns.is_empty() {
        matchers.push(Matcher::Label("and"));
    }
    matchers.extend(rule.patterns.iter().map(|p| pattern(p)));
    if !rule.enabled {
        matchers.push(Matcher::Off);
    }
    let destination = if rule.browser.trim().is_empty() {
        Destination::Unset("Choose a browser")
    } else {
        Destination::Browser {
            badge: badge_for(&rule.browser),
            name: rule.browser.clone().into(),
            profile: (!rule.profile.is_empty()).then(|| rule.profile.clone().into()),
        }
    };
    Handler {
        rule: Rule {
            matchers,
            destination,
            hits: hits(badge),
            enabled: rule.enabled,
            badge,
        },
        title: title(&rule.patterns, "New handler"),
        patterns: rule.patterns.clone(),
        openers: rule.opener.clone(),
        browser: rule.browser.clone(),
        profile: rule.profile.clone(),
        problems,
    }
}

/// The row and editor model of a rewrite.
pub fn rewrite(rewrite: &engine::Rewrite, badge: RowBadge, problems: Vec<String>) -> Rewrite {
    let mut matchers: Vec<Matcher> = rewrite.patterns.iter().map(|p| pattern(p)).collect();
    if !rewrite.enabled {
        matchers.push(Matcher::Off);
    }
    let destination = if rewrite.actions.is_empty() {
        Destination::Unset("Choose a change")
    } else {
        Destination::Rewrite(rewrite.actions.iter().map(change).collect())
    };
    Rewrite {
        rule: Rule {
            matchers,
            destination,
            hits: hits(badge),
            enabled: rewrite.enabled,
            badge,
        },
        title: title(&rewrite.patterns, "New rewrite"),
        patterns: rewrite.patterns.clone(),
        actions: rewrite.actions.clone(),
        problems,
    }
}

/// The catch-all row: the file's default browser, or the engine's fallback
/// when the file names none.
pub fn everything_else(file: &engine::RulesFile) -> Rule {
    let (badge, name) = default_browser(file);
    Rule::plain(
        Vec::new(),
        Destination::Browser {
            badge,
            name,
            profile: (!file.default_profile.is_empty())
                .then(|| file.default_profile.clone().into()),
        },
    )
}

/// The default browser's badge and name as the screen says them.
pub fn default_browser(file: &engine::RulesFile) -> (SharedString, SharedString) {
    if file.default_browser.trim().is_empty() {
        // The engine's fallback for a file that names none, by its name.
        let fallback = engine::FALLBACK_BROWSER;
        let name = fallback.rsplit('.').next().unwrap_or(fallback);
        (badge_for(fallback), name.to_owned().into())
    } else {
        (
            badge_for(&file.default_browser),
            file.default_browser.clone().into(),
        )
    }
}

fn hits(badge: RowBadge) -> SharedString {
    match badge {
        RowBadge::Draft => "draft".into(),
        RowBadge::None | RowBadge::NotSaved => NO_HITS.into(),
    }
}

fn pattern(pattern: &str) -> Matcher {
    if pattern.len() > 1 && pattern.starts_with('/') && pattern.ends_with('/') {
        Matcher::Regex(pattern.to_owned().into())
    } else {
        Matcher::Pattern(pattern.to_owned().into())
    }
}

/// A rule's title in the rail: its first pattern, or what it is until it
/// has one. `rules.json` has no names.
fn title(patterns: &[String], fallback: &'static str) -> SharedString {
    match patterns.iter().find(|p| !p.trim().is_empty()) {
        Some(first) => first.clone().into(),
        None => fallback.into(),
    }
}

/// A rewrite action as its row chip reads: `host xcancel.com`.
pub fn change(action: &RewriteAction) -> Change {
    Change {
        kind: kind_of(action).chip,
        value: value_of(action).into(),
    }
}

/// The five kinds of change the editor offers, in its menu's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kind {
    /// The menu item and the kind box: "Set host".
    pub label: &'static str,
    /// The row chip's dimmed prefix: "host".
    pub chip: &'static str,
    /// The menu item's example: "x.com → xcancel.com".
    pub example: &'static str,
    /// The value field's placeholder.
    pub placeholder: &'static str,
    /// Under the value: what the change does.
    pub help: &'static str,
}

pub const KINDS: [Kind; 5] = [
    Kind {
        label: "Set host",
        chip: "host",
        example: "x.com \u{2192} xcancel.com",
        placeholder: "xcancel.com",
        help: "Replaces the host and keeps the path and query.",
    },
    Kind {
        label: "Set scheme",
        chip: "scheme",
        example: "http \u{2192} https",
        placeholder: "https",
        help: "Replaces the scheme only.",
    },
    Kind {
        label: "Replace path",
        chip: "path",
        example: "/feed",
        placeholder: "/feed",
        help: "Replaces the path and keeps the host and query.",
    },
    Kind {
        label: "Drop query parameters",
        chip: "drop",
        example: "utm_*, fbclid",
        placeholder: "utm_*, fbclid",
        help: "Removes matching query parameters. * matches any run of characters.",
    },
    Kind {
        label: "Replace the whole URL",
        chip: "url",
        example: "https://new.example/",
        placeholder: "https://new.example/",
        help: "Replaces the whole URL.",
    },
];

/// Which of [`KINDS`] an action is.
pub fn kind_index(action: &RewriteAction) -> usize {
    match action {
        RewriteAction::SetHost(_) => 0,
        RewriteAction::SetScheme(_) => 1,
        RewriteAction::ReplacePath(_) => 2,
        RewriteAction::DropQuery(_) => 3,
        RewriteAction::Url(_) => 4,
    }
}

pub fn kind_of(action: &RewriteAction) -> Kind {
    KINDS[kind_index(action)]
}

/// An action's value as one editable string; dropped keys are separated by
/// spaces.
pub fn value_of(action: &RewriteAction) -> String {
    match action {
        RewriteAction::SetHost(v)
        | RewriteAction::SetScheme(v)
        | RewriteAction::ReplacePath(v)
        | RewriteAction::Url(v) => v.clone(),
        RewriteAction::DropQuery(keys) => keys.join(" "),
    }
}

/// The action of kind `kind` with `value` typed into its field.
pub fn action(kind: usize, value: &str) -> RewriteAction {
    let value = value.trim().to_owned();
    match kind {
        0 => RewriteAction::SetHost(value),
        1 => RewriteAction::SetScheme(value),
        2 => RewriteAction::ReplacePath(value),
        3 => RewriteAction::DropQuery(
            value
                .split(|c: char| c.is_whitespace() || c == ',')
                .filter(|k| !k.is_empty())
                .map(str::to_owned)
                .collect(),
        ),
        _ => RewriteAction::Url(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handler_row_reads_its_openers_patterns_and_browser() {
        let row = handler(
            &engine::Rule {
                patterns: vec!["*".into()],
                browser: "Firefox".into(),
                profile: "default-release".into(),
                opener: vec!["com.tinyspeck.slackmacgap".into()],
                enabled: false,
                ..engine::Rule::default()
            },
            RowBadge::None,
            Vec::new(),
        );
        assert_eq!(
            row.rule.matchers,
            [
                Matcher::Label("sent from"),
                Matcher::App {
                    badge: "Sl".into(),
                    name: "com.tinyspeck.slackmacgap".into()
                },
                Matcher::Label("and"),
                Matcher::Pattern("*".into()),
                Matcher::Off,
            ]
        );
        assert_eq!(
            row.rule.destination,
            Destination::Browser {
                badge: "Ff".into(),
                name: "Firefox".into(),
                profile: Some("default-release".into()),
            }
        );
        assert_eq!(row.title, "*");
        assert!(!row.rule.enabled);
    }

    #[test]
    fn a_draft_has_placeholders() {
        let row = handler(&engine::Rule::default(), RowBadge::Draft, Vec::new());
        assert_eq!(row.title, "New handler");
        assert_eq!(row.rule.destination, Destination::Unset("Choose a browser"));
        assert_eq!(row.rule.hits, "draft");
        let row = rewrite(&engine::Rewrite::default(), RowBadge::Draft, Vec::new());
        assert_eq!(row.rule.destination, Destination::Unset("Choose a change"));
    }

    #[test]
    fn actions_round_trip_through_their_fields() {
        let actions = [
            RewriteAction::SetHost("xcancel.com".into()),
            RewriteAction::SetScheme("https".into()),
            RewriteAction::ReplacePath("/feed".into()),
            RewriteAction::DropQuery(vec!["utm_*".into(), "fbclid".into()]),
            RewriteAction::Url("https://new.example/".into()),
        ];
        for (i, original) in actions.iter().enumerate() {
            assert_eq!(kind_index(original), i);
            assert_eq!(&action(i, &value_of(original)), original);
        }
        assert_eq!(
            action(3, "utm_*, fbclid ref"),
            RewriteAction::DropQuery(vec!["utm_*".into(), "fbclid".into(), "ref".into()])
        );
        let chips = rewrite(
            &engine::Rewrite {
                patterns: vec!["x.com/*".into()],
                actions: actions[..2].to_vec(),
                enabled: true,
                ..engine::Rewrite::default()
            },
            RowBadge::None,
            Vec::new(),
        );
        assert_eq!(
            chips.rule.destination,
            Destination::Rewrite(vec![
                Change {
                    kind: "host",
                    value: "xcancel.com".into()
                },
                Change {
                    kind: "scheme",
                    value: "https".into()
                },
            ])
        );
    }

    #[test]
    fn the_catch_all_names_the_fallback_when_the_file_does_not() {
        let empty = engine::RulesFile::default();
        assert_eq!(
            everything_else(&empty).destination,
            Destination::Browser {
                badge: "Sf".into(),
                name: "Safari".into(),
                profile: None
            }
        );
        let file = engine::RulesFile {
            default_browser: "Google Chrome".into(),
            default_profile: "Personal".into(),
            ..engine::RulesFile::default()
        };
        assert_eq!(
            everything_else(&file).destination,
            Destination::Browser {
                badge: "Ch".into(),
                name: "Google Chrome".into(),
                profile: Some("Personal".into())
            }
        );
    }
}
