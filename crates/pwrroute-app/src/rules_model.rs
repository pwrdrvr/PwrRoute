//! What the Rules screen and its editor draw for a rule: rows derived from
//! the engine's `rules.json` types, and the handful of names the editor
//! gives each rewrite action.
//!
//! The store (`rules_store`) owns the file; these are views of it, rebuilt
//! after every change. A Finicky config's read-only rows (`config_view`) use
//! the same [`Rule`].

use gpui_kit::SharedString;
use pwrroute_engine::{self as engine, Privacy, PrivacySetting, RewriteAction, RewriteExample};

use crate::widgets::{AppBadge, DONT_OPEN, badge_for};

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
    App { badge: AppBadge, name: SharedString },
    /// A JS function, by its source: only in a Finicky config.
    Function(SharedString),
    /// The rule is switched off.
    Off,
    /// The rule's set is off on this machine (`machine.json`): it keeps
    /// its place and number, and routes elsewhere.
    OffHere,
    /// What the rule keeps of the links it takes, when less than
    /// everything: "private · host only", "nothing".
    Privacy(SharedString),
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
        badge: AppBadge,
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
    /// The named destination the rule opens (`rules[].destination`), drawn
    /// before what it opens on this machine (design: Synced Rules 2b).
    pub named: Option<SharedString>,
    pub destination: Destination,
    pub hits: SharedString,
    pub enabled: bool,
    pub badge: RowBadge,
}

impl Rule {
    /// The same information as the row, in reading order for assistive tools.
    pub fn accessible_name(&self) -> String {
        let patterns = self
            .matchers
            .iter()
            .map(|matcher| match matcher {
                Matcher::Pattern(value) | Matcher::Regex(value) | Matcher::Privacy(value) => {
                    value.to_string()
                }
                Matcher::Label(value) => (*value).to_owned(),
                Matcher::App { name, .. } => name.to_string(),
                Matcher::Function(_) => "JavaScript matcher".to_owned(),
                Matcher::Off => "Off".to_owned(),
                Matcher::OffHere => "Off on this machine".to_owned(),
            })
            .collect::<Vec<_>>()
            .join(" ");
        let destination = match &self.destination {
            Destination::Browser { name, profile, .. } => match profile {
                Some(profile) => format!("{name}, {profile}"),
                None => name.to_string(),
            },
            Destination::Rewrite(changes) => changes
                .iter()
                .map(|change| format!("{} {}", change.kind, change.value))
                .collect::<Vec<_>>()
                .join(", "),
            Destination::Function(_) => "JavaScript destination".to_owned(),
            Destination::Unset(value) => (*value).to_owned(),
        };
        // A named destination reads as the row draws it: the name, then
        // what it opens here.
        let destination = match &self.named {
            Some(name) => format!("{name}, {destination}"),
            None => destination,
        };
        format!(
            "{patterns}, {destination}, {}",
            if self.enabled { "enabled" } else { "disabled" }
        )
    }

