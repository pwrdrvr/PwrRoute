//! Activity's row actions (design Activity Privacy 2b, 2c): the row's "⋯"
//! menu with Add a rule for this link, Save as example for the
//! `rules.json` rule that took it, and Save as test for a link that went
//! to a default; and the Add a rule sheet, which the expanded row's "Add a rule…" opens
//! too. A row is named by its [`RowKey`]: the record comes from the feed,
//! or from the journal for a journaled row.
//!
//! The sheet takes one dispatch record, offers `suggest_rule`'s patterns
//! narrowest first, and for the chosen pattern lists the recent links in
//! memory it would also catch (`Engine::collisions`, through the window's
//! probe). Browser and profile come from the browsers detected at launch,
//! and the rule's privacy from the same two controls the editor has. Add
//! appends the rule to `rules.json` with the link as its first example and
//! runs the tests; a record kept as host only or nothing has no URL to
//! write a rule for, so the sheet says so and does nothing else.
//!
//! A short link the router expanded is suggested, saved and probed as
//! where it landed (`activity::matched_url`), the URL the rules saw: the
//! saved tests and the probe do not expand short links.
//!
//! Every URL here comes from the router's in-memory records and goes to
//! disk only as the example or test the user adds, by that click alone;
//! the sheet's footer and each note say so.

use std::time::SystemTime;

use chrono::{DateTime, Local};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, HighlightStyle, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, StyledText, Window, deferred, div, px,
};
use pwrroute_engine::{
    self as engine, Expectation, Opener, Precedence, Privacy, RouteTest, Source, Stage,
    suggest_rule,
};
use pwrroute_router::{
    Destination, DispatchRecord, Matched, OpenAs, OpenLaterError, OpenerInfo, Outcome, Owner, Step,
};

use crate::activity::{self, RowKey, matched_url};
use crate::app::{PwrRoute, Selection};
use crate::destinations::{self, Choice};
use crate::rules_model::KEEP_LEVELS;
use crate::rules_store::Status;
use crate::text_field::{Edit, TextField};
use crate::theme::{self, mono, text, tracked};
use crate::widgets::{
    AppBadge, ButtonKind, badge, badge_for, button, input_box, menu_card, menu_row, one_line,
    paragraph, segment, segmented, switch,
};

/// A browser the sheet offers, with the profiles it has on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserChoice {
    pub name: String,
    pub profiles: Vec<String>,
}

/// What the row actions are doing: the row whose "⋯" menu is open, the
/// sheet, and the note the last action left, for the row it was on (the
/// menu's footer shows it).
#[derive(Default)]
pub struct Actions {
    pub menu: Option<RowMenu>,
    /// The menu open when the mouse went down on a "⋯": the menu closes
    /// on that mouse down, so the click that follows reads this.
    pub menu_was: Option<RowKey>,
    pub sheet: Option<AddRule>,
    pub note: Option<(RowKey, String)>,
}

/// The "⋯" menu of one row, resolved when it opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowMenu {
    pub key: RowKey,
    /// The Save entry, when the record has somewhere it went.
    pub save: Option<Save>,
    /// The handler Add rule just added from this row, which the menu
    /// offers to show in Rules.
    pub added: Option<usize>,
}

/// What Save does for a record: joins the examples of the `rules.json`
/// handler that takes the link now, or pins where the link went as a test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Save {
    Example {
        /// The handler's position in the file.
        handler: usize,
        /// Its first pattern, for the menu's hint.
        pattern: Option<String>,
    },
    Test {
        /// "Chrome · Personal": the browser the test expects.
        expects: String,
    },
}

/// Where the new handler goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Position {
    Top,
    #[default]
    Bottom,
}

/// A recent link the chosen pattern would also catch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collision {
    pub time: String,
    pub url: String,
    /// Where it went: "Google Chrome · Personal".
    pub opened: String,
    /// Who routed it, from its record.
    pub took: Took,
}

/// Who routed a recent link, which decides whether the new rule would
/// take it from them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Took {
    /// A default (PwrRoute's or the config's) or the fallback browser:
    /// nothing changes hands.
    Unrouted,
    /// One of `rules.json`'s rules, by its place in the file. A new rule
    /// at the top runs before it, at the bottom after it.
    Rule(usize),
    /// The JS config's handler: it runs before every `rules.json` rule
    /// unless `rules.json` comes first (`"precedence": "pwrroute"`, or no
    /// config in charge).
    ConfigHandler(usize),
    /// A rule or a config handler, and the record does not say which
    /// (`Matched::Unreported`). It changes hands for sure only when the new
    /// rule runs before both: at the top with `rules.json` first.
    Unknown,
}

impl Took {
    fn of(record: &DispatchRecord) -> Self {
        match matched(record) {
            Some(Matched::Rule { index }) => Self::Rule(index),
            Some(Matched::Unreported) => Self::Unknown,
            Some(Matched::Handler { index }) => Self::ConfigHandler(index),
            Some(Matched::Default { .. }) | None => Self::Unrouted,
        }
    }
}

/// The Add a rule sheet's state, for one record.
pub struct AddRule {
    pub key: RowKey,
    pub time: String,
    /// The link the rules saw (`matched_url`): where a short link landed,
    /// else as received. Empty when `disabled`.
    pub url: String,
    pub from: Option<OpenerInfo>,
    /// The sending app as the engine saw it (`engine_opener`), which Add
    /// traces the link from.
    opener: Option<Opener>,
    /// Why no rule can be written for this record: it was kept as host
    /// only or nothing.
    pub disabled: Option<String>,
    /// `suggest_rule`'s patterns, narrowest first, each with its why.
    pub candidates: Vec<(String, &'static str)>,
    /// The reasons a precise rule needs a JS config, when it does.
    pub needs_js: Vec<&'static str>,
    /// Which candidate is selected, when the pattern is one of them.
    pub selected: Option<usize>,
    pub pattern: TextField,
    pub collisions: Vec<Collision>,
    pub collision_error: Option<String>,
    /// `rules.json`'s rules run before the config's handlers: no config is
    /// in charge, or `"precedence": "pwrroute"`. Decides which collisions
    /// change hands.
    pub rules_first: bool,
    /// The config in charge as the user knows it ("~/.finicky.ts"), for
    /// the collision caption.
    pub config: Option<String>,
    /// The rule also requires the link to come from `from`.
    pub only_from: bool,
    /// Match is "Any link from <app>": the pattern is [`ANY_LINK`] and
    /// `only_from` is on (design: Add Rule From an App 1c).
    pub any_from: bool,
    /// `only_from` as it was before "Any link from <app>" turned it on,
    /// put back when a pattern is chosen instead.
    only_from_before: bool,
    /// The link in the head is shown whole, not cut at two lines.
    pub url_expanded: bool,
    /// "9 query parameters", beside Show full link, when the link has a
    /// query: counted once, since the link never changes.
    pub query_summary: Option<String>,
    pub browser: String,
    /// The browser this link opened in, offered in the browser list (as
    /// "where it opened") when the scan does not list it.
    pub opened_in: Option<String>,
    pub profile: String,
    pub private: bool,
    pub keep: Privacy,
    pub position: Position,
    /// Why the rule would not take the link, found when Add was clicked.
    pub problem: Option<Problem>,
}

/// What stops the sheet's rule from taking its link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub kind: ProblemKind,
    /// What takes the link instead, or that the rule is already there.
    pub text: String,
    /// What to do about it, shown under `text` in the sheet.
    pub advice: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProblemKind {
    /// The file already has this rule: Add never adds it again.
    Duplicate,
    /// Another rule, or the config, takes the link first: a second click
    /// adds the rule anyway.
    Shadowed,
}

impl AddRule {
    /// The sheet for a record kept in full: the suggestion for its link,
    /// the first candidate selected, and no browser yet: Add waits for one
    /// (design: Rule Editor Pickers 5c), so a rule never opens in a browser
    /// nobody chose.
    pub fn new(record: &DispatchRecord, cx: &mut App) -> Self {
        let url = matched_url(record).to_owned();
        let query_summary = query_summary(&url);
        let suggestion = suggest_rule(&url);
        let candidates: Vec<(String, &'static str)> = suggestion
            .candidates
            .iter()
            .cloned()
            .zip(suggestion.why.iter().copied())
            .collect();
        let needs_js: Vec<&'static str> = if suggestion.needs_js {
            suggestion.why[candidates.len()..].to_vec()
        } else {
            Vec::new()
        };
        let first = candidates
            .first()
            .map(|(p, _)| p.clone())
            .unwrap_or_default();
        Self {
            key: RowKey {
                id: record.id,
                journaled: false,
            },
            time: clock(record.received_at),
            url,
            from: record.link.opener.clone().filter(|o| !o.name.is_empty()),
            opener: engine_opener(record),
            disabled: None,
            selected: (!candidates.is_empty()).then_some(0),
            candidates,
            needs_js,
            pattern: TextField::new(first, cx),
            collisions: Vec::new(),
            collision_error: None,
            rules_first: true,
            config: None,
            only_from: false,
            any_from: false,
            only_from_before: false,
            url_expanded: false,
            query_summary,
            browser: String::new(),
            opened_in: record
                .browser
                .as_ref()
                .map(|b| b.name.clone())
                .filter(|name| !name.trim().is_empty()),
            profile: String::new(),
            private: false,
            keep: Privacy::Full,
            position: Position::Bottom,
            problem: None,
        }
    }

    /// The sheet for a record that kept less than the URL: nothing to
    /// write a rule for, and the reason.
    pub fn disabled(record: &DispatchRecord, cx: &mut App) -> Self {
        let kept = match record.privacy {
            Privacy::Host => "host only",
            Privacy::None | Privacy::Full => "nothing",
        };
        let reason = format!(
            "The rule that routed this link keeps {kept}, so the router never kept its URL and \
             there is nothing to write a rule for. Change that rule's Keep setting in Rules, \
             then click the link again."
        );
        Self {
            disabled: Some(reason),
            ..Self::new(record, cx)
        }
    }

    /// The sheet's title: "Add a rule for this link".
    pub fn title(&self) -> &'static str {
        "Add a rule for this link"
    }

    /// The eyebrow: "From Activity · 09:40:51 · sent from Slack".
    pub fn eyebrow(&self) -> String {
        match &self.from {
            Some(app) => format!(
                "From Activity \u{00b7} {} \u{00b7} sent from {}",
                self.time, app.name
            ),
            None => format!("From Activity \u{00b7} {}", self.time),
        }
    }

    /// How the rule names the sending app: its bundle id, else its name,
    /// as `rules[].opener` and the router compare them.
    pub fn sender_spec(&self) -> Option<String> {
        self.from.as_ref().map(|app| {
            if app.bundle_id.is_empty() {
                app.name.clone()
            } else {
                app.bundle_id.clone()
            }
        })
    }

    /// The link's host and path, drawn brighter than the rest of it, as a
    /// byte length: everything before the query or fragment.
    pub fn url_lead(&self) -> usize {
        self.url.find(['?', '#']).unwrap_or(self.url.len())
    }

