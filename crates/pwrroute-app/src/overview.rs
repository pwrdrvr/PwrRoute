//! Overview, the landing screen (design: PwrRoute Overview and Setup,
//! 1a-3f): the hero, the setup card, today's tiles and the route map, drawn
//! from [`overview_model::build`].
//!
//! The map is a graph: the rule column and the destination column (one
//! card per browser, one node per profile) over one `canvas` that paints
//! the dot grid and every route, a bezier from a row's port to its node's
//! (`PathBuilder::stroke`; dashed while a route is not running). The rows
//! and each card's nodes record their bounds in prepaint
//! (`on_children_prepainted`) into [`MapBounds`], which the canvas reads
//! when it paints in the same frame, so a hover builds paths from those
//! bounds and lays nothing out again. What is traced comes from
//! [`OverviewState`]: the pin, else the keyboard, else the pointer.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, Animation, AnimationExt as _, AnyElement, AnyView, App, AppContext as _, Bounds,
    BoxShadow, ClickEvent, Context, Div, ElementId, Font, HighlightStyle, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, PathBuilder, Pixels, Point, Render,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, StyledText, TextRun,
    Window, WindowTextSystem, anchored, canvas, deferred, div, ease_out_quint, font,
    linear_color_stop, linear_gradient, point, px,
};

use crate::app::{PwrRoute, Screen, Selection};
use crate::overview_model::{
    self, Action, CHANGE_IN_RULES, Callout, Caption, Card, CardKind, ConfigInputs, ConfigTrouble,
    DONE_TITLE, End, EverythingElse, Focus, FocusKey, GUTTER_MIN, Inputs, Installed, Item, Lit,
    Map, MapWidths, Node, NodeRef, OFFER_BUTTON, OFFER_TITLE, Opener, Overview, Pattern, Pause,
    Private, Route, Row, RowMatch, RowSource, Setup, SetupStep, Share, State, Status, Target, Tone,
    fit_tags,
};
use crate::rules_view::pane_frame;
use crate::theme::{self, icon, mono, text, tracked};
use crate::widgets::{
    ButtonKind, add_button, badge, button, caret, field_label, input_box, key_cap, menu_card,
    menu_row, one_line, paragraph, profiles_hint, tag_frame,
};

/// The share shows on cards at least this wide.
const SHARE_MIN: f32 = 300.0;
/// The rail, and the pane's and the map's padding and borders, for the
/// map's widths at a window width.
const RAIL: f32 = 197.0;
const PANE_X: f32 = 48.0;
const MAP_X: f32 = 34.0;

/// A route at rest, and while something else is traced: 1px accent.
const ROUTE_REST: f32 = 0.30;
const ROUTE_DIM: f32 = 0.08;
/// A route that is not running, at rest: dashed `--text-primary`.
const STOPPED_REST: f32 = 0.22;

/// Hover fades and dims over this.
const FADE: Duration = Duration::from_millis(120);
/// Today's numbers count up over this the first time they show.
const TICK: Duration = Duration::from_millis(400);
/// The override's hand-off route.
const HANDOFF: Duration = Duration::from_millis(900);
/// What is not lit, while something is.
const DIM: f32 = 0.45;

/// The element ids tests reach.
pub const PANE_ID: &str = "overview-pane";
pub const HERO_ACTION_ID: &str = "overview-hero-action";
pub const CALLOUT_BUTTON_ID: &str = "overview-callout-button";
pub const CALLOUT_UNDO_ID: &str = "overview-callout-undo";
/// The legend's Clear, while something is pinned.
pub const PIN_CLEAR_ID: &str = "overview-pin-clear";

/// One route as the canvas last painted it.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(test), allow(dead_code, reason = "read by the window's tests"))]
pub struct Painted {
    /// The row's port, and the node's (where in its fan).
    pub from: Point<Pixels>,
    pub to: Point<Pixels>,
    /// Drawn highlighted.
    pub lit: bool,
    /// Drawn dashed: not running.
    pub dashed: bool,
}

/// Where the rows and nodes were painted this frame, for the routes.
#[derive(Debug, Default)]
pub struct MapBounds {
    pub rows: HashMap<End, Bounds<Pixels>>,
    /// Each card's nodes that show (a special card is its one node).
    pub nodes: HashMap<NodeRef, Bounds<Pixels>>,
    /// Every route the canvas painted last.
    pub painted: Vec<Painted>,
}

/// The screen's own state, on the window.
#[derive(Debug, Default)]
pub struct OverviewState {
    /// Under the pointer. Each is kept by what it is ([`FocusKey`]), not
    /// where it is drawn: the cards re-sort as links come in.
    pub hovered: Option<FocusKey>,
    /// Clicked: traced until it is clicked again, Esc, Clear, or a click on
    /// the map's empty ground. It wins over the pointer and the keyboard.
    pub pinned: Option<FocusKey>,
    /// Where the arrow keys (and Tab) put the keyboard on the map.
    pub keyed: Option<FocusKey>,
    /// The node with no rules yet under the pointer: it offers "+ Add a
    /// rule…".
    pub idle_hover: Option<FocusKey>,
    /// Cards whose "+N more profiles" is open, by key, for the session.
    pub expanded: HashSet<String>,
    /// The card that just opened: its new nodes fade in.
    pub expanding: Option<usize>,
    /// Bumps whenever what is traced changes: the routes ease from how
    /// they were drawn, keyed by it.
    generation: usize,
    /// What was traced before the last change, for the fade.
    previous: Option<FocusKey>,
    /// The callout's switch was just used: Undo stays until the reader
    /// leaves Overview.
    pub just_overridden: bool,
    /// The override's hand-off route while it plays, by its number: every
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
    /// What is traced: the pin, else the keyboard, else the pointer.
    pub fn focus(&self) -> Option<FocusKey> {
        self.pinned
            .clone()
            .or_else(|| self.keyed.clone())
            .or_else(|| self.hovered.clone())
    }

    /// What is traced on `map`: the first of the pin, the keyboard and
    /// the pointer that is still on it.
    pub fn focus_on(&self, map: &Map) -> Option<Focus> {
        [&self.pinned, &self.keyed, &self.hovered]
            .into_iter()
            .find_map(|key| key.as_ref().and_then(|key| map.resolve(key)))
    }

    /// Changes what is traced, keeping the previous for the fade.
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

    /// The pointer moved onto `focus` (or off everything): the keyboard
    /// lets go.
    pub fn hover(&mut self, focus: Option<FocusKey>) -> bool {
        let moved = self.hovered != focus;
        self.lit_change(|this| {
            if focus.is_some() {
                this.keyed = None;
            }
            this.hovered = focus;
        }) || moved
    }

    /// Leaves `focus` if it is what is hovered.
    fn unhover(&mut self, focus: &FocusKey) -> bool {
        if self.hovered.as_ref() == Some(focus) {
            self.hover(None)
        } else {
            false
        }
    }

    pub fn pin(&mut self, focus: Option<FocusKey>) -> bool {
        self.lit_change(|this| this.pinned = focus)
    }

    /// Puts the keyboard on `focus`.
    pub fn key(&mut self, focus: Option<FocusKey>) -> bool {
        let moved = self.keyed != focus;
        self.lit_change(|this| this.keyed = focus) || moved
    }

    /// The pointer is no longer where it was (a scroll, or the rows changed
    /// under it): nothing stays hovered until it moves again.
    pub fn forget_pointer(&mut self) -> bool {
        let idle = self.idle_hover.take().is_some();
        self.hover(None) || idle
    }

    /// Opens or closes a card's "+N more profiles".
    pub fn toggle_expanded(&mut self, key: &str, card: usize) {
        if self.expanded.remove(key) {
            self.expanding = None;
        } else {
            self.expanded.insert(key.to_owned());
            self.expanding = Some(card);
        }
    }

    /// The hand-off playing now, by its number.
    #[cfg(test)]
    pub fn handing_off(&self) -> Option<usize> {
        self.handoff
    }

    /// What is fading out.
    #[cfg(test)]
    pub fn fading(&self) -> Option<FocusKey> {
        self.previous.clone()
    }

