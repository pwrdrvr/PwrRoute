//! Design tokens and the text primitives every view draws with.
//!
//! The colors are `design/pwrroute.css`'s, which mirror the PwrSuite family
//! palette: Tangerine, dark and light, each standard and with the OS's
//! Increase Contrast on (design: Contrast and Scale). PWR-23 adds PwrAgent's
//! other color themes (`design/pwrroute-themes.css`; design: Appearance and
//! Themes): the reader picks a [`DarkTheme`] and a [`LightTheme`], and the
//! scheme decides which draws. Each theme has its own Increase Contrast
//! variant. The palettes are data (`theme_palettes.rs`); every token is a
//! function that reads the palette in force, so a view never names one.
//! Sizes stay in CSS pixels so a view reads like the design it was drawn
//! from; [`px`] multiplies them by the zoom.
//!
//! Where the design falls short of WCAG 2.2 AA a token says so and departs
//! from it (`border_control`, `accent_border`). The tests at the bottom
//! hold every text token to 4.5:1 and every control edge, status dot and
//! focus ring to 3:1 on every surface, in every theme's every palette, and
//! pin each pair's ratio in `contrast.snap`, so a change to a token shows
//! in review.
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

/// The scheme being drawn: what [`ThemeChoice`] resolves to. Dark until
/// `appearance.rs` reads the choice and the OS; the `screens` example
/// draws light with `--light`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    #[default]
    Dark,
    Light,
}

/// Settings › General › Theme (PWR-22; design: Appearance and Themes 1a),
/// as `settings.json`'s `appearance.theme` keeps it. System follows the
/// OS while the app runs (`appearance.rs`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ThemeChoice {
    #[default]
    System,
    Dark,
    Light,
}

impl ThemeChoice {
    pub const ALL: [Self; 3] = [Self::System, Self::Dark, Self::Light];

    /// The scheme drawn when the OS uses `os`.
    pub fn resolve(self, os: Appearance) -> Appearance {
        match self {
            Self::System => os,
            Self::Dark => Appearance::Dark,
            Self::Light => Appearance::Light,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }

    /// The word in `settings.json`.
    pub fn key(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    /// `key`'s inverse; `None` for a word it does not know.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|choice| choice.key() == key)
    }
}

/// What Settings says about a color theme (PWR-23; design: Appearance and
/// Themes 1c-1e).
#[derive(Debug)]
pub struct ThemeInfo {
    /// The word in `settings.json`: PwrAgent's id, so the family agrees.
    pub key: &'static str,
    pub name: &'static str,
    /// The list's hint beside the name: "default", "charcoal", "dark only".
    pub hint: &'static str,
    /// A borrowed palette's credit, under its row. `None` for the Pwr
    /// family's own.
    pub credit: Option<&'static str>,
}

impl ThemeInfo {
    const fn new(
        key: &'static str,
        name: &'static str,
        hint: &'static str,
        credit: Option<&'static str>,
    ) -> Self {
        Self {
            key,
            name,
            hint,
            credit,
        }
    }
}

const CATPPUCCIN: Option<&str> = Some("Palette by Catppuccin (MIT).");
const SOLARIZED: Option<&str> = Some("Solarized palette by Ethan Schoonover (MIT).");

/// Settings › General's Dark theme: the colors whenever PwrRoute is dark,
/// as `settings.json`'s `appearance.darkTheme` keeps it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DarkTheme {
    #[default]
    Tangerine,
    Gray,
    Blue,
    Phosphor,
    CatppuccinMocha,
    SolarizedDark,
}

impl DarkTheme {
    /// The list's order: the Pwr family's, then the community palettes.
    pub const ALL: [Self; 6] = [
        Self::Tangerine,
        Self::Gray,
        Self::Blue,
        Self::Phosphor,
        Self::CatppuccinMocha,
        Self::SolarizedDark,
    ];

