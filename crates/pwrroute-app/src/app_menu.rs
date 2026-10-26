//! The menu bar and the application-wide shortcuts.
//!
//! GPUI adds neither on its own: without them a Mac app has an empty menu
//! bar and no ⌘Q. The layout is the Pwr family's (PwrSuite menus v1, in the
//! PwrSuite Claude Design project: "Menu Standard - Suite Review" and its
//! "Views Slot" addendum): [App] · File · Edit · View · Window · Help.
//! [`menus`] is a pure template for either platform so the tests can hold
//! both to the standard. macOS installs it as the global menu bar; Windows
//! and Linux have none, so the title strip draws the same template there
//! (`menu_bar.rs`).

use gpui_kit::{App, KeyBinding, Keystroke, Menu, MenuItem, SystemMenuType};

use crate::app::Screen;
use crate::chrome::Chrome;
use crate::{help, settings, updater};

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
            /// Minimize the focused window.
            Minimize,
            /// Zoom the focused window.
            Zoom,
            /// Check GitHub Releases for a newer PwrRoute and download it.
            CheckForUpdates,
            /// Open Settings › About.
            About,
            /// Import any Finicky `.js` or `.ts` into the visual rules.
            ImportFinickyConfig,
            /// Edit › Cut, Copy, Paste and Select All: the focused text
            /// field's own keys, replayed (see [`replay`]).
            Cut,
            Copy,
            Paste,
            SelectAll,
            /// View: the main window's screens.
            ShowOverview,
            ShowRules,
            ShowTestLink,
            ShowActivity,
            /// Alt or F10 off macOS: put the keyboard on the strip's menu
            /// bar, or take it off again (`menu_bar.rs`).
            FocusMenuBar,
        ]
    );
}

/// The key context no element ever has. Edit's shortcuts are bound in it
/// so the menu can show ⌘C and the rest, while the keys themselves still
/// reach the focused text field, which reads raw keystrokes (`text_field`).
/// Bound anywhere a window has, ⌘V would become the Paste action and the
/// field would never see it.
const MENU_ONLY: &str = "MenuShortcutOnly";

/// Which menu bar to build: macOS's, or the one Windows and Linux use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Other,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else {
            Self::Other
        }
    }
}

/// Quit, from the menu (⌘Q) or the tray: a downloaded update is handed to
/// Velopack first, which installs it once this process has exited.
pub fn quit(cx: &mut App) {
    updater::apply_pending_on_quit();
    cx.quit();
}

/// Install the menus and shortcuts for this OS.
pub fn install(cx: &mut App) {
    install_for(Chrome::host(), cx);
}

