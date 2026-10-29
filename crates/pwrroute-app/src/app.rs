//! The main window's view: its state, navigation, keyboard handling and
//! layout.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use gpui_kit::{
    App, ClipboardItem, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, Render, Styled as _, Subscription, Window, div,
};

use pwrroute_engine::{Description, Engine, Limits, NativeHost, discovery};

use crate::activity::Activity;
use crate::activity_actions::{self, Actions, BrowserChoice};
use crate::build_info::{BuildInfo, Part};
use crate::config_view::{self, ConfigInCharge};
use crate::editor::{Field, Rail};
use crate::importer::ImportModel;
use crate::importer_view::{FoundConfig, ImportDone};
use crate::menu_bar::MenuBar;
use crate::overview::OverviewState;
use crate::overview_model::Installed;
use crate::rules_model::{self, Handler, Rewrite};
use crate::rules_store::{Disk, RulesStore};
use crate::settings_store::SourceSettings;
use crate::test_link::TestLink;
use crate::text_field::{Edit, TextField};
use crate::try_it::{Probe, TestRuns};
use crate::{activity, chrome, e2e, editor, overview, router, rules_view, test_link, theme};

/// The element id of the window's root, for tests and the inspector.
pub const ROOT_ID: &str = "pwrroute-root";

/// How long after the last edit the file is written: long enough to take a
/// typed word in one write, short enough that the router follows along.
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(400);

/// How long a copied row of the rail's build block shows its check.
pub const COPIED_FOR: Duration = Duration::from_millis(1500);

/// The rail's sections. Overview is where every open lands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Screen {
    #[default]
    Overview,
    Rules,
    TestLink,
    Activity,
}

impl Screen {
    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Rules => "Rules",
            Self::TestLink => "Test a link",
            Self::Activity => "Activity",
        }
    }
}

/// The rule the editor rail shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Selection {
    Rewrite(usize),
    Handler(usize),
}

pub struct PwrRoute {
    pub focus: FocusHandle,
    /// The strip's menu bar off macOS (`menu_bar.rs`).
    pub menu_bar: Entity<MenuBar>,
    pub screen: Screen,
    /// `rules.json`, read on open and written after every edit.
    pub store: RulesStore,
    /// The home directory, for paths shown as `~/…`.
    pub home: Option<PathBuf>,
    /// The rows of the rewrites card, as the store stands.
    pub rewrites: Vec<Rewrite>,
    /// The rows of the handlers card.
    pub handlers: Vec<Handler>,
    /// `None` while there is no rule to select.
    pub selection: Option<Selection>,
    /// The editor's fields for the selected rule.
    pub rail: Option<Rail>,
    /// Try it's engine over the rules as edited, which Activity's Add a
    /// rule sheet also asks about collisions.
    pub(crate) probe: Probe,
    /// The file's examples and tests, run on every save.
    pub tests: TestRuns,
    /// A Finicky config is in charge: the Rules screen is option 1b.
    pub config: Option<ConfigInCharge>,
    /// 1b's folded visual rules are open.
    pub visual_rules_expanded: bool,
    /// The draft handler a surface other than Rules started, for the
    /// note above its fields: its index and where it came from.
    pub draft_from: Option<(usize, crate::editor::DraftFrom)>,
    /// The link the draft handler at this index started from, for Start
    /// from (`start_from.rs`); `None` once cleared, or with no link.
    pub draft_link: Option<(usize, Option<crate::start_from::StartLink>)>,
    pub test: TestLink,
    pub activity: Activity,
    /// Activity's row actions: the Add a rule sheet and what the last
    /// action did.
    pub actions: Actions,
    /// The browsers the sheet offers, with their profiles: the app's one
    /// scan (`browser_scan`), less the apps hidden in Settings › Browsers;
    /// a test or screenshot sets its own.
    pub browsers: Vec<BrowserChoice>,
    /// The last Finicky import, for its banner and Undo import, until the
    /// window closes.
    pub last_import: Option<ImportDone>,
    /// A config discovery found while visual rules are in charge, whose
    /// notice was not dismissed (Finicky Importer 1a).
    pub found_config: Option<FoundConfig>,
    /// The Rules rows' menus, the deleted rule's Undo and the drag
    /// (`rule_menu.rs`).
    pub row_actions: crate::rule_menu::RowActions,
    /// The Overview's hover, pin, callout and setup state.
    pub overview: OverviewState,
    /// The browsers the scan found, for the Overview's cards (a test or
    /// screenshot sets its own; empty, no target is drawn as missing).
    pub installed: Vec<Installed>,
    /// What the config in charge says, for the Overview's map: `None`
    /// while none is in charge or it did not load.
    pub described: Option<Description>,
    /// When this window last read the config in charge, for the rail
    /// foot's "reloaded".
    pub config_loaded: SystemTime,
    /// Which build this is, for the rail's build block (`build_info.rs`);
    /// a test or screenshot sets its own.
    pub build: BuildInfo,
    /// The build block's row that was just copied, and the copy's number:
    /// its check shows until [`COPIED_FOR`] passes with no later copy.
    pub build_copied: Option<(Part, u64)>,
    /// A debounced save is on its way.
    save_scheduled: bool,
    /// Pausing from the tray redraws the Overview.
    _tray: Option<Subscription>,
}