    pub fn info(self) -> &'static ThemeInfo {
        use ThemeInfo as T;
        const TANGERINE: ThemeInfo = T::new("tangerine-dark", "Tangerine", "default", None);
        const GRAY: ThemeInfo = T::new("gray-dark", "Gray", "charcoal", None);
        const BLUE: ThemeInfo = T::new("blue-dark", "Blue", "navy", None);
        const PHOSPHOR: ThemeInfo = T::new("phosphor-dark", "Phosphor", "dark only", None);
        const MOCHA: ThemeInfo = T::new("catppuccin-mocha", "Catppuccin Mocha", "", CATPPUCCIN);
        const SOLARIZED_DARK: ThemeInfo = T::new("solarized-dark", "Solarized Dark", "", SOLARIZED);
        match self {
            Self::Tangerine => &TANGERINE,
            Self::Gray => &GRAY,
            Self::Blue => &BLUE,
            Self::Phosphor => &PHOSPHOR,
            Self::CatppuccinMocha => &MOCHA,
            Self::SolarizedDark => &SOLARIZED_DARK,
        }
    }

    /// The word in `settings.json`.
    pub fn key(self) -> &'static str {
        self.info().key
    }

    /// `key`'s inverse; `None` for a word it does not know.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|theme| theme.key() == key)
    }

    /// The light theme drawn from the same palette. Phosphor has none.
    pub fn pair(self) -> Option<LightTheme> {
        match self {
            Self::Tangerine => Some(LightTheme::Tangerine),
            Self::Gray => Some(LightTheme::Gray),
            Self::Blue => Some(LightTheme::Blue),
            Self::Phosphor => None,
            Self::CatppuccinMocha => Some(LightTheme::CatppuccinLatte),
            Self::SolarizedDark => Some(LightTheme::SolarizedLight),
        }
    }

    pub fn palette(self, increase_contrast: bool) -> &'static Palette {
        use palettes::*;
        match (self, increase_contrast) {
            (Self::Tangerine, false) => &TANGERINE_DARK,
            (Self::Tangerine, true) => &TANGERINE_DARK_HC,
            (Self::Gray, false) => &GRAY_DARK,
            (Self::Gray, true) => &GRAY_DARK_HC,
            (Self::Blue, false) => &BLUE_DARK,
            (Self::Blue, true) => &BLUE_DARK_HC,
            (Self::Phosphor, false) => &PHOSPHOR_DARK,
            (Self::Phosphor, true) => &PHOSPHOR_DARK_HC,
            (Self::CatppuccinMocha, false) => &CATPPUCCIN_MOCHA,
            (Self::CatppuccinMocha, true) => &CATPPUCCIN_MOCHA_HC,
            (Self::SolarizedDark, false) => &SOLARIZED_DARK,
            (Self::SolarizedDark, true) => &SOLARIZED_DARK_HC,
        }
    }
}

/// Settings › General's Light theme: the colors whenever PwrRoute is
/// light, as `settings.json`'s `appearance.lightTheme` keeps it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LightTheme {
    #[default]
    Tangerine,
    Gray,
    Blue,
    CatppuccinLatte,
    SolarizedLight,
}

impl LightTheme {
    /// The list's order: the Pwr family's, then the community palettes.
    pub const ALL: [Self; 5] = [
        Self::Tangerine,
        Self::Gray,
        Self::Blue,
        Self::CatppuccinLatte,
        Self::SolarizedLight,
    ];

    pub fn info(self) -> &'static ThemeInfo {
        use ThemeInfo as T;
        const TANGERINE: ThemeInfo = T::new("tangerine-light", "Tangerine", "default", None);
        const GRAY: ThemeInfo = T::new("gray-light", "Gray", "light gray", None);
        const BLUE: ThemeInfo = T::new("blue-light", "Blue", "pale blue", None);
        const LATTE: ThemeInfo = T::new("catppuccin-latte", "Catppuccin Latte", "", CATPPUCCIN);
        const SOLARIZED_LIGHT: ThemeInfo =
            T::new("solarized-light", "Solarized Light", "", SOLARIZED);
        match self {
            Self::Tangerine => &TANGERINE,
            Self::Gray => &GRAY,
            Self::Blue => &BLUE,
            Self::CatppuccinLatte => &LATTE,
            Self::SolarizedLight => &SOLARIZED_LIGHT,
        }
    }

    /// The word in `settings.json`.
    pub fn key(self) -> &'static str {
        self.info().key
    }

    /// `key`'s inverse; `None` for a word it does not know.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|theme| theme.key() == key)
    }

    /// The dark theme drawn from the same palette.
    pub fn pair(self) -> DarkTheme {
        match self {
            Self::Tangerine => DarkTheme::Tangerine,
            Self::Gray => DarkTheme::Gray,
            Self::Blue => DarkTheme::Blue,
            Self::CatppuccinLatte => DarkTheme::CatppuccinMocha,
            Self::SolarizedLight => DarkTheme::SolarizedDark,
        }
    }

    pub fn palette(self, increase_contrast: bool) -> &'static Palette {
        use palettes::*;
        match (self, increase_contrast) {
            (Self::Tangerine, false) => &TANGERINE_LIGHT,
            (Self::Tangerine, true) => &TANGERINE_LIGHT_HC,
            (Self::Gray, false) => &GRAY_LIGHT,
            (Self::Gray, true) => &GRAY_LIGHT_HC,
            (Self::Blue, false) => &BLUE_LIGHT,
            (Self::Blue, true) => &BLUE_LIGHT_HC,
            (Self::CatppuccinLatte, false) => &CATPPUCCIN_LATTE,
            (Self::CatppuccinLatte, true) => &CATPPUCCIN_LATTE_HC,
            (Self::SolarizedLight, false) => &SOLARIZED_LIGHT,
            (Self::SolarizedLight, true) => &SOLARIZED_LIGHT_HC,
        }
    }
}

