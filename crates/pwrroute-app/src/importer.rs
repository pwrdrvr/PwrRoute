//! The Finicky importer's model (design `PwrRoute Finicky Importer.dc.html`):
//! one window, three steps. Review lists every rewrite, handler and the
//! default browser of a config with the engine's verdict on it
//! (`Engine::review_import`); Place puts the imported handlers before,
//! after or instead of the visual ones; Choose source says what the router
//! reads afterwards. Nothing is written until Import, which writes
//! `rules.json` once, through the main window's `RulesStore` (see
//! `importer_view`), and never the config.
//!
//! The model is plain data, so the tests drive it without a window.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use pwrroute_engine::{
    Description, Engine, Limits, NativeHost, Review, RulesFile, SourceLines, Verdict, Visual,
};

use crate::config_view::{self, display_path};
use crate::rules_model::{self, Destination, RowBadge, Rule};
use crate::settings_store::SourceSettings;
use crate::widgets::badge_for;

/// The window's three steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Review,
    Place,
    Source,
}

/// Which of the config's lists an item comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Rewrite,
    Handler,
    Default,
}

/// The chip each row carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Label {
    Imports,
    StaysJs,
    Suggestion,
}

/// One rewrite, handler or the default, as Review lists it.
#[derive(Clone, Debug)]
pub struct Item {
    pub kind: Kind,
    /// Its row as the config has it.
    pub row: Rule,
    pub verdict: Verdict,
    pub source: Option<SourceLines>,
    /// Imported with the rest. An exact import starts checked, a
    /// suggestion never does, and a rule that stays JS cannot be.
    pub checked: bool,
}

impl Item {
    pub fn label(&self) -> Label {
        match &self.verdict {
            Verdict::Imports(_) => Label::Imports,
            Verdict::StaysJs {
                suggestion: Some(_),
                ..
            } => Label::Suggestion,
            Verdict::StaysJs { .. } => Label::StaysJs,
        }
    }

    pub fn checkable(&self) -> bool {
        self.label() != Label::StaysJs
    }

    /// The line the rule starts on in the config file.
    pub fn line(&self) -> Option<usize> {
        self.source.as_ref().map(|lines| *lines.hit.start())
    }

    /// The visual rule it becomes when checked: the exact import, or the
    /// suggestion.
    pub fn visual(&self) -> Option<Visual> {
        match &self.verdict {
            Verdict::Imports(visual) => Some(visual.clone()),
            Verdict::StaysJs {
                suggestion: Some(suggested),
                ..
            } => Some(Visual::Handler(suggested.rule.clone())),
            Verdict::StaysJs { .. } => None,
        }
    }

    /// The row the visual rule draws as, for Review's "Becomes" and Place's
    /// preview.
    pub fn becomes(&self) -> Option<Rule> {
        self.visual().map(|visual| visual_row(&visual))
    }

    /// "line 8 · rewrite x.com/* → url https://…", for Choose source's list
    /// of what stops running.
    pub fn describe(&self) -> String {
        let what = match self.kind {
            Kind::Rewrite => "rewrite",
            Kind::Handler => "handler",
            Kind::Default => "defaultBrowser",
        };
        let matcher = self.row.matchers.first().map(|m| match m {
            rules_model::Matcher::Pattern(p) | rules_model::Matcher::Regex(p) => p.to_string(),
            rules_model::Matcher::Function(source) => source.to_string(),
            _ => String::new(),
        });
        let to = match &self.row.destination {
            Destination::Browser { name, profile, .. } => match profile {
                Some(profile) => format!("{name} \u{00b7} {profile}"),
                None => name.to_string(),
            },
            Destination::Rewrite(_) => "a new URL".to_owned(),
            Destination::Function(_) if self.kind == Kind::Rewrite => "a url function".to_owned(),
            Destination::Function(_) => "a browser function".to_owned(),
            Destination::Unset(_) => String::new(),
        };
        let mut text = match matcher {
            Some(matcher) => format!("{what} {} \u{2192} {to}", shorten(&matcher, 48)),
            None => format!("{what} \u{2192} {to}"),
        };
        if self.label() == Label::Suggestion {
            text.push_str(" (suggestion not taken)");
        }
        match self.line() {
            Some(line) => format!("line {line} \u{00b7} {text}"),
            None => text,
        }
    }
}