    /// The reader leaves Overview: its numbers have counted up, the
    /// callout's Undo is over, nothing stays traced, and nothing fades or
    /// hands off when they come back. Open cards stay open.
    pub fn leave(&mut self) {
        self.ticked = true;
        self.just_overridden = false;
        self.refused = None;
        self.picking = false;
        self.choosing = None;
        self.hover(None);
        self.pin(None);
        self.key(None);
        self.idle_hover = None;
        self.expanding = None;
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
/// has loaded and every link goes to the fallback browser.
/// `config_not_loaded`: the config never loaded and `rules.json` routes
/// without it. `error` alone: a reload failed and the last load still
/// routes. Neither, while this window could not read the file (`problem`):
/// the router's copy routes, so it is the stale case, not the broken one.
pub fn config_trouble(
    source: crate::router::RoutingSource,
    problem: Option<&str>,
) -> Option<ConfigTrouble> {
    if let Some(fallback) = source.fallback {
        let why = source.error.or_else(|| problem.map(str::to_owned));
        Some(ConfigTrouble::NothingLoaded(first_line(
            why.as_deref().unwrap_or(&fallback),
        )))
    } else if let Some(error) = source.error.as_deref().filter(|_| source.config_not_loaded) {
        Some(ConfigTrouble::NotLoaded(first_line(error)))
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

    /// A card's "+ Rule" (no profile: the browser's own default) or a node
    /// with no rules yet ("+ Rule for Travel"): opens Rules' new-rule
    /// editor with that browser and profile chosen, and a private window
    /// on for a private-window node.
    pub fn add_rule_for(
        &mut self,
        browser: &str,
        profile: &str,
        private: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.navigate(Screen::Rules, window, cx);
        if self.config.is_some() {
            self.visual_rules_expanded = true;
        }
        let index = self.store.add_handler();
        self.store.set_browser(index, browser);
        self.store.set_profile(index, profile);
        if private {
            self.store.set_private(index, true);
        }
        self.selection = Some(Selection::Handler(index));
        self.refresh(cx);
        self.rebuild_rail(cx);
        self.add_pattern(window, cx);
        self.draft_from = Some((index, crate::editor::DraftFrom::Overview));
        // Start from offers the links but fills nothing in: with the
        // browser set, a pattern would make the rule whole, and it would
        // save before the person chose it.
        self.draft_link = Some((index, None));
    }

    /// A key on Overview while the window has the keyboard. Up, Down and
    /// Tab (Shift-Tab back) step through the rows, Everything else and the
    /// nodes, which trace as the pointer does; Return opens the row the
    /// keyboard is on, or pins the node; Esc lets go of the pin and the
    /// keyboard.
    pub fn overview_key(
        &mut self,
        key: &str,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match key {
            "escape" => {
                let unpinned = self.overview.pin(None);
                let unkeyed = self.overview.key(None);
                if unpinned || unkeyed {
                    cx.notify();
                }
            }
            "up" | "down" | "tab" => {
                let status = status(self, cx);
                let map = build(self, &status, cx).map;
                let expanded = &self.overview.expanded;
                let order = map.keyboard_order(|card| expanded.contains(card.key()));
                if order.is_empty() {
                    return;
                }
                let back = key == "up" || (key == "tab" && shift);
                let at = self
                    .overview
                    .keyed
                    .as_ref()
                    .and_then(|keyed| map.resolve(keyed))
                    .and_then(|keyed| order.iter().position(|focus| *focus == keyed));
                let next = match (at, back) {
                    (None, false) => 0,
                    (None, true) => order.len() - 1,
                    (Some(i), false) => (i + 1).min(order.len() - 1),
                    (Some(i), true) => i.saturating_sub(1),
                };
                if self.overview.key(map.key_of(order[next])) {
                    cx.notify();
                }
            }
            // Return opens the row the keyboard is on, adds a rule for a
            // node with no rules yet (as its click does), or pins.
            "enter" => {
                let Some(keyed) = self.overview.keyed.clone() else {
                    return;
                };
                let status = status(self, cx);
                let map = build(self, &status, cx).map;
                match map.resolve(&keyed) {
                    Some(Focus::Row(row)) => {
                        let source = map.rows[row].source;
                        self.open_rule(source, window, cx);
                    }
                    Some(Focus::Node(card, node)) if map.cards[card].nodes[node].addable => {
                        let browser = map.cards[card].name.clone();
                        let node = &map.cards[card].nodes[node];
                        let profile = node.profile.clone().unwrap_or_default();
                        let private = node.private.is_some();
                        self.add_rule_for(&browser, &profile, private, window, cx);
                    }
                    Some(_) => toggle_pin(self, keyed, cx),
                    None => {}
                }
            }
            _ => {}
        }
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

/// The most lines of a ƒ row's source its tooltip shows.
const TIP_LINES: usize = 12;

/// A tooltip's view: one line on the popover card, and a ƒ row's source
/// under it in mono (12 lines at most, 480 wide).
struct Tip {
    label: SharedString,
    source: Option<SharedString>,
}

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        let lines: Vec<String> = self
            .source
            .as_ref()
            .map(|source| {
                let all: Vec<&str> = source.lines().collect();
                let mut lines: Vec<String> = all
                    .iter()
                    .take(TIP_LINES)
                    .map(|line| line.trim_end().to_owned())
                    .collect();
                if all.len() > TIP_LINES {
                    lines.push("\u{2026}".to_owned());
                }
                lines
            })
            .unwrap_or_default();
        div()
            .px(px(8.0))
            .py(px(5.0))
            .max_w(px(480.0))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .bg(theme::bg_popover())
            .border_1()
            .border_color(theme::border_default())
            .rounded(px(6.0))
            .child(text(
                self.label.clone(),
                11.0,
                500.0,
                theme::text_secondary(),
            ))
            .when(!lines.is_empty(), |tip| {
                tip.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .children(lines.into_iter().map(|line| {
                            one_line(
                                if line.is_empty() {
                                    " ".to_owned()
                                } else {
                                    line
                                },
                                theme::mono_family(500.0),
                                500.0,
                                10.5,
                                14.0,
                                theme::text_muted(),
                            )
                            .whitespace_nowrap()
                        })),
                )
            })
    }
}

pub(crate) fn tip(
    label: impl Into<SharedString>,
    source: Option<String>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let label = label.into();
    let source = source.map(SharedString::from);
    move |_, cx| {
        cx.new(|_| Tip {
            label: label.clone(),
            source: source.clone(),
        })
        .into()
    }
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
    let widths = MapWidths::of_content(width - RAIL - PANE_X - MAP_X);
    pane_frame(PANE_ID)
        .debug_selector(|| PANE_ID.into())
        // A scroll moves the map under a pointer that did not move: GPUI
        // reports hover only on a move, so nothing stays traced.
        .on_scroll_wheel(cx.listener(|this, _, _, cx| {
            if this.overview.forget_pointer() {
                cx.notify();
            }
        }))
        .child(hero(&overview, cx))
        .children(
            overview
                .setup
                .as_ref()
                .map(|setup| setup_card(app, setup, cx)),
        )
        .child(today(app, &overview, cx))
        .child(map(app, &overview.map, widths, window, cx))
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

/// The lit state now and before the last change, for one element: `None`
/// while nothing is traced.
#[derive(Clone, Copy)]
struct Light {
    now: Option<bool>,
    before: Option<bool>,
}

impl Light {
    fn lit(self) -> bool {
        self.now == Some(true)
    }

    /// Its opacity before and after: dimmed while something else is traced.
    fn opacities(self) -> (f32, f32) {
        let of = |state: Option<bool>| match state {
            Some(false) => DIM,
            _ => 1.0,
        };
        (of(self.before), of(self.now))
    }
}

/// What the map's parts read while they draw.
struct Look<'a> {
    lit: &'a Lit,
    before: &'a Lit,
    generation: usize,
    hovered: Option<Focus>,
    pinned: Option<Focus>,
    keyed: Option<Focus>,
    idle_hover: Option<NodeRef>,
}

impl Look<'_> {
    fn light(&self, is: impl Fn(&Lit) -> bool) -> Light {
        Light {
            now: self.lit.traced.then(|| is(self.lit)),
            before: self.before.traced.then(|| is(self.before)),
        }
    }

    /// The pointer or the keyboard is on it, or it is pinned.
    fn touched(&self, focus: Focus) -> bool {
        [self.hovered, self.pinned, self.keyed].contains(&Some(focus))
    }
}

/// Text widths for the rows' layout, from the window's text system: a row
/// shows as many whole pattern tags as fit.
struct Measure(Arc<WindowTextSystem>);

impl Measure {
    fn width(&self, text: &str, family: &'static str, css_weight: f32, size: f32) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let run = TextRun {
            len: text.len(),
            font: Font {
                weight: theme::weight(css_weight),
                ..font(family)
            },
            color: theme::text_primary(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        // GPUI rounds a text box up to a whole pixel.
        f32::from(self.0.layout_line(text, px(size), &[run], None).width).ceil()
    }

    fn mono(&self, text: &str, size: f32) -> f32 {
        self.width(text, theme::mono_family(500.0), 500.0, size)
    }

    fn sans(&self, text: &str, css_weight: f32, size: f32) -> f32 {
        self.width(text, theme::SANS, css_weight, size)
    }
}

/// Clears the pin, from the legend's Clear or the map's empty ground.
fn clear_pin(this: &mut PwrRoute, cx: &mut Context<'_, PwrRoute>) {
    if this.overview.pin(None) {
        cx.notify();
    }
}

/// Pins `focus`, or lets go of it when it is the pin.
fn toggle_pin(this: &mut PwrRoute, focus: FocusKey, cx: &mut Context<'_, PwrRoute>) {
    let pin = (this.overview.pinned.as_ref() != Some(&focus)).then_some(focus);
    if this.overview.pin(pin) {
        cx.notify();
    }
}

/// A hover handler for a traced element.
fn hover_listener(
    focus: FocusKey,
    cx: &mut Context<'_, PwrRoute>,
) -> impl Fn(&bool, &mut Window, &mut App) + 'static {
    cx.listener(move |this, over: &bool, _, cx| {
        let changed = if *over {
            this.overview.hover(Some(focus.clone()))
        } else {
            this.overview.unhover(&focus)
        };
        if changed {
            cx.notify();
        }
    })
}

fn map(
    app: &PwrRoute,
    map: &Map,
    widths: MapWidths,
    window: &mut Window,
    cx: &mut Context<'_, PwrRoute>,
) -> Stateful<Div> {
    let state = &app.overview;
    let on_map = |key: &Option<FocusKey>| key.as_ref().and_then(|key| map.resolve(key));
    let focus = state.focus_on(map);
    let lit = map.lit(focus);
    let before = map.lit(on_map(&state.previous));
    let pinned = on_map(&state.pinned);
    let keyed = on_map(&state.keyed);
    // The keyboard on a node with no rules yet offers "+ Rule for …" as
    // the pointer does.
    let idle_hover = [on_map(&state.idle_hover), keyed]
        .into_iter()
        .flatten()
        .find_map(|focus| match focus {
            Focus::Node(card, node)
                if map
                    .cards
                    .get(card)
                    .and_then(|c| c.nodes.get(node))
                    .is_some_and(|n| n.addable) =>
            {
                Some((card, node))
            }
            _ => None,
        });
    let trace = Look {
        lit: &lit,
        before: &before,
        generation: state.generation,
        hovered: on_map(&state.hovered),
        pinned,
        keyed,
        idle_hover,
    };
    let measure = Measure(window.text_system().clone());
    let wide = widths.dest >= SHARE_MIN;

    // The header: the two captions over their columns.
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
        .justify_between()
        .gap(px(GUTTER_MIN))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .max_w(px(widths.rule))
                .flex()
                .items_center()
                .justify_between()
                .pr(px(12.0))
                .child(cap(left_cap, left_small, left_color))
                .when(map.counts, |c1| c1.child(note("today"))),
        )
        .child(
            div()
                .w(px(widths.dest))
                .flex_none()
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
                .child(
                    div()
                        .debug_selector(|| "overview-in-use".into())
                        .min_w_0()
                        .overflow_hidden()
                        .child(one_line(
                            map.in_use.clone(),
                            theme::SANS,
                            500.0,
                            10.5,
                            10.5,
                            theme::text_subtle(),
                        )),
                ),
        );
    let legend = legend(map, &lit, pinned, cx);

