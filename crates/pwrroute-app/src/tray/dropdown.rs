//! The macOS menu bar dropdown: `design/PwrRoute Tray and First Run.dc.html`
//! options 1a (routing) and 1b (paused), with the Incoming links section
//! and the held links of `design/PwrRoute Incoming Links.dc.html` 2a-2f.
//!
//! A 300px GPUI `PopUp` window (a non-activating `NSPanel`) opened under the
//! menu bar item. It takes key focus without activating the app, so the
//! frontmost app keeps its menu bar, and it closes when it loses focus: a
//! click anywhere else, Esc, a command, or the item again.
//!
//! The window is the card (design: Popovers and Overflow 2a). It opens at
//! the card's height ([`fitted_height`]) and follows it whenever the model
//! changes: pausing, a new recent link, the Make default row. GPUI cannot
//! move a window and macOS keeps a window's bottom edge when it resizes, so
//! the resize goes through `macos::set_dropdown_height`, which keeps the
//! top under the menu bar. A transparent window taller than the card is not
//! invisible: macOS draws its frame and shadow at the window's size. The
//! window never runs past the display's visible area; on a display too
//! short for the card, the header and Quit stay and the middle scrolls
//! (2b).
//!
//! The keyboard (Incoming Links 2f): Tab and Shift-Tab walk the switch (one
//! stop, a radio group), the To row, the For choice, its stepper and the
//! live line's button. Left and Right move the switch and the For choice
//! and turn the new one on; Up and Down step a count or a time; Return or
//! Space presses. The focused control wears the 2px accent ring
//! (`.inp.focus`) while the keyboard drives.

use std::rc::Rc;
use std::time::SystemTime;

use crate::theme::px;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, DisplayId, Div, Entity,
    FocusHandle, Font, HighlightStyle, InteractiveElement as _, IntoElement, KeyBinding,
    KeyDownEvent, LineFragment, MouseButton, ParentElement as _, Pixels, Point, Render,
    StatefulInteractiveElement as _, Styled as _, StyledText, Subscription, TextSystem, Window,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions, div, font,
    size,
};
use pwrroute_router::{DispatchId, OpenAs};

use super::TrayCommand;
use super::model::{
    ForKind, HeaderChip, HeldLink, Live, Mode, Recent, Run, SendDestination, SyncRow, SyncTone,
    Tone, TrayModel,
};
use crate::destinations::{self, Density, Lane};
use crate::theme::{self, icon, mono, text, tracked};
use crate::widgets::{badge, key_cap, one_line};

/// The element ids tests and the inspector find things by.
pub const CARD_ID: &str = "tray-card";
/// The Incoming links switch's segments, in [`Mode::ALL`]'s order.
pub const MODE_IDS: [&str; 4] = [
    "tray-mode-rules",
    "tray-mode-send",
    "tray-mode-hold",
    "tray-mode-pause",
];
/// The For choices, in [`ForKind::ALL`]'s order.
pub const FOR_IDS: [&str; 3] = ["tray-for-links", "tray-for-time", "tray-for-until-off"];
pub const SWITCH_ID: &str = "tray-switch";
pub const FOR_ROW_ID: &str = "tray-for";
pub const HINT_ID: &str = "tray-hint";
pub const LIVE_ID: &str = "tray-live";
pub const LIVE_BUTTON_ID: &str = "tray-live-button";
pub const DESTINATION_ID: &str = "tray-destination";
pub const DESTINATIONS_ID: &str = "tray-destinations";
/// The lanes' selectors: `tray-lanes-lane-{l}`, `tray-lanes-{l}-{p}`.
pub const LANES_ID: &str = "tray-lanes";
pub const STEPPER_ID: &str = "tray-stepper";
pub const STEP_DOWN_ID: &str = "tray-step-down";
pub const STEP_UP_ID: &str = "tray-step-up";
pub const HELD_ID: &str = "tray-held";
pub const OPEN_ALL_ID: &str = "tray-held-open-all";
/// A held row's lanes, for Open in…: `tray-held-lanes-lane-{l}`, and so on.
pub const HELD_LANES_ID: &str = "tray-held-lanes";
pub const MAKE_DEFAULT_ID: &str = "tray-make-default";
pub const QUIT_ID: &str = "tray-quit";
/// Sync's row (design: Synced Rules 3c).
pub const SYNC_ID: &str = "tray-sync";
/// The sections between the header and Quit, which scroll on a display too
/// short for the card.
pub const BODY_ID: &str = "tray-body";

/// A held row's controls: `tray-held-{what}-{index}`.
pub fn held_id(what: &str, index: usize) -> String {
    format!("tray-held-{what}-{index}")
}

/// `.tray { width: 300px }`.
pub const WIDTH: f32 = 300.0;

#[allow(
    clippy::derive_partial_eq_without_eq,
    reason = "GPUI's actions! macro writes the derives, not us"
)]
mod dropdown_actions {
    use gpui_kit::actions;

    actions!(
        pwrroute_tray,
        [
            /// Close the dropdown.
            Dismiss,
            /// Test a link… (⌘T).
            TestLink,
            /// Rules… (⌘R).
            Rules,
            /// Settings… (⌘,).
            Settings,
            /// Tab: the next control.
            FocusNext,
            /// Shift-Tab: the one before.
            FocusPrev,
        ]
    );
}
use dropdown_actions::*;

const CONTEXT: &str = "TrayDropdown";

/// The dropdown's shortcuts, live only while it has focus. ⌘Q is the app's.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Dismiss, Some(CONTEXT)),
        KeyBinding::new("cmd-t", TestLink, Some(CONTEXT)),
        KeyBinding::new("cmd-r", Rules, Some(CONTEXT)),
        KeyBinding::new("cmd-,", Settings, Some(CONTEXT)),
        KeyBinding::new("tab", FocusNext, Some(CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrev, Some(CONTEXT)),
    ]);
}

/// Asks the OS to make the window's content this tall, keeping its top
/// edge. Tests record the height instead.
pub type Resize = Rc<dyn Fn(Pixels)>;

/// The OS's resize: AppKit on macOS, keeping the top edge.
fn native_resize() -> Resize {
    Rc::new(|height| {
        // No visible panel: the card keeps the old window and its middle
        // scrolls until the next change.
        #[cfg(target_os = "macos")]
        if !crate::macos::set_dropdown_height(height.as_f32()) {
            log::warn!("the tray dropdown was not resized: no open panel");
        }
        #[cfg(not(target_os = "macos"))]
        let _ = height;
    })
}

/// What is open inside the card, beyond what the model says: the
/// destination list under To, and a held row's actions and its lanes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Unfolded {
    /// The destination list under the To row (design: Send Links 1b).
    pub destinations: bool,
    /// The held row whose "···" is open (Incoming Links 2d).
    pub held: Option<DispatchId>,
    /// That row's Open in… lanes are open too.
    pub held_lanes: bool,
}

/// Open the dropdown with its top-left at `origin` on `display`, at most
/// `max_height` tall: the display's visible area below `origin`.
#[cfg_attr(
    not(target_os = "macos"),
    allow(dead_code, reason = "only macOS has the dropdown")
)]
pub fn open(
    model: Entity<TrayModel>,
    origin: Point<Pixels>,
    display: Option<DisplayId>,
    max_height: Pixels,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<Dropdown>> {
    let height = fitted_height(
        model.read(cx),
        Unfolded::default(),
        &cx.text_system().clone(),
        max_height,
    );
    let window = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin,
                size: size(px(WIDTH), height),
            })),
            titlebar: None,
            focus: true,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            display_id: display,
            window_background: WindowBackgroundAppearance::Transparent,
            app_id: Some("pwrroute".to_owned()),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| Dropdown::new(model, max_height, native_resize(), window, cx)),
    )?;
    window.update(cx, |this, window, cx| {
        window.focus(&this.focus, cx);
        // GPUI opens the panel with a title bar's style mask: check the
        // content it ended up with against the card.
        this.fit(window, cx);
    })?;
    Ok(window)
}

