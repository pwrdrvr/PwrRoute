//! The macOS menu bar dropdown: `design/PwrRoute Tray and First Run.dc.html`
//! options 1a (routing) and 1b (paused), with the Send links section of
//! `design/PwrRoute Send Links.dc.html` 1a-1f above Pause routing.
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

use std::rc::Rc;
use std::time::SystemTime;

use crate::theme::px;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, DisplayId, Div, Entity,
    FocusHandle, Font, HighlightStyle, InteractiveElement as _, IntoElement, KeyBinding,
    LineFragment, MouseButton, ParentElement as _, Pixels, Point, Render,
    StatefulInteractiveElement as _, Styled as _, StyledText, Subscription, TextSystem, Window,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions, div, font,
    size,
};

use super::TrayCommand;
use super::model::{HeaderChip, Recent, SendDestination, SendMode, TrayModel};
use crate::destinations::{self, Density, Lane};
use crate::theme::{self, icon, mono, text, tracked};
use crate::widgets::{badge, key_cap, one_line, switch};
use pwrroute_router::SendScope;

/// The element ids tests and the inspector find things by.
pub const CARD_ID: &str = "tray-card";
pub const PAUSE_ID: &str = "tray-pause";
pub const SEND_RULES_ID: &str = "tray-send-rules";
pub const SEND_NEXT_ID: &str = "tray-send-next";
pub const SEND_ALL_ID: &str = "tray-send-all";
pub const DESTINATION_ID: &str = "tray-destination";
pub const DESTINATIONS_ID: &str = "tray-destinations";
/// The lanes' selectors: `tray-lanes-lane-{l}`, `tray-lanes-{l}-{p}`.
pub const LANES_ID: &str = "tray-lanes";
pub const STEP_DOWN_ID: &str = "tray-step-down";
pub const STEP_UP_ID: &str = "tray-step-up";
pub const SEND_DETAIL_ID: &str = "tray-send-detail";
pub const UNTIL_OFF_ID: &str = "tray-until-off";
pub const FOR_HOUR_ID: &str = "tray-for-hour";
pub const MAKE_DEFAULT_ID: &str = "tray-make-default";
pub const QUIT_ID: &str = "tray-quit";
/// The sections between the header and Quit, which scroll on a display too
/// short for the card.
pub const BODY_ID: &str = "tray-body";

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
    let height = fitted_height(model.read(cx), false, &cx.text_system().clone(), max_height);
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

pub struct Dropdown {
    model: Entity<TrayModel>,
    pub focus: FocusHandle,
    /// The command row under the pointer. GPUI resolves text color before it
    /// knows what is hovered, so the `.hov` label color comes from here.
    hovered: Option<&'static str>,
    /// The destination list is open (design: Send Links 1b).
    pub choosing: bool,
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
        let height = fitted_height(model.read(cx), false, &cx.text_system().clone(), max_height);
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
            hovered: None,
            choosing: false,
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
        let want = fitted_height(
            self.model.read(cx),
            self.choosing,
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
        if command == TrayCommand::TogglePause {
            super::toggle_pause(&self.model, cx);
            return;
        }
        super::dropdown_closed(cx);
        window.remove_window();
        cx.defer(move |cx| super::run(command, cx));
    }

    /// A Send links segment. The dropdown stays open, so the user sees the
    /// switch take.
    fn send(&mut self, mode: SendMode, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.choosing = false;
        super::set_send_mode(mode, cx);
        self.fit(window, cx);
    }

    fn toggle_choosing(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.choosing = !self.choosing;
        self.fit(window, cx);
    }

    fn choose(
        &mut self,
        destination: SendDestination,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.choosing = false;
        super::choose_destination(destination, cx);
        self.fit(window, cx);
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.choosing {
            self.choosing = false;
            window.focus(&self.focus, cx);
            self.fit(window, cx);
            return;
        }
        super::dropdown_closed(cx);
        window.remove_window();
    }
}

