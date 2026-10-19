//! The Dock icon follows the windows: shown while the main window or the
//! first-run window is open, gone when the last one closes, so the resident
//! router lives in the menu bar alone. As PwrSnap does with its Library.
//!
//! The tray dropdown is not a window people switch to and never shows it.
//! Off macOS this tracks nothing: Windows and Linux have no Dock.

use gpui_kit::App;

#[cfg(target_os = "macos")]
use crate::{app::PwrRoute, first_run::FirstRun};

/// Start following the windows. Call once, before any window opens; the app
/// starts in the menu bar only.
pub fn install(cx: &mut App) {
    #[cfg(target_os = "macos")]
    {
        crate::macos::set_dock_icon(false);
        // A window is being built: show the Dock icon before it is on screen,
        // so the app menu is there when it activates.
        cx.observe_new::<PwrRoute>(|_, _, _| crate::macos::set_dock_icon(true))
            .detach();
        cx.observe_new::<FirstRun>(|_, _, _| crate::macos::set_dock_icon(true))
            .detach();
        cx.on_window_closed(|cx, _| {
            crate::macos::set_dock_icon(has_dock_window(cx));
        })
        .detach();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = cx;
}

/// Whether a window that wants the Dock icon is open.
#[cfg(target_os = "macos")]
fn has_dock_window(cx: &App) -> bool {
    cx.windows().into_iter().any(|window| {
        window.downcast::<PwrRoute>().is_some() || window.downcast::<FirstRun>().is_some()
    })
}
