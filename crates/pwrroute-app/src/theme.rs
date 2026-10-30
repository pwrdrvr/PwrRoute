//! Design tokens and the text primitives every view draws with.
//!
//! The colors are `design/pwrroute.css`'s, which mirror the PwrSuite family
//! palette, in four palettes: dark and light, each standard and with the
//! OS's Increase Contrast on (design: Contrast and Scale). Every token is a
//! function that reads the palette in force, so a view never names one.
//! Sizes stay in CSS pixels so a view reads like the design it was drawn
//! from; [`px`] multiplies them by the zoom.
//!
//! Where the design falls short of WCAG 2.2 AA a token says so and departs
//! from it (`border_control`, `accent_border`). The tests at the bottom
//! hold every text token to 4.5:1 and every control edge, status dot and
//! focus ring to 3:1 on every surface, in every palette, and pin each
//! pair's ratio in `contrast.snap`, so a change to a token shows in review.
//!
//! The palette, the zoom and [`dimmed`] are the GPUI thread's: set them
//! there, then refresh the windows.

use std::cell::Cell;

use gpui_kit::{
    Div, FontWeight, Hsla, IntoElement, ParentElement as _, Pixels, SharedString, Styled as _,
};
use gpui_kit::{div, rgb, svg};

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

/// Dark is PwrRoute's look; light is drawn by the `screens` example and
/// `PWRROUTE_APPEARANCE=light` (a preview: the app does not follow the OS's
/// appearance yet).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    #[default]
    Dark,
    Light,
}

/// What every token and size reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub appearance: Appearance,
    /// The OS's Increase Contrast (macOS) or high contrast (Windows):
    /// stronger edges, opaque text one step brighter, solid accent edges.
    pub increase_contrast: bool,
    /// View › Zoom: every [`px`] is multiplied by it. One of [`ZOOM_STEPS`].
    pub zoom: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            appearance: Appearance::Dark,
            increase_contrast: false,
            zoom: 1.0,
        }
    }
}

thread_local! {
    static THEME: Cell<Theme> = Cell::new(Theme::default());
    static DIMMED: Cell<bool> = const { Cell::new(false) };
}

/// The palette and zoom in force.
pub fn current() -> Theme {
    THEME.get()
}

/// Sets the palette and zoom. The caller refreshes the windows.
pub fn set(theme: Theme) {
    THEME.set(theme);
}

/// PWR-15: the OS's Increase Contrast, read by `AccessibilityPrefs`.
pub fn set_increase_contrast(on: bool) {
    set(Theme {
        increase_contrast: on,
        ..current()
    });
}

pub fn increase_contrast() -> bool {
    current().increase_contrast
}

pub fn set_appearance(appearance: Appearance) {
    set(Theme {
        appearance,
        ..current()
    });
}

/// View › Zoom's steps: Actual Size, then up to 200% (WCAG 1.4.4).
pub const ZOOM_STEPS: [f32; 6] = [1.0, 1.1, 1.25, 1.5, 1.75, 2.0];

pub fn zoom() -> f32 {
    current().zoom
}

/// Sets the zoom, snapped to the nearest of [`ZOOM_STEPS`].
pub fn set_zoom(zoom: f32) {
    set(Theme {
        zoom: snap_zoom(zoom),
        ..current()
    });
}

/// The nearest of [`ZOOM_STEPS`]: a hand-edited `settings.json` cannot ask
/// for 0 or 40.
pub fn snap_zoom(zoom: f32) -> f32 {
    ZOOM_STEPS
        .into_iter()
        .min_by(|a, b| (a - zoom).abs().total_cmp(&(b - zoom).abs()))
        .unwrap_or(1.0)
}

/// The step after `zoom` (`1`) or before it (`-1`), held at the ends.
pub fn zoom_step(zoom: f32, by: i32) -> f32 {
    let at = ZOOM_STEPS
        .iter()
        .position(|step| (step - snap_zoom(zoom)).abs() < f32::EPSILON)
        .unwrap_or(0);
    let last = ZOOM_STEPS.len() - 1;
    let to = if by < 0 {
        at.saturating_sub(by.unsigned_abs() as usize)
    } else {
        (at + by as usize).min(last)
    };
    ZOOM_STEPS[to]
}

