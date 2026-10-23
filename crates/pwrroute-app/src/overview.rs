//! Overview, the landing screen (design: PwrRoute Overview and Setup,
//! 1a-3f): the hero, the setup card, today's tiles and the route map, drawn
//! from [`overview_model::build`].
//!
//! The map is two flex columns over an absolutely positioned layer: a
//! `canvas` paints the dot grid and, for what is hovered or pinned, one
//! bezier per lit route from the row's port to the card's
//! (`PathBuilder::stroke`). The columns record their children's bounds in
//! prepaint (`on_children_prepainted`) into [`MapBounds`], which the canvas
//! reads when it paints in the same frame. Hover is stateful
//! (`on_hover`), kept in [`OverviewState`]; a card click pins until Esc.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, Animation, AnimationExt as _, AnyElement, AnyView, App, AppContext as _, Bounds,
    BoxShadow, Context, Div, ElementId, HighlightStyle, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, PathBuilder, Pixels, Point, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, StyledText, Window, anchored, canvas, deferred,
    div, ease_out_quint, linear_color_stop, linear_gradient, point, px,
};

use crate::app::{PwrRoute, Screen, Selection};
use crate::overview_model::{
    self, Action, CHANGE_IN_RULES, Callout, Caption, Card, CardKind, ConfigInputs, ConfigTrouble,
    DONE_TITLE, End, EverythingElse, Focus, Inputs, Installed, Item, Lit, Map, OFFER_BUTTON,
    OFFER_TITLE, Overview, Pause, Row, RowMatch, RowSource, Setup, SetupStep, State, Status,
    Target, Tone,
};
use crate::rules_view::pane_frame;
use crate::theme::{self, icon, mono, text, tracked};
use crate::widgets::{
    ButtonKind, add_button, badge, button, caret, field_label, input_box, menu_card, menu_row,
    one_line, paragraph, profiles_hint, tag_frame,
};

/// The rule column's width, and the gutter the curves cross.
const RULE_COLUMN: f32 = 360.0;
const GUTTER: f32 = 96.0;
/// The share column shows on cards at least this wide.
const SHARE_MIN: f32 = 360.0;
/// The rail, and the pane's and the map's padding and borders, for the
/// cards' width at a window width.
const RAIL: f32 = 197.0;
const PANE_X: f32 = 48.0;
const MAP_X: f32 = 34.0;

/// Hover fades and dims over this.
const FADE: Duration = Duration::from_millis(120);
/// Today's numbers count up over this the first time they show.
const TICK: Duration = Duration::from_millis(400);
/// The override's hand-off curve.
const HANDOFF: Duration = Duration::from_millis(900);
/// What is not lit, while something is.
const DIM: f32 = 0.45;

/// The element ids tests reach.
pub const PANE_ID: &str = "overview-pane";
pub const HERO_ACTION_ID: &str = "overview-hero-action";
pub const CALLOUT_BUTTON_ID: &str = "overview-callout-button";
pub const CALLOUT_UNDO_ID: &str = "overview-callout-undo";

/// Where the rows and cards were painted this frame, for the curves.
#[derive(Debug, Default)]
pub struct MapBounds {
    pub rows: HashMap<End, Bounds<Pixels>>,
    pub cards: HashMap<usize, Bounds<Pixels>>,
    /// The curves the canvas painted last, as (row port, card port).
    pub painted: Vec<(Point<Pixels>, Point<Pixels>)>,
}

/// The screen's own state, on the window.
#[derive(Debug, Default)]
pub struct OverviewState {
    /// Under the pointer.
    pub hovered: Option<Focus>,
    /// A clicked card (or Everything else): lit until Esc.
    pub pinned: Option<Focus>,
    /// Bumps whenever what is lit changes: the old curves fade out and the
    /// new ones in, each layer keyed by it.
    generation: usize,
    /// What was lit before the last change, for the fade-out.
    previous: Option<Focus>,
    /// The callout's switch was just used: Undo stays until the reader
    /// leaves Overview.
    pub just_overridden: bool,
    /// The override's hand-off curve while it plays, by its number: every
    /// switch (and Undo) plays a new one, and it is gone after [`HANDOFF`].
    handoff: Option<usize>,
    handoffs: usize,
    /// Why the callout's last change, or the setup card's last choice, was
    /// not saved: shown on that card until the next one is.
    pub refused: Option<Refused>,
    /// The setup card's browser list is open.
    pub picking: bool,
    /// The browser (in the list's order) whose profiles the list shows in
    /// place of the browsers.
    pub choosing: Option<usize>,
    was_picking: bool,
    /// Today's numbers have counted up once.
    pub ticked: bool,
    pub bounds: Rc<RefCell<MapBounds>>,
    /// Tests and screenshots: the routing state to draw in place of the
    /// router's, the tray's and the OS's.
    pub status: Option<Status>,
}

impl OverviewState {
    /// What is lit: the hover, else the pin.
    pub fn focus(&self) -> Option<Focus> {
        self.hovered.or(self.pinned)
    }

    /// Changes what lit, keeping the previous for the fade.
    fn lit_change(&mut self, change: impl FnOnce(&mut Self)) -> bool {
        let before = self.focus();
        change(self);
        if self.focus() == before {
            return false;
        }
        self.previous = before;
        self.generation += 1;
        true
    }

    pub fn hover(&mut self, focus: Option<Focus>) -> bool {
        self.lit_change(|this| this.hovered = focus)
    }

    /// Leaves `focus` if it is what is hovered.
    fn unhover(&mut self, focus: Focus, to: Option<Focus>) -> bool {
        if self.hovered == Some(focus) {
            self.hover(to)
        } else {
            false
        }
    }

    pub fn pin(&mut self, focus: Option<Focus>) -> bool {
        self.lit_change(|this| this.pinned = focus)
    }

    /// The hand-off playing now, by its number.
    #[cfg(test)]
    pub fn handing_off(&self) -> Option<usize> {
        self.handoff
    }

    /// What is fading out.
    #[cfg(test)]
    pub fn fading(&self) -> Option<Focus> {
        self.previous
    }

    /// The reader leaves Overview: its numbers have counted up, the
    /// callout's Undo is over, nothing stays lit, and nothing fades or
    /// hands off when they come back.
    pub fn leave(&mut self) {
        self.ticked = true;
        self.just_overridden = false;
        self.refused = None;
        self.picking = false;
        self.choosing = None;
        self.hover(None);
        self.pin(None);
        self.previous = None;
        self.handoff = None;
    }
}

/// Which card a change that was not saved came from, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    Callout(String),
    Setup(String),
}

/// The routing state as the router, the tray and the OS report it, or the
/// one a test set.
pub fn status(app: &PwrRoute, cx: &App) -> Status {
    if let Some(status) = &app.overview.status {
        return status.clone();
    }
    let paused = crate::tray::model(cx).and_then(|model| {
        let model = model.read(cx);
        model.is_paused().then(|| Pause {
            name: model.pause_target.name.clone(),
            display: model.pause_target.display(),
        })
    });
    let config_error = app.config.as_ref().and_then(|config| {
        config_trouble(crate::router::routing_source(), config.problem.as_deref())
    });
    Status {
        paused,
        default: crate::default_browser::state(cx),
        config_error,
    }
}

/// What the router says of the config in charge. `fallback` set: nothing
/// has loaded and every link goes to the fallback browser. `error` alone: a
/// reload failed and the last load still routes. Neither, while this window
/// could not read the file (`problem`): the router's copy routes, so it is
/// the stale case, not the broken one.
pub fn config_trouble(
    source: crate::router::RoutingSource,
    problem: Option<&str>,
) -> Option<ConfigTrouble> {
    if let Some(fallback) = source.fallback {
        let why = source.error.or_else(|| problem.map(str::to_owned));
        Some(ConfigTrouble::NotLoaded(first_line(
            why.as_deref().unwrap_or(&fallback),
        )))
    } else if let Some(error) = source.error {
        Some(ConfigTrouble::ReloadFailed(first_line(&error)))
    } else {
        problem.map(|problem| ConfigTrouble::ReloadFailed(first_line(problem)))
    }
}