    /// A read-only row: a config's, or the catch-all.
    pub fn plain(matchers: Vec<Matcher>, destination: Destination) -> Self {
        Self {
            matchers,
            named: None,
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
    /// `rules[].private`: open the link in a private window.
    pub private: bool,
    /// `rules[].privacy` as written; `None` is the default for `private`.
    pub privacy: Option<PrivacySetting>,
    /// `rules[].examples`: URLs that must resolve to this handler.
    pub examples: Vec<String>,
    /// `rules[].destination`: the named destination it opens in.
    pub destination: Option<String>,
    /// `rules[].set`: the rule set it belongs to.
    pub set: Option<String>,
    /// Why the editor cannot save it, in `RulesFile::problems`'s words.
    pub problems: Vec<String>,
}

impl Handler {
    /// The level the handler keeps links at, as the Keep control shows it:
    /// `None` for a word the engine does not know.
    pub fn keep(&self) -> Option<Privacy> {
        match &self.privacy {
            Some(PrivacySetting::Level(level)) => Some(*level),
            Some(PrivacySetting::Unknown(_)) => None,
            None if self.private => Some(Privacy::Host),
            None => Some(Privacy::Full),
        }
    }
}

/// A rewrite row plus what its editor shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rewrite {
    pub rule: Rule,
    pub title: SharedString,
    pub patterns: Vec<String>,
    pub actions: Vec<RewriteAction>,
    /// `rewrites[].examples`: URLs this rewrite must match, and what each
    /// becomes.
    pub examples: Vec<RewriteExample>,
    pub problems: Vec<String>,
}

/// The hits column with no links today, or a count that cannot be trusted.
const NO_HITS: &str = "\u{2014}";

/// The row and editor model of a handler at `badge` in the file.
pub fn handler(rule: &engine::Rule, badge: RowBadge, problems: Vec<String>) -> Handler {
    let mut matchers = Vec::new();
    for (i, app) in rule.opener.iter().enumerate() {
        // Named as the Overview names it: "Slack", not its bundle id.
        let name = crate::default_browser::browser_name(app);
        let name = name.split(':').next().unwrap_or(&name).to_owned();
        matchers.push(Matcher::Label(if i == 0 { "sent from" } else { "or" }));
        matchers.push(Matcher::App {
            badge: badge_for(&name),
            name: name.into(),
        });
    }
    if !rule.opener.is_empty() && !rule.patterns.is_empty() {
        matchers.push(Matcher::Label("and"));
    }
    matchers.extend(rule.patterns.iter().map(|p| pattern(p)));
    if let Some(label) = privacy_chip(rule.private, rule.effective_privacy()) {
        matchers.push(Matcher::Privacy(label.into()));
    }
    if !rule.enabled {
        matchers.push(Matcher::Off);
    }
    let destination = if rule.browser.trim().is_empty() {
        Destination::Unset("Choose a browser")
    } else {
        let name = crate::default_browser::browser_name(&rule.browser);
        // Don't open has no profile to show, whatever the file kept.
        let dont_open = engine::is_dont_open(&rule.browser);
        Destination::Browser {
            badge: badge_for(&name),
            name: name.into(),
            profile: (!rule.profile.is_empty() && !dont_open).then(|| rule.profile.clone().into()),
        }
    };
    Handler {
        rule: Rule {
            matchers,
            named: rule.destination.clone().map(Into::into),
            destination,
            hits: hits(badge),
            enabled: rule.enabled,
            badge,
        },
        title: title(&rule.patterns, "New rule"),
        patterns: rule.patterns.clone(),
        openers: rule.opener.clone(),
        browser: rule.browser.clone(),
        profile: rule.profile.clone(),
        private: rule.private,
        privacy: rule.privacy.clone(),
        examples: rule.examples.clone(),
        destination: rule.destination.clone(),
        set: rule.set.clone(),
        problems,
    }
}

/// The row chip for a rule that keeps less than everything, or opens a
/// private window: "private · host only", "private", "nothing". `None`
/// for a plain rule.
pub fn privacy_chip(private: bool, level: Privacy) -> Option<String> {
    let keep = match level {
        Privacy::Full => None,
        Privacy::Host => Some("host only"),
        Privacy::None => Some("nothing"),
    };
    match (private, keep) {
        (false, None) => None,
        (true, None) => Some("private".into()),
        (false, Some(keep)) => Some(keep.into()),
        (true, Some(keep)) => Some(format!("private \u{00b7} {keep}")),
    }
}

/// The Keep control's three choices, in order, with their labels.
pub const KEEP_LEVELS: [(Privacy, &str); 3] = [
    (Privacy::Full, "Full link"),
    (Privacy::Host, "Host only"),
    (Privacy::None, "Nothing"),
];

/// The row and editor model of a rewrite.
pub fn rewrite(rewrite: &engine::Rewrite, badge: RowBadge, problems: Vec<String>) -> Rewrite {
    let mut matchers: Vec<Matcher> = rewrite.patterns.iter().map(|p| pattern(p)).collect();
    if let Some(label) = privacy_chip(false, rewrite.effective_privacy()) {
        matchers.push(Matcher::Privacy(label.into()));
    }
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
            named: None,
            destination,
            hits: hits(badge),
            enabled: rewrite.enabled,
            badge,
        },
        title: title(&rewrite.patterns, "New rewrite"),
        patterns: rewrite.patterns.clone(),
        actions: rewrite.actions.clone(),
        examples: rewrite.examples.clone(),
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
            profile: (!file.default_profile.is_empty()
                && !engine::is_dont_open(&file.default_browser))
            .then(|| file.default_profile.clone().into()),
        },
    )
}