/// A length in the design's CSS pixels, zoomed. Every view sizes with
/// this, never `gpui_kit::px`, which is for screen geometry (`macos.rs`)
/// and for tests.
pub fn px(css: f32) -> Pixels {
    gpui_kit::px(css * zoom())
}

/// A measured length back in the design's CSS pixels, to compare with the
/// constants a layout was written in.
pub fn css(length: Pixels) -> f32 {
    f32::from(length) / zoom()
}

/// Builds `content` dimmed (PWR-13): a rule that is off, a step the link
/// missed, a row nothing runs. Text drops one step down the neutral ramp
/// (primary to secondary, secondary and subtle to muted; muted is the
/// floor, 4.5:1 everywhere) and colored text keeps its color, so dimmed
/// text never fails contrast. Badges fade to [`DIM_DECORATIVE`]
/// (`widgets::badge`). Never `.opacity()` on a box that holds text that
/// is read: say "off" some other way (a chip, a strike, the switch).
pub fn dimmed<R>(content: impl FnOnce() -> R) -> R {
    let _scope = dim_scope(true);
    content()
}

/// [`dimmed`] while the guard lives, for a builder too long for a
/// closure. `dim: false` changes nothing.
pub fn dim_scope(dim: bool) -> DimScope {
    DimScope(DIMMED.replace(DIMMED.get() || dim))
}

/// What [`dim_scope`] returns: drop it to stop dimming.
#[must_use = "dimming stops when the scope is dropped"]
pub struct DimScope(bool);

impl Drop for DimScope {
    fn drop(&mut self) {
        DIMMED.set(self.0);
    }
}

/// [`dimmed`] when `dim`, else `content` as it is.
pub fn dimmed_if<R>(dim: bool, content: impl FnOnce() -> R) -> R {
    if dim { dimmed(content) } else { content() }
}

/// Inside [`dimmed`].
pub fn is_dimmed() -> bool {
    DIMMED.get()
}

/// A dimmed row's badges, icons and route lines: never text.
pub const DIM_DECORATIVE: f32 = 0.45;
/// A control that cannot be used now (WCAG 1.4.3 exempts it).
pub const DISABLED: f32 = 0.5;
/// A row lifted by a drag, and the cards it may not drop in: for as long
/// as the drag lasts.
pub const LIFTED: f32 = 0.4;
/// What the pointer is not on while it traces a route or a card: for as
/// long as the hover lasts.
pub const SPOTLIGHT: f32 = 0.45;

fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

fn hex_alpha(value: u32, alpha: f32) -> Hsla {
    let mut color: Hsla = rgb(value).into();
    color.a = alpha;
    color
}