    // The rule column, and which child is which row for the routes.
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
                ends.push(Some(End::Row(*index)));
                let drawn = rule_row(*index, row, &trace, widths, &measure, cx);
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
        &trace,
        !map.items.is_empty() || map.empty.is_some(),
        cx,
    ));

    let bounds = state.bounds.clone();
    let rows_bounds = bounds.clone();
    let rule_column = div()
        .on_children_prepainted(move |children, _, _| {
            let mut bounds = rows_bounds.borrow_mut();
            bounds.rows.clear();
            // The cards prepaint after this column: their nodes record
            // themselves anew.
            bounds.nodes.clear();
            for (child, end) in children.iter().zip(&ends) {
                if let Some(end) = end {
                    bounds.rows.insert(*end, *child);
                }
            }
        })
        .relative()
        .flex_1()
        .min_w_0()
        .max_w(px(widths.rule))
        .flex()
        .flex_col()
        .gap(px(4.0))
        .children(left);

    // The destinations: routed browsers, the special cards, then "Also
    // installed" and the browsers with no rules yet.
    let mut right: Vec<AnyElement> = Vec::new();
    let mut also_shown = false;
    for (index, card) in map.cards.iter().enumerate() {
        if card.unused && !also_shown {
            also_shown = true;
            if let Some(also) = &map.also {
                right.push(also_header(also.clone()).into_any_element());
            }
        }
        let expanded = state.expanded.contains(card.key());
        let drawn = if card.single() {
            single_card(index, card, &trace, &bounds, cx)
        } else {
            browser_card(index, card, &trace, expanded, wide, app, &bounds, cx)
        };
        // Fed only by rows nothing runs.
        right.push(if card.idle {
            div().opacity(DIM).child(drawn).into_any_element()
        } else {
            drawn
        });
    }
    let card_column = div()
        .relative()
        .w(px(widths.dest))
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(10.0))
        .children(right);

    // Every route, at rest or dimmed, then the traced ones over them; they
    // ease to their new state when the trace changes.
    let routes = Rc::new(lit.routes.clone());
    let routes_before = Rc::new(if state.generation == 0 {
        lit.routes.clone()
    } else {
        before.routes.clone()
    });
    let layer = {
        let bounds = bounds.clone();
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .with_animation(
                ("overview-routes", state.generation),
                Animation::new(FADE).with_easing(ease_out_quint()),
                move |layer, t| {
                    let strokes = strokes(&routes, &routes_before, t);
                    let bounds = bounds.clone();
                    layer.child(
                        canvas(
                            |_, _, _| {},
                            move |area, _, window, _| {
                                paint_dots(area, window);
                                paint_routes(&strokes, &bounds, true, window);
                            },
                        )
                        .size_full(),
                    )
                },
            )
    };
    let handoff = app.overview.handoff.and_then(|handoff| {
        let route = lit
            .routes
            .iter()
            .find(|route| route.from == End::EverythingElse)
            .copied()?;
        let bounds = bounds.clone();
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .child(
                    canvas(
                        |_, _, _| {},
                        move |_, _, window, _| {
                            let stroke = Stroke {
                                base: 0.0,
                                glow: 1.0,
                                ..Stroke::of(&route)
                            };
                            paint_routes(&[stroke], &bounds, false, window);
                        },
                    )
                    .size_full(),
                )
                .with_animation(
                    ("overview-handoff", handoff),
                    Animation::new(HANDOFF),
                    // Up quickly, held, then gone.
                    |layer, t| layer.opacity(((1.0 - t) * 3.0).min(1.0) * (t * 6.0).min(1.0)),
                ),
        )
    });

    let body = div()
        .id("overview-map-body")
        .debug_selector(|| "overview-map-body".into())
        .relative()
        .flex()
        .items_start()
        // The columns stop at their caps; every pixel past them is the
        // routes' gutter, never under its 96.
        .justify_between()
        .gap(px(GUTTER_MIN))
        .p(px(16.0))
        .when(map.dimmed, |body| body.opacity(0.4))
        // A click on the empty ground lets go of the pin.
        .on_click(cx.listener(|this, _, _, cx| clear_pin(this, cx)))
        .child(layer)
        .children(handoff)
        .child(rule_column)
        .child(card_column);

    div()
        .id("overview-map")
        .debug_selector(|| "overview-map".into())
        .flex_none()
        .rounded(px(14.0))
        .border_1()
        .border_color(theme::border_subtle())
        .bg(theme::bg_panel())
        .child(header)
        .child(legend)
        .child(body)
}

/// The legend's line: a 22px swatch and its word.
fn swatch(label: &'static str, style: Swatch) -> Div {
    let line = match style {
        Swatch::Route => div().w(px(22.0)).h(px(1.0)).bg(alpha(theme::accent(), 0.3)),
        Swatch::Highlighted => div().w(px(22.0)).h(px(2.0)).bg(theme::accent()),
        Swatch::NotRunning => div()
            .w(px(22.0))
            .flex()
            .gap(px(4.0))
            .children((0..3).map(|_| {
                div()
                    .w(px(4.0))
                    .h(px(1.0))
                    .bg(alpha(theme::text_primary(), 0.45))
            })),
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .child(line)
        .child(text(label, 10.5, 500.0, theme::text_muted()))
}

#[derive(Clone, Copy)]
enum Swatch {
    Route,
    Highlighted,
    NotRunning,
}

/// The line under the header: what the strokes mean, and how to trace, or
/// what is pinned.
fn legend(map: &Map, lit: &Lit, pinned: Option<Focus>, cx: &mut Context<'_, PwrRoute>) -> Div {
    let hint: AnyElement = match pinned.and_then(|focus| map.focus_label(focus)) {
        Some(label) => div()
            .debug_selector(|| "overview-pinned".into())
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(text("Pinned:", 10.5, 400.0, theme::text_subtle()))
                    .child(text(label, 10.5, 600.0, theme::accent_bright())),
            )
            .child(key_cap("esc"))
            .child(
                button("Clear", ButtonKind::Quiet, true)
                    .id(PIN_CLEAR_ID)
                    .debug_selector(|| PIN_CLEAR_ID.into())
                    .py(px(4.0))
                    .px(px(8.0))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        clear_pin(this, cx);
                    })),
            )
            .into_any_element(),
        None => text(
            "Hover a rule or profile to trace its routes \u{00b7} click to pin",
            10.5,
            400.0,
            theme::text_subtle(),
        )
        .into_any_element(),
    };
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .h(px(30.0))
        .px(px(16.0))
        .border_b_1()
        .border_color(theme::border_faint())
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(14.0))
                .child(swatch("route", Swatch::Route))
                .child(swatch("highlighted", Swatch::Highlighted))
                .when(lit.any_not_running(), |legend| {
                    legend.child(swatch("not running", Swatch::NotRunning))
                }),
        )
        .child(hint)
}

