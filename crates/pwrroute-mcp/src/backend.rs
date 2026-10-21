//! What the tools read and act on.
//!
//! Each tool is backed by one narrow trait. [`crate::live`] implements them
//! over the real router, engine and browser catalog; [`crate::fake`] has
//! in-memory stand-ins for tests, and [`crate::EngineResolver`] adapts the
//! engine alone.
//!
//! The traits are synchronous: the server calls them on a blocking thread,
//! so an implementation may wait on its own thread or lock without stalling
//! the link.

use std::sync::Arc;

use pwrroute_engine::{Browser, Opener, Privacy, RulesFile, TestReport};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Where a URL goes and why: each rewrite and handler that fired, then the
/// destination. The same trace backs Test a link, Activity and
/// `pwrroute_resolve`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Trace {
    /// The URL the rules matched, before any rewrite: the link as it
    /// arrived, or, for a short link, where it landed (`original_url` then
    /// holds the short link).
    pub url: String,
    /// The short link as it arrived, when `url` is where it landed. Absent
    /// when the link was not expanded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_url: Option<String>,
    pub steps: Vec<TraceStep>,
    /// The browser and how to open it. `browser.url` is the URL after every
    /// rewrite.
    #[schemars(with = "Value")]
    pub browser: Browser,
    /// Set when a matcher, rewrite or browser function threw; the browser is
    /// then the default, as in Finicky.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// What the config printed with `console` during this run, in order,
    /// when the backend resolves with the engine itself (the router's
    /// records carry none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub console: Vec<ConsoleLine>,
}

/// One `console` line a config printed: `level` is the method called,
/// `debug`, `log`, `warn` or `error`. The message is the config's own
/// words, which often quote the URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleLine {
    pub level: String,
    pub message: String,
}

impl From<&pwrroute_engine::ConsoleLine> for ConsoleLine {
    fn from(line: &pwrroute_engine::ConsoleLine) -> Self {
        Self {
            level: line.level.name().to_owned(),
            message: line.message.clone(),
        }
    }
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
        trace.original_url = trace.original_url.as_deref().map(host_of);
        trace.browser.url = host_of(&trace.browser.url);
        for step in &mut trace.steps {
            step.url = step.url.as_deref().map(host_of);
        }
        // A config's browser arguments often carry the URL.
        trace.browser.args.clear();
        // A function's error message can quote the URL it was given.
        if trace.error.is_some() {
            trace.error = Some("a rule failed; details need full-URL access".to_owned());
        }
        // And a config prints whatever it likes, the URL most of all.
        trace.console.clear();
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
/// The rules document is the file's own JSON ([`RulesFile`]: `defaultBrowser`,
/// `rewrites`, `rules` with their `examples`, and `tests`); the store
/// validates every write against that schema. A Finicky config's own
/// rewrites and handlers are not in it: they run first, from the config.
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

/// Runs `rules.json`'s saved examples and tests through the engine in
/// charge, and matches candidate patterns the way routing would. Nothing
/// here opens or writes anything.
pub trait RulesTester: Send + Sync + 'static {
    /// The saved examples and tests, through the config and rules the
    /// router routes by; or those of `proposal`, through a temporary engine
    /// built from the same config and the proposal in place of the saved
    /// rules.
    fn test(&self, proposal: Option<&RulesFile>) -> anyhow::Result<TestReport>;

    /// Which of `urls` `pattern` matches, by Finicky's own `isMatch`.
    fn collisions(&self, pattern: &str, urls: &[String]) -> anyhow::Result<Vec<String>>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RulesSnapshot {
    /// Changes on every write; `pwrroute_rules_apply` must name the revision
    /// it read.
    pub revision: String,
    pub source: RulesSource,
    /// The Finicky config that runs before these rules, when one is in
    /// charge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    /// `rules.json` as written: `defaultBrowser`, `rewrites`, `rules` and
    /// `tests`.
    pub rules: Value,
}

/// Where the router reads its rules from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum RulesSource {
    /// `rules.json`, editable. A Finicky config, when one is found, runs
    /// first and owns the default browser; `rules.json`'s rewrites and
    /// handlers follow its own.
    Visual,
    /// A Finicky `.js`/`.ts` config alone: `rules.json` is not in use and
    /// cannot be changed from here meanwhile.
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

/// Recent dispatches with their traces, newest first: the router's
/// in-memory buffer, and its on-disk journal when the user turned that on.
/// Every entry arrives already reduced to its rule's [`Privacy`].
pub trait ActivityLog: Send + Sync + 'static {
    /// From memory, newest first, at most `limit`.
    fn recent(&self, limit: usize) -> anyhow::Result<Vec<ActivityEntry>>;

