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
    (
        "pwrroute/map.svg",
        include_bytes!("../assets/icons/map.svg"),
    ),
    (
        "pwrroute/wand.svg",
        include_bytes!("../assets/icons/wand.svg"),
    ),
    (
        "pwrroute/pause.svg",
        include_bytes!("../assets/icons/pause.svg"),
    ),
    (
        "pwrroute/forward.svg",
        include_bytes!("../assets/icons/forward.svg"),
    ),
    (
        "pwrroute/alert.svg",
        include_bytes!("../assets/icons/alert.svg"),
    ),
    (
        "pwrroute/mask.svg",
        include_bytes!("../assets/icons/mask.svg"),
    ),
    (
        "pwrroute/wrap.svg",
        include_bytes!("../assets/icons/wrap.svg"),
    ),
    (
        "pwrroute/bookmark.svg",
        include_bytes!("../assets/icons/bookmark.svg"),
    ),
    (
        "pwrroute/chevron-up.svg",
        include_bytes!("../assets/icons/chevron-up.svg"),
    ),
    (
        "pwrroute/chevron-down.svg",
        include_bytes!("../assets/icons/chevron-down.svg"),
    ),
    (
        "pwrroute/close.svg",
        include_bytes!("../assets/icons/close.svg"),
    ),
    // The rail's build block (design: Build Identity), drawn by
    // `chrome::build_block`.
    (
        "pwrroute/folder.svg",
        include_bytes!("../assets/icons/folder.svg"),
    ),
    (
        "pwrroute/branch.svg",
        include_bytes!("../assets/icons/branch.svg"),
    ),
    (
        "pwrroute/commit.svg",
        include_bytes!("../assets/icons/commit.svg"),
    ),
    (
        "pwrroute/copy.svg",
        include_bytes!("../assets/icons/copy.svg"),
    ),
    (
        "pwrroute/check.svg",
        include_bytes!("../assets/icons/check.svg"),
    ),
    // The Windows strip's caption buttons (design: Windows and Linux
    // Chrome 1a), drawn by `chrome::captions`.
    (
        "pwrroute/caption-minimize.svg",
        include_bytes!("../assets/icons/caption-minimize.svg"),
    ),
    (
        "pwrroute/caption-maximize.svg",
        include_bytes!("../assets/icons/caption-maximize.svg"),
    ),
    (
        "pwrroute/caption-restore.svg",
        include_bytes!("../assets/icons/caption-restore.svg"),
    ),
    (
        "pwrroute/caption-close.svg",
        include_bytes!("../assets/icons/caption-close.svg"),
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
        log::warn!("could not load Geist, using the system font: {error}");
    }
}
