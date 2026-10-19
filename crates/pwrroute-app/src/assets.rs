//! Everything the window draws that is not code: the family's fonts and the
//! SVG icons, all embedded in the binary.
//!
//! The fonts are Geist and Geist Mono (SIL OFL 1.1, see
//! `assets/fonts/OFL-Geist.txt`), the Latin subset of the static cuts.
//! Glyphs outside it (arrows, ⋯, ▾, ⌘) come from the system fallback font.

use std::borrow::Cow;

use gpui_kit::{App, AssetSource, SharedString};

/// The icons, by the path views pass to `svg().path(..)`.
const ICONS: &[(&str, &[u8])] = &[
    (
        "pwrroute/mark.svg",
        include_bytes!("../assets/icons/mark.svg"),
    ),
    (
        "pwrroute/rules.svg",
        include_bytes!("../assets/icons/rules.svg"),
    ),
    (
        "pwrroute/test.svg",
        include_bytes!("../assets/icons/test.svg"),
    ),
    (
        "pwrroute/activity.svg",
        include_bytes!("../assets/icons/activity.svg"),
    ),
    (
        "pwrroute/settings.svg",
        include_bytes!("../assets/icons/settings.svg"),
    ),
    (
        "pwrroute/plus.svg",
        include_bytes!("../assets/icons/plus.svg"),
    ),
    (
        "pwrroute/search.svg",
        include_bytes!("../assets/icons/search.svg"),
    ),
    (
        "pwrroute/code.svg",
        include_bytes!("../assets/icons/code.svg"),
    ),
    (
        "pwrroute/grip.svg",
        include_bytes!("../assets/icons/grip.svg"),
    ),
];

const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/GeistSans-400.otf"),
    include_bytes!("../assets/fonts/GeistSans-500.otf"),
    include_bytes!("../assets/fonts/GeistSans-600.otf"),
    include_bytes!("../assets/fonts/GeistSans-700.otf"),
    include_bytes!("../assets/fonts/GeistMono-400.ttf"),
    include_bytes!("../assets/fonts/GeistMono-500.ttf"),
    include_bytes!("../assets/fonts/GeistMono-600.ttf"),
];

/// PwrRoute's icons first, then gpui-kit's bundled Lucide set.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        match ICONS.iter().find(|(name, _)| *name == path) {
            Some((_, bytes)) => Ok(Some(Cow::Borrowed(*bytes))),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut names: Vec<SharedString> = ICONS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect();
        names.extend(gpui_kit::assets::Assets.list(path)?);
        Ok(names)
    }
}

/// Register the embedded fonts with GPUI's text system.
///
/// A failure is not fatal: GPUI falls back to the system font, and the window
/// is still usable.
pub fn load_fonts(cx: &App) {
    let fonts = FONTS.iter().map(|bytes| Cow::Borrowed(*bytes)).collect();
    if let Err(error) = cx.text_system().add_fonts(fonts) {
        eprintln!("pwrroute: could not load Geist, using the system font: {error}");
    }
}