/// What every token and size reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub appearance: Appearance,
    /// Drawn while `appearance` is dark.
    pub dark_theme: DarkTheme,
    /// Drawn while `appearance` is light.
    pub light_theme: LightTheme,
    /// The OS's Increase Contrast (macOS) or high contrast (Windows):
    /// stronger edges, opaque text one step brighter, solid accent edges,
    /// in the theme's own colors.
    pub increase_contrast: bool,
    /// View › Zoom: every [`px`] is multiplied by it. One of [`ZOOM_STEPS`].
    pub zoom: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            appearance: Appearance::Dark,
            dark_theme: DarkTheme::Tangerine,
            light_theme: LightTheme::Tangerine,
            increase_contrast: false,
            zoom: 1.0,
        }
    }
}

/// Puts the default theme back when dropped, even when a test fails.
#[cfg(test)]
pub struct DefaultOnDrop;

#[cfg(test)]
impl Drop for DefaultOnDrop {
    fn drop(&mut self) {
        set(Theme::default());
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

pub fn set_dark_theme(dark_theme: DarkTheme) {
    set(Theme {
        dark_theme,
        ..current()
    });
}

pub fn set_light_theme(light_theme: LightTheme) {
    set(Theme {
        light_theme,
        ..current()
    });
}

/// The palette in force: the scheme's theme, with Increase Contrast or not.
pub fn palette() -> &'static Palette {
    let theme = current();
    match theme.appearance {
        Appearance::Dark => theme.dark_theme.palette(theme.increase_contrast),
        Appearance::Light => theme.light_theme.palette(theme.increase_contrast),
    }
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

#[cfg(test)]
fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

fn hex_alpha(value: u32, alpha: f32) -> Hsla {
    let mut color: Hsla = rgb(value).into();
    color.a = alpha;
    color
}

/// A token's color: sRGB and its alpha, as the design writes it. `Hsla`
/// construction is not `const`; this is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    rgb: u32,
    alpha: f32,
}

impl Color {
    fn hsla(self) -> Hsla {
        hex_alpha(self.rgb, self.alpha)
    }
}

/// Declares [`Palette`], a color for every token, and one function per
/// token that reads the palette in force, so a view never names a theme.
macro_rules! palette {
    ($($(#[$doc:meta])* $vis:vis $name:ident;)*) => {
        /// One palette: a color for every token. Each theme has two, standard
        /// and with Increase Contrast (`theme_palettes.rs`).
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct Palette {
            $($name: Color,)*
        }

        $(
            $(#[$doc])*
            $vis fn $name() -> Hsla {
                palette().$name.hsla()
            }
        )*

        impl Palette {
            /// A theme's swatch in Settings: canvas, sidebar, accent, text.
            pub fn swatch(&self) -> [Hsla; 4] {
                [
                    self.bg_app.hsla(),
                    self.bg_sidebar.hsla(),
                    self.accent.hsla(),
                    self.ink_primary.hsla(),
                ]
            }

            /// Every token by name.
            #[cfg(test)]
            fn tokens(&self) -> Vec<(&'static str, Color)> {
                vec![$((stringify!($name), self.$name),)*]
            }
        }
    };
}

#[path = "theme_palettes.rs"]
mod palettes;

// Where a token's doc names a value, it is Tangerine's.
palette! {
    pub bg_app;
    pub bg_titlebar;
    pub bg_sidebar;
    pub bg_panel;
    pub bg_input;
    pub bg_elevated;
    /// Row hover.
    pub bg_hover;
    pub bg_hover_strong;
    /// Selected row and rail item.
    pub bg_row_active;
    /// Dividers. With Increase Contrast every edge comes up to where a
    /// card, a divider and a field read without the fill behind them.
    pub border_faint;
    pub border_subtle;
    pub border_default;
    pub border_strong;
    /// The edge of a control that is known by its edge alone: a text
    /// field's box, a switch's track when off, a radio or a checkbox.
    /// The design draws those in `--border-default` (1.4:1 on the
    /// panels); WCAG 1.4.11 asks 3:1, which this is on every surface
    /// (`tests::controls_meet_3_to_1`). Cards, dividers and frames stay
    /// on the fainter borders: they are decoration.
    pub border_control;
    ink_primary;
    ink_secondary;
    ink_muted;
    /// Opaque with Increase Contrast: no text is see-through.
    ink_subtle;
    /// Light's accent ramp runs the other way: brighter is darker on
    /// white. The design's `#b74c00` is 4.1:1 on `bg_hover_strong`.
    pub accent;
    pub accent_bright;
    pub accent_on;
    pub accent_soft;
    /// `.rr.new`, a line that is the rule in a source frame.
    pub accent_tint;
    /// A selected tile's or lit row's edge. The design's 0.42 is 2.3:1 on
    /// the panels, and for a tile the edge is what says it is chosen:
    /// 0.56 is 3:1 (WCAG 1.4.11) and the same orange. Solid with Increase
    /// Contrast.
    pub accent_border;
    /// The keyboard focus ring (`widgets::focus_visible`): 7:1 or more on
    /// every surface, solid in every palette.
    pub focus_ring;
    /// The ring on a control filled with the accent, where an accent ring
    /// would not show: Tangerine's bone is 18:1 on the panels around it.
    /// It is the button's border, over its fill, so where the theme's text
    /// color would melt into the fill (Gray dark, Phosphor, Catppuccin,
    /// Solarized, Gray and Blue light) it is the button's ink, `accent_on`,
    /// 4.5:1 or more against the fill it replaces.
    pub focus_ring_on_accent;
    pub danger_soft;
    pub danger_text;
    pub success_soft;
    pub success_text;
    pub info_soft;
    pub info_text;
    pub status_ok;
    pub status_error;
    pub status_warning;
    pub warn_soft;
    /// The design's light block kept dark's `#e8c890` (1.6:1 on white).
    pub warn_text;
    pub transparent;
    /// Windows' own close-button hover, under a white glyph: the caption
    /// buttons are the OS's, so they keep its colours.
    pub caption_close;
    /// `.strip-chip.off`'s dot: not routing.
    pub status_suspended;
    /// The menu bar dropdown's fill: `.tray`'s rgba(16,16,16,0.98), made
    /// opaque. In the transparent popup window the 0.98 fill let the window
    /// behind it show through visibly.
    pub bg_popover;
    /// Help › Logs (PwrAgent's "Log Viewer Workflows UX Review"): a line's
    /// target, a selected line, and the warning and error rows' washes.
    pub log_target;
    pub log_selected;
    pub log_warn_row;
    pub log_error_row;
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

    /// Every theme in its scheme, standard. Tangerine's are plain "dark"
    /// and "light", as they were before there were other themes.
    fn themes() -> Vec<(String, Theme)> {
        let dark = DarkTheme::ALL.into_iter().map(|dark_theme| {
            let name = match dark_theme {
                DarkTheme::Tangerine => "dark",
                _ => dark_theme.key(),
            };
            let theme = Theme {
                appearance: Appearance::Dark,
                dark_theme,
                ..Theme::default()
            };
            (name.to_owned(), theme)
        });
        let light = LightTheme::ALL.into_iter().map(|light_theme| {
            let name = match light_theme {
                LightTheme::Tangerine => "light",
                _ => light_theme.key(),
            };
            let theme = Theme {
                appearance: Appearance::Light,
                light_theme,
                ..Theme::default()
            };
            (name.to_owned(), theme)
        });
        dark.chain(light).collect()
    }

    /// Every theme, then the same with Increase Contrast.
    fn palettes() -> Vec<(String, Theme)> {
        themes()
            .into_iter()
            .flat_map(|(name, theme)| {
                let increased = Theme {
                    increase_contrast: true,
                    ..theme
                };
                [
                    (name.clone(), theme),
                    (format!("{name}, increase contrast"), increased),
                ]
            })
            .collect()
    }

    /// Runs `check` in each palette, with the test thread's theme back to
    /// the default afterwards.
    fn in_each_palette(mut check: impl FnMut(&str)) {
        let _restore = DefaultOnDrop;
        for (name, theme) in palettes() {
            set(theme);
            check(&name);
        }
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
            let (on_accent, controls): (Vec<_>, Vec<_>) = control_tokens()
                .into_iter()
                .partition(|(name, _)| *name == "focus_ring_on_accent");
            below(&controls, 3.0, palette, &mut failing);
            // The ring on a primary button is its border, over its fill: it
            // shows against the fill it replaces (2:1 at least, as
            // Tangerine's bone on orange), and clears 3:1 either there (dark
            // ink on a light accent) or on every surface around the button.
            let ring = contrast(focus_ring_on_accent(), accent());
            let mut around = Vec::new();
            below(&on_accent, 3.0, palette, &mut around);
            if ring < 2.0 || (ring < 3.0 && !around.is_empty()) {
                failing.push(format!(
                    "{palette}: focus_ring_on_accent on accent: {ring:.2}"
                ));
                failing.extend(around);
            }
        });
        assert!(failing.is_empty(), "below 3:1: {failing:#?}");
    }

    /// With Increase Contrast every edge and quiet text is at least as
    /// strong as without, and no text is see-through.
    #[test]
    fn increase_contrast_only_strengthens() {
        let _restore = DefaultOnDrop;
        let mut failing = Vec::new();
        for (theme_name, theme) in themes() {
            let read = |increase_contrast| {
                set(Theme {
                    increase_contrast,
                    ..theme
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
                    .map(|(name, color)| {
                        // The ring on a primary button shows against its
                        // fill or the surface around it, whichever is more.
                        let mut ratio = contrast(color, panel);
                        if name == "focus_ring_on_accent" {
                            ratio = ratio.max(contrast(color, accent()));
                        }
                        (name, color.a, ratio)
                    })
                    .collect::<Vec<_>>()
            };
            let standard = read(false);
            let increased = read(true);
            for ((name, _, before), (_, alpha, after)) in standard.iter().zip(&increased) {
                if after + 0.005 < *before {
                    failing.push(format!("{theme_name} {name}: {before:.2} -> {after:.2}"));
                }
                if name.starts_with("text_") && *alpha < 1.0 {
                    failing.push(format!("{theme_name} {name} is translucent"));
                }
            }
        }
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

    /// `design/pwrroute-themes.css` draws the same colors as
    /// `theme_palettes.rs`, token for token, in every theme but Tangerine
    /// (whose design values `pwrroute.css` keeps as drawn).
    #[test]
    fn the_design_draws_the_same_themes() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../design/pwrroute-themes.css"
        );
        let css = std::fs::read_to_string(path).expect("read design/pwrroute-themes.css");
        let css = without_comments(&css);
        let mut palettes: Vec<(String, &Palette)> = Vec::new();
        for theme in DarkTheme::ALL.into_iter().skip(1) {
            palettes.push((theme.key().to_owned(), theme.palette(false)));
            palettes.push((format!("{}.hc", theme.key()), theme.palette(true)));
        }
        for theme in LightTheme::ALL.into_iter().skip(1) {
            palettes.push((theme.key().to_owned(), theme.palette(false)));
            palettes.push((format!("{}.hc", theme.key()), theme.palette(true)));
        }
        let mut failing = Vec::new();
        for (selector, palette) in palettes {
            let Some(block) = css_block(&css, &format!(".th-{selector}")) else {
                failing.push(format!("no .th-{selector} block"));
                continue;
            };
            let mut compared = 0;
            for (token, color) in palette.tokens() {
                let Some(variable) = css_variable(token) else {
                    continue;
                };
                compared += 1;
                match css_color(block, variable) {
                    Some(drawn) if same_color(drawn, color) => {}
                    drawn => failing.push(format!(
                        ".th-{selector} {variable}: design {drawn:?}, theme_palettes.rs {color:?}"
                    )),
                }
            }
            assert!(
                compared > 30,
                ".th-{selector}: only {compared} tokens compared"
            );
        }
        assert!(failing.is_empty(), "{failing:#?}");
    }

    /// The design's variable for a token; `None` for the tokens it does
    /// not draw.
    fn css_variable(token: &str) -> Option<&'static str> {
        Some(match token {
            "bg_app" => "--bg-app",
            "bg_titlebar" => "--bg-titlebar",
            "bg_sidebar" => "--bg-sidebar",
            "bg_panel" => "--bg-panel",
            "bg_input" => "--bg-input",
            "bg_elevated" => "--bg-panel-elevated",
            "bg_hover" => "--bg-panel-hover",
            "bg_hover_strong" => "--bg-hover-strong",
            "bg_row_active" => "--bg-row-active",
            "border_faint" => "--border-faint",
            "border_subtle" => "--border-subtle",
            "border_default" => "--border-default",
            "border_strong" => "--border-strong",
            "border_control" => "--border-control",
            "ink_primary" => "--text-primary",
            "ink_secondary" => "--text-secondary",
            "ink_muted" => "--text-muted",
            "ink_subtle" => "--text-subtle",
            "accent" => "--accent",
            "accent_bright" => "--accent-bright",
            "accent_on" => "--accent-on",
            "accent_soft" => "--accent-soft",
            "accent_tint" => "--accent-tint",
            "accent_border" => "--accent-border",
            "focus_ring" => "--focus-ring",
            "danger_soft" => "--danger-soft",
            "danger_text" => "--danger-text",
            "success_soft" => "--success-soft",
            "success_text" => "--success-text",
            "info_soft" => "--info-soft",
            "info_text" => "--info-text",
            "status_ok" => "--status-ok",
            "status_error" => "--status-error",
            "status_warning" => "--status-warning",
            "status_suspended" => "--status-suspended",
            "warn_soft" => "--warn-soft",
            "warn_text" => "--warn-text",
            "log_target" => "--log-target",
            _ => return None,
        })
    }

    fn without_comments(css: &str) -> String {
        let mut out = String::new();
        let mut rest = css;
        while let Some(start) = rest.find("/*") {
            out.push_str(&rest[..start]);
            rest = rest[start..]
                .find("*/")
                .map_or("", |end| &rest[start + end + 2..]);
        }
        out.push_str(rest);
        out
    }

    /// The declarations of the rule whose selector list starts with
    /// `selector` exactly (`.th-gray-dark {`, `.th-gray-dark.hc, ...`).
    fn css_block<'a>(css: &'a str, selector: &str) -> Option<&'a str> {
        css.split('}').find_map(|rule| {
            let (selectors, body) = rule.split_once('{')?;
            let first = selectors.trim().split(',').next()?.trim();
            (first == selector).then_some(body)
        })
    }

