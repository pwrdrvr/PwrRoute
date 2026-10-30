//! The menu bar inside the title strip on Windows and Linux (design:
//! "PwrRoute Windows and Linux Chrome" 1a/1b, after PwrGit's `AppMenuBar`).
//!
//! Neither OS has a global menu bar, so the strip draws
//! `app_menu::menus(Platform::Other)` after the brand: a click on a title
//! opens its menu (`widgets::menu_card`, design: Popovers and Overflow
//! 1a-1d), hovering across the titles while one is open switches menus,
//! and Alt or F10 puts the keyboard on the titles. Each row shows its
//! shortcut from the keymap and dispatches its action; Window's list of
//! open windows follows Minimize.
//!
//! Keys reach the bar through a keystroke interceptor, which runs before
//! any binding or field sees them, so focus never moves to the bar: a row
//! dispatches in the window as it was, and Edit's rows replay their key
//! into the field that had it, as the macOS menu bar does.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::theme::px;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Action, AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, Global, HighlightStyle,
    InteractiveElement as _, IntoElement, Keystroke, MenuItem, MouseButton, MouseDownEvent,
    ParentElement as _, Pixels, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    StyledText, Subscription, UnderlineStyle, WeakEntity, Window, deferred, div,
};

use crate::app_menu::{self, Platform};
use crate::e2e;
use crate::theme;
use crate::widgets::{menu_card, menu_divider, menu_row};

/// The dropdown's width: Help's longest row, "Report a Security
/// Vulnerability…", with room for a shortcut.
pub const MENU_WIDTH: f32 = 264.0;

/// From a title's top edge to its dropdown's: the strip's fill is 40px and
/// a 22px title sits 9px down, so the card starts 2px under the strip's
/// 1px border.
const CARD_TOP: f32 = 33.0;

/// The titles' regions for `e2e::regions`, in the template's order.
const TITLE_REGIONS: &[&str] = &[
    "menuBar.file",
    "menuBar.edit",
    "menuBar.view",
    "menuBar.window",
    "menuBar.help",
];

/// One row of a strip menu.
pub enum Row {
    Separator,
    /// A template row: its label, its action and the keymap's shortcut.
    Action {
        label: SharedString,
        action: Box<dyn Action>,
        shortcut: Option<SharedString>,
    },
    /// Window's list: an open window by name, checked when it is this one.
    Window {
        label: SharedString,
        window: AnyWindowHandle,
        current: bool,
    },
}

impl Row {
    fn label(&self) -> Option<&SharedString> {
        match self {
            Self::Separator => None,
            Self::Action { label, .. } | Self::Window { label, .. } => Some(label),
        }
    }
}

/// A title and its rows, as the strip draws them.
pub struct StripMenu {
    pub name: SharedString,
    pub rows: Vec<Row>,
}

/// The strip's menus for the window `current`: the template's, each row
/// with its shortcut, and Window's list of the windows with a strip.
pub fn strip_menus(current: Option<AnyWindowHandle>, cx: &App) -> Vec<StripMenu> {
    app_menu::menus(Platform::Other)
        .into_iter()
        .map(|menu| {
            let mut rows: Vec<Row> = menu
                .items
                .into_iter()
                .filter_map(|item| match item {
                    MenuItem::Separator => Some(Row::Separator),
                    MenuItem::Action { name, action, .. } => {
                        let shortcut = shortcut(action.as_ref(), cx);
                        Some(Row::Action {
                            label: name,
                            action,
                            shortcut,
                        })
                    }
                    // Neither is in the template off macOS.
                    MenuItem::Submenu(_) | MenuItem::SystemMenu(_) => None,
                })
                .collect();
            if menu.name == "Window" {
                let windows = open_windows(cx);
                if !windows.is_empty() {
                    rows.push(Row::Separator);
                }
                rows.extend(windows.into_iter().map(|(window, label)| Row::Window {
                    label,
                    window,
                    current: Some(window) == current,
                }));
            }
            StripMenu {
                name: menu.name,
                rows,
            }
        })
        .collect()
}

/// The keymap's last binding for `action` (the one the menu shows), as
/// "Ctrl+W". Edit's live in `MENU_ONLY`, which the keymap lists all the
/// same.
fn shortcut(action: &dyn Action, cx: &App) -> Option<SharedString> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let binding = keymap.bindings_for_action(action).last()?;
    match binding.keystrokes() {
        [keystroke] => Some(shortcut_label(keystroke.inner()).into()),
        _ => None,
    }
}