/// The tab stops, in order (Incoming Links 2f).
pub struct Stops {
    pub switch: FocusHandle,
    pub to: FocusHandle,
    pub for_row: FocusHandle,
    pub stepper: FocusHandle,
    pub live: FocusHandle,
}

impl Stops {
    fn new(cx: &mut App) -> Self {
        // In draw order with the menu's rows, as the window's Tab walks it.
        let stop = |cx: &mut App| cx.focus_handle().tab_stop(true);
        Self {
            switch: stop(cx),
            to: stop(cx),
            for_row: stop(cx),
            stepper: stop(cx),
            live: stop(cx),
        }
    }
}

pub struct Dropdown {
    model: Entity<TrayModel>,
    pub focus: FocusHandle,
    pub stops: Stops,
    /// The command row under the pointer. GPUI resolves text color before it
    /// knows what is hovered, so the `.hov` label color comes from here.
    hovered: Option<&'static str>,
    pub unfolded: Unfolded,
    /// The tallest the window may be: the display's visible area under it.
    max_height: Pixels,
    /// The height the window was last asked to be: [`fitted_height`].
    pub height: Pixels,
    resize: Resize,
    _subscriptions: Vec<Subscription>,
}

impl Dropdown {
    pub fn new(
        model: Entity<TrayModel>,
        max_height: Pixels,
        resize: Resize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let height = fitted_height(
            model.read(cx),
            Unfolded::default(),
            &cx.text_system().clone(),
            max_height,
        );
        let subscriptions = vec![
            // The window follows the card.
            cx.observe_in(&model, window, |this, _, window, cx| this.fit(window, cx)),
            // A click anywhere else takes focus away: close, as a menu does.
            cx.observe_window_activation(window, |_, window, cx| {
                if !window.is_window_active() {
                    super::dropdown_closed(cx);
                    window.remove_window();
                }
            }),
        ];
        Self {
            model,
            focus: cx.focus_handle(),
            stops: Stops::new(cx),
            hovered: None,
            unfolded: Unfolded::default(),
            max_height,
            height,
            resize,
            _subscriptions: subscriptions,
        }
    }

    /// Size the window to the card as the model has it now. The OS call
    /// runs on the foreground executor, outside any update, as GPUI's own
    /// `Window::resize` does: AppKit reports the new size to GPUI at once,
    /// and GPUI drops that report while the app is borrowed (in an update
    /// or an effect flush), leaving the card drawn at the old height.
    pub fn fit(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        // A held row that was opened is no longer listed: fold it.
        if let Some(id) = self.unfolded.held
            && !self
                .model
                .read(cx)
                .shown_held()
                .iter()
                .any(|held| held.id == id)
        {
            self.unfolded.held = None;
            self.unfolded.held_lanes = false;
        }
        if self.model.read(cx).mode() != Mode::Send {
            self.unfolded.destinations = false;
        }
        let want = fitted_height(
            self.model.read(cx),
            self.unfolded,
            &cx.text_system().clone(),
            self.max_height,
        );
        self.height = want;
        if (want - window.viewport_size().height).abs() >= px(0.5) {
            let resize = self.resize.clone();
            cx.spawn(async move |_, _| resize(want)).detach();
        }
        cx.notify();
    }

    /// Close, then run `command` once this window is gone.
    fn run(&mut self, command: TrayCommand, window: &mut Window, cx: &mut Context<'_, Self>) {
        super::dropdown_closed(cx);
        window.remove_window();
        cx.defer(move |cx| super::run(command, cx));
    }

    /// A segment of the switch. The dropdown stays open, so the user sees
    /// it take.
    fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.unfolded.destinations = false;
        super::set_mode(mode, cx);
        self.fit(window, cx);
    }

    fn set_for(&mut self, kind: ForKind, window: &mut Window, cx: &mut Context<'_, Self>) {
        super::set_for(kind, cx);
        self.fit(window, cx);
    }

    fn step(&mut self, delta: i64, window: &mut Window, cx: &mut Context<'_, Self>) {
        super::step(delta, cx);
        self.fit(window, cx);
    }

    fn toggle_destinations(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.unfolded.destinations = !self.unfolded.destinations;
        self.fit(window, cx);
    }

    fn choose(
        &mut self,
        destination: SendDestination,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.unfolded.destinations = false;
        super::choose_destination(destination, cx);
        self.fit(window, cx);
    }

    /// "···" on a held row: its actions in place, under it.
    fn toggle_held(&mut self, id: DispatchId, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.unfolded.held == Some(id) {
            self.unfolded.held = None;
        } else {
            self.unfolded.held = Some(id);
        }
        self.unfolded.held_lanes = false;
        self.fit(window, cx);
    }

    fn toggle_held_lanes(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.unfolded.held_lanes = !self.unfolded.held_lanes;
        self.fit(window, cx);
    }

    /// Open, or a pill under Open in…: the router opens it; the feed then
    /// drops the row.
    fn open_held(
        &mut self,
        id: DispatchId,
        how: OpenAs,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.unfolded = Unfolded {
            destinations: self.unfolded.destinations,
            ..Unfolded::default()
        };
        super::open_held(id, how, cx);
        self.fit(window, cx);
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        // Esc folds what is open first (the To lanes, a held link's
        // actions), then closes.
        if self.unfolded != Unfolded::default() {
            self.unfolded = Unfolded::default();
            window.focus(&self.focus, cx);
            self.fit(window, cx);
            return;
        }
        super::dropdown_closed(cx);
        window.remove_window();
    }

    /// Left and Right on the switch: the next segment, turned on. Hold is
    /// passed over while it cannot turn on.
    fn arrow_mode(&mut self, delta: isize, window: &mut Window, cx: &mut Context<'_, Self>) {
        let model = self.model.read(cx);
        let usable: Vec<Mode> = Mode::ALL
            .into_iter()
            // The lit segment always counts, so the arrows move from where
            // the switch is.
            .filter(|mode| *mode != Mode::Hold || model.can_hold || *mode == model.mode())
            .collect();
        let at = usable
            .iter()
            .position(|mode| *mode == model.mode())
            .unwrap_or(0);
        let next = at.saturating_add_signed(delta).min(usable.len() - 1);
        if next != at {
            self.set_mode(usable[next], window, cx);
        }
    }

    fn arrow_for(&mut self, delta: isize, window: &mut Window, cx: &mut Context<'_, Self>) {
        let shown = self.model.read(cx).for_shown();
        let at = ForKind::ALL
            .iter()
            .position(|kind| *kind == shown)
            .unwrap_or(0);
        let next = at.saturating_add_signed(delta).min(ForKind::ALL.len() - 1);
        if next != at {
            self.set_for(ForKind::ALL[next], window, cx);
        }
    }

    /// The keys a focused control takes; a key one took goes no further,
    /// and says so.
    fn key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.modified() {
            return false;
        }
        let key = keystroke.key.as_str();
        let press = matches!(key, "enter" | "space");
        let step = match key {
            "left" | "up" => -1,
            "right" | "down" => 1,
            _ => 0,
        };
        if self.stops.switch.is_focused(window) && matches!(key, "left" | "right") {
            self.arrow_mode(step, window, cx);
        } else if self.stops.to.is_focused(window) && press {
            self.toggle_destinations(window, cx);
        } else if self.stops.for_row.is_focused(window) && matches!(key, "left" | "right") {
            self.arrow_for(step, window, cx);
        } else if self.stops.stepper.is_focused(window) && matches!(key, "up" | "down") {
            // Up is more.
            self.step(if key == "up" { 1 } else { -1 }, window, cx);
        } else if self.stops.live.is_focused(window) && press {
            self.set_mode(Mode::Rules, window, cx);
            // The live line is gone: keep the keyboard on the switch.
            window.focus(&self.stops.switch, cx);
        } else {
            return false;
        }
        cx.stop_propagation();
        true
    }
}