/// One route as the canvas strokes it: the 1px resting line's alpha, and
/// how far it is highlighted (0 to 1).
#[derive(Clone, Copy, Debug)]
struct Stroke {
    from: End,
    to: NodeRef,
    slot: usize,
    fan: usize,
    running: bool,
    base: f32,
    glow: f32,
}

impl Stroke {
    fn of(route: &Route) -> Self {
        let (base, glow) = match (route.trace, route.running) {
            (overview_model::Trace::Lit, _) => (0.0, 1.0),
            (overview_model::Trace::Rest, true) => (ROUTE_REST, 0.0),
            (overview_model::Trace::Rest, false) => (STOPPED_REST, 0.0),
            (overview_model::Trace::Dim, _) => (ROUTE_DIM, 0.0),
        };
        Self {
            from: route.from,
            to: route.to,
            slot: route.slot,
            fan: route.fan,
            running: route.running,
            base,
            glow,
        }
    }
}

/// Each route `t` of the way from how it was drawn to how it is: a route
/// that was not there starts unseen, and one that is gone fades out.
fn strokes(now: &[Route], before: &[Route], t: f32) -> Vec<Stroke> {
    let mix = |a: f32, b: f32| a + (b - a) * t;
    let was: HashMap<(End, NodeRef), &Route> = before
        .iter()
        .map(|route| ((route.from, route.to), route))
        .collect();
    let is: HashSet<(End, NodeRef)> = now.iter().map(|route| (route.from, route.to)).collect();
    let mut out: Vec<Stroke> = now
        .iter()
        .map(|route| {
            let to = Stroke::of(route);
            let from = was.get(&(route.from, route.to)).copied().map_or(
                Stroke {
                    base: 0.0,
                    glow: 0.0,
                    ..to
                },
                Stroke::of,
            );
            Stroke {
                base: mix(from.base, to.base),
                glow: mix(from.glow, to.glow),
                ..to
            }
        })
        .collect();
    out.extend(
        before
            .iter()
            .filter(|old| !is.contains(&(old.from, old.to)))
            .map(|old| {
                let from = Stroke::of(old);
                Stroke {
                    base: mix(from.base, 0.0),
                    glow: mix(from.glow, 0.0),
                    ..from
                }
            }),
    );
    out
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

/// Every route from its row's port to its node's, as
/// `M x1 y1 C x1+dx y1, x2-dx y2, x2 y2` with dx half the gutter as drawn
/// (from the port to the node), so a wide window's gutter gives long,
/// gentle curves: the resting and dimmed lines first, then the highlighted
/// ones over them. Routes into one node fan into its port in their rows'
/// order.
fn paint_routes(
    strokes: &[Stroke],
    bounds: &Rc<RefCell<MapBounds>>,
    record: bool,
    window: &mut Window,
) {
    let curve = |from: Point<Pixels>, to: Point<Pixels>, width: f32, dashed: bool| {
        let dx = ((to.x - from.x) / 2.0).max(px(0.0));
        let mut path = PathBuilder::stroke(px(width));
        if dashed {
            path = path.dash_array(&[px(4.0), px(4.0)]);
        }
        path.move_to(from);
        path.cubic_bezier_to(to, point(from.x + dx, from.y), point(to.x - dx, to.y));
        path.build().ok()
    };
    let mut painted = Vec::new();
    {
        let bounds = bounds.borrow();
        let ends: Vec<Option<(Point<Pixels>, Point<Pixels>)>> = strokes
            .iter()
            .map(|stroke| {
                let row = bounds.rows.get(&stroke.from)?;
                let node = bounds.nodes.get(&stroke.to)?;
                let (step, spread) = overview_model::fan_spread(stroke.fan);
                let y = node.center().y - px(spread / 2.0) + px(step * stroke.slot as f32);
                Some((point(row.right(), row.center().y), point(node.left(), y)))
            })
            .collect();
        for (stroke, ends) in strokes.iter().zip(&ends) {
            let Some((from, to)) = *ends else { continue };
            if stroke.base > 0.003 {
                let color = if stroke.running {
                    alpha(theme::accent(), stroke.base)
                } else {
                    alpha(theme::text_primary(), stroke.base)
                };
                if let Some(path) = curve(from, to, 1.0, !stroke.running) {
                    window.paint_path(path, color);
                }
            }
        }
        // Many lit routes into one node: their halos stack into a blob by
        // the port, so each is fainter.
        let mut lit_into: HashMap<NodeRef, usize> = HashMap::new();
        for stroke in strokes.iter().filter(|stroke| stroke.glow > 0.003) {
            *lit_into.entry(stroke.to).or_insert(0) += 1;
        }
        for (stroke, ends) in strokes.iter().zip(&ends) {
            let Some((from, to)) = *ends else { continue };
            if stroke.glow > 0.003 {
                if stroke.running {
                    let stacked = lit_into.get(&stroke.to).copied().unwrap_or(1).max(1);
                    let halo = 0.16 / (stacked as f32).sqrt();
                    if let Some(path) = curve(from, to, 6.0, false) {
                        window.paint_path(path, alpha(theme::accent(), halo * stroke.glow));
                    }
                    if let Some(path) = curve(from, to, 2.0, false) {
                        window.paint_path(path, alpha(theme::accent(), stroke.glow));
                    }
                } else if let Some(path) = curve(from, to, 1.5, true) {
                    window.paint_path(path, alpha(theme::text_secondary(), stroke.glow));
                }
            }
            painted.push(Painted {
                from,
                to,
                lit: stroke.glow >= 0.5,
                dashed: !stroke.running,
            });
        }
    }
    if record {
        bounds.borrow_mut().painted = painted;
    }
}

/// `.port`: the 9px dot on a row's right edge or a node's left, a capsule
/// when `fan` routes come in.
fn port(right: bool, fan: usize, style: PortStyle) -> Div {
    let (_, spread) = overview_model::fan_spread(fan);
    let height = 9.0 + spread;
    let dot = div()
        .absolute()
        .top(px(-height / 2.0))
        .left_0()
        .w(px(9.0))
        .h(px(height))
        .rounded_full()
        .map(|dot| match style {
            PortStyle::Lit => dot
                .border(px(1.5))
                .bg(theme::accent())
                .border_color(theme::accent()),
            PortStyle::Route => dot
                .border(px(1.5))
                .bg(theme::bg_panel())
                .border_color(alpha(theme::accent(), 0.45)),
            PortStyle::Stopped => dot
                .border(px(1.5))
                .bg(theme::bg_panel())
                .border_color(theme::border_strong()),
            PortStyle::StoppedLit => dot
                .border(px(1.5))
                .bg(theme::bg_panel())
                .border_color(theme::text_secondary()),
            PortStyle::Empty => dot
                .border_1()
                .bg(theme::bg_panel())
                .border_color(theme::border_default()),
        });
    // Centred on the edge: a zero-height anchor at the middle.
    div()
        .absolute()
        .top(gpui_kit::relative(0.5))
        .map(|anchor| {
            if right {
                anchor.right(px(-5.0))
            } else {
                anchor.left(px(-6.0))
            }
        })
        .w(px(9.0))
        .h(px(0.0))
        .child(dot)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PortStyle {
    Lit,
    Route,
    /// A browser that is not installed.
    Stopped,
    /// Traced, but its routes are not running (paused, the config never
    /// loaded, not installed, a rule that is off): ringed, not filled.
    StoppedLit,
    /// A node nothing routes to yet.
    Empty,
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
            cx.stop_propagation();
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

/// The row's frame: 40 high (44 for Everything else), lit, pinned or not.
fn row_frame(id: ElementId, lit: bool, pinned: bool, ghost: bool) -> Stateful<Div> {
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
            if pinned {
                row.border_color(theme::accent_border())
                    .bg(theme::bg_hover_strong())
                    .shadow(vec![inset(theme::accent())])
            } else if lit {
                row.border_color(theme::accent_border())
                    .bg(theme::bg_row_active())
                    .shadow(vec![shadow(alpha(theme::accent(), 0.08), 0.0, 0.0, 3.0)])
            } else if ghost {
                row.border_dashed()
                    .border_color(theme::border_default())
                    .bg(theme::transparent())
                    .hover(|row| row.bg(theme::bg_hover()))
            } else {
                row.border_color(theme::border_faint())
                    .bg(theme::bg_elevated())
                    .hover(|row| row.bg(theme::bg_hover()))
            }
        })
}

/// The pin's outline: an inset 1px ring.
fn inset(color: Hsla) -> BoxShadow {
    BoxShadow {
        color,
        offset: point(px(0.0), px(0.0)),
        blur_radius: px(0.0),
        spread_radius: px(1.0),
        inset: true,
    }
}

fn rank(label: String, lit: bool, size: f32, color: Hsla) -> Div {
    div().w(px(18.0)).flex_none().child(mono(
        label,
        size,
        600.0,
        if lit { theme::accent_bright() } else { color },
    ))
}

/// A pattern tag: whole, or cut with an ellipsis (never narrower than
/// 24px) when it is the one tag shown.
fn pattern_tag(pattern: &Pattern, cut: bool) -> Div {
    let tag = tag_frame();
    let tag = if cut {
        tag.min_w(px(24.0)).flex_shrink(1.0).overflow_hidden()
    } else {
        tag
    };
    let tag = if pattern.regex {
        tag.bg(theme::info_soft())
            .border_color(theme::transparent())
    } else {
        tag
    };
    tag.child(one_line(
        pattern.text.clone(),
        theme::mono_family(500.0),
        500.0,
        10.5,
        10.5,
        if pattern.regex {
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

/// "from [Sl Slack]".
fn opener_part(opener: &Opener) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.0))
        .child(text("from", 11.0, 500.0, theme::text_muted()))
        .child(tag_frame().child(badge(opener.badge.clone())).child(mono(
            opener.name.clone(),
            10.5,
            500.0,
            theme::text_primary(),
        )))
        .children(more(opener.more))
}

fn opener_width(opener: &Opener, measure: &Measure) -> f32 {
    let more = if opener.more > 0 {
        5.0 + measure.mono(&format!("+{}", opener.more), 10.5)
    } else {
        0.0
    };
    measure.sans("from", 500.0, 11.0)
        + 5.0
        + 16.0
        + 12.0
        + 5.0
        + measure.mono(&opener.name, 10.5)
        + more
}

/// The match column, laid out for `room` pixels: as many whole pattern
/// tags as fit, then "+N"; a ƒ row's name and what it tests.
fn match_column(row: &Row, room: f32, measure: &Measure) -> Div {
    let column = div()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(5.0))
        .overflow_hidden();
    let off = !row.enabled;
    let off_width = if off { 5.0 + 34.0 } else { 0.0 };
    let column = match &row.matcher {
        RowMatch::Patterns {
            patterns,
            more: extra,
            opener,
        } => {
            let opener_room = opener
                .as_ref()
                .map_or(0.0, |opener| 5.0 + opener_width(opener, measure));
            let widths: Vec<f32> = patterns
                .iter()
                .map(|pattern| measure.mono(&pattern.text, 10.5) + 16.0)
                .collect();
            let shown = fit_tags(&widths, *extra, room - opener_room - off_width, |n| {
                measure.mono(&format!("+{n}"), 10.5)
            });
            let rest = patterns.len() - shown + extra;
            column
                .children(
                    patterns
                        .iter()
                        .take(shown)
                        .map(|pattern| pattern_tag(pattern, shown == 1)),
                )
                .children(more(rest))
                .children(opener.as_ref().map(opener_part))
        }
        RowMatch::Opener(opener) => column.child(opener_part(opener)),
        RowMatch::Function {
            name,
            excerpt,
            more: n,
            ..
        } => {
            let code = |label: String, color: Hsla| {
                one_line(label, theme::mono_family(500.0), 500.0, 10.5, 13.0, color)
            };
            column
                .child(fn_mark())
                .map(|column| match (name, excerpt) {
                    (Some(name), Some(excerpt)) => column
                        .child(code(name.clone(), theme::text_secondary()).flex_none())
                        .child(
                            code(excerpt.clone(), theme::text_subtle())
                                .flex_shrink(1.0)
                                .ml(px(-1.0)),
                        ),
                    (Some(name), None) => column.child(code(name.clone(), theme::text_secondary())),
                    (None, Some(excerpt)) => {
                        column.child(code(excerpt.clone(), theme::text_secondary()))
                    }
                    (None, None) => {
                        column.child(text("unnamed function", 11.0, 500.0, theme::text_muted()))
                    }
                })
                .children(more(*n))
        }
        RowMatch::Other { text: t, more: n } => column
            .child(text(t.clone(), 11.0, 500.0, theme::text_muted()))
            .children(more(*n)),
        RowMatch::Any => column.child(text("every link", 11.0, 500.0, theme::text_muted())),
    };
    column.when(off, |column| {
        column.child(crate::widgets::chip("off").py(px(3.0)).px(px(7.0)))
    })
}

/// The longest a target's label gets before it is cut.
const TARGET_LABEL: f32 = 200.0;
/// The private-window glyph: a mask.
const MASK: &str = "pwrroute/mask.svg";
/// A private rule's tag when its browser cannot open a private window.
const NOT_PRIVATE: &str = "not private";

fn target_column(target: &Target) -> Div {
    let to = div()
        .flex()
        .flex_none()
        .min_w_0()
        .items_center()
        .gap(px(6.0))
        .child(text("\u{2192}", 11.5, 500.0, theme::text_subtle()));
    let label = |label: String, color: Hsla| {
        div().max_w(px(TARGET_LABEL)).min_w_0().child(one_line(
            label,
            theme::SANS,
            500.0,
            11.5,
            11.5,
            color,
        ))
    };
    match target {
        Target::Browser {
            badge: b, label: l, ..
        } => to
            .child(badge(b.clone()))
            .when(target.private_window(), |to| {
                to.child(
                    div()
                        .mr(px(-2.0))
                        .child(icon(MASK, 11.0, theme::text_subtle())),
                )
            })
            .child(label(l.clone(), theme::text_secondary()))
            .when(target.not_private(), |to| {
                to.child(mono(NOT_PRIVATE, 10.0, 500.0, theme::warn_text()))
            }),
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
            .child(badge(b.without_icon()).border_dashed())
            .child(label(name.clone(), theme::text_subtle()))
            .child(mono("not installed", 10.0, 500.0, theme::warn_text())),
        Target::Unset => to.child(text("Choose a browser", 11.5, 500.0, theme::text_subtle())),
    }
}

/// The target column's width, for the room the patterns get.
fn target_width(target: &Target, measure: &Measure) -> f32 {
    let arrow = measure.sans("\u{2192}", 500.0, 11.5) + 6.0;
    let label = |label: &str| measure.sans(label, 500.0, 11.5).min(TARGET_LABEL);
    arrow
        + match target {
            Target::Browser {
                label: l, private, ..
            } => {
                12.0 + 6.0
                    + label(l)
                    + match private {
                        Private::No => 0.0,
                        Private::Window(_) => 11.0 - 2.0 + 6.0,
                        Private::Unsupported => 6.0 + measure.mono(NOT_PRIVATE, 10.0),
                    }
            }
            Target::Function { .. } => 26.0 + 6.0 + label("JavaScript"),
            Target::NotOpened { .. } => label(crate::widgets::NOT_OPENED),
            Target::Ghost { name, .. } => {
                12.0 + 6.0 + label(name) + 6.0 + measure.mono("not installed", 10.0)
            }
            Target::Unset => label("Choose a browser"),
        }
}

fn count_column(count: Option<u64>, lit: bool) -> Div {
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
                if lit {
                    theme::accent_bright()
                } else {
                    theme::text_subtle()
                },
            )
        }))
}