    /// From the journal, newest first, at most `limit`. Empty when the
    /// journal is off.
    fn journal(&self, limit: usize) -> anyhow::Result<Vec<ActivityEntry>>;

    /// Where dispatches are kept and for how long, in the router's own
    /// words, shown verbatim.
    fn retention(&self) -> String;
}

/// Where an [`ActivityEntry`] was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ActivitySource {
    /// The router's buffer: gone when PwrRoute quits.
    Memory,
    /// The opt-in journal on disk.
    Journal,
}

/// How a dispatch ended, as Activity shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ActivityOutcome {
    Opened,
    DryRun,
    /// `browser: null`: deliberately not opened.
    NotOpened,
    Failed,
}

/// One dispatch, at the level its rule lets the router keep: a `full`
/// entry has the whole trace, a `host` entry has scheme and host in place
/// of every URL, and a `none` entry has no trace, no host and no opener,
/// only that it happened and how it ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    /// The dispatch's number in this PwrRoute run; a journaled entry from an
    /// earlier run can repeat one.
    pub id: u64,
    /// Milliseconds since the Unix epoch.
    pub at_ms: u64,
    /// The app the link came from, or the agent that opened it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    /// How much of the link this entry keeps: `full`, `host` or `none`.
    #[schemars(with = "String")]
    pub privacy: Privacy,
    pub source: ActivitySource,
    pub outcome: ActivityOutcome,
    /// No rule chose the browser: the default, or the fallback while no
    /// config was in charge. These are the links a new rule is for.
    pub unrouted: bool,
    /// A rewrite changed the URL.
    pub rewritten: bool,
    /// Opened in a private window.
    pub private: bool,
    /// Absent for a `none` entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<Trace>,
}

impl ActivityEntry {
    /// The same entry with every URL cut down to its host.
    #[must_use]
    pub fn hosts_only(&self) -> Self {
        let mut entry = self.clone();
        entry.trace = entry.trace.as_ref().map(Trace::hosts_only);
        entry
    }
}

/// Everything the tools are backed by.
#[derive(Clone)]
pub struct Backends {
    pub resolver: Arc<dyn Resolver>,
    pub router: Arc<dyn Router>,
    pub browsers: Arc<dyn BrowserCatalog>,
    pub rules: Arc<dyn RulesStore>,
    pub tester: Arc<dyn RulesTester>,
    pub activity: Arc<dyn ActivityLog>,
}

impl std::fmt::Debug for Backends {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backends").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use pwrroute_engine::AppType;

    use super::*;

    #[test]
    fn hosts_only_withholds_every_url() {
        let url = "https://docs.example.org/private?token=abc";
        let trace = Trace {
            url: url.to_owned(),
            original_url: Some("https://go.example/s3cr3t".to_owned()),
            steps: vec![TraceStep {
                kind: StepKind::Rewrite,
                index: Some(0),
                summary: "rewrite".to_owned(),
                url: Some(url.to_owned()),
            }],
            browser: Browser {
                name: "Firefox".to_owned(),
                app_type: AppType::AppName,
                url: url.to_owned(),
                profile: String::new(),
                args: vec!["--new-window".to_owned(), url.to_owned()],
                open_in_background: None,
                private: false,
                privacy: pwrroute_engine::Privacy::Full,
            },
            error: Some(format!("could not open {url}")),
            console: vec![ConsoleLine {
                level: "log".to_owned(),
                message: format!("saw {url}"),
            }],
        };
        let hosts = serde_json::to_string(&trace.hosts_only()).unwrap();
        assert!(!hosts.contains("/private"), "{hosts}");
        assert!(hosts.contains("docs.example.org"), "{hosts}");
        assert!(!hosts.contains("s3cr3t"), "{hosts}");
        assert!(hosts.contains(r#""originalUrl":"go.example""#), "{hosts}");
    }
}
