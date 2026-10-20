//! The main window's view: its state, navigation, keyboard handling and
//! layout.

use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{
    Context, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _,
    Render, Styled as _, Window, div,
};

use pwrroute_engine::{DefaultHost, Engine, Limits};

use crate::activity::Activity;
use crate::config_view::{self, ConfigInCharge};
use crate::fixtures::{self, Handler, Rewrite};
use crate::test_link::TestLink;
use crate::{activity, chrome, editor, router, rules_view, test_link, theme};

/// The element id of the window's root, for tests and the inspector.
pub const ROOT_ID: &str = "pwrroute-root";

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
    pub rewrites: Vec<Rewrite>,
    pub handlers: Vec<Handler>,
    pub selection: Selection,
    /// A Finicky config is in charge: the Rules screen is option 1b.
    pub config: Option<ConfigInCharge>,
    /// 1b's folded visual rules are open.
    pub visual_rules_expanded: bool,
    pub test: TestLink,
    pub activity: Activity,
}

impl PwrRoute {
    /// The window for the config the router chose, if any.
    pub fn new(cx: &mut Context<'_, Self>) -> Self {
        Self::with_config(router::config_in_charge(), std::env::home_dir(), cx)
    }

    /// The window as it is with `config` in charge (`None`: the visual
    /// rules). `home` shortens the config's path to `~/…`.
    pub fn with_config(
        config: Option<PathBuf>,
        home: Option<PathBuf>,
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
        Self {
            focus: cx.focus_handle(),
            screen: Screen::Rules,
            rewrites: fixtures::rewrites(),
            handlers: fixtures::handlers(),
            selection: Selection::Handler(2),
            config: in_charge,
            visual_rules_expanded: false,
            test: TestLink::new(config, engine.and_then(Result::ok), cx),
            activity: Activity::new(cx),
        }
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

    /// The editor shows a rule: the visual rules are on screen.
    pub fn editing(&self) -> bool {
        self.screen == Screen::Rules && (self.config.is_none() || self.visual_rules_expanded)
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
        if selection != self.selection && self.order().contains(&selection) {
            self.selection = selection;
            cx.notify();
        }
    }

    /// Moves the selection `by` rows through both lists, clamped at the ends.
    fn step(&mut self, by: isize, cx: &mut Context<'_, Self>) {
        let order = self.order();
        let Some(at) = order.iter().position(|s| *s == self.selection) else {
            return;
        };
        let to = at
            .saturating_add_signed(by)
            .min(order.len().saturating_sub(1));
        self.select(order[to], cx);
    }

    /// Switch the selected rule on or off.
    pub fn toggle_selected(&mut self, cx: &mut Context<'_, Self>) {
        let rule = match self.selection {
            Selection::Rewrite(i) => self.rewrites.get_mut(i).map(|r| &mut r.rule),
            Selection::Handler(i) => self.handlers.get_mut(i).map(|h| &mut h.rule),
        };
        if let Some(rule) = rule {
            rule.enabled = !rule.enabled;
            let enabled = rule.enabled;
            self.test.set_enabled(self.selection, enabled);
            cx.notify();
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<'_, Self>) {
        if !self.editing() {
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
        let rail = self.editing().then(|| editor::rail(self, cx));
        div()
            .id(ROOT_ID)
            .debug_selector(|| ROOT_ID.into())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                this.on_key_down(event, cx);
            }))
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
    }
}