impl Render for Dropdown {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let model = self.model.read(cx).clone();
        let ring = Ring::of(&self.stops, window);
        div()
            .id("tray-dropdown")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .role(gpui_kit::Role::Menu)
            .aria_label("PwrRoute routing")
            .tab_group()
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| this.dismiss(window, cx)))
            .on_action(cx.listener(|this, _: &TestLink, window, cx| {
                this.run(TrayCommand::TestLink, window, cx);
            }))
            .on_action(cx.listener(|this, _: &Rules, window, cx| {
                this.run(TrayCommand::Rules, window, cx);
            }))
            .on_action(cx.listener(|this, _: &Settings, window, cx| {
                this.run(TrayCommand::Settings, window, cx);
            }))
            .on_action(cx.listener(|_, _: &FocusNext, window, cx| {
                window.focus_next(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|_, _: &FocusPrev, window, cx| {
                window.focus_prev(cx);
                cx.notify();
            }))
            // The focused control's keys first (the stepper takes Up and
            // Down); what it leaves moves focus through the menu.
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.key(event, window, cx) {
                    return;
                }
                let key = event.keystroke.key.as_str();
                if !event.keystroke.modifiers.modified()
                    && matches!(key, "up" | "down" | "home" | "end")
                {
                    crate::accessibility::move_focus(&this.focus, key, window, cx);
                    cx.stop_propagation();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .font_family(theme::SANS)
            .child(card(
                &model,
                SystemTime::now(),
                self.hovered,
                self.unfolded,
                &self.stops,
                ring,
                cx,
            ))
            // Until the window catches up with a card that shrank, a click
            // below it is a click outside the dropdown.
            .child(
                div()
                    .flex_1()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| this.dismiss(window, cx)),
                    ),
            )
    }
}

/// Which stop wears the focus ring: the focused one, while the keyboard
/// drives (a click focuses too, without a ring).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ring {
    pub switch: bool,
    pub to: bool,
    pub for_row: bool,
    pub stepper: bool,
    pub live: bool,
}

impl Ring {
    fn of(stops: &Stops, window: &Window) -> Self {
        let keyboard = window.last_input_was_keyboard();
        let on = |handle: &FocusHandle| keyboard && handle.is_focused(window);
        Self {
            switch: on(&stops.switch),
            to: on(&stops.to),
            for_row: on(&stops.for_row),
            stepper: on(&stops.stepper),
            live: on(&stops.live),
        }
    }
}

/// `.foc`: the 2px accent ring 1px outside a control, drawn over it so it
/// takes no layout space. Its parent must be `relative`.
fn focus_ring(radius: f32) -> Div {
    div()
        .absolute()
        .top(px(-3.0))
        .left(px(-3.0))
        .right(px(-3.0))
        .bottom(px(-3.0))
        .border_2()
        .border_color(theme::accent())
        .rounded(px(radius + 3.0))
}

/// `.tray`: the card, as tall as what it shows.
fn card(
    model: &TrayModel,
    now: SystemTime,
    hovered: Option<&'static str>,
    unfolded: Unfolded,
    stops: &Stops,
    ring: Ring,
    cx: &mut Context<'_, Dropdown>,
) -> gpui_kit::Stateful<Div> {
    let mut recents = section().child(caption("Recent"));
    for (index, recent) in model.shown_recents().iter().enumerate() {
        recents = recents.child(recent_row(index, recent, now, cx));
    }
    if model.shown_recents().is_empty() {
        recents = recents.child(small_row(
            "tray-empty",
            "No links routed yet",
            None,
            TrayCommand::Activity,
            hovered,
            cx,
        ));
    }
    if model.shows_activity_row() {
        recents = recents.child(small_row(
            "tray-activity",
            "Open Activity",
            Some(model.today_label()),
            TrayCommand::Activity,
            hovered,
            cx,
        ));
    }

    let sync = model
        .sync
        .as_ref()
        .map(|row| section().child(sync_row(row, hovered, cx)));

    let commands = section()
        .child(menu_row(
            "tray-test",
            "Test a link\u{2026}",
            Some("\u{2318}T"),
            None,
            TrayCommand::TestLink,
            hovered,
            cx,
        ))
        .child(menu_row(
            "tray-rules",
            "Rules\u{2026}",
            Some("\u{2318}R"),
            None,
            TrayCommand::Rules,
            hovered,
            cx,
        ))
        .child(menu_row(
            "tray-settings",
            "Settings\u{2026}",
            Some("\u{2318},"),
            None,
            TrayCommand::Settings,
            hovered,
            cx,
        ))
        .when(model.shows_make_default_row(), |section| {
            section.child(menu_row(
                MAKE_DEFAULT_ID,
                "Make default browser\u{2026}",
                None,
                None,
                TrayCommand::MakeDefault,
                hovered,
                cx,
            ))
        });

    div()
        .id(CARD_ID)
        .debug_selector(|| CARD_ID.into())
        .w(px(WIDTH))
        // No taller than the window; when the window is capped by the
        // display, the sections between the header and Quit scroll.
        .max_h_full()
        .min_h_0()
        .flex_shrink(1.0)
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(theme::bg_popover())
        .border_1()
        .border_color(theme::border_default())
        .rounded(px(10.0))
        .child(header(model.chip()).flex_none())
        .child(
            div()
                .id(BODY_ID)
                .debug_selector(|| BODY_ID.into())
                .flex()
                .flex_col()
                .flex_shrink(1.0)
                .min_h_0()
                .overflow_y_scroll()
                .when(!model.held.is_empty(), |body| {
                    body.child(held_section(model, hovered, unfolded, cx).flex_none())
                })
                .child(recents.flex_none())
                .child(incoming_section(model, now, unfolded, stops, ring, cx).flex_none())
                .children(sync.map(|sync| sync.flex_none()))
                .child(commands.flex_none()),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .flex_col()
                .p(px(6.0))
                .child(menu_row(
                    QUIT_ID,
                    "Quit PwrRoute",
                    Some("\u{2318}Q"),
                    model.quit_warning(),
                    TrayCommand::Quit,
                    hovered,
                    cx,
                )),
        )
}

/// `.cap`: a section's caption.
fn caption(label: &str) -> Div {
    div()
        .h(px(CAPTION))
        .flex()
        .items_center()
        .gap(px(6.0))
        .px(px(6.0))
        .child(tracked(label, 9.5, 600.0, 0.12, theme::text_subtle()))
}

/// `.th`: the mark, the wordmark and the state chip.
fn header(chip: HeaderChip) -> Div {
    let wordmark = StyledText::new("PwrRoute").with_highlights([(
        3..8,
        HighlightStyle {
            color: Some(theme::accent()),
            ..Default::default()
        },
    )]);
    let (fill, border, color) = match chip {
        HeaderChip::Default => (
            theme::success_soft(),
            theme::transparent(),
            theme::success_text(),
        ),
        HeaderChip::Paused => (theme::warn_soft(), theme::transparent(), theme::warn_text()),
        HeaderChip::Sending(_) => (
            theme::accent_soft(),
            theme::transparent(),
            theme::accent_bright(),
        ),
        HeaderChip::Holding => (theme::info_soft(), theme::transparent(), theme::info_text()),
        HeaderChip::NotDefault => (
            theme::bg_elevated(),
            theme::border_subtle(),
            theme::text_muted(),
        ),
    };
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .px(px(12.0))
        .py(px(10.0))
        .border_b_1()
        .border_color(theme::border_faint())
        .child(icon("pwrroute/mark.svg", 16.0, theme::accent()))
        .child(
            div()
                .font_family(theme::SANS)
                .font_weight(theme::weight(700.0))
                .text_size(px(14.0))
                .line_height(px(14.0))
                .text_color(theme::text_primary())
                .whitespace_nowrap()
                .child(wordmark),
        )
        .child(
            div()
                .ml_auto()
                .flex()
                .flex_none()
                .items_center()
                .px(px(9.0))
                .py(px(4.0))
                .rounded_full()
                .border_1()
                .border_color(border)
                .bg(fill)
                .debug_selector(|| "tray-chip".into())
                .child(text(chip.label(), 10.0, 600.0, color)),
        )
}

