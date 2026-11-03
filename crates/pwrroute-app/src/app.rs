//! The main window's view: its state, navigation, keyboard handling and
//! layout.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use gpui_kit::{
    App, ClipboardItem, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, div,
};

use pwrroute_engine::{Description, Engine, Limits, NativeHost, discovery};
use crate::engine_worker::{EngineWorker, Lane};

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
use crate::rules_store::{Discarded, Disk, RulesStore};
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

/// What a [`Leave::Then`] runs once the leave prompt is answered.
pub type LeaveThen = Box<dyn FnOnce(&mut PwrRoute, &mut Window, &mut Context<'_, PwrRoute>)>;

/// Somewhere to go from a changed draft (design: Rule Drafts 1d). It waits
/// while the rail's foot asks, and goes ahead after Save or Discard.
pub enum Leave {
    /// Another row.
    Select(Selection),
    /// Copy a row, then open the copy. Like Select, its index is valid
    /// only until the rules reload.
    Duplicate(Selection),
    /// Back to the list ("‹ All rules", Esc in the stacked layout).
    Deselect,
    /// The window's close button or ⌘W.
    Close,
    /// Anything else that starts or opens another rule: Add rule, the
    /// Overview's and Test a link's. Must not capture a row index: use a
    /// typed transition above for those. `what` finishes the question
    /// ("before starting a new rule?").
    Then { what: &'static str, then: LeaveThen },
}

impl Leave {
    /// "Save your changes to rule 4 before …?"
    pub fn question(&self, draft: &str) -> String {
        let to = match self {
            Self::Select(Selection::Handler(i)) => format!("opening rule {}", i + 1),
            Self::Select(Selection::Rewrite(i)) => format!("opening rewrite {}", i + 1),
            Self::Duplicate(_) => "duplicating it".to_owned(),
            Self::Deselect => "going back to all rules".to_owned(),
            Self::Close => "closing the window".to_owned(),
            Self::Then { what, .. } => (*what).to_owned(),
        };
        format!("Save your changes to {draft} before {to}?")
    }
}

/// The leave prompt's answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaveAnswer {
    KeepEditing,
    Discard,
    Save,
}

pub struct PwrRoute {
    pub focus: FocusHandle,
    /// The strip's menu bar off macOS (`menu_bar.rs`).
    pub menu_bar: Entity<MenuBar>,
    pub screen: Screen,
    /// `rules.json`, read on open and written after every command, and on
    /// Save for the editor's draft.
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
    pub(crate) engine_tasks: [Option<gpui_kit::Task<()>>; crate::engine_worker::LANES],
    pub(crate) engine_worker: EngineWorker,
    pub(crate) engine_revision: u64,
    preview_request: u64,
    config_request: u64,
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
    /// Sync's line for the rail foot and strip (`sync::line`); a test or
    /// screenshot sets its own.
    pub sync_line: Option<crate::sync::Line>,
    /// A debounced save is on its way.
    save_scheduled: bool,
    /// Where the person asked to go from a changed draft, while the
    /// rail's foot asks Save or Discard first (Rule Drafts 1d).
    pub leaving: Option<Leave>,
    /// The last Discard, for its Undo until the next edit (Rule Drafts 1e).
    pub discarded: Option<Discarded>,
    /// Why the last Save did nothing (problems, or a write the store
    /// refused), until the next edit.
    pub draft_note: Option<String>,
    /// Pausing from the tray redraws the Overview.
    _tray: Option<Subscription>,
    /// Each sync pass redraws the rail foot's line.
    _sync: Option<Subscription>,
    /// The update toast and the strip's ready chip follow the updater.
    _updates: Option<Subscription>,
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
        let in_charge = config.as_deref().map(|path| pending_config(path, home.as_deref()));
        let described = None;
        // Pausing and resuming from the tray changes the Overview's hero.
        let tray = crate::tray::model(cx).map(|model| cx.observe(&model, |_, _, cx| cx.notify()));
        let sync = crate::sync::feed(cx).map(|feed| {
            cx.observe(&feed, |this: &mut Self, _, cx| {
                this.sync_line = crate::sync::line(cx);
                cx.notify();
            })
        });
        let updates = crate::update_notice::notices(cx)
            .map(|notices| cx.observe(&notices, |_, _, cx| cx.notify()));
        // Rules opens as a list: the editor appears on a click (or Down).
        let engine_worker = EngineWorker::new();
        let mut this = Self {
            // A Tab stop, so Tab comes back round to the list and map keys.
            focus: cx.focus_handle().tab_stop(true),
            menu_bar: MenuBar::entity("PwrRoute", cx),
            screen: Screen::Overview,
            rewrites: store.rewrites(),
            handlers: store.handlers(),
            selection: None,
            rail: None,
            probe: Probe::at_home(config.clone(), home.as_deref(), store.edited()),
            engine_tasks: std::array::from_fn(|_| None),
            engine_revision: 0,
            preview_request: 0,
            config_request: 0,
            tests: TestRuns::new(config.clone(), store.edited()),
            test: TestLink::new(
                config,
                home.as_deref(),
                engine_worker.client(),
                store.written(),
                cx,
            ),
            engine_worker,
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
            overview: OverviewState::new(cx),
            installed: Vec::new(),
            described,
            config_loaded: SystemTime::now(),
            build: BuildInfo::current(),
            build_copied: None,
            sync_line: crate::sync::line(cx),
            save_scheduled: false,
            leaving: None,
            discarded: None,
            draft_note: None,
            _tray: tray,
            _sync: sync,
            _updates: updates,
        };
        if this.config.is_none() {
            this.found_config = crate::importer_view::found_config(this.home.as_deref(), cx);
        }
        let shown = this.config.as_ref().map(|config| config.path.clone());
        this.activity.set_config(shown, cx);
        this.describe_config(cx);
        this.rebuild_rail(cx);
        // The file's examples and tests, as it was opened.
        if this.tests.counts() != (0, 0) {
            this.schedule_tests(cx);
        }
        this
    }

    /// Shows a screen. Test a link runs its sample the first time and takes
    /// the keyboard; the others give it to the window.
    pub fn navigate(&mut self, screen: Screen, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.screen != screen {
            // A View shortcut dismisses the old screen's transient surfaces
            // before focusing the new screen, just as leaving via the rail does.
            if self.actions.sheet.is_some() {
                self.cancel_add_rule(window, cx);
            }
            self.actions.menu = None;
            self.row_actions.menu = None;
            // The draft stays; the question about leaving it does not.
            self.leaving = None;
            self.test.picking = false;
            crate::rules_view::set_picker_open(false, cx);
            if let Some(rail) = &mut self.rail {
                rail.menu = None;
                rail.picking_kind = None;
            }
        }
        if self.screen == Screen::Overview && screen != Screen::Overview {
            self.overview.leave();
        }
        self.screen = screen;
        match screen {
            Screen::TestLink => {
                // Test a link tests the file as it is.
                self.sync_with_disk(cx);
                if !self.test.has_run() {
                    self.test.run(cx);
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

    /// The store has this row (the rows themselves may not be rebuilt yet).
    fn in_store(&self, selection: Selection) -> bool {
        match selection {
            Selection::Rewrite(i) => i < self.store.rewrite_count(),
            Selection::Handler(i) => i < self.store.handler_count(),
        }
    }

    /// The store's last row in list order: rewrites, then handlers.
    fn last_in_store(&self) -> Option<Selection> {
        match (self.store.rewrite_count(), self.store.handler_count()) {
            (_, h) if h > 0 => Some(Selection::Handler(h - 1)),
            (r, _) if r > 0 => Some(Selection::Rewrite(r - 1)),
            _ => None,
        }
    }

    /// Select a row; one that is not in the lists is ignored. From a
    /// changed draft the rail's foot asks first (Rule Drafts 1d).
    pub fn select(&mut self, selection: Selection, cx: &mut Context<'_, Self>) {
        if Some(selection) != self.selection && self.order().contains(&selection) {
            if self.ask_before_leaving(Leave::Select(selection), cx) {
                return;
            }
            self.selection = Some(selection);
            // Undo discard belongs to the rule it was discarded on.
            self.discarded = None;
            self.follow_draft();
            self.rebuild_rail(cx);
            cx.notify();
        }
    }

    // --- the draft (design: Rule Drafts 1) --------------------------------

    /// The store holds the selected rule as the draft; a changed draft
    /// keeps the selection on its rule instead (after a reload or a move
    /// put it at another index).
    pub(crate) fn follow_draft(&mut self) {
        if self.store.draft_changed()
            && let Some(draft) = self.store.draft()
        {
            self.selection = Some(draft);
            return;
        }
        match self.selection {
            Some(selection) => self.store.hold(selection),
            None => self.store.release(),
        }
    }

    /// The draft's name in the leave prompt: "rule 4", "the new rule".
    fn draft_name(&self) -> String {
        match (self.store.draft(), self.store.draft_is_new()) {
            (Some(Selection::Handler(_)), true) => "the new rule".to_owned(),
            (Some(Selection::Rewrite(_)), true) => "the new rewrite".to_owned(),
            (Some(Selection::Handler(i)), false) => format!("rule {}", i + 1),
            (Some(Selection::Rewrite(i)), false) => format!("rewrite {}", i + 1),
            (None, _) => "this rule".to_owned(),
        }
    }

    /// The leave prompt's question, while it asks.
    pub fn leave_question(&self) -> Option<String> {
        self.leaving
            .as_ref()
            .map(|leave| leave.question(&self.draft_name()))
    }

    /// Whether `to` waits for the leave prompt: only from a changed draft.
    /// Then the Rules screen shows the draft with the question at its foot,
    /// and `to` goes ahead after Save or Discard ([`Self::answer_leave`]).
    pub(crate) fn ask_before_leaving(&mut self, to: Leave, cx: &mut Context<'_, Self>) -> bool {
        if !self.store.draft_changed() {
            return false;
        }
        if let Some(draft) = self.store.draft() {
            self.selection = Some(draft);
        }
        if self.screen != Screen::Rules {
            if self.screen == Screen::Overview {
                self.overview.leave();
            }
            self.screen = Screen::Rules;
        }
        self.visual_rules_expanded = true;
        self.leaving = Some(to);
        cx.notify();
        true
    }

    /// Runs `then` now, or once the person answers the leave prompt.
    pub(crate) fn leave_draft_then(
        &mut self,
        what: &'static str,
        then: impl FnOnce(&mut PwrRoute, &mut Window, &mut Context<'_, PwrRoute>) + 'static,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if self.store.draft_changed() {
            self.ask_before_leaving(
                Leave::Then {
                    what,
                    then: Box::new(then),
                },
                cx,
            );
            window.focus(&self.focus, cx);
            return true;
        }
        false
    }

    /// The leave prompt's answer: stay, or Save or Discard and go on.
    pub fn answer_leave(
        &mut self,
        answer: LeaveAnswer,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if answer == LeaveAnswer::KeepEditing {
            self.leaving = None;
            cx.notify();
            return;
        }
        if self.leaving.is_none() {
            return;
        }
        // A prompt's row index may already be stale when the answer is
        // pressed. Reload before touching the draft, and let reload cancel
        // that transition while keeping the rebased draft to edit.
        self.sync_with_disk(cx);
        if self.leaving.is_none() {
            return;
        }
        let removed = match answer {
            LeaveAnswer::KeepEditing => unreachable!("handled above"),
            LeaveAnswer::Save => {
                if !self.write_draft(cx) {
                    // The note says why; the prompt stays.
                    return;
                }
                None
            }
            LeaveAnswer::Discard => self.discard_draft(window, cx),
        };
        // Keep ownership in the window until Save/Discard finishes:
        // their refresh can also reload and invalidate a row transition.
        let Some(to) = self.leaving.take() else {
            return;
        };
        // A never-saved rule left its list: rows below it moved up one.
        let shift = |selection: Selection| match (selection, removed) {
            (selection, Some(gone)) if selection == gone => None,
            (Selection::Handler(i), Some(Selection::Handler(gone))) if i > gone => {
                Some(Selection::Handler(i - 1))
            }
            (Selection::Rewrite(i), Some(Selection::Rewrite(gone))) if i > gone => {
                Some(Selection::Rewrite(i - 1))
            }
            (selection, _) => Some(selection),
        };
        match to {
            Leave::Select(selection) => {
                if let Some(selection) = shift(selection) {
                    self.select(selection, cx);
                }
            }
            Leave::Duplicate(selection) => {
                if let Some(selection) = shift(selection) {
                    self.duplicate_rule(selection, window, cx);
                }
            }
            Leave::Deselect => self.deselect(cx),
            Leave::Close => window.remove_window(),
            Leave::Then { then, .. } => then(self, window, cx),
        }
        cx.notify();
    }

    /// Save (the button, ⌘S, Return in a rule's field): writes the draft
    /// over the file as it stands. `false`, with the reason in the rail's
    /// foot, when it has problems or the store refused the write.
    pub fn save_draft(&mut self, cx: &mut Context<'_, Self>) -> bool {
        // Another writer's change first: the draft rides on top of it.
        self.sync_with_disk(cx);
        let saved = self.write_draft(cx);
        if saved {
            // A direct Save finishes any question about leaving.
            self.leaving = None;
        }
        saved
    }

    /// Save after checking disk, keeping a deferred transition in the
    /// window so a reload during refresh can still invalidate it.
    fn write_draft(&mut self, cx: &mut Context<'_, Self>) -> bool {
        match self.store.save_draft() {
            Ok(()) => {
                self.refresh(cx);
                self.draft_note = None;
                self.rebuild_rail_kept(cx);
                if self.tests.counts() != (0, 0) {
                    self.schedule_tests(cx);
                }
                cx.notify();
                true
            }
            Err(reason) => {
                self.rewrites = self.store.rewrites();
                self.handlers = self.store.handlers();
                self.draft_note = Some(reason);
                cx.notify();
                false
            }
        }
    }

    /// Discard (the button, Esc): the rule back as saved, or a never-saved
    /// one out of the list, with Undo until the next edit. Returns the
    /// row a never-saved rule left.
    pub fn discard_draft(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<Selection> {
        let draft = self.store.draft();
        let discarded = self.store.discard_draft()?;
        let removed = matches!(discarded, Discarded::New(_))
            .then_some(draft)
            .flatten();
        if let Some(gone) = removed {
            if let Selection::Handler(index) = gone {
                self.drafts_after_delete(index);
            }
            self.selection = None;
        }
        self.refresh(cx);
        self.discarded = Some(discarded);
        self.rebuild_rail(cx);
        window.focus(&self.focus, cx);
        cx.notify();
        removed
    }

    /// Undo discard: the edits back as the draft. `false` with nothing to
    /// undo.
    pub fn undo_discard(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> bool {
        if self.store.draft_changed() {
            return false;
        }
        let Some(discarded) = self.discarded.take() else {
            return false;
        };
        let new = matches!(discarded, Discarded::New(_));
        let Some(selection) = self.store.undo_discard(discarded) else {
            return false;
        };
        if new && let Selection::Handler(index) = selection {
            self.drafts_after_insert(index);
        }
        self.selection = Some(selection);
        self.refresh(cx);
        self.rebuild_rail(cx);
        window.focus(&self.focus, cx);
        true
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
        self.engine_revision += 1;
        self.engine_worker.cancel(Lane::Impact);
        // Undo import and Undo delete are good until the next edit.
        if let Some(done) = &mut self.last_import {
            done.undoable = false;
        }
        self.row_actions.edited();
        self.discarded = None;
        self.draft_note = None;
        let disk = self.store.check_disk();
        // A selected row that is gone gives way to the last; no selection
        // stays none.
        self.selection = match self.selection {
            Some(selection) if !self.in_store(selection) => self.last_in_store(),
            selection => selection,
        };
        self.follow_draft();
        // The rows once the hold is settled: it decides their badges.
        self.rewrites = self.store.rewrites();
        self.handlers = self.store.handlers();
        self.test.set_rules(self.store.written());
        self.probe.set_rules(self.store.edited());
        self.tests.set_rules(self.store.edited());
        if disk == Disk::Reloaded {
            self.cancel_row_leave();
            // The rail's fields held a rule that is gone: nothing was
            // unsaved, so they are rebuilt from the file.
            self.rebuild_rail(cx);
        }
        self.try_it(cx);
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
        model.validate_target(self.store.path(), &previous)?;
        let plan = model.plan(&previous);
        let before = self.store.clone();
        self.store.replace_all(plan.file.clone());
        if let Err(reason) = self.store.save() {
            // The importer owns this staged edit. A retry must start from
            // the same rules and keep the original Undo snapshot.
            self.store = before;
            self.reloaded(cx);
            return Err(reason);
        }
        self.reloaded(cx);
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
            saved_rules: crate::importer_view::SavedRules::new(self.store.path(), plan.file),
            undoable: true,
            rules_restored: false,
            problem: None,
        });
        cx.notify();
        Ok(())
    }

    /// Undo import: `rules.json` as it was before, in one save. Returns the
    /// source to restore once that save went through. Another writer's
    /// change since the import ends Undo instead of being overwritten.
    pub fn undo_import(
        &mut self,
        cx: &mut Context<'_, Self>,
    ) -> Result<Option<SourceSettings>, String> {
        let disk = self.sync_with_disk(cx);
        let Some(done) = &self.last_import else {
            return Ok(None);
        };
        if !done.undoable || disk != Disk::Unchanged || self.store.save_refused().is_some() {
            if let Some(done) = &mut self.last_import {
                done.undoable = false;
            }
            cx.notify();
            return Ok(None);
        }
        let done = done.clone();
        if !done.rules_restored {
            let before = self.store.clone();
            self.store.replace_all(done.previous);
            if let Err(reason) = self.store.save() {
                self.store = before;
                self.reloaded(cx);
                return Err(reason);
            }
            self.reloaded(cx);
            if let Some(done) = &mut self.last_import {
                done.rules_restored = true;
                done.saved_rules.invalidate();
            }
        }
        Ok(done.previous_source)
    }

    /// The config in charge changed (the importer's Choose source, or its
    /// undo): the Rules screen, Test a link and the test runs follow it.
    pub fn set_config(&mut self, config: Option<PathBuf>, cx: &mut Context<'_, Self>) {
        self.engine_revision += 1;
        self.engine_worker.cancel(Lane::Impact);
        self.config = config.as_deref().map(|path| pending_config(path, self.home.as_deref()));
        self.visual_rules_expanded = false;
        self.described = None;
        self.config_loaded = SystemTime::now();
        self.describe_config(cx);
        self.probe = Probe::at_home(config.clone(), self.home.as_deref(), self.store.edited());
        self.engine_tasks[Lane::Tests as usize] = None;
        self.tests = TestRuns::new(config.clone(), self.store.edited());
        self.test = TestLink::new(
            config,
            self.home.as_deref(),
            self.engine_worker.client(),
            self.store.written(),
            cx,
        );
        if self.tests.counts() != (0, 0) {
            self.schedule_tests(cx);
        }
        let shown = self.config.as_ref().map(|config| config.path.clone());
        self.activity.set_config(shown, cx);
        self.try_it(cx);
        cx.notify();
    }

    /// The store holds the file again: the rows, the selection (cleared
    /// when its row is gone), the rail, and every engine over the rules.
    fn reloaded(&mut self, cx: &mut Context<'_, Self>) {
        self.engine_revision += 1;
        self.engine_worker.cancel(Lane::Impact);
        // The rows may be other rules now: a menu, a drag or an Undo for
        // one of them would land on the wrong one.
        self.row_actions = crate::rule_menu::RowActions::default();
        self.cancel_row_leave();
        self.selection = self.selection.filter(|s| self.in_store(*s));
        // The draft stays on its rule, wherever the file put it.
        self.follow_draft();
        self.rewrites = self.store.rewrites();
        self.handlers = self.store.handlers();
        self.test.set_rules(self.store.written());
        self.probe.set_rules(self.store.edited());
        self.tests.set_rules(self.store.edited());
        self.rebuild_rail(cx);
        if self.tests.counts() != (0, 0) {
            self.schedule_tests(cx);
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

    /// Reload recreates store ids and may replace or reorder any row.
    /// Cancel indexed work; the person can choose again against this file.
    fn cancel_row_leave(&mut self) {
        if matches!(self.leaving, Some(Leave::Select(_) | Leave::Duplicate(_))) {
            self.leaving = None;
        }
    }

    /// Checks the file each time the window comes to the front: an editor
    /// or a sync changed it while something else had the screen.
    pub fn watch_activation(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        // The close button asks first while a draft has unsaved changes
        // (Rule Drafts 1d); ⌘W does the same through `ask_before_close`.
        let view = cx.entity();
        window.on_window_should_close(cx, move |_, cx| {
            !view.update(cx, |this, cx| this.ask_before_leaving(Leave::Close, cx))
        });
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                // machine.json edited by hand: the rows and Try it follow,
                // as the router's reloader does.
                if crate::sync::reload_machine(cx) {
                    this.refresh(cx);
                }
                this.sync_with_disk(cx);
                // Another machine may have saved while this one was away.
                crate::sync::wake(cx);
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
        // The rule the rail shows is the one the store holds back.
        self.follow_draft();
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
        self.try_it(cx);
    }

    /// Runs the rail's sample through the edited rules.
    pub(crate) fn try_it(&mut self, cx: &mut Context<'_, Self>) {
        self.preview_request += 1;
        self.engine_worker.cancel(Lane::Preview);
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
        rail.try_result = None;
        if problems || sample.trim().is_empty() { return; }
        let input = self.probe.input();
        let request = self.preview_request;
        let revision = self.engine_revision;
        let result = self.engine_worker.submit(Lane::Preview, Duration::from_millis(100), move |_| {
            let mut probe = Probe::from_input(input);
            move || {
                let result = probe.run(&sample, opener.as_deref(), selection);
                Some((result, probe.stamps()))
            }
        });
        self.engine_tasks[Lane::Preview as usize] = Some(cx.spawn(async move |this, cx| {
            if let Ok((result, stamps)) = result.recv().await {
                let _ = this.update(cx, |this, cx| {
                    if this.preview_request == request && this.engine_revision == revision && this.selection == Some(selection) {
                        if !crate::engine_worker::unchanged(&stamps) { this.try_it(cx); return; }
                        if let Some(rail) = &mut this.rail { rail.try_result = Some(result); }
                        cx.notify();
                    }
                });
            }
        }));
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
            self.schedule_tests(cx);
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
        if self.leave_draft_then("starting a new rewrite", Self::add_rewrite, window, cx) {
            return;
        }
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
        if self.leave_draft_then("starting a new rule", Self::add_handler, window, cx) {
            return;
        }
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
        if self.store.draft_changed() {
            self.leave_draft_then(
                "starting a new rule",
                move |this, window, cx| this.add_handler_from(link, window, cx),
                window,
                cx,
            );
            return;
        }
        self.add_blank_handler(window, cx);
        self.start_from(link, window, cx);
    }

    /// A new, empty handler just above Everything else, selected, with its
    /// first pattern ready to type.
    pub fn add_blank_handler(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.leave_draft_then("starting a new rule", Self::add_blank_handler, window, cx) {
            return;
        }
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
        if self.leave_draft_then("starting a new rule", Self::add_rule_from_test, window, cx) {
            return;
        }
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
                field.select_all();
            }
            rail.sample.set(url);
            rail.sample_edited = true;
        }
        self.try_it(cx);
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
        window: &mut Window,
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
                    self.try_it(cx);
                    cx.notify();
                } else {
                    self.apply_field(field, &text);
                    self.refresh(cx);
                }
                true
            }
            Edit::Submit => {
                match field {
                    Field::Sample => {
                        self.try_it(cx);
                        cx.notify();
                    }
                    // Return in the rule's own fields saves the draft (Rule
                    // Drafts keys); with nothing changed it does nothing.
                    Field::Pattern(_)
                    | Field::Browser
                    | Field::Profile
                    | Field::Value(_)
                    | Field::Set => {
                        if self.leaving.is_some() {
                            // The leave prompt's Return: save, then go.
                            self.answer_leave(LeaveAnswer::Save, window, cx);
                        } else if self.store.draft_changed() {
                            self.save_draft(cx);
                        }
                    }
                    Field::Example | Field::Becomes | Field::Name => {}
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
            (Field::Set, Selection::Handler(h)) => self.store.set_rule_set(h, text),
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
        // ⌘S (Ctrl+S off macOS) saves the draft from anywhere on Rules,
        // a field included: a field never takes a ⌘ key it has no use for.
        let keystroke = &event.keystroke;
        let m = &keystroke.modifiers;
        if keystroke.key == "s" && m.secondary(cx) && !m.alt && !m.shift && self.editing() {
            if self.leaving.is_some() {
                self.answer_leave(LeaveAnswer::Save, window, cx);
            } else if self.store.draft_changed() {
                self.save_draft(cx);
            }
            cx.stop_propagation();
            return;
        }
        // An unhandled Esc bubbles out of a field after its selection is
        // collapsed. Dismiss only the innermost transient surface.
        if event.keystroke.key == "escape" {
            let mut dismissed = true;
            if self
                .rail
                .as_ref()
                .is_some_and(|r| r.menu.is_some() || r.picking_kind.is_some())
            {
                if let Some(rail) = &mut self.rail {
                    rail.menu = None;
                    rail.picking_kind = None;
                }
            } else if self.actions.menu.take().is_some() || self.row_actions.menu.take().is_some() {
            } else if self.test.picking {
                self.test.picking = false;
            } else if self.overview.picking {
                self.overview.picking = false;
                self.overview.choosing = None;
            } else if crate::rules_view::picker_open(cx) {
                crate::rules_view::set_picker_open(false, cx);
            } else if self.actions.sheet.is_some() {
                self.cancel_add_rule(window, cx);
            } else if self.leaving.is_some() {
                // The leave prompt: Esc keeps editing (Rule Drafts 1d).
                self.leaving = None;
            } else if self.editing() && self.store.draft_changed() {
                // Then Esc discards the draft, with Undo (1e).
                self.discard_draft(window, cx);
            } else {
                dismissed = false;
            }
            if dismissed {
                cx.stop_propagation();
                cx.notify();
                return;
            }
        }
        // A menu owns focus, but its documented list shortcuts still name
        // the row that opened it (rather than the currently selected row).
        if (self.actions.menu.is_some() && self.activity_key(event, window, cx))
            || (self.row_actions.menu.is_some() && self.rules_list_key(event, window, cx))
        {
            cx.stop_propagation();
            return;
        }
        // A field in the rail has the keyboard: its keys are its own.
        if !self.focus.is_focused(window) {
            return;
        }
        let key = event.keystroke.key.as_str();
        match self.screen {
            // The map's keyboard: Esc lets go of the pin.
            Screen::Overview => {
                self.overview_key(key, window, cx);
            }
            Screen::Rules if self.config.is_none() || self.visual_rules_expanded => {
                if key == "enter" && self.leaving.is_some() {
                    self.answer_leave(LeaveAnswer::Save, window, cx);
                    return;
                }
                if key == "z"
                    && m.secondary(cx)
                    && !m.alt
                    && !m.shift
                    && self.discarded.is_some()
                    && self.undo_discard(window, cx)
                {
                    return;
                }
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
                    ("escape", Some(_)) if Layout::of(self, window).stacked => self.deselect(cx),
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

impl PwrRoute {
    pub(crate) fn schedule_tests(&mut self, cx: &mut Context<'_, Self>) {
        let (request, config, rules) = self.tests.begin();
        let result = self.engine_worker.submit(Lane::Tests, Duration::ZERO, move |_| crate::try_it::test_steps(config, rules));
        self.engine_tasks[Lane::Tests as usize] = Some(cx.spawn(async move |this, cx| {
            if let Ok((report, stamps)) = result.recv().await {
                let _ = this.update(cx, |this, cx| {
                    if !crate::engine_worker::unchanged(&stamps) {
                        if this.tests.is_current(&request) { this.schedule_tests(cx); }
                        return;
                    }
                    if this.tests.complete(request, report) { cx.notify(); }
                });
            }
        }));
    }

    fn describe_config(&mut self, cx: &mut Context<'_, Self>) {
        let Some(path) = self.config.as_ref().map(|config| config.file.clone()) else {
            self.config_request += 1;
            self.engine_worker.cancel(Lane::Config);
            return;
        };
        let home = self.home.clone();
        self.config_request += 1;
        let request = self.config_request;
        let result = self.engine_worker.once(Lane::Config, move || {
            let engine = Engine::load(&path, Rc::new(NativeHost::new()), Limits::default());
            let description = engine.as_ref().ok().and_then(|engine| engine.describe().ok().flatten());
            let config = ConfigInCharge::new(&path, home.as_deref(), engine.as_ref());
            let stamps = engine.as_ref().map(crate::engine_worker::stamps).unwrap_or_default();
            (config, description, stamps)
        });
        self.engine_tasks[Lane::Config as usize] = Some(cx.spawn(async move |this, cx| {
            if let Ok((config, description, stamps)) = result.recv().await {
                let _ = this.update(cx, |this, cx| {
                    if this.config_request == request {
                        if !crate::engine_worker::unchanged(&stamps) { this.describe_config(cx); return; }
                        this.config = Some(config);
                        this.described = description;
                        cx.notify();
                    }
                });
            }
        }));
    }
}

fn pending_config(path: &std::path::Path, home: Option<&std::path::Path>) -> ConfigInCharge {
    ConfigInCharge::new(path, home, Err(&anyhow::anyhow!("Reading config…")))
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

/// How the main window lays out at its width, in the design's pixels, so
/// that View › Zoom reflows rather than clips (design: Contrast and Scale
/// 3). At 100% and 125% nothing changes; zoomed past what the display
/// holds, the rail goes to icons, then Rules shows the list or the rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The nav rail as icons, each named by its tooltip.
    pub compact_rail: bool,
    /// Rules' list and editor take turns: with a rule selected the editor
    /// fills the pane, and "All rules" (or Esc) goes back.
    pub stacked: bool,
}

impl Layout {
    /// Below this the nav rail is icons: 175% and 200% on a laptop.
    pub const COMPACT_RAIL_BELOW: f32 = 960.0;
    /// The rules list's least width beside the editor.
    pub const LIST_MIN: f32 = 428.0;

    /// The layout at `width`. `full_rail`: the rail keeps its labels
    /// however narrow, while its foot asks to Reload or Overwrite
    /// `rules.json`.
    pub fn of_width(width: f32, full_rail: bool) -> Self {
        let compact_rail = width < Self::COMPACT_RAIL_BELOW && !full_rail;
        let mut layout = Self {
            compact_rail,
            stacked: false,
        };
        layout.stacked = width - layout.rail_width() < Self::LIST_MIN + editor::WIDTH;
        layout
    }

    pub fn of(app: &PwrRoute, window: &Window) -> Self {
        Self::of_width(
            theme::css(window.viewport_size().width),
            app.store.changed_on_disk(),
        )
    }

    /// The nav rail's width, without its border.
    pub fn rail_width(self) -> f32 {
        if self.compact_rail {
            crate::chrome::RAIL_COMPACT
        } else {
            crate::chrome::RAIL_WIDTH
        }
    }
}

impl PwrRoute {
    /// Back to the list with nothing selected: the stacked layout's "All
    /// rules" and Esc.
    pub fn deselect(&mut self, cx: &mut Context<'_, Self>) {
        if self.selection.is_some() && self.ask_before_leaving(Leave::Deselect, cx) {
            return;
        }
        if self.selection.take().is_some() {
            self.discarded = None;
            self.store.release();
            self.rebuild_rail(cx);
            cx.notify();
        }
    }
}

impl Render for PwrRoute {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if cx.reduce_motion() {
            self.overview.finish_handoff();
        }
        let layout = Layout::of(self, window);
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
        let editing = self.editing();
        let rail = editing.then(|| editor::rail(self, layout.stacked, window, cx));
        // Stacked, the rule being edited takes the list's place.
        let pane = (rail.is_none() || !layout.stacked).then_some(pane);
        // Activity's Add a rule sheet, over everything.
        let sheet = activity_actions::sheet(self, window, cx);
        div()
            .on_children_prepainted(|bounds, window, cx| {
                e2e::frame(window);
                e2e::regions(&["titleStrip", "body"])(bounds, window, cx);
            })
            .id(ROOT_ID)
            .role(gpui_kit::Role::Pane)
            .aria_label(self.screen.label())
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
                    .child(chrome::nav_rail(self, &status, layout.compact_rail, cx))
                    .children(pane)
                    .children(rail),
            )
            // The update toast (`toast.rs`), over the pane's lower right:
            // left of the editor beside the list, so its Save and Discard
            // stay clear. A stacked editor fills the pane, so there the
            // lasting Update ready card gives way to the strip's chip.
            .children(
                crate::toast::update_toast(editing && layout.stacked, cx).map(|toast| {
                    let right = if editing && !layout.stacked {
                        editor::WIDTH + 16.0
                    } else {
                        16.0
                    };
                    div()
                        .absolute()
                        .right(theme::px(right))
                        .bottom(theme::px(16.0))
                        .child(toast)
                }),
            )
            // Over the toast: a sheet is modal.
            .children(sheet)
    }
}

impl crate::text_field::InputOwner for PwrRoute {
    fn input_field(&mut self, focus: &FocusHandle) -> Option<&mut TextField> {
        if let Some(sheet) = &mut self.actions.sheet {
            return (sheet.pattern.focus == *focus).then_some(&mut sheet.pattern);
        }
        if self.test.url.focus == *focus {
            return Some(&mut self.test.url);
        }
        if self.activity.search.focus == *focus {
            return Some(&mut self.activity.search);
        }
        let rail = self.rail.as_mut()?;
        let key = rail.input_field_key(focus)?;
        rail.field_mut(key)
    }

    fn input_changed(&mut self, focus: &FocusHandle, cx: &mut Context<'_, Self>) {
        if self
            .actions
            .sheet
            .as_ref()
            .is_some_and(|s| s.pattern.focus == *focus)
        {
            self.add_rule_input_changed(cx);
            return;
        }
        let Some(key) = self.rail.as_ref().and_then(|r| r.input_field_key(focus)) else {
            return;
        };
        let Some(field) = self.rail.as_mut().and_then(|r| r.field_mut(key)) else {
            return;
        };
        let text = field.text.clone();
        match key {
            Field::Sample => {
                if let Some(rail) = &mut self.rail {
                    rail.sample_edited = true;
                }
                self.try_it(cx);
            }
            Field::Example | Field::Becomes | Field::Name => {}
            _ => {
                self.apply_field(key, &text);
                self.refresh(cx);
            }
        }
    }
}
