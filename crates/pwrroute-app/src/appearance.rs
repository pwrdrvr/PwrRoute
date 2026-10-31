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
//! PWR-23 adds the Dark theme and the Light theme (design: Appearance and
//! Themes 1c-1e), `appearance.darkTheme` and `appearance.lightTheme`, read
//! with the Theme before the first window opens and applied live.
//!
//! The palettes are `theme`'s: this module only decides which scheme and
//! themes draw, then refreshes the windows, the tray icon and Settings ›
//! General.

use gpui_kit::{App, Global, Window, WindowAppearance};

use crate::settings::model::{General, SettingsRequest};
use crate::settings_store::AppearanceSettings;
use crate::theme::{self, Appearance, DarkTheme, LightTheme, ThemeChoice};

/// The choice in force.
#[derive(Clone, Copy, Debug, Default)]
struct Chosen(ThemeChoice);

impl Global for Chosen {}

/// Reads `settings.json`'s Appearance and applies it. Call before the
/// first window opens.
pub fn install(settings: AppearanceSettings, cx: &mut App) {
    theme::set_dark_theme(settings.dark_theme);
    theme::set_light_theme(settings.light_theme);
    cx.set_global(Chosen(settings.theme));
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
    save(cx, "Theme", |appearance| appearance.theme = choice);
    cx.set_global(Chosen(choice));
    apply(cx);
}

/// Settings › General's Dark theme: saves it, then draws it at once when
/// PwrRoute is dark.
pub fn set_dark_theme(dark_theme: DarkTheme, cx: &mut App) {
    save(cx, "Dark theme", |appearance| {
        appearance.dark_theme = dark_theme
    });
    let drawn = theme::current().appearance == Appearance::Dark;
    if theme::current().dark_theme != dark_theme {
        theme::set_dark_theme(dark_theme);
        if drawn {
            log::info!("appearance: drawing {}", dark_theme.key());
            redraw(cx);
        }
    }
    report(cx);
}

/// Settings › General's Light theme: saves it, then draws it at once when
/// PwrRoute is light.
pub fn set_light_theme(light_theme: LightTheme, cx: &mut App) {
    save(cx, "Light theme", |appearance| {
        appearance.light_theme = light_theme;
    });
    let drawn = theme::current().appearance == Appearance::Light;
    if theme::current().light_theme != light_theme {
        theme::set_light_theme(light_theme);
        if drawn {
            log::info!("appearance: drawing {}", light_theme.key());
            redraw(cx);
        }
    }
    report(cx);
}

fn save(cx: &mut App, what: &str, edit: impl FnOnce(&mut AppearanceSettings)) {
    if let Some(store) = crate::settings_store::store(cx) {
        store.update(cx, |store, _| {
            if let Err(error) = store.update(|settings| edit(&mut settings.appearance)) {
                log::warn!("{what} is not saved: {error:#}");
            }
        });
    }
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
        dark_theme: current.dark_theme,
        light_theme: current.light_theme,
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
        SettingsRequest::SetDarkTheme(dark_theme) => {
            set_dark_theme(*dark_theme, cx);
            true
        }
        SettingsRequest::SetLightTheme(light_theme) => {
            set_light_theme(*light_theme, cx);
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
        redraw(cx);
    }
    report(cx);
}

/// Every window and the tray icon in the palette now in force.
fn redraw(cx: &mut App) {
    crate::tray::follow_scheme(cx);
    cx.refresh_windows();
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
            install(
                AppearanceSettings {
                    theme: ThemeChoice::Light,
                    ..AppearanceSettings::default()
                },
                cx,
            );
            assert_eq!(theme::current().appearance, Appearance::Light);
            assert_eq!(general(cx).theme, ThemeChoice::Light);
            set(ThemeChoice::System, cx);
            assert_eq!(theme::current().appearance, Appearance::Dark);
            set(ThemeChoice::Dark, cx);
            assert_eq!(theme::current().appearance, Appearance::Dark);
            assert_eq!(choice(cx), ThemeChoice::Dark);
        });
    }

    #[gpui_kit::test]
    fn a_theme_pick_draws_in_its_own_scheme_only(cx: &mut gpui_kit::TestAppContext) {
        let _restore = theme::DefaultOnDrop;
        cx.update(|cx| {
            install(
                AppearanceSettings {
                    theme: ThemeChoice::Dark,
                    dark_theme: DarkTheme::Gray,
                    light_theme: LightTheme::Blue,
                },
                cx,
            );
            assert_eq!(theme::palette(), DarkTheme::Gray.palette(false));
            // The light theme waits for the light scheme.
            set_light_theme(LightTheme::CatppuccinLatte, cx);
            assert_eq!(theme::palette(), DarkTheme::Gray.palette(false));
            set_dark_theme(DarkTheme::Phosphor, cx);
            assert_eq!(theme::palette(), DarkTheme::Phosphor.palette(false));
            set(ThemeChoice::Light, cx);
            assert_eq!(theme::palette(), LightTheme::CatppuccinLatte.palette(false));
            let general = general(cx);
            assert_eq!(general.dark_theme, DarkTheme::Phosphor);
            assert_eq!(general.light_theme, LightTheme::CatppuccinLatte);
        });
    }
}