/// Where Place puts the imported handlers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Placement {
    /// The config's handlers win where both match: how it routed until now.
    #[default]
    Before,
    After,
    /// The visual handlers go; Undo import brings them back.
    Replace,
}

/// What the router reads after the import.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SourceChoice {
    /// The config in charge stays in charge; the imported rules run after
    /// it. When that is the file under review, what stayed JS keeps
    /// running in it.
    #[default]
    KeepConfig,
    /// The router stops reading the config in charge.
    VisualOnly,
}

/// Which config the router reads while the importer is open, against the
/// file under review. File › Import Finicky Config… takes any file, so it
/// need not be the one in charge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Charge {
    /// The file under review is the config in charge.
    This,
    /// Another config is in charge; Keep leaves it there.
    Other { path: PathBuf, shown: String },
    /// No config is in charge: the visual rules are, and stay so.
    NoConfig,
}

impl Charge {
    /// `in_charge` (the router's config) against the file at `path`.
    pub fn of(path: &Path, in_charge: Option<&Path>, home: Option<&Path>) -> Self {
        match in_charge {
            None => Self::NoConfig,
            Some(config) if same_file(config, path) => Self::This,
            Some(config) => Self::Other {
                path: config.to_path_buf(),
                shown: display_path(config, home),
            },
        }
    }
}

/// Whether `a` and `b` name one file: the same path, or the same file once
/// links are followed.
pub fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (a.canonicalize(), b.canonicalize()),
            (Ok(a), Ok(b)) if a == b
        )
}

/// What an import writes, against the file as it stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub file: RulesFile,
    pub rewrites: usize,
    pub handlers: usize,
    /// The default browser was imported.
    pub default: bool,
    /// Imported rules already in the file, left out rather than doubled.
    pub duplicates: usize,
    /// Of the handlers, how many came from a suggestion.
    pub suggestions: usize,
}

impl Plan {
    /// Rules the import adds.
    pub fn count(&self) -> usize {
        self.rewrites + self.handlers + usize::from(self.default)
    }
}

/// The importer as the window draws it.
#[derive(Clone, Debug)]
pub struct ImportModel {
    /// The config, as the engine loaded it.
    pub path: PathBuf,
    /// As the design names it: `~/.finicky.ts`.
    pub shown: String,
    pub items: Vec<Item>,
    /// The item Review's detail pane shows.
    pub selected: usize,
    pub step: Step,
    pub placement: Placement,
    pub source: SourceChoice,
    /// The config the router read when the importer opened.
    pub charge: Charge,
    /// `rules.json` when the importer opened, for Place's preview. Import
    /// plans against the file as it stands then.
    pub existing: RulesFile,
    /// How many links the suggestions were probed on, and how many of
    /// them were recent links.
    pub probes: usize,
    pub recent: usize,
    /// Why the config could not be read: there is nothing to review.
    pub problem: Option<String>,
    /// Why the last Import was not written, for the footer.
    pub refused: Option<String>,
}

impl ImportModel {
    /// Reads the config at `path` the way the router does and reviews it,
    /// probing suggestions on `recent` links too. `in_charge` is the config
    /// the router reads now, if any; `existing` is `rules.json` as it
    /// stands.
    pub fn read(
        path: &Path,
        home: Option<&Path>,
        in_charge: Option<&Path>,
        recent: &[&str],
        existing: RulesFile,
    ) -> Self {
        let shown = display_path(path, home);
        let charge = Charge::of(path, in_charge, home);
        let engine = Engine::load(path, Rc::new(NativeHost::new()), Limits::default());
        let reviewed = engine.and_then(|engine| {
            let description = engine.describe()?;
            let review = engine.review_import(recent)?;
            Ok(description.zip(review))
        });
        let mut model = match reviewed {
            Ok(Some((description, review))) => {
                Self::from_review(path, shown, &description, review, existing)
            }
            Ok(None) => Self::failed(path, shown, "it is not a Finicky config".to_owned()),
            Err(error) => Self::failed(path, shown, format!("{error:#}")),
        };
        model.charge = charge;
        model
    }

