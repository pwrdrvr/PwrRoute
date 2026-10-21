//! The main window's view: its state, navigation, keyboard handling and
//! layout.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{
    Context, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _,
    Render, Styled as _, Window, div,
};

use pwrroute_engine::{DefaultHost, Engine, Limits, discovery};

use crate::activity::Activity;
use crate::activity_actions::{self, Actions, BrowserChoice};
use crate::config_view::{self, ConfigInCharge};
use crate::editor::{Field, Rail};
use crate::rules_model::{self, Handler, Rewrite};
use crate::rules_store::RulesStore;
use crate::test_link::TestLink;
use crate::text_field::{Edit, TextField};
use crate::try_it::{Probe, TestRuns};
use crate::{activity, chrome, editor, router, rules_view, test_link, theme};

/// The element id of the window's root, for tests and the inspector.
pub const ROOT_ID: &str = "pwrroute-root";

/// How long after the last edit the file is written: long enough to take a
/// typed word in one write, short enough that the router follows along.
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(400);

/// The rail's sections.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Screen {
    #[default]
    Rules,
    TestLink,
    Activity,
}

impl Screen {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rules => "Rules",
            Self::TestLink => "Test a link",
            Self::Activity => "Activity",
        }
    }
}

/// The rule the editor rail shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Rewrite(usize),
    Handler(usize),
}

pub struct PwrRoute {
    pub focus: FocusHandle,
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
    pub test: TestLink,
    pub activity: Activity,
    /// Activity's row actions: the Add a rule sheet and what the last
    /// action did.
    pub actions: Actions,
    /// The browsers the sheet offers, with their profiles: detected at
    /// launch; a test or screenshot sets its own.
    pub browsers: Vec<BrowserChoice>,
    /// A debounced save is on its way.
    save_scheduled: bool,
}

