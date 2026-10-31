//! Synced rules: the same `rules.json` (and, if asked, the Finicky config)
//! on several machines through folders a cloud-drive client keeps in step:
//! iCloud Drive, Google Drive for desktop, Dropbox, or any folder (an
//! rclone or GNOME Online Accounts mount, a network share).
//!
//! PwrRoute talks to no cloud service. It copies files between its own data
//! directory and `<cloud root>/PwrRoute/`, and the client does the rest.
//!
//! - [`places`] finds the clients' folders on this machine.
//! - [`folder`] is one sync folder: its layout, the `sync.json` manifest,
//!   staged writes, and files the client has not downloaded.
//! - [`decide`] is the three-way decision for one file, from its local
//!   hash, the folder's and the base they last agreed on.
//! - [`Syncer`] runs a pass over every file and folder, settles another
//!   machine's write before applying it, backs up whatever it replaces and
//!   holds conflicts for the user ([`Syncer::resolve`]).
//! - [`Schedule`] says when a pass runs; [`Service`] is the thread that
//!   runs them.
//!
//! What differs between machines is not synced: the engine's
//! `machine.json` binds named destinations and switches rule sets per
//! machine. Nothing here watches a directory; every path read is exact.

pub mod decide;
pub mod folder;
pub mod places;
mod schedule;
mod service;
mod state;
mod syncer;

pub use decide::ConflictKind;
pub use folder::{FOLDER, FileEntry, Folder, MANIFEST, Machine, Manifest, hash};
pub use places::{Env, Kind, Os, Place, classify, detect};
pub use schedule::{Schedule, Why};
pub use service::{Clock, EVERY, Service, SystemClock, TICK};
pub use state::{Backup, KEEP_BACKUPS};
pub use syncer::{
    Check, Choice, Conflict, Content, FileStatus, Health, Item, Status, Syncer, Target,
    TargetStatus, Wait,
};