/// "Edit ›" ("View ›" for a config handler): opens the rule in Rules.
fn edit_label(source: RowSource) -> &'static str {
    match source {
        RowSource::Rule(_) => "Edit \u{203a}",
        RowSource::Handler(_) => "View \u{203a}",
    }
}

fn rule_row(
    index: usize,
    row: &Row,
    trace: &Look<'_>,
    widths: MapWidths,
    measure: &Measure,
    cx: &mut Context<'_, PwrRoute>,
) -> AnyElement {
    let focus = Focus::Row(index);
    let key = FocusKey::Row(row.source);
    let light = trace.light(|lit| lit.row(index));
    let lit = light.lit();
    let pinned = trace.pinned == Some(focus);
    let edit = trace.touched(focus);
    let ghost = row.target.ghost();
    let source = row.source;
    let edit_width = if edit {
        8.0 + measure.sans(edit_label(source), 550.0, 10.5) + 16.0
    } else {
        0.0
    };
    // The match column gets what the rank, the target, the count and the
    // gaps leave.
    let room = widths.rule
        - 2.0
        - 22.0
        - 18.0
        - 8.0
        - 8.0
        - target_width(&row.target, measure)
        - 8.0
        - 36.0
        - edit_width
        - 4.0;
    let tip_source = match &row.matcher {
        RowMatch::Function { source, .. } => source.clone(),
        _ => None,
    };
    let element = row_frame(("overview-row", index).into(), lit, pinned, ghost)
        .debug_selector(move || format!("overview-row-{index}"))
        .tooltip(tip(row.tooltip.clone(), tip_source))
        .on_hover(hover_listener(key.clone(), cx))
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            cx.stop_propagation();
            // A double-click opens the rule; a click pins it.
            if event.click_count() >= 2 {
                this.open_rule(source, window, cx);
            } else {
                toggle_pin(this, key.clone(), cx);
            }
        }))
        .child(rank(
            format!("{:02}", row.rank),
            lit,
            10.5,
            theme::text_subtle(),
        ))
        .child(match_column(row, room, measure))
        .when(edit, |element| {
            element.child(
                div()
                    .id(("overview-row-edit", index))
                    .debug_selector(move || format!("overview-row-edit-{index}"))
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(20.0))
                    .px(px(7.0))
                    .rounded(px(5.0))
                    .border_1()
                    .border_color(theme::border_default())
                    .bg(theme::bg_panel())
                    .cursor_pointer()
                    .hover(|button| button.bg(theme::bg_hover()))
                    .child(text(
                        edit_label(source),
                        10.5,
                        550.0,
                        theme::text_secondary(),
                    ))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.open_rule(source, window, cx);
                    })),
            )
        })
        .child(target_column(&row.target))
        .child(count_column(row.count, lit));
    // The port sits over the row's border: a sibling, painted after it. A
    // rule that is off has none.
    let running = trace
        .lit
        .route(End::Row(index))
        .is_some_and(|route| route.running);
    let port_style = if lit && running {
        PortStyle::Lit
    } else if lit {
        PortStyle::StoppedLit
    } else if ghost {
        PortStyle::Stopped
    } else {
        PortStyle::Route
    };
    let element = div()
        .relative()
        .flex_none()
        .child(element)
        .when(row.node.is_some() && row.enabled, |row| {
            row.child(port(true, 1, port_style))
        });
    // A switched-off rule stays at 40%, and dims with the rest.
    let (from, to) = light.opacities();
    let scale = if row.enabled { 1.0 } else { 0.4 };
    faded(
        element,
        ("overview-row-dim", index * 1_000_000 + trace.generation).into(),
        from * scale,
        to * scale,
    )
}

