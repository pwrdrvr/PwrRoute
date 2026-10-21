//! The menu bar and the application-wide shortcuts.
//!
//! GPUI adds neither on its own: without them a Mac app has an empty menu
//! bar and no ⌘Q. Same shape as disktree's.

use gpui_kit::{App, KeyBinding, Menu, MenuItem, SystemMenuType};

use crate::{settings, updater};

pub use menu_actions::*;

#[allow(
    clippy::derive_partial_eq_without_eq,
    reason = "GPUI's actions! macro writes the derives, not us"
)]
mod menu_actions {
    use gpui_kit::actions;

    actions!(
        pwrroute,
        [
            /// Quit the application.
            Quit,
            /// Hide the application's windows.
            Hide,
            /// Hide every other application.
            HideOthers,
            /// Show every application again.
            ShowAll,
            /// Close the focused window.
            CloseWindow,
            /// Check GitHub Releases for a newer PwrRoute and download it.
            CheckForUpdates,
            /// Import any Finicky `.js` or `.ts` into the visual rules.
            ImportFinickyConfig,
        ]
    );
}

/// Quit, from the menu (⌘Q) or the tray: a downloaded update is handed to
/// Velopack first, which installs it once this process has exited.
pub fn quit(cx: &mut App) {
    updater::apply_pending_on_quit();
    cx.quit();
}

/// Install the menus and shortcuts.
pub fn install(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| quit(cx));
    // ⌘, or Ctrl+, and the Settings window's control bindings.
    settings::install(cx);
    // Registered on every platform so other surfaces can dispatch it; only
    // the macOS menu bar shows it today.
    cx.on_action(|_: &CheckForUpdates, _| {
        updater::check_in_background(updater::Trigger::Menu);
    });
    cx.on_action(|_: &ImportFinickyConfig, cx| crate::importer_view::prompt_for_config(cx));
    cx.on_action(|_: &CloseWindow, cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, _| window.remove_window());
        }
    });
    if !cfg!(target_os = "macos") {
        cx.bind_keys([
            KeyBinding::new("ctrl-q", Quit, None),
            KeyBinding::new("ctrl-w", CloseWindow, None),
        ]);
        return;
    }
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
    ]);
    cx.set_menus([
        Menu::new("PwrRoute").items([
            MenuItem::action("Check for Updates…", CheckForUpdates),
            MenuItem::separator(),
            MenuItem::action("Settings…", settings::OpenSettings),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide PwrRoute", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit PwrRoute", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("Import Finicky Config\u{2026}", ImportFinickyConfig),
            MenuItem::separator(),
            MenuItem::action("Close Window", CloseWindow),
        ]),
    ]);
}