/// A keystroke in Windows' and GNOME's words: "Ctrl+Shift+N", "F10".
pub fn shortcut_label(keystroke: &Keystroke) -> String {
    let modifiers = &keystroke.modifiers;
    let mut parts: Vec<String> = Vec::new();
    if modifiers.control {
        parts.push("Ctrl".into());
    }
    if modifiers.alt {
        parts.push("Alt".into());
    }
    if modifiers.shift {
        parts.push("Shift".into());
    }
    if modifiers.platform {
        parts.push(
            if cfg!(target_os = "windows") {
                "Win"
            } else {
                "Super"
            }
            .into(),
        );
    }
    let key = keystroke.key.as_str();
    parts.push(match key {
        "escape" => "Esc".into(),
        "delete" => "Del".into(),
        _ => {
            let mut chars = key.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        }
    });
    parts.join("+")
}

/// Where the keyboard and the pointer are in the bar. Plain data, so the
/// tests can walk it without a window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// The title in hand: open, or under the keyboard with its menu shut.
    /// `None` is the bar at rest.
    pub title: Option<usize>,
    /// Its menu is open.
    pub open: bool,
    /// The row lit in the open menu, by index into its rows.
    pub row: Option<usize>,
    /// The keyboard has the bar (Alt, F10, an arrow): the titles underline
    /// their letters and the one in hand has the accent ring.
    pub keyboard: bool,
}

/// What a key does to the bar, besides moving it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Not the bar's: the bindings and the focused field get it.
    Pass,
    /// The bar took it.
    Handled,
    /// Run this menu's row; the bar is already at rest.
    Run { menu: usize, row: usize },
}

/// A key, as the bar reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Enter,
    Escape,
    F10,
    /// A letter with Alt alone: its menu's mnemonic.
    Alt(char),
    /// A letter on its own.
    Letter(char),
    /// Shift, Alt, Ctrl or the platform key on its own.
    Modifier,
    /// A shortcut (Ctrl, the platform key, or Alt with anything but a
    /// letter: Alt+F4).
    Chord,
    Other,
}

impl Key {
    pub fn of(keystroke: &Keystroke) -> Self {
        let modifiers = &keystroke.modifiers;
        let key = keystroke.key.as_str();
        if matches!(key, "alt" | "shift" | "control" | "platform" | "function") {
            return Self::Modifier;
        }
        let letter = {
            let mut chars = key.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphabetic() => Some(c.to_ascii_lowercase()),
                _ => None,
            }
        };
        if modifiers.control || modifiers.platform {
            return Self::Chord;
        }
        if modifiers.alt {
            return match letter {
                Some(c) if !modifiers.shift => Self::Alt(c),
                _ => Self::Chord,
            };
        }
        match key {
            "left" => Self::Left,
            "right" => Self::Right,
            "up" => Self::Up,
            "down" => Self::Down,
            "enter" | "space" => Self::Enter,
            "escape" => Self::Escape,
            "f10" if !modifiers.shift => Self::F10,
            _ => letter.map_or(Self::Other, Self::Letter),
        }
    }
}

/// The menus as the keyboard sees them: each title's letter and which of
/// its rows can be lit (every one but the separators).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shape {
    pub menus: Vec<(char, Vec<bool>)>,
}

impl Shape {
    pub fn of(menus: &[StripMenu]) -> Self {
        Self {
            menus: menus
                .iter()
                .map(|menu| {
                    let letter = menu
                        .name
                        .chars()
                        .next()
                        .map_or(' ', |c| c.to_ascii_lowercase());
                    let rows = menu.rows.iter().map(|row| row.label().is_some()).collect();
                    (letter, rows)
                })
                .collect(),
        }
    }

    fn mnemonic(&self, letter: char) -> Option<usize> {
        self.menus.iter().position(|(c, _)| *c == letter)
    }

