//! The Settings design's controls and layout pieces, each named for its class
//! in `design/pwrroute.css` or the Settings canvas's own `<style>`.
//!
//! Interactive controls are gpui-kit's unstyled primitives (`base::Button`,
//! `Switch`, `Checkbox`, `Radio`), which own focus, keyboard activation and
//! accessibility; these functions only dress them in the tokens.

use gpui_kit::base::{Button, Checkbox, Radio, Switch};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Div, ElementId, Hsla, InteractiveElement as _, ParentElement as _, SharedString,
    Styled as _, div, px,
};

use super::model::Status;
use super::style::{self, ch, para};
use crate::theme::{self, mono, text, tracked};
use crate::widgets::tag_frame;

/// `.btn` and its modifiers.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Default,
    Primary,
    /// `.quiet`: no fill, no border.
    Quiet,
    /// `.quiet.danger`.
    Danger,
}

/// `.btn`, or `.btn.sm` when `small`. The caller adds `on_click`.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    kind: Kind,
    small: bool,
) -> Button {
    button_when(id, label, kind, small, true)
}

/// [`button`], or an inert one at 60% when not `enabled`: no pointer, no
/// hover, no activation.
pub fn button_when(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    kind: Kind,
    small: bool,
    enabled: bool,
) -> Button {
    let foreground = match kind {
        Kind::Default => theme::text_primary(),
        Kind::Primary => theme::accent_on(),
        Kind::Quiet => theme::text_secondary(),
        Kind::Danger => theme::danger_text(),
    };
    let (fill, border, hover) = match kind {
        Kind::Default => (
            theme::bg_elevated(),
            theme::border_default(),
            theme::bg_hover(),
        ),
        Kind::Primary => (
            theme::accent(),
            theme::transparent(),
            theme::accent_bright(),
        ),
        Kind::Quiet | Kind::Danger => (
            theme::transparent(),
            theme::transparent(),
            theme::bg_hover(),
        ),
    };
    let size = if small { 11.0 } else { 11.5 };
    Button::new(id)
        .flex_none()
        .gap(px(6.0))
        .map(|button| {
            if small {
                button.py(px(5.0)).px(px(9.0))
            } else {
                button.py(px(7.0)).px(px(12.0))
            }
        })
        .border_1()
        .border_color(border)
        .rounded(px(7.0))
        .bg(fill)
        .disabled(!enabled)
        .map(|button| {
            if enabled {
                button.cursor_pointer().hover(move |style| style.bg(hover))
            } else {
                button.opacity(0.6)
            }
        })
        .child(text(label, size, 550.0, foreground))
}

/// `.sw`: a switch on gpui-kit's `Switch`. The caller adds `on_change`.
pub fn switch(id: impl Into<ElementId>, on: bool) -> Switch {
    let (track, border, knob) = if on {
        (theme::accent(), theme::transparent(), theme::accent_on())
    } else {
        (
            theme::bg_hover_strong(),
            theme::border_default(),
            theme::text_muted(),
        )
    };
    Switch::new(id)
        .checked(on)
        .relative()
        .flex_none()
        .w(px(30.0))
        .h(px(18.0))
        .rounded_full()
        .border_1()
        .bg(track)
        .border_color(border)
        .cursor_pointer()
        .child(
            div()
                .absolute()
                .top(px(2.0))
                .left(px(if on { 14.0 } else { 2.0 }))
                .size(px(12.0))
                .rounded_full()
                .bg(knob),
        )
}

/// `.ck2`: the importer's checkbox; `.na` (dimmed, inert) when `available`
/// is false.
pub fn checkbox(id: impl Into<ElementId>, checked: bool, available: bool) -> Checkbox {
    Checkbox::new(id)
        .checked(checked && available)
        .disabled(!available)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(px(14.0))
        .rounded(px(4.0))
        .border_1()
        .map(|checkbox| {
            if checked && available {
                checkbox
                    .bg(theme::accent())
                    .border_color(theme::transparent())
                    .child(text("\u{2713}", 9.0, 600.0, theme::accent_on()))
            } else {
                checkbox.border_color(theme::border_strong())
            }
        })
        .when(!available, |checkbox| checkbox.opacity(0.4))
        .when(available, |checkbox| checkbox.cursor_pointer())
}

/// `.radio`, drawn inside a `Radio` the caller builds around a whole tile.
pub fn radio_dot(on: bool) -> Div {
    let dot = div().flex_none().size(px(14.0)).rounded_full();
    if on {
        dot.border(px(4.0)).border_color(theme::accent())
    } else {
        dot.border_1().border_color(theme::border_strong())
    }
}