/// The tone's colors: the lit segment's fill, border and text.
fn tone_colors(tone: Tone) -> (gpui_kit::Hsla, gpui_kit::Hsla, gpui_kit::Hsla) {
    match tone {
        Tone::Send => (
            theme::bg_row_active(),
            theme::accent_border(),
            theme::accent_bright(),
        ),
        Tone::Hold => (theme::info_soft(), info_border(), theme::info_text()),
        Tone::Pause => (theme::warn_soft(), warn_border(), theme::warn_text()),
    }
}

/// `rgba(159,200,255,0.42)`: a lit Hold segment's edge.
fn info_border() -> gpui_kit::Hsla {
    let mut color = theme::info_text();
    color.a = 0.42;
    color
}

/// `rgba(232,200,144,0.42)`: a lit Pause segment's edge.
fn warn_border() -> gpui_kit::Hsla {
    let mut color = theme::warn_text();
    color.a = 0.42;
    color
}

fn tone_of(mode: Mode) -> Option<Tone> {
    match mode {
        Mode::Rules => None,
        Mode::Send => Some(Tone::Send),
        Mode::Hold => Some(Tone::Hold),
        Mode::Pause => Some(Tone::Pause),
    }
}

/// Held (design: Incoming Links 2c, 2d): the links a hold kept, newest
/// first, each with Open (where the rules send it now) and "···" (Open in…,
/// Make a rule…, Copy link, in place under the row).
fn held_section(
    model: &TrayModel,
    hovered: Option<&'static str>,
    unfolded: Unfolded,
    cx: &mut Context<'_, Dropdown>,
) -> Div {
    let head = caption("Held")
        .child(mono(
            model.held.len().to_string(),
            10.0,
            600.0,
            theme::info_text(),
        ))
        .child(
            div()
                .id(OPEN_ALL_ID)
                .debug_selector(|| OPEN_ALL_ID.into())
                .ml_auto()
                .cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.unfolded.held = None;
                    this.unfolded.held_lanes = false;
                    super::open_all_held(cx);
                    this.fit(window, cx);
                }))
                .child(text("Open all", 10.5, 550.0, theme::accent_bright())),
        );
    let mut section = section().debug_selector(|| HELD_ID.into()).child(head);
    for (index, held) in model.shown_held().iter().enumerate() {
        let open = unfolded.held == Some(held.id);
        section = section.child(held_row(index, held, open, cx));
        if open && unfolded.held_lanes {
            section = section.child(held_lanes(model, held.id, cx));
        }
    }
    let more = model.held.len().saturating_sub(model.shown_held().len());
    if more > 0 {
        section = section.child(small_row(
            "tray-held-more",
            "More held links in Activity",
            Some(format!("+{more}")),
            TrayCommand::HeldLinks,
            hovered,
            cx,
        ));
    }
    section
}

/// `.hl`: a held link's host and path, where it came from and where the
/// rules would send it, with Open and "···"; and, while "···" is open, its
/// other actions.
fn held_row(
    index: usize,
    held: &HeldLink,
    open: bool,
    cx: &mut Context<'_, Dropdown>,
) -> impl IntoElement {
    let id = held.id;
    let url = StyledText::new(format!("{}{}", held.host, held.rest)).with_highlights([(
        0..held.host.len(),
        HighlightStyle {
            color: Some(theme::text_primary()),
            font_weight: Some(theme::weight(600.0)),
            ..Default::default()
        },
    )]);
    let detail = div()
        .h(px(HELD_DETAIL))
        .flex()
        .min_w_0()
        .items_center()
        .gap(px(5.0))
        .overflow_hidden()
        .child(
            one_line(
                held.detail(),
                theme::SANS,
                500.0,
                10.0,
                HELD_DETAIL,
                theme::text_muted(),
            )
            .flex_shrink_0(),
        )
        .when_some(held.rules.clone(), |line, rules| {
            line.child(badge(rules.badge.clone())).child(
                one_line(
                    rules.display(),
                    theme::SANS,
                    500.0,
                    10.0,
                    HELD_DETAIL,
                    theme::text_secondary(),
                )
                .min_w_0(),
            )
        });
    let actions = div()
        .flex()
        .flex_none()
        .gap(px(4.0))
        .child(
            small_button(held_id("open", index), "Open", true, false).on_click(cx.listener(
                move |this, _: &ClickEvent, window, cx| {
                    this.open_held(id, OpenAs::Rules, window, cx);
                },
            )),
        )
        .child(
            small_button(
                held_id("more", index),
                "\u{00b7}\u{00b7}\u{00b7}",
                false,
                open,
            )
            .w(px(22.0))
            .justify_center()
            .px_0()
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.toggle_held(id, window, cx);
            })),
        );
    div()
        .id(("tray-held", index))
        .flex()
        .flex_col()
        .p(px(6.0))
        .rounded(px(6.0))
        .when(open, |row| row.bg(theme::bg_hover_strong()))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(HELD_GAP))
                        .child(
                            div()
                                .h(px(HELD_URL))
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .font_family(theme::mono_family(500.0))
                                .text_size(px(11.0))
                                .line_height(px(HELD_URL))
                                .text_color(theme::text_secondary())
                                .debug_selector(move || held_id("url", index))
                                .child(url),
                        )
                        .child(detail),
                )
                .child(actions),
        )
        .when(open, |row| {
            row.child(
                div()
                    .pt(px(HELD_ACTIONS_GAP))
                    .flex()
                    .gap(px(4.0))
                    .child(
                        small_button(held_id("open-in", index), "Open in\u{2026}", false, false)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.toggle_held_lanes(window, cx);
                            })),
                    )
                    .child(
                        small_button(held_id("rule", index), "Make a rule\u{2026}", false, false)
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.dismiss(window, cx);
                                cx.defer(move |cx| super::make_rule_for(id, cx));
                            })),
                    )
                    .child(
                        small_button(held_id("copy", index), "Copy link", false, false).on_click(
                            cx.listener(move |this, _: &ClickEvent, window, cx| {
                                super::copy_held(id, cx);
                                this.toggle_held(id, window, cx);
                            }),
                        ),
                    ),
            )
        })
}

/// Open in… under a held row: the destination lanes, for a one-off choice.
fn held_lanes(model: &TrayModel, id: DispatchId, cx: &mut Context<'_, Dropdown>) -> Div {
    let lanes = model.lanes();
    let height = held_list_height(&lanes, &cx.text_system().clone());
    let list = div()
        .id(HELD_LANES_ID)
        .debug_selector(|| HELD_LANES_ID.into())
        .mx(px(LIST_MARGIN))
        .h(px(height))
        .flex_none()
        .p(px(LIST_PADDING))
        .border_1()
        .border_color(theme::border_default())
        .rounded(px(7.0))
        .bg(theme::bg_elevated())
        .overflow_y_scroll();
    let list = if lanes.is_empty() {
        list.child(no_browsers())
    } else {
        list.child(destinations::draw(
            HELD_LANES_ID,
            &lanes,
            Density::Compact,
            move |this: &mut Dropdown, choice, window, cx| {
                let destination = super::destination_of(&choice).target();
                this.open_held(id, OpenAs::In(destination), window, cx);
            },
            cx,
        ))
    };
    div().flex_none().child(list)
}