    /// The next row that can be lit after `from` (`None`: the first),
    /// going `forward` or back, round the end.
    fn next_row(&self, menu: usize, from: Option<usize>, forward: bool) -> Option<usize> {
        let rows = &self.menus.get(menu)?.1;
        let n = rows.len();
        if n == 0 {
            return None;
        }
        let mut at = match (from, forward) {
            (Some(row), _) => row,
            (None, true) => n - 1,
            (None, false) => 0,
        };
        for _ in 0..n {
            at = if forward {
                (at + 1) % n
            } else {
                (at + n - 1) % n
            };
            if rows[at] {
                return Some(at);
            }
        }
        None
    }

    fn first_row(&self, menu: usize) -> Option<usize> {
        self.next_row(menu, None, true)
    }
}

impl State {
    /// The keyboard on `title` with its menu `open`, its first row lit.
    fn keyed(title: usize, open: bool, shape: &Shape) -> Self {
        Self {
            title: Some(title),
            open,
            row: if open { shape.first_row(title) } else { None },
            keyboard: true,
        }
    }

    /// What `key` does. At rest the bar takes only F10 and Alt with a
    /// title's letter; once it has the keyboard it takes every key but a
    /// shortcut (which also puts it back at rest) and a lone modifier.
    pub fn key(self, key: Key, shape: &Shape) -> (Self, Outcome) {
        let count = shape.menus.len();
        let Some(title) = self.title.filter(|title| *title < count) else {
            return match key {
                Key::F10 if count > 0 => (Self::keyed(0, false, shape), Outcome::Handled),
                Key::Alt(letter) => match shape.mnemonic(letter) {
                    Some(menu) => (Self::keyed(menu, true, shape), Outcome::Handled),
                    None => (self, Outcome::Pass),
                },
                _ => (self, Outcome::Pass),
            };
        };
        match key {
            Key::Modifier => (self, Outcome::Pass),
            Key::Chord => (Self::default(), Outcome::Pass),
            Key::F10 => (Self::default(), Outcome::Handled),
            // Back to the title from a menu the keyboard opened; a menu the
            // pointer opened closes outright.
            Key::Escape if self.open && self.keyboard => (
                Self {
                    open: false,
                    row: None,
                    ..self
                },
                Outcome::Handled,
            ),
            Key::Escape => (Self::default(), Outcome::Handled),
            Key::Left | Key::Right => {
                let next = if key == Key::Right {
                    (title + 1) % count
                } else {
                    (title + count - 1) % count
                };
                (Self::keyed(next, self.open, shape), Outcome::Handled)
            }
            Key::Down | Key::Up if !self.open => {
                let row = shape.next_row(title, None, key == Key::Down);
                (
                    Self {
                        open: true,
                        row,
                        keyboard: true,
                        ..self
                    },
                    Outcome::Handled,
                )
            }
            Key::Down | Key::Up => (
                Self {
                    row: shape.next_row(title, self.row, key == Key::Down),
                    keyboard: true,
                    ..self
                },
                Outcome::Handled,
            ),
            Key::Enter if !self.open => (Self::keyed(title, true, shape), Outcome::Handled),
            Key::Enter => match self.row {
                Some(row) => (Self::default(), Outcome::Run { menu: title, row }),
                None => (self, Outcome::Handled),
            },
            Key::Alt(letter) | Key::Letter(letter) => match shape.mnemonic(letter) {
                Some(menu) if !self.open || key == Key::Alt(letter) => {
                    (Self::keyed(menu, true, shape), Outcome::Handled)
                }
                _ => (self, Outcome::Handled),
            },
            Key::Other => (self, Outcome::Handled),
        }
    }

    /// Alt (`FocusMenuBar`): the keyboard on the first title, or the bar
    /// back at rest.
    pub fn toggled(self) -> Self {
        if self.title.is_some() {
            Self::default()
        } else {
            Self {
                title: Some(0),
                keyboard: true,
                ..Self::default()
            }
        }
    }

    /// A press on `title`: opens its menu, or closes it when it is open.
    pub fn title_down(self, title: usize) -> Self {
        if self.open && self.title == Some(title) {
            Self::default()
        } else {
            Self {
                title: Some(title),
                open: true,
                row: None,
                keyboard: false,
            }
        }
    }

    /// The pointer over `title`: while a menu is open, that one opens.
    pub fn title_hover(self, title: usize) -> Self {
        if self.open && self.title != Some(title) {
            Self {
                title: Some(title),
                row: None,
                ..self
            }
        } else {
            self
        }
    }
}