    /// Whether the link may run past the head's two lines, so the head
    /// offers Show full link.
    pub fn url_is_long(&self) -> bool {
        self.url.chars().count() > LINK_FITS
    }

    /// Whether the sheet still needs a JS handler for a precise rule: never
    /// for "Any link from <app>", which `rules.json` holds as it is.
    pub fn needs_js_now(&self) -> bool {
        !self.any_from && !self.needs_js.is_empty()
    }

    /// "Any link from <app>" (design: Add Rule From an App 1c): the
    /// pattern becomes [`ANY_LINK`] and Sent from turns on. Only for a
    /// record that names its app.
    pub fn choose_any_from(&mut self) {
        if self.from.is_none() || self.disabled.is_some() {
            return;
        }
        if !self.any_from {
            self.only_from_before = self.only_from;
        }
        self.any_from = true;
        self.only_from = true;
        self.selected = None;
        self.pattern.set(ANY_LINK);
        self.problem = None;
    }

    /// One of the suggested patterns. Leaving "Any link from <app>" puts
    /// Sent from back where it was.
    pub fn choose_candidate(&mut self, index: usize) {
        let Some((pattern, _)) = self.candidates.get(index) else {
            return;
        };
        self.pattern.set(pattern.clone());
        self.selected = Some(index);
        if self.any_from {
            self.any_from = false;
            self.only_from = self.only_from_before;
        }
        self.problem = None;
    }

    /// The Sent from switch. Off while Match is "Any link from <app>" goes
    /// back to the narrowest pattern, so the sheet never offers a rule that
    /// takes every link from every app.
    pub fn toggle_only_from(&mut self) {
        // Any link from the app always has Sent from on.
        if self.any_from {
            self.any_from = false;
            self.only_from = false;
            match self.candidates.first() {
                Some((pattern, _)) => {
                    self.pattern.set(pattern.clone());
                    self.selected = Some(0);
                }
                None => {
                    self.pattern.set(String::new());
                    self.selected = None;
                }
            }
        } else {
            self.only_from = !self.only_from;
        }
        self.problem = None;
    }

    /// A pill in Open in: browser, profile and private window at once. Keep
    /// follows the private window while it is what private implies.
    pub fn choose(&mut self, choice: Choice) {
        let implied = |private| {
            if private {
                Privacy::Host
            } else {
                Privacy::Full
            }
        };
        if self.keep == implied(self.private) {
            self.keep = implied(choice.private);
        }
        self.browser = choice.browser;
        self.profile = choice.profile;
        self.private = choice.private;
    }

    /// After a keystroke in the pattern field: which choice it is now.
    fn pattern_typed(&mut self) {
        let typed = self.pattern.text.trim().to_owned();
        self.selected = self.candidates.iter().position(|(p, _)| *p == typed);
        let any_from = typed == ANY_LINK && self.only_from && self.from.is_some();
        // Entering it by hand: Sent from was already on.
        if any_from && !self.any_from {
            self.only_from_before = true;
        }
        self.any_from = any_from;
        self.problem = None;
    }

    /// The rule the sheet would add, with the link as its example. A rule
    /// that requires the sending app gets no example, since examples run
    /// from no app: the link goes in as a test instead ([`AddRule::test`]).
    /// `None` while it has no pattern or browser.
    pub fn rule(&self) -> Option<engine::Rule> {
        let pattern = self.pattern.text.trim();
        if pattern.is_empty() || self.browser.trim().is_empty() || self.disabled.is_some() {
            return None;
        }
        let opener = match self.sender_spec() {
            Some(app) if self.only_from => vec![app],
            _ => Vec::new(),
        };
        // Written only when it differs from what `private` implies, so a
        // plain rule stays as the Go host would write it.
        let implied = if self.private {
            Privacy::Host
        } else {
            Privacy::Full
        };
        let examples = if opener.is_empty() {
            vec![self.url.clone()]
        } else {
            Vec::new()
        };
        Some(engine::Rule {
            patterns: vec![pattern.to_owned()],
            browser: self.browser.trim().to_owned(),
            profile: self.profile.trim().to_owned(),
            opener,
            enabled: true,
            private: self.private,
            privacy: (self.keep != implied).then(|| self.keep.into()),
            examples,
        })
    }

    /// The test that pins the link to the new rule when the rule requires
    /// the sending app: the link from that app must open where the rule
    /// says. `None` when the link is the rule's example instead.
    pub fn test(&self) -> Option<RouteTest> {
        let rule = self.rule()?;
        let from = self.from.as_ref().filter(|_| self.only_from)?;
        Some(RouteTest {
            url: self.url.clone(),
            opener: opener_spec(from),
            expect: Expectation {
                browser: rule.browser,
                profile: (!rule.profile.is_empty()).then_some(rule.profile),
                url: None,
            },
        })
    }

    /// The footer: where the rule lands and how the link is kept with it.
    pub fn footer(&self, handler: usize) -> String {
        if self.disabled.is_none()
            && self.browser.trim().is_empty()
            && !self.pattern.text.trim().is_empty()
        {
            return format!("Choose a browser to add it as rule {handler}.");
        }
        if self.test().is_some() {
            let app = self
                .from
                .as_ref()
                .map(|f| f.name.as_str())
                .unwrap_or_default();
            format!(
                "Becomes rule {handler}, with this link from {app} as a saved test. Saved to \
                 rules.json."
            )
        } else {
            format!("Becomes rule {handler}, with this link as its example. Saved to rules.json.")
        }
    }

    /// The caption over the collision list: how many recent links the
    /// pattern matches, and which of them would change hands. A link a
    /// default or the fallback took moves to the new rule; one a
    /// `rules.json` rule took moves only when the new rule goes on top;
    /// one a config handler took moves only when `rules.json` runs first.
    /// One whose record does not say who took it moves for sure only at the
    /// top with `rules.json` first, stays for sure at the bottom with the
    /// config first, and otherwise may move.
    pub fn collision_caption(&self) -> String {
        let n = self.collisions.len();
        let app = self
            .from
            .as_ref()
            .filter(|_| self.only_from)
            .map(|app| app.name.as_str());
        if n == 0 {
            return match app {
                Some(app) if self.any_from => format!("No other recent link came from {app}."),
                Some(app) => format!("No other recent link from {app} matches this pattern."),
                None => "No other recent link matches this pattern.".into(),
            };
        }
        let mut head = if n == 1 {
            "Would also catch 1 recent link".to_owned()
        } else {
            format!("Would also catch these {n} recent links")
        };
        if let Some(app) = app {
            head = format!("{head} from {app}");
        }
        let top = self.position == Position::Top;
        let (mut moved_rules, mut moved_config, mut moved_unknown) = (0, 0, 0);
        let (mut kept_rules, mut kept_config, mut kept_unknown, mut maybe) = (0, 0, 0, 0);
        for collision in &self.collisions {
            match collision.took {
                Took::Unrouted => {}
                Took::Rule(_) if top => moved_rules += 1,
                Took::Rule(_) => kept_rules += 1,
                Took::ConfigHandler(_) if self.rules_first => moved_config += 1,
                Took::ConfigHandler(_) => kept_config += 1,
                Took::Unknown if top && self.rules_first => moved_unknown += 1,
                Took::Unknown if !top && !self.rules_first => kept_unknown += 1,
                Took::Unknown => maybe += 1,
            }
        }
        let mut parts = Vec::new();
        let moved = moved_rules + moved_config + moved_unknown;
        if moved > 0 {
            let by = match (moved_rules > 0, moved_config > 0, moved_unknown > 0) {
                (true, false, false) => "a rule",
                (false, true, false) => "a config handler",
                _ => "a rule or config handler",
            };
            parts.push(format!(
                "{moved} routed by {by} now, which this rule would take first"
            ));
        }
        if kept_rules > 0 {
            parts.push(format!(
                "{kept_rules} {} with {}, which {} first",
                plural(kept_rules, "stays", "stay"),
                plural(kept_rules, "an earlier rule", "earlier rules"),
                plural(kept_rules, "runs", "run"),
            ));
        }
        let config = self.config.as_deref().unwrap_or("the config");
        if kept_config > 0 {
            parts.push(format!(
                "{kept_config} {} with {config}'s handlers, which run first",
                plural(kept_config, "stays", "stay"),
            ));
        }
        if kept_unknown > 0 {
            parts.push(format!(
                "{kept_unknown} {} with an earlier rule or {config}'s handlers, which run first",
                plural(kept_unknown, "stays", "stay"),
            ));
        }
        if maybe > 0 {
            parts.push(format!(
                "{maybe} may change hands: {} not say who routed {}",
                plural(maybe, "its record does", "their records do"),
                plural(maybe, "it", "them"),
            ));
        }
        if parts.is_empty() {
            let unrouted = plural(n, "unrouted", "all unrouted");
            format!("{head} \u{00b7} {unrouted}, nothing changes hands")
        } else {
            format!("{head} \u{00b7} {}", parts.join("; "))
        }
    }
}

/// The pattern of "Any link from <app>": Finicky's wildcard for every link.
pub const ANY_LINK: &str = "*";

/// A link longer than this may not fit the head's two lines, so the head
/// offers Show full link (design: Add Rule From an App 1b). The sheet's
/// 524px of 11px mono hold about 79 characters a line; a link that fits on
/// one line never runs past two however the wrapper breaks it, while a
/// longer one can (a long token moved whole to the next line).
const LINK_FITS: usize = 72;

/// "9 query parameters" for a link with a query.
fn query_summary(url: &str) -> Option<String> {
    let count = url::Url::parse(url).ok()?.query_pairs().count();
    (count > 0).then(|| format!("{count} query {}", plural(count, "parameter", "parameters")))
}

/// `one` for 1, else `many`.
fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 { one } else { many }
}

/// What chose the record's browser.
fn matched(record: &DispatchRecord) -> Option<Matched> {
    record.steps.iter().find_map(|step| match step {
        Step::Matched { by } => Some(*by),
        _ => None,
    })
}

/// "09:40:51", local time.
fn clock(at: SystemTime) -> String {
    DateTime::<Local>::from(at).format("%H:%M:%S").to_string()
}

/// "Google Chrome · Personal", or the browser alone.
pub(crate) fn opened_label(record: &DispatchRecord) -> String {
    match &record.browser {
        Some(browser) if browser.profile.is_empty() => browser.name.clone(),
        Some(browser) => format!("{} \u{00b7} {}", browser.name, browser.profile),
        None => "not opened".into(),
    }
}

/// The opener as the engine sees it, for a re-resolution.
pub(crate) fn engine_opener(record: &DispatchRecord) -> Option<Opener> {
    record
        .link
        .opener
        .as_ref()
        .filter(|o| !o.name.is_empty() || !o.bundle_id.is_empty())
        .map(Opener::from)
}

