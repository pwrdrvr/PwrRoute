//! The app whose window is frontmost, from the window server's list.
//!
//! `NSWorkspace.frontmostApplication` only changes when the main run loop
//! runs, so a reader on another thread sees it late exactly when the main
//! thread is busy. `CGWindowListCopyWindowInfo` asks the window server
//! directly and works from any thread. Owner pids and layers need no
//! Screen Recording permission (window titles would).

#![allow(unsafe_code)]

use objc2::rc::autoreleasepool;
use objc2_app_kit::NSRunningApplication;
use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_graphics::{
    CGWindowListCopyWindowInfo, CGWindowListOption, kCGNullWindowID, kCGWindowAlpha,
    kCGWindowLayer, kCGWindowOwnerPID,
};

use crate::WindowOwner;

/// The owner of the frontmost visible window in the normal layer (0): the
/// window a person sees on top. Menu bar, Dock and overlay windows sit in
/// other layers and are skipped, as are fully transparent windows.
///
/// Called from a plain thread, many times a second while a link is
/// watched, so its AppKit objects are released in a pool of its own.
pub fn top_window_owner() -> Option<WindowOwner> {
    autoreleasepool(|_| top_window_owner_in_pool())
}

fn top_window_owner_in_pool() -> Option<WindowOwner> {
    let list = CGWindowListCopyWindowInfo(
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements,
        kCGNullWindowID,
    )?;
    // SAFETY: CGWindowListCopyWindowInfo returns a CFArray of
    // CFDictionary, keyed by CFString, ordered front to back.
    let list: CFRetained<CFArray<CFDictionary<CFString, CFType>>> =
        unsafe { CFRetained::cast_unchecked(list) };
    // SAFETY: the keys are immutable CFString constants CoreGraphics
    // exports; reading them has no side effects.
    let (layer_key, alpha_key, pid_key) =
        unsafe { (kCGWindowLayer, kCGWindowAlpha, kCGWindowOwnerPID) };
    for window in list.iter() {
        let number = |key: &CFString| {
            window
                .get(key)
                .and_then(|value| value.downcast::<CFNumber>().ok())
        };
        if number(layer_key).and_then(|n| n.as_i64()) != Some(0) {
            continue;
        }
        if number(alpha_key)
            .and_then(|n| n.as_f64())
            .is_some_and(|alpha| alpha <= 0.0)
        {
            continue;
        }
        let pid = number(pid_key).and_then(|n| n.as_i32())?;
        let bundle_id = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
            .and_then(|app| app.bundleIdentifier())
            .map(|id| id.to_string());
        return Some(WindowOwner { pid, bundle_id });
    }
    None
}
