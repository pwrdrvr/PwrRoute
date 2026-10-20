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
//! - **Paths.** [`router_dir`] is where the router's lock and socket live.
//!
//! Everything that needs `unsafe` (the LaunchServices calls) is in one
//! module, `macos::launch_services`.

mod args;
#[cfg(target_os = "macos")]
mod macos;
mod paths;
mod registration;

pub use args::links_from_args;
pub use paths::{data_dir, router_dir};
pub use registration::{
    BUNDLE_ID, DESKTOP_ID, DefaultRequest, DefaultStatus, PROG_ID, REGISTERED_NAME,
    default_browser_status, open_default_apps_settings, register_browser, set_default_browser,
    unregister_browser,
};
// Plans the platform code applies, public so they can be checked anywhere.
pub use registration::{RegistryValue, desktop_entry, windows_registry_values};

use pwrroute_router::OpenerInfo;

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