/// The opener as a saved test names it: `Name:bundle.id`, or the one that
/// is known.
fn opener_spec(opener: &OpenerInfo) -> Option<String> {
    match (opener.name.is_empty(), opener.bundle_id.is_empty()) {
        (true, true) => None,
        (false, true) => Some(opener.name.clone()),
        (true, false) => Some(opener.bundle_id.clone()),
        (false, false) => Some(format!("{}:{}", opener.name, opener.bundle_id)),
    }
}

/// The record behind a row, from the feed every surface shares: memory,
/// or the journal for a journaled row.
fn record(key: RowKey, cx: &App) -> Option<DispatchRecord> {
    activity::feed(cx)?.read(cx).record(key)
}

/// Whether `record` came from `app` (a name or bundle id), compared as the
/// router's `rules[].opener` compares them: either, ignoring case.
fn sent_by(record: &DispatchRecord, app: &str) -> bool {
    record.link.opener.as_ref().is_some_and(|opener| {
        opener.name.eq_ignore_ascii_case(app) || opener.bundle_id.eq_ignore_ascii_case(app)
    })
}

/// The same rule, whatever examples each keeps.
fn same_rule(a: &engine::Rule, b: &engine::Rule) -> bool {
    engine::Rule {
        examples: Vec::new(),
        ..a.clone()
    } == engine::Rule {
        examples: Vec::new(),
        ..b.clone()
    }
}

/// A test that pins where this record went, from the URL the rules saw:
/// a saved test does not expand short links.
fn test_for(record: &DispatchRecord) -> Option<RouteTest> {
    let browser = record.browser.as_ref()?;
    if browser.name.trim().is_empty() {
        return None;
    }
    Some(RouteTest {
        url: matched_url(record).to_owned(),
        opener: record.link.opener.as_ref().and_then(opener_spec),
        expect: Expectation {
            browser: browser.name.clone(),
            profile: (!browser.profile.is_empty()).then(|| browser.profile.clone()),
            url: record.final_url.clone(),
        },
    })
}

impl PwrRoute {
    /// `rules.json`'s rules run before the config's handlers: no config
    /// is in charge, or the file says `"precedence": "pwrroute"`.
    pub(crate) fn rules_first(&self) -> bool {
        self.config.is_none() || self.store.precedence() == Precedence::PwrRoute
    }

    /// Opens or closes the row's "⋯" menu. The Save entry is resolved
    /// here: a link a `rules.json` rule took joins that rule's examples
    /// ([`PwrRoute::example_rule`]); anything else is pinned as a test.
    pub fn toggle_row_menu(&mut self, key: RowKey, cx: &mut Context<'_, Self>) {
        if self.actions.menu_was == Some(key) {
            self.actions.menu = None;
            cx.notify();
            return;
        }
        let Some(record) = record(key, cx) else {
            return;
        };
        // The record's rule number holds only while the file is as it was
        // when the link was routed: notice a change on disk first.
        self.sync_with_disk(cx);
        let save = match matched(&record) {
            _ if record.privacy != Privacy::Full || record.browser.is_none() => None,
            // A rules.json rule took it: the link becomes its example.
            Some(Matched::Rule { .. } | Matched::Unreported) => {
                Some(match self.example_rule(&record) {
                    Some(handler) => Save::Example {
                        handler,
                        pattern: self
                            .handlers
                            .get(handler)
                            .and_then(|h| h.patterns.first().cloned()),
                    },
                    None => Save::Test {
                        expects: opened_label(&record),
                    },
                })
            }
            // A default, or a config handler: pin the whole resolution.
            Some(Matched::Default { .. } | Matched::Handler { .. }) => Some(Save::Test {
                expects: opened_label(&record),
            }),
            None => None,
        };
        self.actions.menu = Some(RowMenu {
            key,
            save,
            added: None,
        });
        cx.notify();
    }

    /// The `rules.json` rule a record's link joins as an example: the one
    /// the record names (`Matched::Rule`), read from the record with no
    /// probe, while the file is as it was when the link was routed and
    /// that rule is still on and sending links where this one went.
    /// Otherwise (a record that did not number its rule, or a file saved,
    /// reloaded or edited since) the rule that takes the link now, by
    /// probe: a rule inserted, moved or edited since may have put another
    /// rule at that number. `None` when no rule takes it.
    fn example_rule(&mut self, record: &DispatchRecord) -> Option<usize> {
        match matched(record) {
            Some(Matched::Rule { index })
                if self.rules_unchanged_since(record.received_at)
                    && self.still_routes(index, record) =>
            {
                Some(index)
            }
            Some(Matched::Rule { .. } | Matched::Unreported) => {
                let url = matched_url(record).to_owned();
                let opener = engine_opener(record);
                self.probe.winning_handler(&url, opener.as_ref())
            }
            Some(Matched::Default { .. } | Matched::Handler { .. }) | None => None,
        }
    }

    /// `rules.json` has not changed since `at`: nothing is unsaved, and the
    /// file was last written (by anyone, read at load) or reloaded no later.
    /// A reload's time is when the change was noticed, which is never
    /// earlier than the write, so the answer errs towards a probe. While
    /// the router's last reload failed it still routes with the files as
    /// they were before it, whatever the store says, so never then.
    fn rules_unchanged_since(&self, at: SystemTime) -> bool {
        if crate::router::routing_source().error.is_some() {
            return false;
        }
        match self.store.status() {
            Status::Saved { at: Some(saved) } => *saved <= at,
            Status::Reloaded { at: reloaded } => *reloaded <= at,
            Status::Saved { at: None }
            | Status::Unsaved
            | Status::Saving
            | Status::Failed(_)
            | Status::Changed => false,
        }
    }

    /// The file's rule at `index` is on and opens what the record opened.
    fn still_routes(&self, index: usize, record: &DispatchRecord) -> bool {
        let (Some(rule), Some(browser)) = (self.handlers.get(index), &record.browser) else {
            return false;
        };
        rule.rule.enabled
            && rule.browser.trim() == browser.name
            && rule.profile.trim() == browser.profile
    }

    /// Add a rule for this link (design 2c): opens the sheet for the row's
    /// record, from the expanded row's "Add a rule…" or the "⋯" menu. A
    /// record kept as host only or nothing gets the sheet's disabled state.
    pub fn add_rule(&mut self, key: RowKey, _window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(record) = record(key, cx) else {
            return;
        };
        // "Becomes rule N" counts the file as it is.
        self.sync_with_disk(cx);
        let mut sheet = if record.privacy == Privacy::Full {
            AddRule::new(&record, cx)
        } else {
            AddRule::disabled(&record, cx)
        };
        sheet.key = key;
        self.find_collisions(&mut sheet, cx);
        sheet.rules_first = self.rules_first();
        sheet.config = self.config.as_ref().map(|config| config.path.clone());
        self.actions.sheet = Some(sheet);
        self.actions.menu = None;
        self.actions.note = None;
        cx.notify();
    }

    /// Open in… on a not-opened row: shows its lanes, or hides them.
    pub fn toggle_open_in(&mut self, key: RowKey, cx: &mut Context<'_, Self>) {
        self.activity.open_in = if self.activity.open_in == Some(key) {
            None
        } else {
            Some(key)
        };
        self.activity.open_note = None;
        cx.notify();
    }

