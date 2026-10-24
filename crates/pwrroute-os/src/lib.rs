//! PwrRoute's OS edge: how links arrive, and how PwrRoute becomes a browser
//! the OS knows about.
//!
//! - **Link receipt.** macOS hands links over as `kAEGetURL` Apple Events,
//!   which GPUI turns into its open-URLs callback; [`current_opener`] reads
//!   the sending app off the event being handled. Windows (a ProgID command
//!   line) and Linux (a `.desktop` `Exec=pwrroute %u`) pass links in argv,
//!   which [`links_from_args`] picks out.
//! - **Registration.** [`register_browser`] makes the OS list PwrRoute as a
//!   browser and [`unregister_browser`] takes it off, for installers.
//!   [`set_default_browser`] asks the OS to make PwrRoute the default, which
//!   always ends in the OS's own consent: a prompt or the Settings page.
//!   [`open_default_apps_settings`] only shows that page.
//!   Nothing calls any of them at launch, and tests never do.
//! - **Paths.** [`router_dir`] is where the router's lock and socket live;
//!   [`log_dir`] is where the diagnostic log goes.
//! - **Window order.** [`top_window_owner`] names the app whose window is
//!   frontmost, read from the window server, so a link's hand-off to the
//!   browser can be timed from any thread. macOS only.
//!
//! - **App icons.** [`app_bundle`] finds the app a rule or the scan names and
//!   [`app_icon_png`] renders the icon Finder shows for it. macOS only for
//!   now: elsewhere both are `None` and the app draws its letter badge.
//!
//! Everything that needs `unsafe` is in three modules: `macos::launch_services`
//! (the LaunchServices calls), `macos::app_icon` (two AppKit calls) and
//! `macos::window_order` (the window server's list).

mod args;
#[cfg(target_os = "macos")]
mod macos;
mod paths;
mod registration;

pub use args::links_from_args;
pub use paths::{data_dir, log_dir, router_dir};
pub use registration::{
    BUNDLE_ID, DESKTOP_ID, DefaultRequest, DefaultStatus, PROG_ID, REGISTERED_NAME,
    app_display_name, default_browser_status, open_default_apps_settings, register_browser,
    set_default_browser, unregister_browser,
};
// Plans the platform code applies, public so they can be checked anywhere.
pub use registration::{RegistryValue, desktop_entry, windows_registry_values};

use std::path::{Path, PathBuf};

use pwrroute_router::OpenerInfo;

/// The process that owns a window, and its bundle id when it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowOwner {
    pub pid: i32,
    pub bundle_id: Option<String>,
}

/// Who owns the frontmost window a person sees, from any thread. Asks the
/// window server, so it is current even while this app's main thread is
/// busy. Always `None` off macOS.
pub fn top_window_owner() -> Option<WindowOwner> {
    #[cfg(target_os = "macos")]
    {
        macos::top_window_owner()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// The installed app `app` names: a bundle id, an app's name as Finder
/// shows it, or a path to any file. Reads LaunchServices and the disk, so call it
/// off the UI thread. Always `None` off macOS, which has no implementation
/// yet.
pub fn app_bundle(app: &str) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        macos::installed_app_bundle(app)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        None
    }
}

/// The icon Finder shows for the app at `bundle`, unmodified, as a
/// `pixels`-square PNG (clamped to 16-1024). Reads the disk, so call it off
/// the UI thread. Always `None` off macOS, which has no implementation yet.
pub fn app_icon_png(bundle: &Path, pixels: u32) -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        macos::app_icon_png(bundle, pixels)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (bundle, pixels);
        None
    }
}

/// Whether an app with `bundle_id` is running in this login session.
/// Always `false` off macOS, where browsers are not found by bundle id.
pub fn app_is_running(bundle_id: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::app_is_running(bundle_id)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = bundle_id;
        false
    }
}

/// The app that sent the link being handled right now, when the OS says.
///
/// On macOS call it from GPUI's open-URLs callback: AppKit is handling the
/// `kAEGetURL` event then, and the event carries the sender's pid. Elsewhere
/// the OS does not say, and this is `None`.
pub fn current_opener() -> Option<OpenerInfo> {
    #[cfg(target_os = "macos")]
    {
        macos::current_apple_event_opener()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(all(test, target_os = "macos"))]
mod window_order_tests {
    /// Needs a desktop session; run by hand with `--ignored`.
    #[test]
    #[ignore = "needs a logged-in desktop"]
    fn the_top_window_has_an_owner() {
        let owner = super::top_window_owner().expect("a window is on top");
        eprintln!("top window: {owner:?}");
        assert!(owner.pid > 0);
    }
}