impl Render for Dropdown {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let model = self.model.read(cx).clone();
        div()
            .id("tray-dropdown")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .role(gpui_kit::Role::Menu)
            .aria_label("PwrRoute routing")
            .tab_group()
            .on_key_down(cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                if matches!(key, "up" | "down" | "home" | "end") {
                    crate::accessibility::move_focus(&this.focus, key, window, cx);
                    cx.stop_propagation();
                }
            }))
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
            .size_full()
            .flex()
            .flex_col()
            .font_family(theme::SANS)
            .child(card(
                &model,
                SystemTime::now(),
                self.hovered,
                self.choosing,
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

/// `.tray`: the card, as tall as what it shows.
fn card(
    model: &TrayModel,
    now: SystemTime,
    hovered: Option<&'static str>,
    choosing: bool,
    cx: &mut Context<'_, Dropdown>,
) -> gpui_kit::Stateful<Div> {
    let paused = model.is_paused();
    let mut recents = section().child(div().px(px(6.0)).py(px(4.0)).child(tracked(
        "Recent",
        9.5,
        600.0,
        0.12,
        theme::text_subtle(),
    )));
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

    let pause = section().child(
        div()
            .id(PAUSE_ID)
            .debug_selector(|| PAUSE_ID.into())
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(6.0))
            .py(px(7.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .map(crate::accessibility::clickable)
            .role(gpui_kit::Role::Switch)
            .aria_label("Pause routing")
            .aria_toggled(gpui_kit::Toggled::from(paused))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.run(TrayCommand::TogglePause, window, cx);
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .child(
                        text("Pause routing", 12.0, 500.0, theme::text_primary())
                            .line_height(px(PAUSE_TITLE)),
                    )
                    .child(
                        div()
                            .font_family(theme::SANS)
                            .font_weight(theme::weight(400.0))
                            .text_size(px(10.0))
                            .line_height(px(PAUSE_LINE))
                            .text_color(theme::text_muted())
                            .child(model.pause_detail()),
                    ),
            )
            .child(switch(paused)),
    );
    let commands = section()
        .child(menu_row(
            "tray-test",
            "Test a link\u{2026}",
            Some("\u{2318}T"),
            TrayCommand::TestLink,
            hovered,
            cx,
        ))
        .child(menu_row(
            "tray-rules",
            "Rules\u{2026}",
            Some("\u{2318}R"),
            TrayCommand::Rules,
            hovered,
            cx,
        ))
        .child(menu_row(
            "tray-settings",
            "Settings\u{2026}",
            Some("\u{2318},"),
            TrayCommand::Settings,
            hovered,
            cx,
        ))
        .when(model.shows_make_default_row(), |section| {
            section.child(menu_row(
                MAKE_DEFAULT_ID,
                "Make default browser\u{2026}",
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
                .child(recents.flex_none())
                .child(send_section(model, now, choosing, cx).flex_none())
                .child(pause.flex_none())
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
                    TrayCommand::Quit,
                    hovered,
                    cx,
                )),
        )
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

/// Send links (design: Send Links 1a-1f): the three-way switch, the
/// destination row (and its list, while choosing), and the line for the
/// mode that is on: the stepper for Next link, the length for All links.
fn send_section(
    model: &TrayModel,
    now: SystemTime,
    choosing: bool,
    cx: &mut Context<'_, Dropdown>,
) -> Div {
    let mode = model.send_mode();
    let segment = |id: &'static str,
                   label: &'static str,
                   target: SendMode,
                   cx: &mut Context<'_, Dropdown>| {
        let on = mode == Some(target);
        div()
            .id(id)
            .debug_selector(|| id.into())
            .flex_1()
            .h(px(SEG_BUTTON))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.0))
            .cursor_pointer()
            .map(|button| {
                if on {
                    button
                        .bg(theme::bg_row_active())
                        .border_1()
                        .border_color(theme::accent_border())
                } else {
                    button.hover(|button| button.bg(theme::bg_hover()))
                }
            })
            .map(crate::accessibility::clickable)
            .map(|element| crate::accessibility::radio(element, on))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.send(target, window, cx);
            }))
            .child(text(
                label,
                11.0,
                550.0,
                if on {
                    theme::accent_bright()
                } else {
                    theme::text_secondary()
                },
            ))
    };
    let switch_row = div()
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
        .bg(theme::bg_input())
        .child(segment(SEND_RULES_ID, "Rules", SendMode::Rules, cx))
        .child(segment(SEND_NEXT_ID, "Next link", SendMode::Next, cx))
        .child(segment(SEND_ALL_ID, "All links", SendMode::All, cx));

    let live = model.sending().is_some();
    let destination = &model.destination;
    let destination_row = div()
        .id(DESTINATION_ID)
        .debug_selector(|| DESTINATION_ID.into())
        .mx(px(6.0))
        .h(px(DEST_ROW))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(7.0))
        .px(px(8.0))
        .border_1()
        .border_color(if live || choosing {
            theme::accent_border()
        } else {
            theme::border_subtle()
        })
        .map(|row| {
            if choosing {
                row.rounded_t(px(7.0))
            } else {
                row.rounded(px(7.0))
            }
        })
        .bg(if live {
            theme::bg_row_active()
        } else {
            theme::bg_panel()
        })
        .cursor_pointer()
        .hover(|row| row.border_color(theme::accent_border()))
        .map(crate::accessibility::clickable)
        .aria_expanded(choosing)
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.toggle_choosing(window, cx);
        }))
        .child(
            div()
                .w(px(18.0))
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
                        if live {
                            theme::text_primary()
                        } else {
                            theme::text_secondary()
                        },
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
                        if live {
                            theme::accent_bright()
                        } else {
                            theme::text_muted()
                        },
                    )
                    .min_w_0()
                })),
        )
        .child(text(
            if choosing { "\u{2304}" } else { "\u{203a}" },
            12.0,
            500.0,
            theme::text_subtle(),
        ));

    let mut body = section()
        .child(div().px(px(6.0)).py(px(4.0)).child(tracked(
            "Send links",
            9.5,
            600.0,
            0.12,
            theme::text_subtle(),
        )))
        .child(switch_row)
        .child(destination_row);
    if choosing {
        body = body.child(destination_list(model, cx));
    }
    match model.sending() {
        Some(SendScope::Next { .. }) => body = body.child(next_line(model, now, cx)),
        Some(SendScope::All { .. }) => body = body.child(all_line(model, now, cx)),
        None => {}
    }
    body
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
        return list.child(
            div()
                .h(px(NO_BROWSERS))
                .flex()
                .items_center()
                .px(px(6.0))
                .child(text("No browsers found", 11.0, 500.0, theme::text_muted())),
        );
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