    /// Opens a not-opened link now: once, `how` the user picked, as a new
    /// dispatch that names the one it came from (`Step::Released`). The
    /// rules are not changed. The router does it on its own thread; the
    /// feed brings the new record, and a link no longer waiting says why
    /// beside the row. Hold's rows open through here too.
    pub fn open_not_opened(&mut self, key: RowKey, how: OpenAs, cx: &mut Context<'_, Self>) {
        self.activity.open_in = None;
        self.activity.open_note = None;
        cx.notify();
        let Some(router) = activity::feed(cx).and_then(|feed| feed.read(cx).router()) else {
            self.activity.open_note = Some((key, "PwrRoute isn't routing links now.".to_owned()));
            return;
        };
        let opened = cx
            .background_executor()
            .spawn(async move { router.open_not_opened(key.id, how) });
        cx.spawn(async move |this, cx| {
            let result = opened.await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    let note = match error {
                        OpenLaterError::NotPending => activity::NO_LONGER_KEPT.to_owned(),
                        OpenLaterError::Gone(_) => "PwrRoute isn't routing links now.".to_owned(),
                    };
                    this.activity.open_note = Some((key, note));
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// ⌘N (Ctrl+N) on Activity: Add a rule for the row whose "⋯" menu is
    /// open, else the expanded row, as the menu's first item would. Returns
    /// whether it took the key.
    pub fn activity_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let keystroke = &event.keystroke;
        let modifiers = &keystroke.modifiers;
        if keystroke.key != "n" || !modifiers.secondary() || modifiers.shift || modifiers.alt {
            return false;
        }
        if self.actions.sheet.is_some() {
            return false;
        }
        // Only a row on screen: the menu and the expanded row outlive a
        // change of filter or search that hides them.
        let Some(key) = self
            .actions
            .menu
            .as_ref()
            .map(|menu| menu.key)
            .or(self.activity.expanded)
            .filter(|key| self.activity.is_visible(*key))
        else {
            return false;
        };
        self.add_rule(key, window, cx);
        true
    }

    /// Closes the sheet without adding anything, and gives the list the
    /// keyboard back (the sheet's field may have had it).
    pub fn cancel_add_rule(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.actions.sheet = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// The recent links in memory the sheet's pattern would also catch,
    /// by Finicky's own matching.
    fn find_collisions(&mut self, sheet: &mut AddRule, cx: &App) {
        sheet.collisions.clear();
        sheet.collision_error = None;
        let pattern = sheet.pattern.text.trim().to_owned();
        if pattern.is_empty() || sheet.disabled.is_some() {
            return;
        }
        let Some(feed) = activity::feed(cx) else {
            return;
        };
        // A rule that requires the app only ever sees that app's links.
        let opener = sheet.sender_spec().filter(|_| sheet.only_from);
        let records: Vec<DispatchRecord> = feed
            .read(cx)
            .records()
            .iter()
            .filter(|r| (sheet.key.journaled || r.id != sheet.key.id) && r.privacy == Privacy::Full)
            .filter(|r| !matches!(r.outcome, Outcome::Failed))
            .filter(|r| match &opener {
                Some(app) => sent_by(r, app),
                None => true,
            })
            .cloned()
            .collect();
        let urls: Vec<&str> = records.iter().map(matched_url).collect();
        match self.probe.collisions(&pattern, &urls) {
            Ok(hits) => {
                sheet.collisions = records
                    .iter()
                    .filter(|r| hits.iter().any(|hit| hit == matched_url(r)))
                    .map(|r| Collision {
                        time: clock(r.received_at),
                        url: matched_url(r).to_owned(),
                        opened: opened_label(r),
                        took: Took::of(r),
                    })
                    .collect();
            }
            Err(error) => sheet.collision_error = Some(error),
        }
    }

    /// Picks one of the suggested patterns.
    pub fn pick_candidate(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        let Some(mut sheet) = self.actions.sheet.take() else {
            return;
        };
        sheet.choose_candidate(index);
        self.find_collisions(&mut sheet, cx);
        self.actions.sheet = Some(sheet);
        cx.notify();
    }

    /// Picks "Any link from <app>".
    pub fn pick_any_from(&mut self, cx: &mut Context<'_, Self>) {
        self.change_match(AddRule::choose_any_from, cx);
    }

    /// The Sent from switch, which changes what the rule catches.
    pub fn toggle_only_from(&mut self, cx: &mut Context<'_, Self>) {
        self.change_match(AddRule::toggle_only_from, cx);
    }

    /// A change to what the rule matches: the collisions follow it.
    fn change_match(&mut self, change: impl FnOnce(&mut AddRule), cx: &mut Context<'_, Self>) {
        let Some(mut sheet) = self.actions.sheet.take() else {
            return;
        };
        change(&mut sheet);
        self.find_collisions(&mut sheet, cx);
        self.actions.sheet = Some(sheet);
        cx.notify();
    }

    /// A keystroke in the sheet's pattern field. Returns whether it took it.
    pub fn add_rule_key(&mut self, event: &KeyDownEvent, cx: &mut Context<'_, Self>) -> bool {
        let Some(mut sheet) = self.actions.sheet.take() else {
            return false;
        };
        let edit = sheet.pattern.key(&event.keystroke, cx);
        if edit == Edit::Changed {
            sheet.pattern_typed();
            self.find_collisions(&mut sheet, cx);
        }
        self.actions.sheet = Some(sheet);
        edit != Edit::Ignored
    }

    /// Changes one of the sheet's controls. A change makes a new rule, so
    /// the next Add checks it again.
    pub fn set_add_rule(&mut self, change: impl FnOnce(&mut AddRule), cx: &mut Context<'_, Self>) {
        if let Some(sheet) = &mut self.actions.sheet {
            change(sheet);
            sheet.problem = None;
            cx.notify();
        }
    }

    /// Add rule: the sheet's rule goes into the file with the link as its
    /// example (or, when it requires the sending app, as a test), at the
    /// top or the bottom, and the tests run.
    ///
    /// The first click checks the rule against the file first. A rule the
    /// file already has is never added again. One that would not take its
    /// link (an earlier rule or the config in charge takes it first, or
    /// its pattern misses) keeps the sheet open and says why; a second
    /// click adds it anyway.
    pub fn add_rule_from_sheet(&mut self, cx: &mut Context<'_, Self>) {
        let Some(sheet) = &self.actions.sheet else {
            return;
        };
        let Some(rule) = sheet.rule() else {
            return;
        };
        let at = match sheet.position {
            Position::Top => 0,
            Position::Bottom => self.store.handler_count(),
        };
        let warning = match &sheet.problem {
            Some(problem) if problem.kind == ProblemKind::Duplicate => return,
            Some(problem) => Some(problem.text.clone()),
            None => {
                if let Some(problem) = self.check_new_rule(&rule, at) {
                    if let Some(sheet) = &mut self.actions.sheet {
                        sheet.problem = Some(problem);
                    }
                    cx.notify();
                    return;
                }
                None
            }
        };
        let Some(sheet) = self.actions.sheet.take() else {
            return;
        };
        let key = sheet.key;
        let test = sheet.test();
        let index = self.store.insert_rule(at, rule);
        // A draft at or below the new rule moved down one.
        self.drafts_after_insert(index);
        let kept = match test {
            Some(test) => {
                let number = self.store.add_test(test);
                format!("this link as test {}", number + 1)
            }
            None => "this link as its example".to_owned(),
        };
        let after = match &self.config {
            Some(config) if self.rules_first() => {
                format!("; your rules run before {}'s handlers", config.path)
            }
            Some(config) => format!("; visual rules run after {}", config.path),
            None => String::new(),
        };
        let mut note = format!(
            "Added rule {} with {kept}{after}. Saved to rules.json; Run tests fails if \
             another rule ever takes it.",
            index + 1
        );
        if let Some(warning) = warning {
            note = format!("{note} {warning}");
        }
        self.actions.note = Some((key, note));
        // The note shows in the row's menu, so it opens again with it, and
        // with a way to the new rule.
        self.actions.menu = Some(RowMenu {
            key,
            save: None,
            added: Some(index),
        });
        self.selection = Some(Selection::Handler(index));
        // Beside a config the visual rules start collapsed: open them, or
        // the Rules screen shows the config's handlers without this one.
        self.visual_rules_expanded = true;
        self.refresh(cx);
        self.rebuild_rail(cx);
        self.run_tests(cx);
    }

    /// Why `rule`, added at `at`, would not take the sheet's link: the
    /// file already has it, or the engine over the file with it added
    /// (and the config in charge) sends the link elsewhere. `None` when it
    /// would take the link, or the probe fails (the tests report that).
    fn check_new_rule(&mut self, rule: &engine::Rule, at: usize) -> Option<Problem> {
        let sheet = self.actions.sheet.as_ref()?;
        let mut rules = self.store.edited();
        if let Some(same) = rules.rules.iter().position(|r| same_rule(r, rule)) {
            return Some(Problem {
                kind: ProblemKind::Duplicate,
                text: format!("Rule {} is already this rule. Nothing was added.", same + 1),
                advice: "Change the pattern, browser or position to add a different rule.",
            });
        }
        rules.rules.insert(at, rule.clone());
        let trace = self
            .probe
            .with_rules(rules.clone())
            .trace(&sheet.url, sheet.opener.as_ref())
            .ok()?;
        let winner = trace.winner()?;
        let shadowed = |text: String, advice| {
            Some(Problem {
                kind: ProblemKind::Shadowed,
                text,
                advice,
            })
        };
        match (winner.source, winner.stage) {
            (Source::Rules, Stage::Handler) if winner.index == at => None,
            // An earlier rule; a later one means this rule missed.
            (Source::Rules, Stage::Handler) if winner.index < at => {
                let pattern = rules
                    .rules
                    .get(winner.index)
                    .and_then(|r| r.patterns.get(winner.pattern.unwrap_or(0)))
                    .map(|p| format!(" ({p})"))
                    .unwrap_or_default();
                shadowed(
                    format!(
                        "Rule {}{pattern} takes this link first, so this rule would never see \
                         it.",
                        winner.index + 1
                    ),
                    "Put it at the top, or click Add anyway.",
                )
            }
            // Config handlers run first only under the config's order;
            // under PwrRoute's, one winning means this rule missed.
            (Source::Config, Stage::Handler) if !self.rules_first() => {
                let config = self
                    .config
                    .as_ref()
                    .map_or("the config", |config| config.path.as_str());
                shadowed(
                    format!(
                        "Config handler {} of {config} takes this link first, and visual rules \
                         run after the config, so this rule would never see it.",
                        winner.index + 1
                    ),
                    "Route this link in the config instead, or click Add anyway.",
                )
            }
            _ => shadowed(
                "This rule does not match the link as the rules see it, so the link would \
                 still go where it goes now."
                    .to_owned(),
                "Check the pattern, or click Add anyway.",
            ),
        }
    }

    /// Show in Rules: the handler Add rule just added, selected, with the
    /// visual rules open beside a config.
    pub fn show_added_rule(
        &mut self,
        handler: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.actions.menu = None;
        self.visual_rules_expanded = true;
        self.selection = Some(Selection::Handler(handler));
        self.navigate(crate::app::Screen::Rules, window, cx);
    }

    /// Save as example: the link joins the examples of the `rules.json`
    /// rule that took it ([`PwrRoute::example_rule`]). A link no rule
    /// takes becomes a test.
    pub fn save_record_as_example(&mut self, key: RowKey, cx: &mut Context<'_, Self>) {
        let Some(record) = record(key, cx) else {
            return;
        };
        if record.privacy != Privacy::Full {
            return;
        }
        let url = matched_url(&record).to_owned();
        match self.example_rule(&record) {
            Some(handler) => {
                self.store.add_example(handler, &url);
                self.actions.note = Some((
                    key,
                    format!(
                        "Saved to rules.json under rule {}. Run tests fails if another rule \
                         ever takes it.",
                        handler + 1
                    ),
                ));
                self.refresh(cx);
                self.run_tests(cx);
            }
            None => self.save_record_as_test(key, cx),
        }
    }

    /// Save as test: a top-level test that expects where this link went.
    pub fn save_record_as_test(&mut self, key: RowKey, cx: &mut Context<'_, Self>) {
        let Some(record) = record(key, cx) else {
            return;
        };
        if record.privacy != Privacy::Full {
            return;
        }
        let Some(test) = test_for(&record) else {
            return;
        };
        let expects = opened_label(&record);
        let index = self.store.add_test(test);
        self.actions.note = Some((
            key,
            format!(
                "Saved to rules.json \u{203a} tests as test {}: expects {expects}. Run tests \
                 checks it on every save.",
                index + 1
            ),
        ));
        self.refresh(cx);
        self.run_tests(cx);
    }
}

/// Add a rule's key as the window's chrome writes it: ⌘N on macOS,
/// Ctrl+N on Windows and Linux. `PwrRoute::activity_key` handles it.
pub fn add_rule_shortcut(cx: &App) -> &'static str {
    match crate::chrome::current(cx) {
        crate::chrome::Chrome::Mac => "\u{2318}N",
        crate::chrome::Chrome::Windows | crate::chrome::Chrome::Linux => "Ctrl+N",
    }
}

/// `.menu`: the row's "⋯" menu (design 2b), under the button while
/// `Actions::menu` names this row: Add a rule for this link, then Save as
/// example for the `rules.json` rule that took the link or Save
/// as test for where it went, and a footer with what the last action here
/// did, or why there is no Save. The menu closes on a mouse down outside
/// it. Nothing for a record the feed no longer has.
pub fn row_menu(
    app: &PwrRoute,
    index: usize,
    key: RowKey,
    cx: &mut Context<'_, PwrRoute>,
) -> Option<AnyElement> {
    let menu = app.actions.menu.as_ref().filter(|menu| menu.key == key)?;
    let record = record(key, cx)?;
    let item = |id: &'static str, label: String, hint: Option<String>| {
        menu_row(
            ElementId::NamedInteger(id.into(), index as u64),
            None,
            label,
            hint.map(Into::into),
        )
        .debug_selector(move || format!("{id}-{index}"))
        .hover(|row| row.bg(theme::bg_row_active()))
    };
    let add = item(
        "activity-add-rule-menu",
        "Add a rule for this link\u{2026}".into(),
        Some(add_rule_shortcut(cx).to_owned()),
    )
    .on_click(cx.listener(move |this, _, window, cx| {
        cx.stop_propagation();
        this.add_rule(key, window, cx);
    }));
    let save = menu.save.as_ref().map(|save| {
        let (label, hint, example) = match save {
            Save::Example { handler, pattern } => (
                format!("Save as example for rule {}", handler + 1),
                pattern.clone(),
                true,
            ),
            Save::Test { expects } => (
                "Save as test".to_owned(),
                Some(format!("expects {expects}")),
                false,
            ),
        };
        item("activity-save", label, hint).on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            if example {
                this.save_record_as_example(key, cx);
            } else {
                this.save_record_as_test(key, cx);
            }
        }))
    });
    let show = menu.added.map(|handler| {
        item(
            "activity-show-rule",
            // No number: the Rules list numbers its own rows.
            "Show in Rules".to_owned(),
            None,
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.show_added_rule(handler, window, cx);
        }))
    });
    let foot = |content: String| {
        div().px(px(8.0)).pt(px(6.0)).pb(px(4.0)).child(paragraph(
            content,
            10.5,
            15.0,
            theme::text_subtle(),
        ))
    };
    let note = app
        .actions
        .note
        .as_ref()
        .filter(|(noted, _)| *noted == key)
        .map(|(_, note)| {
            foot(note.clone())
                .id("activity-action-note")
                .debug_selector(|| "activity-action-note".into())
                .into_any_element()
        });
    let why = note.is_none().then(|| match record.privacy {
        Privacy::Full => match matched(&record) {
            Some(Matched::Default { owner }) => Some(format!(
                "No rule matched; it went to {}. Add a rule, or save it as a test so a future \
                 rule cannot take it unnoticed.",
                match owner {
                    Some(Owner::PwrRoute) => "PwrRoute's default",
                    Some(Owner::Config) => "the config's default",
                    None => "the default",
                }
            )),
            _ => None,
        },
        Privacy::None => None,
        Privacy::Host => Some(
            "Kept as host only: there is no full URL to write a rule or a test from.".to_owned(),
        ),
    });
    let menu = menu_card(("activity-menu", index))
        .debug_selector(move || format!("activity-menu-{index}"))
        .absolute()
        .top(px(30.0))
        .right_0()
        .w(px(320.0))
        .rounded(px(8.0))
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.actions.menu = None;
            cx.notify();
        }))
        .child(add)
        .children(show)
        .children(save)
        .children(note)
        .children(why.flatten().map(foot));
    Some(deferred(menu).into_any_element())
}

