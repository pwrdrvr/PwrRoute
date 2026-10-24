//! Which running link helpers the app replaces when it starts
//! ([`crate::start_link_helper`]).
//!
//! An update swaps the whole `PwrRoute.app` and restarts only the app, so a
//! helper started before it is still running, from a binary that is gone,
//! and macOS keeps handing it every link. The new app asks such a helper to
//! quit and starts the one in its own bundle. Plain data, so the decision is
//! tested on every OS; `macos::start_link_helper` reads it from AppKit.

use std::time::SystemTime;

/// A link helper that is running when the app starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunningHelper {
    /// Its executable is the path of the helper nested in this app.
    pub ours: bool,
    /// A file is still at its executable's path. `true` when macOS does not
    /// say where it runs from.
    pub on_disk: bool,
    /// When LaunchServices launched it, if it says.
    pub launched: Option<SystemTime>,
}

/// Whether `running` makes way for the helper in this app, whose executable
/// was created at `bundled`:
///
/// - its executable is gone (the app it came from was updated, moved or
///   deleted), whichever copy of PwrRoute it belongs to;
/// - or it is this app's helper, launched before the executable on disk was
///   created: an update put a new file there while it ran. Updates extract
///   new files, so the creation time is the install's.
///
/// Any time that macOS or the disk does not give means "not stale": a helper
/// is never stopped on a guess, and one that was replaced also exits by
/// itself after its next link.
pub fn helper_is_stale(running: RunningHelper, bundled: Option<SystemTime>) -> bool {
    if !running.on_disk {
        return true;
    }
    running.ours
        && matches!(
            (running.launched, bundled),
            (Some(launched), Some(created)) if launched < created
        )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn at(secs: u64) -> Option<SystemTime> {
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
    }

    fn running(ours: bool, on_disk: bool, launched: Option<SystemTime>) -> RunningHelper {
        RunningHelper {
            ours,
            on_disk,
            launched,
        }
    }

    #[test]
    fn a_helper_launched_before_its_update_is_stale() {
        // Installed at 100; this helper ran since 50, from the old binary.
        assert!(helper_is_stale(running(true, true, at(50)), at(100)));
        // Started after the install: current.
        assert!(!helper_is_stale(running(true, true, at(150)), at(100)));
    }

    #[test]
    fn a_helper_whose_binary_is_gone_is_stale_from_any_copy() {
        assert!(helper_is_stale(running(true, false, at(150)), at(100)));
        assert!(helper_is_stale(running(false, false, None), None));
    }

    #[test]
    fn another_copys_helper_and_unknown_times_are_left_alone() {
        // A helper from another PwrRoute.app (a dev build, a second copy).
        assert!(!helper_is_stale(running(false, true, at(50)), at(100)));
        // macOS did not say when it launched, or the disk did not say when
        // the binary was created.
        assert!(!helper_is_stale(running(true, true, None), at(100)));
        assert!(!helper_is_stale(running(true, true, at(50)), None));
    }
}
