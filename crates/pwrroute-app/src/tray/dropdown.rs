//! The macOS menu bar dropdown: `design/PwrRoute Tray and First Run.dc.html`
//! options 1a (routing) and 1b (paused).
//!
//! A 300px GPUI `PopUp` window (a non-activating `NSPanel`) opened under the
//! menu bar item. It takes key focus without activating the app, so the
//! frontmost app keeps its menu bar, and it closes when it loses focus: a
//! click anywhere else, Esc, a command, or the item again.
//!
//! GPUI cannot move a window, and macOS keeps a window's bottom edge when it
//! resizes, so the window opens at the tallest the dropdown can get (see
//! [`window_height`]) with a transparent background, and the card is drawn at
//! its top at the height of what it shows now. Pausing, or a new recent link,
//! changes the card and never the window.

use std::time::SystemTime;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, DisplayId, Div, Entity,
    FocusHandle, Font, HighlightStyle, InteractiveElement as _, IntoElement, KeyBinding,
    LineFragment, MouseButton, ParentElement as _, Pixels, Point, Render,
    StatefulInteractiveElement as _, Styled as _, StyledText, Subscription, TextSystem, Window,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions, div, font,
    px, size,
};

use super::TrayCommand;
use super::model::{HeaderChip, Recent, TrayModel};
use crate::theme::{self, icon, mono, text, tracked};
use crate::widgets::{badge_text, key_cap, switch};

/// The element ids tests and the inspector find things by.
pub const CARD_ID: &str = "tray-card";
pub const PAUSE_ID: &str = "tray-pause";

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

/// Open the dropdown with its top-left at `origin` on `display`.
#[cfg_attr(
    not(target_os = "macos"),
    allow(dead_code, reason = "only macOS has the dropdown")
)]
pub fn open(
    model: Entity<TrayModel>,
    origin: Point<Pixels>,
    display: Option<DisplayId>,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<Dropdown>> {
    let height = window_height(model.read(cx), &cx.text_system().clone());
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
        |window, cx| cx.new(|cx| Dropdown::new(model, window, cx)),
    )?;
    window.update(cx, |this, window, cx| window.focus(&this.focus, cx))?;
    Ok(window)
}

pub struct Dropdown {
    model: Entity<TrayModel>,
    pub focus: FocusHandle,
    /// The command row under the pointer. GPUI resolves text color before it
    /// knows what is hovered, so the `.hov` label color comes from here.
    hovered: Option<&'static str>,
    _subscriptions: Vec<Subscription>,
}

impl Dropdown {
    pub fn new(model: Entity<TrayModel>, window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
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
            _subscriptions: subscriptions,
        }
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

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
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
            .child(card(&model, SystemTime::now(), self.hovered, cx))
            // The window is taller than the card; a click below it is a
            // click outside the dropdown.
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

    div()
        .id(CARD_ID)
        .debug_selector(|| CARD_ID.into())
        .flex_none()
        .w(px(WIDTH))
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(theme::bg_popover())
        .border_1()
        .border_color(theme::border_default())
        .rounded(px(10.0))
        .child(header(model.chip()))
        .child(recents)
        .child(
            section().child(
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
            ),
        )
        .child(
            section()
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
                )),
        )
        .child(div().flex().flex_col().p(px(6.0)).child(menu_row(
            "tray-quit",
            "Quit PwrRoute",
            Some("\u{2318}Q"),
            TrayCommand::Quit,
            hovered,
            cx,
        )))
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
                .child(text(chip.label(), 10.0, 600.0, color)),
        )
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
                        .items_center()
                        .gap(px(5.0))
                        .child(badge_text(recent.browser.badge.clone().into()))
                        .child(
                            text(recent.detail(), 10.0, 500.0, theme::text_muted())
                                .line_height(px(REC_DETAIL)),
                        ),
                ),
        )
        .child(mono(recent.age(now), 10.0, 500.0, theme::text_subtle()))
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
    row_frame(id, command, hovered, cx)
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

/// What the card is made of, for its height.
#[derive(Clone, Copy, Debug)]
struct Rows {
    recents: usize,
    /// "Open Activity", or the empty-state row.
    small_rows: usize,
    pause_lines: usize,
}

impl Rows {
    #[cfg(test)]
    fn of(model: &TrayModel, text_system: &std::sync::Arc<TextSystem>) -> Self {
        let recents = model.shown_recents().len();
        Self {
            recents,
            small_rows: usize::from(recents == 0) + usize::from(model.shows_activity_row()),
            pause_lines: pause_lines(&model.pause_detail(), text_system),
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
        let commands = SECTION_PADDING + MENU_ROW * 3.0 + 1.0;
        let quit = SECTION_PADDING + MENU_ROW;
        px(2.0 + HEADER + recents + pause + commands + quit)
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

/// The card's height for `model` as it is now.
#[cfg(test)]
pub fn card_height(model: &TrayModel, text_system: &std::sync::Arc<TextSystem>) -> Pixels {
    Rows::of(model, text_system).height()
}

/// The tallest the card can get for this pause target: every recent row,
/// whichever pause line wraps more, and the activity row. A few px spare
/// absorb GPUI's rounding.
pub fn window_height(model: &TrayModel, text_system: &std::sync::Arc<TextSystem>) -> Pixels {
    let mut other = model.clone();
    other.set_paused(!model.is_paused());
    let rows = Rows {
        recents: super::model::RECENT_LIMIT,
        small_rows: 1,
        pause_lines: pause_lines(&model.pause_detail(), text_system)
            .max(pause_lines(&other.pause_detail(), text_system)),
    };
    rows.height() + px(4.0)
}