/// Every window's bar and the window's name, so Alt (an app action) and
/// Window's list find them. The name is kept here: Window's list is built
/// while a bar draws, when that bar cannot be read.
#[derive(Default)]
struct Bars(Vec<(AnyWindowHandle, WeakEntity<MenuBar>, SharedString)>);

impl Global for Bars {}

fn bar_for(window: AnyWindowHandle, cx: &App) -> Option<Entity<MenuBar>> {
    cx.try_global::<Bars>()?
        .0
        .iter()
        .find(|(handle, ..)| *handle == window)
        .and_then(|(_, bar, _)| bar.upgrade())
}

/// The windows with a strip, in the order they opened, by name.
fn open_windows(cx: &App) -> Vec<(AnyWindowHandle, SharedString)> {
    let Some(bars) = cx.try_global::<Bars>() else {
        return Vec::new();
    };
    let open = cx.windows();
    bars.0
        .iter()
        .filter(|(handle, bar, _)| open.contains(handle) && bar.upgrade().is_some())
        .map(|(handle, _, name)| (*handle, name.clone()))
        .collect()
}

/// How long after PwrRoute brings a window forward a lone Alt is taken for
/// GPUI's, not the user's (see [`bring_forward`]).
pub const FAKE_ALT_WINDOW: Duration = Duration::from_millis(600);

/// When PwrRoute last brought a window forward.
struct BroughtForward(Instant);

impl Global for BroughtForward {}

/// Brings `window` forward. Every PwrRoute call that does goes through
/// here: on Windows GPUI's `activate_window` taps Alt itself (`SendInput`,
/// so Windows lets the window take the foreground), and that tap must not
/// read as the user's Alt and put the keyboard on File.
pub fn bring_forward(window: &mut Window, cx: &mut App) {
    note_brought_forward(cx);
    window.activate_window();
}

/// Starts [`FAKE_ALT_WINDOW`]: on a call to `bring_forward`, and whenever a
/// window becomes active, since activation re-reports a held Alt (Alt+Tab
/// back in) and its release would read as a lone Alt.
fn note_brought_forward(cx: &mut App) {
    let now = cx.background_executor().now();
    cx.set_global(BroughtForward(now));
}

/// A lone Alt now is GPUI's tap from [`bring_forward`].
fn alt_is_gpuis(cx: &App) -> bool {
    cx.try_global::<BroughtForward>().is_some_and(|brought| {
        cx.background_executor()
            .now()
            .saturating_duration_since(brought.0)
            < FAKE_ALT_WINDOW
    })
}

/// Off macOS: the keys (`menu_bar` takes them before anything else) and
/// Alt's action.
pub fn install(cx: &mut App) {
    cx.intercept_keystrokes(|event, window, cx| {
        let Some(bar) = bar_for(window.window_handle(), cx) else {
            return;
        };
        if bar.update(cx, |bar, cx| bar.keystroke(&event.keystroke, window, cx)) {
            cx.stop_propagation();
        }
    })
    .detach();
    cx.on_action(|_: &app_menu::FocusMenuBar, cx| {
        let Some(bar) = cx.active_window().and_then(|window| bar_for(window, cx)) else {
            return;
        };
        let gpuis = alt_is_gpuis(cx);
        bar.update(cx, |bar, cx| {
            // An Alt that clicked is not a lone Alt, though GPUI counts only
            // keys as using it.
            let clicked = std::mem::take(&mut bar.alt_clicked);
            if !gpuis && !clicked {
                bar.state = bar.state.toggled();
                cx.notify();
            }
        });
    });
}

/// One window's menu bar. Every window with a strip owns one; only the
/// Windows and Linux strip draws it.
pub struct MenuBar {
    /// The window's name in Window's list.
    name: SharedString,
    pub state: State,
    /// The titles' bounds as last painted: a press there is the title's,
    /// not a press outside the open menu.
    titles: Rc<RefCell<Vec<Bounds<Pixels>>>>,
    /// A press with Alt held since the last key: its release is not Alt's.
    alt_clicked: bool,
    window: Option<AnyWindowHandle>,
    _activation: Option<Subscription>,
}

impl MenuBar {
    pub fn new(name: impl Into<SharedString>) -> Self {
        Self {
            name: name.into(),
            state: State::default(),
            titles: Rc::default(),
            alt_clicked: false,
            window: None,
            _activation: None,
        }
    }