    /// The model for an engine's description and review of the config.
    pub fn from_review(
        path: &Path,
        shown: String,
        description: &Description,
        review: Review,
        existing: RulesFile,
    ) -> Self {
        let mut items = Vec::new();
        for (rule, reviewed) in description.rewrites.iter().zip(review.rewrites) {
            items.push(item(
                Kind::Rewrite,
                config_view::rewrite_row(rule),
                reviewed,
            ));
        }
        for (rule, reviewed) in description.handlers.iter().zip(review.handlers) {
            items.push(item(
                Kind::Handler,
                config_view::handler_row(rule),
                reviewed,
            ));
        }
        items.push(item(
            Kind::Default,
            Rule::plain(
                Vec::new(),
                config_view::destination(&description.default_browser),
            ),
            review.default_browser,
        ));
        // The detail pane opens on the first rule that imports, as the
        // design's does.
        let selected = items
            .iter()
            .position(|item| item.label() == Label::Imports)
            .unwrap_or(0);
        Self {
            path: path.to_path_buf(),
            shown,
            items,
            selected,
            step: Step::Review,
            placement: Placement::Before,
            source: SourceChoice::KeepConfig,
            charge: Charge::This,
            existing,
            probes: review.probes,
            recent: review.recent,
            problem: None,
            refused: None,
        }
    }

    /// A config that did not load: Review says why instead of listing
    /// nothing.
    pub fn failed(path: &Path, shown: String, problem: String) -> Self {
        Self {
            path: path.to_path_buf(),
            shown,
            items: Vec::new(),
            selected: 0,
            step: Step::Review,
            placement: Placement::Before,
            source: SourceChoice::KeepConfig,
            charge: Charge::This,
            existing: RulesFile::default(),
            probes: 0,
            recent: 0,
            problem: Some(problem),
            refused: None,
        }
    }

    /// First run's "Import N rules": every exact import checked, straight
    /// to the last step that applies.
    pub fn start_at_source(&mut self) {
        self.set_all(true);
        self.step = if self.source_needed() {
            Step::Source
        } else if self.place_needed() {
            Step::Place
        } else {
            Step::Review
        };
    }

    pub fn found(&self) -> usize {
        self.items.len()
    }

    /// How many import, stay JS and carry a suggestion.
    pub fn counts(&self) -> (usize, usize, usize) {
        let count = |label| self.items.iter().filter(|i| i.label() == label).count();
        (
            count(Label::Imports),
            count(Label::StaysJs),
            count(Label::Suggestion),
        )
    }

    /// "6 import · 2 stay JS · 1 suggestion"
    pub fn counts_label(&self) -> String {
        let (imports, stays, suggestions) = self.counts();
        let mut parts = vec![format!("{imports} import")];
        if stays > 0 {
            parts.push(format!("{stays} stay JS"));
        }
        if suggestions > 0 {
            parts.push(plural(suggestions, "suggestion"));
        }
        parts.join(" \u{00b7} ")
    }

    pub fn checked(&self) -> usize {
        self.items.iter().filter(|item| item.checked).count()
    }

    /// Ticks or clears one row; a rule that stays JS has no box.
    pub fn toggle(&mut self, index: usize) {
        if let Some(item) = self.items.get_mut(index)
            && item.checkable()
        {
            item.checked = !item.checked;
        }
    }

    /// The list header's box: every exact import, never a suggestion.
    pub fn set_all(&mut self, checked: bool) {
        for item in &mut self.items {
            if item.label() == Label::Imports {
                item.checked = checked;
            }
        }
    }