/// `.sbtn`: a 22px button; `.pri` filled, `on` lit.
fn small_button(
    id: String,
    label: &'static str,
    primary: bool,
    on: bool,
) -> gpui_kit::Stateful<Div> {
    let (fill, border, color) = if primary {
        (theme::accent(), theme::transparent(), theme::accent_on())
    } else if on {
        (
            theme::bg_row_active(),
            theme::accent_border(),
            theme::accent_bright(),
        )
    } else {
        (
            theme::bg_panel(),
            theme::border_default(),
            theme::text_primary(),
        )
    };
    let selector = id.clone();
    div()
        .id(gpui_kit::SharedString::from(id))
        .debug_selector(move || selector.clone())
        .h(px(22.0))
        .flex()
        .flex_none()
        .items_center()
        .px(px(8.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(border)
        .bg(fill)
        .cursor_pointer()
        .when(!primary, |button| {
            button.hover(|button| button.bg(theme::bg_hover()))
        })
        .child(text(label, 10.5, 550.0, color))
}

/// Incoming links (design: Incoming Links 2a-2f): the four-way switch,
/// then for the mode that is on its rows and the live line; at rest, the
/// hint.
fn incoming_section(
    model: &TrayModel,
    now: SystemTime,
    unfolded: Unfolded,
    stops: &Stops,
    ring: Ring,
    cx: &mut Context<'_, Dropdown>,
) -> Div {
    let mode = model.mode();
    let mut section = section()
        .child(caption("Incoming links"))
        .child(mode_switch(model, stops, ring, cx));
    if let Some(hint) = model.hint() {
        section = section.child(
            div()
                .debug_selector(|| HINT_ID.into())
                .px(px(6.0))
                .pb(px(HINT_BOTTOM))
                .child(runs(
                    &hint,
                    theme::text_muted(),
                    theme::text_secondary(),
                    HINT_LINE,
                )),
        );
    }
    if mode == Mode::Send {
        section = section.child(
            div()
                .flex()
                .flex_col()
                .mb(px(ROW_GAP))
                .child(destination_row(
                    model,
                    unfolded.destinations,
                    stops,
                    ring,
                    cx,
                ))
                .when(unfolded.destinations, |rows| {
                    rows.child(destination_list(model, cx))
                }),
        );
    }
    if mode.has_scope() {
        section = section.child(for_row(model, stops, ring, cx));
    }
    if let Some(live) = model.live(now) {
        section = section.child(live_line(&live, stops, ring, cx));
    }
    section
}

/// `.mseg4`: Rules, Send, Hold, Pause; the one on lit in its tone. One tab
/// stop, a radio group: Left and Right move it.
fn mode_switch(
    model: &TrayModel,
    stops: &Stops,
    ring: Ring,
    cx: &mut Context<'_, Dropdown>,
) -> impl IntoElement {
    let current = model.mode();
    let mut switch = div()
        .id(SWITCH_ID)
        .debug_selector(|| SWITCH_ID.into())
        .track_focus(&stops.switch)
        .role(gpui_kit::Role::RadioGroup)
        .aria_label("Incoming links")
        .relative()
        .mt(px(2.0))
        .mb(px(6.0))
        .mx(px(6.0))
        .h(px(SEG))
        .p(px(2.0))
        .flex()
        .gap(px(1.0))
        .rounded(px(7.0))
        .border_1()
        .border_color(theme::border_subtle())
        .bg(theme::bg_input());
    for (mode, id) in Mode::ALL.into_iter().zip(MODE_IDS) {
        let on = mode == current;
        let usable = mode != Mode::Hold || model.can_hold;
        let (fill, border, color) = match tone_of(mode) {
            _ if !on => (
                theme::transparent(),
                theme::transparent(),
                if usable {
                    theme::text_secondary()
                } else {
                    theme::text_subtle()
                },
            ),
            None => (
                theme::bg_hover_strong(),
                theme::border_default(),
                theme::text_primary(),
            ),
            Some(tone) => tone_colors(tone),
        };
        switch = switch.child(
            div()
                .id(id)
                .debug_selector(|| id.into())
                .flex_1()
                .h(px(SEG_BUTTON))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .border_1()
                .border_color(border)
                .bg(fill)
                .when(usable, |button| button.cursor_pointer())
                .when(usable && !on, |button| {
                    button.hover(|button| button.bg(theme::bg_hover()))
                })
                .map(|button| {
                    let label = if usable {
                        mode.label().to_owned()
                    } else {
                        format!("{}, off while Activity keeps nothing", mode.label())
                    };
                    crate::accessibility::selected(button, label, on)
                })
                .when(usable, |button| {
                    button.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.set_mode(mode, window, cx);
                    }))
                })
                .child(text(mode.label(), 11.0, 550.0, color)),
        );
    }
    switch.when(ring.switch, |switch| switch.child(focus_ring(7.0)))
}

/// A paragraph of runs, wrapped, the strong ones in `strong`.
fn runs(runs: &[Run], plain: gpui_kit::Hsla, strong: gpui_kit::Hsla, line: f32) -> Div {
    let text: String = super::model::runs_text(runs);
    let mut highlights = Vec::new();
    let mut at = 0;
    for run in runs {
        if run.strong {
            highlights.push((
                at..at + run.text.len(),
                HighlightStyle {
                    color: Some(strong),
                    ..Default::default()
                },
            ));
        }
        at += run.text.len();
    }
    div()
        .font_family(theme::SANS)
        .font_weight(theme::weight(400.0))
        .text_size(px(10.5))
        .line_height(px(line))
        .text_color(plain)
        .child(StyledText::new(text).with_highlights(highlights))
}

/// `.dest`: To, the destination, and the chevron that opens the lanes.
fn destination_row(
    model: &TrayModel,
    choosing: bool,
    stops: &Stops,
    ring: Ring,
    cx: &mut Context<'_, Dropdown>,
) -> impl IntoElement {
    let destination = &model.destination;
    div()
        .id(DESTINATION_ID)
        .debug_selector(|| DESTINATION_ID.into())
        .track_focus(&stops.to)
        .relative()
        .mx(px(6.0))
        .h(px(DEST_ROW))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(7.0))
        .px(px(8.0))
        .border_1()
        .border_color(theme::accent_border())
        .map(|row| {
            if choosing {
                row.rounded_t(px(7.0))
            } else {
                row.rounded(px(7.0))
            }
        })
        .bg(theme::bg_row_active())
        .cursor_pointer()
        .role(gpui_kit::Role::Button)
        .aria_label("Send to")
        .aria_expanded(choosing)
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.toggle_destinations(window, cx);
        }))
        .child(
            div()
                .w(px(ROW_LABEL))
                .child(tracked("To", 9.5, 600.0, 0.1, theme::text_subtle())),
        )
        .child(badge(destination.browser.badge.clone()))
        .when(destination.private.is_some(), |row| {
            row.child(icon("pwrroute/mask.svg", 13.0, theme::text_subtle()))
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(4.0))
                .overflow_hidden()
                .child(
                    one_line(
                        destination.browser.name.clone(),
                        theme::SANS,
                        500.0,
                        11.5,
                        12.0,
                        theme::text_primary(),
                    )
                    .flex_shrink_0(),
                )
                .children(destination.detail().map(|detail| {
                    one_line(
                        format!("\u{00b7} {detail}"),
                        theme::SANS,
                        500.0,
                        11.5,
                        12.0,
                        theme::accent_bright(),
                    )
                    .min_w_0()
                })),
        )
        .child(text(
            if choosing { "\u{2304}" } else { "\u{203a}" },
            12.0,
            500.0,
            theme::text_subtle(),
        ))
        .when(ring.to, |row| row.child(focus_ring(7.0)))
}