    /// A bar for a window named `name`, as an entity.
    pub fn entity(name: impl Into<SharedString>, cx: &mut App) -> Entity<Self> {
        let name = name.into();
        cx.new(|_| Self::new(name))
    }

    /// Joins the window on first draw: listed for Alt and Window's list,
    /// and back at rest whenever the window stops being the active one.
    fn attach(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.window.is_some() {
            return;
        }
        let handle = window.window_handle();
        self.window = Some(handle);
        let me = cx.entity().downgrade();
        let name = self.name.clone();
        let bars = cx.default_global::<Bars>();
        bars.0.retain(|(_, bar, _)| bar.upgrade().is_some());
        bars.0.push((handle, me, name));
        self._activation = Some(cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                note_brought_forward(cx);
            } else {
                this.rest(cx);
            }
        }));
    }

    fn rest(&mut self, cx: &mut Context<'_, Self>) {
        if self.state != State::default() {
            self.state = State::default();
            cx.notify();
        }
    }

    fn set(&mut self, state: State, cx: &mut Context<'_, Self>) {
        if self.state != state {
            self.state = state;
            cx.notify();
        }
    }

    /// A key in this bar's window; `true` when the bar took it.
    fn keystroke(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let key = Key::of(keystroke);
        // A key after the click uses Alt up as GPUI sees it; a modifier
        // alone (a simulated lone Alt) does not.
        if key != Key::Modifier {
            self.alt_clicked = false;
        }
        // At rest only F10 and Alt+letter are the bar's: skip building the
        // menus for every other key typed.
        if self.state.title.is_none() && !matches!(key, Key::F10 | Key::Alt(_)) {
            return false;
        }
        let shape = Shape::of(&strip_menus(self.window, cx));
        let (state, outcome) = self.state.key(key, &shape);
        self.set(state, cx);
        match outcome {
            Outcome::Pass => false,
            Outcome::Handled => true,
            Outcome::Run { menu, row } => {
                self.run(menu, row, window, cx);
                true
            }
        }
    }

    /// Runs `menu`'s `row`, with the bar put back at rest first. An action
    /// goes to the window's focused element, as the row's shortcut would;
    /// GPUI dispatches it once this event is done.
    fn run(&mut self, menu: usize, row: usize, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.rest(cx);
        let mut menus = strip_menus(self.window, cx);
        let Some(menu) = menus.get_mut(menu) else {
            return;
        };
        if row >= menu.rows.len() {
            return;
        }
        match menu.rows.swap_remove(row) {
            Row::Action { action, .. } => window.dispatch_action(action, cx),
            Row::Window { window: target, .. } => cx.defer(move |cx| {
                let _ = target.update(cx, |_, window, cx| bring_forward(window, cx));
            }),
            Row::Separator => {}
        }
    }

    /// A title, with its menu's `rows` when it is the open one.
    fn title(
        &self,
        index: usize,
        name: &SharedString,
        rows: Option<Vec<Row>>,
        max_height: f32,
        cx: &mut Context<'_, Self>,
    ) -> impl IntoElement {
        let state = self.state;
        let held = state.title == Some(index);
        let open = held && state.open;
        let ring = held && state.keyboard && !state.open;
        let slug = slug(name);
        let label = {
            let text = StyledText::new(name.clone());
            // Alt's letters, underlined while the keyboard has the bar.
            if state.keyboard && state.title.is_some() {
                let first = name.chars().next().map_or(0, char::len_utf8);
                text.with_highlights([(
                    0..first,
                    HighlightStyle {
                        underline: Some(UnderlineStyle {
                            thickness: px(1.0),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )])
            } else {
                text
            }
        };
        let title_id = format!("menu-title-{slug}");
        div()
            .id(("menu-title", index))
            .debug_selector(move || title_id)
            .relative()
            .occlude()
            .px(px(6.0))
            .py(px(4.0))
            .border_1()
            .border_color(if ring {
                theme::accent()
            } else {
                theme::transparent()
            })
            .rounded(px(7.0))
            .font_family(theme::SANS)
            .font_weight(theme::weight(500.0))
            .text_size(px(12.0))
            .line_height(px(12.0))
            .whitespace_nowrap()
            .text_color(if open || ring {
                theme::text_primary()
            } else {
                theme::text_secondary()
            })
            .when(open || ring, |title| title.bg(theme::bg_hover()))
            .when(!open, |title| {
                title.hover(|style| style.bg(theme::bg_hover()))
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    this.alt_clicked |= event.modifiers.alt;
                    let state = this.state.title_down(index);
                    this.set(state, cx);
                }),
            )
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if *hovered {
                    let state = this.state.title_hover(index);
                    this.set(state, cx);
                }
            }))
            .child(label)
            .when_some(rows.filter(|_| open), |title, rows| {
                title.child(deferred(self.card(index, &slug, rows, max_height, cx)))
            })
    }

    fn card(
        &self,
        index: usize,
        slug: &str,
        rows: Vec<Row>,
        max_height: f32,
        cx: &mut Context<'_, Self>,
    ) -> impl IntoElement {
        let lit = self.state.row;
        let titles = self.titles.clone();
        let card_id = format!("menu-card-{slug}");
        let rows = rows.into_iter().enumerate().map(|(row, item)| {
            // A window's row keeps its window: the list can change before
            // the click, and the row's index would then name another.
            let (label, hint, target) = match item {
                Row::Separator => return menu_divider().into_any_element(),
                Row::Action {
                    label, shortcut, ..
                } => (label, shortcut, None),
                Row::Window {
                    label,
                    current,
                    window,
                } => (
                    label,
                    current.then(|| SharedString::from("\u{2713}")),
                    Some(window),
                ),
            };
            let row_id = format!("menu-row-{slug}-{}", self::slug(&label));
            menu_row(("menu-row", index * 1000 + row), None, label, hint)
                .debug_selector(move || row_id)
                // `.mi.hov` (design: Popovers and Overflow 1a). The pointer
                // lights the row it is over, so one highlight serves both.
                .when(lit == Some(row), |item| item.bg(theme::bg_row_active()))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    if *hovered {
                        let state = State {
                            row: Some(row),
                            ..this.state
                        };
                        this.set(state, cx);
                    }
                }))
                .on_click(cx.listener(move |this, _, window, cx| match target {
                    Some(target) => {
                        this.rest(cx);
                        cx.defer(move |cx| {
                            let _ = target.update(cx, |_, window, cx| bring_forward(window, cx));
                        });
                    }
                    None => this.run(index, row, window, cx),
                }))
                .into_any_element()
        });
        menu_card(("menu-card", index))
            .existing_navigation()
            .debug_selector(move || card_id)
            .absolute()
            .top(px(CARD_TOP))
            .left(px(-1.0))
            .w(px(MENU_WIDTH))
            .max_h(px(max_height))
            .on_mouse_down_out(cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                // A title's own press opens or closes it.
                if !titles
                    .borrow()
                    .iter()
                    .any(|bounds| bounds.contains(&event.position))
                {
                    this.rest(cx);
                }
            }))
            .children(rows)
    }
}