/// The first line of an error, short enough for a row.
fn first_line(error: &str) -> String {
    let line = error.lines().next().unwrap_or_default().trim();
    if line.chars().count() > 160 {
        let cut: String = line.chars().take(159).collect();
        format!("{cut}\u{2026}")
    } else {
        line.to_owned()
    }
}

/// The screen's model as things stand.
pub fn build(app: &PwrRoute, status: &Status, cx: &App) -> Overview {
    let rules = app.store.edited();
    let tally = crate::activity::feed(cx)
        .map(|feed| feed.read(cx).tally(SystemTime::now()))
        .unwrap_or_default();
    let config = app.config.as_ref().map(|config| ConfigInputs {
        shown: &config.path,
        description: app.described.as_ref(),
    });
    overview_model::build(&Inputs {
        rules: &rules,
        config,
        status,
        installed: &app.installed,
        today: app.activity.today,
        tally: &tally,
        found_config: app.found_config.as_ref().map(|found| found.shown.as_str()),
        just_overridden: app.overview.just_overridden,
    })
}

impl PwrRoute {
    /// A row's click: Rules, with that rule selected and its editor open.
    /// A config handler opens the config's read-only list.
    pub fn open_rule(
        &mut self,
        source: RowSource,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.navigate(Screen::Rules, window, cx);
        if let RowSource::Rule(index) = source {
            if self.config.is_some() {
                self.visual_rules_expanded = true;
            }
            self.select(Selection::Handler(index), cx);
        }
        cx.notify();
    }

    /// The callout's switch: PwrRoute's default takes everything else
    /// (`"precedence": "pwrroute"`), written now; Undo writes it back. A
    /// change the store would not write (or that failed) leaves the
    /// callout where the router is and says why, as the Rules banner does.
    pub fn set_override(&mut self, on: bool, cx: &mut Context<'_, Self>) {
        use pwrroute_engine::Precedence;
        let next = if on {
            Precedence::PwrRoute
        } else {
            Precedence::Config
        };
        if let Err(reason) = self.write_precedence(|_| next, cx) {
            self.overview.just_overridden = false;
            self.overview.handoff = None;
            self.overview.refused = Some(Refused::Callout(reason));
            cx.notify();
            return;
        }
        self.overview.refused = None;
        self.overview.just_overridden = on;
        self.overview.handoffs += 1;
        let handoff = self.overview.handoffs;
        self.overview.handoff = Some(handoff);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HANDOFF).await;
            let _ = this.update(cx, |this, cx| {
                if this.overview.handoff == Some(handoff) {
                    this.overview.handoff = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// The setup card's browser list: `rules.json`'s default browser and
    /// profile, written now. A choice that was not saved keeps the card
    /// open on that step, with the reason.
    pub fn choose_default(&mut self, browser: &str, profile: &str, cx: &mut Context<'_, Self>) {
        self.overview.picking = false;
        self.overview.choosing = None;
        let written = self.write_now(
            |store| {
                let file = store.edited();
                let (was, was_profile) =
                    (file.default_browser.clone(), file.default_profile.clone());
                store.set_default_browser(browser, profile);
                move |store: &mut crate::rules_store::RulesStore| {
                    store.set_default_browser(&was, &was_profile);
                }
            },
            cx,
        );
        self.overview.refused = written.err().map(Refused::Setup);
        cx.notify();
    }

    /// A tile's click: Activity, filtered to what it counts.
    pub fn open_activity(
        &mut self,
        filter: crate::activity::Filter,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.activity.filter = filter;
        self.navigate(Screen::Activity, window, cx);
    }

    fn act(&mut self, action: Action, window: &mut Window, cx: &mut Context<'_, Self>) {
        match action {
            Action::ShowError | Action::TestLink => self.navigate(
                if action == Action::TestLink {
                    Screen::TestLink
                } else {
                    Screen::Rules
                },
                window,
                cx,
            ),
            Action::Resume => {
                // Only while paused: the tray may have resumed already, and
                // the toggle would pause again.
                let paused = crate::tray::model(cx).is_some_and(|model| model.read(cx).is_paused());
                if paused {
                    crate::tray::run(crate::tray::TrayCommand::TogglePause, cx);
                }
                cx.notify();
            }
            Action::MakeDefault => {
                crate::default_browser::handle_settings_request(
                    &crate::settings::model::SettingsRequest::RegisterDefaultBrowser,
                    cx,
                );
            }
            Action::OpenBrowsersSettings => {
                crate::settings::open(cx, Some(crate::settings::Pane::Browsers));
            }
        }
    }
}

/// A tooltip's view: one line on the popover card.
struct Tip(SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        div()
            .px(px(8.0))
            .py(px(5.0))
            .bg(theme::bg_popover())
            .border_1()
            .border_color(theme::border_default())
            .rounded(px(6.0))
            .child(text(self.0.clone(), 11.0, 500.0, theme::text_secondary()))
    }
}

fn tip(label: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let label = label.into();
    move |_, cx| cx.new(|_| Tip(label.clone())).into()
}

fn alpha(color: Hsla, a: f32) -> Hsla {
    Hsla { a, ..color }
}

/// `color` at `a` over `base`, as one opaque color: a gradient stop that
/// does not let the window show through.
fn over(base: Hsla, color: Hsla, a: f32) -> Hsla {
    let base = base.to_rgb();
    let top = color.to_rgb();
    let mix = |b: f32, t: f32| b + (t - b) * a;
    gpui_kit::Rgba {
        r: mix(base.r, top.r),
        g: mix(base.g, top.g),
        b: mix(base.b, top.b),
        a: 1.0,
    }
    .into()
}

fn shadow(color: Hsla, y: f32, blur: f32, spread: f32) -> BoxShadow {
    BoxShadow {
        color,
        offset: point(px(0.0), px(y)),
        blur_radius: px(blur),
        spread_radius: px(spread),
        inset: false,
    }
}

/// `.cap`: 9.5/600 caps, with an optional `small` note.
fn cap(label: &str, small: Option<&str>, color: Hsla) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .child(tracked(label, 9.5, 600.0, 0.14, color))
        .when_some(small, |cap, small| {
            cap.child(text(small.to_owned(), 10.5, 500.0, theme::text_subtle()))
        })
}

fn note(label: impl Into<SharedString>) -> Div {
    text(label, 10.5, 500.0, theme::text_subtle())
}

/// Fades an element between two opacities when its state changed with the
/// last change of what is lit; static otherwise.
fn faded(
    element: impl IntoElement + gpui_kit::Styled + 'static,
    id: ElementId,
    from: f32,
    to: f32,
) -> AnyElement {
    if (from - to).abs() < f32::EPSILON {
        element.opacity(to).into_any_element()
    } else {
        element
            .with_animation(
                id,
                Animation::new(FADE).with_easing(ease_out_quint()),
                move |element, t| element.opacity(from + (to - from) * t),
            )
            .into_any_element()
    }
}

/// The pane.
pub fn pane(
    app: &PwrRoute,
    status: &Status,
    window: &mut Window,
    cx: &mut Context<'_, PwrRoute>,
) -> Stateful<Div> {
    let overview = build(app, status, cx);
    let width = f32::from(window.viewport_size().width);
    let cards_width = width - RAIL - PANE_X - MAP_X - RULE_COLUMN - GUTTER;
    let wide = cards_width >= SHARE_MIN;
    pane_frame(PANE_ID)
        .debug_selector(|| PANE_ID.into())
        .child(hero(&overview, cx))
        .children(
            overview
                .setup
                .as_ref()
                .map(|setup| setup_card(app, setup, cx)),
        )
        .child(today(app, &overview, cx))
        .child(map(app, &overview.map, wide, cx))
        .children(
            overview
                .callout
                .as_ref()
                .map(|callout| callout_card(app, callout, cx)),
        )
}

/// Why a change on this screen was not saved (`.errl`).
fn refused_line(id: &'static str, reason: &str) -> Div {
    div().debug_selector(move || id.into()).flex().child(
        div()
            .py(px(6.0))
            .px(px(8.0))
            .rounded(px(5.0))
            .bg(theme::danger_soft())
            .child(paragraph(
                crate::settings::configuration_host::not_saved(reason),
                10.5,
                14.7,
                theme::danger_text(),
            )),
    )
}

/// The callout's "not saved" line.
pub const CALLOUT_REFUSED_ID: &str = "overview-callout-refused";
/// The setup card's "not saved" line.
pub const SETUP_REFUSED_ID: &str = "overview-setup-refused";

// --- the hero ------------------------------------------------------------

fn hero(overview: &Overview, cx: &mut Context<'_, PwrRoute>) -> Div {
    let hero = &overview.hero;
    let (border, wash, orb_fg, orb_bg, orb_border) = match hero.tone {
        Tone::Accent => (
            theme::accent_border(),
            Some(theme::accent()),
            theme::accent(),
            theme::accent_soft(),
            theme::accent_border(),
        ),
        Tone::Warn => (
            alpha(theme::status_warning(), 0.4),
            Some(theme::status_warning()),
            theme::status_warning(),
            theme::warn_soft(),
            alpha(theme::status_warning(), 0.4),
        ),
        Tone::Neutral => (
            theme::border_default(),
            None,
            theme::text_muted(),
            theme::bg_elevated(),
            theme::border_default(),
        ),
        Tone::Danger => (
            alpha(theme::status_error(), 0.5),
            Some(theme::status_error()),
            theme::danger_text(),
            theme::danger_soft(),
            theme::transparent(),
        ),
    };
    let orb_icon = match hero.state {
        State::Paused => "pwrroute/pause.svg",
        State::ConfigError | State::ConfigStale => "pwrroute/alert.svg",
        _ => "pwrroute/mark.svg",
    };
    let orb_size = if orb_icon == "pwrroute/mark.svg" {
        30.0
    } else {
        24.0
    };
    let orb = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(56.0))
        .rounded(px(14.0))
        .bg(orb_bg)
        .border_1()
        .border_color(orb_border)
        .when(hero.tone == Tone::Accent, |orb| {
            orb.shadow(vec![
                shadow(alpha(theme::accent(), 0.06), 0.0, 0.0, 4.0),
                shadow(alpha(theme::accent(), 0.2), 0.0, 28.0, 0.0),
            ])
        })
        .child(icon(orb_icon, orb_size, orb_fg));
    // The detail's separators are subtle, as `.ht span em`.
    let mut detail = String::new();
    let mut separators = Vec::new();
    for (i, part) in hero.detail.iter().enumerate() {
        if i > 0 {
            let start = detail.len();
            detail.push_str(" \u{00b7} ");
            separators.push((
                start..detail.len(),
                HighlightStyle {
                    color: Some(theme::text_subtle()),
                    ..Default::default()
                },
            ));
        }
        detail.push_str(part);
    }
    let words = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(7.0))
        .child(
            div()
                .debug_selector(|| "overview-hero-sentence".into())
                .font_family(theme::SANS)
                .font_weight(theme::weight(700.0))
                .text_size(px(24.0))
                .line_height(px(27.6))
                .text_color(theme::text_primary())
                .child(hero.sentence.clone()),
        )
        .when(!detail.is_empty(), |words| {
            words.child(
                div()
                    .font_family(theme::SANS)
                    .font_weight(theme::weight(500.0))
                    .text_size(px(11.5))
                    .line_height(px(17.25))
                    .text_color(theme::text_muted())
                    .child(StyledText::new(detail).with_highlights(separators)),
            )
        });
    let action = hero.action.map(|action| {
        let kind = if action.primary() {
            ButtonKind::Primary
        } else {
            ButtonKind::Default
        };
        button(action.label(), kind, false)
            .id(HERO_ACTION_ID)
            .debug_selector(|| HERO_ACTION_ID.into())
            .on_click(cx.listener(move |this, _, window, cx| this.act(action, window, cx)))
    });
    let background = match wash {
        Some(wash) => linear_gradient(
            90.0,
            linear_color_stop(over(theme::bg_panel(), wash, 0.11), 0.0),
            linear_color_stop(theme::bg_panel(), 0.72),
        ),
        None => theme::bg_panel().into(),
    };
    div()
        .debug_selector(|| "overview-hero".into())
        .flex()
        .flex_none()
        .items_center()
        .gap(px(16.0))
        .min_h(px(96.0))
        .px(px(20.0))
        .py(px(18.0))
        .rounded(px(14.0))
        .border_1()
        .border_color(border)
        .bg(background)
        .child(orb)
        .child(words)
        .children(action)
}