// --- the sheet ------------------------------------------------------------

/// `.scrim`: black at 62%, as the design dims the window behind a sheet.
fn scrim() -> gpui_kit::Hsla {
    let mut color: gpui_kit::Hsla = gpui_kit::rgb(0x000000).into();
    color.a = 0.62;
    color
}

/// `.field`: a label and what it says on the left, the control on the right.
fn field(label: &'static str, sub: Option<&'static str>, control: Div) -> Div {
    div()
        .flex()
        .gap(px(20.0))
        .py(px(11.0))
        .px(px(18.0))
        .border_b_1()
        .border_color(theme::border_faint())
        .child(
            div()
                .w(px(136.0))
                .flex_none()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(text(label, 12.0, 600.0, theme::text_primary()).line_height(px(15.6)))
                .when_some(sub, |cell, sub| {
                    cell.child(paragraph(sub, 11.0, 16.5, theme::text_muted()))
                }),
        )
        .child(control.flex_1().min_w_0().flex().flex_col().gap(px(8.0)))
}

/// `.field-help`.
fn help(content: impl Into<SharedString>) -> Div {
    paragraph(content, 10.5, 15.75, theme::text_subtle()).max_w(px(360.0))
}

/// `.pat`: one Match choice, its radio, what is chosen and why. The
/// choice takes its width up to what the explanation leaves it, then ends
/// in an ellipsis; the explanation keeps `WHY_MIN` and wraps, so a long
/// pattern never pushes it out of the row or stretches the row tall.
fn choice_row(
    id: ElementId,
    selector: String,
    selected: bool,
    chosen: Div,
    why: &'static str,
) -> Stateful<Div> {
    let radio = div()
        .flex_none()
        .size(px(13.0))
        .rounded_full()
        .border_1()
        .border_color(theme::border_control())
        .when(selected, |radio| {
            radio.border_4().border_color(theme::accent())
        });
    let value = format!("{selector}-value");
    let explained = format!("{selector}-why");
    div()
        .id(id)
        .debug_selector(move || selector.clone())
        .flex()
        .items_center()
        .gap(px(9.0))
        .py(px(7.0))
        .px(px(10.0))
        .border_1()
        .rounded(px(7.0))
        .cursor_pointer()
        .map(|row| {
            if selected {
                row.border_color(theme::accent_border())
                    .bg(theme::bg_row_active())
            } else {
                row.border_color(theme::border_subtle())
                    .bg(theme::bg_input())
            }
        })
        .child(radio)
        .child(chosen.min_w_0().debug_selector(move || value.clone()))
        .child(
            div()
                .debug_selector(move || explained.clone())
                .flex_1()
                .min_w(px(WHY_MIN))
                .pl(px(10.0))
                .text_right()
                .child(
                    paragraph(
                        why,
                        10.5,
                        14.0,
                        if selected {
                            theme::accent_bright()
                        } else {
                            theme::text_muted()
                        },
                    )
                    .font_weight(theme::weight(500.0)),
                ),
        )
}

/// The narrowest a Match choice's explanation gets: about two words a
/// line, so a pattern as long as `auth.example.com/authorize*` still fits
/// beside it in the sheet.
const WHY_MIN: f32 = 110.0;

/// A pattern longer than this may be cut in its row (about what fits
/// beside a `WHY_MIN` explanation): it shows whole in a tooltip, and in the
/// field under the choices.
const PATTERN_FITS: usize = 24;

/// One suggested pattern.
fn candidate(
    index: usize,
    pattern: &str,
    why: &'static str,
    selected: bool,
    cx: &mut Context<'_, PwrRoute>,
) -> Stateful<Div> {
    choice_row(
        ElementId::NamedInteger("add-rule-candidate".into(), index as u64),
        format!("add-rule-candidate-{index}"),
        selected,
        div().child(one_line(
            pattern.to_owned(),
            theme::mono_family(500.0),
            500.0,
            11.5,
            11.5,
            theme::text_primary(),
        )),
        why,
    )
    .when(pattern.chars().count() > PATTERN_FITS, |row| {
        row.tooltip(crate::overview::tip(pattern.to_owned(), None))
    })
    .on_click(cx.listener(move |this, _, _, cx| this.pick_candidate(index, cx)))
}

/// `.pat.app`: "Any link from <app>", above the patterns (design: Add Rule
/// From an App 1b).
fn any_from_choice(sheet: &AddRule, cx: &mut Context<'_, PwrRoute>) -> Option<Stateful<Div>> {
    let from = sheet.from.as_ref()?;
    let app = [&from.path, &from.bundle_id, &from.name]
        .into_iter()
        .find(|s| !s.is_empty())
        .cloned()
        .unwrap_or_default();
    let icon = AppBadge::new(badge_for(&from.name).letters().clone(), app);
    Some(
        choice_row(
            "add-rule-any-from".into(),
            "add-rule-any-from".into(),
            sheet.any_from,
            div()
                .flex()
                .items_center()
                .gap(px(9.0))
                .child(badge(icon))
                .child(one_line(
                    format!("Any link from {}", from.name),
                    theme::SANS,
                    600.0,
                    12.0,
                    12.0,
                    theme::text_primary(),
                )),
            "Every site. Turns on Sent from.",
        )
        .on_click(cx.listener(|this, _, _, cx| this.pick_any_from(cx))),
    )
}

/// The link in the sheet's head: host and path brighter than the query,
/// cut at two lines with Show full link (design: Add Rule From an App 1b,
/// 1d).
fn head_link(sheet: &AddRule, cx: &mut Context<'_, PwrRoute>) -> Div {
    let lead = sheet.url_lead();
    let link = StyledText::new(sheet.url.clone()).with_highlights([(
        0..lead,
        HighlightStyle {
            color: Some(theme::text_primary()),
            ..HighlightStyle::default()
        },
    )]);
    let long = sheet.url_is_long();
    let expanded = sheet.url_expanded;
    let body = div()
        .id("add-rule-link")
        .debug_selector(|| "add-rule-link".into())
        .mt(px(4.0))
        .font_family(theme::mono_family(500.0))
        .font_weight(theme::weight(500.0))
        .text_size(px(11.0))
        .line_height(px(16.5))
        .text_color(theme::text_secondary())
        .map(|body| {
            if expanded {
                // Eight lines, then it scrolls.
                body.max_h(px(132.0)).overflow_y_scroll()
            } else {
                body.line_clamp(2).text_ellipsis()
            }
        })
        .child(link);
    div().flex().flex_col().child(body).when(long, |head| {
        let label = if expanded {
            "Show less"
        } else {
            "Show full link"
        };
        head.child(
            div()
                .mt(px(5.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                            .id("add-rule-link-toggle")
                            .debug_selector(|| "add-rule-link-toggle".into())
                            .cursor_pointer()
                            .child(text(label, 10.5, 500.0, theme::accent_bright()))
                            // Only the view changes: a warning from Add stays.
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(sheet) = &mut this.actions.sheet {
                                    sheet.url_expanded = !sheet.url_expanded;
                                    cx.notify();
                                }
                            })),
                )
                .when_some(
                    sheet.query_summary.clone().filter(|_| !expanded),
                    |row, summary| {
                        row.child(text(
                            format!("\u{00b7} {summary}"),
                            10.5,
                            500.0,
                            theme::text_subtle(),
                        ))
                    },
                ),
        )
    })
}

/// `.coll`: the recent links the pattern would also catch.
fn collisions(sheet: &AddRule) -> Div {
    let list = div()
        .flex()
        .flex_col()
        .border_1()
        .border_color(theme::border_subtle())
        .rounded(px(7.0))
        .overflow_hidden()
        .child(
            div()
                .py(px(6.0))
                .px(px(10.0))
                .bg(theme::bg_elevated())
                // The caption wraps: who keeps what can take a sentence.
                .child(
                    paragraph(
                        sheet
                            .collision_error
                            .clone()
                            .unwrap_or_else(|| sheet.collision_caption()),
                        10.5,
                        13.65,
                        theme::text_secondary(),
                    )
                    .font_weight(theme::weight(600.0)),
                ),
        );
    sheet
        .collisions
        .iter()
        .enumerate()
        .fold(list, |list, (index, hit)| {
            let last = index + 1 == sheet.collisions.len();
            list.child(
                div()
                    .id(("add-rule-collision", index))
                    .debug_selector(move || format!("add-rule-collision-{index}"))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .py(px(6.0))
                    .px(px(10.0))
                    .when(!last, |row| {
                        row.border_b_1().border_color(theme::border_faint())
                    })
                    .child(div().w(px(50.0)).flex_none().child(mono(
                        hit.time.clone(),
                        10.0,
                        500.0,
                        theme::text_subtle(),
                    )))
                    .child(
                        one_line(
                            hit.url.clone(),
                            theme::mono_family(500.0),
                            500.0,
                            10.5,
                            10.5,
                            theme::text_primary(),
                        )
                        .flex_1(),
                    )
                    .child(
                        div()
                            .w(px(150.0))
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .child(badge(badge_for(&hit.opened)))
                            .child(one_line(
                                hit.opened.clone(),
                                theme::SANS,
                                400.0,
                                10.5,
                                10.5,
                                theme::text_secondary(),
                            )),
                    ),
            )
        })
}