/// One function per token: `Hsla` construction is not `const`. Each gives
/// its dark and light values, then optionally (`hc:`) the two with
/// Increase Contrast on; without them the token does not change.
macro_rules! tokens {
    ($($(#[$doc:meta])* $vis:vis $name:ident = $dark:expr, $light:expr $(, hc: $hc_dark:expr, $hc_light:expr)?;)*) => {
        $(
            $(#[$doc])*
            $vis fn $name() -> Hsla {
                let theme = current();
                match (theme.appearance, theme.increase_contrast) {
                    (Appearance::Dark, false) => $dark,
                    (Appearance::Light, false) => $light,
                    (Appearance::Dark, true) => tokens!(@or $dark $(, $hc_dark)?),
                    (Appearance::Light, true) => tokens!(@or $light $(, $hc_light)?),
                }
            }
        )*
    };
    (@or $base:expr) => { $base };
    (@or $base:expr, $hc:expr) => { $hc };
}

/// Bone (`#f7f3eb`) and ink (`#1a1612`) at an alpha: dark's and light's
/// edges and quiet text.
fn bone(alpha: f32) -> Hsla {
    hex_alpha(0xf7f3eb, alpha)
}

fn ink(alpha: f32) -> Hsla {
    hex_alpha(0x1a1612, alpha)
}

fn black(alpha: f32) -> Hsla {
    hex_alpha(0x000000, alpha)
}

tokens! {
    pub bg_app = hex(0x000000), hex(0xffffff);
    pub bg_titlebar = hex(0x050505), hex(0xf7f4ef);
    pub bg_sidebar = hex(0x050505), hex(0xf7f4ef);
    pub bg_panel = hex(0x0a0a0a), hex(0xfdfcfa);
    pub bg_input = hex(0x080808), hex(0xffffff);
    pub bg_elevated = hex(0x101010), hex(0xffffff);
    /// Row hover.
    pub bg_hover = hex(0x14110d), hex(0xf4f0e8);
    pub bg_hover_strong = hex(0x1f1a12), hex(0xebe5da);
    /// Selected row and rail item.
    pub bg_row_active = hex(0x120800), hex(0xfff5e9);
    /// Dividers. With Increase Contrast every edge comes up to where a
    /// card, a divider and a field read without the fill behind them.
    pub border_faint = bone(0.06), black(0.05), hc: bone(0.24), black(0.16);
    pub border_subtle = bone(0.10), black(0.08), hc: bone(0.36), black(0.26);
    pub border_default = bone(0.14), black(0.12), hc: bone(0.44), black(0.36);
    pub border_strong = bone(0.20), black(0.18), hc: bone(0.52), black(0.45);
    /// The edge of a control that is known by its edge alone: a text
    /// field's box, a switch's track when off, a radio or a checkbox.
    /// The design draws those in `--border-default` (1.4:1 on the
    /// panels); WCAG 1.4.11 asks 3:1, which this is on every surface
    /// (`tests::controls_meet_3_to_1`). Cards, dividers and frames stay
    /// on the fainter borders: they are decoration.
    pub border_control = bone(0.40), black(0.45), hc: bone(0.62), black(0.62);
    ink_primary = hex(0xf7f3eb), hex(0x1a1612);
    ink_secondary = hex(0xb8b0a5), hex(0x524a40), hc: hex(0xd3cbbf), hex(0x3d362e);
    ink_muted = hex(0x8c857a), hex(0x6a635b), hc: hex(0xb8b0a5), hex(0x524a40);
    /// Opaque with Increase Contrast: no text is see-through.
    ink_subtle = bone(0.54), ink(0.68), hc: hex(0xb8b0a5), hex(0x524a40);
    /// Light's accent ramp runs the other way: brighter is darker on
    /// white. The design's `#b74c00` is 4.1:1 on `bg_hover_strong`.
    pub accent = hex(0xff8a1f), hex(0xa64400), hc: hex(0xff8a1f), hex(0x8f3a00);
    pub accent_bright = hex(0xffb35c), hex(0x994c00), hc: hex(0xffc27a), hex(0x803500);
    pub accent_on = hex(0x160a00), hex(0xffffff);
    pub accent_soft = hex_alpha(0xff8a1f, 0.12), hex_alpha(0xa64400, 0.10);
    /// `.rr.new`, a line that is the rule in a source frame.
    pub accent_tint = hex_alpha(0xff8a1f, 0.08), hex_alpha(0xa64400, 0.06);
    /// A selected tile's or lit row's edge. The design's 0.42 is 2.3:1 on
    /// the panels, and for a tile the edge is what says it is chosen:
    /// 0.56 is 3:1 (WCAG 1.4.11) and the same orange. Solid with Increase
    /// Contrast.
    pub accent_border = hex_alpha(0xff8a1f, 0.56), hex_alpha(0xa64400, 0.80),
        hc: hex(0xff8a1f), hex(0x8f3a00);
    /// The keyboard focus ring (`widgets::focus_visible`): 7:1 or more on
    /// every surface, solid in every palette.
    pub focus_ring = hex(0xff8a1f), hex(0xa64400), hc: hex(0xff8a1f), hex(0x8f3a00);
    /// The ring on a control filled with the accent, where an accent ring
    /// would not show: 18:1 on the panels around it.
    pub focus_ring_on_accent = hex(0xf7f3eb), hex(0x1a1612);
    pub danger_soft = hex_alpha(0xb94232, 0.24), hex_alpha(0x8a1f12, 0.12);
    pub danger_text = hex(0xffb0a1), hex(0x8a1f12), hc: hex(0xffc8bd), hex(0x6e170c);
    pub success_soft = hex_alpha(0x4a945c, 0.18), hex_alpha(0x2e6e40, 0.14);
    pub success_text = hex(0x9ce5b3), hex(0x1a5c2e), hc: hex(0xb8f0ca), hex(0x124722);
    pub info_soft = hex_alpha(0x5689c4, 0.16), hex_alpha(0x3666a5, 0.12);
    pub info_text = hex(0x9fc8ff), hex(0x1a4f8a), hc: hex(0xbcd9ff), hex(0x123d6e);
    pub status_ok = hex(0x5fa969), hex(0x2e7d3c), hc: hex(0x74c47f), hex(0x24672f);
    pub status_error = hex(0xc45a3a), hex(0xa8472a), hc: hex(0xe0714e), hex(0x8f3a20);
    pub status_warning = hex(0xd99a3d), hex(0x805200), hc: hex(0xe8b060), hex(0x6b4700);
    pub warn_soft = hex_alpha(0xd99a3d, 0.14), hex_alpha(0xa86b00, 0.14);
    /// The design's light block kept dark's `#e8c890` (1.6:1 on white).
    pub warn_text = hex(0xe8c890), hex(0x6b4700), hc: hex(0xf2dab0), hex(0x553800);
    pub transparent = hex_alpha(0x000000, 0.0), hex_alpha(0x000000, 0.0);
    /// Windows' own close-button hover, under a white glyph: the caption
    /// buttons are the OS's, so they keep its colours.
    pub caption_close = hex(0xc42b1c), hex(0xc42b1c);
    /// `.strip-chip.off`'s dot: not routing.
    pub status_suspended = hex(0x6b6660), hex(0x7a746c), hc: hex(0x8c857a), hex(0x5f5951);
    /// The menu bar dropdown's fill: `.tray`'s rgba(16,16,16,0.98), made
    /// opaque. In the transparent popup window the 0.98 fill let the window
    /// behind it show through visibly.
    pub bg_popover = hex(0x101010), hex(0xffffff);
    /// Help › Logs (PwrAgent's "Log Viewer Workflows UX Review"): a line's
    /// target, a selected line, and the warning and error rows' washes.
    pub log_target = hex(0x7fcfc4), hex(0x1f6f66), hc: hex(0xa3e3da), hex(0x175a52);
    pub log_selected = hex_alpha(0xff8a1f, 0.10), hex_alpha(0xa64400, 0.10);
    pub log_warn_row = hex_alpha(0xd99a3d, 0.045), hex_alpha(0xa86b00, 0.06);
    pub log_error_row = hex_alpha(0xb94232, 0.075), hex_alpha(0x8a1f12, 0.06);
}

/// Titles and body copy.
pub fn text_primary() -> Hsla {
    if is_dimmed() {
        ink_secondary()
    } else {
        ink_primary()
    }
}

/// Supporting copy.
pub fn text_secondary() -> Hsla {
    if is_dimmed() {
        ink_muted()
    } else {
        ink_secondary()
    }
}

/// Metadata, labels: the floor. Dimmed text stops here.
pub fn text_muted() -> Hsla {
    ink_muted()
}

/// Quiet counts, ages and grips: bone at 54%, so it sits on every surface.
pub fn text_subtle() -> Hsla {
    if is_dimmed() {
        ink_muted()
    } else {
        ink_subtle()
    }
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

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use gpui_kit::Rgba;

    use super::*;

    const PALETTES: [(&str, Appearance, bool); 4] = [
        ("dark", Appearance::Dark, false),
        ("dark, increase contrast", Appearance::Dark, true),
        ("light", Appearance::Light, false),
        ("light, increase contrast", Appearance::Light, true),
    ];

    /// Runs `check` in each palette, with the test thread's theme back to
    /// the default afterwards.
    fn in_each_palette(mut check: impl FnMut(&str)) {
        for (name, appearance, increase_contrast) in PALETTES {
            set(Theme {
                appearance,
                increase_contrast,
                zoom: 1.0,
            });
            check(name);
        }
        set(Theme::default());
    }

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

    /// Text, held to 4.5:1 on every surface.
    fn text_tokens() -> Vec<(&'static str, Hsla)> {
        vec![
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
        ]
    }

    /// Control edges, status dots and rings, held to 3:1 (WCAG 1.4.11).
    fn control_tokens() -> Vec<(&'static str, Hsla)> {
        vec![
            ("border_control", border_control()),
            ("accent_border", accent_border()),
            ("focus_ring", focus_ring()),
            ("focus_ring_on_accent", focus_ring_on_accent()),
            ("status_ok", status_ok()),
            ("status_error", status_error()),
            ("status_warning", status_warning()),
            ("status_suspended", status_suspended()),
        ]
    }

    /// Text drawn on a tint: chips, tags, the selected tile. The tint is
    /// laid over the panel first.
    fn tinted_pairs() -> Vec<(&'static str, Hsla, Hsla)> {
        vec![
            ("accent on accent_soft", accent(), accent_soft()),
            (
                "accent_bright on accent_soft",
                accent_bright(),
                accent_soft(),
            ),
            ("text_muted on accent_soft", text_muted(), accent_soft()),
            ("text_primary on accent_soft", text_primary(), accent_soft()),
            ("danger_text on danger_soft", danger_text(), danger_soft()),
            (
                "success_text on success_soft",
                success_text(),
                success_soft(),
            ),
            ("info_text on info_soft", info_text(), info_soft()),
            ("warn_text on warn_soft", warn_text(), warn_soft()),
            (
                "text_primary on log_selected",
                text_primary(),
                log_selected(),
            ),
            ("text_muted on log_warn_row", text_muted(), log_warn_row()),
            ("text_muted on log_error_row", text_muted(), log_error_row()),
        ]
    }

    /// `color` over the opaque `under`, as the renderer blends it.
    fn over(color: Hsla, under: [f32; 3]) -> [f32; 3] {
        let top = Rgba::from(color);
        let a = top.a;
        [
            top.r * a + under[0] * (1.0 - a),
            top.g * a + under[1] * (1.0 - a),
            top.b * a + under[2] * (1.0 - a),
        ]
    }

    fn opaque(color: Hsla) -> [f32; 3] {
        let color = Rgba::from(color);
        [color.r, color.g, color.b]
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

    fn ratio(top: [f32; 3], under: [f32; 3]) -> f32 {
        let a = luminance(top);
        let b = luminance(under);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    fn contrast(color: Hsla, under: Hsla) -> f32 {
        ratio(over(color, opaque(under)), opaque(under))
    }

    /// `color` on `tint` laid over the panel.
    fn contrast_on_tint(color: Hsla, tint: Hsla) -> f32 {
        let under = over(tint, opaque(bg_panel()));
        ratio(over(color, under), under)
    }

    fn below(tokens: &[(&str, Hsla)], minimum: f32, palette: &str, failing: &mut Vec<String>) {
        for (name, color) in tokens {
            for (surface, under) in surfaces() {
                let ratio = contrast(*color, under);
                if ratio < minimum {
                    failing.push(format!("{palette}: {name} on {surface}: {ratio:.2}"));
                }
            }
        }
    }

    #[test]
    fn contrast_matches_known_values() {
        // WCAG's own extremes, and the design's muted text on black.
        assert!((contrast(hex(0xffffff), hex(0x000000)) - 21.0).abs() < 0.01);
        assert!((contrast(hex(0x8c857a), hex(0x000000)) - 5.75).abs() < 0.01);
    }

    #[test]
    fn text_meets_4_5_to_1() {
        let mut failing = Vec::new();
        in_each_palette(|palette| {
            below(&text_tokens(), 4.5, palette, &mut failing);
            // Dimmed text is the ramp a step down, never fainter.
            dimmed(|| below(&text_tokens(), 4.5, palette, &mut failing));
            for (pair, color, tint) in tinted_pairs() {
                let ratio = contrast_on_tint(color, tint);
                if ratio < 4.5 {
                    failing.push(format!("{palette}: {pair}: {ratio:.2}"));
                }
            }
            let on_accent = contrast(accent_on(), accent());
            if on_accent < 4.5 {
                failing.push(format!("{palette}: accent_on on accent: {on_accent:.2}"));
            }
        });
        assert!(failing.is_empty(), "below 4.5:1: {failing:#?}");
    }

    #[test]
    fn controls_meet_3_to_1() {
        let mut failing = Vec::new();
        in_each_palette(|palette| {
            below(&control_tokens(), 3.0, palette, &mut failing);
            // The ring on a primary button shows against its own fill too.
            let ring = contrast(focus_ring_on_accent(), accent());
            if ring < 2.0 {
                failing.push(format!(
                    "{palette}: focus_ring_on_accent on accent: {ring:.2}"
                ));
            }
        });
        assert!(failing.is_empty(), "below 3:1: {failing:#?}");
    }

    /// With Increase Contrast every edge and quiet text is at least as
    /// strong as without, and no text is see-through.
    #[test]
    fn increase_contrast_only_strengthens() {
        let mut failing = Vec::new();
        for appearance in [Appearance::Dark, Appearance::Light] {
            let read = |increase_contrast| {
                set(Theme {
                    appearance,
                    increase_contrast,
                    zoom: 1.0,
                });
                let mut tokens = text_tokens();
                tokens.extend(control_tokens());
                tokens.extend([
                    ("border_faint", border_faint()),
                    ("border_subtle", border_subtle()),
                    ("border_default", border_default()),
                    ("border_strong", border_strong()),
                ]);
                let panel = bg_panel();
                tokens
                    .into_iter()
                    .map(|(name, color)| (name, color.a, contrast(color, panel)))
                    .collect::<Vec<_>>()
            };
            let standard = read(false);
            let increased = read(true);
            for ((name, _, before), (_, alpha, after)) in standard.iter().zip(&increased) {
                if after + 0.005 < *before {
                    failing.push(format!("{appearance:?} {name}: {before:.2} -> {after:.2}"));
                }
                if name.starts_with("text_") && *alpha < 1.0 {
                    failing.push(format!("{appearance:?} {name} is translucent"));
                }
            }
        }
        set(Theme::default());
        assert!(failing.is_empty(), "{failing:#?}");
    }

    /// Every pair's ratio, so that changing a token changes this file and
    /// the change is read in review. `UPDATE_CONTRAST_SNAP=1 cargo test -p
    /// pwrroute-app theme::` rewrites it.
    #[test]
    fn contrast_snapshot() {
        let mut table = String::from(
            "# Contrast of every token pair, in each palette. Written by\n\
             # theme::tests::contrast_snapshot: do not edit by hand.\n",
        );
        in_each_palette(|palette| {
            let names: Vec<_> = surfaces().into_iter().map(|(name, _)| name).collect();
            let _ = writeln!(table, "\n[{palette}]\n{:<28} {}", "", names.join(" "));
            let mut tokens = text_tokens();
            tokens.extend(control_tokens());
            for (name, color) in tokens {
                let _ = write!(table, "{name:<28}");
                for ((surface, under), width) in surfaces().into_iter().zip(&names) {
                    debug_assert_eq!(surface, *width);
                    let _ = write!(table, " {:>w$.2}", contrast(color, under), w = width.len());
                }
                table.push('\n');
            }
            for (pair, color, tint) in tinted_pairs() {
                let _ = writeln!(table, "{pair:<36} {:.2}", contrast_on_tint(color, tint));
            }
            let _ = writeln!(
                table,
                "{:<36} {:.2}",
                "accent_on on accent",
                contrast(accent_on(), accent())
            );
        });
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/contrast.snap");
        if std::env::var_os("UPDATE_CONTRAST_SNAP").is_some() {
            std::fs::write(path, &table).expect("write contrast.snap");
            return;
        }
        let pinned = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            pinned == table,
            "a token's contrast changed; check it by eye, then rerun with \
             UPDATE_CONTRAST_SNAP=1 and commit src/contrast.snap:\n{table}"
        );
    }

    #[test]
    fn dimmed_steps_down_and_restores() {
        let primary = text_primary();
        let inner = dimmed(|| (text_primary(), dimmed(text_secondary), is_dimmed()));
        assert_eq!(inner.0, ink_secondary());
        assert_eq!(inner.1, ink_muted());
        assert!(inner.2);
        assert!(!is_dimmed());
        assert_eq!(text_primary(), primary);
        assert_eq!(dimmed(text_muted), text_muted());
    }

    #[test]
    fn zoom_steps_and_scales() {
        assert_eq!(snap_zoom(0.0), 1.0);
        assert_eq!(snap_zoom(1.3), 1.25);
        assert_eq!(snap_zoom(40.0), 2.0);
        assert_eq!(zoom_step(1.0, 1), 1.1);
        assert_eq!(zoom_step(1.0, -1), 1.0);
        assert_eq!(zoom_step(2.0, 1), 2.0);
        assert_eq!(zoom_step(1.5, -1), 1.25);
        set_zoom(1.5);
        assert_eq!(px(10.0), gpui_kit::px(15.0));
        assert_eq!(css(gpui_kit::px(15.0)), 10.0);
        set(Theme::default());
        assert_eq!(px(10.0), gpui_kit::px(10.0));
    }
}
