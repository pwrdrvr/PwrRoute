//! How long a link takes to reach the screen. The dispatch's own timings
//! end when the launcher returns; what a person waits for ends when the
//! browser's window is on top. From the instant the router received a link,
//! a short-lived thread samples who owns the frontmost window
//! ([`pwrroute_os::top_window_owner`], from the window server, so a busy
//! main thread cannot hide the delay) and logs each change:
//!
//! ```text
//! link 42 hand-off: com.tinyspeck.slackmacgap at receipt, PwrRoute +12ms, com.microsoft.edgemac +296ms
//! ```
//!
//! It stops when an app other than PwrRoute and the one on top at receipt
//! comes to the front, when that first app comes back after PwrRoute did,
//! or after [`WATCH_FOR`]. A hand-off that took [`SLOW_HANDOFF`] or longer,
//! or never happened, is a warning. macOS only: elsewhere there is no
//! window order to read, and nothing is logged.
//!
//! Privacy follows the link's record: nothing while links are not kept at
//! all, and only the times, not the apps, for a `privacy: none` rule's link
//! or a link whose record is not in memory to ask.

use std::{
    sync::{Arc, OnceLock},
    thread,
    time::{Duration, Instant},
};

use pwrroute_router::{DispatchId, Privacy, RouterHandle};

/// How long the window order is watched after a link arrives.
pub const WATCH_FOR: Duration = Duration::from_secs(3);

/// A hand-off at least this slow is logged as a warning.
pub const SLOW_HANDOFF: Duration = Duration::from_millis(1_000);

const EVERY: Duration = Duration::from_millis(15);

/// How long the watcher waits for the link's record after the hand-off.
const RECORD_WAIT: Duration = Duration::from_secs(2);

/// Filled once the router is spawned, so the watcher can read the link's
/// record (its privacy) when it is done.
pub type HandleSlot = Arc<OnceLock<RouterHandle>>;

/// Starts watching the hand-off of link `id`, received at `received`.
pub fn watch(id: DispatchId, received: Instant, router: HandleSlot) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let me = std::process::id() as i32;
    let _ = thread::Builder::new()
        .name("pwrroute-handoff".into())
        .spawn(move || {
            let mut track = Track::default();
            loop {
                let owner = pwrroute_os::top_window_owner().map(|owner| {
                    if owner.pid == me {
                        Owner::PwrRoute
                    } else {
                        Owner::App(
                            owner
                                .bundle_id
                                .unwrap_or_else(|| format!("pid {}", owner.pid)),
                        )
                    }
                });
                let Some(owner) = owner else { return };
                if track.see(received.elapsed(), owner) {
                    break;
                }
                thread::sleep(EVERY);
            }
            let Some(router) = router.get() else { return };
            let retention = router.retention();
            if retention.memory.count == 0 && retention.journal.is_none() {
                return;
            }
            // The browser can be on top before the dispatch thread has
            // stored the record, so wait for it a little. A record that
            // never shows up (not kept in memory) hides the apps.
            let hide_apps = finished(router, id).is_none_or(|privacy| privacy == Privacy::None);
            let line = track.line(id, hide_apps);
            if track.slow() {
                log::warn!(target: "pwrroute::link", "{line}");
            } else {
                log::info!(target: "pwrroute::link", "{line}");
            }
        });
}

/// The privacy level link `id`'s record was kept at, once the dispatch
/// thread has stored it; `None` if it is not there within [`RECORD_WAIT`].
fn finished(router: &RouterHandle, id: DispatchId) -> Option<Privacy> {
    let deadline = Instant::now() + RECORD_WAIT;
    loop {
        if let Some(record) = router.get(id) {
            return Some(record.privacy);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(EVERY);
    }
}

/// Who owns the frontmost window.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Owner {
    PwrRoute,
    App(String),
}

/// The window order's changes since receipt, and when to stop.
#[derive(Debug, Default)]
struct Track {
    /// Each owner in turn, with when it was first seen on top.
    changes: Vec<(Duration, Owner)>,
    /// A third app came to the front: the browser, most likely.
    landed: bool,
    timed_out: bool,
}