    /// Whether every exact import is checked, for the header's box.
    pub fn all_checked(&self) -> bool {
        self.items
            .iter()
            .filter(|item| item.label() == Label::Imports)
            .all(|item| item.checked)
    }

    /// "6 of 9 selected. Nothing is written yet."
    pub fn review_footer(&self) -> String {
        format!(
            "{} of {} selected. Nothing is written yet.",
            self.checked(),
            self.found()
        )
    }

    /// Place is a choice only when there are visual handlers already and
    /// handlers to put among them.
    pub fn place_needed(&self) -> bool {
        !self.existing.rules.is_empty()
            && self
                .items
                .iter()
                .any(|item| item.checked && matches!(item.visual(), Some(Visual::Handler(_))))
    }

    /// Choose source is a choice when the file under review is in charge
    /// and something stays behind (with nothing left in JS, visual rules
    /// are selected), and always when another config is in charge: only
    /// the reader turns that one off. With no config in charge there is
    /// nothing to choose.
    pub fn source_needed(&self) -> bool {
        match self.charge {
            Charge::This => self.items.iter().any(|item| !item.checked),
            Charge::Other { .. } => true,
            Charge::NoConfig => false,
        }
    }

    /// What stops running if the router stops reading the config.
    pub fn left_behind(&self) -> Vec<&Item> {
        self.items.iter().filter(|item| !item.checked).collect()
    }

    /// The source the import ends with: the reader's choice, or visual
    /// rules when nothing stays in JS or no config is in charge.
    pub fn final_source(&self) -> SourceChoice {
        if self.source_needed() {
            self.source
        } else {
            SourceChoice::VisualOnly
        }
    }

    /// The import turns off a config that is in charge now.
    pub fn stops_config(&self) -> bool {
        self.final_source() == SourceChoice::VisualOnly && self.charge != Charge::NoConfig
    }

    /// `settings.json`'s source after the import, from `current`. Keep
    /// leaves it as it is, whichever config is in charge; Visual rules only
    /// turns the config in charge off. While visual rules are in charge,
    /// the file just imported is dismissed from the Rules screen's "Found
    /// …" notice.
    pub fn source_after(&self, current: &SourceSettings) -> SourceSettings {
        let mut next = current.clone();
        if self.stops_config() {
            next.visual_only = true;
            next.config = None;
        }
        if next.visual_only && !next.dismissed.iter().any(|d| same_file(d, &self.path)) {
            next.dismissed.push(self.path.clone());
        }
        next
    }

    /// What the router reads after the import, for the Rules screen's
    /// banner.
    pub fn outcome(&self) -> String {
        match (self.final_source(), &self.charge) {
            (SourceChoice::KeepConfig, Charge::Other { shown, .. }) => {
                format!("{shown} stays in charge and runs first.")
            }
            (SourceChoice::KeepConfig, _) => format!(
                "{} stays in charge for its {} and runs first.",
                self.shown,
                plural(self.left_behind().len(), "JS rule")
            ),
            (SourceChoice::VisualOnly, Charge::NoConfig) => {
                "The router reads visual rules only, as before.".to_owned()
            }
            (SourceChoice::VisualOnly, _) => "The router reads visual rules only now.".to_owned(),
        }
    }

    /// The step after this one, skipping those that are not a choice.
    /// `None`: the next thing is Import.
    pub fn next_step(&self) -> Option<Step> {
        match self.step {
            Step::Review if self.place_needed() => Some(Step::Place),
            Step::Review | Step::Place if self.source_needed() => Some(Step::Source),
            _ => None,
        }
    }

    /// The step before this one, skipping those that are not a choice.
    pub fn previous_step(&self) -> Option<Step> {
        match self.step {
            Step::Source if self.place_needed() => Some(Step::Place),
            Step::Source | Step::Place => Some(Step::Review),
            Step::Review => None,
        }
    }