impl PwrRoute {
    /// The window for the config the router chose, if any, over the real
    /// `rules.json`.
    pub fn new(cx: &mut Context<'_, Self>) -> Self {
        let store = RulesStore::load(discovery::rules_path(&discovery::Environment::current()));
        let mut this =
            Self::with_store(router::config_in_charge(), std::env::home_dir(), store, cx);
        // The one scan the app makes, less the apps hidden in Settings.
        this.browsers = crate::browser_scan::choices(cx);
        this.installed = crate::browser_scan::installed(cx);
        this
    }

    /// The window as it is with `config` in charge (`None`: the visual
    /// rules) over `store`. `home` shortens paths to `~/…`.
    pub fn with_store(
        config: Option<PathBuf>,
        home: Option<PathBuf>,
        store: RulesStore,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        // One engine for the config: the Rules screen describes it and Test a
        // link resolves with it, so the config is evaluated once here. Test
        // a link's runs see the OS's answers (modifier keys, running apps)
        // and its console output, as the router's do.
        let engine = config
            .as_deref()
            .map(|path| Engine::load(path, Rc::new(NativeHost::new()), Limits::default()));
        let in_charge = config
            .as_deref()
            .zip(engine.as_ref())
            .map(|(path, engine)| ConfigInCharge::new(path, home.as_deref(), engine.as_ref()));
        let described = described(engine.as_ref());
        // Pausing and resuming from the tray changes the Overview's hero.
        let tray = crate::tray::model(cx).map(|model| cx.observe(&model, |_, _, cx| cx.notify()));
        // Rules opens as a list: the editor appears on a click (or Down).
        let mut this = Self {
            focus: cx.focus_handle(),
            menu_bar: MenuBar::entity("PwrRoute", cx),
            screen: Screen::Overview,
            rewrites: store.rewrites(),
            handlers: store.handlers(),
            selection: None,
            rail: None,
            probe: Probe::at_home(config.clone(), home.as_deref(), store.edited()),
            tests: TestRuns::new(config.clone(), store.edited()),
            test: TestLink::new(
                config,
                home.as_deref(),
                engine.and_then(Result::ok),
                store.written(),
                cx,
            ),
            store,
            home,
            config: in_charge,
            visual_rules_expanded: false,
            draft_from: None,
            draft_link: None,
            activity: Activity::new(cx),
            actions: Actions::default(),
            browsers: Vec::new(),
            last_import: None,
            found_config: None,
            row_actions: crate::rule_menu::RowActions::default(),
            overview: OverviewState::default(),
            installed: Vec::new(),
            described,
            config_loaded: SystemTime::now(),
            build: BuildInfo::current(),
            build_copied: None,
            save_scheduled: false,
            _tray: tray,
        };
        if this.config.is_none() {
            this.found_config = crate::importer_view::found_config(this.home.as_deref(), cx);
        }
        let shown = this.config.as_ref().map(|config| config.path.clone());
        this.activity.set_config(shown, cx);
        this.rebuild_rail(cx);
        // The file's examples and tests, as it was opened.
        if this.tests.counts() != (0, 0) {
            this.tests.run();
        }
        this
    }

    /// Shows a screen. Test a link runs its sample the first time and takes
    /// the keyboard; the others give it to the window.
    pub fn navigate(&mut self, screen: Screen, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.screen == Screen::Overview && screen != Screen::Overview {
            self.overview.leave();
        }
        self.screen = screen;
        match screen {
            Screen::TestLink => {
                // Test a link tests the file as it is.
                self.sync_with_disk(cx);
                if !self.test.has_run() {
                    self.test.run();
                }
                window.focus(&self.test.url.focus, cx);
            }
            Screen::Overview | Screen::Rules | Screen::Activity => window.focus(&self.focus, cx),
        }
        cx.notify();
    }