/// The destination list under the row: the rule editor's lanes
/// (`destinations::draw`), compact, the destination's pill lit. It
/// scrolls past [`LIST_MAX`].
fn destination_list(model: &TrayModel, cx: &mut Context<'_, Dropdown>) -> impl IntoElement {
    let lanes = model.lanes();
    let height = list_height(&lanes, &cx.text_system().clone());
    let list = div()
        .id(DESTINATIONS_ID)
        .debug_selector(|| DESTINATIONS_ID.into())
        .mx(px(LIST_MARGIN))
        .h(px(height))
        .flex_none()
        .p(px(LIST_PADDING))
        .border_1()
        .border_t_0()
        .border_color(theme::accent_border())
        .rounded_b(px(7.0))
        .bg(theme::bg_elevated())
        .overflow_y_scroll();
    if lanes.is_empty() {
        return list.child(no_browsers());
    }
    list.child(destinations::draw(
        LANES_ID,
        &lanes,
        Density::Compact,
        |this: &mut Dropdown, choice, window, cx| {
            this.choose(super::destination_of(&choice), window, cx);
        },
        cx,
    ))
}

fn no_browsers() -> Div {
    div()
        .h(px(NO_BROWSERS))
        .flex()
        .items_center()
        .px(px(6.0))
        .child(text("No browsers found", 11.0, 500.0, theme::text_muted()))
}

/// For: Links, Time or Until off, and the stepper for a count or a time.
/// One tab stop for the choice (Left and Right) and one for the stepper
/// (Up and Down).
fn for_row(
    model: &TrayModel,
    stops: &Stops,
    ring: Ring,
    cx: &mut Context<'_, Dropdown>,
) -> impl IntoElement {
    let shown = model.for_shown();
    let tone = tone_of(model.mode()).unwrap_or(Tone::Send);
    let (lit_fill, _, lit_text) = tone_colors(tone);
    let mut choices = div()
        .id(FOR_ROW_ID)
        .debug_selector(|| FOR_ROW_ID.into())
        .track_focus(&stops.for_row)
        .role(gpui_kit::Role::RadioGroup)
        .aria_label("For")
        .relative()
        .flex()
        .flex_none()
        .gap(px(1.0))
        .p(px(2.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(theme::border_subtle())
        .bg(theme::bg_input());
    for (kind, id) in ForKind::ALL.into_iter().zip(FOR_IDS) {
        let on = kind == shown;
        choices = choices.child(
            div()
                .id(id)
                .debug_selector(|| id.into())
                .px(px(6.0))
                .py(px(4.0))
                .rounded(px(4.0))
                .cursor_pointer()
                .when(on, |option| option.bg(lit_fill))
                .when(!on, |option| {
                    option.hover(|option| option.bg(theme::bg_hover()))
                })
                .map(|option| crate::accessibility::selected(option, kind.label(), on))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_for(kind, window, cx);
                }))
                .child(text(
                    kind.label(),
                    10.0,
                    550.0,
                    if on {
                        lit_text
                    } else {
                        theme::text_secondary()
                    },
                )),
        );
    }
    let choices = choices.when(ring.for_row, |choices| choices.child(focus_ring(6.0)));
    div()
        .mx(px(6.0))
        .mb(px(ROW_GAP))
        .h(px(DEST_ROW))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .px(px(8.0))
        .border_1()
        .border_color(theme::border_subtle())
        .rounded(px(7.0))
        .bg(theme::bg_panel())
        .child(
            div()
                .w(px(ROW_LABEL))
                .child(tracked("For", 9.5, 600.0, 0.1, theme::text_subtle())),
        )
        .child(choices)
        .child(div().flex_1())
        .children(model.stepper().map(|value| stepper(value, stops, ring, cx)))
}

/// `.stp`: − value +.
fn stepper(
    value: String,
    stops: &Stops,
    ring: Ring,
    cx: &mut Context<'_, Dropdown>,
) -> impl IntoElement {
    let step =
        |id: &'static str, glyph: &'static str, delta: i64, cx: &mut Context<'_, Dropdown>| {
            div()
                .id(id)
                .debug_selector(|| id.into())
                .w(px(20.0))
                .h(px(20.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .cursor_pointer()
                .hover(|button| button.bg(theme::bg_hover()))
                .role(gpui_kit::Role::Button)
                .aria_label(if delta > 0 { "Increase" } else { "Decrease" })
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.step(delta, window, cx);
                }))
                .child(text(glyph, 12.0, 600.0, theme::text_secondary()))
        };
    div()
        .id(STEPPER_ID)
        .debug_selector(|| STEPPER_ID.into())
        .track_focus(&stops.stepper)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .border_1()
        .border_color(theme::border_default())
        .rounded(px(6.0))
        .bg(theme::bg_panel())
        .child(step(STEP_DOWN_ID, "\u{2212}", -1, cx))
        .child(div().min_w(px(26.0)).flex().justify_center().child(mono(
            value,
            11.0,
            600.0,
            theme::text_primary(),
        )))
        .child(step(STEP_UP_ID, "+", 1, cx))
        .when(ring.stepper, |stepper| stepper.child(focus_ring(6.0)))
}

/// `.live`: a dot in the tone, what is on and what is left, and the way
/// back.
fn live_line(
    live: &Live,
    stops: &Stops,
    ring: Ring,
    cx: &mut Context<'_, Dropdown>,
) -> impl IntoElement {
    let (_, _, strong) = tone_colors(live.tone);
    let (wash, dot) = match live.tone {
        Tone::Send => (theme::accent_tint(), theme::accent()),
        Tone::Hold => {
            let mut wash = theme::info_soft();
            wash.a = 0.10;
            (wash, theme::info_text())
        }
        Tone::Pause => (theme::warn_soft(), theme::status_warning()),
    };
    let button_width = live_button_width(live.button);
    div()
        .debug_selector(|| LIVE_ID.into())
        .mx(px(6.0))
        .mb(px(LIVE_BOTTOM))
        .py(px(LIVE_PADDING))
        .pl(px(9.0))
        .pr(px(6.0))
        .flex()
        .items_center()
        .gap(px(8.0))
        .rounded(px(7.0))
        .bg(wash)
        .child(div().size(px(6.0)).flex_none().rounded_full().bg(dot))
        .child(div().flex_1().min_w_0().child(runs(
            &live.runs,
            theme::text_secondary(),
            strong,
            LIVE_LINE,
        )))
        .child(
            div()
                .id(LIVE_BUTTON_ID)
                .debug_selector(|| LIVE_BUTTON_ID.into())
                .track_focus(&stops.live)
                .relative()
                .w(px(button_width))
                .h(px(LIVE_BUTTON))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .border_1()
                .border_color(theme::border_default())
                .bg(theme::bg_panel())
                .cursor_pointer()
                .hover(|button| button.bg(theme::bg_hover()))
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.set_mode(Mode::Rules, window, cx);
                }))
                .child(text(live.button, 10.5, 550.0, theme::text_primary()))
                .when(ring.live, |button| button.child(focus_ring(6.0))),
        )
}

/// The live button's width: wide enough for its words.
fn live_button_width(label: &str) -> f32 {
    if label == "Resume" { 60.0 } else { 88.0 }
}

/// `.sec`: a group of rows with a hairline under it.
fn section() -> Div {
    div()
        .flex()
        .flex_col()
        .p(px(6.0))
        .border_b_1()
        .border_color(theme::border_faint())
}

