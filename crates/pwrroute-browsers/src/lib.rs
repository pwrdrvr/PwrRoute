//! PwrRoute's browsers: which ones exist, which are installed, what profiles
//! they have, and how to open a URL in one.
//!
//! - [`Catalog`]: the browsers PwrRoute knows (`data/browsers.json`), with
//!   their macOS bundle ids, Windows executables and ProgIDs, Linux desktop
//!   ids, and profile directories. It covers Finicky's `browsers.json` and
//!   more.
//! - [`detect_installed`]: the browsers on this machine.
//! - [`list_profiles`]: a browser's profiles, from Chromium's `Local State`
//!   or Firefox's `profiles.ini`.
//! - [`Launcher`]: turns the engine's [`pwrroute_engine::Browser`] into a
//!   command ([`Launcher::plan`], the dry run) and runs it
//!   ([`Launcher::launch`]).
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

pub use catalog::{BrowserEngine, BrowserInfo, Catalog, Linux, MacOs, ProfileFlag, Windows};
pub use detect::{InstalledBrowser, detect_installed};
pub use env::{Env, Platform};
pub use launch::{LaunchError, LaunchOptions, Launcher, Note, Plan};
pub use profiles::{Profile, list_profiles};