impl Render for MenuBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.attach(window, cx);
        // Under the strip, down to 8px above the window's bottom edge; past
        // that the card scrolls.
        let max_height =
            (theme::css(window.viewport_size().height) - crate::chrome::STRIP_HEIGHT - 11.0)
                .max(120.0);
        // Only the open menu's rows are built (shortcuts, Window's list);
        // the rest of the time the bar is five words.
        let mut open = self
            .state
            .title
            .filter(|_| self.state.open)
            .and_then(|title| strip_menus(self.window, cx).into_iter().nth(title));
        let titles = self.titles.clone();
        let items: Vec<_> = app_menu::menus(Platform::Other)
            .into_iter()
            .enumerate()
            .map(|(index, menu)| {
                let rows = open
                    .take_if(|_| self.state.title == Some(index))
                    .map(|menu| menu.rows);
                self.title(index, &menu.name, rows, max_height, cx)
                    .into_any_element()
            })
            .collect();
        div()
            .on_children_prepainted(move |bounds, window, cx| {
                titles.borrow_mut().clone_from(&bounds);
                e2e::regions(TITLE_REGIONS)(bounds, window, cx);
            })
            .id("menu-bar")
            .debug_selector(|| "menu-bar".into())
            .flex()
            .flex_none()
            .items_center()
            .gap(px(1.0))
            .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                this.alt_clicked |= event.modifiers.alt;
                // Alt or F10 with no menu open: a press anywhere else lets
                // go. An open menu's card decides for itself.
                if this.state.title.is_some() && !this.state.open {
                    this.rest(cx);
                }
            }))
            .children(items)
    }
}

