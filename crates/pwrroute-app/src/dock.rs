//! The Dock icon follows the windows: shown while the main, Settings,
//! first-run or one of Help's windows is open, gone when the last one
//! closes, so the resident router lives in the menu bar alone. As PwrSnap
//! does with its Library.
//!
//! The tray dropdown is not a window people switch to and never shows it.
//! Off macOS this tracks nothing: Windows and Linux have no Dock.

use gpui_kit::App;

#[cfg(any(target_os = "macos", test))]
use crate::{
    app::PwrRoute, doc_window::DocWindow, first_run::FirstRun, logs_window::LogsWindow,
    settings::SettingsWindow,
};

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
        cx.observe_new::<SettingsWindow>(|_, _, _| crate::macos::set_dock_icon(true))
            .detach();
        cx.observe_new::<DocWindow>(|_, _, _| crate::macos::set_dock_icon(true))
            .detach();
        cx.observe_new::<LogsWindow>(|_, _, _| crate::macos::set_dock_icon(true))
            .detach();
        cx.on_window_closed(|cx, _| {
            crate::macos::set_dock_icon(has_dock_window(cx));
        })
        .detach();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = cx;
}

/// A link from the OS just arrived: keep the Dock icon as the windows want
/// it (`macos::restore_dock_icon`), and with a window open give the front
/// back to the app the link came from. Call on the main thread, while the
/// link's Apple Event is being handled.
///
/// With every window closed, going back to `Accessory` already hands the
/// front back. With one open, LaunchServices' activation raised it and put
/// PwrRoute second in Cmd-Tab, behind the browser, until the sender takes
/// the front again. Not while Settings' "Yield focus to browsers" is on:
/// its yield needs PwrRoute still active when the browser is handed the
/// link, or the browser cannot come forward.
pub fn after_link(yield_to_browser: bool) {
    #[cfg(target_os = "macos")]
    if crate::macos::restore_dock_icon() && !yield_to_browser {
        pwrroute_os::return_front_to_sender();
    }
    #[cfg(not(target_os = "macos"))]
    let _ = yield_to_browser;
}

/// Whether a window that wants the Dock icon is open.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn has_dock_window(cx: &App) -> bool {
    cx.windows().into_iter().any(|window| {
        window.downcast::<PwrRoute>().is_some()
            || window.downcast::<FirstRun>().is_some()
            || window.downcast::<SettingsWindow>().is_some()
            || window.downcast::<DocWindow>().is_some()
            || window.downcast::<LogsWindow>().is_some()
    })
}
