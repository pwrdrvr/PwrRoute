//! Design tokens and the text primitives every view draws with.
//!
//! The colors are the dark-theme values from `design/pwrroute.css`, which
//! mirror the PwrSuite family palette. Sizes stay in CSS pixels so a view
//! reads like the design it was drawn from.
//!
//! Where the design falls short of WCAG 2.2 AA a token says so and departs
//! from it (`border_control`, `accent_border`); the tests at the bottom
//! hold every text token to 4.5:1 and every control edge, status dot and
//! focus ring to 3:1 on every surface.

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
    /// The edge of a control that is known by its edge alone: a text
    /// field's box, a switch's track when off, a radio or a checkbox.
    /// The design draws those in `--border-default` (1.4:1 on the
    /// panels); WCAG 1.4.11 asks 3:1, which this is on every surface
    /// (`tests::controls_meet_3_to_1`). Cards, dividers and frames stay
    /// on the fainter borders: they are decoration.
    border_control = hex_alpha(0xf7f3eb, 0.40);
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
    /// A selected tile's or lit row's edge. The design's 0.42 is 2.3:1 on
    /// the panels, and for a tile the edge is what says it is chosen:
    /// 0.56 is 3:1 (WCAG 1.4.11) and the same orange.
    accent_border = hex_alpha(0xff8a1f, 0.56);
    /// The keyboard focus ring (`widgets::focus_visible`): 8:1 on every
    /// surface.
    focus_ring = hex(0xff8a1f);
    /// The ring on a control filled with the accent, where an accent ring
    /// would not show: 18:1 on the panels around it.
    focus_ring_on_accent = hex(0xf7f3eb);
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
#[track_caller]
pub fn text(content: impl Into<SharedString>, size: f32, css_weight: f32, color: Hsla) -> Div {
    div()
        .flex_none()
        .font_family(SANS)
        .font_weight(weight(css_weight))
        .text_size(px(size))
        .line_height(px(size))
        .text_color(color)
        .whitespace_nowrap()
        .child(crate::accessibility::label(content))
}

/// The same in Geist Mono.
#[track_caller]
pub fn mono(content: impl Into<SharedString>, size: f32, css_weight: f32, color: Hsla) -> Div {
    text(content, size, css_weight, color).font_family(mono_family(css_weight))
}

/// Uppercase text with CSS `letter-spacing: <em>em`.
///
/// GPUI has no letter-spacing, so each glyph is its own flex item and the
/// tracking is the gap. GPUI rounds every measured text box up to a whole
/// pixel, about 0.5 to 0.75px per glyph, so that much comes back out of the
/// gap. Good for short eyebrows; not for running text.
#[track_caller]
pub fn tracked(content: &str, size: f32, css_weight: f32, em: f32, color: Hsla) -> Div {
    let mut row = div()
        .flex()
        .flex_none()
        .gap(px((size * em - 0.75).max(0.0)));
    for ch in content.to_uppercase().chars() {
        let glyph = if ch == ' ' { '\u{00a0}' } else { ch };
        row = row.child(
            div()
                .font_family(SANS)
                .font_weight(weight(css_weight))
                .text_size(px(size))
                .line_height(px(size))
                .text_color(color)
                .child(glyph.to_string()),
        );
    }
    crate::accessibility::heading(row, content)
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

// PWR-15 palette: filled in by the contrast thread
static INCREASE_CONTRAST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn set_increase_contrast(on: bool) {
    INCREASE_CONTRAST.store(on, std::sync::atomic::Ordering::Relaxed);
}
pub fn increase_contrast() -> bool {
    INCREASE_CONTRAST.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use gpui_kit::Rgba;

    use super::*;

    /// Every fill a view draws text or a control on.
    fn surfaces() -> Vec<(&'static str, Hsla)> {
        vec![
            ("bg_app", bg_app()),
            ("bg_titlebar", bg_titlebar()),
            ("bg_sidebar", bg_sidebar()),
            ("bg_panel", bg_panel()),
            ("bg_input", bg_input()),
            ("bg_elevated", bg_elevated()),
            ("bg_hover", bg_hover()),
            ("bg_hover_strong", bg_hover_strong()),
            ("bg_row_active", bg_row_active()),
            ("bg_popover", bg_popover()),
        ]
    }

    /// `color` over the opaque `under`, as the renderer blends it.
    fn over(color: Hsla, under: Hsla) -> [f32; 3] {
        let top = Rgba::from(color);
        let bottom = Rgba::from(under);
        let a = top.a;
        [
            top.r * a + bottom.r * (1.0 - a),
            top.g * a + bottom.g * (1.0 - a),
            top.b * a + bottom.b * (1.0 - a),
        ]
    }

    /// WCAG 2.2 relative luminance of an sRGB color.
    fn luminance([r, g, b]: [f32; 3]) -> f32 {
        let linear = |c: f32| {
            if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    }

    fn contrast(color: Hsla, under: Hsla) -> f32 {
        let a = luminance(over(color, under));
        let b = luminance(over(under, under));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    fn assert_on_every_surface(tokens: &[(&str, Hsla)], minimum: f32) {
        let mut failing = Vec::new();
        for (name, color) in tokens {
            for (surface, under) in surfaces() {
                let ratio = contrast(*color, under);
                if ratio < minimum {
                    failing.push(format!("{name} on {surface}: {ratio:.2}"));
                }
            }
        }
        assert!(failing.is_empty(), "below {minimum}:1: {failing:#?}");
    }

    #[test]
    fn contrast_matches_known_values() {
        // WCAG's own extremes, and the design's muted text on black.
        assert!((contrast(hex(0xffffff), hex(0x000000)) - 21.0).abs() < 0.01);
        assert!((contrast(hex(0x8c857a), hex(0x000000)) - 5.75).abs() < 0.01);
    }

    #[test]
    fn text_meets_4_5_to_1() {
        assert_on_every_surface(
            &[
                ("text_primary", text_primary()),
                ("text_secondary", text_secondary()),
                ("text_muted", text_muted()),
                ("text_subtle", text_subtle()),
                ("accent", accent()),
                ("accent_bright", accent_bright()),
                ("danger_text", danger_text()),
                ("success_text", success_text()),
                ("info_text", info_text()),
                ("warn_text", warn_text()),
                ("log_target", log_target()),
            ],
            4.5,
        );
        assert!(contrast(accent_on(), accent()) >= 4.5);
    }

    #[test]
    fn controls_meet_3_to_1() {
        assert_on_every_surface(
            &[
                ("border_control", border_control()),
                ("accent_border", accent_border()),
                ("focus_ring", focus_ring()),
                ("focus_ring_on_accent", focus_ring_on_accent()),
                ("status_ok", status_ok()),
                ("status_error", status_error()),
                ("status_warning", status_warning()),
                ("status_suspended", status_suspended()),
            ],
            3.0,
        );
        // The ring on a primary button shows against its own fill too.
        assert!(contrast(focus_ring_on_accent(), accent()) >= 2.0);
    }
}