/// Next link's line: the stepper, and what happens after (1c, 1d).
fn next_line(model: &TrayModel, now: SystemTime, cx: &mut Context<'_, Dropdown>) -> Div {
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
                .map(crate::accessibility::clickable)
                .aria_label(if delta > 0 {
                    "Increase link count"
                } else {
                    "Decrease link count"
                })
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    super::step_next(delta, cx);
                }))
                .child(text(glyph, 12.0, 600.0, theme::text_secondary()))
        };
    let stepper = div()
        .flex()
        .flex_none()
        .items_center()
        .border_1()
        .border_color(theme::border_default())
        .rounded(px(6.0))
        .bg(theme::bg_panel())
        .child(step(STEP_DOWN_ID, "\u{2212}", -1, cx))
        .child(div().min_w(px(18.0)).flex().justify_center().child(mono(
            model.next_shown().to_string(),
            11.0,
            600.0,
            theme::text_primary(),
        )))
        .child(step(STEP_UP_ID, "+", 1, cx));
    send_line()
        .child(stepper)
        .child(detail_text(model.send_detail(now).unwrap_or_default()))
}

/// All links' line: until switched back or for an hour, and the time
/// left (1e).
fn all_line(model: &TrayModel, now: SystemTime, cx: &mut Context<'_, Dropdown>) -> Div {
    let option =
        |id: &'static str, label: &'static str, hour: bool, cx: &mut Context<'_, Dropdown>| {
            let on = model.all_for_hour == hour;
            div()
                .id(id)
                .debug_selector(|| id.into())
                .px(px(7.0))
                .py(px(4.0))
                .rounded(px(4.0))
                .cursor_pointer()
                .when(on, |option| option.bg(theme::bg_row_active()))
                .when(!on, |option| {
                    option.hover(|option| option.bg(theme::bg_hover()))
                })
                .map(crate::accessibility::clickable)
                .map(|element| crate::accessibility::radio(element, on))
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    super::set_all_for_hour(hour, cx);
                }))
                .child(text(
                    label,
                    10.0,
                    550.0,
                    if on {
                        theme::accent_bright()
                    } else {
                        theme::text_secondary()
                    },
                ))
        };
    send_line()
        .child(
            div()
                .flex()
                .flex_none()
                .gap(px(1.0))
                .p(px(2.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(theme::border_subtle())
                .bg(theme::bg_input())
                .child(option(UNTIL_OFF_ID, "Until switched back", false, cx))
                .child(option(FOR_HOUR_ID, "For 1 hour", true, cx)),
        )
        .child(
            div()
                .flex_1()
                .flex()
                .justify_end()
                .child(detail_text(model.send_detail(now).unwrap_or_default())),
        )
}

/// `.sline`: the row under the destination.
fn send_line() -> Div {
    div()
        .mt(px(4.0))
        .h(px(SEND_LINE))
        .flex()
        .items_center()
        .gap(px(8.0))
        .pl(px(12.0))
        .pr(px(6.0))
}

fn detail_text(detail: String) -> impl IntoElement {
    div()
        .debug_selector(|| SEND_DETAIL_ID.into())
        .min_w_0()
        .child(one_line(
            detail,
            theme::SANS,
            400.0,
            10.5,
            14.0,
            theme::text_muted(),
        ))
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

/// `.mi-row`: a command, with its shortcut.
fn menu_row(
    id: &'static str,
    label: &'static str,
    shortcut: Option<&'static str>,
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
        .when_some(shortcut, |row, shortcut| {
            row.child(div().ml_auto().child(key_cap(shortcut)))
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
/// `.pause .pt b` and `.pause .pt span`: 12px at 1.2, 10px at 1.4.
const PAUSE_TITLE: f32 = 14.4;
const PAUSE_LINE: f32 = 14.0;
/// The width the pause line wraps in: the card inside its border, less the
/// section's and the row's padding, the gap and the 30px switch.
const PAUSE_TEXT_WIDTH: f32 = WIDTH - 2.0 - 12.0 - 12.0 - 10.0 - 30.0;

/// Send links' switch: its 24px buttons in 2px padding and a border.
const SEG_BUTTON: f32 = 24.0;
const SEG: f32 = SEG_BUTTON + 4.0 + 2.0;
/// The switch's margins above and below.
const SEG_MARGIN: f32 = 2.0 + 6.0;
/// The destination row.
const DEST_ROW: f32 = 30.0;
/// The stepper's or the length's line, and the gap above it.
const SEND_LINE: f32 = 26.0;
const SEND_LINE_GAP: f32 = 4.0;
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
    let content = if lanes.is_empty() {
        NO_BROWSERS
    } else {
        lanes_height(lanes, text_system)
    };
    (content + 2.0 * LIST_PADDING + 1.0).min(LIST_MAX)
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

/// What the card is made of, for its height.
#[derive(Clone, Copy, Debug)]
struct Rows {
    recents: usize,
    /// "Open Activity", or the empty-state row.
    small_rows: usize,
    pause_lines: usize,
    /// "Make default browser…" under the commands.
    make_default: bool,
    /// The destination list's height, while it is open.
    list: Option<f32>,
    /// Next link or All links is on: its line shows.
    send_line: bool,
}

impl Rows {
    fn of(model: &TrayModel, choosing: bool, text_system: &std::sync::Arc<TextSystem>) -> Self {
        let recents = model.shown_recents().len();
        Self {
            recents,
            small_rows: usize::from(recents == 0) + usize::from(model.shows_activity_row()),
            pause_lines: pause_lines(&model.pause_detail(), text_system),
            make_default: model.shows_make_default_row(),
            list: choosing.then(|| list_height(&model.lanes(), text_system)),
            send_line: model.sending().is_some(),
        }
    }

    fn height(self) -> Pixels {
        let recents = SECTION_PADDING
            + CAPTION
            + RECENT * self.recents as f32
            + SMALL_ROW * self.small_rows as f32
            + 1.0;
        let pause =
            SECTION_PADDING + 14.0 + PAUSE_TITLE + 3.0 + PAUSE_LINE * self.pause_lines as f32 + 1.0;
        let commands =
            SECTION_PADDING + MENU_ROW * (3.0 + f32::from(u8::from(self.make_default))) + 1.0;
        let send = SECTION_PADDING
            + CAPTION
            + SEG
            + SEG_MARGIN
            + DEST_ROW
            + self.list.unwrap_or(0.0)
            + if self.send_line {
                SEND_LINE_GAP + SEND_LINE
            } else {
                0.0
            }
            + 1.0;
        let quit = SECTION_PADDING + MENU_ROW;
        px(2.0 + HEADER + recents + send + pause + commands + quit)
    }
}

/// How many lines the pause description wraps to.
fn pause_lines(detail: &str, text_system: &std::sync::Arc<TextSystem>) -> usize {
    let font = Font {
        weight: theme::weight(400.0),
        ..font(theme::SANS)
    };
    let mut wrapper = text_system.line_wrapper(font, px(10.0));
    let fragments = [LineFragment::text(detail)];
    1 + wrapper.wrap_line(&fragments, px(PAUSE_TEXT_WIDTH)).count()
}

/// The card's height for `model` as it is now, with the destination list
/// open or not.
pub fn card_height(
    model: &TrayModel,
    choosing: bool,
    text_system: &std::sync::Arc<TextSystem>,
) -> Pixels {
    Rows::of(model, choosing, text_system).height()
}

/// The window's height: the card's, rounded up to a whole point, and no
/// taller than `max_height`.
pub fn fitted_height(
    model: &TrayModel,
    choosing: bool,
    text_system: &std::sync::Arc<TextSystem>,
    max_height: Pixels,
) -> Pixels {
    // Measured, so already zoomed: rounded in screen points.
    gpui_kit::px(card_height(model, choosing, text_system).as_f32().ceil()).min(max_height)
}