/// The sheet's pattern field: `.inp.mono`, editable.
fn pattern_field(
    sheet: &AddRule,
    window: &Window,
    cx: &mut Context<'_, PwrRoute>,
) -> Stateful<Div> {
    let focused = sheet.pattern.focus.is_focused(window);
    let look = crate::text_field::Look {
        mono: true,
        placeholder: "example.com/*",
    };
    let mut inner =
        input_box()
            .w_full()
            .child(crate::text_field::text(&sheet.pattern, look, focused));
    if sheet.any_from {
        inner = inner.child(text("any link", 10.5, 500.0, theme::text_muted()).ml(px(2.0)));
    }
    inner = inner.child(
        div()
            .ml_auto()
            .child(text("edit", 10.5, 500.0, theme::text_subtle())),
    );
    let boxed = if focused {
        crate::widgets::focus_ring(inner)
    } else {
        inner
    };
    let area = div()
        .id("add-rule-pattern")
        .debug_selector(|| "add-rule-pattern".into());
    crate::text_field::hit_area(area, &sheet.pattern)
        .w_full()
        .child(boxed)
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
            if this.add_rule_key(event, cx) {
                cx.stop_propagation();
                cx.notify();
            }
        }))
        .on_click(cx.listener(|this, _, window, cx| {
            if let Some(sheet) = &this.actions.sheet {
                window.focus(&sheet.pattern.focus, cx);
                cx.notify();
            }
        }))
}

/// Open in: the browsers and their profiles as one control (design:
/// Destination and App Pickers 1a, 1b). The browser the link opened in
/// leads; one the scan does not list still gets its lane.
fn open_in(app: &PwrRoute, sheet: &AddRule, cx: &mut Context<'_, PwrRoute>) -> Div {
    let mut browsers = app.browsers.clone();
    if let Some(opened) = &sheet.opened_in
        && !browsers.iter().any(|b| b.name.eq_ignore_ascii_case(opened))
    {
        browsers.push(BrowserChoice {
            name: opened.clone(),
            profiles: Vec::new(),
        });
    }
    let current = Choice {
        browser: sheet.browser.clone(),
        profile: sheet.profile.clone(),
        private: sheet.private,
    };
    let lanes = destinations::lanes(
        &browsers,
        &app.installed,
        &current,
        sheet
            .opened_in
            .as_deref()
            .map(|opened| (opened, "where it opened")),
        crate::editor::scanned(app, cx),
        true,
    );
    let private_word = sheet
        .private
        .then(|| crate::overview_model::private_term(&sheet.browser))
        .flatten();
    div()
        .child(destinations::draw(
            "add-rule-dest",
            &lanes,
            destinations::Density::Regular,
            |this: &mut PwrRoute, choice, _, cx| {
                this.set_add_rule(|sheet| sheet.choose(choice), cx)
            },
            cx,
        ))
        .when_some(private_word, |control, word| {
            control.child(help(format!(
                "{} opens these in its {word} window. A private rule keeps host only unless \
                 Keep says otherwise.",
                crate::default_browser::browser_name(&sheet.browser)
            )))
        })
}

/// The sheet's line about the config in charge, in the order
/// `rules.json`'s `precedence` sets, as the router runs it.
pub(crate) fn config_hint(path: &str, rules_first: bool) -> String {
    if rules_first {
        format!(
            "{path} is in charge, and your rules run before its handlers: a rule here takes \
             the link ahead of the config."
        )
    } else {
        format!(
            "{path} is in charge, and visual rules run after it: a rule here only wins when \
             the config does not take the link."
        )
    }
}

/// `.needjs`: why a precise rule needs a JS config, when it does.
fn needs_js(app: &PwrRoute, sheet: &AddRule) -> Option<Div> {
    if !sheet.needs_js_now() && app.config.is_none() {
        return None;
    }
    let mut lines: Vec<String> = Vec::new();
    if sheet.needs_js_now() {
        // The reasons are sentences; the first one continues this one, so
        // its capital goes, unless it starts an acronym ("URL").
        let reasons = sheet.needs_js.join(" ");
        let mut chars = reasons.chars();
        let reasons = match (chars.next(), chars.next()) {
            (Some(first), Some(second)) if second.is_lowercase() || second == ' ' => {
                first.to_lowercase().chain([second]).chain(chars).collect()
            }
            _ => reasons.clone(),
        };
        lines.push(format!(
            "This needs a JS handler because {reasons} A wildcard rule here also catches \
             every link its pattern matches."
        ));
    }
    if let Some(config) = &app.config {
        lines.push(config_hint(&config.path, sheet.rules_first));
    }
    Some(
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .py(px(10.0))
            .px(px(12.0))
            .border_1()
            .border_color(theme::accent_border())
            .rounded(px(7.0))
            .bg(theme::bg_row_active())
            .children(
                lines
                    .into_iter()
                    .map(|line| paragraph(line, 11.5, 17.8, theme::text_secondary())),
            ),
    )
}

/// The sheet's body for a record kept in full.
fn sheet_body(
    app: &PwrRoute,
    sheet: &AddRule,
    window: &Window,
    cx: &mut Context<'_, PwrRoute>,
) -> Vec<AnyElement> {
    let candidates = sheet.candidates.iter().enumerate().fold(
        div().flex().flex_col().gap(px(5.0)),
        |list, (index, (pattern, why))| {
            list.child(candidate(
                index,
                pattern,
                why,
                sheet.selected == Some(index),
                cx,
            ))
        },
    );
    let any_from = any_from_choice(sheet, cx);
    let sub = if any_from.is_some() {
        "Every link from the app, or a pattern, narrowest first. The query and fragment are \
         left out."
    } else {
        "Narrowest first. The query and fragment are left out."
    };
    let choices = match any_from {
        Some(any_from) => div().flex().flex_col().gap(px(5.0)).child(any_from).when(
            !sheet.candidates.is_empty(),
            |list| {
                list.child(
                    tracked("or by pattern", 9.0, 600.0, 0.14, theme::text_subtle())
                        .mt(px(3.0))
                        .mb(px(1.0)),
                )
                .child(candidates)
            },
        ),
        None => div().when(!sheet.candidates.is_empty(), |list| list.child(candidates)),
    };
    let mut body: Vec<AnyElement> = Vec::new();
    body.push(
        field(
            "Match",
            Some(sub),
            div().child(choices).child(pattern_field(sheet, window, cx)),
        )
        .into_any_element(),
    );
    body.push(
        field(
            "Also catches",
            Some("Recent links in memory that this pattern matches."),
            div().child(collisions(sheet)),
        )
        .into_any_element(),
    );
    if let Some(from) = &sheet.from {
        let name = from.name.clone();
        body.push(
            field(
                "Sent from",
                Some("Optional."),
                div()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .child(
                                div()
                                    .id("add-rule-only-from")
                                    .debug_selector(|| "add-rule-only-from".into())
                                    .cursor_pointer()
                                    .child(switch(sheet.only_from))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_only_from(cx);
                                    })),
                            )
                            .child(text(
                                format!("Only when sent from {name}"),
                                11.5,
                                500.0,
                                theme::text_primary(),
                            )),
                    )
                    .child(help(if sheet.any_from {
                        format!(
                            "On because Match is any link from {name}. Off goes back to the \
                             narrowest pattern."
                        )
                    } else {
                        format!("{name} sent this one. Off, the rule matches a link from any app.")
                    })),
            )
            .into_any_element(),
        );
    }
    body.push(
        field(
            "Open in",
            Some("Pick a profile, or the browser's private window."),
            open_in(app, sheet, cx),
        )
        .into_any_element(),
    );
    let keep = KEEP_LEVELS
        .iter()
        .fold(segmented().self_start(), |seg, (level, label)| {
            let level = *level;
            let key = match level {
                Privacy::Full => "full",
                Privacy::Host => "host",
                Privacy::None => "none",
            };
            seg.child(
                segment(*label, sheet.keep == level)
                    .id(ElementId::Name(format!("add-rule-keep-{key}").into()))
                    .debug_selector(move || format!("add-rule-keep-{key}"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_add_rule(|sheet| sheet.keep = level, cx);
                    })),
            )
        });
    body.push(
        field(
            "Keep",
            Some("What Activity and the journal remember."),
            div().child(keep).child(help(
                "Full link: the link as received and as opened. Host only: scheme and host. Nothing: \
                 a count.",
            )),
        )
        .into_any_element(),
    );
    let position = [(Position::Top, "Top"), (Position::Bottom, "Bottom")]
        .into_iter()
        .fold(segmented().self_start(), |seg, (position, label)| {
            let key = label.to_lowercase();
            seg.child(
                segment(label, sheet.position == position)
                    .id(ElementId::Name(format!("add-rule-position-{key}").into()))
                    .debug_selector(move || format!("add-rule-position-{key}"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_add_rule(|sheet| sheet.position = position, cx);
                    })),
            )
        });
    body.push(
        field("Position", Some("First match wins."), div().child(position)).into_any_element(),
    );
    if let Some(note) = needs_js(app, sheet) {
        body.push(
            div()
                .py(px(11.0))
                .px(px(18.0))
                .child(note)
                .into_any_element(),
        );
    }
    body
}

/// Why the rule would not take the link, found by the first Add. It
/// takes the footer's place, beside the button, so it is on screen
/// however far the body has scrolled.
fn problem_note(problem: &Problem) -> Stateful<Div> {
    div()
        .id("add-rule-problem")
        .debug_selector(|| "add-rule-problem".into())
        .flex()
        .flex_col()
        .gap(px(4.0))
        .py(px(10.0))
        .px(px(12.0))
        .rounded(px(7.0))
        .bg(theme::warn_soft())
        .child(paragraph(
            problem.text.clone(),
            11.5,
            17.8,
            theme::warn_text(),
        ))
        .child(paragraph(
            problem.advice,
            11.5,
            17.8,
            theme::text_secondary(),
        ))
}

