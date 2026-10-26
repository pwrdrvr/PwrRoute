//! PwrRoute's browsers: which ones exist, which are installed, what profiles
//! they have, and how to open a URL in one.
//!
//! - [`Catalog`]: the browsers PwrRoute knows (`data/browsers.json`), with
//!   their macOS bundle ids, Windows executables and ProgIDs, Linux desktop
//!   ids, and profile directories. It covers Finicky's `browsers.json` and
//!   more.
//! - [`detect_installed`]: the browsers on this machine.
//! - [`list_profiles`]: a browser's profiles, from Chromium's `Local State`
//!   or Firefox's `profiles.ini`; [`read_profiles`] also says when the file
//!   is there but the OS will not let PwrRoute read it.
//! - [`Launcher`]: turns the engine's [`pwrroute_engine::Browser`] into a
//!   command ([`Launcher::plan`], the dry run) and runs it
//!   ([`Launcher::launch`]), adding the browser's private-window flag when
//!   the rule asked for one. A running Chromium browser with a verified
//!   protocol gets the link over its singleton socket ([`SocketHandOff`]),
//!   with `open` as the fallback; other browsers handle their own hand-off.
//!
//! Everything that reads the machine takes an [`Env`], so tests run against
//! fixture trees for any platform on any host. Nothing here writes to disk,
//! and nothing logs: URLs are private, and only [`Plan`] holds one.

mod catalog;
mod desktop_entry;
mod detect;
mod env;
mod launch;
mod plist;
mod profiles;
mod registry;
mod singleton;

pub use catalog::{
    BrowserEngine, BrowserInfo, Catalog, Linux, MacOs, PrivateFlag, ProfileFlag, Windows,
};
pub use detect::{InstalledBrowser, detect_installed};
pub use env::{Env, Platform};
pub use launch::{
    Delivery, LaunchError, LaunchOptions, Launched, Launcher, Note, Plan, catalog_entry,
    private_window,
};
pub use profiles::{Profile, ProfilesBlocked, list_profiles, read_profiles};
pub use singleton::{SocketError, SocketHandOff};
