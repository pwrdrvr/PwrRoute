//! Settings › General › Theme (PWR-22; design: Appearance and Themes 1a,
//! 1b): System, Dark or Light, saved as `settings.json`'s
//! `appearance.theme` and read before the first window opens, as
//! `view.zoom` is.
//!
//! System follows the OS while the app runs, windows or none: GPUI reads
//! macOS's effective appearance, Windows' app mode and the desktop
//! portal's `color-scheme` on Linux (no preference is light), and
//! `accessibility_prefs` asks again on its poll, beside Increase Contrast;
//! an open main or Settings window also hears the change at once
//! ([`watch`]). Dark and Light set the app's own appearance too
//! (`App::set_window_appearance`, `NSApp.appearance` on macOS), so the
//! window's native background, title bar, menus and panels match the theme
//! rather than the OS, and no window opens in the other scheme first;
//! System clears it.
//!
//! The palette is `theme`'s: this module only decides which scheme draws,
//! then refreshes the windows, the tray icon and Settings › General.

use gpui_kit::{App, Global, Window, WindowAppearance};

use crate::settings::model::{General, SettingsRequest};
use crate::theme::{self, Appearance, ThemeChoice};

/// The choice in force.
#[derive(Clone, Copy, Debug, Default)]
struct Chosen(ThemeChoice);

impl Global for Chosen {}

/// Reads `choice` (from `settings.json`) and applies it. Call before the
/// first window opens.
pub fn install(choice: ThemeChoice, cx: &mut App) {
    cx.set_global(Chosen(choice));
    apply(cx);
}

/// The choice in force.
pub fn choice(cx: &App) -> ThemeChoice {
    cx.try_global::<Chosen>()
        .map(|chosen| chosen.0)
        .unwrap_or_default()
}

/// Settings › General's Theme: saves `choice`, then applies it.
pub fn set(choice: ThemeChoice, cx: &mut App) {
    if let Some(store) = crate::settings_store::store(cx) {
        store.update(cx, |store, _| {
            if let Err(error) = store.update(|settings| settings.appearance.theme = choice) {
                log::warn!("Theme is not saved: {error:#}");
            }
        });
    }
    cx.set_global(Chosen(choice));
    apply(cx);
}

/// Asks the OS again. A change matters only under System; Settings ›
/// General hears about it either way (Increase Contrast shares its row).
pub fn follow_os(cx: &mut App) {
    resolve(cx);
}

/// Follows the OS at once while `window` is open, rather than at the next
/// poll.
pub fn watch(window: &mut Window) {
    window
        .observe_window_appearance(|_, cx| follow_os(cx))
        .detach();
}

/// General's model, as Settings draws it.
pub fn general(cx: &App) -> General {
    let current = theme::current();
    General {
        theme: choice(cx),
        showing: current.appearance,
        increase_contrast: current.increase_contrast,
        os: if cfg!(target_os = "macos") {
            "macOS"
        } else if cfg!(target_os = "windows") {
            "Windows"
        } else {
            "your desktop"
        },
    }
}

/// The Settings requests this module answers. `false` for any other.
pub fn handle_settings_request(request: &SettingsRequest, cx: &mut App) -> bool {
    match request {
        SettingsRequest::SetTheme(choice) => {
            set(*choice, cx);
            true
        }
        _ => false,
    }
}

/// Sets the app's own appearance for the choice, then the scheme.
fn apply(cx: &mut App) {
    if !in_test(cx) {
        cx.set_window_appearance(match choice(cx) {
            ThemeChoice::System => None,
            ThemeChoice::Dark => Some(WindowAppearance::Dark),
            ThemeChoice::Light => Some(WindowAppearance::Light),
        });
    }
    resolve(cx);
}

/// Draws the scheme the choice resolves to, when it is not already drawn,
/// and tells Settings.
fn resolve(cx: &mut App) {
    let chosen = choice(cx);
    let scheme = chosen.resolve(os_scheme(cx));
    if theme::current().appearance != scheme {
        log::info!("appearance: drawing {scheme:?} (Theme: {})", chosen.label());
        theme::set_appearance(scheme);
        crate::tray::follow_scheme(cx);
        cx.refresh_windows();
    }
    report(cx);
}

/// Tells Settings › General, when it shows something else. Deferred: a
/// request and the Settings window's own appearance observer both run
/// inside that window's update, where updating it again fails silently.
fn report(cx: &mut App) {
    cx.defer(|cx| {
        let general = general(cx);
        crate::settings::update_if(cx, |model| {
            (model.general != general).then(|| model.general = general)
        });
    });
}

/// The OS's scheme. Tests and the fixture renderer never read the OS:
/// System is dark there, as PwrRoute always drew.
fn os_scheme(cx: &App) -> Appearance {
    if in_test(cx) {
        return Appearance::Dark;
    }
    match cx.window_appearance() {
        WindowAppearance::Dark | WindowAppearance::VibrantDark => Appearance::Dark,
        WindowAppearance::Light | WindowAppearance::VibrantLight => Appearance::Light,
    }
}

/// Tests and the fixture renderer: never read or set the OS's appearance.
pub(crate) fn in_test(cx: &App) -> bool {
    cx.background_executor()
        .scheduler_executor()
        .scheduler()
        .as_test()
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui_kit::test]
    fn a_choice_draws_its_scheme_and_system_is_dark_in_tests(cx: &mut gpui_kit::TestAppContext) {
        let _restore = theme::DefaultOnDrop;
        cx.update(|cx| {
            install(ThemeChoice::Light, cx);
            assert_eq!(theme::current().appearance, Appearance::Light);
            assert_eq!(general(cx).theme, ThemeChoice::Light);
            set(ThemeChoice::System, cx);
            assert_eq!(theme::current().appearance, Appearance::Dark);
            set(ThemeChoice::Dark, cx);
            assert_eq!(theme::current().appearance, Appearance::Dark);
            assert_eq!(choice(cx), ThemeChoice::Dark);
        });
    }
}
