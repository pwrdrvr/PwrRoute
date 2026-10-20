//! The tokens Settings needs that the main window does not draw yet, and the
//! wrapping-text helpers its cards are made of.
//!
//! Values from `design/pwrroute.css`. If the main window comes to need one
//! of these, move it into `theme` rather than copying it.

use gpui_kit::{
    Div, Font, Hsla, ParentElement as _, SharedString, Styled as _, StyledText, TextRun, div, font,
    px, rgb,
};

use crate::theme;

fn rgba(value: u32, alpha: f32) -> Hsla {
    let mut color: Hsla = rgb(value).into();
    color.a = alpha;
    color
}

pub fn accent_soft() -> Hsla {
    rgba(0xff8a1f, 0.12)
}

pub fn accent_border() -> Hsla {
    rgba(0xff8a1f, 0.42)
}

pub fn success_soft() -> Hsla {
    rgba(0x4a945c, 0.18)
}

pub fn success_text() -> Hsla {
    rgb(0x9ce5b3).into()
}

pub fn warn_soft() -> Hsla {
    rgba(0xd99a3d, 0.14)
}

pub fn warn_text() -> Hsla {
    rgb(0xe8c890).into()
}

pub fn danger_soft() -> Hsla {
    rgba(0xb94232, 0.24)
}

pub fn status_error() -> Hsla {
    rgb(0xc45a3a).into()
}

pub fn status_suspended() -> Hsla {
    rgb(0x6b6660).into()
}

/// CSS `ch` for Geist: its "0" is 0.663em in Chrome.
pub fn ch(count: f32, size: f32) -> f32 {
    count * size * 0.663
}

/// A wrapping paragraph: CSS `font: <weight> <size>/<line-height>`.
pub fn para(
    content: impl Into<SharedString>,
    size: f32,
    css_weight: f32,
    line_height: f32,
    color: Hsla,
) -> Div {
    div()
        .min_w_0()
        .font_family(theme::SANS)
        .font_weight(theme::weight(css_weight))
        .text_size(px(size))
        .line_height(px(size * line_height))
        .text_color(color)
        .child(content.into())
}

/// The font a run of [`rich`] text is drawn in.
pub fn sans(css_weight: f32) -> Font {
    let mut sans = font(theme::SANS);
    sans.weight = theme::weight(css_weight);
    sans
}

pub fn mono_font(css_weight: f32) -> Font {
    let mut mono = font(theme::mono_family(css_weight));
    mono.weight = theme::weight(css_weight);
    mono
}

/// Wrapping text whose runs change font or color, such as a sentence with
/// a `<code>` in it. GPUI's runs share one size, so the size is the
/// paragraph's.
pub fn rich(runs: &[(&str, Font, Hsla)], size: f32, line_height: f32) -> Div {
    let content: String = runs.iter().map(|(words, _, _)| *words).collect();
    let runs = runs
        .iter()
        .map(|(words, font, color)| TextRun {
            len: words.len(),
            font: font.clone(),
            color: *color,
            background_color: None,
            underline: None,
            strikethrough: None,
        })
        .collect();
    div()
        .min_w_0()
        .text_size(px(size))
        .line_height(px(size * line_height))
        .child(StyledText::new(content).with_runs(runs))
}