// --- the setup card --------------------------------------------------------

fn step_number(step: &SetupStep, number: usize, current: bool) -> Div {
    let (bg, border, fg, label) = if step.done {
        (
            theme::success_soft(),
            theme::transparent(),
            theme::success_text(),
            "\u{2713}".to_owned(),
        )
    } else if current {
        (
            theme::accent(),
            theme::transparent(),
            theme::accent_on(),
            number.to_string(),
        )
    } else {
        (
            theme::bg_elevated(),
            theme::border_default(),
            theme::text_muted(),
            number.to_string(),
        )
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(26.0))
        .rounded_full()
        .border_1()
        .border_color(border)
        .bg(bg)
        .child(mono(label, 11.0, 600.0, fg))
}

fn step_frame(step: &SetupStep, number: usize, current: bool) -> Div {
    div()
        .relative()
        .flex()
        .items_center()
        .gap(px(14.0))
        .px(px(16.0))
        .py(px(14.0))
        .rounded(px(10.0))
        .border_1()
        .map(|frame| {
            if current {
                frame
                    .border_color(theme::accent_border())
                    .bg(theme::bg_row_active())
            } else {
                frame
                    .border_color(theme::border_subtle())
                    .bg(theme::bg_panel())
            }
        })
        .child(
            div()
                .w(px(28.0))
                .flex_none()
                .child(step_number(step, number, current)),
        )
}

fn step_words(step: &SetupStep) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(5.0))
        .child(text(step.title, 13.0, 600.0, theme::text_primary()))
        .when_some(step.result.clone(), |words, (badge_label, label)| {
            words.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(badge(badge_label))
                    .child(text(label, 11.5, 500.0, theme::text_secondary())),
            )
        })
        .when(step.result.is_none(), |words| {
            words.when_some(step.body.clone(), |words, body| {
                words.child(paragraph(body, 11.5, 17.8, theme::text_muted()))
            })
        })
}

/// The setup card's select width: the field's, and the list's under
/// "Change…".
const SELECT_WIDTH: f32 = 220.0;

/// The browser list the setup card's select opens, as first run's: one
/// row per browser with its profiles as the hint; a browser with profiles
/// shows its profile list in place of the browsers ("Default profile",
/// then each). It is the field's width, under "Change…" too, where the
/// popover aligns it to the right.
fn browser_menu(app: &PwrRoute, cx: &mut Context<'_, PwrRoute>) -> Stateful<Div> {
    let browsers: Vec<&Installed> = app
        .installed
        .iter()
        .filter(|browser| !browser.hidden && !browser.name.eq_ignore_ascii_case("PwrRoute"))
        .collect();
    let menu = menu_card("overview-setup-menu")
        .debug_selector(|| "overview-setup-menu".into())
        .w(px(SELECT_WIDTH))
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.overview.picking = false;
            this.overview.choosing = None;
            cx.notify();
        }));
    match app.overview.choosing.and_then(|index| browsers.get(index)) {
        Some(browser) => {
            let name = browser.name.clone();
            let profiles = std::iter::once(String::new()).chain(browser.profiles.iter().cloned());
            profiles.enumerate().fold(menu, |menu, (index, profile)| {
                let name = name.clone();
                let label = if profile.is_empty() {
                    "Default profile".to_owned()
                } else {
                    profile.clone()
                };
                menu.child(
                    menu_row(("overview-setup-profile", index), None, label, None)
                        .debug_selector(move || format!("overview-setup-profile-{index}"))
                        .hover(|row| row.bg(theme::bg_hover()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.choose_default(&name, &profile, cx);
                        })),
                )
            })
        }
        None => browsers
            .iter()
            .enumerate()
            .fold(menu, |menu, (index, browser)| {
                let name = browser.name.clone();
                let has_profiles = !browser.profiles.is_empty();
                menu.child(
                    menu_row(
                        ("overview-setup-choice", index),
                        Some(crate::widgets::badge_for(&name)),
                        name.clone(),
                        profiles_hint(&browser.profiles).map(SharedString::from),
                    )
                    .debug_selector(move || format!("overview-setup-choice-{index}"))
                    .hover(|row| row.bg(theme::bg_hover()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if has_profiles {
                            this.overview.choosing = Some(index);
                            cx.notify();
                        } else {
                            this.choose_default(&name, "", cx);
                        }
                    })),
                )
            }),
    }
}