fn everything_else_row(
    row: &EverythingElse,
    trace: &Look<'_>,
    after: bool,
    cx: &mut Context<'_, PwrRoute>,
) -> AnyElement {
    let focus = Focus::EverythingElse;
    let light = trace.light(|lit| lit.everything_else);
    let lit = light.lit();
    let pinned = trace.pinned == Some(focus);
    let element = row_frame("overview-everything-else".into(), lit, pinned, false)
        .debug_selector(|| "overview-everything-else".into())
        .tooltip(tip("Everything else goes here", None))
        .h(px(44.0))
        .when(!lit && !pinned, |row| {
            row.border_color(theme::accent_border()).bg(linear_gradient(
                90.0,
                linear_color_stop(over(theme::bg_elevated(), theme::accent(), 0.10), 0.0),
                linear_color_stop(theme::bg_elevated(), 0.7),
            ))
        })
        .on_hover(hover_listener(FocusKey::EverythingElse, cx))
        .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            toggle_pin(this, FocusKey::EverythingElse, cx);
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
        .child(count_column(row.count, lit));
    let running = trace
        .lit
        .route(End::EverythingElse)
        .is_some_and(|route| route.running);
    let port_style = if lit && running {
        PortStyle::Lit
    } else if lit {
        PortStyle::StoppedLit
    } else if row.target.ghost() {
        PortStyle::Stopped
    } else {
        PortStyle::Route
    };
    let element = div()
        .relative()
        .flex_none()
        .when(after, |row| row.mt(px(8.0)))
        .child(element)
        .when(row.node.is_some(), |row| {
            row.child(port(true, 1, port_style))
        });
    let (from, to) = light.opacities();
    faded(
        element,
        ("overview-ee-dim", trace.generation).into(),
        from,
        to,
    )
}

fn empty_prompt(empty: &overview_model::Empty, cx: &mut Context<'_, PwrRoute>) -> Div {
    let last = crate::start_from::last_link(cx);
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
        .when_some(last.clone(), |prompt, last| prompt.child(last_link(&last)))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .map(|actions| match last {
                    // The prompt names the last link and offers a rule for
                    // it (design: Rule From a Link 2a).
                    Some(last) => actions
                        .child(
                            add_button("Add a rule for this link", true)
                                .id("overview-add-rule")
                                .debug_selector(|| "overview-add-rule".into())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.navigate(Screen::Rules, window, cx);
                                    this.add_handler_from(last.start(), window, cx);
                                })),
                        )
                        .child(
                            button("Blank rule", ButtonKind::Quiet, false)
                                .id("overview-blank-rule")
                                .debug_selector(|| "overview-blank-rule".into())
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.navigate(Screen::Rules, window, cx);
                                    this.add_blank_handler(window, cx);
                                })),
                        ),
                    None => actions.child(
                        add_button("Add your first rule", true)
                            .id("overview-add-rule")
                            .debug_selector(|| "overview-add-rule".into())
                            .on_click(cx.listener(|this, _, window, cx| {
                                cx.stop_propagation();
                                this.navigate(Screen::Rules, window, cx);
                                this.add_handler(window, cx);
                            })),
                    ),
                })
                .when_some(empty.import.clone(), |actions, label| {
                    actions.child(
                        button(label, ButtonKind::Default, false)
                            .id("overview-import")
                            .debug_selector(|| "overview-import".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                if let Some(found) = &this.found_config {
                                    crate::importer_view::open(found.file.clone(), false, cx);
                                }
                            })),
                    )
                }),
        )
}

