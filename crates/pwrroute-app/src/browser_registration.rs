//! Windows browser registration, called from Velopack's install, update and
//! uninstall hooks (see `updater::run_startup_hooks`). The work is in
//! `pwrroute_os`: HKCU `StartMenuInternet`, `RegisteredApplications` and the
//! `PwrRouteURL` ProgID.
//!
//! Both run in a hook process with no window, which Velopack kills after
//! 15–30 s. Registering only lists PwrRoute among the browsers Windows
//! offers; it never makes PwrRoute the default, and the hooks never open
//! Default Apps. That stays the explicit action in the app.

/// List PwrRoute among Windows' browsers.
pub fn register_browser() {
    // A hook has nowhere to report a failure; the install itself goes on,
    // and the next update re-registers.
    let _ = pwrroute_os::register_browser();
}

/// Remove what [`register_browser`] wrote.
pub fn unregister_browser() {
    let _ = pwrroute_os::unregister_browser();
}
