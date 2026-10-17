//! STUB: Windows browser registration, called from Velopack's install,
//! update and uninstall hooks (see `updater::run_startup_hooks`).
//!
//! The real code is being written in the OS-integration crate (HKCU
//! `StartMenuInternet`, `RegisteredApplications` and the ProgID). When it
//! lands, replace these bodies with calls into it and delete this module;
//! `updater.rs` needs no other change.
//!
//! Both run in a hook process with no window, which Velopack kills after
//! 15–30 s. Registering only lists PwrRoute among the browsers Windows
//! offers; it never makes PwrRoute the default, and the hooks never open
//! Default Apps. That stays the explicit action in the app.

/// List PwrRoute among Windows' browsers. STUB: does nothing yet.
pub fn register_browser() {}

/// Remove what [`register_browser`] wrote. STUB: does nothing yet.
pub fn unregister_browser() {}