/// The Add a rule sheet over the window, when it is open.
pub fn sheet(
    app: &PwrRoute,
    window: &Window,
    cx: &mut Context<'_, PwrRoute>,
) -> Option<impl IntoElement + use<>> {
    let sheet = app.actions.sheet.as_ref()?;
    let header = div()
        .pt(px(16.0))
        .px(px(18.0))
        .pb(px(12.0))
        .border_b_1()
        .border_color(theme::border_faint())
        .flex()
        .flex_col()
        .child(tracked(
            &sheet.eyebrow(),
            9.5,
            600.0,
            0.14,
            theme::text_muted(),
        ))
        .child(
            text(sheet.title(), 16.0, 700.0, theme::text_primary())
                .mt(px(6.0))
                .line_height(px(19.2)),
        )
        .when(sheet.disabled.is_none(), |header| {
            header.child(head_link(sheet, cx))
        });
    let (body, footer): (Vec<AnyElement>, Div) = match &sheet.disabled {
        Some(reason) => (
            vec![
                div()
                    .py(px(14.0))
                    .px(px(18.0))
                    .child(paragraph(
                        reason.clone(),
                        11.5,
                        17.8,
                        theme::text_secondary(),
                    ))
                    .into_any_element(),
            ],
            div().child(div().flex_1()).child(
                button("Close", ButtonKind::Default, false)
                    .id("add-rule-cancel")
                    .debug_selector(|| "add-rule-cancel".into())
                    .on_click(cx.listener(|this, _, window, cx| this.cancel_add_rule(window, cx))),
            ),
        ),
        None => {
            let handler = match sheet.position {
                Position::Top => 1,
                Position::Bottom => app.store.handler_count() + 1,
            };
            let duplicate = sheet
                .problem
                .as_ref()
                .is_some_and(|problem| problem.kind == ProblemKind::Duplicate);
            let ready = sheet.rule().is_some() && !duplicate;
            let label = if !sheet.needs_js_now() && sheet.problem.is_none() {
                "Add rule"
            } else {
                "Add anyway"
            };
            let add = button(label, ButtonKind::Primary, false)
                .id("add-rule-save")
                .debug_selector(|| "add-rule-save".into())
                .when(!ready, |add| add.opacity(0.5))
                .on_click(cx.listener(|this, _, _, cx| this.add_rule_from_sheet(cx)));
            (
                sheet_body(app, sheet, window, cx),
                div()
                    .child(match &sheet.problem {
                        Some(problem) => {
                            problem_note(problem).flex_1().min_w_0().into_any_element()
                        }
                        None => paragraph(sheet.footer(handler), 10.5, 15.75, theme::text_subtle())
                            .flex_1()
                            .min_w_0()
                            .into_any_element(),
                    })
                    .child(
                        button("Cancel", ButtonKind::Quiet, false)
                            .id("add-rule-cancel")
                            .debug_selector(|| "add-rule-cancel".into())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.cancel_add_rule(window, cx)),
                            ),
                    )
                    .child(add),
            )
        }
    };
    let footer = footer
        .flex()
        .items_center()
        .gap(px(8.0))
        .py(px(12.0))
        .px(px(18.0))
        .border_t_1()
        .border_color(theme::border_faint());
    let panel = div()
        .id("add-rule-sheet")
        .debug_selector(|| "add-rule-sheet".into())
        .occlude()
        .w(px(560.0))
        // Shrinks to the window: the body scrolls, the head and foot stay.
        .min_h_0()
        .flex()
        .flex_col()
        .bg(theme::bg_elevated())
        .border_1()
        .border_color(theme::border_default())
        .rounded(px(12.0))
        .shadow_lg()
        .overflow_hidden()
        .child(header)
        .child(
            div()
                .id("add-rule-body")
                .debug_selector(|| "add-rule-body".into())
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .py(px(4.0))
                .children(body),
        )
        .child(footer);
    Some(deferred(
        div()
            .id("add-rule-scrim")
            .debug_selector(|| "add-rule-scrim".into())
            .occlude()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .items_center()
            .pt(px(56.0))
            .pb(px(24.0))
            .bg(scrim())
            // A click outside the sheet does nothing: only Cancel throws
            // away what the person has chosen so far.
            .child(panel),
    ))
}