impl Track {
    /// Takes one sample; `true` when the watch is over.
    fn see(&mut self, at: Duration, owner: Owner) -> bool {
        if self.changes.last().is_none_or(|(_, last)| *last != owner) {
            self.changes.push((at, owner.clone()));
        }
        let first = &self.changes[0].1;
        let pwrroute_seen = self.changes.iter().any(|(_, o)| *o == Owner::PwrRoute);
        if owner != Owner::PwrRoute && owner != *first {
            self.landed = true;
            return true;
        }
        if self.changes.len() > 1 && owner == *first && pwrroute_seen {
            // The first app took its window back after PwrRoute's: the link
            // opened where it was clicked, or the browser never came up.
            self.landed = true;
            return true;
        }
        if at >= WATCH_FOR {
            self.timed_out = true;
            return true;
        }
        false
    }

    fn slow(&self) -> bool {
        self.timed_out
            || self
                .changes
                .last()
                .is_some_and(|(at, _)| self.landed && *at >= SLOW_HANDOFF)
    }

    fn line(&self, id: DispatchId, hide_apps: bool) -> String {
        let name = |owner: &Owner| match owner {
            Owner::PwrRoute => "PwrRoute".to_owned(),
            Owner::App(_) if hide_apps => "another app".to_owned(),
            Owner::App(app) => app.clone(),
        };
        let mut parts = Vec::with_capacity(self.changes.len());
        for (i, (at, owner)) in self.changes.iter().enumerate() {
            if i == 0 {
                parts.push(format!("{} at receipt", name(owner)));
            } else {
                parts.push(format!("{} +{}ms", name(owner), at.as_millis()));
            }
        }
        let mut line = format!("link {} hand-off: {}", id.0, parts.join(", "));
        if self.timed_out {
            line.push_str(&format!(
                "; no other app's window on top after {}ms",
                WATCH_FOR.as_millis()
            ));
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn app(id: &str) -> Owner {
        Owner::App(id.to_owned())
    }

    #[test]
    fn the_watch_ends_when_the_browser_comes_to_the_front() {
        let mut track = Track::default();
        assert!(!track.see(ms(1), app("com.example.chat")));
        assert!(!track.see(ms(16), app("com.example.chat")));
        assert!(!track.see(ms(31), Owner::PwrRoute));
        assert!(!track.see(ms(250), Owner::PwrRoute));
        assert!(track.see(ms(296), app("com.example.browser")));
        assert!(!track.slow());
        assert_eq!(
            track.line(DispatchId(42), false),
            "link 42 hand-off: com.example.chat at receipt, PwrRoute +31ms, com.example.browser +296ms"
        );
        assert_eq!(
            track.line(DispatchId(42), true),
            "link 42 hand-off: another app at receipt, PwrRoute +31ms, another app +296ms"
        );
    }

    #[test]
    fn a_browser_that_takes_a_second_or_more_is_slow() {
        let mut track = Track::default();
        track.see(ms(1), app("com.example.chat"));
        track.see(ms(20), Owner::PwrRoute);
        assert!(track.see(ms(1_850), app("com.example.browser")));
        assert!(track.slow());
    }

    #[test]
    fn the_first_app_taking_its_window_back_after_pwrroute_ends_the_watch() {
        let mut track = Track::default();
        track.see(ms(1), app("com.example.browser"));
        track.see(ms(20), Owner::PwrRoute);
        assert!(track.see(ms(240), app("com.example.browser")));
        assert!(!track.slow());
    }

    #[test]
    fn no_change_in_three_seconds_is_slow_and_says_so() {
        let mut track = Track::default();
        track.see(ms(1), app("com.example.chat"));
        track.see(ms(15), Owner::PwrRoute);
        assert!(!track.see(ms(2_999), Owner::PwrRoute));
        assert!(track.see(WATCH_FOR, Owner::PwrRoute));
        assert!(track.slow());
        assert_eq!(
            track.line(DispatchId(7), false),
            "link 7 hand-off: com.example.chat at receipt, PwrRoute +15ms; no other app's window on top after 3000ms"
        );
    }
}