/// The default browser's badge and name as the screen says them.
pub fn default_browser(file: &engine::RulesFile) -> (AppBadge, SharedString) {
    if file.default_browser.trim().is_empty() {
        // The engine's fallback for a file that names none, by its name.
        let fallback = engine::FALLBACK_BROWSER;
        let name = fallback.rsplit('.').next().unwrap_or(fallback);
        (badge_for(fallback), name.to_owned().into())
    } else if engine::is_dont_open(&file.default_browser) {
        (badge_for(DONT_OPEN), DONT_OPEN.into())
    } else {
        (
            badge_for(&file.default_browser),
            file.default_browser.clone().into(),
        )
    }
}

fn hits(badge: RowBadge) -> SharedString {
    hits_label(badge, None)
}

/// The hits column: today's links for a saved rule ("1,204"), "—" with
/// none (or no count to trust), and "draft" for a draft.
pub fn hits_label(badge: RowBadge, today: Option<u64>) -> SharedString {
    match (badge, today) {
        (RowBadge::Draft, _) => "draft".into(),
        (RowBadge::None, Some(n)) if n > 0 => crate::activity::thousands(n).into(),
        (RowBadge::None | RowBadge::NotSaved, _) => NO_HITS.into(),
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
                    badge: badge_for("com.tinyspeck.slackmacgap"),
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
                badge: badge_for("Firefox"),
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
        assert_eq!(row.title, "New rule");
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
    fn a_private_handler_says_what_it_keeps() {
        let row = handler(
            &engine::Rule {
                patterns: vec!["bank.example/*".into()],
                browser: "Safari".into(),
                private: true,
                ..engine::Rule::default()
            },
            RowBadge::None,
            Vec::new(),
        );
        assert_eq!(
            row.rule.matchers,
            [
                Matcher::Pattern("bank.example/*".into()),
                Matcher::Privacy("private \u{00b7} host only".into()),
            ]
        );
        assert!(row.private);
        assert_eq!(row.keep(), Some(Privacy::Host));
        assert_eq!(privacy_chip(false, Privacy::Full), None);
        assert_eq!(
            privacy_chip(true, Privacy::Full).as_deref(),
            Some("private")
        );
        assert_eq!(
            privacy_chip(false, Privacy::None).as_deref(),
            Some("nothing")
        );
        let unknown = handler(
            &engine::Rule {
                patterns: vec!["a/*".into()],
                browser: "Safari".into(),
                privacy: Some(PrivacySetting::Unknown("hidden".into())),
                ..engine::Rule::default()
            },
            RowBadge::None,
            Vec::new(),
        );
        assert_eq!(unknown.keep(), None);
        assert_eq!(
            unknown.rule.matchers[1],
            Matcher::Privacy("nothing".into()),
            "an unknown word keeps nothing, as the engine treats it"
        );
    }

    #[test]
    fn the_catch_all_names_the_fallback_when_the_file_does_not() {
        let empty = engine::RulesFile::default();
        assert_eq!(
            everything_else(&empty).destination,
            Destination::Browser {
                badge: badge_for(engine::FALLBACK_BROWSER),
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
                badge: badge_for("Google Chrome"),
                name: "Google Chrome".into(),
                profile: Some("Personal".into())
            }
        );
    }
}