    /// The editor shows a rule: the visual rules are on screen and one of
    /// them is selected.
    pub fn editing(&self) -> bool {
        self.screen == Screen::Rules
            && (self.config.is_none() || self.visual_rules_expanded)
            && self.selection.is_some()
    }

    /// No file and nothing to route by: the design's "no configuration"
    /// state rather than two empty cards.
    pub fn nothing_to_route_by(&self) -> bool {
        self.config.is_none() && self.store.is_empty() && !self.store.exists()
    }

    /// Every row in list order: rewrites, then handlers.
    fn order(&self) -> Vec<Selection> {
        (0..self.rewrites.len())
            .map(Selection::Rewrite)
            .chain((0..self.handlers.len()).map(Selection::Handler))
            .collect()
    }

    /// Select a row; one that is not in the lists is ignored.
    pub fn select(&mut self, selection: Selection, cx: &mut Context<'_, Self>) {
        if Some(selection) != self.selection && self.order().contains(&selection) {
            self.selection = Some(selection);
            self.rebuild_rail(cx);
            cx.notify();
        }
    }

    /// Moves the selection `by` rows through both lists, clamped at the ends.
    fn step(&mut self, by: isize, cx: &mut Context<'_, Self>) {
        let order = self.order();
        let Some(at) = order.iter().position(|s| Some(*s) == self.selection) else {
            return;
        };
        let to = at
            .saturating_add_signed(by)
            .min(order.len().saturating_sub(1));
        self.select(order[to], cx);
    }

    // --- edits -----------------------------------------------------------

    /// The store changed: redraw the rows, hand Test a link, Try it and the
    /// test runs the new rules, and write the file after the debounce. The
    /// rail's fields are kept, so typing keeps its focus. The file is
    /// checked first: an edit over a file another writer changed is kept
    /// on screen and not saved, and the status says so.
    pub(crate) fn refresh(&mut self, cx: &mut Context<'_, Self>) {
        // Undo import and Undo delete are good until the next edit.
        if let Some(done) = &mut self.last_import {
            done.undoable = false;
        }
        self.row_actions.edited();
        let disk = self.store.check_disk();
        self.rewrites = self.store.rewrites();
        self.handlers = self.store.handlers();
        let order = self.order();
        // A selected row that is gone gives way to the last; no selection
        // stays none.
        self.selection = match self.selection {
            Some(selection) if !order.contains(&selection) => order.last().copied(),
            selection => selection,
        };
        self.test.set_rules(self.store.written());
        self.probe.set_rules(self.store.edited());
        self.tests.set_rules(self.store.edited());
        if disk == Disk::Reloaded {
            // The rail's fields held a rule that is gone: nothing was
            // unsaved, so they are rebuilt from the file.
            self.rebuild_rail(cx);
        }
        self.try_it();
        if self.store.needs_save() {
            self.schedule_save(cx);
        }
        cx.notify();
    }

    // --- the file's other writers ------------------------------------------

    /// Follows `rules.json` when something else wrote it: an agent's
    /// `pwrroute_rules_apply`, an editor, a sync. Called when the window
    /// comes to the front, on every dispatch, after an in-process agent
    /// write, and before the Add a rule sheet and Test a link read the
    /// store. A changed file is read again while nothing is unsaved; under
    /// unsaved edits the rail foot shows Reload and Overwrite instead.
    pub fn sync_with_disk(&mut self, cx: &mut Context<'_, Self>) -> Disk {
        let disk = self.store.check_disk();
        match disk {
            Disk::Unchanged => {}
            Disk::Reloaded => {
                // Another writer changed the file: the import is no longer
                // what Undo would take back.
                if let Some(done) = &mut self.last_import {
                    done.undoable = false;
                }
                self.reloaded(cx);
            }
            Disk::Changed => cx.notify(),
        }
        disk
    }

    /// Reload: drops every unsaved edit and reads the file again.
    pub fn reload_rules(&mut self, cx: &mut Context<'_, Self>) {
        self.store.reload();
        self.reloaded(cx);
    }

    /// Overwrite: writes the edits over the file as it now is.
    pub fn overwrite_rules(&mut self, cx: &mut Context<'_, Self>) {
        // The status carries the error; nothing else reports it.
        let _ = self.store.overwrite();
        self.rewrites = self.store.rewrites();
        self.handlers = self.store.handlers();
        self.run_tests(cx);
    }

    // --- the Finicky importer ----------------------------------------------