/// `.rec`: a host, where it went, and how long ago.
fn recent_row(
    index: usize,
    recent: &Recent,
    now: SystemTime,
    cx: &mut Context<'_, Dropdown>,
) -> impl IntoElement {
    let host_line = div()
        .flex()
        .min_w_0()
        .gap(px(4.0))
        .overflow_hidden()
        .child(
            mono(recent.host.clone(), 11.5, 500.0, theme::text_primary())
                .line_height(px(REC_HOST))
                .flex_shrink_1()
                .min_w_0()
                .truncate(),
        )
        .when_some(recent.rewritten_from.clone(), |line, before| {
            line.child(
                mono(
                    format!("\u{2190} {before}"),
                    11.5,
                    500.0,
                    theme::text_subtle(),
                )
                .line_height(px(REC_HOST)),
            )
        });
    div()
        .id(("tray-recent", index))
        .flex()
        .items_center()
        .gap(px(10.0))
        .px(px(6.0))
        .py(px(5.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(|row| row.bg(theme::bg_hover()))
        .map(crate::accessibility::clickable)
        .aria_label(format!(
            "{}, {}, {}",
            recent.host,
            recent.detail(),
            recent.age(now)
        ))
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.run(TrayCommand::Activity, window, cx);
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(host_line)
                .child(
                    div()
                        .flex()
                        .min_w_0()
                        .items_center()
                        .gap(px(5.0))
                        .child(badge(recent.browser.badge.clone()))
                        // A long profile name ends in an ellipsis before
                        // the age column.
                        .child(
                            one_line(
                                recent.detail(),
                                theme::SANS,
                                500.0,
                                10.0,
                                REC_DETAIL,
                                theme::text_muted(),
                            )
                            .debug_selector(move || format!("tray-recent-detail-{index}")),
                        ),
                ),
        )
        .child(
            mono(recent.age(now), 10.0, 500.0, theme::text_subtle())
                .debug_selector(move || format!("tray-recent-age-{index}")),
        )
}

/// The muted 11px row under the recents ("Open Activity").
fn small_row(
    id: &'static str,
    label: &'static str,
    trailing: Option<String>,
    command: TrayCommand,
    hovered: Option<&'static str>,
    cx: &mut Context<'_, Dropdown>,
) -> impl IntoElement {
    row_frame(id, command, hovered, cx)
        .child(label_text(
            label,
            11.0,
            if hovered == Some(id) {
                theme::accent_bright()
            } else {
                theme::text_muted()
            },
        ))
        .when_some(trailing, |row, trailing| {
            row.child(
                div()
                    .ml_auto()
                    .child(mono(trailing, 10.0, 500.0, theme::text_subtle())),
            )
        })
}

/// `.mi-row`: a command, with its shortcut, and a muted note before it
/// ("2 held links will be lost" beside Quit).
fn menu_row(
    id: &'static str,
    label: &'static str,
    shortcut: Option<&'static str>,
    note: Option<String>,
    command: TrayCommand,
    hovered: Option<&'static str>,
    cx: &mut Context<'_, Dropdown>,
) -> impl IntoElement {
    let color = if hovered == Some(id) {
        theme::accent_bright()
    } else if command == TrayCommand::Quit {
        theme::text_secondary()
    } else {
        theme::text_primary()
    };
    // Every command row is the height of one with a key cap, so a row
    // without a shortcut (Make default browser…) adds up the same.
    row_frame(id, command, hovered, cx)
        .h(px(MENU_ROW))
        .child(label_text(label, 12.0, color))
        .child(div().flex_1())
        .when_some(note, |row, note| {
            row.child(
                one_line(note, theme::SANS, 500.0, 10.0, 12.0, theme::info_text())
                    .min_w_0()
                    .debug_selector(move || format!("{id}-note")),
            )
        })
        .when_some(shortcut, |row, shortcut| row.child(key_cap(shortcut)))
}

/// `.mi-row.sync`: how sync stands, a click to Settings › Cloud Sync.
fn sync_row(
    row: &SyncRow,
    hovered: Option<&'static str>,
    cx: &mut Context<'_, Dropdown>,
) -> impl IntoElement {
    let (icon_color, color) = match row.tone {
        SyncTone::Ok => (theme::status_ok(), theme::text_secondary()),
        SyncTone::Busy => (theme::accent_bright(), theme::accent_bright()),
        SyncTone::Warn => (theme::warn_text(), theme::warn_text()),
        SyncTone::Bad => (theme::danger_text(), theme::danger_text()),
    };
    let color = if hovered == Some(SYNC_ID) && row.tone == SyncTone::Ok {
        theme::accent_bright()
    } else {
        color
    };
    row_frame(SYNC_ID, TrayCommand::Sync, hovered, cx)
        .h(px(MENU_ROW))
        .child(icon("pwrroute/sync.svg", 12.0, icon_color))
        .child(text(row.title.clone(), 11.5, 500.0, color))
        .when_some(row.sub.clone(), |frame, sub| {
            frame.child(div().ml_auto().min_w_0().child(mono(
                sub,
                10.0,
                500.0,
                theme::text_subtle(),
            )))
        })
}

/// `.mi-row`'s box; `.hov` while the pointer is on it.
fn row_frame(
    id: &'static str,
    command: TrayCommand,
    hovered: Option<&'static str>,
    cx: &mut Context<'_, Dropdown>,
) -> gpui_kit::Stateful<Div> {
    div()
        .id(id)
        .role(gpui_kit::Role::MenuItem)
        .debug_selector(|| id.into())
        .flex()
        .items_center()
        .gap(px(8.0))
        .p(px(6.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .when(hovered == Some(id), |row| row.bg(theme::bg_row_active()))
        .on_hover(cx.listener(move |this, over: &bool, _, cx| {
            let hovered = if *over {
                Some(id)
            } else {
                this.hovered.filter(|hovered| *hovered != id)
            };
            if hovered != this.hovered {
                this.hovered = hovered;
                cx.notify();
            }
        }))
        .map(|e| crate::widgets::keyboard_control(e, false))
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.run(command, window, cx);
        }))
}

/// A row's label, 500 weight.
fn label_text(label: &'static str, size: f32, color: gpui_kit::Hsla) -> AnyElement {
    text(label, size, 500.0, color).into_any_element()
}

// The card's metrics, in CSS px, from the design's classes. `card_height`
// adds them up; a test holds it to the laid-out card.

/// `.th`: 10px padding around the 20px chip, and its hairline.
const HEADER: f32 = 10.0 + 20.0 + 10.0 + 1.0;
/// `.sec` padding, top and bottom.
const SECTION_PADDING: f32 = 12.0;
/// `.cap`: 9.5px text in 4px padding.
const CAPTION: f32 = 4.0 + 9.5 + 4.0;
/// `.rec .h` and `.rec .b`: 11.5px and 10px at line-height 1.2.
const REC_HOST: f32 = 13.8;
const REC_DETAIL: f32 = 12.0;
const RECENT: f32 = 5.0 + REC_HOST + 2.0 + REC_DETAIL + 5.0;
/// "Open Activity": 11px in 6px padding.
const SMALL_ROW: f32 = 6.0 + 11.0 + 6.0;
/// `.mi-row` with its 18px `.kbd`.
const MENU_ROW: f32 = 6.0 + 18.0 + 6.0;

/// `.hl`: the URL line, the line under it and the gap between, in 6px
/// padding; the actions row under it while "···" is open.
const HELD_URL: f32 = 14.0;
const HELD_DETAIL: f32 = 12.0;
const HELD_GAP: f32 = 3.0;
const HELD_ROW: f32 = 6.0 + HELD_URL + HELD_GAP + HELD_DETAIL + 6.0;
const HELD_ACTIONS_GAP: f32 = 4.0;
const HELD_ACTIONS: f32 = HELD_ACTIONS_GAP + 22.0;

