//! Whether a macOS reopen asks for the window.
//!
//! GPUI passes a reopen on only while PwrRoute has no window open. A person
//! sends one by opening PwrRoute from Spotlight or Finder while it runs in
//! the menu bar, and wants the main window. LaunchServices also sends one
//! beside some links it hands the app, before or after the link's own Apple
//! Event, and that one must not open the window: with the window closed,
//! every link would bring it back, put PwrRoute in Cmd-Tab between the
//! browser and the app the link was clicked in, and leave it there. So a
//! reopen waits [`REOPEN_GRACE`] for a link that follows it, and opens the
//! window only when no link arrived from [`REOPEN_AFTER_LINK`] before it
//! until then (as the PwrFinicky router coalesced them).
//!
//! Each decision is logged, never with the link; how long before or after
//! a link it came only while links are kept at all.

use std::{
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

/// How long a macOS launch waits for an Apple Event before it opens the
/// window: a launch for a link must not show it.
pub const LINK_GRACE: Duration = Duration::from_millis(250);

/// How long a reopen waits for a link LaunchServices sends after it. The
/// first link after the app has been idle can take half a second to reach
/// it.
pub const REOPEN_GRACE: Duration = Duration::from_millis(500);

/// A reopen this soon after a link is part of that link, not a request for
/// the window.
pub const REOPEN_AFTER_LINK: Duration = Duration::from_secs(2);

/// When the OS last handed the app a link.
#[derive(Clone, Default)]
pub struct LastLink(Arc<Mutex<Option<Instant>>>);

impl LastLink {
    pub fn mark(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
    }

    pub fn at(&self) -> Option<Instant> {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn ever(&self) -> bool {
        self.at().is_some()
    }
}

/// What a reopen was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reopen {
    /// No link came near it: the person asked for the window. The time
    /// since the last link, if there was one.
    Window { since_link: Option<Duration> },
    /// Part of a link that arrived this long before it.
    LinkBefore(Duration),
    /// Part of a link that arrived this long after it, while it waited.
    LinkAfter(Duration),
}

/// Decides a reopen received at `reopened`, once it has waited
/// [`REOPEN_GRACE`], from when the last link arrived.
pub fn decide(reopened: Instant, last_link: Option<Instant>) -> Reopen {
    match last_link {
        Some(at) if at >= reopened => Reopen::LinkAfter(at.duration_since(reopened)),
        Some(at) if reopened.duration_since(at) < REOPEN_AFTER_LINK => {
            Reopen::LinkBefore(reopened.duration_since(at))
        }
        since => Reopen::Window {
            since_link: since.map(|at| reopened.duration_since(at)),
        },
    }
}

/// Logs the decision. Link timings only while links are kept at all, as
/// for the other per-link lines.
pub fn log(decision: Reopen, keeps_links: bool) {
    match decision {
        Reopen::Window { since_link } => match since_link.filter(|_| keeps_links) {
            Some(gap) => log::info!(
                target: "pwrroute",
                "reopen {}ms after the last link: opening the window",
                gap.as_millis()
            ),
            None => log::info!(target: "pwrroute", "reopen: opening the window"),
        },
        _ if !keeps_links => {}
        Reopen::LinkBefore(gap) => log::info!(
            target: "pwrroute::link",
            "reopen {}ms after a link: part of the link, the window stays closed",
            gap.as_millis()
        ),
        Reopen::LinkAfter(gap) => log::info!(
            target: "pwrroute::link",
            "reopen {}ms before a link: part of the link, the window stays closed",
            gap.as_millis()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn a_reopen_with_no_link_opens_the_window() {
        let now = Instant::now();
        assert_eq!(decide(now, None), Reopen::Window { since_link: None });
    }

    #[test]
    fn a_reopen_beside_a_link_keeps_the_window_closed() {
        let reopened = Instant::now() + Duration::from_secs(10);
        // Before it, as late as the old 750 ms check missed.
        assert_eq!(
            decide(reopened, Some(reopened - 900 * MS)),
            Reopen::LinkBefore(900 * MS)
        );
        assert_eq!(
            decide(reopened, Some(reopened - (REOPEN_AFTER_LINK - MS))),
            Reopen::LinkBefore(REOPEN_AFTER_LINK - MS)
        );
        // After it, while it waited: the old check missed a link more
        // than 250 ms late.
        assert_eq!(
            decide(reopened, Some(reopened + 400 * MS)),
            Reopen::LinkAfter(400 * MS)
        );
        assert_eq!(
            decide(reopened, Some(reopened)),
            Reopen::LinkAfter(Duration::ZERO)
        );
    }

    #[test]
    fn a_reopen_long_after_a_link_opens_the_window() {
        let reopened = Instant::now() + Duration::from_secs(10);
        assert_eq!(
            decide(reopened, Some(reopened - REOPEN_AFTER_LINK)),
            Reopen::Window {
                since_link: Some(REOPEN_AFTER_LINK)
            }
        );
    }

    #[test]
    fn last_link_remembers_the_latest() {
        let last = LastLink::default();
        assert!(!last.ever());
        last.mark();
        let first = last.at().unwrap();
        last.mark();
        assert!(last.at().unwrap() >= first);
        assert!(last.clone().ever(), "clones share the mark");
    }
}