    /// `variable`'s value in `block`, as sRGB and alpha.
    fn css_color(block: &str, variable: &str) -> Option<(u32, f32)> {
        let value = block.split(';').find_map(|declaration| {
            let (name, value) = declaration.split_once(':')?;
            (name.trim() == variable).then(|| value.trim())
        })?;
        if let Some(hex) = value.strip_prefix('#') {
            return Some((u32::from_str_radix(hex, 16).ok()?, 1.0));
        }
        let inner = value.strip_prefix("rgba(")?.strip_suffix(')')?;
        let parts: Vec<f32> = inner
            .split(',')
            .map(|part| part.trim().parse().ok())
            .collect::<Option<_>>()?;
        let [r, g, b, alpha] = parts[..] else {
            return None;
        };
        Some((((r as u32) << 16) | ((g as u32) << 8) | b as u32, alpha))
    }

    fn same_color((rgb, alpha): (u32, f32), color: Color) -> bool {
        rgb == color.rgb && (alpha - color.alpha).abs() < 0.005
    }

    /// Each theme names its pair, and the pair names it back; Phosphor is
    /// dark only. Keys are PwrAgent's ids and read back.
    #[test]
    fn themes_pair_and_read_back() {
        for theme in DarkTheme::ALL {
            assert_eq!(DarkTheme::from_key(theme.key()), Some(theme));
            match theme.pair() {
                Some(light) => assert_eq!(light.pair(), theme),
                None => assert_eq!(theme, DarkTheme::Phosphor),
            }
        }
        for theme in LightTheme::ALL {
            assert_eq!(LightTheme::from_key(theme.key()), Some(theme));
            assert_eq!(theme.pair().pair(), Some(theme));
        }
        assert_eq!(DarkTheme::from_key("tangerine-light"), None);
        assert_eq!(LightTheme::from_key("phosphor-dark"), None);
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
    fn a_theme_choice_resolves_against_the_os() {
        for os in [Appearance::Dark, Appearance::Light] {
            assert_eq!(ThemeChoice::System.resolve(os), os);
            assert_eq!(ThemeChoice::Dark.resolve(os), Appearance::Dark);
            assert_eq!(ThemeChoice::Light.resolve(os), Appearance::Light);
        }
        for choice in ThemeChoice::ALL {
            assert_eq!(ThemeChoice::from_key(choice.key()), Some(choice));
        }
        assert_eq!(ThemeChoice::from_key("sepia"), None);
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