fn setup_card(app: &PwrRoute, setup: &Setup, cx: &mut Context<'_, PwrRoute>) -> Div {
    let done = setup.done();
    let progress = div()
        .flex()
        .items_center()
        .gap(px(4.0))
        .children((0..2).map(|i| {
            div()
                .w(px(22.0))
                .h(px(4.0))
                .rounded(px(2.0))
                .bg(if i < done {
                    theme::accent()
                } else {
                    theme::border_default()
                })
        }))
        .child(note(format!("{done} of 2 done")).ml(px(6.0)));
    let first_current = !setup.everything_else.done;
    let picking = app.overview.picking;
    let toggle = cx.listener(|this, _, _, cx| {
        this.overview.picking = !this.overview.was_picking;
        this.overview.choosing = None;
        cx.notify();
    });
    let remember = cx.listener(|this, _, _, _| {
        this.overview.was_picking = this.overview.picking;
    });
    let choose: AnyElement = if setup.everything_else.done {
        button("Change\u{2026}", ButtonKind::Quiet, true)
            .id("overview-setup-change")
            .debug_selector(|| "overview-setup-change".into())
            .capture_any_mouse_down(remember)
            .on_click(toggle)
            .into_any_element()
    } else {
        input_box()
            .id("overview-setup-choose")
            .debug_selector(|| "overview-setup-choose".into())
            .w(px(SELECT_WIDTH))
            .cursor_pointer()
            .child(field_label("Choose a browser", theme::text_subtle()))
            .child(caret())
            .capture_any_mouse_down(remember)
            .on_click(toggle)
            .into_any_element()
    };
    let control = div()
        .relative()
        .flex_none()
        .child(choose)
        .when(picking, |control| {
            // A popover under the control that stays in the window: under
            // the field from its left edge, under "Change…" from its right.
            let field = !setup.everything_else.done;
            let corner = div().absolute().top_0().map(|corner| {
                if field {
                    corner.left_0()
                } else {
                    corner.right_0()
                }
            });
            control.child(
                corner.child(
                    deferred(
                        anchored()
                            .anchor(if field {
                                Anchor::TopLeft
                            } else {
                                Anchor::TopRight
                            })
                            .offset(point(px(0.0), px(34.0)))
                            .snap_to_window_with_margin(px(8.0))
                            .child(browser_menu(app, cx)),
                    )
                    .with_priority(1),
                ),
            )
        });
    let refused = match &app.overview.refused {
        Some(Refused::Setup(reason)) => Some(reason.clone()),
        _ => None,
    };
    let first = step_frame(&setup.everything_else, 1, first_current)
        .child(
            step_words(&setup.everything_else).when_some(refused, |words, reason| {
                words.child(refused_line(SETUP_REFUSED_ID, &reason).mt(px(4.0)))
            }),
        )
        .child(control);
    let second_current = setup.everything_else.done && !setup.default_browser.done;
    let second = step_frame(&setup.default_browser, 2, second_current)
        .child(step_words(&setup.default_browser))
        .when(!setup.default_browser.done, |step| {
            step.child(
                button(Action::MakeDefault.label(), ButtonKind::Primary, false)
                    .id("overview-setup-default")
                    .debug_selector(|| "overview-setup-default".into())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.act(Action::MakeDefault, window, cx);
                    })),
            )
        });
    div()
        .debug_selector(|| "overview-setup".into())
        .flex()
        .flex_none()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(cap("Finish setting up", None, theme::text_muted()))
                .child(progress),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(first)
                .child(second),
        )
}

// --- today -------------------------------------------------------------------

