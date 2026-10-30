//! Each app's own icon, as Finder or Explorer shows it, for the `.bi` badges
//! (`widgets::badge`). macOS and Windows draw it through `pwrroute_os`;
//! Linux has no implementation yet, so its badges keep the letters. Windows
//! finds an app only by the `.exe` the scan placed it at.
//!
//! An icon is read lazily, through GPUI's asset cache on its background
//! executor: a badge asks for its app at the pixel size it draws while it
//! lays out, and the view is drawn again once the icon has loaded. Until
//! then, and for good when there is no icon (the app is not installed, the
//! platform has no implementation, or [`install`] never ran: tests, and the
//! `screens` example unless asked), the badge draws its letters. Nothing
//! here reads the disk on the UI thread.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::{
    App, Asset, Global, Image, ImageCacheError, ImageFormat, ImageSource, RenderImage, SharedString,
};

use crate::browser_scan;

/// Whether `pwrroute_os` can find and draw an app's icon here.
const HAS_ICONS: bool = cfg!(any(target_os = "macos", windows));

/// What a badge asks for: the app as the rule or the scan names it, where
/// the scan found it (when it did), and the square's size in pixels.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IconSource {
    app: SharedString,
    bundle: Option<PathBuf>,
    pixels: u32,
}

/// Every icon asked for so far, so [`forget`] can drop them, and where the
/// scan placed each app once the scan has run, so the key stays the same
/// from then on.
#[derive(Default)]
struct AppIcons {
    asked: HashSet<IconSource>,
    bundles: HashMap<SharedString, Option<PathBuf>>,
}

impl Global for AppIcons {}

/// Turns icons on for this process. Without it every badge is letters. On
/// Linux there is no implementation yet, so badges stay letters from the
/// first frame and nothing is loaded.
pub fn install(cx: &mut App) {
    cx.set_global(AppIcons::default());
}

/// Drops every icon read so far, found or not, so an app installed, moved
/// or updated since is read again: Settings › Browsers' Rescan.
pub fn forget(cx: &mut App) {
    if !cx.has_global::<AppIcons>() {
        return;
    }
    let icons = std::mem::take(cx.global_mut::<AppIcons>());
    let loaded: Vec<Arc<RenderImage>> = icons
        .asked
        .iter()
        .filter_map(|source| cx.fetch_asset::<AppIcon>(source)?.ok())
        .collect();
    for source in &icons.asked {
        cx.remove_asset::<AppIcon>(source);
    }
    // Their textures stay in each window's atlas until dropped.
    for window in cx.windows() {
        let _ = window.update(cx, |_, window, _| {
            for image in &loaded {
                let _ = window.drop_image(image.clone());
            }
        });
    }
    cx.refresh_windows();
}

/// The image of `app`'s icon, drawn `points` square. `app` is what
/// `widgets::badge_for` was given: a name, a bundle id or a path.
pub fn source(app: SharedString, points: f32) -> ImageSource {
    ImageSource::Custom(Arc::new(move |window, cx| {
        if !HAS_ICONS || !cx.has_global::<AppIcons>() {
            return Some(Err(ImageCacheError::Asset("app icons are off".into())));
        }
        let pixels = (points * window.scale_factor()).ceil() as u32;
        let bundle = match cx.global::<AppIcons>().bundles.get(&app) {
            Some(bundle) => bundle.clone(),
            // Before the scan runs, LaunchServices finds the app (Windows
            // only a path); once it has, its answer is kept so a later
            // frame keeps the same key.
            None if !browser_scan::has_run(cx) => None,
            None => {
                let bundle = browser_scan::location_of(&app, cx);
                cx.global_mut::<AppIcons>()
                    .bundles
                    .insert(app.clone(), bundle.clone());
                bundle
            }
        };
        let source = IconSource {
            app: app.clone(),
            bundle,
            pixels,
        };
        if !cx.global::<AppIcons>().asked.contains(&source) {
            cx.global_mut::<AppIcons>().asked.insert(source.clone());
        }
        window.use_asset::<AppIcon>(&source, cx)
    }))
}

/// Finds the app and renders its icon, on GPUI's background executor.
enum AppIcon {}

impl Asset for AppIcon {
    type Source = IconSource;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        let renderer = cx.svg_renderer();
        async move {
            let bundle = source
                .bundle
                .or_else(|| pwrroute_os::app_bundle(&source.app))
                .ok_or_else(|| ImageCacheError::Asset("no installed app".into()))?;
            let png = pwrroute_os::app_icon_png(&bundle, source.pixels)
                .ok_or_else(|| ImageCacheError::Asset("no icon".into()))?;
            Image::from_bytes(ImageFormat::Png, png)
                .to_image_data(renderer)
                .map_err(|error| ImageCacheError::Other(Arc::new(error)))
        }
    }
}