    /// Writes an import into `rules.json` in one save, over the file as it
    /// stands, and keeps the file and `source` as they were for Undo
    /// import. A save the store would refuse (the file changed under
    /// unsaved edits, or did not load) leaves the rules untouched and comes
    /// back as the reason, for the importer to show.
    pub fn apply_import(
        &mut self,
        model: &ImportModel,
        previous_source: Option<SourceSettings>,
        cx: &mut Context<'_, Self>,
    ) -> Result<(), String> {
        self.sync_with_disk(cx);
        if let Some(reason) = self.store.save_refused() {
            return Err(reason);
        }
        let previous = self.store.written();
        let plan = model.plan(&previous);
        self.store.replace_all(plan.file.clone());
        let saved = self.store.save();
        self.reloaded(cx);
        // A write that failed leaves the edit on screen with the reason, as
        // any other failed save does.
        saved?;
        self.found_config = None;
        self.last_import = Some(ImportDone {
            shown: model.shown.clone(),
            file: model.path.clone(),
            count: plan.count(),
            suggestions: plan.suggestions,
            stays: model.left_behind().len(),
            outcome: model.outcome(),
            at: std::time::SystemTime::now(),
            previous,
            previous_source,
            undoable: true,
        });
        cx.notify();
        Ok(())
    }

    /// Undo import: `rules.json` as it was before, in one save. Returns the
    /// source to restore once that save went through. Another writer's
    /// change since the import ends Undo instead of being overwritten.
    pub fn undo_import(&mut self, cx: &mut Context<'_, Self>) -> Option<SourceSettings> {
        let disk = self.sync_with_disk(cx);
        let undoable = self.last_import.as_ref()?.undoable;
        if !undoable || disk != Disk::Unchanged || self.store.save_refused().is_some() {
            if let Some(done) = &mut self.last_import {
                done.undoable = false;
            }
            cx.notify();
            return None;
        }
        let done = self.last_import.take()?;
        self.store.replace_all(done.previous);
        let saved = self.store.save();
        self.reloaded(cx);
        saved.ok().and(done.previous_source)
    }

    /// The config in charge changed (the importer's Choose source, or its
    /// undo): the Rules screen, Test a link and the test runs follow it.
    pub fn set_config(&mut self, config: Option<PathBuf>, cx: &mut Context<'_, Self>) {
        let engine = config
            .as_deref()
            .map(|path| Engine::load(path, Rc::new(NativeHost::new()), Limits::default()));
        self.config = config
            .as_deref()
            .zip(engine.as_ref())
            .map(|(path, engine)| ConfigInCharge::new(path, self.home.as_deref(), engine.as_ref()));
        self.visual_rules_expanded = false;
        self.described = described(engine.as_ref());
        self.config_loaded = SystemTime::now();
        self.probe = Probe::at_home(config.clone(), self.home.as_deref(), self.store.edited());
        self.tests = TestRuns::new(config.clone(), self.store.edited());
        self.test = TestLink::new(
            config,
            self.home.as_deref(),
            engine.and_then(Result::ok),
            self.store.written(),
            cx,
        );
        if self.tests.counts() != (0, 0) {
            self.tests.run();
        }
        let shown = self.config.as_ref().map(|config| config.path.clone());
        self.activity.set_config(shown, cx);
        self.try_it();
        cx.notify();
    }

    /// The store holds the file again: the rows, the selection (cleared
    /// when its row is gone), the rail, and every engine over the rules.
    fn reloaded(&mut self, cx: &mut Context<'_, Self>) {
        // The rows may be other rules now: a menu, a drag or an Undo for
        // one of them would land on the wrong one.
        self.row_actions = crate::rule_menu::RowActions::default();
        self.rewrites = self.store.rewrites();
        self.handlers = self.store.handlers();
        let order = self.order();
        self.selection = self.selection.filter(|s| order.contains(s));
        self.rebuild_rail(cx);
        self.test.set_rules(self.store.written());
        self.probe.set_rules(self.store.edited());
        self.tests.set_rules(self.store.edited());
        if self.tests.counts() != (0, 0) {
            self.tests.run();
        }
        // The Overview's rows are by number, and the file's may now be
        // other rules: what the pointer or the keyboard was on lets go.
        self.overview.forget_pointer();
        self.overview.key(None);
        if matches!(
            self.overview.pinned,
            Some(crate::overview_model::FocusKey::Row(_))
        ) {
            self.overview.pin(None);
        }
        cx.notify();
    }