/// The first-rule prompt's last link: when, the URL on one line, and
/// where it went.
fn last_link(last: &crate::start_from::RecentLink) -> Div {
    div()
        .debug_selector(|| "overview-last-link".into())
        .flex()
        .flex_col()
        .self_stretch()
        .gap(px(5.0))
        .mb(px(4.0))
        .py(px(9.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(theme::border_subtle())
        .bg(theme::bg_app())
        .child(theme::tracked(
            &format!("Last link \u{00b7} {}", last.time),
            9.5,
            600.0,
            0.12,
            theme::text_subtle(),
        ))
        .child(crate::widgets::one_line(
            last.url.clone(),
            theme::mono_family(500.0),
            500.0,
            11.5,
            15.0,
            theme::text_primary(),
        ))
        .child(theme::text(
            format!("Went to {}", last.opened),
            10.5,
            500.0,
            theme::text_muted(),
        ))
}

/// "Also installed", over the browsers with no rules yet.
fn also_header(note_text: String) -> Div {
    div()
        .debug_selector(|| "overview-also".into())
        .h(px(24.0))
        .mt(px(6.0))
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .px(px(2.0))
        .child(text("Also installed", 11.0, 600.0, theme::text_secondary()))
        .child(note(note_text))
}

/// A card's 28px badge: the browser's icon, else its letters in the box.
fn card_badge(card: &Card, lit: bool) -> Div {
    let frame = || {
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(28.0))
            .rounded(px(7.0))
            .border_1()
    };
    match card.kind {
        CardKind::Function => frame()
            .bg(theme::accent_soft())
            .border_color(theme::transparent())
            .child(text("\u{0192}", 14.0, 700.0, theme::accent_bright())),
        CardKind::Ghost => frame()
            .border_dashed()
            .border_color(theme::border_strong())
            .child(mono(
                card.badge.letters().clone(),
                11.0,
                700.0,
                theme::text_subtle(),
            )),
        _ => {
            let (border, background, color) = if lit {
                (
                    theme::accent_border(),
                    theme::bg_hover_strong(),
                    theme::accent_bright(),
                )
            } else if card.unused {
                (
                    theme::border_default(),
                    theme::transparent(),
                    theme::text_subtle(),
                )
            } else {
                (
                    theme::border_strong(),
                    theme::bg_app(),
                    theme::text_secondary(),
                )
            };
            crate::widgets::icon_or(&card.badge, 28.0, move |letters| {
                frame()
                    .border_color(border)
                    .bg(background)
                    .child(mono(letters, 11.0, 700.0, color))
            })
        }
    }
}

/// A card's frame for its state: traced by its header (the full lit
/// style), holding a lit node, taking Everything else, idle, a ghost.
fn card_frame<E: gpui_kit::Styled + IntoElement>(
    frame: E,
    card: &Card,
    lit: bool,
    warm: bool,
) -> E {
    let ring = card.ribbon.is_some();
    frame.rounded(px(12.0)).border_1().map(|frame| {
        if lit {
            frame
                .border_color(theme::accent())
                .bg(theme::bg_elevated())
                .shadow(vec![
                    shadow(alpha(theme::accent(), 0.14), 0.0, 0.0, 3.0),
                    shadow(alpha(theme::accent(), 0.12), 10.0, 30.0, 0.0),
                ])
        } else if card.kind == CardKind::Ghost {
            frame
                .border_dashed()
                .border_color(theme::border_default())
                .bg(theme::transparent())
        } else if card.unused {
            frame
                .border_color(theme::border_faint())
                .bg(theme::bg_panel())
        } else if ring {
            frame
                .border_color(theme::accent_border())
                .bg(theme::bg_elevated())
                .shadow(vec![shadow(alpha(theme::accent(), 0.10), 0.0, 0.0, 3.0)])
        } else if warm {
            frame
                .border_color(theme::accent_border())
                .bg(theme::bg_elevated())
        } else {
            frame
                .border_color(theme::border_subtle())
                .bg(theme::bg_elevated())
        }
    })
}

/// "Everything else · PwrRoute" over the card's top edge.
fn ribbon(label: String) -> Div {
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
        .child(text(label, 10.0, 650.0, theme::accent_on()))
}

/// The card's share of today's links: 13/700 over a 3px bar.
fn share_block(index: usize, share: &Share, accent: bool, ticked: bool) -> Div {
    let fill = if accent {
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
        .w(px(64.0))
        .flex_none()
        .flex()
        .flex_col()
        .items_end()
        .gap(px(6.0))
        .child(text(
            share.label.clone(),
            13.0,
            700.0,
            theme::text_primary(),
        ))
        .child(
            div()
                .w_full()
                .h(px(3.0))
                .rounded(px(2.0))
                .bg(theme::border_subtle())
                .overflow_hidden()
                .child(bar),
        )
}

/// The card's "+ Rule": a new rule for the browser, in whichever profile
/// the browser opens (design: Rule Editor Pickers 2a, 2b). A profile's own
/// "+ Rule for …" is on its node.
fn card_add(index: usize, name: &str, cx: &mut Context<'_, PwrRoute>) -> impl IntoElement + use<> {
    let browser = name.to_owned();
    div()
        .id(("overview-card-add", index))
        .debug_selector(move || format!("overview-card-add-{index}"))
        .flex_none()
        .px(px(8.0))
        .py(px(3.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(theme::border_subtle())
        .cursor_pointer()
        .hover(|button| {
            button
                .bg(theme::bg_hover())
                .border_color(theme::border_default())
        })
        .tooltip(tip(
            format!("New rule for {name}, in the profile {name} chooses"),
            None,
        ))
        .child(text("+ Rule", 10.5, 550.0, theme::text_muted()))
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.add_rule_for(&browser, "", false, window, cx);
        }))
}

#[allow(clippy::too_many_arguments, reason = "one card's parts, drawn once")]
fn browser_card(
    index: usize,
    card: &Card,
    trace: &Look<'_>,
    expanded: bool,
    wide: bool,
    app: &PwrRoute,
    bounds: &Rc<RefCell<MapBounds>>,
    cx: &mut Context<'_, PwrRoute>,
) -> AnyElement {
    let lit = trace.lit.card(index);
    let warm = trace.lit.warm(index);
    let ring = card.ribbon.is_some();
    // While something else is traced, the node with no rules yet under the
    // pointer stays at full strength with its "+ Rule for …"; the rest of
    // its card dims around it.
    let spot = trace.lit.traced
        && !warm
        && trace
            .idle_hover
            .is_some_and(|(card_index, _)| card_index == index);
    let key = FocusKey::Card(card.key().to_owned());
    let header = div()
        .id(("overview-card", index))
        .debug_selector(move || format!("overview-card-{index}"))
        .h(px(48.0))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(10.0))
        .pl(px(12.0))
        .pr(px(14.0))
        .rounded_t(px(12.0))
        .child(card_badge(card, lit))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(5.0))
                .child(one_line(
                    card.name.clone(),
                    theme::SANS,
                    600.0,
                    13.0,
                    16.0,
                    if card.unused {
                        theme::text_secondary()
                    } else {
                        theme::text_primary()
                    },
                ))
                .child(one_line(
                    card.meta.clone(),
                    theme::mono_family(500.0),
                    500.0,
                    10.0,
                    12.0,
                    theme::text_subtle(),
                )),
        )
        .children(
            card.share
                .as_ref()
                .filter(|_| wide)
                .map(|share| share_block(index, share, ring || lit, app.overview.ticked)),
        )
        // At rest only where nothing routes yet; on a busy card while it is
        // under the pointer or pinned, so its counts keep their room.
        .children(
            (card.kind == CardKind::Browser
                && (card.unused
                    || app.overview.hovered.as_ref() == Some(&key)
                    || app.overview.pinned.as_ref() == Some(&key)))
            .then(|| card_add(index, &card.name, cx)),
        )
        .when(!card.unused, |header| {
            header
                .cursor_pointer()
                .on_hover(hover_listener(key.clone(), cx))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    toggle_pin(this, key.clone(), cx);
                }))
        })
        .when(spot, |header| header.opacity(DIM));

    // The profiles that show, "+N more profiles" when some fold, then the
    // private-window node, which never folds: which child is which node,
    // for the routes.
    let visible = card.visible(expanded);
    let private = card.private_node();
    let mut children: Vec<Option<usize>> = visible
        .iter()
        .copied()
        .filter(|node| Some(*node) != private)
        .map(Some)
        .collect();
    if card.folded() > 0 {
        children.push(None);
    }
    children.extend(private.map(Some));
    let nodes_bounds = bounds.clone();
    let drawn: Vec<AnyElement> = children
        .iter()
        .map(|child| match child {
            Some(n) => {
                let drawn = node_row(index, *n, card, &card.nodes[*n], trace, spot, cx);
                if Some(*n) != private && *n >= card.shown && app.overview.expanding == Some(index)
                {
                    // "+N more profiles" just opened: the new nodes fade in.
                    div()
                        .child(drawn)
                        .with_animation(
                            ("overview-node-in", index * 1_000 + n),
                            Animation::new(FADE).with_easing(ease_out_quint()),
                            |node, t| node.opacity(t),
                        )
                        .into_any_element()
                } else {
                    drawn
                }
            }
            None => {
                let key = card.key().to_owned();
                let label = if expanded {
                    "Show fewer \u{25b4}".to_owned()
                } else {
                    format!("+{} more profiles \u{25be}", card.folded())
                };
                div()
                    .id(("overview-more", index))
                    .debug_selector(move || format!("overview-more-{index}"))
                    .h(px(26.0))
                    .px(px(16.0))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .hover(|more| more.bg(theme::bg_hover()))
                    .when(spot, |more| more.opacity(DIM))
                    .child(text(label, 10.5, 500.0, theme::text_subtle()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.overview.toggle_expanded(&key, index);
                        cx.notify();
                    }))
                    .into_any_element()
            }
        })
        .collect();
    let nodes = div()
        .on_children_prepainted(move |bounds, _, _| {
            let mut recorded = nodes_bounds.borrow_mut();
            for (child, node) in bounds.iter().zip(&children) {
                if let Some(node) = node {
                    recorded.nodes.insert((index, *node), *child);
                }
            }
        })
        .flex()
        .flex_col()
        .py(px(4.0))
        .border_t_1()
        .border_color(theme::border_faint())
        .children(drawn);

    let frame = card_frame(
        div()
            .debug_selector(move || format!("overview-card-frame-{index}"))
            .flex()
            .flex_col(),
        card,
        lit,
        warm,
    )
    .child(header)
    .child(nodes);
    let element = div()
        .relative()
        .flex_none()
        .child(frame)
        .when_some(card.ribbon.clone(), |element, label| {
            element.child(ribbon(label))
        });
    let (from, to) = trace.light(|lit| lit.warm(index)).opacities();
    let (from, to) = if spot { (1.0, 1.0) } else { (from, to) };
    faded(
        element,
        ("overview-card-dim", index * 1_000_000 + trace.generation).into(),
        from,
        to,
    )
}