fn today(app: &PwrRoute, overview: &Overview, cx: &mut Context<'_, PwrRoute>) -> Div {
    let tick = !app.overview.ticked;
    let tiles = overview.tiles.iter().enumerate().map(|(i, tile)| {
        let filter = tile.filter;
        let color = if tile.danger {
            theme::danger_text()
        } else if tile.count == 0 {
            theme::text_subtle()
        } else {
            theme::text_primary()
        };
        let count = tile.count;
        let number = div().h(px(22.0)).flex().items_center();
        let number: AnyElement = if tick && count > 0 {
            number
                .with_animation(
                    ("overview-tick", i),
                    Animation::new(TICK).with_easing(ease_out_quint()),
                    move |number, t| {
                        number.child(tile_number((count as f32 * t).round() as u64, color))
                    },
                )
                .into_any_element()
        } else {
            number.child(tile_number(count, color)).into_any_element()
        };
        div()
            .id(("overview-tile", i))
            .debug_selector(move || format!("overview-tile-{i}"))
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(9.0))
            .px(px(14.0))
            .py(px(12.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(theme::border_subtle())
            .bg(theme::bg_panel())
            .cursor_pointer()
            .hover(|tile| {
                tile.border_color(theme::accent_border())
                    .bg(theme::bg_row_active())
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                this.open_activity(filter, window, cx);
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .overflow_hidden()
                    .child(tracked(tile.label, 9.5, 600.0, 0.14, theme::text_muted()))
                    .child(text("\u{203a}", 13.0, 500.0, theme::text_subtle())),
            )
            .child(number)
    });
    div()
        .flex()
        .flex_none()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(cap("Today", None, theme::text_muted()))
                .child(note("Since PwrRoute started")),
        )
        .child(div().flex().gap(px(10.0)).children(tiles))
}

fn tile_number(n: u64, color: Hsla) -> Div {
    text(crate::activity::thousands(n), 22.0, 700.0, color)
}

// --- the map -----------------------------------------------------------------

/// The lit state now and before the last change, for one element.
#[derive(Clone, Copy)]
struct Light {
    now: Option<bool>,
    before: Option<bool>,
}

impl Light {
    fn lit(self) -> bool {
        self.now == Some(true)
    }

    /// Its opacity before and after: dimmed while something else is lit.
    fn opacities(self) -> (f32, f32) {
        let of = |state: Option<bool>| match state {
            Some(false) => DIM,
            _ => 1.0,
        };
        (of(self.before), of(self.now))
    }
}

fn map(app: &PwrRoute, map: &Map, wide: bool, cx: &mut Context<'_, PwrRoute>) -> Div {
    let focus = app.overview.focus();
    let lit = focus.map(|focus| map.lit(focus));
    let before = app.overview.previous.map(|focus| map.lit(focus));
    let generation = app.overview.generation;
    let light = |is: &dyn Fn(&Lit) -> bool| Light {
        now: lit.as_ref().map(is),
        before: before.as_ref().map(is),
    };

    // The header.
    let (left_cap, left_small, left_color) = match map.caption {
        Caption::InOrder => ("In order", Some("first match wins"), theme::text_muted()),
        Caption::WhenResumes => ("When routing resumes", None, theme::warn_text()),
        Caption::WhatWillHappen => (
            "What will happen",
            Some("first match wins"),
            theme::text_muted(),
        ),
    };
    let header = div()
        .flex()
        .items_center()
        .h(px(40.0))
        .px(px(16.0))
        .border_b_1()
        .border_color(theme::border_faint())
        .child(
            div()
                .w(px(RULE_COLUMN))
                .flex_none()
                .flex()
                .items_center()
                .justify_between()
                .pr(px(12.0))
                .child(cap(left_cap, left_small, left_color))
                .when(map.counts, |c1| c1.child(note("today"))),
        )
        .child(
            div()
                .ml(px(GUTTER))
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .child(cap(
                    "Destinations",
                    wide.then_some("where links open"),
                    theme::text_muted(),
                ))
                .when_some(map.not_used.clone(), |c2, not_used| {
                    c2.child(div().min_w_0().overflow_hidden().child(one_line(
                        not_used,
                        theme::SANS,
                        500.0,
                        10.5,
                        10.5,
                        theme::text_subtle(),
                    )))
                }),
        );

    // The rule column, and which child is which row for the curves.
    let mut ends: Vec<Option<End>> = Vec::new();
    let mut left: Vec<AnyElement> = Vec::new();
    if let Some(rewrites) = &map.rewrites {
        ends.push(None);
        left.push(rewrites_band(rewrites.clone(), cx).into_any_element());
    }
    if let Some(empty) = &map.empty {
        ends.push(None);
        left.push(empty_prompt(empty, cx).into_any_element());
    }
    let mut after_row = false;
    for item in &map.items {
        match item {
            Item::Header { label, note: right } => {
                ends.push(None);
                left.push(
                    section_header(label.clone(), right.clone(), after_row).into_any_element(),
                );
                after_row = false;
            }
            Item::Error { title, detail } => {
                ends.push(None);
                left.push(error_row(title.clone(), detail.clone()).into_any_element());
                after_row = true;
            }
            Item::Row(index) => {
                let row = &map.rows[*index];
                let index = *index;
                let light = light(&|lit: &Lit| lit.row(index));
                ends.push(Some(End::Row(index)));
                let drawn = rule_row(index, row, light, generation, cx);
                // Nothing runs it (the config never loaded): faded, as a
                // row that is not lit.
                left.push(if row.idle {
                    div().opacity(DIM).child(drawn).into_any_element()
                } else {
                    drawn
                });
                after_row = true;
            }
        }
    }
    ends.push(Some(End::EverythingElse));
    left.push(everything_else_row(
        &map.everything_else,
        light(&|lit: &Lit| lit.everything_else),
        generation,
        !map.items.is_empty() || map.empty.is_some(),
        cx,
    ));

    let bounds = app.overview.bounds.clone();
    let rows_bounds = bounds.clone();
    let rule_column = div()
        .on_children_prepainted(move |children, _, _| {
            let mut bounds = rows_bounds.borrow_mut();
            bounds.rows.clear();
            for (child, end) in children.iter().zip(&ends) {
                if let Some(end) = end {
                    bounds.rows.insert(*end, *child);
                }
            }
        })
        .relative()
        .w(px(RULE_COLUMN))
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .children(left);

    let cards_bounds = bounds.clone();
    let cards = map.cards.iter().enumerate().map(|(index, card)| {
        let drawn = destination_card(
            index,
            card,
            light(&|lit: &Lit| lit.card(index)),
            &|pill| light(&move |lit: &Lit| lit.pill(index, pill)),
            generation,
            wide,
            app.overview.ticked,
            cx,
        );
        // Fed only by rows nothing runs.
        if card.idle {
            div().opacity(DIM).child(drawn).into_any_element()
        } else {
            drawn
        }
    });
    let card_column = div()
        .on_children_prepainted(move |children, _, _| {
            let mut bounds = cards_bounds.borrow_mut();
            bounds.cards = children.iter().copied().enumerate().collect();
        })
        .relative()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .children(cards);

    // The curves for what is lit now, fading in, over the ones for what
    // was lit before, fading out; and the override's hand-off.
    let curves_now = lit
        .as_ref()
        .map(|lit| lit.curves.clone())
        .unwrap_or_default();
    let curves_before = before
        .as_ref()
        .map(|lit| lit.curves.clone())
        .unwrap_or_default();
    let handoff = app.overview.handoff.and_then(|_| {
        map.everything_else
            .target
            .card()
            .map(|card| vec![(End::EverythingElse, card)])
    });
    let layer = |curves: Vec<(End, usize)>, record: bool| {
        let bounds = bounds.clone();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| paint_curves(&curves, &bounds, record, window),
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    };
    let fading = |curves: Vec<(End, usize)>, id: ElementId, from: f32, to: f32, record: bool| {
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(layer(curves, record))
            .with_animation(
                id,
                Animation::new(FADE).with_easing(ease_out_quint()),
                move |layer, t| layer.opacity(from + (to - from) * t),
            )
    };

    let body = div()
        .debug_selector(|| "overview-map-body".into())
        .relative()
        .flex()
        .gap(px(GUTTER))
        .p(px(16.0))
        .when(map.dimmed, |body| body.opacity(0.4))
        .child(
            canvas(
                |_, _, _| {},
                |bounds, _, window, _| paint_dots(bounds, window),
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .when(!curves_before.is_empty(), |body| {
            body.child(fading(
                curves_before,
                ("overview-curves-out", generation).into(),
                1.0,
                0.0,
                false,
            ))
        })
        .when(!curves_now.is_empty(), |body| {
            body.child(fading(
                curves_now,
                ("overview-curves-in", generation).into(),
                0.0,
                1.0,
                true,
            ))
        })
        .when(lit.is_none() || curves_now_is_empty(&lit), |body| {
            // Nothing is drawn: forget the last curves.
            bounds.borrow_mut().painted.clear();
            body
        })
        .when_some(handoff, |body, curves| {
            body.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .child(layer(curves, false))
                    .with_animation(
                        ("overview-handoff", app.overview.handoff.unwrap_or_default()),
                        Animation::new(HANDOFF),
                        // Up quickly, held, then gone.
                        |layer, t| layer.opacity(((1.0 - t) * 3.0).min(1.0) * (t * 6.0).min(1.0)),
                    ),
            )
        })
        .child(rule_column)
        .child(card_column);

    div()
        .debug_selector(|| "overview-map".into())
        .flex_none()
        .rounded(px(14.0))
        .border_1()
        .border_color(theme::border_subtle())
        .bg(theme::bg_panel())
        .child(header)
        .child(body)
}

fn curves_now_is_empty(lit: &Option<Lit>) -> bool {
    lit.as_ref().is_none_or(|lit| lit.curves.is_empty())
}

/// The dot grid: 1px dots every 16px at 5%, offset 8.
fn paint_dots(bounds: Bounds<Pixels>, window: &mut Window) {
    let color = alpha(theme::text_primary(), 0.05);
    let step = 16.0;
    let mut y = f32::from(bounds.origin.y) + 8.0;
    let bottom = f32::from(bounds.bottom());
    let right = f32::from(bounds.right());
    while y < bottom {
        let mut x = f32::from(bounds.origin.x) + 8.0;
        while x < right {
            window.paint_quad(gpui_kit::fill(
                Bounds::new(
                    point(px(x - 0.5), px(y - 0.5)),
                    gpui_kit::size(px(1.0), px(1.0)),
                ),
                color,
            ));
            x += step;
        }
        y += step;
    }
}

/// One curve per lit route: a 6px glow at 16%, then the 1.5px line.
fn paint_curves(
    curves: &[(End, usize)],
    bounds: &Rc<RefCell<MapBounds>>,
    record: bool,
    window: &mut Window,
) {
    let mut painted = Vec::new();
    {
        let bounds = bounds.borrow();
        for (end, card) in curves {
            let (Some(row), Some(card)) = (bounds.rows.get(end), bounds.cards.get(card)) else {
                continue;
            };
            let from = point(row.right(), row.center().y);
            let to = point(card.left(), card.center().y);
            for (width, color) in [(6.0, alpha(theme::accent(), 0.16)), (1.5, theme::accent())] {
                let mut path = PathBuilder::stroke(px(width));
                path.move_to(from);
                path.cubic_bezier_to(
                    to,
                    point(from.x + px(56.0), from.y),
                    point(to.x - px(56.0), to.y),
                );
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            }
            painted.push((from, to));
        }
    }
    if record {
        bounds.borrow_mut().painted = painted;
    }
}

/// `.port`: the 9px dot on a row's right edge or a card's left.
fn port(right: bool, lit: bool) -> Div {
    let dot = div()
        .absolute()
        .top(px(-4.5))
        .size(px(9.0))
        .rounded_full()
        .border(px(1.5))
        .map(|dot| {
            if lit {
                dot.bg(theme::accent()).border_color(theme::accent())
            } else {
                dot.bg(theme::bg_panel())
                    .border_color(theme::border_strong())
            }
        });
    // Centred on the edge: a zero-height anchor at the middle.
    div()
        .absolute()
        .top(gpui_kit::relative(0.5))
        .map(|anchor| {
            if right {
                anchor.right(px(-5.0))
            } else {
                anchor.left(px(-5.0))
            }
        })
        .w(px(9.0))
        .h(px(0.0))
        .child(dot.left_0())
}

fn rewrites_band(label: String, cx: &mut Context<'_, PwrRoute>) -> Stateful<Div> {
    div()
        .id("overview-rewrites")
        .debug_selector(|| "overview-rewrites".into())
        .h(px(32.0))
        .mb(px(6.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .px(px(10.0))
        .border_1()
        .border_dashed()
        .border_color(theme::border_default())
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(|band| band.bg(theme::bg_hover()))
        .on_click(cx.listener(|this, _, window, cx| {
            this.navigate(Screen::Rules, window, cx);
        }))
        .child(icon("pwrroute/wand.svg", 13.0, theme::accent()))
        .child(div().flex_1().min_w_0().child(one_line(
            label,
            theme::SANS,
            500.0,
            11.0,
            11.0,
            theme::text_muted(),
        )))
        .child(text("\u{203a}", 13.0, 500.0, theme::text_subtle()))
}

fn section_header(label: String, right: String, after_row: bool) -> Div {
    div()
        .h(px(24.0))
        .when(after_row, |header| header.mt(px(8.0)))
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .px(px(2.0))
        .child(text(label, 11.0, 600.0, theme::text_secondary()))
        .child(note(right))
}

fn error_row(title: String, detail: String) -> Div {
    div()
        .debug_selector(|| "overview-config-error".into())
        .relative()
        .h(px(44.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .pl(px(10.0))
        .pr(px(12.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(alpha(theme::status_error(), 0.5))
        .bg(theme::danger_soft())
        .child(
            div()
                .w(px(18.0))
                .flex_none()
                .child(mono("!", 13.0, 600.0, theme::danger_text())),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(one_line(
                    title,
                    theme::SANS,
                    600.0,
                    12.0,
                    12.0,
                    theme::danger_text(),
                ))
                .child(one_line(
                    detail,
                    theme::mono_family(500.0),
                    500.0,
                    10.0,
                    10.0,
                    theme::text_muted(),
                )),
        )
}

/// The row's frame: 40 high (44 for Everything else), lit or not.
fn row_frame(id: ElementId, lit: bool, ghost: bool) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .h(px(40.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .pl(px(10.0))
        .pr(px(12.0))
        .rounded(px(8.0))
        .border_1()
        .cursor_pointer()
        .map(|row| {
            if lit {
                row.border_color(theme::accent_border())
                    .bg(theme::bg_row_active())
                    .shadow(vec![shadow(alpha(theme::accent(), 0.08), 0.0, 0.0, 3.0)])
            } else if ghost {
                row.border_dashed()
                    .border_color(theme::border_default())
                    .bg(theme::transparent())
            } else {
                row.border_color(theme::border_faint())
                    .bg(theme::bg_elevated())
            }
        })
}

fn rank(label: String, lit: bool, size: f32, color: Hsla) -> Div {
    div().w(px(18.0)).flex_none().child(mono(
        label,
        size,
        600.0,
        if lit { theme::accent_bright() } else { color },
    ))
}

/// A pattern tag that ends in an ellipsis, never narrower than 24px.
fn pattern_tag(label: String, regex: bool) -> Div {
    let tag = tag_frame()
        .min_w(px(24.0))
        .flex_shrink(1.0)
        .overflow_hidden();
    let tag = if regex {
        tag.bg(theme::info_soft())
            .border_color(theme::transparent())
    } else {
        tag
    };
    tag.child(one_line(
        label,
        theme::mono_family(500.0),
        500.0,
        10.5,
        10.5,
        if regex {
            theme::info_text()
        } else {
            theme::text_secondary()
        },
    ))
}

fn more(n: usize) -> Option<Div> {
    (n > 0).then(|| mono(format!("+{n}"), 10.5, 500.0, theme::text_subtle()))
}

fn fn_mark() -> Div {
    tag_frame()
        .bg(theme::accent_soft())
        .border_color(theme::transparent())
        .child(text("\u{0192}", 11.0, 600.0, theme::accent_bright()))
}

fn match_column(row: &Row) -> Div {
    let column = div()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(5.0))
        .overflow_hidden();
    let column = match &row.matcher {
        RowMatch::Pattern {
            text: t,
            regex,
            more: n,
        } => column
            .child(pattern_tag(t.clone(), *regex))
            .children(more(*n)),
        RowMatch::Opener {
            badge: b,
            name,
            more: n,
        } => column
            .child(text("from", 11.0, 500.0, theme::text_muted()))
            .child(tag_frame().child(badge(b.clone())).child(mono(
                name.clone(),
                10.5,
                500.0,
                theme::text_primary(),
            )))
            .children(more(*n)),
        RowMatch::Function { name, more: n } => column
            .child(fn_mark())
            .child(match name {
                Some(name) => mono(name.clone(), 11.0, 500.0, theme::text_secondary()),
                None => text("unnamed function", 11.0, 500.0, theme::text_muted()),
            })
            .children(more(*n)),
        RowMatch::Other { text: t, more: n } => column
            .child(text(t.clone(), 11.0, 500.0, theme::text_muted()))
            .children(more(*n)),
        RowMatch::Any => column.child(text("every link", 11.0, 500.0, theme::text_muted())),
    };
    column.when(!row.enabled, |column| {
        column.child(crate::widgets::chip("off").py(px(3.0)).px(px(7.0)))
    })
}

fn target_column(target: &Target) -> Div {
    let to = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .child(text("\u{2192}", 11.5, 500.0, theme::text_subtle()));
    match target {
        Target::Browser {
            badge: b, label, ..
        } => to.child(badge(b.clone())).child(text(
            label.clone(),
            11.5,
            500.0,
            theme::text_secondary(),
        )),
        Target::Function { .. } => {
            to.child(fn_mark())
                .child(text("JavaScript", 11.5, 500.0, theme::text_secondary()))
        }
        Target::NotOpened { .. } => to.child(text(
            crate::widgets::NOT_OPENED,
            11.5,
            500.0,
            theme::text_secondary(),
        )),
        Target::Ghost { badge: b, name, .. } => to
            .child(badge(b.clone()).border_dashed())
            .child(text(name.clone(), 11.5, 500.0, theme::text_subtle()))
            .child(mono("not installed", 10.0, 500.0, theme::warn_text())),
        Target::Unset => to.child(text("Choose a browser", 11.5, 500.0, theme::text_subtle())),
    }
}

fn count_column(count: Option<u64>) -> Div {
    div()
        .w(px(36.0))
        .flex_none()
        .flex()
        .justify_end()
        .children(count.map(|n| {
            mono(
                crate::activity::thousands(n),
                10.5,
                500.0,
                theme::text_subtle(),
            )
        }))
}

fn rule_row(
    index: usize,
    row: &Row,
    light: Light,
    generation: usize,
    cx: &mut Context<'_, PwrRoute>,
) -> AnyElement {
    let lit = light.lit();
    let ghost = matches!(row.target, Target::Ghost { .. });
    let source = row.source;
    let focus = Focus::Row(index);
    let element = row_frame(("overview-row", index).into(), lit, ghost)
        .debug_selector(move || format!("overview-row-{index}"))
        .tooltip(tip(row.tooltip.clone()))
        .on_hover(cx.listener(move |this, over: &bool, _, cx| {
            let changed = if *over {
                this.overview.hover(Some(focus))
            } else {
                this.overview.unhover(focus, None)
            };
            if changed {
                cx.notify();
            }
        }))
        .on_click(cx.listener(move |this, _, window, cx| {
            this.open_rule(source, window, cx);
        }))
        .child(rank(
            format!("{:02}", row.rank),
            lit,
            10.5,
            theme::text_subtle(),
        ))
        .child(match_column(row))
        .child(target_column(&row.target))
        .child(count_column(row.count));
    // The port sits over the row's border: a sibling, painted after it.
    let element = div()
        .relative()
        .flex_none()
        .child(element)
        .when(row.target.card().is_some(), |row| {
            row.child(port(true, lit))
        });
    // A switched-off rule stays at 40%, and dims with the rest.
    let (from, to) = light.opacities();
    let scale = if row.enabled { 1.0 } else { 0.4 };
    faded(
        element,
        ("overview-row-dim", index * 1_000_000 + generation).into(),
        from * scale,
        to * scale,
    )
}

fn everything_else_row(
    row: &EverythingElse,
    light: Light,
    generation: usize,
    after: bool,
    cx: &mut Context<'_, PwrRoute>,
) -> AnyElement {
    let lit = light.lit();
    let element = row_frame("overview-everything-else".into(), lit, false)
        .debug_selector(|| "overview-everything-else".into())
        .tooltip(tip("Everything else goes here"))
        .h(px(44.0))
        .when(!lit, |row| {
            row.border_color(theme::accent_border()).bg(linear_gradient(
                90.0,
                linear_color_stop(over(theme::bg_elevated(), theme::accent(), 0.10), 0.0),
                linear_color_stop(theme::bg_elevated(), 0.7),
            ))
        })
        .on_hover(cx.listener(|this, over: &bool, _, cx| {
            let changed = if *over {
                this.overview.hover(Some(Focus::EverythingElse))
            } else {
                this.overview.unhover(Focus::EverythingElse, None)
            };
            if changed {
                cx.notify();
            }
        }))
        .on_click(cx.listener(|this, _, _, cx| {
            let pin = (this.overview.pinned != Some(Focus::EverythingElse))
                .then_some(Focus::EverythingElse);
            if this.overview.pin(pin) {
                cx.notify();
            }
        }))
        .child(rank("\u{221e}".into(), true, 13.0, theme::accent_bright()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(text("Everything else", 12.0, 600.0, theme::text_primary()))
                .child(one_line(
                    row.owner.clone(),
                    theme::mono_family(500.0),
                    500.0,
                    10.0,
                    10.0,
                    theme::text_muted(),
                )),
        )
        .child(target_column(&row.target))
        .child(count_column(row.count));
    let element = div()
        .relative()
        .flex_none()
        .when(after, |row| row.mt(px(8.0)))
        .child(element)
        .when(row.target.card().is_some(), |row| {
            row.child(port(true, lit))
        });
    let (from, to) = light.opacities();
    faded(element, ("overview-ee-dim", generation).into(), from, to)
}

fn empty_prompt(empty: &overview_model::Empty, cx: &mut Context<'_, PwrRoute>) -> Div {
    div()
        .debug_selector(|| "overview-empty".into())
        .flex()
        .flex_col()
        .items_start()
        .gap(px(8.0))
        .p(px(16.0))
        .rounded(px(10.0))
        .border_1()
        .border_dashed()
        .border_color(theme::border_default())
        .bg(theme::bg_elevated())
        .child(
            div()
                .font_family(theme::SANS)
                .font_weight(theme::weight(600.0))
                .text_size(px(13.5))
                .line_height(px(16.2))
                .text_color(theme::text_primary())
                .child(empty.title.clone()),
        )
        .child(
            paragraph(
                "Rules send matching links somewhere else: work docs to a work profile, local \
                 servers to a dev browser. First match wins.",
                11.5,
                17.8,
                theme::text_muted(),
            )
            .mb(px(4.0)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    add_button("Add your first rule", true)
                        .id("overview-add-rule")
                        .debug_selector(|| "overview-add-rule".into())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.navigate(Screen::Rules, window, cx);
                            this.add_handler(window, cx);
                        })),
                )
                .when_some(empty.import.clone(), |actions, label| {
                    actions.child(
                        button(label, ButtonKind::Default, false)
                            .id("overview-import")
                            .debug_selector(|| "overview-import".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(found) = &this.found_config {
                                    crate::importer_view::open(found.file.clone(), false, cx);
                                }
                            })),
                    )
                }),
        )
}

#[allow(clippy::too_many_arguments, reason = "one card's parts, drawn once")]
fn destination_card(
    index: usize,
    card: &Card,
    light: Light,
    pill_light: &dyn Fn(usize) -> Light,
    generation: usize,
    wide: bool,
    ticked: bool,
    cx: &mut Context<'_, PwrRoute>,
) -> AnyElement {
    let lit = light.lit();
    let special = matches!(card.kind, CardKind::Function | CardKind::NotOpened)
        || (card.kind == CardKind::Ghost && card.pills.iter().all(|p| p.profile.is_none()));
    let ring = card.ribbon.is_some();
    let ghost = card.kind == CardKind::Ghost;
    let big = if special { 32.0 } else { 40.0 };
    let badge_box = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(big))
        .rounded(px(if special { 8.0 } else { 10.0 }))
        .border_1()
        .map(|b| match card.kind {
            CardKind::Function => b
                .bg(theme::accent_soft())
                .border_color(theme::transparent())
                .child(text("\u{0192}", 15.0, 700.0, theme::accent_bright())),
            CardKind::Ghost => b
                .border_dashed()
                .border_color(theme::border_strong())
                .child(mono(card.badge.clone(), 11.0, 700.0, theme::text_subtle())),
            _ if lit => b
                .border_color(theme::accent_border())
                .bg(theme::bg_hover_strong())
                .child(mono(
                    card.badge.clone(),
                    13.0,
                    700.0,
                    theme::accent_bright(),
                )),
            _ => b
                .border_color(theme::border_strong())
                .bg(theme::bg_app())
                .child(mono(
                    card.badge.clone(),
                    if special { 11.0 } else { 13.0 },
                    700.0,
                    theme::text_secondary(),
                )),
        });

    let name = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(text(
            card.name.clone(),
            13.5,
            600.0,
            if ghost {
                theme::text_muted()
            } else {
                theme::text_primary()
            },
        ))
        .when(ghost, |name| {
            name.child(mono("not installed", 10.0, 500.0, theme::warn_text()))
        });
    let pills = (!special && !card.pills.is_empty()).then(|| {
        div()
            .flex()
            .gap(px(6.0))
            .overflow_hidden()
            .children(card.pills.iter().enumerate().map(|(p, pill)| {
                pill_chip(index, p, pill, pill_light(p).lit(), cx).into_any_element()
            }))
            .children((card.more_pills > 0).then(|| {
                pill_frame(false)
                    .child(mono(
                        format!("+{}", card.more_pills),
                        10.5,
                        500.0,
                        theme::text_subtle(),
                    ))
                    .into_any_element()
            }))
    });
    let meta = |line: String| {
        one_line(
            line,
            theme::mono_family(500.0),
            500.0,
            10.5,
            12.0,
            theme::text_subtle(),
        )
    };
    let words = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(if special { 6.0 } else { 8.0 }))
        .child(name)
        .children(pills)
        .map(|words| match &card.alone {
            Some((line, today)) => words
                .child(
                    paragraph(line.clone(), 10.5, 14.0, theme::text_subtle())
                        .font_family(theme::mono_family(500.0)),
                )
                .children(today.clone().map(meta)),
            // A special card's meta explains it, so it wraps rather than
            // cut off at a narrow window.
            None if special => words.child(
                paragraph(card.meta.clone(), 10.5, 14.0, theme::text_subtle())
                    .font_family(theme::mono_family(500.0)),
            ),
            None => words.child(meta(card.meta.clone())),
        });
    let share = card.share.clone().filter(|_| wide).map(|share| {
        let fill = if ring || lit {
            theme::accent()
        } else {
            theme::text_muted()
        };
        let fraction = share.fraction.max(0.01);
        let bar = div().h_full().rounded(px(2.0)).bg(fill);
        let bar: AnyElement = if ticked {
            bar.w(gpui_kit::relative(fraction)).into_any_element()
        } else {
            bar.with_animation(
                ("overview-share", index),
                Animation::new(TICK).with_easing(ease_out_quint()),
                move |bar, t| bar.w(gpui_kit::relative(fraction * t)),
            )
            .into_any_element()
        };
        div()
            .w(px(112.0))
            .flex_none()
            .flex()
            .flex_col()
            .items_end()
            .justify_center()
            .gap(px(7.0))
            .child(text(share.label, 18.0, 700.0, theme::text_primary()))
            .child(
                div()
                    .w_full()
                    .h(px(4.0))
                    .rounded(px(2.0))
                    .bg(theme::border_subtle())
                    .overflow_hidden()
                    .child(bar),
            )
            .child(text("of today's links", 9.5, 500.0, theme::text_subtle()))
    });

    let focus = Focus::Card(index);
    let element = div()
        .id(("overview-card", index))
        .debug_selector(move || format!("overview-card-{index}"))
        .relative()
        .flex()
        .flex_none()
        .gap(px(14.0))
        .px(px(16.0))
        .py(px(14.0))
        .map(|frame| match (&card.alone, special) {
            (Some(_), _) => frame.min_h(px(96.0)),
            (None, true) => frame.min_h(px(60.0)).items_center(),
            (None, false) => frame.h(px(96.0)),
        })
        .rounded(px(12.0))
        .border_1()
        .cursor_pointer()
        .map(|frame| {
            if lit {
                frame
                    .border_color(theme::accent())
                    .bg(linear_gradient(
                        90.0,
                        linear_color_stop(theme::bg_row_active(), 0.0),
                        linear_color_stop(theme::bg_elevated(), 0.75),
                    ))
                    .shadow(vec![
                        shadow(alpha(theme::accent(), 0.14), 0.0, 0.0, 3.0),
                        shadow(alpha(theme::accent(), 0.12), 10.0, 30.0, 0.0),
                    ])
            } else if ghost {
                frame
                    .border_dashed()
                    .border_color(theme::border_default())
                    .bg(theme::transparent())
            } else if ring {
                frame
                    .border_color(theme::accent_border())
                    .bg(theme::bg_elevated())
                    .shadow(vec![shadow(alpha(theme::accent(), 0.10), 0.0, 0.0, 3.0)])
            } else {
                frame
                    .border_color(theme::border_subtle())
                    .bg(theme::bg_elevated())
            }
        })
        .on_hover(cx.listener(move |this, over: &bool, _, cx| {
            let changed = if *over {
                // Entering a pill's card from the pill keeps the pill.
                match this.overview.hovered {
                    Some(Focus::Pill(card, _)) if card == index => false,
                    _ => this.overview.hover(Some(focus)),
                }
            } else {
                match this.overview.hovered {
                    Some(Focus::Card(card) | Focus::Pill(card, _)) if card == index => {
                        this.overview.hover(None)
                    }
                    _ => false,
                }
            };
            if changed {
                cx.notify();
            }
        }))
        .on_click(cx.listener(move |this, _, _, cx| {
            let pin = (this.overview.pinned != Some(focus)).then_some(focus);
            if this.overview.pin(pin) {
                cx.notify();
            }
        }))
        .child(badge_box)
        .child(words)
        .children(share);
    // The port and the ribbon sit over the card's border: siblings,
    // painted after it.
    let element = div()
        .relative()
        .flex_none()
        .child(element)
        .when_some(card.ribbon.clone(), |frame, ribbon| {
            frame.child(
                div()
                    .absolute()
                    .top(px(-9.0))
                    .right(px(14.0))
                    .h(px(18.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .rounded_full()
                    .bg(theme::accent())
                    .child(text(ribbon, 10.0, 650.0, theme::accent_on())),
            )
        })
        .child(port(false, lit));
    let (from, to) = light.opacities();
    faded(
        element,
        ("overview-card-dim", index * 1_000_000 + generation).into(),
        from,
        to,
    )
}

fn pill_frame(lit: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.0))
        .h(px(22.0))
        .px(px(9.0))
        .rounded_full()
        .border_1()
        .map(|pill| {
            if lit {
                pill.border_color(theme::accent_border())
                    .bg(theme::accent_soft())
            } else {
                pill.border_color(theme::border_subtle())
                    .bg(theme::bg_panel())
            }
        })
}

fn pill_chip(
    card: usize,
    index: usize,
    pill: &overview_model::Pill,
    lit: bool,
    cx: &mut Context<'_, PwrRoute>,
) -> Stateful<Div> {
    let color = if lit {
        theme::accent_bright()
    } else if pill.profile.is_none() {
        theme::text_subtle()
    } else {
        theme::text_secondary()
    };
    let focus = Focus::Pill(card, index);
    pill_frame(lit)
        .id(("overview-pill", card * 100 + index))
        .debug_selector(move || format!("overview-pill-{card}-{index}"))
        .child(mono(pill.label().to_owned(), 10.5, 500.0, color))
        .when(pill.everything_else, |chip| {
            chip.child(text("\u{221e}", 11.0, 500.0, theme::accent_bright()))
                .tooltip(tip("Everything else goes here"))
        })
        .on_hover(cx.listener(move |this, over: &bool, _, cx| {
            let changed = if *over {
                this.overview.hover(Some(focus))
            } else {
                this.overview.unhover(focus, Some(Focus::Card(card)))
            };
            if changed {
                cx.notify();
            }
        }))
}

// --- the callout -----------------------------------------------------------

fn callout_card(app: &PwrRoute, callout: &Callout, cx: &mut Context<'_, PwrRoute>) -> Div {
    let refused = match &app.overview.refused {
        Some(Refused::Callout(reason)) => Some(reason.clone()),
        _ => None,
    };
    let (plain, icon_label, title, body, warnings) = match callout {
        Callout::Offer { body, warnings } => (
            false,
            "\u{221e}",
            Some(OFFER_TITLE),
            body.clone(),
            warnings.clone(),
        ),
        Callout::Done { body } => (
            false,
            "\u{2713}",
            Some(DONE_TITLE),
            body.clone(),
            Vec::new(),
        ),
        Callout::Settled { body } => (true, "\u{221e}", None, body.clone(), Vec::new()),
    };
    let tile = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(28.0))
        .rounded(px(8.0))
        .map(|tile| {
            if plain {
                tile.bg(theme::bg_elevated()).child(text(
                    icon_label,
                    16.0,
                    600.0,
                    theme::text_muted(),
                ))
            } else {
                tile.bg(theme::accent_soft()).child(text(
                    icon_label,
                    16.0,
                    600.0,
                    theme::accent_bright(),
                ))
            }
        });
    let words = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .when_some(title, |words, title| {
            words.child(text(title, 12.5, 600.0, theme::text_primary()))
        })
        .child(paragraph(body, 11.0, 16.5, theme::text_muted()))
        .children(warnings.into_iter().map(|warning| {
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .mt(px(2.0))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(px(14.0))
                        .rounded_full()
                        .bg(theme::warn_soft())
                        .child(text("!", 9.0, 700.0, theme::warn_text())),
                )
                .child(paragraph(warning, 10.5, 15.75, theme::warn_text()))
        }))
        .when_some(refused, |words, reason| {
            words.child(refused_line(CALLOUT_REFUSED_ID, &reason).mt(px(4.0)))
        });
    let rules = || {
        button(CHANGE_IN_RULES, ButtonKind::Quiet, true)
            .id("overview-callout-rules")
            .debug_selector(|| "overview-callout-rules".into())
            .on_click(cx.listener(|this, _, window, cx| {
                this.navigate(Screen::Rules, window, cx);
            }))
    };
    let buttons: Vec<AnyElement> = match callout {
        Callout::Offer { .. } => vec![
            button(OFFER_BUTTON, ButtonKind::Default, true)
                .id(CALLOUT_BUTTON_ID)
                .debug_selector(|| CALLOUT_BUTTON_ID.into())
                .on_click(cx.listener(|this, _, _, cx| this.set_override(true, cx)))
                .into_any_element(),
        ],
        Callout::Done { .. } => vec![
            button("Undo", ButtonKind::Default, true)
                .id(CALLOUT_UNDO_ID)
                .debug_selector(|| CALLOUT_UNDO_ID.into())
                .on_click(cx.listener(|this, _, _, cx| this.set_override(false, cx)))
                .into_any_element(),
            rules().into_any_element(),
        ],
        Callout::Settled { .. } => vec![rules().into_any_element()],
    };
    div()
        .debug_selector(|| "overview-callout".into())
        .flex()
        .flex_none()
        .items_center()
        .gap(px(14.0))
        .px(px(14.0))
        .py(px(12.0))
        .rounded(px(10.0))
        .border_1()
        .map(|card| {
            if plain {
                card.border_color(theme::border_subtle())
                    .bg(theme::bg_panel())
            } else {
                card.border_color(theme::accent_border())
                    .bg(theme::bg_row_active())
            }
        })
        .child(tile)
        .child(words)
        .children(buttons)
}