/// `.slot`: a selectable tile on gpui-kit's `Radio`.
pub fn slot(id: impl Into<ElementId>, selected: bool) -> Radio {
    Radio::new(id)
        .checked(selected)
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .py(px(11.0))
        .px(px(12.0))
        .border_1()
        .rounded(px(8.0))
        .map(|slot| {
            if selected {
                slot.bg(theme::bg_row_active())
                    .border_color(style::accent_border())
            } else {
                slot.bg(theme::bg_input())
                    .border_color(theme::border_subtle())
                    .cursor_pointer()
                    .hover(|style| style.border_color(theme::border_default()))
            }
        })
}

/// A `.chip`'s color.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Ok,
    Warn,
    Bad,
}

/// `.chip`, with `.ok`, `.warn` or `.bad`.
pub fn chip(label: impl Into<SharedString>, tone: Tone) -> Div {
    let (fill, border, foreground) = match tone {
        Tone::Neutral => (
            theme::bg_elevated(),
            theme::border_subtle(),
            theme::text_muted(),
        ),
        Tone::Ok => (
            style::success_soft(),
            theme::transparent(),
            style::success_text(),
        ),
        Tone::Warn => (style::warn_soft(), theme::transparent(), style::warn_text()),
        Tone::Bad => (
            style::danger_soft(),
            theme::transparent(),
            theme::danger_text(),
        ),
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.0))
        .px(px(9.0))
        .py(px(4.0))
        .rounded_full()
        .border_1()
        .border_color(border)
        .bg(fill)
        .child(text(label, 10.0, 600.0, foreground))
}

/// `.st`: a status dot and its word.
pub fn status(label: impl Into<SharedString>, status: Status) -> Div {
    let dot = match status {
        Status::Ok => theme::status_ok(),
        Status::Warning => theme::status_warning(),
        Status::Error => style::status_error(),
        Status::Off => style::status_suspended(),
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .min_w_0()
        .child(theme::dot(dot))
        .child(text(label, 11.0, 500.0, theme::text_secondary()))
}

/// `.tag`.
pub fn tag(label: impl Into<SharedString>) -> Div {
    tag_frame().child(mono(label, 10.5, 500.0, theme::text_secondary()))
}

/// `.tag` in accent text: a rewrite's action.
pub fn action_tag(label: impl Into<SharedString>) -> Div {
    tag_frame().child(mono(label, 10.5, 500.0, theme::accent_bright()))
}

/// `.tag.fn`.
pub fn fn_tag() -> Div {
    tag_frame()
        .bg(style::accent_soft())
        .border_color(theme::transparent())
        .child(mono("fn", 10.5, 500.0, theme::accent_bright()))
}

/// `.bi` at the Browsers list's 20px size.
pub fn badge(label: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(20.0))
        .rounded(px(5.0))
        .border_1()
        .border_color(theme::border_strong())
        .child(mono(label, 9.0, 700.0, theme::text_muted()))
}

/// `.note`.
pub fn note(content: impl Into<SharedString>) -> Div {
    para(content, 10.5, 400.0, 1.5, theme::text_subtle())
}

/// `.field-help`.
pub fn field_help(content: impl Into<SharedString>) -> Div {
    para(content, 10.5, 400.0, 1.5, theme::text_subtle()).max_w(px(ch(52.0, 10.5)))
}

/// `.cmd`: copyable mono text, one line per entry, with an optional button.
pub fn command_box(lines: &[SharedString], button: Option<AnyElement>) -> Div {
    let body = lines
        .iter()
        .fold(div().flex_1().min_w_0().flex().flex_col(), |body, line| {
            body.child(
                div()
                    .font_family(theme::mono_family(500.0))
                    .font_weight(theme::weight(500.0))
                    .text_size(px(11.0))
                    .line_height(px(16.5))
                    .text_color(theme::text_secondary())
                    .child(line.clone()),
            )
        });
    div()
        .w_full()
        .flex()
        .items_center()
        .gap(px(10.0))
        .py(px(8.0))
        .px(px(10.0))
        .bg(theme::bg_input())
        .border_1()
        .border_color(theme::border_subtle())
        .rounded(px(7.0))
        .child(body)
        .children(button)
}

/// `.row-actions`.
pub fn row_actions() -> Div {
    div().flex().flex_wrap().items_center().gap(px(8.0))
}