/// Install the menus and shortcuts as `chrome` has them. Tests and the
/// `screens` example pass Windows or Linux on a Mac to draw that strip,
/// with its Ctrl shortcuts.
pub fn install_for(chrome: Chrome, cx: &mut App) {
    crate::chrome::set(chrome, cx);
    cx.on_action(|_: &Quit, cx| quit(cx));
    // ⌘, or Ctrl+, and the Settings window's control bindings.
    settings::install(cx);
    help::install(cx);
    // Registered on every platform so other surfaces can dispatch it.
    cx.on_action(|_: &CheckForUpdates, _| {
        updater::check_in_background(updater::Trigger::Menu);
    });
    // About and View's rows update a window that may be the one the action
    // was dispatched in, so they wait for that dispatch to finish (as
    // `in_front_window` does): GPUI still holds it, and an update fails
    // without a word.
    cx.on_action(|_: &About, cx| {
        cx.defer(|cx| settings::open(cx, Some(settings::Pane::About)));
    });
    cx.on_action(|_: &ImportFinickyConfig, cx| crate::importer_view::prompt_for_config(cx));
    cx.on_action(|_: &CloseWindow, cx| {
        in_front_window(cx, |window, _| window.remove_window());
    });
    cx.on_action(|_: &Minimize, cx| {
        in_front_window(cx, |window, _| window.minimize_window());
    });
    cx.on_action(|_: &Zoom, cx| in_front_window(cx, |window, _| window.zoom_window()));
    cx.on_action(|_: &Cut, cx| replay("x", cx));
    cx.on_action(|_: &Copy, cx| replay("c", cx));
    cx.on_action(|_: &Paste, cx| replay("v", cx));
    cx.on_action(|_: &SelectAll, cx| replay("a", cx));
    let show = |screen: Screen, cx: &mut App| {
        cx.defer(move |cx| crate::tray::show_screen(screen, cx));
    };
    cx.on_action(move |_: &ShowOverview, cx| show(Screen::Overview, cx));
    cx.on_action(move |_: &ShowRules, cx| show(Screen::Rules, cx));
    cx.on_action(move |_: &ShowTestLink, cx| show(Screen::TestLink, cx));
    cx.on_action(move |_: &ShowActivity, cx| show(Screen::Activity, cx));

    let platform = chrome.menus();
    let modifier = match platform {
        Platform::Mac => "cmd",
        Platform::Other => "ctrl",
    };
    let key = |chord: &str| format!("{modifier}-{chord}");
    cx.bind_keys([
        KeyBinding::new(&key("q"), Quit, None),
        KeyBinding::new(&key("w"), CloseWindow, None),
        KeyBinding::new(&key("m"), Minimize, None),
        KeyBinding::new(&key("x"), Cut, Some(MENU_ONLY)),
        KeyBinding::new(&key("c"), Copy, Some(MENU_ONLY)),
        KeyBinding::new(&key("v"), Paste, Some(MENU_ONLY)),
        KeyBinding::new(&key("a"), SelectAll, Some(MENU_ONLY)),
    ]);
    if platform == Platform::Mac {
        cx.on_action(|_: &Hide, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
        cx.bind_keys([
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("alt-cmd-h", HideOthers, None),
        ]);
        cx.set_menus(menus(platform));
    } else {
        // A lone Alt is GPUI's modifier-only keystroke: pressed and
        // released with nothing else.
        cx.bind_keys([KeyBinding::new("alt", FocusMenuBar, None)]);
        // Settings binds the host's shortcut; drawing another OS's strip
        // (tests, `screens`) needs that OS's.
        if chrome != Chrome::host() {
            cx.bind_keys([KeyBinding::new(&key(","), settings::OpenSettings, None)]);
        }
        crate::menu_bar::install(cx);
    }
}

/// Edit's rows: the same key, sent to the focused window as if pressed.
/// The text fields handle ⌘X/⌘C/⌘V/⌘A (Ctrl elsewhere) themselves, so a
/// menu click does exactly what the shortcut does; with no field focused
/// nothing takes it. The key is bound only in [`MENU_ONLY`], so this never
/// comes back here.
fn replay(key: &str, cx: &mut App) {
    let modifier = match Platform::current() {
        Platform::Mac => "cmd",
        Platform::Other => "ctrl",
    };
    let Ok(keystroke) = Keystroke::parse(&format!("{modifier}-{key}")) else {
        return;
    };
    in_front_window(cx, move |window, cx| {
        window.dispatch_keystroke(keystroke, cx);
    });
}

/// Runs `f` on the window in front, once the action that asked is done:
/// GPUI can still hold the window an action was dispatched in, and an
/// update inside that dispatch fails without a word.
fn in_front_window(cx: &mut App, f: impl FnOnce(&mut gpui_kit::Window, &mut App) + 'static) {
    let Some(window) = cx.active_window() else {
        return;
    };
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, cx| f(window, cx));
    });
}

/// The menu bar, in the family's order, for `platform`.
pub fn menus(platform: Platform) -> Vec<Menu> {
    let mac = platform == Platform::Mac;
    let mut menus = Vec::new();
    if mac {
        menus.push(Menu::new("PwrRoute").items([
            MenuItem::action("About PwrRoute", About),
            MenuItem::action("Check for Updates\u{2026}", CheckForUpdates),
            MenuItem::separator(),
            MenuItem::action("Settings\u{2026}", settings::OpenSettings),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide PwrRoute", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit PwrRoute", Quit),
        ]));
    }

    let mut file = vec![
        MenuItem::action("Import Finicky Config\u{2026}", ImportFinickyConfig),
        MenuItem::separator(),
    ];
    if !mac {
        file.push(MenuItem::action("Settings\u{2026}", settings::OpenSettings));
        file.push(MenuItem::separator());
    }
    file.push(MenuItem::action("Close Window", CloseWindow));
    if !mac {
        file.push(MenuItem::action("Quit", Quit));
    }
    menus.push(Menu::new("File").items(file));

    // No Undo or Redo: no field here keeps a history to undo.
    menus.push(Menu::new("Edit").items([
        MenuItem::action("Cut", Cut),
        MenuItem::action("Copy", Copy),
        MenuItem::action("Paste", Paste),
        MenuItem::separator(),
        MenuItem::action("Select All", SelectAll),
    ]));

    // Rule 7: the app's own screens open View. The standard's Reload Window
    // and zoom rows are the Electron renderer's; a GPUI window has neither,
    // and macOS adds Enter Full Screen itself.
    menus.push(Menu::new("View").items([
        MenuItem::action(Screen::Overview.label(), ShowOverview),
        MenuItem::action(Screen::Rules.label(), ShowRules),
        MenuItem::action(Screen::TestLink.label(), ShowTestLink),
        MenuItem::action(Screen::Activity.label(), ShowActivity),
    ]));

    // macOS lists the open windows under these itself (`setWindowsMenu`).
    let mut window = vec![MenuItem::action("Minimize", Minimize)];
    if mac {
        window.push(MenuItem::action("Zoom", Zoom));
    }
    menus.push(Menu::new("Window").items(window));

    menus.push(Menu::new("Help").items(help_items(platform)));
    menus
}