/// One profile, or the card's private-window node: its port, its name (and
/// ∞), its rules and today's links. The private-window node sits apart
/// under a dashed hairline, with the mask before its name. `spot`: its
/// card holds the node with no rules yet under the pointer while something
/// else is traced, and every other node dims.
#[allow(clippy::too_many_arguments, reason = "one node's parts, drawn once")]
fn node_row(
    card_index: usize,
    index: usize,
    card: &Card,
    node: &Node,
    trace: &Look<'_>,
    spot: bool,
    cx: &mut Context<'_, PwrRoute>,
) -> AnyElement {
    let focus = Focus::Node(card_index, index);
    let private = node.private.is_some();
    let key = if private {
        FocusKey::Private(card.key().to_owned())
    } else {
        FocusKey::Node(card.key().to_owned(), node.profile.clone())
    };
    let lit = trace.lit.node(card_index, index);
    let pinned = trace.pinned == Some(focus);
    let adding = node.addable && trace.idle_hover == Some((card_index, index));
    let fan = trace.lit.fan(card_index, index);
    let port_style = if node.unused {
        PortStyle::Empty
    } else if lit && trace.lit.node_runs(card_index, index) {
        PortStyle::Lit
    } else if lit {
        PortStyle::StoppedLit
    } else {
        PortStyle::Route
    };
    let name_color = if lit {
        theme::accent_bright()
    } else if node.unused {
        theme::text_muted()
    } else {
        theme::text_secondary()
    };
    let element =
        div()
            .id(("overview-node", card_index * 1_000 + index))
            .debug_selector(move || format!("overview-node-{card_index}-{index}"))
            .relative()
            .h(px(30.0))
            .when(private, |row| {
                row.mt(px(4.0)).child(
                    div()
                        .absolute()
                        .top(px(-3.0))
                        .left(px(16.0))
                        .right(px(14.0))
                        .h(px(0.0))
                        .border_t_1()
                        .border_dashed()
                        .border_color(theme::border_subtle()),
                )
            })
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .pl(px(16.0))
            .pr(px(14.0))
            .cursor_pointer()
            .map(|row| {
                if pinned {
                    row.bg(theme::bg_hover_strong())
                        .shadow(vec![inset(theme::accent())])
                } else if lit {
                    row.bg(theme::bg_row_active())
                } else if adding {
                    // Its own hover fill, whatever is pinned.
                    row.bg(theme::bg_hover())
                } else {
                    row.hover(|row| row.bg(theme::bg_hover()))
                }
            })
            .tooltip(tip(node.tooltip.clone(), None))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .when(private, |name| {
                        name.child(icon(
                            MASK,
                            13.0,
                            if lit || pinned {
                                theme::accent_bright()
                            } else {
                                theme::text_subtle()
                            },
                        ))
                    })
                    .child(one_line(
                        node.label().to_owned(),
                        theme::SANS,
                        500.0,
                        12.0,
                        12.0,
                        name_color,
                    ))
                    .when(node.everything_else, |name| {
                        name.child(
                            div()
                                .id(("overview-node-ee", card_index * 1_000 + index))
                                .tooltip(tip("Everything else goes here", None))
                                .child(text("\u{221e}", 12.0, 600.0, theme::accent_bright())),
                        )
                    }),
            )
            .map(|row| {
                if adding {
                    // Names the profile it adds for, so it never reads as
                    // the card's own button (design: Rule Editor Pickers 2c).
                    row.child(
                        one_line(
                            format!("+ Rule for {}", node.label()),
                            theme::SANS,
                            550.0,
                            10.5,
                            12.0,
                            theme::accent_bright(),
                        )
                        .min_w_0(),
                    )
                } else {
                    row.children(
                        node.meta
                            .clone()
                            .map(|meta| mono(meta, 10.0, 500.0, theme::text_subtle())),
                    )
                }
            })
            .when(!node.unused, |row| {
                row.child(div().w(px(40.0)).flex_none().flex().justify_end().children(
                    node.links.map(|n| {
                        mono(
                            crate::activity::thousands(n),
                            10.5,
                            500.0,
                            if lit {
                                theme::accent_bright()
                            } else {
                                theme::text_secondary()
                            },
                        )
                    }),
                ))
            })
            .child(port(false, fan, port_style));
    let element = if node.addable {
        let browser = card.name.clone();
        let profile = node.profile.clone().unwrap_or_default();
        element
            .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                let next = if *over {
                    Some(key.clone())
                } else if this.overview.idle_hover.as_ref() == Some(&key) {
                    None
                } else {
                    return;
                };
                if this.overview.idle_hover != next {
                    this.overview.idle_hover = next;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.add_rule_for(&browser, &profile, private, window, cx);
            }))
    } else {
        element
            .on_hover(hover_listener(key.clone(), cx))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                toggle_pin(this, key.clone(), cx);
            }))
    };
    // In a card that holds a lit node, only its other nodes dim; a card
    // that holds none dims whole.
    let (from, to) = trace
        .light(|lit| lit.node(card_index, index) || !lit.warm(card_index))
        .opacities();
    let (from, to) = if spot {
        let at = if adding { 1.0 } else { DIM };
        (at, at)
    } else {
        (from, to)
    };
    faded(
        element,
        (
            "overview-node-dim",
            (card_index * 1_000 + index) * 1_000_000 + trace.generation,
        )
            .into(),
        from,
        to,
    )
}

/// Decided by JavaScript, Not opened, not installed: one node, the whole
/// card, 44 high with its port at the middle.
fn single_card(
    index: usize,
    card: &Card,
    trace: &Look<'_>,
    bounds: &Rc<RefCell<MapBounds>>,
    cx: &mut Context<'_, PwrRoute>,
) -> AnyElement {
    let focus = Focus::Node(index, 0);
    let key = FocusKey::Node(card.key().to_owned(), None);
    let lit = trace.lit.node(index, 0);
    let pinned = trace.pinned == Some(focus);
    let ghost = card.kind == CardKind::Ghost;
    let node = card.nodes.first();
    let fan = trace.lit.fan(index, 0);
    let port_style = if lit && trace.lit.node_runs(index, 0) {
        PortStyle::Lit
    } else if lit {
        PortStyle::StoppedLit
    } else if ghost {
        PortStyle::Stopped
    } else {
        PortStyle::Route
    };
    let frame = card_frame(
        div()
            .id(("overview-card", index))
            .debug_selector(move || format!("overview-card-{index}"))
            .h(px(44.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .pl(px(12.0))
            .pr(px(14.0))
            .cursor_pointer(),
        card,
        false,
        lit,
    )
    .when(pinned, |frame| {
        frame
            .bg(theme::bg_hover_strong())
            .shadow(vec![inset(theme::accent())])
    })
    .when(lit && !pinned, |frame| frame.bg(theme::bg_row_active()))
    .when_some(node.map(|node| node.tooltip.clone()), |frame, tooltip| {
        frame.tooltip(tip(tooltip, None))
    })
    .on_hover(hover_listener(key.clone(), cx))
    .on_click(cx.listener(move |this, _, _, cx| {
        cx.stop_propagation();
        toggle_pin(this, key.clone(), cx);
    }))
    .child(card_badge(card, lit))
    .child(
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(text(
                card.name.clone(),
                13.0,
                600.0,
                if ghost {
                    theme::text_muted()
                } else if lit {
                    theme::accent_bright()
                } else {
                    theme::text_primary()
                },
            ))
            .when(ghost, |name| {
                name.child(mono("not installed", 10.0, 500.0, theme::warn_text()))
            })
            .when(node.is_some_and(|node| node.everything_else), |name| {
                name.child(text("\u{221e}", 12.0, 600.0, theme::accent_bright()))
            }),
    )
    .child(div().flex_1())
    // The meta gives way on a narrow card ("1 rule · falls through" beside
    // "Orion not installed" at 266 wide), cut with "…".
    .children(node.and_then(|node| node.meta.clone()).map(|meta| {
        div()
            .min_w_0()
            .flex_shrink(1.0)
            .overflow_hidden()
            .child(one_line(
                meta,
                theme::mono_family(500.0),
                500.0,
                10.0,
                12.0,
                theme::text_subtle(),
            ))
    }))
    .children(node.and_then(|node| node.links).map(|n| {
        div().w(px(40.0)).flex_none().flex().justify_end().child(mono(
            crate::activity::thousands(n),
            10.5,
            500.0,
            theme::text_secondary(),
        ))
    }));
    let node_bounds = bounds.clone();
    let element = div()
        .on_children_prepainted(move |children, _, _| {
            if let Some(frame) = children.first() {
                node_bounds.borrow_mut().nodes.insert((index, 0), *frame);
            }
        })
        .relative()
        .flex_none()
        .child(frame)
        .child(port(false, fan, port_style))
        .when_some(card.ribbon.clone(), |element, label| {
            element.child(ribbon(label))
        });
    let (from, to) = trace.light(|lit| lit.warm(index)).opacities();
    faded(
        element,
        ("overview-card-dim", index * 1_000_000 + trace.generation).into(),
        from,
        to,
    )
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