    /// Checks the file each time the window comes to the front: an editor
    /// or a sync changed it while something else had the screen.
    pub fn watch_activation(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.sync_with_disk(cx);
            }
        })
        .detach();
    }

    /// The selected rule's editor, fresh from the store, and its Try it.
    pub(crate) fn rebuild_rail(&mut self, cx: &mut Context<'_, Self>) {
        self.rail = None;
        self.rebuild_rail_kept(cx);
    }

    /// The same, after an edit that added or removed one of the selected
    /// rule's fields: what the person chose to type (Open in, Profile, a
    /// Try it link) stays. Only for the rule the rail was built for; a
    /// delete, an insert or a reload can put another rule at its index, so
    /// those use [`Self::rebuild_rail`].
    pub(crate) fn rebuild_rail_kept(&mut self, cx: &mut Context<'_, Self>) {
        let old = self.rail.take();
        self.rail = match self.selection {
            Some(Selection::Handler(i)) => self.handlers.get(i).map(|h| Rail::for_handler(h, cx)),
            Some(Selection::Rewrite(i)) => self.rewrites.get(i).map(|r| Rail::for_rewrite(r, cx)),
            None => None,
        };
        if let Some(rail) = &mut self.rail {
            rail.of = self.selection;
            if let Some(old) = old.filter(|old| old.of == self.selection) {
                rail.keep_from(old);
            }
        }
        self.try_it();
    }

    /// Runs the rail's sample through the edited rules.
    pub(crate) fn try_it(&mut self) {
        let Some(selection) = self.selection else {
            return;
        };
        let Some(rail) = &mut self.rail else {
            return;
        };
        let (problems, opener, patterns) = match selection {
            Selection::Handler(i) => {
                let handler = self.handlers.get(i);
                (
                    handler.is_some_and(|h| !h.problems.is_empty()),
                    handler.and_then(|h| h.openers.iter().find(|o| !o.trim().is_empty()).cloned()),
                    handler.map(|h| h.patterns.as_slice()).unwrap_or_default(),
                )
            }
            Selection::Rewrite(i) => {
                let rewrite = self.rewrites.get(i);
                (
                    rewrite.is_some_and(|r| !r.problems.is_empty()),
                    None,
                    rewrite.map(|r| r.patterns.as_slice()).unwrap_or_default(),
                )
            }
        };
        // Until the sample is typed, it follows the first pattern.
        if !rail.sample_edited {
            rail.sample.set(crate::try_it::sample_url(patterns));
        }
        let sample = rail.sample.text.clone();
        rail.try_result = (!problems && !sample.trim().is_empty())
            .then(|| self.probe.run(&sample, opener.as_deref(), selection));
    }

    /// Copies a part of the build (the rail's build block) and shows its
    /// check for [`COPIED_FOR`].
    pub fn copy_build(&mut self, part: Part, cx: &mut Context<'_, Self>) {
        let Some(text) = self.build.copy_text(part) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        let copy = self.build_copied.map_or(0, |(_, copy)| copy + 1);
        self.build_copied = Some((part, copy));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            let _ = this.update(cx, |this, cx| {
                if this.build_copied.is_some_and(|(_, last)| last == copy) {
                    this.build_copied = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Writes the file once nothing has been typed for [`SAVE_DEBOUNCE`].
    fn schedule_save(&mut self, cx: &mut Context<'_, Self>) {
        if self.save_scheduled {
            return;
        }
        self.save_scheduled = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| {
                this.save_scheduled = false;
                this.save_now(cx);
            });
        })
        .detach();
    }

    /// Writes every saveable edit now. The status shows how it went; an
    /// error stays on screen until the next save succeeds.
    pub fn save_now(&mut self, cx: &mut Context<'_, Self>) {
        if self.store.needs_save() {
            // The status carries the error; nothing else reports it.
            let _ = self.store.save();
            // A save turns drafts into saved rules: the badges change.
            self.rewrites = self.store.rewrites();
            self.handlers = self.store.handlers();
        }
        // Tests run on save: every example and test, against the rules as
        // they now stand.
        if self.tests.report().is_none() && self.tests.counts() != (0, 0) {
            self.tests.run();
        }
        cx.notify();
    }

    /// One change the router must see on its next link (the precedence
    /// switches, Overview's default browser), written now over the file as
    /// it stands: another writer's change is read first, a store that would
    /// refuse the save writes nothing, and a write that fails is taken back
    /// (by the undo `change` returns), so the screen keeps showing what the
    /// router has. `Err` is why it was not saved; the selection stays
    /// either way.
    pub fn write_now<U: FnOnce(&mut RulesStore)>(
        &mut self,
        change: impl FnOnce(&mut RulesStore) -> U,
        cx: &mut Context<'_, Self>,
    ) -> Result<(), String> {
        self.sync_with_disk(cx);
        if let Some(reason) = self.store.save_refused() {
            cx.notify();
            return Err(reason);
        }
        let undo = change(&mut self.store);
        let selection = self.selection;
        self.refresh(cx);
        self.selection = selection;
        self.save_now(cx);
        let failed = match self.store.status() {
            crate::rules_store::Status::Failed(reason) => Some(reason.clone()),
            crate::rules_store::Status::Changed => {
                Some(crate::rules_store::CHANGED_ON_DISK.to_owned())
            }
            _ => None,
        };
        if let Some(reason) = failed {
            undo(&mut self.store);
            self.refresh(cx);
            self.selection = selection;
            return Err(reason);
        }
        Ok(())
    }

    /// `rules.json`'s `precedence`, through [`PwrRoute::write_now`]: `next`
    /// is given the precedence the file has once another writer's change
    /// is read.
    pub fn write_precedence(
        &mut self,
        next: impl FnOnce(pwrroute_engine::Precedence) -> pwrroute_engine::Precedence,
        cx: &mut Context<'_, Self>,
    ) -> Result<(), String> {
        self.write_now(
            |store| {
                let previous = store.precedence();
                store.set_precedence(next(previous));
                move |store: &mut RulesStore| store.set_precedence(previous)
            },
            cx,
        )
    }

    /// `rules.json`'s default browser and profile, through
    /// [`PwrRoute::write_now`]: the Overview's setup card and Everything
    /// else's menu.
    pub fn write_default_browser(
        &mut self,
        browser: &str,
        profile: &str,
        cx: &mut Context<'_, Self>,
    ) -> Result<(), String> {
        self.write_now(
            |store| {
                let file = store.edited();
                let (was, was_profile) = (file.default_browser, file.default_profile);
                store.set_default_browser(browser, profile);
                move |store: &mut RulesStore| store.set_default_browser(&was, &was_profile)
            },
            cx,
        )
    }

    /// A new, empty rewrite at the end of its list, selected, with its
    /// first pattern ready to type.
    pub fn add_rewrite(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        // Under a config the visual rules are folded: open them, so the
        // new row is on screen beside its editor.
        self.visual_rules_expanded = true;
        let index = self.store.add_rewrite();
        self.selection = Some(Selection::Rewrite(index));
        self.refresh(cx);
        self.rebuild_rail(cx);
        self.add_pattern(window, cx);
    }

    /// A new handler just above Everything else, selected, started from
    /// the last link the person opened (design: Rule From a Link 1a), or
    /// blank with its first pattern ready to type when there is none.
    pub fn add_handler(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.add_blank_handler(window, cx);
        self.start_from_last(window, cx);
    }

    /// A new handler started from `url`: the Overview's "Add a rule for
    /// this link".
    pub fn add_handler_from(
        &mut self,
        link: crate::start_from::StartLink,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.add_blank_handler(window, cx);
        self.start_from(link, window, cx);
    }

    /// A new, empty handler just above Everything else, selected, with its
    /// first pattern ready to type.
    pub fn add_blank_handler(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.visual_rules_expanded = true;
        self.draft_from = None;
        let index = self.store.add_handler();
        self.draft_link = Some((index, None));
        self.selection = Some(Selection::Handler(index));
        self.refresh(cx);
        self.rebuild_rail(cx);
        self.add_pattern(window, cx);
    }

    /// Test a link's "Add a rule…": a new handler on Rules whose first
    /// pattern is the narrowest `suggest_rule` offers for the tested link,
    /// selected so typing replaces it, and Try it holding the link (design:
    /// Rule Editor Pickers 5a, 5b). Nothing else is preset: the browser is
    /// the person's to choose.
    pub fn add_rule_from_test(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        // From the last run, not the field: rules match the URL after the
        // rewrites, and the field may have changed since.
        let Some((pattern, link)) = self.test.rule_start() else {
            return;
        };
        self.navigate(Screen::Rules, window, cx);
        self.add_blank_handler(window, cx);
        let Some(Selection::Handler(h)) = self.selection else {
            return;
        };
        let url = link.url.clone();
        self.draft_link = Some((h, Some(link)));
        self.store.set_pattern(Selection::Handler(h), 0, &pattern);
        self.refresh(cx);
        self.rebuild_rail_kept(cx);
        if let Some(rail) = &mut self.rail {
            if let Some(field) = rail.patterns.first_mut() {
                field.selected = !field.text.is_empty();
            }
            rail.sample.set(url);
            rail.sample_edited = true;
        }
        self.try_it();
        self.focus_field(window, cx, |rail| rail.patterns.first());
        self.draft_from = Some((h, crate::editor::DraftFrom::TestLink));
        cx.notify();
    }

    /// The draft's note and start link after the handler at `deleted` went:
    /// gone with it, or up one below it.
    pub(crate) fn drafts_after_delete(&mut self, deleted: usize) {
        fn shift<T>(draft: Option<(usize, T)>, deleted: usize) -> Option<(usize, T)> {
            match draft {
                Some((at, _)) if at == deleted => None,
                Some((at, value)) if at > deleted => Some((at - 1, value)),
                kept => kept,
            }
        }
        self.draft_from = shift(self.draft_from.take(), deleted);
        self.draft_link = shift(self.draft_link.take(), deleted);
    }

    /// The same after a handler was inserted at `index`: a draft at or below
    /// it moved down one.
    pub(crate) fn drafts_after_insert(&mut self, index: usize) {
        if let Some((draft, _)) = &mut self.draft_from
            && *draft >= index
        {
            *draft += 1;
        }
        if let Some((draft, _)) = &mut self.draft_link
            && *draft >= index
        {
            *draft += 1;
        }
    }

    /// The same after the handler at `from` moved to `to`.
    pub(crate) fn drafts_after_move(&mut self, from: usize, to: usize) {
        if let Some((draft, _)) = &mut self.draft_from {
            *draft = crate::rule_menu::shifted(*draft, from, to);
        }
        if let Some((draft, _)) = &mut self.draft_link {
            *draft = crate::rule_menu::shifted(*draft, from, to);
        }
    }

    /// Removes the selected rule, leaving Undo in its place; the row that
    /// takes its place is selected.
    pub fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(selection) = self.selection {
            self.delete_rule(selection, window, cx);
        }
    }

    /// Switch the selected rule on or off.
    pub fn toggle_selected(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(selection) = self.selection
            && let Some(enabled) = self.store.enabled(selection)
        {
            self.store.set_enabled(selection, !enabled);
            self.refresh(cx);
        }
    }

    /// Adds an empty pattern to the selected rule and focuses its field.
    pub fn add_pattern(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(selection) = self.selection else {
            return;
        };
        self.store.add_pattern(selection);
        self.refresh(cx);
        self.rebuild_rail_kept(cx);
        self.focus_field(window, cx, |rail| rail.patterns.last());
    }

    pub fn remove_pattern(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(selection) = self.selection else {
            return;
        };
        self.store.remove_pattern(selection, index);
        self.refresh(cx);
        self.rebuild_rail_kept(cx);
        window.focus(&self.focus, cx);
    }

    pub fn remove_opener(&mut self, index: usize, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(Selection::Handler(h)) = self.selection else {
            return;
        };
        self.store.remove_opener(h, index);
        self.refresh(cx);
        self.rebuild_rail_kept(cx);
        window.focus(&self.focus, cx);
    }

    /// Adds a change to the selected rewrite and focuses its value.
    pub fn add_action(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(Selection::Rewrite(r)) = self.selection else {
            return;
        };
        self.store.add_action(r);
        self.refresh(cx);
        self.rebuild_rail_kept(cx);
        self.focus_field(window, cx, |rail| rail.values.last());
    }

    pub fn remove_action(&mut self, index: usize, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(Selection::Rewrite(r)) = self.selection else {
            return;
        };
        self.store.remove_action(r, index);
        self.refresh(cx);
        self.rebuild_rail_kept(cx);
        window.focus(&self.focus, cx);
    }

    /// Changes what change `index` of the selected rewrite does, keeping
    /// its value, and closes the kind menu.
    pub fn set_action_kind(&mut self, index: usize, kind: usize, cx: &mut Context<'_, Self>) {
        let Some(Selection::Rewrite(r)) = self.selection else {
            return;
        };
        let value = self
            .rail
            .as_ref()
            .and_then(|rail| rail.values.get(index))
            .map(|field| field.text.clone())
            .unwrap_or_default();
        self.store
            .set_action(r, index, rules_model::action(kind, &value));
        if let Some(rail) = &mut self.rail {
            rail.picking_kind = None;
        }
        self.refresh(cx);
    }

    pub(crate) fn focus_field(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
        pick: impl FnOnce(&Rail) -> Option<&TextField>,
    ) {
        if let Some(focus) = self.rail.as_ref().and_then(pick).map(|f| f.focus.clone()) {
            window.focus(&focus, cx);
        }
    }

    /// A keystroke in one of the rail's fields. Returns whether the field
    /// took it.
    pub fn rail_key(
        &mut self,
        field: Field,
        event: &KeyDownEvent,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(text_field) = self.rail.as_mut().and_then(|rail| rail.field_mut(field)) else {
            return false;
        };
        match text_field.key(&event.keystroke, cx) {
            Edit::Ignored => false,
            Edit::Handled => true,
            Edit::Changed => {
                let text = text_field.text.clone();
                if field == Field::Sample {
                    if let Some(rail) = &mut self.rail {
                        rail.sample_edited = true;
                    }
                    self.try_it();
                    cx.notify();
                } else {
                    self.apply_field(field, &text);
                    self.refresh(cx);
                }
                true
            }
            Edit::Submit => {
                if field == Field::Sample {
                    self.try_it();
                    cx.notify();
                }
                true
            }
        }
    }

    /// Writes a field's text into the store.
    fn apply_field(&mut self, field: Field, text: &str) {
        let Some(selection) = self.selection else {
            return;
        };
        match (field, selection) {
            (Field::Pattern(i), _) => self.store.set_pattern(selection, i, text),
            (Field::Browser, Selection::Handler(h)) => self.store.set_browser(h, text),
            (Field::Profile, Selection::Handler(h)) => self.store.set_profile(h, text),
            (Field::Value(i), Selection::Rewrite(r)) => {
                let kind = self
                    .rewrites
                    .get(r)
                    .and_then(|rewrite| rewrite.actions.get(i))
                    .map_or(0, rules_model::kind_index);
                self.store.set_action(r, i, rules_model::action(kind, text));
            }
            // The sample is the rail's own; a handler field on a rewrite,
            // or the reverse, cannot happen.
            _ => {}
        }
    }

    fn on_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        // A field in the rail has the keyboard: its keys are its own.
        if !self.focus.is_focused(window) {
            return;
        }
        let key = event.keystroke.key.as_str();
        match self.screen {
            // The map's keyboard: Esc lets go of the pin.
            Screen::Overview => {
                self.overview_key(key, event.keystroke.modifiers.shift, window, cx);
            }
            Screen::Rules if self.config.is_none() || self.visual_rules_expanded => {
                if self.rules_list_key(event, window, cx) {
                    return;
                }
                match (key, self.selection) {
                    // Nothing selected yet: Down selects the first row.
                    ("down", None) => {
                        if let Some(first) = self.order().first().copied() {
                            self.select(first, cx);
                        }
                    }
                    ("up", Some(_)) => self.step(-1, cx),
                    ("down", Some(_)) => self.step(1, cx),
                    _ => {}
                }
            }
            Screen::Activity => {
                self.activity_key(event, window, cx);
            }
            _ => {}
        }
    }
}

