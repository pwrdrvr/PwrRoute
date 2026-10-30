//! An app's icon as Finder draws it: `NSWorkspace`'s `iconForFile:` on the
//! bundle, drawn into a bitmap of the size asked for and saved as a PNG. Two
//! AppKit calls take a raw pointer or an untyped dictionary in objc2, so this module allows
//! `unsafe` for them, each with a `SAFETY` comment.
#![allow(unsafe_code)]

use std::path::{Path, PathBuf};

use objc2::AllocAnyThread as _;
use objc2::rc::autoreleasepool;
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSColorSpace, NSGraphicsContext, NSImageInterpolation,
    NSWorkspace,
};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

/// The `.app` bundle `app` names: a bundle id (`com.google.Chrome`) or an
/// app's name (`Google Chrome`), as LaunchServices finds it, or a path,
/// which may be any file (Finder draws an icon for each). `None` when no
/// installed app or file answers to it.
pub fn bundle_path(app: &str) -> Option<PathBuf> {
    let app = app.trim();
    if app.is_empty() {
        return None;
    }
    if app.starts_with('/') {
        let path = PathBuf::from(app);
        return path.exists().then_some(path);
    }
    autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let name = NSString::from_str(app);
        if looks_like_bundle_id(app)
            && let Some(path) = workspace
                .URLForApplicationWithBundleIdentifier(&name)
                .and_then(|url| url.path())
        {
            return Some(PathBuf::from(path.to_string()));
        }
        // By name, as `open -a` and Finicky's launcher find an app. There is
        // no replacement that takes a name.
        #[allow(deprecated)]
        let path = workspace.fullPathForApplication(&name)?;
        Some(PathBuf::from(path.to_string()))
    })
}

/// `com.google.Chrome`, not `Google Chrome` or `Arc`.
fn looks_like_bundle_id(app: &str) -> bool {
    app.contains('.') && !app.contains(char::is_whitespace)
}

/// The icon Finder shows for `bundle`, as a `pixels`-square PNG with its
/// own transparency, never tinted or masked. Drawn into a bitmap of exactly
/// that many pixels, so AppKit picks the icon's representation for that
/// size (the screen's scale plays no part). `None` when AppKit cannot draw it.
pub fn icon_png(bundle: &Path, pixels: u32) -> Option<Vec<u8>> {
    let path = bundle.to_str()?;
    let side = pixels.clamp(16, 1024);
    let points = f64::from(side);
    autoreleasepool(|_| {
        let icon = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(path));
        // SAFETY: null `planes` asks AppKit to allocate and own the pixels;
        // the rest are plain numbers for an 8-bit RGBA bitmap, and the
        // color space name is AppKit's `NSDeviceRGBColorSpace` constant's
        // value.
        let bitmap = unsafe {
            NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                NSBitmapImageRep::alloc(),
                std::ptr::null_mut(),
                side as isize,
                side as isize,
                8,
                4,
                true,
                false,
                &NSString::from_str("NSDeviceRGBColorSpace"),
                0,
                0,
            )
        }?;
        // One point per pixel.
        bitmap.setSize(NSSize::new(points, points));
        let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&bitmap)?;
        NSGraphicsContext::saveGraphicsState_class();
        NSGraphicsContext::setCurrentContext(Some(&context));
        context.setImageInterpolation(NSImageInterpolation::High);
        icon.drawInRect(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(points, points),
        ));
        context.flushGraphics();
        NSGraphicsContext::restoreGraphicsState_class();
        let bitmap = bitmap
            .bitmapImageRepByRetaggingWithColorSpace(&NSColorSpace::sRGBColorSpace())
            .unwrap_or(bitmap);
        let properties = NSDictionary::new();
        // SAFETY: an empty dictionary has no values of the wrong type.
        let png = unsafe {
            bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
        }?;
        Some(png.to_vec())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icon_pixels::png_size;

    #[test]
    fn finder_is_found_by_bundle_id_name_and_path() {
        let by_id = bundle_path("com.apple.finder").expect("Finder by bundle id");
        assert!(by_id.ends_with("Finder.app"), "{by_id:?}");
        assert_eq!(bundle_path("Finder").as_ref(), Some(&by_id));
        assert_eq!(bundle_path(by_id.to_str().unwrap()).as_ref(), Some(&by_id));
        // A path to a file in it, as a rule may name an app.
        let file = by_id.join("Contents/Info.plist");
        assert_eq!(bundle_path(file.to_str().unwrap()), Some(file));
    }

    #[test]
    fn an_app_that_is_not_installed_has_no_bundle() {
        assert_eq!(bundle_path("com.example.no-such-app"), None);
        assert_eq!(bundle_path("No Such App Example"), None);
        assert_eq!(bundle_path("/Applications/No Such App Example.app"), None);
        assert_eq!(bundle_path("  "), None);
    }

    #[test]
    fn the_icon_renders_at_the_size_asked_off_the_main_thread() {
        let finder = bundle_path("com.apple.finder").expect("Finder");
        let png = std::thread::spawn(move || icon_png(&finder, 32))
            .join()
            .unwrap()
            .expect("Finder's icon");
        assert_eq!(png_size(&png), (32, 32));
    }
}
