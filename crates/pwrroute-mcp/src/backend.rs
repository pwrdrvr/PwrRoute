//! What the tools read and act on.
//!
//! Each tool is backed by one narrow trait. The engine exists today and
//! [`crate::EngineResolver`] adapts it; the router, browser launcher, rules
//! store and activity log are being built elsewhere, so they appear here only
//! as the calls the tools need, with in-memory fakes in [`crate::fake`].
//!
//! The traits are synchronous: the server calls them on a blocking thread,
//! so an implementation may wait on its own thread or lock without stalling
//! the HTTP listener.

use std::sync::Arc;

use pwrroute_engine::{Browser, Opener};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Where a URL goes and why: each rewrite and handler that fired, then the
/// destination. The same trace backs Test a link, Activity and
/// `pwrroute_resolve`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Trace {
    /// The URL as it arrived, before any rewrite.
    pub url: String,
    pub steps: Vec<TraceStep>,
    /// The browser and how to open it. `browser.url` is the URL after every
    /// rewrite.
    #[schemars(with = "Value")]
    pub browser: Browser,
    /// Set when a matcher, rewrite or browser function threw; the browser is
    /// then the default, as in Finicky.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// One step of a [`Trace`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TraceStep {
    pub kind: StepKind,
    /// The rule's position in its list, when the source can say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    /// What the rule matched or did, as the Rules view words it.
    pub summary: String,
    /// The URL after this step, for a rewrite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum StepKind {
    Rewrite,
    Handler,
    Default,
}

impl Trace {
    /// The same trace with every URL cut down to its host, for a caller
    /// that may see where links went but not the links themselves.
    #[must_use]
    pub fn hosts_only(&self) -> Self {
        let mut trace = self.clone();
        trace.url = host_of(&trace.url);
        trace.browser.url = host_of(&trace.browser.url);
        for step in &mut trace.steps {
            step.url = step.url.as_deref().map(host_of);
        }
        // A function's error message can quote the URL it was given.
        if trace.error.is_some() {
            trace.error = Some("a rule failed; details need full-URL access".to_owned());
        }
        trace
    }
}

/// The host of `url`, or an empty string when it has none. Never the input
/// itself: that is the URL being withheld.
pub(crate) fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The agent a call came from, as Activity records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentRef {
    /// The approved client name, e.g. `claude-code`.
    pub name: String,
    /// Which of its connections, counted from 1 since PwrRoute started.
    pub connection: u64,
}

/// Resolves a URL without opening anything.
pub trait Resolver: Send + Sync + 'static {
    fn resolve(&self, url: &str, opener: Option<&Opener>) -> anyhow::Result<Trace>;
}

/// The router: dispatches a URL as a click would, and records it in
/// Activity against `agent`.
pub trait Router: Send + Sync + 'static {
    fn open(&self, url: &str, opener: Option<&Opener>, agent: &AgentRef) -> anyhow::Result<Trace>;
}

/// Detected browsers and the OS default.
pub trait BrowserCatalog: Send + Sync + 'static {
    fn browsers(&self) -> anyhow::Result<BrowserList>;
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BrowserList {
    pub browsers: Vec<DetectedBrowser>,
    /// Whether the OS hands http and https links to PwrRoute.
    pub pwrroute_is_default: bool,
    /// The app the OS hands links to, when it can say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_default: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DetectedBrowser {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub profiles: Vec<String>,
    /// Hidden from pickers, but rules that name it still resolve.
    pub hidden: bool,
}

/// The visual rules (`rules.json`).
///
/// The rules document is opaque here: its schema belongs to the store, which
/// validates every write. Function matchers and transformers from a Finicky
/// config appear in it marked opaque, as the Rules view shows them.
pub trait RulesStore: Send + Sync + 'static {
    fn read(&self) -> anyhow::Result<RulesSnapshot>;

    /// Validates `rules` and, unless `dry_run`, writes it, but only if the
    /// stored revision is still `base_revision`.
    fn write(
        &self,
        base_revision: &str,
        rules: Value,
        dry_run: bool,
    ) -> Result<RulesSnapshot, RulesError>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RulesSnapshot {
    /// Changes on every write; `pwrroute_rules_apply` must name the revision
    /// it read.
    pub revision: String,
    pub source: RulesSource,
    /// Rewrites, handlers and the default, as the visual model.
    pub rules: Value,
}

/// Where the router reads its rules from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum RulesSource {
    /// `rules.json`, editable.
    Visual,
    /// A Finicky `.js`/`.ts` config. It runs first and owns the default
    /// browser, so `rules.json` cannot be changed from here meanwhile.
    FinickyConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum RulesError {
    #[error("a Finicky config is selected; rules.json cannot be changed until visual rules are")]
    FinickyConfigSelected,
    #[error("the rules changed since revision {base}; read them again (now {current})")]
    Conflict { base: String, current: String },
    #[error("the rules are not valid: {0}")]
    Invalid(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Recent dispatches with their traces, newest first. Held in memory only.
pub trait ActivityLog: Send + Sync + 'static {
    fn recent(&self, limit: usize) -> anyhow::Result<Vec<ActivityEntry>>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    /// Milliseconds since the Unix epoch.
    pub at_ms: u64,
    /// The app the link came from, or the agent that opened it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    pub trace: Trace,
}

/// Everything the tools are backed by.
#[derive(Clone)]
pub struct Backends {
    pub resolver: Arc<dyn Resolver>,
    pub router: Arc<dyn Router>,
    pub browsers: Arc<dyn BrowserCatalog>,
    pub rules: Arc<dyn RulesStore>,
    pub activity: Arc<dyn ActivityLog>,
}

impl std::fmt::Debug for Backends {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backends").finish_non_exhaustive()
    }
}
