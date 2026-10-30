//! The wrapping-text helpers Settings' cards are made of, and the tokens
//! its panes name through `style::`, which are `theme`'s: never a copy.

use crate::theme::px;
use gpui_kit::{
    Div, Font, Hsla, ParentElement as _, SharedString, Styled as _, StyledText, TextRun, div, font,
};

use crate::theme;

pub use theme::{
    accent_border, accent_soft, danger_soft, status_error, status_suspended, success_soft,
    success_text, warn_soft, warn_text,
};

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
