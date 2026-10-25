//! Design tokens and the text primitives every view draws with.
//!
//! The colors are the dark-theme values from `design/pwrroute.css`, which
//! mirror the PwrSuite family palette. Sizes stay in CSS pixels so a view
//! reads like the design it was drawn from.

use gpui_kit::{Div, FontWeight, Hsla, IntoElement, ParentElement as _, SharedString, Styled as _};
use gpui_kit::{div, px, rgb, svg};

/// The family's sans face. Embedded; see `assets`.
pub const SANS: &str = "Geist";

/// Added to every CSS weight before it reaches GPUI.
///
/// GPUI draws light-on-dark text about one weight step heavier than Chrome
/// with `-webkit-font-smoothing: antialiased`, which is what the design was
/// reviewed in. Turning off CoreText font smoothing does not change that, so
/// the views ask for one step lighter: 500 renders the way the design's 500
/// looks. Set to `0.0` to request the design's weights verbatim.
pub const WEIGHT_SHIFT: f32 = -100.0;

/// A CSS weight, adjusted by [`WEIGHT_SHIFT`].
pub fn weight(css: f32) -> FontWeight {
    FontWeight(shifted(css))
}

fn shifted(css: f32) -> f32 {
    (css + WEIGHT_SHIFT).clamp(100.0, 900.0)
}

/// The Geist Mono family for a weight.
///
/// The static Medium and SemiBold cuts carry no typographic family name
/// (nameID 16), so CoreText lists each as a family of its own. Pick by the
/// weight that will actually be drawn.
pub fn mono_family(css: f32) -> &'static str {
    let drawn = shifted(css);
    if drawn < 450.0 {
        "Geist Mono"
    } else if drawn < 550.0 {
        "Geist Mono Medium"
    } else {
        "Geist Mono SemiBold"
    }
}

fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

fn hex_alpha(value: u32, alpha: f32) -> Hsla {
    let mut color: Hsla = rgb(value).into();
    color.a = alpha;
    color
}

/// One function per token: `Hsla` construction is not `const`.
macro_rules! tokens {
    ($($(#[$doc:meta])* $name:ident = $value:expr;)*) => {
        $(
            $(#[$doc])*
            pub fn $name() -> Hsla {
                $value
            }
        )*
    };
}

tokens! {
    bg_app = hex(0x000000);
    bg_titlebar = hex(0x050505);
    bg_sidebar = hex(0x050505);
    bg_panel = hex(0x0a0a0a);
    bg_input = hex(0x080808);
    bg_elevated = hex(0x101010);
    /// Row hover.
    bg_hover = hex(0x14110d);
    bg_hover_strong = hex(0x1f1a12);
    /// Selected row and rail item.
    bg_row_active = hex(0x120800);
    border_faint = hex_alpha(0xf7f3eb, 0.06);
    border_subtle = hex_alpha(0xf7f3eb, 0.10);
    border_default = hex_alpha(0xf7f3eb, 0.14);
    border_strong = hex_alpha(0xf7f3eb, 0.20);
    text_primary = hex(0xf7f3eb);
    text_secondary = hex(0xb8b0a5);
    text_muted = hex(0x8c857a);
    text_subtle = hex_alpha(0xf7f3eb, 0.54);
    accent = hex(0xff8a1f);
    accent_bright = hex(0xffb35c);
    accent_on = hex(0x160a00);
    accent_soft = hex_alpha(0xff8a1f, 0.12);
    /// `.rr.new`, a line that is the rule in a source frame.
    accent_tint = hex_alpha(0xff8a1f, 0.08);
    accent_border = hex_alpha(0xff8a1f, 0.42);
    danger_soft = hex_alpha(0xb94232, 0.24);
    danger_text = hex(0xffb0a1);
    success_soft = hex_alpha(0x4a945c, 0.18);
    success_text = hex(0x9ce5b3);
    info_soft = hex_alpha(0x5689c4, 0.16);
    info_text = hex(0x9fc8ff);
    status_ok = hex(0x5fa969);
    status_error = hex(0xc45a3a);
    status_warning = hex(0xd99a3d);
    warn_soft = hex_alpha(0xd99a3d, 0.14);
    warn_text = hex(0xe8c890);
    transparent = hex_alpha(0x000000, 0.0);
    /// Windows' own close-button hover, under a white glyph: the caption
    /// buttons are the OS's, so they keep its colours.
    caption_close = hex(0xc42b1c);
    /// `.strip-chip.off`'s dot: not routing.
    status_suspended = hex(0x6b6660);
    /// The menu bar dropdown's fill: `.tray`'s rgba(16,16,16,0.98), made
    /// opaque. In the transparent popup window the 0.98 fill let the window
    /// behind it show through visibly.
    bg_popover = hex(0x101010);
    /// Help › Logs (PwrAgent's "Log Viewer Workflows UX Review"): a line's
    /// target, a selected line, and the warning and error rows' washes.
    log_target = hex(0x7fcfc4);
    log_selected = hex_alpha(0xff8a1f, 0.10);
    log_warn_row = hex_alpha(0xd99a3d, 0.045);
    log_error_row = hex_alpha(0xb94232, 0.075);
}

/// A single-line run of sans text: CSS `font: <weight> <size>/1`.
pub fn text(content: impl Into<SharedString>, size: f32, css_weight: f32, color: Hsla) -> Div {
    div()
        .flex_none()
        .font_family(SANS)
        .font_weight(weight(css_weight))
        .text_size(px(size))
        .line_height(px(size))
        .text_color(color)
        .whitespace_nowrap()
        .child(content.into())
}

/// The same in Geist Mono.
pub fn mono(content: impl Into<SharedString>, size: f32, css_weight: f32, color: Hsla) -> Div {
    text(content, size, css_weight, color).font_family(mono_family(css_weight))
}

/// Uppercase text with CSS `letter-spacing: <em>em`.
///
/// GPUI has no letter-spacing, so each glyph is its own flex item and the
/// tracking is the gap. GPUI rounds every measured text box up to a whole
/// pixel, about 0.5 to 0.75px per glyph, so that much comes back out of the
/// gap. Good for short eyebrows; not for running text.
pub fn tracked(content: &str, size: f32, css_weight: f32, em: f32, color: Hsla) -> Div {
    let mut row = div()
        .flex()
        .flex_none()
        .gap(px((size * em - 0.75).max(0.0)));
    for ch in content.to_uppercase().chars() {
        let glyph = if ch == ' ' { '\u{00a0}' } else { ch };
        row = row.child(text(glyph.to_string(), size, css_weight, color));
    }
    row
}

/// A monochrome SVG from the embedded assets, tinted with `color`.
pub fn icon(path: &'static str, size: f32, color: Hsla) -> impl IntoElement {
    svg()
        .path(path)
        .flex_none()
        .size(px(size))
        .text_color(color)
}

/// The 6px status dot.
pub fn dot(color: Hsla) -> Div {
    div().flex_none().size(px(6.0)).rounded_full().bg(color)
}