    /// Imported handlers already in `base`, by their position there:
    /// `(item, handler index in base)`.
    pub fn duplicates(&self, base: &RulesFile) -> Vec<(usize, usize)> {
        let mut found = Vec::new();
        for (i, item) in self.items.iter().enumerate() {
            if !item.checked {
                continue;
            }
            let at = match item.visual() {
                Some(Visual::Handler(rule)) => base.rules.iter().position(|r| same_rule(r, &rule)),
                Some(Visual::Rewrite(rewrite)) => base.rewrites.iter().position(|r| {
                    r.enabled && r.patterns == rewrite.patterns && r.actions == rewrite.actions
                }),
                _ => None,
            };
            if let Some(at) = at {
                found.push((i, at));
            }
        }
        found
    }

    /// The file an import writes over `base`: imported rewrites after the
    /// file's own (every rewrite runs anyway), imported handlers where
    /// Place says, in the config's order, and the default. A rule the file
    /// already holds is skipped, except when the import replaces the
    /// file's handlers.
    pub fn plan(&self, base: &RulesFile) -> Plan {
        let mut file = base.clone();
        let duplicates = self.duplicates(base);
        let is_duplicate = |i: usize| duplicates.iter().any(|(item, _)| *item == i);
        let mut rewrites = Vec::new();
        let mut handlers = Vec::new();
        let mut default = None;
        let mut suggestions = 0;
        let mut skipped = 0;
        for (i, item) in self.items.iter().enumerate() {
            if !item.checked {
                continue;
            }
            match item.visual() {
                Some(Visual::Rewrite(_)) if is_duplicate(i) => skipped += 1,
                Some(Visual::Rewrite(rewrite)) => rewrites.push(rewrite),
                Some(Visual::Handler(_))
                    if is_duplicate(i) && self.placement != Placement::Replace =>
                {
                    skipped += 1;
                }
                Some(Visual::Handler(rule)) => {
                    if item.label() == Label::Suggestion {
                        suggestions += 1;
                    }
                    handlers.push(rule);
                }
                Some(Visual::Default { browser, profile }) => default = Some((browser, profile)),
                None => {}
            }
        }
        let counts = (rewrites.len(), handlers.len());
        file.rewrites.extend(rewrites);
        file.rules = match self.placement {
            Placement::Before => handlers.into_iter().chain(base.rules.clone()).collect(),
            Placement::After => base.rules.iter().cloned().chain(handlers).collect(),
            Placement::Replace => handlers,
        };
        if let Some((browser, profile)) = &default {
            file.default_browser.clone_from(browser);
            file.default_profile.clone_from(profile);
        }
        Plan {
            file,
            rewrites: counts.0,
            handlers: counts.1,
            default: default.is_some(),
            duplicates: skipped,
            suggestions,
        }
    }

    /// The footer of Place and Choose source: "5 rules: 1 rewrite, 3
    /// handlers before yours, the default. 1 duplicate skipped."
    pub fn plan_summary(&self, plan: &Plan) -> String {
        let mut parts = Vec::new();
        if plan.rewrites > 0 {
            parts.push(plural(plan.rewrites, "rewrite"));
        }
        if plan.handlers > 0 {
            let place = if self.existing.rules.is_empty() {
                ""
            } else {
                match self.placement {
                    Placement::Before => " before yours",
                    Placement::After => " after yours",
                    Placement::Replace => " instead of yours",
                }
            };
            parts.push(format!("{}{place}", plural(plan.handlers, "handler")));
        }
        if plan.default {
            parts.push("the default".to_owned());
        }
        let mut text = format!("{}: {}.", plural(plan.count(), "rule"), parts.join(", "));
        if plan.count() == 0 {
            text = "Nothing to import.".to_owned();
        }
        if plan.duplicates > 0 {
            text.push_str(&format!(
                " {} skipped.",
                plural(plan.duplicates, "duplicate")
            ));
        }
        text
    }

    /// "Import 5 rules"
    pub fn import_label(&self, plan: &Plan) -> String {
        format!("Import {}", plural(plan.count(), "rule"))
    }
}