/// Help, in the standard's fixed order: learn, get help, the project, the
/// legal notices; off macOS, Check for Updates… and About last. No
/// "PwrRoute Website" row: PwrRoute has no site yet (rule 4: every item
/// unless nothing is behind it).
fn help_items(platform: Platform) -> Vec<MenuItem> {
    let mut items = vec![
        MenuItem::action("PwrRoute Documentation", help::OpenDocumentation),
        MenuItem::action("Changelog", help::OpenChangelog),
        MenuItem::action("Replay Onboarding\u{2026}", help::ReplayOnboarding),
        MenuItem::separator(),
        MenuItem::action("Report an Issue\u{2026}", help::ReportIssue),
        MenuItem::action(
            "Report a Security Vulnerability\u{2026}",
            help::ReportVulnerability,
        ),
        MenuItem::action("Copy Diagnostics Info", help::CopyDiagnostics),
        MenuItem::action("Logs", help::OpenLogs),
        MenuItem::separator(),
        MenuItem::action("View Source", help::ViewSource),
        MenuItem::separator(),
        MenuItem::action("View License", help::ViewLicense),
        MenuItem::action("Third-Party Notices", help::ThirdPartyNotices),
    ];
    if platform == Platform::Other {
        items.push(MenuItem::separator());
        items.push(MenuItem::action(
            "Check for Updates\u{2026}",
            CheckForUpdates,
        ));
        items.push(MenuItem::action("About PwrRoute", About));
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A menu as its labels, with "---" for a separator and "<Services>"
    /// for the OS's submenu.
    fn labels(menu: &Menu) -> Vec<String> {
        menu.items
            .iter()
            .map(|item| match item {
                MenuItem::Separator => "---".to_owned(),
                MenuItem::Action { name, .. } => name.to_string(),
                MenuItem::Submenu(menu) => format!("[{}]", menu.name),
                MenuItem::SystemMenu(menu) => format!("<{}>", menu.name),
            })
            .collect()
    }

    fn menu<'a>(menus: &'a [Menu], name: &str) -> &'a Menu {
        menus
            .iter()
            .find(|menu| menu.name == name)
            .unwrap_or_else(|| panic!("no {name} menu"))
    }

    #[test]
    fn the_menu_bar_follows_the_family_order() {
        let names = |platform| {
            menus(platform)
                .iter()
                .map(|menu| menu.name.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(Platform::Mac),
            ["PwrRoute", "File", "Edit", "View", "Window", "Help"]
        );
        assert_eq!(
            names(Platform::Other),
            ["File", "Edit", "View", "Window", "Help"]
        );
    }

    #[test]
    fn the_mac_app_menu_opens_with_about_and_updates() {
        let menus = menus(Platform::Mac);
        assert_eq!(
            labels(menu(&menus, "PwrRoute")),
            [
                "About PwrRoute",
                "Check for Updates…",
                "---",
                "Settings…",
                "---",
                "<Services>",
                "---",
                "Hide PwrRoute",
                "Hide Others",
                "Show All",
                "---",
                "Quit PwrRoute",
            ]
        );
        assert_eq!(
            labels(menu(&menus, "File")),
            ["Import Finicky Config…", "---", "Close Window"]
        );
        assert_eq!(labels(menu(&menus, "Window")), ["Minimize", "Zoom"]);
    }

    #[test]
    fn elsewhere_settings_is_in_file_and_about_ends_help() {
        let menus = menus(Platform::Other);
        assert_eq!(
            labels(menu(&menus, "File")),
            [
                "Import Finicky Config…",
                "---",
                "Settings…",
                "---",
                "Close Window",
                "Quit",
            ]
        );
        let help = labels(menu(&menus, "Help"));
        assert_eq!(
            help[help.len() - 3..],
            ["---", "Check for Updates…", "About PwrRoute"]
        );
    }

    #[test]
    fn help_is_in_the_standard_order_on_both_platforms() {
        let expected = [
            "PwrRoute Documentation",
            "Changelog",
            "Replay Onboarding…",
            "---",
            "Report an Issue…",
            "Report a Security Vulnerability…",
            "Copy Diagnostics Info",
            "Logs",
            "---",
            "View Source",
            "---",
            "View License",
            "Third-Party Notices",
        ];
        for platform in [Platform::Mac, Platform::Other] {
            let menus = menus(platform);
            let help = labels(menu(&menus, "Help"));
            assert_eq!(help[..expected.len()], expected, "{platform:?}");
        }
        assert_eq!(labels(menu(&menus(Platform::Mac), "Help")).len(), 13);
    }

    #[test]
    fn view_lists_the_screens_and_edit_has_no_undo() {
        for platform in [Platform::Mac, Platform::Other] {
            let menus = menus(platform);
            assert_eq!(
                labels(menu(&menus, "View")),
                ["Overview", "Rules", "Test a link", "Activity"]
            );
            assert_eq!(
                labels(menu(&menus, "Edit")),
                ["Cut", "Copy", "Paste", "---", "Select All"]
            );
        }
    }
}