impl PwrRoute {
    /// The window for the config the router chose, if any, over the real
    /// `rules.json`.
    pub fn new(cx: &mut Context<'_, Self>) -> Self {
        let store = RulesStore::load(discovery::rules_path(&discovery::Environment::current()));
        let mut this =
            Self::with_store(router::config_in_charge(), std::env::home_dir(), store, cx);
        this.browsers = activity_actions::detected_browsers();
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
        // link resolves with it, so the config is evaluated once here.
        let engine = config
            .as_deref()
            .map(|path| Engine::load(path, Rc::new(DefaultHost), Limits::default()));
        let in_charge = config
            .as_deref()
            .zip(engine.as_ref())
            .map(|(path, engine)| ConfigInCharge::new(path, home.as_deref(), engine.as_ref()));
        let selection = if store.handler_count() > 0 {
            Some(Selection::Handler(0))
        } else if store.rewrite_count() > 0 {
            Some(Selection::Rewrite(0))
        } else {
            None
        };
        let mut this = Self {
            focus: cx.focus_handle(),
            screen: Screen::Rules,
            rewrites: store.rewrites(),
            handlers: store.handlers(),
            selection,
            rail: None,
            probe: Probe::new(config.clone(), store.edited()),
            tests: TestRuns::new(config.clone(), store.edited()),
            test: TestLink::new(config, engine.and_then(Result::ok), store.written(), cx),
            store,
            home,
            config: in_charge,
            visual_rules_expanded: false,
            activity: Activity::new(cx),
            actions: Actions::default(),
            browsers: Vec::new(),
            save_scheduled: false,
        };
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
        self.screen = screen;
        match screen {
            Screen::TestLink => {
                if !self.test.has_run() {
                    self.test.run();
                }
                window.focus(&self.test.url.focus, cx);
            }
            Screen::Rules | Screen::Activity => window.focus(&self.focus, cx),
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
    /// rail's fields are kept, so typing keeps its focus.
    pub(crate) fn refresh(&mut self, cx: &mut Context<'_, Self>) {
        self.rewrites = self.store.rewrites();
        self.handlers = self.store.handlers();
        let order = self.order();
        self.selection = self
            .selection
            .filter(|s| order.contains(s))
            .or_else(|| order.last().copied());
        self.test.set_rules(self.store.written());
        self.probe.set_rules(self.store.edited());
        self.tests.set_rules(self.store.edited());
        self.try_it();
        if self.store.needs_save() {
            self.schedule_save(cx);
        }
        cx.notify();
    }

    /// The selected rule's editor, fresh from the store, and its Try it.
    pub(crate) fn rebuild_rail(&mut self, cx: &mut Context<'_, Self>) {
        self.rail = match self.selection {
            Some(Selection::Handler(i)) => self.handlers.get(i).map(|h| Rail::for_handler(h, cx)),
            Some(Selection::Rewrite(i)) => self.rewrites.get(i).map(|r| Rail::for_rewrite(r, cx)),
            None => None,
        };
        self.try_it();
    }

    /// Runs the rail's sample through the edited rules.
    fn try_it(&mut self) {
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

    /// A new, empty rewrite at the end of its list, selected, with its
    /// first pattern ready to type.
    pub fn add_rewrite(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let index = self.store.add_rewrite();
        self.selection = Some(Selection::Rewrite(index));
        self.refresh(cx);
        self.rebuild_rail(cx);
        self.add_pattern(window, cx);
    }

    /// A new, empty handler just above Everything else, selected, with its
    /// first pattern ready to type.
    pub fn add_handler(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let index = self.store.add_handler();
        self.selection = Some(Selection::Handler(index));
        self.refresh(cx);
        self.rebuild_rail(cx);
        self.add_pattern(window, cx);
    }

    /// Removes the selected rule; the row that takes its place is selected.
    pub fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(selection) = self.selection else {
            return;
        };
        self.store.delete(selection);
        // The row that took its place, or the last of its list.
        let (len, make): (usize, fn(usize) -> Selection) = match selection {
            Selection::Rewrite(_) => (self.store.rewrite_count(), Selection::Rewrite),
            Selection::Handler(_) => (self.store.handler_count(), Selection::Handler),
        };
        let index = match selection {
            Selection::Rewrite(i) | Selection::Handler(i) => i,
        };
        self.selection = (len > 0).then(|| make(index.min(len - 1)));
        self.refresh(cx);
        self.rebuild_rail(cx);
        window.focus(&self.focus, cx);
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
        self.rebuild_rail(cx);
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
        self.rebuild_rail(cx);
        window.focus(&self.focus, cx);
    }

    /// Adds an app the selected handler requires the link to come from.
    pub fn add_opener(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(Selection::Handler(h)) = self.selection else {
            return;
        };
        self.store.add_opener(h);
        self.refresh(cx);
        self.rebuild_rail(cx);
        self.focus_field(window, cx, |rail| rail.openers.last());
    }

    pub fn remove_opener(&mut self, index: usize, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(Selection::Handler(h)) = self.selection else {
            return;
        };
        self.store.remove_opener(h, index);
        self.refresh(cx);
        self.rebuild_rail(cx);
        window.focus(&self.focus, cx);
    }

    /// Adds a change to the selected rewrite and focuses its value.
    pub fn add_action(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(Selection::Rewrite(r)) = self.selection else {
            return;
        };
        self.store.add_action(r);
        self.refresh(cx);
        self.rebuild_rail(cx);
        self.focus_field(window, cx, |rail| rail.values.last());
    }

    pub fn remove_action(&mut self, index: usize, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(Selection::Rewrite(r)) = self.selection else {
            return;
        };
        self.store.remove_action(r, index);
        self.refresh(cx);
        self.rebuild_rail(cx);
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

    fn focus_field(
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
            (Field::Opener(i), Selection::Handler(h)) => self.store.set_opener(h, i, text),
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

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<'_, Self>) {
        // A field in the rail has the keyboard: its keys are its own.
        if !self.editing() || !self.focus.is_focused(window) {
            return;
        }
        match event.keystroke.key.as_str() {
            "up" => self.step(-1, cx),
            "down" => self.step(1, cx),
            _ => {}
        }
    }
}

impl Render for PwrRoute {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let pane = match (self.screen, &self.config) {
            (Screen::Rules, None) => rules_view::pane(self, cx).into_any_element(),
            (Screen::Rules, Some(config)) => config_view::pane(self, config, cx).into_any_element(),
            (Screen::TestLink, _) => test_link::pane(self, window, cx).into_any_element(),
            (Screen::Activity, _) => activity::pane(self, window, cx).into_any_element(),
        };
        let rail = self.editing().then(|| editor::rail(self, window, cx));
        // Activity's Add a rule sheet, over everything.
        let sheet = activity_actions::sheet(self, window, cx);
        div()
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
            .child(chrome::title_strip(self))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(chrome::nav_rail(self, cx))
                    .child(pane)
                    .children(rail),
            )
            .children(sheet)
    }
}
