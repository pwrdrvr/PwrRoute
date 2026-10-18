//! What one dispatch did, step by step: the record Activity lists, Test a
//! link shows, and an agent's `pwrroute_resolve` returns.

use std::time::{Duration, SystemTime};

use pwrroute_engine::Browser;
use serde::{Deserialize, Serialize, Serializer};

use crate::link::{Link, Source};

/// A dispatch's number, unique within one router process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DispatchId(pub u64);

/// One dispatch: the link, every step that fired, and how long each part took.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchRecord {
    pub id: DispatchId,
    #[serde(serialize_with = "unix_millis")]
    pub received_at: SystemTime,
    pub link: Link,
    /// The link came from a second launch, over the IPC socket.
    pub forwarded: bool,
    /// The URL after every rewrite: what the browser was asked to open.
    pub final_url: Option<String>,
    /// The browser the link went to, or would have, when one was chosen.
    pub browser: Option<Browser>,
    pub steps: Vec<Step>,
    pub outcome: Outcome,
    pub timings: Timings,
}

/// One step in a dispatch's trace, in the order it happened.
///
/// Rewrites and handlers are two lists, as in Finicky v4: every matching
/// rewrite runs in order, then the first matching handler wins, then the
/// default. Rule numbers are indexes into the config's `rewrite` and
/// `handlers` arrays, present when the resolver reports them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Step {
    /// The link as received, before any rule ran.
    Received {
        url: String,
    },
    /// A rewrite changed the URL. `rule` is `None` when the resolver only
    /// reports the final URL; `url` is then the URL after every rewrite.
    Rewrite {
        rule: Option<usize>,
        url: String,
    },
    /// Which rule chose the browser.
    Matched {
        by: Matched,
    },
    /// The config threw or timed out. Finicky still returns the default
    /// browser in that case, and so does the router.
    ConfigError {
        message: String,
    },
    /// No config is loaded, so the fallback browser applies.
    Fallback {
        reason: String,
    },
    /// The config sent the link back to PwrRoute, which would loop. The
    /// fallback browser opens it instead.
    RefusedLoop {
        browser: String,
    },
    /// The chosen browser.
    Browser {
        browser: Browser,
    },
    /// `browser: null` in the config: the link is deliberately not opened.
    NotOpened,
    /// The launcher was asked to open it.
    Launched {
        dry_run: bool,
    },
    LaunchFailed {
        message: String,
    },
    /// The link was refused before routing: not a URL the router accepts.
    Rejected {
        reason: String,
    },
}

/// The rule that chose the browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Matched {
    /// `handlers[index]` matched first.
    Handler { index: usize },
    /// No handler matched; `defaultBrowser` applied.
    Default,
    /// The resolver did not say. The current engine reports the browser, not
    /// the rule; the trace fills in when it does.
    Unreported,
}

/// How a dispatch ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    /// Handed to the launcher, which reported success.
    Opened,
    /// Resolved and handed to a dry-run launcher.
    DryRun,
    /// Resolved only, for Test a link. Nothing opened, nothing recorded.
    Tested,
    /// The config chose `browser: null`.
    NotOpened,
    /// Rejected before routing, or the launcher failed.
    Failed,
}

/// Durations, in microseconds, measured from the moment the OS edge handed
/// the link to the router.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timings {
    /// Waiting for the dispatch thread.
    pub queue_us: u64,
    pub resolve_us: u64,
    pub launch_us: u64,
    pub total_us: u64,
}

impl Timings {
    pub(crate) fn micros(d: Duration) -> u64 {
        u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
    }
}

/// A dispatch without its link: what may be written to disk. It carries
/// timings and fixed labels only, never URL contents, opener names or error
/// text (which can quote the URL).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchTiming {
    pub id: DispatchId,
    pub source: &'static str,
    pub forwarded: bool,
    pub outcome: Outcome,
    pub rewrites: usize,
    pub config_error: bool,
    pub timings: Timings,
}

impl DispatchRecord {
    /// The URL-free view of this dispatch, for logs.
    pub fn timing(&self) -> DispatchTiming {
        DispatchTiming {
            id: self.id,
            source: Source::label(self.link.source),
            forwarded: self.forwarded,
            outcome: self.outcome,
            rewrites: self
                .steps
                .iter()
                .filter(|s| matches!(s, Step::Rewrite { .. }))
                .count(),
            config_error: self
                .steps
                .iter()
                .any(|s| matches!(s, Step::ConfigError { .. })),
            timings: self.timings,
        }
    }
}

fn unix_millis<S: Serializer>(t: &SystemTime, s: S) -> Result<S::Ok, S::Error> {
    let ms = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0);
    s.serialize_u64(ms)
}