/// `.pane-head`: eyebrow, title, help and the pane's actions.
pub fn pane_head(
    eyebrow: &str,
    title: &'static str,
    help: &'static str,
    actions: Option<AnyElement>,
) -> Div {
    div()
        .flex_none()
        .max_w(px(super::CARD_MAX_WIDTH))
        .flex()
        .items_start()
        .gap(px(16.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(tracked(eyebrow, 9.5, 600.0, 0.14, theme::accent()))
                .child(
                    text(title, 19.0, 700.0, theme::text_primary())
                        .mt(px(6.0))
                        .line_height(px(22.8)),
                )
                .child(
                    para(help, 11.5, 400.0, 1.6, theme::text_muted())
                        .mt(px(6.0))
                        .max_w(px(ch(56.0, 11.5))),
                ),
        )
        .when_some(actions, |head, actions| {
            head.child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.0))
                    .child(actions),
            )
        })
}

/// `.card`: the box. Header and body go in as children.
pub fn card(id: impl Into<ElementId>) -> gpui_kit::Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .w_full()
        .max_w(px(super::CARD_MAX_WIDTH))
        .flex()
        .flex_col()
        .border_1()
        .border_color(theme::border_subtle())
        .rounded(px(10.0))
        .bg(theme::bg_panel())
        .overflow_hidden()
}

/// What a card's header says: eyebrow, title and an optional description.
/// The title also names the card's fold toggle for assistive technology.
pub struct Heading {
    pub eyebrow: &'static str,
    pub title: &'static str,
    description: Option<Div>,
}

pub fn heading(eyebrow: &'static str, title: &'static str) -> Heading {
    Heading {
        eyebrow,
        title,
        description: None,
    }
}

impl Heading {
    /// A `.card-desc` under the title.
    pub fn description(mut self, description: Div) -> Self {
        self.description = Some(description);
        self
    }

    pub fn render(self) -> Div {
        card_heading(self.eyebrow, self.title, self.description)
    }
}

/// `.card-hd-main`: eyebrow, title and the optional description.
fn card_heading(eyebrow: &str, title: &'static str, description: Option<Div>) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(tracked(eyebrow, 9.5, 600.0, 0.14, theme::text_muted()))
        .child(
            text(title, 13.5, 600.0, theme::text_primary())
                .mt(px(6.0))
                .line_height(px(16.2)),
        )
        .when_some(description, |heading, description| {
            heading.child(description.mt(px(6.0)).max_w(px(ch(58.0, 11.5))))
        })
}

/// `.card-desc`.
pub fn card_description(content: impl Into<SharedString>) -> Div {
    para(content, 11.5, 400.0, 1.6, theme::text_muted())
}

/// `.field`: the label column on the left, its controls on the right.
pub fn field(
    label: impl Into<SharedString>,
    sub: Option<Div>,
    content: impl IntoIterator<Item = AnyElement>,
) -> Div {
    div()
        .flex()
        .gap(px(20.0))
        .py(px(14.0))
        .px(px(16.0))
        .child(
            div()
                .w(px(200.0))
                .flex_none()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(para(label, 12.0, 600.0, 1.3, theme::text_primary()))
                .children(sub),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .items_start()
                .gap(px(8.0))
                .children(content),
        )
}

/// `.field-sub`.
pub fn field_sub(content: &'static str) -> Div {
    para(content, 11.0, 400.0, 1.5, theme::text_muted())
}

/// `.field-sub` with an inline `<code>` between `before` and `after`.
pub fn field_sub_code(before: &str, code: &str, after: &str) -> Div {
    let muted: Hsla = theme::text_muted();
    style::rich(
        &[
            (before, style::sans(400.0), muted),
            (code, style::mono_font(500.0), theme::accent_bright()),
            (after, style::sans(400.0), muted),
        ],
        11.0,
        1.5,
    )
}

/// Rows separated by `--border-faint`: every row's bottom border but the
/// last one's (`:last-child { border-bottom: none }`).
pub fn separated(rows: Vec<Div>) -> Vec<Div> {
    let last = rows.len().saturating_sub(1);
    rows.into_iter()
        .enumerate()
        .map(|(index, row)| {
            if index < last {
                row.border_b_1().border_color(theme::border_faint())
            } else {
                row
            }
        })
        .collect()
}

/// `.sess.head` and friends: an uppercase column heading.
pub fn column_head(label: &str) -> Div {
    tracked(label, 9.5, 600.0, 0.14, theme::text_muted())
}