/// What a loaded config says about itself, for the Overview's map.
fn described(engine: Option<&anyhow::Result<Engine>>) -> Option<Description> {
    engine
        .and_then(|engine| engine.as_ref().ok())
        .and_then(|engine| engine.describe().ok().flatten())
}

/// Every main window checks the file now: an in-process agent write
/// landed, so the Rules screen reloads at once rather than on the next
/// activation or dispatch.
pub fn sync_rules_windows(cx: &mut App) {
    for window in cx.windows() {
        if let Some(window) = window.downcast::<PwrRoute>() {
            let _ = window.update(cx, |this, _, cx| {
                this.sync_with_disk(cx);
            });
        }
    }
}

impl Render for PwrRoute {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        // One reading of the routing state for the strip, the rail foot and
        // the Overview, so they agree.
        let status = overview::status(self, cx);
        let pane = match (self.screen, &self.config) {
            (Screen::Overview, _) => overview::pane(self, &status, window, cx).into_any_element(),
            (Screen::Rules, None) => rules_view::pane(self, cx).into_any_element(),
            (Screen::Rules, Some(config)) => config_view::pane(self, config, cx).into_any_element(),
            (Screen::TestLink, _) => test_link::pane(self, window, cx).into_any_element(),
            (Screen::Activity, _) => activity::pane(self, window, cx).into_any_element(),
        };
        let rail = self.editing().then(|| editor::rail(self, window, cx));
        // Activity's Add a rule sheet, over everything.
        let sheet = activity_actions::sheet(self, window, cx);
        div()
            .on_children_prepainted(|bounds, window, cx| {
                e2e::frame(window);
                e2e::regions(&["titleStrip", "body"])(bounds, window, cx);
            })
            .id(ROOT_ID)
            .debug_selector(|| ROOT_ID.into())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.on_key_down(event, window, cx);
            }))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::bg_app())
            .text_color(theme::text_primary())
            .font_family(theme::SANS)
            .child(chrome::title_strip(self, &status, window, cx))
            .child(
                div()
                    .on_children_prepainted(e2e::regions(&["rail", "pane", "editor"]))
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(chrome::nav_rail(self, &status, cx))
                    .child(pane)
                    .children(rail),
            )
            .children(sheet)
    }
}