fn item(kind: Kind, row: Rule, reviewed: pwrroute_engine::Reviewed) -> Item {
    Item {
        kind,
        row,
        checked: matches!(reviewed.verdict, Verdict::Imports(_)),
        verdict: reviewed.verdict,
        source: reviewed.source,
    }
}

/// Two handlers that route the same links the same way.
fn same_rule(a: &pwrroute_engine::Rule, b: &pwrroute_engine::Rule) -> bool {
    a.enabled
        && a.opener.is_empty()
        && a.patterns == b.patterns
        && a.browser.eq_ignore_ascii_case(&b.browser)
        && a.profile == b.profile
}

/// A visual rule as its row draws.
pub fn visual_row(visual: &Visual) -> Rule {
    match visual {
        Visual::Rewrite(rewrite) => rules_model::rewrite(rewrite, RowBadge::None, Vec::new()).rule,
        Visual::Handler(rule) => rules_model::handler(rule, RowBadge::None, Vec::new()).rule,
        Visual::Default { browser, profile } => Rule::plain(
            Vec::new(),
            Destination::Browser {
                badge: badge_for(browser),
                name: browser.clone().into(),
                profile: (!profile.is_empty()).then(|| profile.clone().into()),
            },
        ),
    }
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

fn shorten(text: &str, max: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match line.char_indices().nth(max) {
        Some((cut, _)) => format!("{}\u{2026}", &line[..cut]),
        None => line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pwrroute_engine::{Rewrite, RewriteAction};

    /// The design's config (Finicky Importer 2a), from the fixtures.
    fn fixture_config() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/importer/finicky.ts")
    }

    fn handler(patterns: &[&str], browser: &str, profile: &str) -> pwrroute_engine::Rule {
        pwrroute_engine::Rule {
            patterns: patterns.iter().map(|p| (*p).to_owned()).collect(),
            browser: browser.into(),
            profile: profile.into(),
            ..pwrroute_engine::Rule::default()
        }
    }

    /// The visual rules of the design's Place step: two handlers, one of
    /// them the config's bsky.app rule.
    fn existing() -> RulesFile {
        RulesFile {
            default_browser: "Google Chrome".into(),
            rules: vec![
                handler(&["localhost:*"], "Arc", ""),
                handler(&["bsky.app/*"], "Firefox", ""),
            ],
            ..RulesFile::default()
        }
    }

    /// The fixture config, read while it is the config in charge.
    fn model(existing: RulesFile) -> ImportModel {
        let config = fixture_config();
        ImportModel::read(&config, None, Some(&config), &[], existing)
    }

    #[test]
    fn review_counts_each_verdict_and_selects_the_exact_imports() {
        let model = model(RulesFile::default());
        assert!(model.problem.is_none());
        assert_eq!(model.found(), 9);
        assert_eq!(model.counts(), (5, 3, 1));
        assert_eq!(
            model.counts_label(),
            "5 import \u{00b7} 3 stay JS \u{00b7} 1 suggestion"
        );
        assert_eq!(model.checked(), 5);
        assert_eq!(
            model.review_footer(),
            "5 of 9 selected. Nothing is written yet."
        );
        let labels: Vec<Label> = model.items.iter().map(Item::label).collect();
        assert_eq!(
            labels,
            [
                Label::Imports,
                Label::StaysJs,
                Label::StaysJs,
                Label::Suggestion,
                Label::Imports,
                Label::StaysJs,
                Label::Imports,
                Label::Imports,
                Label::Imports,
            ]
        );
        // Every rule found its lines in the file.
        assert!(model.items.iter().all(|item| item.source.is_some()));
    }

    #[test]
    fn a_suggestion_is_off_until_ticked_and_a_js_rule_cannot_be() {
        let mut model = model(RulesFile::default());
        let meet = 3;
        assert!(!model.items[meet].checked);
        model.toggle(meet);
        assert_eq!(model.checked(), 6);
        model.toggle(2);
        assert!(!model.items[2].checked, "a rule that stays JS has no box");
        // The header's box leaves the suggestion as the reader set it.
        model.set_all(false);
        assert_eq!(model.checked(), 1);
        model.set_all(true);
        assert_eq!(model.checked(), 6);
        assert!(model.all_checked());
        let plan = model.plan(&RulesFile::default());
        assert_eq!(plan.suggestions, 1);
        assert_eq!(
            plan.file.rules[0],
            handler(&["meet.google.com/*"], "Firefox", "default-release")
        );
    }

    #[test]
    fn place_orders_the_imported_handlers_as_a_group() {
        let mut model = model(existing());
        assert!(model.place_needed());
        let imported = [
            handler(&["google.com/*", "*.google.com*"], "Google Chrome", ""),
            handler(&["*.atlassian.net/*"], "Google Chrome", "Work"),
        ];
        let patterns = |file: &RulesFile| -> Vec<String> {
            file.rules.iter().map(|r| r.patterns[0].clone()).collect()
        };

        // Before: the config's handlers win, in their own order; the
        // bsky.app rule the file already has is skipped, not doubled.
        let plan = model.plan(&model.existing);
        assert_eq!(plan.duplicates, 1);
        assert_eq!(
            patterns(&plan.file),
            [
                "google.com/*",
                "*.atlassian.net/*",
                "localhost:*",
                "bsky.app/*"
            ]
        );
        assert_eq!(plan.file.rules[..2], imported);
        assert_eq!(
            model.plan_summary(&plan),
            "4 rules: 1 rewrite, 2 handlers before yours, the default. 1 duplicate skipped."
        );

        model.placement = Placement::After;
        let plan = model.plan(&model.existing);
        assert_eq!(
            patterns(&plan.file),
            [
                "localhost:*",
                "bsky.app/*",
                "google.com/*",
                "*.atlassian.net/*"
            ]
        );

        // Replace: the file's handlers go, so nothing is a duplicate.
        model.placement = Placement::Replace;
        let plan = model.plan(&model.existing);
        assert_eq!(plan.duplicates, 0);
        assert_eq!(
            patterns(&plan.file),
            ["bsky.app/*", "google.com/*", "*.atlassian.net/*"]
        );

        // Rewrites go after the file's own.
        let mut base = existing();
        base.rewrites.push(Rewrite {
            patterns: vec!["mine.example/*".into()],
            actions: vec![RewriteAction::SetHost("other.example".into())],
            ..Rewrite::default()
        });
        let plan = model.plan(&base);
        assert_eq!(plan.file.rewrites.len(), 2);
        assert_eq!(plan.file.rewrites[1].patterns, ["x.com/*"]);
    }

    #[test]
    fn place_is_skipped_without_visual_handlers() {
        let model = model(RulesFile::default());
        assert!(!model.place_needed());
        assert_eq!(model.next_step(), Some(Step::Source));
    }

    #[test]
    fn choose_source_lists_what_stops_and_is_skipped_when_nothing_stays() {
        let mut model = model(existing());
        model.step = Step::Place;
        assert_eq!(model.next_step(), Some(Step::Source));
        assert_eq!(model.final_source(), SourceChoice::KeepConfig);
        let behind: Vec<String> = model.left_behind().iter().map(|i| i.describe()).collect();
        assert_eq!(behind.len(), 4);
        assert!(behind[0].starts_with("line "), "{}", behind[0]);
        assert!(
            behind
                .iter()
                .any(|line| line.ends_with("(suggestion not taken)"))
        );
        model.source = SourceChoice::VisualOnly;
        assert_eq!(model.final_source(), SourceChoice::VisualOnly);

        // A config with nothing left in JS needs no choice: visual rules.
        model.items.retain(|item| item.label() == Label::Imports);
        model.source = SourceChoice::KeepConfig;
        assert!(!model.source_needed());
        assert_eq!(model.next_step(), None);
        assert_eq!(model.final_source(), SourceChoice::VisualOnly);
    }

    #[test]
    fn importing_the_config_in_charge_changes_the_source_only_for_visual_rules() {
        let mut model = model(RulesFile::default());
        assert_eq!(model.charge, Charge::This);
        let current = SourceSettings::default();
        // Keep: settings.json's source is left as it is.
        assert_eq!(model.final_source(), SourceChoice::KeepConfig);
        assert!(!model.stops_config());
        assert_eq!(model.source_after(&current), current);
        assert!(
            model
                .outcome()
                .ends_with(" stays in charge for its 4 JS rules and runs first."),
            "{}",
            model.outcome()
        );
        // Visual rules only: the config goes, and its notice with it.
        model.source = SourceChoice::VisualOnly;
        let after = model.source_after(&current);
        assert!(after.visual_only);
        assert_eq!(after.config, None);
        assert_eq!(after.dismissed, [fixture_config()]);
    }

    #[test]
    fn importing_a_file_not_in_charge_leaves_the_config_in_charge() {
        let config = fixture_config();
        let other = config.with_file_name("in-charge.js");
        let home = config.parent().unwrap().to_path_buf();
        let current = SourceSettings {
            config: Some(other.clone()),
            ..SourceSettings::default()
        };
        let mut model = ImportModel::read(&config, Some(&home), Some(&other), &[], existing());
        assert_eq!(
            model.charge,
            Charge::Other {
                path: other.clone(),
                shown: "~/in-charge.js".to_owned()
            }
        );
        // Every rule ticked: Choose source is still asked, since only the
        // reader turns the config in charge off, and Keep is the default.
        model.items.retain(|item| item.label() == Label::Imports);
        assert!(model.source_needed());
        model.step = Step::Place;
        assert_eq!(model.next_step(), Some(Step::Source));
        assert_eq!(model.final_source(), SourceChoice::KeepConfig);
        assert!(!model.stops_config());
        assert_eq!(model.source_after(&current), current);
        assert_eq!(
            model.outcome(),
            "~/in-charge.js stays in charge and runs first."
        );
        // Visual rules only turns the one in charge off.
        model.source = SourceChoice::VisualOnly;
        assert!(model.stops_config());
        let after = model.source_after(&current);
        assert!(after.visual_only);
        assert_eq!(after.config, None);
    }

    #[test]
    fn with_no_config_in_charge_an_import_adds_rules_and_nothing_else() {
        let mut model = ImportModel::read(&fixture_config(), None, None, &[], RulesFile::default());
        assert_eq!(model.charge, Charge::NoConfig);
        assert!(!model.source_needed(), "nothing is in charge to switch");
        model.step = Step::Review;
        assert_eq!(model.next_step(), None);
        assert!(!model.stops_config());
        // Discovery stays on: a config added later is still found.
        let discovering = SourceSettings::default();
        assert_eq!(model.source_after(&discovering), discovering);
        // Visual rules were already in charge (1a): the notice for the
        // file just imported does not come back.
        let visual = SourceSettings {
            visual_only: true,
            ..SourceSettings::default()
        };
        let after = model.source_after(&visual);
        assert!(after.visual_only);
        assert_eq!(after.dismissed, [fixture_config()]);
        assert_eq!(
            model.outcome(),
            "The router reads visual rules only, as before."
        );
    }

    #[test]
    fn first_run_goes_straight_to_choose_source_with_every_proven_rule() {
        let mut model = model(RulesFile::default());
        model.set_all(false);
        model.start_at_source();
        assert_eq!(model.step, Step::Source);
        assert_eq!(model.checked(), 5);
        assert_eq!(model.previous_step(), Some(Step::Review));
    }

    #[test]
    fn a_config_that_does_not_load_has_nothing_to_review() {
        let missing = fixture_config().with_file_name("missing.ts");
        let model = ImportModel::read(&missing, None, None, &[], RulesFile::default());
        assert!(model.problem.is_some());
        assert_eq!(model.found(), 0);
    }
}
