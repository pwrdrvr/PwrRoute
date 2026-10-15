//! The design's small controls: chips, tags, badges, buttons, inputs and
//! switches. Each mirrors one class in `design/pwrroute.css`.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{Div, Hsla, ParentElement as _, Styled as _, div, px};

use crate::theme::{self, icon, mono, text};

/// `.tag`: a mono chip for a pattern or identifier.
pub fn tag(label: &'static str) -> Div {
    tag_frame().child(mono(label, 10.5, 500.0, theme::text_secondary()))
}

/// `.tag.re`: a regex pattern.
pub fn regex_tag(label: &'static str) -> Div {
    tag_frame()
        .bg(theme::info_soft())
        .border_color(theme::transparent())
        .child(mono(label, 10.5, 500.0, theme::info_text()))
}

/// `.tag.app`: an app, with its badge.
pub fn app_tag(badge_label: &'static str, name: &'static str) -> Div {
    tag_frame()
        .child(badge(badge_label))
        .child(mono(name, 10.5, 500.0, theme::text_primary()))
}

/// `.tag.act`: a rewrite action, `kind` dimmed before the value.
pub fn action_tag(kind: &'static str, value: &'static str) -> Div {
    tag_frame()
        .child(mono(kind, 10.5, 500.0, theme::text_subtle()))
        .child(mono(value, 10.5, 500.0, theme::accent_bright()))
}

/// The box every `.tag` shares.
pub fn tag_frame() -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.0))
        .h(px(20.0))
        .px(px(7.0))
        .border_1()
        .border_color(theme::border_subtle())
        .rounded(px(6.0))
        .bg(theme::bg_elevated())
}

/// `.bi`: a browser or app's two-letter badge.
pub fn badge(label: &'static str) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(12.0))
        .rounded(px(3.0))
        .border_1()
        .border_color(theme::border_strong())
        .font_family(theme::mono_family(700.0))
        .font_weight(theme::weight(700.0))
        .text_size(px(7.0))
        .line_height(px(7.0))
        .text_color(theme::text_muted())
        .child(label)
}

/// `.chip`: a count or state in a card header.
pub fn chip(label: &'static str) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .px(px(9.0))
        .py(px(4.0))
        .rounded_full()
        .border_1()
        .border_color(theme::border_subtle())
        .bg(theme::bg_elevated())
        .child(text(label, 10.0, 600.0, theme::text_muted()))
}

/// `.btn` with a plus icon; `.btn.primary` when `primary`.
pub fn add_button(label: &'static str, primary: bool) -> Div {
    let foreground = if primary {
        theme::accent_on()
    } else {
        theme::text_primary()
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .py(px(7.0))
        .px(px(12.0))
        .border_1()
        .rounded(px(7.0))
        .map(|button| {
            if primary {
                button
                    .bg(theme::accent())
                    .border_color(theme::transparent())
            } else {
                button
                    .bg(theme::bg_elevated())
                    .border_color(theme::border_default())
            }
        })
        .child(icon("pwrroute/plus.svg", 13.0, foreground))
        .child(text(label, 11.5, 550.0, foreground))
}

/// `.inp`: a field's box. Content goes in as children.
pub fn input_box() -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .min_h(px(30.0))
        .px(px(10.0))
        .bg(theme::bg_input())
        .border_1()
        .border_color(theme::border_default())
        .rounded(px(7.0))
        .overflow_hidden()
}

/// `.inp.focus`. GPUI has no `outline`, so the 2px ring with its 1px offset
/// is a border around the box, pulled out by a negative margin so it takes
/// no layout space.
pub fn focus_ring(input: Div) -> Div {
    div()
        .m(px(-3.0))
        .p(px(1.0))
        .border_2()
        .border_color(theme::accent())
        .rounded(px(10.0))
        .child(input)
}

/// `.caret`: a select's ▾.
pub fn caret() -> Div {
    div()
        .ml_auto()
        .child(text("\u{25be}", 10.0, 400.0, theme::text_subtle()))
}

/// `.sw`: a switch.
pub fn switch(on: bool) -> Div {
    let (track, border, knob): (Hsla, Hsla, Hsla) = if on {
        (theme::accent(), theme::transparent(), theme::accent_on())
    } else {
        (
            theme::bg_hover_strong(),
            theme::border_default(),
            theme::text_muted(),
        )
    };
    div()
        .relative()
        .flex_none()
        .w(px(30.0))
        .h(px(18.0))
        .rounded_full()
        .border_1()
        .bg(track)
        .border_color(border)
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

/// `.strip-chip`: a status chip in the title strip.
pub fn status_chip(label: &'static str) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .h(px(24.0))
        .px(px(9.0))
        .border_1()
        .border_color(theme::border_subtle())
        .rounded_full()
        .bg(theme::bg_elevated())
        .child(theme::dot(theme::status_ok()))
        .child(mono(label, 10.5, 600.0, theme::text_muted()))
}

/// `.kbd`: a key cap.
pub fn key_cap(label: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(18.0))
        .px(px(5.0))
        .border_1()
        .border_color(theme::border_default())
        .rounded(px(4.0))
        .child(mono(label, 10.0, 600.0, theme::text_muted()))
}