/// Open in…'s lanes under a not-opened row (design: Not Opened 2b): the
/// tray's compact lanes over the scan, no pill on, and a pill opens the
/// link there at once.
pub fn open_in_lanes(
    app: &PwrRoute,
    index: usize,
    key: RowKey,
    cx: &mut Context<'_, PwrRoute>,
) -> Div {
    let lanes = destinations::lanes(
        &app.browsers,
        &app.installed,
        &Choice::default(),
        None,
        crate::editor::scanned(app, cx),
        false,
    );
    div()
        .debug_selector(move || format!("activity-open-in-lanes-{index}"))
        .mt(px(4.0))
        .w(px(320.0))
        .p(px(8.0))
        .flex()
        .flex_col()
        .gap(px(6.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(theme::border_default())
        .bg(theme::bg_elevated())
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(text(
                    "Open this link in",
                    11.5,
                    600.0,
                    theme::text_primary(),
                ))
                .child(text(
                    "once \u{00b7} rules unchanged",
                    10.0,
                    500.0,
                    theme::text_subtle(),
                )),
        )
        .child(destinations::draw(
            "activity-open-in",
            &lanes,
            destinations::Density::Compact,
            move |this: &mut PwrRoute, choice, _, cx| {
                let destination = Destination {
                    browser: choice.browser,
                    profile: choice.profile,
                    private: choice.private,
                };
                this.open_not_opened(key, OpenAs::In(destination), cx);
            },
            cx,
        ))
        .when(app.browsers.is_empty(), |list| {
            list.child(paragraph(
                "No browsers found. Rescan in Settings \u{203a} Browsers.",
                10.5,
                15.2,
                theme::text_subtle(),
            ))
        })
        .child(paragraph(
            "Add a rule to send links like this somewhere every time.",
            10.5,
            15.2,
            theme::text_subtle(),
        ))
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use pwrroute_router::{AppType, Browser, DispatchId, Link, Source, Timings};

    use super::*;

    fn record(url: &str, privacy: Privacy, by: Matched, from: Option<&str>) -> DispatchRecord {
        let browser = Browser {
            name: "Google Chrome".into(),
            app_type: AppType::AppName,
            url: url.into(),
            profile: "Personal".into(),
            args: Vec::new(),
            open_in_background: None,
            private: false,
            privacy,
        };
        let mut record = DispatchRecord {
            id: DispatchId(1),
            received_at: SystemTime::now(),
            privacy,
            link: Link::new(url, Source::AppleEvent).with_opener(from.map(|name| OpenerInfo {
                name: name.into(),
                bundle_id: format!("com.example.{}", name.to_lowercase()),
                ..OpenerInfo::default()
            })),
            forwarded: false,
            final_url: Some(url.into()),
            browser: Some(browser.clone()),
            steps: vec![
                Step::Received { url: url.into() },
                Step::Matched { by },
                Step::Browser { browser },
                Step::Launched { dry_run: true },
            ],
            outcome: Outcome::DryRun,
            timings: Timings::default(),
        };
        if privacy != Privacy::Full {
            record.reduce(privacy);
        }
        record
    }

    #[gpui_kit::test]
    fn the_sheet_builds_a_rule_with_the_link_as_its_example(cx: &mut gpui_kit::TestAppContext) {
        let record = record(
            "https://app.metrics.example/dashboard/abc-123?from_ts=1&to_ts=2",
            Privacy::Full,
            Matched::Default { owner: None },
            Some("Slack"),
        );
        let mut sheet = cx.update(|cx| AddRule::new(&record, cx));
        assert_eq!(
            sheet.eyebrow(),
            format!(
                "From Activity \u{00b7} {} \u{00b7} sent from Slack",
                sheet.time
            )
        );
        assert_eq!(
            sheet
                .candidates
                .iter()
                .map(|(p, _)| p.as_str())
                .collect::<Vec<_>>(),
            ["app.metrics.example/dashboard/*", "app.metrics.example/*"]
        );
        assert!(sheet.needs_js.is_empty());
        assert_eq!(sheet.selected, Some(0));
        assert_eq!(sheet.pattern.text, "app.metrics.example/dashboard/*");
        // No browser is chosen for the person: Add waits for one.
        assert_eq!(sheet.browser, "");
        assert_eq!(sheet.rule(), None);
        assert_eq!(sheet.footer(8), "Choose a browser to add it as rule 8.");
        sheet.browser = "Firefox".into();
        let rule = sheet.rule().expect("a rule");
        assert_eq!(rule.patterns, ["app.metrics.example/dashboard/*"]);
        assert_eq!(rule.browser, "Firefox");
        assert!(rule.opener.is_empty());
        assert_eq!(rule.privacy, None);
        assert_eq!(
            rule.examples,
            ["https://app.metrics.example/dashboard/abc-123?from_ts=1&to_ts=2"]
        );
        // Only from Slack, private, keeping nothing.
        sheet.only_from = true;
        sheet.private = true;
        sheet.keep = Privacy::None;
        sheet.profile = "default-release".into();
        let rule = sheet.rule().expect("a rule");
        assert_eq!(rule.opener, ["com.example.slack"]);
        assert!(rule.private);
        assert_eq!(rule.privacy, Some(Privacy::None.into()));
        assert_eq!(rule.profile, "default-release");
        // A rule that requires the app keeps the link as a test from it.
        assert!(rule.examples.is_empty());
        let test = sheet.test().expect("a test");
        assert_eq!(test.opener.as_deref(), Some("Slack:com.example.slack"));
        assert_eq!(test.expect.browser, "Firefox");
        assert_eq!(test.expect.profile.as_deref(), Some("default-release"));
        assert!(sheet.footer(10).contains("from Slack as a saved test"));
        sheet.only_from = false;
        assert!(sheet.test().is_none());
        assert!(sheet.footer(10).contains("as its example"));
        // Private with host only is what private implies: not written.
        sheet.keep = Privacy::Host;
        assert_eq!(sheet.rule().unwrap().privacy, None);
        assert_eq!(
            sheet.collision_caption(),
            "No other recent link matches this pattern."
        );
        sheet.collisions = vec![Collision {
            time: "09:33:05".into(),
            url: "https://app.metrics.example/dashboard/9f2e".into(),
            opened: "Google Chrome \u{00b7} Personal".into(),
            took: Took::Unrouted,
        }];
        assert_eq!(
            sheet.collision_caption(),
            "Would also catch 1 recent link \u{00b7} unrouted, nothing changes hands"
        );
    }

    #[gpui_kit::test]
    fn any_link_from_the_app_is_every_site_from_that_app(cx: &mut gpui_kit::TestAppContext) {
        // An id in the path: a precise rule needs JS, but not one by app.
        let from = record(
            "https://issues.example/12345/comments?view=full&tab=2",
            Privacy::Full,
            Matched::Default { owner: None },
            Some("Mail"),
        );
        let mut sheet = cx.update(|cx| AddRule::new(&from, cx));
        sheet.browser = "Firefox".into();
        assert!(sheet.needs_js_now());
        sheet.choose_any_from();
        assert!(!sheet.needs_js_now());
        let rule = sheet.rule().expect("a rule");
        assert_eq!(rule.patterns, [ANY_LINK]);
        assert_eq!(rule.opener, ["com.example.mail"]);
        assert!(rule.examples.is_empty());
        assert!(sheet.test().is_some());
        assert_eq!(
            sheet.collision_caption(),
            "No other recent link came from Mail."
        );
        // Typing a pattern leaves it, Sent from stays as it is.
        sheet.pattern.set("issues.example/*");
        sheet.pattern_typed();
        assert!(!sheet.any_from && sheet.only_from);
        assert_eq!(
            sheet.collision_caption(),
            "No other recent link from Mail matches this pattern."
        );

        // The head: host and path lead, and the query is counted.
        assert_eq!(
            &sheet.url[..sheet.url_lead()],
            "https://issues.example/12345/comments"
        );
        assert_eq!(sheet.query_summary.as_deref(), Some("2 query parameters"));
        assert!(!sheet.url_is_long());

        // A keystroke that leaves the pattern `*` keeps where Sent from was:
        // choosing a pattern still turns it back off.
        sheet.only_from = false;
        sheet.choose_any_from();
        sheet.pattern.set("* ");
        sheet.pattern_typed();
        assert!(sheet.any_from);
        sheet.choose_candidate(0);
        assert!(!sheet.any_from && !sheet.only_from);

        // A scheme without `//` and a `://` in the query: the lead stops at
        // the query.
        let mail = record(
            "mailto:a@b.example?body=see%20https://x.example/y",
            Privacy::Full,
            Matched::Default { owner: None },
            None,
        );
        let mail = cx.update(|cx| AddRule::new(&mail, cx));
        assert_eq!(&mail.url[..mail.url_lead()], "mailto:a@b.example");

        // No app on record: nothing to offer.
        let anon = record(
            "https://issues.example/a",
            Privacy::Full,
            Matched::Default { owner: None },
            None,
        );
        let mut sheet = cx.update(|cx| AddRule::new(&anon, cx));
        sheet.choose_any_from();
        assert!(!sheet.any_from && !sheet.only_from);
        assert_ne!(sheet.pattern.text, ANY_LINK);
        assert_eq!(sheet.query_summary, None);
    }

    #[gpui_kit::test]
    fn an_id_in_the_path_needs_js_and_a_host_record_is_disabled(cx: &mut gpui_kit::TestAppContext) {
        let with_id = record(
            "https://issues.example/12345/comments",
            Privacy::Full,
            Matched::Default { owner: None },
            None,
        );
        let sheet = cx.update(|cx| AddRule::new(&with_id, cx));
        assert_eq!(sheet.needs_js.len(), 1);
        assert!(sheet.needs_js[0].contains("regex"), "{:?}", sheet.needs_js);
        assert_eq!(
            sheet.eyebrow(),
            format!("From Activity \u{00b7} {}", sheet.time)
        );
        assert_eq!(sheet.browser, "", "no browser is chosen for the person");

        let host = record(
            "https://bank.example/login",
            Privacy::Host,
            Matched::Unreported,
            None,
        );
        let sheet = cx.update(|cx| AddRule::disabled(&host, cx));
        let reason = sheet.disabled.clone().expect("disabled");
        assert!(
            reason.starts_with("The rule that routed this link keeps host only"),
            "{reason}"
        );
        assert!(sheet.rule().is_none());
        assert!(
            !reason.contains("login"),
            "the reason never quotes the link"
        );
    }

    #[test]
    fn a_record_becomes_a_test_that_expects_where_it_went() {
        let record = record(
            "https://www.apple.com/macos/",
            Privacy::Full,
            Matched::Default { owner: None },
            Some("Safari"),
        );
        let test = test_for(&record).expect("a test");
        assert_eq!(test.url, "https://www.apple.com/macos/");
        assert_eq!(test.opener.as_deref(), Some("Safari:com.example.safari"));
        assert_eq!(test.expect.browser, "Google Chrome");
        assert_eq!(test.expect.profile.as_deref(), Some("Personal"));
        assert_eq!(
            test.expect.url.as_deref(),
            Some("https://www.apple.com/macos/")
        );
        assert_eq!(opened_label(&record), "Google Chrome \u{00b7} Personal");
    }

    /// A short link the router expanded: the rules matched where it landed,
    /// and Test a link and the saved tests do not expand, so everything
    /// saved from the row uses the landing URL.
    #[gpui_kit::test]
    fn an_expanded_record_saves_and_suggests_the_landing_url(cx: &mut gpui_kit::TestAppContext) {
        let short = "https://bit.ly/gh";
        let landed = "https://github.com/pwrdrvr/PwrGit/pull/361";
        let mut record = record(short, Privacy::Full, Matched::Unreported, None);
        record
            .steps
            .insert(1, Step::Expanded { url: landed.into() });
        // fixtures/rules.json's handler 2 (github.com/pwrdrvr/*) took it.
        let browser = Browser {
            name: "Google Chrome".into(),
            app_type: AppType::AppName,
            url: landed.into(),
            profile: "Work".into(),
            args: Vec::new(),
            open_in_background: None,
            private: false,
            privacy: Privacy::Full,
        };
        record.final_url = Some(landed.into());
        record.browser = Some(browser.clone());
        for step in &mut record.steps {
            if let Step::Browser { browser: b } = step {
                *b = browser.clone();
            }
        }
        assert_eq!(matched_url(&record), landed);

        // The menu's probe finds the handler for the landing URL, not the
        // short one, so the row offers Save as example under it.
        let mut probe = crate::try_it::Probe::new(None, crate::fixtures::rules_file());
        assert_eq!(probe.winning_handler(short, None), None);
        assert_eq!(probe.winning_handler(matched_url(&record), None), Some(1));

        // Save as test pins the landing URL, and it passes as saved.
        let test = test_for(&record).expect("a test");
        assert_eq!(test.url, landed);
        assert_eq!(test.expect.url.as_deref(), Some(landed));
        let mut rules = crate::fixtures::rules_file();
        rules.tests = vec![test];
        rules.rules[1].examples = vec![matched_url(&record).to_owned()];
        let mut runs = crate::try_it::TestRuns::new(None, rules);
        let report = runs.run().as_ref().expect("the tests ran");
        assert_eq!((report.total, report.passed), (2, 2), "{report:?}");

        // Add a rule suggests patterns for the landing host, with it as
        // the example.
        let mut sheet = cx.update(|cx| AddRule::new(&record, cx));
        sheet.browser = "Firefox".into();
        assert_eq!(sheet.url, landed);
        assert!(
            sheet
                .candidates
                .iter()
                .all(|(p, _)| p.starts_with("github.com/")),
            "{:?}",
            sheet.candidates
        );
        assert_eq!(sheet.rule().expect("a rule").examples, [landed]);
    }

    /// Attribution (WP-B): the caption says which caught links would
    /// change hands, by who routed them and the order rules run in.
    #[gpui_kit::test]
    fn the_collision_caption_follows_the_precedence(cx: &mut gpui_kit::TestAppContext) {
        let record = record(
            "https://docs.example.com/a",
            Privacy::Full,
            Matched::Default { owner: None },
            None,
        );
        let mut sheet = cx.update(|cx| AddRule::new(&record, cx));
        let hit = |took| Collision {
            time: "09:33:05".into(),
            url: "https://docs.example.com/b".into(),
            opened: "Firefox".into(),
            took,
        };
        sheet.config = Some("~/.finicky.ts".into());
        sheet.collisions = vec![
            hit(Took::Unrouted),
            hit(Took::Rule(0)),
            hit(Took::ConfigHandler(1)),
            hit(Took::ConfigHandler(2)),
        ];

        // The config's order, at the bottom: rule 1 runs first, and the
        // config's handlers run before every rules.json rule.
        sheet.rules_first = false;
        sheet.position = Position::Bottom;
        assert_eq!(
            sheet.collision_caption(),
            "Would also catch these 4 recent links \u{00b7} 1 stays with an earlier rule, \
             which runs first; 2 stay with ~/.finicky.ts's handlers, which run first"
        );
        // On top it takes rule 1's link; the config's handlers still win.
        sheet.position = Position::Top;
        assert_eq!(
            sheet.collision_caption(),
            "Would also catch these 4 recent links \u{00b7} 1 routed by a rule now, which \
             this rule would take first; 2 stay with ~/.finicky.ts's handlers, which run first"
        );
        // PwrRoute's order: rules.json runs before the config's handlers.
        sheet.rules_first = true;
        assert_eq!(
            sheet.collision_caption(),
            "Would also catch these 4 recent links \u{00b7} 3 routed by a rule or config \
             handler now, which this rule would take first"
        );
        sheet.position = Position::Bottom;
        assert_eq!(
            sheet.collision_caption(),
            "Would also catch these 4 recent links \u{00b7} 2 routed by a config handler now, \
             which this rule would take first; 1 stays with an earlier rule, which runs first"
        );
        // A record that does not say who routed it moves for sure only on
        // top with rules.json first, stays for sure at the bottom behind
        // the config, and otherwise may move: never counted as moved.
        sheet.collisions = vec![hit(Took::Unknown), hit(Took::Unknown)];
        sheet.position = Position::Top;
        assert_eq!(
            sheet.collision_caption(),
            "Would also catch these 2 recent links \u{00b7} 2 routed by a rule or config \
             handler now, which this rule would take first"
        );
        sheet.position = Position::Bottom;
        assert_eq!(
            sheet.collision_caption(),
            "Would also catch these 2 recent links \u{00b7} 2 may change hands: their records \
             do not say who routed them"
        );
        sheet.rules_first = false;
        sheet.collisions = vec![hit(Took::Unknown)];
        assert_eq!(
            sheet.collision_caption(),
            "Would also catch 1 recent link \u{00b7} 1 stays with an earlier rule or \
             ~/.finicky.ts's handlers, which run first"
        );
        sheet.position = Position::Top;
        sheet.collisions = vec![hit(Took::Unknown), hit(Took::Rule(0))];
        assert_eq!(
            sheet.collision_caption(),
            "Would also catch these 2 recent links \u{00b7} 1 routed by a rule now, which this \
             rule would take first; 1 may change hands: its record does not say who routed it"
        );

        // Only defaults and the fallback: nobody loses a link.
        sheet.collisions = vec![hit(Took::Unrouted), hit(Took::Unrouted)];
        assert_eq!(
            sheet.collision_caption(),
            "Would also catch these 2 recent links \u{00b7} all unrouted, nothing changes hands"
        );
    }

    #[test]
    fn a_collision_is_attributed_from_its_record() {
        let took = |by| {
            Took::of(&record(
                "https://docs.example.com/a",
                Privacy::Full,
                by,
                None,
            ))
        };
        assert_eq!(took(Matched::Rule { index: 3 }), Took::Rule(3));
        assert_eq!(took(Matched::Unreported), Took::Unknown);
        assert_eq!(took(Matched::Handler { index: 1 }), Took::ConfigHandler(1));
        assert_eq!(
            took(Matched::Default {
                owner: Some(Owner::Config)
            }),
            Took::Unrouted
        );
    }

    /// The sheet's line about the config follows `precedence`: the visual
    /// rules run after the config only in the config's order.
    #[test]
    fn the_config_hint_follows_the_precedence() {
        assert_eq!(
            config_hint("~/.finicky.ts", false),
            "~/.finicky.ts is in charge, and visual rules run after it: a rule here only wins \
             when the config does not take the link."
        );
        assert_eq!(
            config_hint("~/.finicky.ts", true),
            "~/.finicky.ts is in charge, and your rules run before its handlers: a rule here \
             takes the link ahead of the config."
        );
    }
}
