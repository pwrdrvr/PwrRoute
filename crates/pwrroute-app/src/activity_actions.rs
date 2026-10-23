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

use chrono::{DateTime, Local};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _,
    Window, deferred, div, px,
};
use pwrroute_engine::{
    self as engine, Expectation, Opener, Precedence, Privacy, RouteTest, Source, Stage,
    suggest_rule,
};
use pwrroute_router::{DispatchRecord, Matched, OpenerInfo, Outcome, Owner, Step};

use crate::activity::{self, RowKey, matched_url};
use crate::app::{PwrRoute, Selection};
use crate::rules_model::KEEP_LEVELS;
use crate::text_field::{Edit, TextField};
use crate::theme::{self, mono, text, tracked};
use crate::widgets::{
    ButtonKind, badge, badge_for, button, caret, field_label, input_box, menu_card, menu_row,
    one_line, paragraph, profiles_hint, segment, segmented, switch,
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

/// Which of the sheet's menus is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Picking {
    #[default]
    None,
    Browser,
    Profile,
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
    /// One of `rules.json`'s rules; `None` for a record that did not
    /// number it. A new rule at the top runs before it, at the bottom
    /// after it.
    Rule(Option<usize>),
    /// The JS config's handler: it runs before every `rules.json` rule
    /// unless `rules.json` comes first (`"precedence": "pwrroute"`, or no
    /// config in charge).
    ConfigHandler(usize),
}