/// A name as a selector: lowercase words joined by "-" ("Check for
/// Updates…" is "check-for-updates").
pub fn slug(name: &str) -> String {
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// File, Edit, View, Window and Help with the template's rows and
    /// separators, and Window's list of two windows.
    fn shape() -> Shape {
        let rows = |items: &[bool]| items.to_vec();
        Shape {
            menus: vec![
                ('f', rows(&[true, false, true, false, true, true])),
                ('e', rows(&[true, true, true, false, true])),
                ('v', rows(&[true, true, true, true])),
                ('w', rows(&[true, false, true, true])),
                ('h', rows(&[true, true, false, true])),
            ],
        }
    }

    fn keys(mut state: State, keys: &[Key]) -> (State, Outcome) {
        let shape = shape();
        let mut outcome = Outcome::Pass;
        for key in keys {
            (state, outcome) = state.key(*key, &shape);
        }
        (state, outcome)
    }

    #[test]
    fn at_rest_only_f10_and_alt_letters_are_the_bars() {
        let rest = State::default();
        for key in [
            Key::Down,
            Key::Enter,
            Key::Escape,
            Key::Letter('f'),
            Key::Chord,
        ] {
            assert_eq!(rest.key(key, &shape()), (rest, Outcome::Pass), "{key:?}");
        }
        // Alt with a letter no title starts with is the field's.
        assert_eq!(rest.key(Key::Alt('z'), &shape()).1, Outcome::Pass);
        assert_eq!(
            rest.key(Key::F10, &shape()),
            (
                State {
                    title: Some(0),
                    open: false,
                    row: None,
                    keyboard: true
                },
                Outcome::Handled
            )
        );
        // Alt+V opens View with its first row lit.
        assert_eq!(
            rest.key(Key::Alt('v'), &shape()).0,
            State {
                title: Some(2),
                open: true,
                row: Some(0),
                keyboard: true
            }
        );
    }

    #[test]
    fn arrows_walk_titles_and_rows_and_skip_separators() {
        let (state, _) = keys(State::default(), &[Key::F10, Key::Right, Key::Right]);
        assert_eq!((state.title, state.open), (Some(2), false));
        // Left from File wraps to Help.
        let (state, _) = keys(State::default(), &[Key::F10, Key::Left]);
        assert_eq!(state.title, Some(4));
        // Down opens File on its first row; the next skips the separator.
        let (state, _) = keys(State::default(), &[Key::F10, Key::Down, Key::Down]);
        assert_eq!((state.open, state.row), (true, Some(2)));
        // Up from the first row wraps to the last.
        let (state, _) = keys(State::default(), &[Key::F10, Key::Down, Key::Up]);
        assert_eq!(state.row, Some(5));
        // Up on a shut title opens it on its last row.
        let (state, _) = keys(State::default(), &[Key::F10, Key::Up]);
        assert_eq!((state.open, state.row), (true, Some(5)));
        // Right with a menu open opens the next on its first row.
        let (state, _) = keys(State::default(), &[Key::Alt('f'), Key::Down, Key::Right]);
        assert_eq!(
            state,
            State {
                title: Some(1),
                open: true,
                row: Some(0),
                keyboard: true
            }
        );
    }

    #[test]
    fn enter_runs_the_lit_row_and_puts_the_bar_at_rest() {
        let (state, outcome) = keys(
            State::default(),
            &[Key::Alt('h'), Key::Down, Key::Down, Key::Enter],
        );
        assert_eq!(state, State::default());
        assert_eq!(outcome, Outcome::Run { menu: 4, row: 3 });
        // Enter on a shut title opens it.
        let (state, outcome) = keys(State::default(), &[Key::F10, Key::Enter]);
        assert_eq!(
            (state.open, state.row, outcome),
            (true, Some(0), Outcome::Handled)
        );
    }

    #[test]
    fn escape_steps_back_and_shortcuts_pass_through() {
        // From a menu the keyboard opened, back to its title, then rest.
        let (state, _) = keys(State::default(), &[Key::Alt('e'), Key::Escape]);
        assert_eq!((state.title, state.open, state.row), (Some(1), false, None));
        let (state, outcome) = keys(state, &[Key::Escape]);
        assert_eq!((state, outcome), (State::default(), Outcome::Handled));
        // A menu the pointer opened closes outright.
        let state = State::default().title_down(3);
        assert_eq!(
            state.key(Key::Escape, &shape()),
            (State::default(), Outcome::Handled)
        );
        // Ctrl+Q (or Alt+F4) while the bar has the keyboard: the bar lets
        // go and the shortcut runs. A lone modifier changes nothing.
        let open = State::default().title_down(0);
        assert_eq!(
            open.key(Key::Chord, &shape()),
            (State::default(), Outcome::Pass)
        );
        assert_eq!(open.key(Key::Modifier, &shape()), (open, Outcome::Pass));
        // Any other key is swallowed while the bar has it.
        assert_eq!(open.key(Key::Other, &shape()), (open, Outcome::Handled));
        // F10 again lets go.
        let (state, outcome) = keys(State::default(), &[Key::F10, Key::F10]);
        assert_eq!((state, outcome), (State::default(), Outcome::Handled));
    }

    #[test]
    fn letters_open_menus_by_their_mnemonic() {
        let (state, _) = keys(State::default(), &[Key::F10, Key::Letter('w')]);
        assert_eq!(
            (state.title, state.open, state.row),
            (Some(3), true, Some(0))
        );
        // Inside an open menu a plain letter is not a title's.
        let (state, _) = keys(state, &[Key::Letter('h')]);
        assert_eq!(state.title, Some(3));
        // Alt+letter still switches.
        let (state, _) = keys(state, &[Key::Alt('h')]);
        assert_eq!(state.title, Some(4));
    }

    #[test]
    fn the_pointer_opens_toggles_and_switches_menus() {
        let rest = State::default();
        // Hovering at rest opens nothing.
        assert_eq!(rest.title_hover(2), rest);
        let file = rest.title_down(0);
        assert_eq!(
            (file.title, file.open, file.keyboard),
            (Some(0), true, false)
        );
        // Across to View while File is open: View opens.
        let view = file.title_hover(2);
        assert_eq!((view.title, view.open), (Some(2), true));
        // A second press on the open title closes it.
        assert_eq!(view.title_down(2), rest);
        // Alt toggles.
        assert_eq!(rest.toggled().title, Some(0));
        assert_eq!(rest.toggled().toggled(), rest);
    }

    #[test]
    fn keys_read_as_the_bar_reads_them() {
        let key = |text: &str| Key::of(&Keystroke::parse(text).expect("a keystroke"));
        assert_eq!(key("f10"), Key::F10);
        assert_eq!(key("alt-f"), Key::Alt('f'));
        assert_eq!(key("alt-f4"), Key::Chord);
        assert_eq!(key("ctrl-q"), Key::Chord);
        assert_eq!(key("alt"), Key::Modifier);
        assert_eq!(key("space"), Key::Enter);
        assert_eq!(key("shift-x"), Key::Letter('x'));
        assert_eq!(key("tab"), Key::Other);
    }

    #[test]
    fn shortcuts_read_in_windows_words() {
        let label = |text: &str| shortcut_label(&Keystroke::parse(text).expect("a keystroke"));
        assert_eq!(label("ctrl-w"), "Ctrl+W");
        assert_eq!(label("ctrl-,"), "Ctrl+,");
        assert_eq!(label("ctrl-shift-n"), "Ctrl+Shift+N");
        assert_eq!(label("f10"), "F10");
        assert_eq!(label("escape"), "Esc");
    }

    /// The titles' E2E regions are positional: they name the template's
    /// menus in its order.
    #[test]
    fn title_regions_follow_the_template() {
        let expected: Vec<String> = app_menu::menus(Platform::Other)
            .iter()
            .map(|menu| format!("menuBar.{}", slug(&menu.name)))
            .collect();
        assert_eq!(TITLE_REGIONS, expected);
    }

    #[test]
    fn slugs_name_rows_for_selectors() {
        assert_eq!(slug("Check for Updates\u{2026}"), "check-for-updates");
        assert_eq!(slug("Third-Party Notices"), "third-party-notices");
        assert_eq!(slug("PwrRoute"), "pwrroute");
    }
}
