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
//!   A resolver that watches files catches up in [`Resolver::refresh`],
//!   which runs before each link and when [`RouterHandle::refresh`] asks.
//! - [`Launcher`] is the seam to the browser launcher. [`DryRunLauncher`]
//!   records what would open and opens nothing.
//! - [`DispatchRecord`] is one dispatch with its trace. The last
//!   [`Options::retention`] of them stay in memory for Activity; Test a link
//!   gets the same record without opening anything or recording it.
//! - [`RouterHandle::set_paused`] sends every dispatch to a [`PauseTarget`]
//!   after its rewrites, with [`Step::Paused`] in the trace, until resumed.
//! - [`instance`] keeps one router per user: a second launch forwards its
//!   links over a local socket ([`ipc`]) and exits.
//!
//! Privacy: each record is reduced to the [`Privacy`] of the rule that
//! routed it (full, host only, or counted only) before anything stores or
//! sees it, and stays in memory for [`Options::retention`]. Nothing here
//! writes a URL to disk unless [`Options::journal`] is set; then each record
//! goes to a daily JSON-lines file, at its own level or the journal's,
//! whichever keeps less. [`DispatchRecord::timing`] is the URL-free view
//! meant for logs, and [`Options::retention_description`] says in words
//! what is kept where.
//!
//! The pipeline follows the PwrFinicky router (a fork of Finicky, MIT,
//! Copyright (c) 2015-2025 John Sterling); see `LICENSE`.

mod dispatch;
pub mod instance;
pub mod ipc;
mod journal;
mod launcher;
mod link;
mod recent;
mod trace;

pub use dispatch::{
    DispatchCallback, FALLBACK_BROWSER, FallbackSource, Options, PauseTarget, Resolved, Resolver,
    ResolverFactory, RetentionInfo, Router, RouterGone, RouterHandle, RouterStatus,
};
pub use journal::{JournalConfig, JournalMode, JournalStamp, read_recent as read_journal};
pub use launcher::{DryRunLauncher, Launcher};
pub use link::{Link, LinkError, MAX_URL_LEN, OpenerInfo, Source, normalize_url};
pub use recent::{RecentDispatches, Retention};
pub use trace::{
    DispatchId, DispatchRecord, DispatchTiming, Matched, Outcome, Owner, Privacy, PrivacyExt, Step,
    Timings, WITHHELD, host_only,
};

/// Re-exported so a launcher or UI can name the engine's types without a
/// direct dependency on the engine crate.
pub use pwrroute_engine::{AppType, Browser, Opener};
