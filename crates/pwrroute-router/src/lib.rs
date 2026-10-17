//! PwrRoute's resident router.
//!
//! Every link the OS hands PwrRoute goes through one pipeline: normalize the
//! URL, resolve it with the engine, hand the result to a [`Launcher`], and
//! record what happened. The router owns that pipeline on a thread of its own,
//! so it keeps working with every window closed, and a [`RouterHandle`] lets
//! the OS edge, the IPC server and the UI feed it and read it from anywhere.
//!
//! - [`Router`] runs the dispatch thread. The engine is single-threaded, so
//!   the router builds its [`Resolver`] on that thread and never moves it.
//! - [`Launcher`] is the seam to the browser launcher. [`DryRunLauncher`]
//!   records what would open and opens nothing.
//! - [`DispatchRecord`] is one dispatch with its trace. The last
//!   [`Options::capacity`] of them stay in memory for Activity; Test a link
//!   gets the same record without opening anything or recording it.
//! - [`instance`] keeps one router per user: a second launch forwards its
//!   links over a local socket ([`ipc`]) and exits.
//!
//! Privacy: full URLs stay in memory. Nothing here writes a URL to disk;
//! [`DispatchRecord::timing`] is the URL-free view meant for logs.
//!
//! The pipeline follows the PwrFinicky router (a fork of Finicky, MIT,
//! Copyright (c) 2015-2025 John Sterling); see `LICENSE`.

mod dispatch;
pub mod instance;
pub mod ipc;
mod launcher;
mod link;
mod recent;
mod trace;

pub use dispatch::{
    DispatchCallback, FALLBACK_BROWSER, Options, Resolved, Resolver, ResolverFactory, Router,
    RouterGone, RouterHandle, RouterStatus,
};
pub use launcher::{DryRunLauncher, Launcher};
pub use link::{Link, LinkError, MAX_URL_LEN, OpenerInfo, Source, normalize_url};
pub use recent::RecentDispatches;
pub use trace::{DispatchId, DispatchRecord, DispatchTiming, Matched, Outcome, Step, Timings};

/// Re-exported so a launcher or UI can name the engine's types without a
/// direct dependency on the engine crate.
pub use pwrroute_engine::{AppType, Browser, Opener};