/// The switch: its 24px buttons in 2px padding and a border.
const SEG_BUTTON: f32 = 24.0;
const SEG: f32 = SEG_BUTTON + 4.0 + 2.0;
/// The switch's margins above and below.
const SEG_MARGIN: f32 = 2.0 + 6.0;
/// `.hint`: 10.5px at 1.45, rounded, and 6px under it.
const HINT_LINE: f32 = 15.0;
const HINT_BOTTOM: f32 = 6.0;
/// The width the hint wraps in: the card inside its border, less the
/// section's and the hint's padding.
const HINT_WIDTH: f32 = WIDTH - 2.0 - 12.0 - 12.0;
/// The To and For rows, the gap under each, and their labels' width.
const DEST_ROW: f32 = 30.0;
const ROW_GAP: f32 = 6.0;
const ROW_LABEL: f32 = 22.0;
/// `.live`: 10.5px lines at 1.35, rounded, beside a 22px button, in 6px
/// padding, and 2px under it.
const LIVE_LINE: f32 = 14.0;
const LIVE_BUTTON: f32 = 22.0;
const LIVE_PADDING: f32 = 6.0;
const LIVE_BOTTOM: f32 = 2.0;
/// The width the live line wraps in, less its button: the section inside,
/// its margins and padding, the dot and two gaps.
const LIVE_WIDTH: f32 = WIDTH - 2.0 - 12.0 - 12.0 - 9.0 - 6.0 - 6.0 - 8.0 - 8.0;
/// The destination list: its margin in the section, its padding, and the
/// lanes' width inside its side borders.
const LIST_MARGIN: f32 = 6.0;
const LIST_PADDING: f32 = 4.0;
pub const LANES_WIDTH: f32 =
    WIDTH - 2.0 - SECTION_PADDING - 2.0 * LIST_MARGIN - 2.0 - 2.0 * LIST_PADDING;
/// "No browsers found", before a scan finds one.
const NO_BROWSERS: f32 = 26.0;
/// The list stops here and scrolls: three lanes of two rows and one of one.
pub const LIST_MAX: f32 = 260.0;

/// The destination list's height: the lanes in their padding and a
/// border (none on top), at most [`LIST_MAX`].
fn list_height(lanes: &[Lane], text_system: &std::sync::Arc<TextSystem>) -> f32 {
    (lanes_content(lanes, text_system) + 2.0 * LIST_PADDING + 1.0).min(LIST_MAX)
}

/// Open in…'s list under a held row: bordered all round.
fn held_list_height(lanes: &[Lane], text_system: &std::sync::Arc<TextSystem>) -> f32 {
    (lanes_content(lanes, text_system) + 2.0 * LIST_PADDING + 2.0).min(LIST_MAX)
}

fn lanes_content(lanes: &[Lane], text_system: &std::sync::Arc<TextSystem>) -> f32 {
    if lanes.is_empty() {
        NO_BROWSERS
    } else {
        lanes_height(lanes, text_system)
    }
}

/// The lanes' own height, wrapped as `destinations::draw` wraps them.
pub fn lanes_height(lanes: &[Lane], text_system: &std::sync::Arc<TextSystem>) -> f32 {
    destinations::height(lanes, Density::Compact, LANES_WIDTH, |label, weight| {
        let font = text_system.resolve_font(&Font {
            weight: theme::weight(weight),
            ..font(theme::SANS)
        });
        let width: f32 = label
            .chars()
            .map(|ch| {
                text_system
                    .advance(font, px(destinations::PILL_TEXT), ch)
                    .map_or(0.0, |size| theme::css(size.width))
            })
            .sum();
        // GPUI rounds a text box up to a whole pixel.
        width.ceil()
    })
}

/// How many lines `runs` wraps to at 10.5px in `width`.
fn lines(runs: &[Run], width: f32, text_system: &std::sync::Arc<TextSystem>) -> usize {
    let font = Font {
        weight: theme::weight(400.0),
        ..font(theme::SANS)
    };
    let text = super::model::runs_text(runs);
    let mut wrapper = text_system.line_wrapper(font, px(10.5));
    let fragments = [LineFragment::text(&text)];
    1 + wrapper.wrap_line(&fragments, px(width)).count()
}

/// The held section's height.
fn held_height(
    model: &TrayModel,
    unfolded: Unfolded,
    text_system: &std::sync::Arc<TextSystem>,
) -> f32 {
    if model.held.is_empty() {
        return 0.0;
    }
    let mut rows = 0.0;
    for held in model.shown_held() {
        rows += HELD_ROW;
        if unfolded.held == Some(held.id) {
            rows += HELD_ACTIONS;
            if unfolded.held_lanes {
                rows += held_list_height(&model.lanes(), text_system);
            }
        }
    }
    if model.held.len() > model.shown_held().len() {
        rows += SMALL_ROW;
    }
    SECTION_PADDING + CAPTION + rows + 1.0
}

/// The Incoming links section's height.
fn incoming_height(
    model: &TrayModel,
    unfolded: Unfolded,
    now: SystemTime,
    text_system: &std::sync::Arc<TextSystem>,
) -> f32 {
    let mode = model.mode();
    let mut height = SECTION_PADDING + CAPTION + SEG + SEG_MARGIN + 1.0;
    if let Some(hint) = model.hint() {
        height += HINT_LINE * lines(&hint, HINT_WIDTH, text_system) as f32 + HINT_BOTTOM;
    }
    if mode == Mode::Send {
        height += DEST_ROW + ROW_GAP;
        if unfolded.destinations {
            height += list_height(&model.lanes(), text_system);
        }
    }
    if mode.has_scope() {
        height += DEST_ROW + ROW_GAP;
    }
    if let Some(live) = model.live(now) {
        let width = LIVE_WIDTH - live_button_width(live.button);
        let text = LIVE_LINE * lines(&live.runs, width, text_system) as f32;
        height += 2.0 * LIVE_PADDING + text.max(LIVE_BUTTON) + LIVE_BOTTOM;
    }
    height
}

/// The card's height for `model` as it is now, with what is unfolded in
/// it.
pub fn card_height(
    model: &TrayModel,
    unfolded: Unfolded,
    text_system: &std::sync::Arc<TextSystem>,
) -> Pixels {
    card_height_at(model, unfolded, SystemTime::now(), text_system)
}

fn card_height_at(
    model: &TrayModel,
    unfolded: Unfolded,
    now: SystemTime,
    text_system: &std::sync::Arc<TextSystem>,
) -> Pixels {
    let recents_shown = model.shown_recents().len();
    let small_rows = usize::from(recents_shown == 0) + usize::from(model.shows_activity_row());
    let recents = SECTION_PADDING
        + CAPTION
        + RECENT * recents_shown as f32
        + SMALL_ROW * small_rows as f32
        + 1.0;
    let commands = SECTION_PADDING
        + MENU_ROW * (3.0 + f32::from(u8::from(model.shows_make_default_row())))
        + 1.0;
    // Sync's row, while rules sync with a folder.
    let sync = if model.sync.is_some() {
        SECTION_PADDING + MENU_ROW + 1.0
    } else {
        0.0
    };
    let quit = SECTION_PADDING + MENU_ROW;
    px(2.0
        + HEADER
        + held_height(model, unfolded, text_system)
        + recents
        + incoming_height(model, unfolded, now, text_system)
        + sync
        + commands
        + quit)
}

/// The window's height: the card's, rounded up to a whole point, and no
/// taller than `max_height`.
pub fn fitted_height(
    model: &TrayModel,
    unfolded: Unfolded,
    text_system: &std::sync::Arc<TextSystem>,
    max_height: Pixels,
) -> Pixels {
    // Measured, so already zoomed: rounded in screen points.
    gpui_kit::px(card_height(model, unfolded, text_system).as_f32().ceil()).min(max_height)
}