impl Took {
    fn of(record: &DispatchRecord) -> Self {
        match matched(record) {
            Some(Matched::Rule { index }) => Self::Rule(Some(index)),
            Some(Matched::Unreported) => Self::Rule(None),
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
    pub browser: String,
    pub profile: String,
    pub picking: Picking,
    was_picking: Picking,
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
    /// the first candidate selected, `browser` as the first choice.
    pub fn new(record: &DispatchRecord, browsers: &[BrowserChoice], cx: &mut App) -> Self {
        let url = matched_url(record).to_owned();
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
        let browser = browsers
            .first()
            .map(|b| b.name.clone())
            .or_else(|| record.browser.as_ref().map(|b| b.name.clone()))
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
            browser,
            profile: String::new(),
            picking: Picking::None,
            was_picking: Picking::None,
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
            ..Self::new(record, &[], cx)
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

    /// The rule the sheet would add, with the link as its example. A rule
    /// that requires the sending app gets no example, since examples run
    /// from no app: the link goes in as a test instead ([`AddRule::test`]).
    /// `None` while it has no pattern or browser.
    pub fn rule(&self) -> Option<engine::Rule> {
        let pattern = self.pattern.text.trim();
        if pattern.is_empty() || self.browser.trim().is_empty() || self.disabled.is_some() {
            return None;
        }
        let opener = match (&self.from, self.only_from) {
            (Some(app), true) => vec![if app.bundle_id.is_empty() {
                app.name.clone()
            } else {
                app.bundle_id.clone()
            }],
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
    pub fn collision_caption(&self) -> String {
        let n = self.collisions.len();
        if n == 0 {
            return "No other recent link matches this pattern.".into();
        }
        let head = format!(
            "Would also catch these {n} recent {}",
            plural(n, "link", "links")
        );
        let (mut moved_rules, mut moved_config, mut kept_rules, mut kept_config) = (0, 0, 0, 0);
        for collision in &self.collisions {
            match collision.took {
                Took::Unrouted => {}
                Took::Rule(_) if self.position == Position::Top => moved_rules += 1,
                Took::Rule(_) => kept_rules += 1,
                Took::ConfigHandler(_) if self.rules_first => moved_config += 1,
                Took::ConfigHandler(_) => kept_config += 1,
            }
        }
        let mut parts = Vec::new();
        let moved = moved_rules + moved_config;
        if moved > 0 {
            let by = match (moved_rules > 0, moved_config > 0) {
                (true, false) => "a rule",
                (false, true) => "a config handler",
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
        if kept_config > 0 {
            let config = self.config.as_deref().unwrap_or("the config");
            parts.push(format!(
                "{kept_config} {} with {config}'s handlers, which run first",
                plural(kept_config, "stays", "stay"),
            ));
        }
        if parts.is_empty() {
            format!("{head} \u{00b7} all unrouted, nothing changes hands")
        } else {
            format!("{head} \u{00b7} {}", parts.join("; "))
        }
    }
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
fn clock(at: std::time::SystemTime) -> String {
    DateTime::<Local>::from(at).format("%H:%M:%S").to_string()
}

/// "Google Chrome · Personal", or the browser alone.
fn opened_label(record: &DispatchRecord) -> String {
    match &record.browser {
        Some(browser) if browser.profile.is_empty() => browser.name.clone(),
        Some(browser) => format!("{} \u{00b7} {}", browser.name, browser.profile),
        None => "not opened".into(),
    }
}

/// The opener as the engine sees it, for a re-resolution.
fn engine_opener(record: &DispatchRecord) -> Option<Opener> {
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
    fn rules_first(&self) -> bool {
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
    /// probe, while the file still has it there, on and sending links
    /// where this one went. Otherwise (a record that did not number its
    /// rule, or a file changed since) the rule that takes the link now, by
    /// probe. `None` when no rule takes it.
    fn example_rule(&mut self, record: &DispatchRecord) -> Option<usize> {
        match matched(record) {
            Some(Matched::Rule { index }) if self.still_routes(index, record) => Some(index),
            Some(Matched::Rule { .. } | Matched::Unreported) => {
                let url = matched_url(record).to_owned();
                let opener = engine_opener(record);
                self.probe.winning_handler(&url, opener.as_ref())
            }
            Some(Matched::Default { .. } | Matched::Handler { .. }) | None => None,
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
            AddRule::new(&record, &self.browsers, cx)
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

    /// Closes the sheet without adding anything.
    pub fn cancel_add_rule(&mut self, cx: &mut Context<'_, Self>) {
        self.actions.sheet = None;
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
        let records: Vec<DispatchRecord> = feed
            .read(cx)
            .records()
            .iter()
            .filter(|r| (sheet.key.journaled || r.id != sheet.key.id) && r.privacy == Privacy::Full)
            .filter(|r| !matches!(r.outcome, Outcome::Failed))
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
        if let Some((pattern, _)) = sheet.candidates.get(index) {
            sheet.pattern.set(pattern.clone());
            sheet.selected = Some(index);
            sheet.problem = None;
            self.find_collisions(&mut sheet, cx);
        }
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
            let typed = sheet.pattern.text.trim().to_owned();
            sheet.selected = sheet.candidates.iter().position(|(p, _)| *p == typed);
            sheet.problem = None;
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
                    .map_or("The config", |config| config.path.as_str());
                shadowed(
                    format!(
                        "{config} handler {} takes this link first, and visual rules run after \
                         the config, so this rule would never see it.",
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
        None,
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
            format!("Show rule {} in Rules", handler + 1),
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

/// `.pat`: one suggested pattern, its radio and why.
fn candidate(
    index: usize,
    pattern: &str,
    why: &'static str,
    selected: bool,
    cx: &mut Context<'_, PwrRoute>,
) -> Stateful<Div> {
    let radio = div()
        .flex_none()
        .size(px(13.0))
        .rounded_full()
        .border_1()
        .border_color(theme::border_strong())
        .when(selected, |radio| {
            radio.border_4().border_color(theme::accent())
        });
    div()
        .id(("add-rule-candidate", index))
        .debug_selector(move || format!("add-rule-candidate-{index}"))
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
        // The pattern is what is chosen: it keeps its width and the
        // explanation wraps.
        .child(div().flex_none().child(one_line(
            pattern.to_owned(),
            theme::mono_family(500.0),
            500.0,
            11.5,
            11.5,
            theme::text_primary(),
        )))
        .child(
            div().flex_1().min_w_0().pl(px(10.0)).text_right().child(
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
        .on_click(cx.listener(move |this, _, _, cx| this.pick_candidate(index, cx)))
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
                .child(text(
                    sheet
                        .collision_error
                        .clone()
                        .unwrap_or_else(|| sheet.collision_caption()),
                    10.5,
                    600.0,
                    theme::text_secondary(),
                )),
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
    let content = if sheet.pattern.text.is_empty() {
        div()
            .text_color(theme::text_subtle())
            .child("example.com/*")
    } else {
        div()
            .text_color(theme::text_primary())
            .when(sheet.pattern.selected, |text| text.bg(theme::accent_soft()))
            .child(sheet.pattern.shared())
    };
    let content = content
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .font_family(theme::mono_family(500.0))
        .font_weight(theme::weight(500.0))
        .text_size(px(12.0))
        .line_height(px(12.0));
    let mut inner = input_box().w_full().child(content);
    if focused && !sheet.pattern.selected {
        inner = inner.child(
            div()
                .flex_none()
                .w(px(1.0))
                .h(px(14.0))
                .bg(theme::accent())
                .ml(px(-8.0)),
        );
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
    let focus = sheet.pattern.focus.clone();
    div()
        .id("add-rule-pattern")
        .debug_selector(|| "add-rule-pattern".into())
        .track_focus(&focus)
        .w_full()
        .cursor_text()
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

/// A picker: the current value in a box, and while open its menu.
fn picker(
    id: &'static str,
    which: Picking,
    label: Div,
    choices: Vec<(String, Option<String>)>,
    sheet: &AddRule,
    cx: &mut Context<'_, PwrRoute>,
) -> Div {
    let menu = choices.into_iter().enumerate().fold(
        menu_card(ElementId::Name(format!("{id}-menu").into()))
            .debug_selector(move || format!("{id}-menu"))
            // As wide as the field, never narrower.
            .absolute()
            .top(px(34.0))
            .left_0()
            .right_0()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.set_add_rule(|sheet| sheet.picking = Picking::None, cx);
            })),
        |menu, (index, (name, extra))| {
            let chosen = name.clone();
            let badge_label = (which == Picking::Browser).then(|| badge_for(&name));
            menu.child(
                menu_row(
                    ElementId::Name(format!("{id}-choice-{index}").into()),
                    badge_label,
                    name.clone(),
                    extra.map(Into::into),
                )
                .debug_selector(move || format!("{id}-choice-{index}"))
                .hover(|row| row.bg(theme::bg_hover()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let chosen = chosen.clone();
                    this.set_add_rule(
                        move |sheet| {
                            match which {
                                Picking::Browser => {
                                    if sheet.browser != chosen {
                                        sheet.profile.clear();
                                    }
                                    sheet.browser = chosen;
                                }
                                Picking::Profile => {
                                    sheet.profile = if chosen == "Default" {
                                        String::new()
                                    } else {
                                        chosen
                                    };
                                }
                                Picking::None => {}
                            }
                            sheet.picking = Picking::None;
                        },
                        cx,
                    );
                })),
            )
        },
    );
    div()
        .relative()
        .w_full()
        .child(
            input_box()
                .id(ElementId::Name(id.into()))
                .debug_selector(move || id.into())
                .cursor_pointer()
                .child(label)
                .child(caret())
                .capture_any_mouse_down(cx.listener(|this, _, _, _| {
                    if let Some(sheet) = &mut this.actions.sheet {
                        sheet.was_picking = sheet.picking;
                    }
                }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_add_rule(
                        |sheet| {
                            sheet.picking = if sheet.was_picking == which {
                                Picking::None
                            } else {
                                which
                            };
                        },
                        cx,
                    );
                })),
        )
        .when(sheet.picking == which, |picker| {
            picker.child(deferred(menu))
        })
}

/// The browser and profile pickers, from the detected browsers.
fn open_in(app: &PwrRoute, sheet: &AddRule, cx: &mut Context<'_, PwrRoute>) -> Div {
    let mut browsers: Vec<(String, Option<String>)> = app
        .browsers
        .iter()
        .map(|b| (b.name.clone(), profiles_hint(&b.profiles)))
        .collect();
    if !sheet.browser.is_empty() && !browsers.iter().any(|(name, _)| *name == sheet.browser) {
        browsers.push((sheet.browser.clone(), None));
    }
    let mut profiles: Vec<(String, Option<String>)> = vec![("Default".into(), None)];
    profiles.extend(
        app.browsers
            .iter()
            .find(|b| b.name == sheet.browser)
            .map(|b| {
                b.profiles
                    .iter()
                    .map(|p| (p.clone(), None))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );
    let browser_label = if sheet.browser.is_empty() {
        div().child(text("Choose a browser", 12.5, 400.0, theme::text_subtle()))
    } else {
        div()
            .flex()
            .min_w_0()
            .items_center()
            .gap(px(8.0))
            .child(badge(badge_for(&sheet.browser)))
            .child(field_label(sheet.browser.clone(), theme::text_primary()))
    };
    let profile_label = if sheet.profile.is_empty() {
        div().child(text("Default", 12.5, 400.0, theme::text_subtle()))
    } else {
        div()
            .min_w_0()
            .child(field_label(sheet.profile.clone(), theme::text_primary()))
    };
    div()
        .child(picker(
            "add-rule-browser",
            Picking::Browser,
            browser_label,
            browsers,
            sheet,
            cx,
        ))
        .child(picker(
            "add-rule-profile",
            Picking::Profile,
            profile_label,
            profiles,
            sheet,
            cx,
        ))
}

/// `.needjs`: why a precise rule needs a JS config, when it does.
fn needs_js(app: &PwrRoute, sheet: &AddRule) -> Option<Div> {
    if sheet.needs_js.is_empty() && app.config.is_none() {
        return None;
    }
    let mut lines: Vec<String> = Vec::new();
    if !sheet.needs_js.is_empty() {
        lines.push(format!(
            "This needs a JS handler because {} A wildcard rule here also catches every link \
             its pattern matches.",
            sheet.needs_js.join(" ")
        ));
    }
    if let Some(config) = &app.config {
        lines.push(format!(
            "{} is in charge, and visual rules run after it: a rule here only wins when the \
             config does not take the link.",
            config.path
        ));
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
    let mut body: Vec<AnyElement> = Vec::new();
    body.push(
        field(
            "Match",
            Some("Narrowest first. The query and fragment are left out."),
            div()
                .when(!sheet.candidates.is_empty(), |field| {
                    field.child(candidates)
                })
                .child(pattern_field(sheet, window, cx)),
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
                                        this.set_add_rule(
                                            |sheet| sheet.only_from = !sheet.only_from,
                                            cx,
                                        );
                                    })),
                            )
                            .child(text(
                                format!("Only when sent from {name}"),
                                11.5,
                                500.0,
                                theme::text_primary(),
                            )),
                    )
                    .child(help(format!(
                        "{name} sent this one. Off, the rule matches a link from any app."
                    ))),
            )
            .into_any_element(),
        );
    }
    body.push(field("Open in", None, open_in(app, sheet, cx)).into_any_element());
    body.push(
        field(
            "Open in a private window",
            None,
            div()
                .child(
                    div()
                        .id("add-rule-private")
                        .debug_selector(|| "add-rule-private".into())
                        .cursor_pointer()
                        .self_start()
                        .child(switch(sheet.private))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_add_rule(|sheet| sheet.private = !sheet.private, cx);
                        })),
                )
                .child(help(
                    "The browser's own private-window flag. On, the rule keeps host only \
                     unless Keep says otherwise.",
                )),
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
                "Full: the link as received and as opened. Host only: scheme and host. Nothing: \
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
            header.child(
                div()
                    .mt(px(4.0))
                    .font_family(theme::mono_family(500.0))
                    .font_weight(theme::weight(500.0))
                    .text_size(px(11.0))
                    .line_height(px(16.5))
                    .text_color(theme::text_secondary())
                    .child(sheet.url.clone()),
            )
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
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_add_rule(cx))),
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
            let label = if sheet.needs_js.is_empty() && sheet.problem.is_none() {
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
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_add_rule(cx))),
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
            .on_click(cx.listener(|this, _, _, cx| this.cancel_add_rule(cx)))
            .child(panel.on_click(|_, _, cx| cx.stop_propagation())),
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
        let browsers = vec![BrowserChoice {
            name: "Firefox".into(),
            profiles: vec!["default-release".into()],
        }];
        let mut sheet = cx.update(|cx| AddRule::new(&record, &browsers, cx));
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
        assert_eq!(sheet.browser, "Firefox");
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
            "Would also catch these 1 recent link \u{00b7} all unrouted, nothing changes hands"
        );
    }

    #[gpui_kit::test]
    fn an_id_in_the_path_needs_js_and_a_host_record_is_disabled(cx: &mut gpui_kit::TestAppContext) {
        let with_id = record(
            "https://issues.example/12345/comments",
            Privacy::Full,
            Matched::Default { owner: None },
            None,
        );
        let sheet = cx.update(|cx| AddRule::new(&with_id, &[], cx));
        assert_eq!(sheet.needs_js.len(), 1);
        assert!(sheet.needs_js[0].contains("regex"), "{:?}", sheet.needs_js);
        assert_eq!(
            sheet.eyebrow(),
            format!("From Activity \u{00b7} {}", sheet.time)
        );
        assert_eq!(
            sheet.browser, "Google Chrome",
            "the record's browser when none is detected"
        );

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
        let sheet = cx.update(|cx| AddRule::new(&record, &[], cx));
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
        let mut sheet = cx.update(|cx| AddRule::new(&record, &[], cx));
        let hit = |took| Collision {
            time: "09:33:05".into(),
            url: "https://docs.example.com/b".into(),
            opened: "Firefox".into(),
            took,
        };
        sheet.config = Some("~/.finicky.ts".into());
        sheet.collisions = vec![
            hit(Took::Unrouted),
            hit(Took::Rule(Some(0))),
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
        assert_eq!(took(Matched::Rule { index: 3 }), Took::Rule(Some(3)));
        assert_eq!(took(Matched::Unreported), Took::Rule(None));
        assert_eq!(took(Matched::Handler { index: 1 }), Took::ConfigHandler(1));
        assert_eq!(
            took(Matched::Default {
                owner: Some(Owner::Config)
            }),
            Took::Unrouted
        );
    }
}
