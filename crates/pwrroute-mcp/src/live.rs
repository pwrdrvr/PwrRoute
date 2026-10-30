//! The backends over the real thing: the router's handle, the browser
//! catalog and `rules.json` on disk. The app builds these in `mcp.rs`;
//! tests build them over a dry-run router, a fixture machine and a scratch
//! file, so nothing here reaches the OS on its own.
//!
//! Everything an agent sees comes from the router's own records, already
//! reduced to their rule's [`Privacy`] on the dispatch thread, so this
//! module cannot show more of a link than Activity does.

use std::{
    hash::{DefaultHasher, Hash as _, Hasher as _},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, anyhow};
use pwrroute_browsers::{Env, detect_installed, list_profiles};
use pwrroute_engine::{AppType, Browser, Opener, Privacy, RulesFile};
use pwrroute_os::DefaultStatus;
use pwrroute_router::{
    DispatchRecord, Link, Matched, OpenerInfo, Outcome, Owner, RouterHandle, Source, Step,
};
use serde_json::Value;

use crate::backend::{
    ActivityEntry, ActivityLog, ActivityOutcome, ActivitySource, AgentRef, BrowserCatalog,
    BrowserList, DetectedBrowser, Resolver, Router, RulesError, RulesSnapshot, RulesSource,
    RulesStore, StepKind, StepSource, Trace, TraceStep,
};

/// The router behind `pwrroute_resolve`, `pwrroute_open` and
/// `pwrroute_activity`: resolving is the router's Test a link, opening is a
/// dispatch, and activity is its buffer and journal.
#[derive(Clone)]
pub struct LiveRouter {
    handle: RouterHandle,
    config: Option<ConfigName>,
}

/// The config in charge as the app shows it ("~/.finicky.ts"), asked at
/// each call: a trace names the config's default after it.
pub type ConfigName = Arc<dyn Fn() -> Option<String> + Send + Sync>;

impl std::fmt::Debug for LiveRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveRouter")
            .field("handle", &self.handle)
            .finish_non_exhaustive()
    }
}

impl LiveRouter {
    pub fn new(handle: RouterHandle) -> Self {
        Self {
            handle,
            config: None,
        }
    }

    /// Names the config in charge in traces: its default is "~/.finicky.ts's
    /// default", as the app's views say it, not "the config's default".
    pub fn with_config(self, config: ConfigName) -> Self {
        Self {
            config: Some(config),
            ..self
        }
    }

    fn config(&self) -> Option<String> {
        self.config.as_ref().and_then(|config| config())
    }
}

impl Resolver for LiveRouter {
    fn resolve(&self, url: &str, opener: Option<&Opener>) -> Result<Trace> {
        let link = Link::new(url, Source::Api).with_opener(opener.map(opener_info));
        let record = self.handle.test(link)?;
        Ok(trace_of(&record, self.config().as_deref()))
    }
}

impl Router for LiveRouter {
    fn open(&self, url: &str, opener: Option<&Opener>, agent: &AgentRef) -> Result<Trace> {
        // Without an app to name, the link came from the agent: that is
        // what Activity shows, and what a rule's `opener` list sees.
        let opener = opener.map(opener_info).unwrap_or_else(|| OpenerInfo {
            name: agent.name.clone(),
            ..OpenerInfo::default()
        });
        let link = Link::new(url, Source::Api).with_opener(Some(opener));
        let record = self.handle.dispatch_and_wait(link)?;
        Ok(trace_of(&record, self.config().as_deref()))
    }
}

impl ActivityLog for LiveRouter {
    fn recent(&self, limit: usize) -> Result<Vec<ActivityEntry>> {
        let config = self.config();
        Ok(self
            .handle
            .recent()
            .iter()
            .take(limit)
            .map(|r| entry_of(r, ActivitySource::Memory, config.as_deref()))
            .collect())
    }

    fn journal(&self, limit: usize) -> Result<Vec<ActivityEntry>> {
        let config = self.config();
        Ok(self
            .handle
            .journal(limit)
            .context("reading the activity journal")?
            .iter()
            .map(|r| entry_of(r, ActivitySource::Journal, config.as_deref()))
            .collect())
    }

    fn retention(&self) -> String {
        self.handle.retention().description
    }
}

fn opener_info(opener: &Opener) -> OpenerInfo {
    OpenerInfo {
        name: opener.name.clone(),
        bundle_id: opener.bundle_id.clone(),
        path: opener.path.clone(),
        window_title: opener.window_title.clone(),
    }
}

/// A record as an entry. The record is already at its level: a `none`
/// record has no link, opener, browser or steps, so the entry has no trace
/// and no `via`. `config` names the config in charge, as in [`trace_of`].
pub fn entry_of(
    record: &DispatchRecord,
    source: ActivitySource,
    config: Option<&str>,
) -> ActivityEntry {
    let unrouted = record.steps.iter().any(|s| {
        matches!(
            s,
            Step::Matched {
                by: Matched::Default { .. }
            } | Step::Fallback { .. }
                | Step::Paused { .. }
                | Step::Sent { .. }
        )
    });
    let rewritten = record
        .steps
        .iter()
        .any(|s| matches!(s, Step::Rewrite { .. }));
    let private = record.browser.as_ref().is_some_and(|b| b.private);
    let trace = (record.privacy != Privacy::None).then(|| trace_of(record, config));
    ActivityEntry {
        id: record.id.0,
        at_ms: record
            .received_at
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
        via: record
            .link
            .opener
            .as_ref()
            .map(|o| o.name.clone())
            .filter(|name| !name.is_empty()),
        privacy: record.privacy,
        source,
        outcome: match record.outcome {
            Outcome::Opened => ActivityOutcome::Opened,
            Outcome::DryRun | Outcome::Tested => ActivityOutcome::DryRun,
            Outcome::NotOpened => ActivityOutcome::NotOpened,
            Outcome::Failed => ActivityOutcome::Failed,
        },
        unrouted,
        rewritten,
        private,
        trace,
    }
}

/// The router's record as the tools' trace: each rewrite that ran, then
/// what chose the browser, naming who decided: `rules.json`'s rule N, the
/// config's handler N, or whose default (PwrRoute's or the config's). A
/// record that does not say is not passed off as a rule. Error text comes
/// along as the record has it (withheld already for a `host` record).
/// `config` is the config in charge as the app shows it ("~/.finicky.ts"),
/// which names its default; without one it is "the config's default".
pub fn trace_of(record: &DispatchRecord, config: Option<&str>) -> Trace {
    let mut steps = Vec::new();
    let mut error = None;
    // A fallback after a launch failure is the chosen browser missing, not
    // a config out of charge.
    let mut launch_failed = false;
    let mut received = None;
    let mut expanded = None;
    // What chose the browser, for a pause that passed it over.
    let mut matched = None;
    let step = |kind, index, summary: String, source| TraceStep {
        kind,
        index,
        summary,
        url: None,
        source,
    };
    for each in &record.steps {
        match each {
            Step::Received { url } => received = Some(url.clone()),
            Step::Expanded { url } => expanded = Some(url.clone()),
            Step::Rewrite { rule, url } => steps.push(TraceStep {
                url: Some(url.clone()),
                ..match rule {
                    Some(n) => step(
                        StepKind::Rewrite,
                        Some(*n),
                        format!("config rewrite {} matched", n + 1),
                        Some(StepSource::Config),
                    ),
                    None => step(
                        StepKind::Rewrite,
                        None,
                        "a rules.json rewrite matched".to_owned(),
                        Some(StepSource::Rules),
                    ),
                }
            }),
            Step::Matched { by } => {
                matched = Some(*by);
                steps.push(match by {
                    Matched::Handler { index } => step(
                        StepKind::Handler,
                        Some(*index),
                        format!("config handler {} matched", index + 1),
                        Some(StepSource::Config),
                    ),
                    Matched::Rule { index } => step(
                        StepKind::Handler,
                        Some(*index),
                        format!("rules.json rule {} matched", index + 1),
                        Some(StepSource::Rules),
                    ),
                    Matched::Default {
                        owner: Some(Owner::PwrRoute),
                    } => step(
                        StepKind::Default,
                        None,
                        crate::engine::DEFAULT_PWRROUTE.to_owned(),
                        Some(StepSource::Rules),
                    ),
                    Matched::Default {
                        owner: Some(Owner::Config),
                    } => step(
                        StepKind::Default,
                        None,
                        crate::engine::default_config(config),
                        Some(StepSource::Config),
                    ),
                    Matched::Default { owner: None } => step(
                        StepKind::Default,
                        None,
                        "no rule or handler matched: the default browser".to_owned(),
                        None,
                    ),
                    Matched::Unreported => step(
                        StepKind::Handler,
                        None,
                        "a rule or handler matched; the router did not say which".to_owned(),
                        None,
                    ),
                });
            }
            Step::Fallback { reason } => steps.push(step(
                StepKind::Default,
                None,
                if launch_failed {
                    format!("the fallback browser: {reason}")
                } else {
                    format!("no config in charge: the fallback browser ({reason})")
                },
                None,
            )),
            Step::RefusedLoop { browser } => steps.push(step(
                StepKind::Default,
                None,
                format!("{browser} would send the link back: the fallback browser"),
                None,
            )),
            // The step that chose the browser says which list the skipped
            // rule is in; the pause's own number is the config's.
            Step::Paused { skipped } => steps.push(held_step(
                "routing is paused",
                "the pause target instead",
                matched,
                *skipped,
            )),
            Step::Sent { sent, skipped } => {
                let why = format!("the user sent this link ({})", sent.label());
                steps.push(held_step(
                    &why,
                    "their destination instead",
                    matched,
                    *skipped,
                ));
            }
            Step::Held { held, skipped, .. } => {
                let why = format!("the user held this link ({})", held.label());
                steps.push(held_step(
                    &why,
                    "nothing opened; it waits to be opened",
                    matched,
                    *skipped,
                ));
            }
            Step::Released { from, chosen } => steps.push(step(
                StepKind::Default,
                None,
                format!(
                    "not opened as dispatch {}; opened later {}",
                    from.0,
                    if *chosen {
                        "where the user chose"
                    } else {
                        "as the rules say now"
                    }
                ),
                None,
            )),
            Step::LaunchFailed { message } => {
                launch_failed = true;
                error.get_or_insert_with(|| message.clone());
            }
            Step::ConfigError { message } | Step::Rejected { reason: message } => {
                error.get_or_insert_with(|| message.clone());
            }
            Step::Browser { .. } | Step::NotOpened | Step::Launched { .. } => {}
        }
    }
    let browser = record.browser.clone().unwrap_or_else(|| Browser {
        name: String::new(),
        app_type: AppType::None,
        url: record.final_url.clone().unwrap_or_default(),
        profile: String::new(),
        args: Vec::new(),
        open_in_background: None,
        private: false,
        privacy: record.privacy,
    });
    // A short link was followed: the rules matched where it landed, and
    // the trace says what arrived.
    let (url, original_url) = match expanded {
        Some(landed) => (
            landed,
            Some(received.unwrap_or_else(|| record.link.url.clone())),
        ),
        None => (record.link.url.clone(), None),
    };
    Trace {
        url,
        original_url,
        steps,
        browser,
        error,
        // The router keeps a record, not the engine's trace: no console.
        console: Vec::new(),
    }
}

/// The browsers on this machine, from the catalog, and whether the OS hands
/// links to PwrRoute. `default` is `pwrroute_os::default_browser_status` in
/// the app; tests pass a fixed answer.
pub struct LiveBrowsers {
    env: Env,
    default: Box<dyn Fn() -> Option<DefaultStatus> + Send + Sync>,
}

impl std::fmt::Debug for LiveBrowsers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveBrowsers").finish_non_exhaustive()
    }
}

impl LiveBrowsers {
    pub fn new(
        env: Env,
        default: impl Fn() -> Option<DefaultStatus> + Send + Sync + 'static,
    ) -> Self {
        Self {
            env,
            default: Box::new(default),
        }
    }
}

impl BrowserCatalog for LiveBrowsers {
    fn browsers(&self) -> Result<BrowserList> {
        let browsers = detect_installed(&self.env)
            .into_iter()
            .map(|installed| DetectedBrowser {
                name: installed.name.clone(),
                bundle_id: installed.bundle_id.clone(),
                path: Some(installed.location.display().to_string()),
                profiles: installed.browser.map_or_else(Vec::new, |info| {
                    list_profiles(info, &self.env)
                        .into_iter()
                        .map(|p| p.name)
                        .collect()
                }),
                hidden: false,
            })
            .collect();
        let status = (self.default)();
        Ok(BrowserList {
            browsers,
            pwrroute_is_default: status.as_ref().is_some_and(|s| s.is_default),
            os_default: status.and_then(|s| s.http.or(s.https)),
        })
    }
}

/// `rules.json` on disk. The revision is a hash of the file's bytes, so a
/// write by the Rules screen or by hand changes it too and an agent's
/// stale `baseRevision` is refused. Writes go through [`RulesFile::save`]:
/// a sibling file renamed into place.
pub struct LiveRules {
    path: PathBuf,
    /// The Finicky config in charge right now, when one is.
    config: Arc<dyn Fn() -> Option<PathBuf> + Send + Sync>,
    /// Held from the revision check to the write, and across a read, so
    /// two agents writing at once cannot both pass the check and the
    /// second clobber the first.
    lock: Mutex<()>,
}

impl std::fmt::Debug for LiveRules {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveRules")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl LiveRules {
    pub fn new(
        path: impl Into<PathBuf>,
        config: impl Fn() -> Option<PathBuf> + Send + Sync + 'static,
    ) -> Self {
        Self {
            path: path.into(),
            config: Arc::new(config),
            lock: Mutex::new(()),
        }
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The file and its revision, read under the lock so they match.
    fn snapshot(&self) -> Result<RulesSnapshot> {
        let file = RulesFile::load(&self.path)?;
        Ok(RulesSnapshot {
            revision: revision_of(&self.path),
            source: RulesSource::Visual,
            config: (self.config)().map(|p| p.display().to_string()),
            rules: serde_json::to_value(file)?,
        })
    }
}

/// A short hash of the file as it is: `0` when there is no file yet.
fn revision_of(path: &Path) -> String {
    match std::fs::read(path) {
        Ok(bytes) => {
            let mut hasher = DefaultHasher::new();
            bytes.hash(&mut hasher);
            format!("{:016x}", hasher.finish())
        }
        Err(_) => "0".to_owned(),
    }
}

impl RulesStore for LiveRules {
    fn read(&self) -> Result<RulesSnapshot> {
        let _held = self.locked();
        self.snapshot()
    }

    fn write(
        &self,
        base_revision: &str,
        rules: Value,
        dry_run: bool,
    ) -> std::result::Result<RulesSnapshot, RulesError> {
        let _held = self.locked();
        let current = revision_of(&self.path);
        if current != base_revision {
            return Err(RulesError::Conflict {
                base: base_revision.to_owned(),
                current,
            });
        }
        let validated = crate::fake::validate(rules)?;
        if dry_run {
            let mut snapshot = self.snapshot()?;
            snapshot.rules = validated;
            return Ok(snapshot);
        }
        let file: RulesFile = serde_json::from_value(validated)
            .map_err(|e| RulesError::Other(anyhow!("re-reading the validated rules: {e}")))?;
        file.save(&self.path)
            .map_err(|e| RulesError::Other(e.context("writing rules.json")))?;
        Ok(self.snapshot()?)
    }
}

/// Milliseconds since the epoch, for callers that build entries by hand.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// A pause's or a send's step: what it passed over, named from the step
/// that chose the browser (which says which list), else the config's
/// handler number it carries.
fn held_step(
    why: &str,
    instead: &str,
    matched: Option<Matched>,
    skipped: Option<usize>,
) -> TraceStep {
    let (index, summary, source) = match (matched, skipped) {
        (Some(Matched::Rule { index }), _) => (
            Some(index),
            format!("{why}: rules.json rule {} skipped, {instead}", index + 1),
            Some(StepSource::Rules),
        ),
        (Some(Matched::Handler { index }), _) | (_, Some(index)) => (
            Some(index),
            format!("{why}: config handler {} skipped, {instead}", index + 1),
            Some(StepSource::Config),
        ),
        (_, None) => (
            None,
            format!("{why}: every rule and handler skipped, {instead}"),
            None,
        ),
    };
    TraceStep {
        kind: StepKind::Default,
        index,
        summary,
        url: None,
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use pwrroute_browsers::Platform;
    use pwrroute_engine::{Expectation, RouteTest, Rule};
    use pwrroute_router::{DryRunLauncher, Options, Resolved, Router as RealRouter};
    use serde_json::json;

    use super::*;

    const BANK: &str = "https://bank.example.com/account?token=hunter2";
    const PUBLIC: &str = "https://docs.example.org/guide?draft=1";

    /// A resolver that keeps the bank's host only and a secret link not at
    /// all, as `rules.json` rules with `privacy` would.
    struct Levels;

    impl pwrroute_router::Resolver for Levels {
        fn resolve(
            &self,
            link: &pwrroute_engine::Link,
            _opener: Option<&Opener>,
        ) -> Result<Resolved> {
            let url = link.url.as_str();
            let host = url::Url::parse(url)?
                .host_str()
                .unwrap_or_default()
                .to_owned();
            let privacy = match host.as_str() {
                "bank.example.com" => Privacy::Host,
                "secret.example.com" => Privacy::None,
                _ => Privacy::Full,
            };
            Ok(Resolved {
                browser: Browser {
                    name: "Firefox".into(),
                    app_type: AppType::AppName,
                    url: url.to_owned(),
                    profile: String::new(),
                    args: Vec::new(),
                    open_in_background: None,
                    private: privacy == Privacy::Host,
                    privacy,
                },
                error: None,
                rule_steps: vec![Step::Matched {
                    by: Matched::Default { owner: None },
                }],
                privacy,
            })
        }
    }

    fn router() -> RealRouter {
        RealRouter::spawn(
            Options::default(),
            Some(Box::new(|| Ok(Box::new(Levels)))),
            DryRunLauncher::new(),
        )
        .unwrap()
    }

    /// Fails Firefox, as the launcher does for an app that is not installed.
    struct NoFirefox;

    impl pwrroute_router::Launcher for NoFirefox {
        fn launch(&mut self, browser: &Browser) -> Result<()> {
            if browser.name == "Firefox" {
                anyhow::bail!("browser \"Firefox\" is not installed");
            }
            Ok(())
        }
    }

    #[test]
    fn the_trace_tells_a_missing_browser_from_a_config_out_of_charge() {
        let summaries = |record: &DispatchRecord| -> Vec<String> {
            trace_of(record, None)
                .steps
                .into_iter()
                .map(|step| step.summary)
                .collect()
        };
        let link = || Link::new(PUBLIC, Source::CommandLine);

        // A fixed fallback: on Linux the OS's own browser is Firefox, the
        // very app this test makes missing.
        let router = RealRouter::spawn(
            Options {
                fallback: pwrroute_router::FallbackSource::fixed("Brave", ""),
                ..Options::default()
            },
            Some(Box::new(|| Ok(Box::new(Levels)))),
            NoFirefox,
        )
        .unwrap();
        let record = router.handle().dispatch_and_wait(link()).unwrap();
        assert_eq!(record.outcome, Outcome::Opened);
        let trace = trace_of(&record, None);
        assert_eq!(
            trace.error.as_deref(),
            Some("browser \"Firefox\" is not installed")
        );
        assert_eq!(
            summaries(&record).last().map(String::as_str),
            Some("the fallback browser: Firefox could not be opened")
        );

        let router = RealRouter::spawn(
            Options::default(),
            Some(Box::new(|| Err(anyhow!("the config did not load")))),
            DryRunLauncher::new(),
        )
        .unwrap();
        let record = router.handle().dispatch_and_wait(link()).unwrap();
        assert_eq!(
            summaries(&record),
            ["no config in charge: the fallback browser (the config did not load)"]
        );
    }

    #[test]
    fn the_router_resolves_opens_and_lists_at_each_level() {
        let router = router();
        let live = LiveRouter::new(router.handle());
        let agent = AgentRef {
            name: "claude-code".into(),
            connection: 1,
        };

        let trace = live.resolve(PUBLIC, None).unwrap();
        assert_eq!(trace.url, PUBLIC);
        assert_eq!(trace.browser.name, "Firefox");
        assert_eq!(trace.steps[0].kind, StepKind::Default);
        assert!(
            live.recent(10).unwrap().is_empty(),
            "a resolve is not recorded"
        );

        live.open(PUBLIC, None, &agent).unwrap();
        live.open(BANK, None, &agent).unwrap();
        live.open("https://secret.example.com/x", None, &agent)
            .unwrap();
        let entries = live.recent(10).unwrap();
        assert_eq!(entries.len(), 3);

        let secret = &entries[0];
        assert_eq!(secret.privacy, Privacy::None);
        assert!(secret.trace.is_none() && secret.via.is_none());
        let json = serde_json::to_string(secret).unwrap();
        assert!(
            !json.contains("secret") && !json.contains("claude"),
            "{json}"
        );

        let bank = &entries[1];
        assert_eq!(bank.privacy, Privacy::Host);
        assert!(bank.private && bank.unrouted);
        let trace = bank.trace.as_ref().unwrap();
        assert_eq!(trace.url, "https://bank.example.com");
        assert_eq!(trace.browser.url, "https://bank.example.com");
        let json = serde_json::to_string(bank).unwrap();
        assert!(
            !json.contains("hunter2") && !json.contains("/account"),
            "{json}"
        );

        let public = &entries[2];
        assert_eq!(public.privacy, Privacy::Full);
        assert_eq!(public.via.as_deref(), Some("claude-code"));
        assert_eq!(public.trace.as_ref().unwrap().url, PUBLIC);
        assert_eq!(public.outcome, ActivityOutcome::DryRun);

        assert!(live.journal(10).unwrap().is_empty(), "the journal is off");
        assert!(live.retention().starts_with("Kept in memory only"));
    }

    /// Sends `bit.ly/gh` to GitHub; anything else is offline.
    struct GitHubShortener;

    impl pwrroute_engine::Network for GitHubShortener {
        fn request(
            &self,
            _method: pwrroute_engine::Method,
            url: &str,
            _timeout: std::time::Duration,
        ) -> Result<pwrroute_engine::Reply> {
            match url {
                "https://bit.ly/gh" => Ok(pwrroute_engine::Reply {
                    status: 301,
                    location: Some("https://github.com/pwrdrvr/x".into()),
                }),
                "https://github.com/pwrdrvr/x" => Ok(pwrroute_engine::Reply {
                    status: 200,
                    location: None,
                }),
                _ => Err(anyhow::anyhow!("offline")),
            }
        }
    }

    #[test]
    fn a_short_link_resolve_reports_where_it_landed_and_what_arrived() {
        let rules = RulesFile {
            default_browser: "Safari".into(),
            rules: vec![Rule {
                patterns: vec!["github.com/*".into()],
                browser: "Firefox".into(),
                ..Rule::default()
            }],
            ..RulesFile::default()
        };
        let router = RealRouter::spawn(
            Options {
                network: Some(std::sync::Arc::new(GitHubShortener)),
                ..Options::default()
            },
            Some(Box::new(move || {
                let engine = pwrroute_engine::Engine::from_sources(
                    &pwrroute_engine::Sources {
                        config: None,
                        rules,
                    },
                    std::rc::Rc::new(pwrroute_engine::DefaultHost),
                    pwrroute_engine::Limits::default(),
                )?;
                Ok(Box::new(engine))
            })),
            DryRunLauncher::new(),
        )
        .unwrap();
        let live = LiveRouter::new(router.handle());
        let trace = live.resolve("https://bit.ly/gh", None).unwrap();
        assert_eq!(trace.url, "https://github.com/pwrdrvr/x");
        assert_eq!(trace.original_url.as_deref(), Some("https://bit.ly/gh"));
        assert_eq!(trace.browser.name, "Firefox");
        assert_eq!(trace.browser.url, "https://github.com/pwrdrvr/x");
        assert_eq!(
            trace.steps.len(),
            1,
            "the expansion is not a step: {trace:?}"
        );
        assert_eq!(trace.steps[0].kind, StepKind::Handler);

        let plain = live.resolve("https://github.com/direct", None).unwrap();
        assert_eq!(plain.url, "https://github.com/direct");
        assert_eq!(plain.original_url, None);
    }

    #[test]
    fn rules_on_disk_have_a_revision_that_follows_the_file() {
        let dir =
            std::env::temp_dir().join(format!("pwrroute-mcp-live-rules-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rules.json");
        let rules = LiveRules::new(path.clone(), || None);

        let empty = rules.read().unwrap();
        assert_eq!(empty.revision, "0");
        assert_eq!(empty.source, RulesSource::Visual);

        let file = RulesFile {
            default_browser: "Safari".into(),
            rules: vec![Rule {
                patterns: vec!["*.example.org/*".into()],
                browser: "Firefox".into(),
                ..Rule::default()
            }],
            tests: vec![RouteTest {
                url: "https://example.net/".into(),
                opener: None,
                expect: Expectation {
                    browser: "Safari".into(),
                    ..Expectation::default()
                },
            }],
            ..RulesFile::default()
        };
        let written = rules
            .write("0", serde_json::to_value(&file).unwrap(), false)
            .unwrap();
        assert_ne!(written.revision, "0");
        assert_eq!(RulesFile::load(&path).unwrap(), file);

        // A hand edit changes the revision; a stale write is refused.
        fs::write(&path, r#"{ "defaultBrowser": "Firefox", "rules": [] }"#).unwrap();
        let stale = rules.write(&written.revision, json!({ "rules": [] }), false);
        assert!(matches!(stale, Err(RulesError::Conflict { .. })));

        // A broken document is refused before anything is written.
        let now = rules.read().unwrap();
        let bad = rules.write(&now.revision, json!({ "rules": "x" }), false);
        assert!(matches!(bad, Err(RulesError::Invalid(_))));
        let incomplete = rules.write(
            &now.revision,
            json!({ "rules": [{ "match": "a.example/*" }] }),
            false,
        );
        assert!(
            matches!(incomplete, Err(RulesError::Invalid(_))),
            "{incomplete:?}"
        );
        assert_eq!(rules.read().unwrap().revision, now.revision);

        // A dry run validates and writes nothing.
        let dry = rules
            .write(
                &now.revision,
                json!({ "defaultBrowser": "Arc", "rules": [] }),
                true,
            )
            .unwrap();
        assert_eq!(dry.rules["defaultBrowser"], "Arc");
        assert_eq!(rules.read().unwrap().revision, now.revision);
        fs::remove_dir_all(dir).unwrap();
    }

    /// Two agents (Claude Code and Codex, say) each read the revision and
    /// write. Without the lock both can pass the check and one write is
    /// lost; with it the loser sees a conflict, reads again and keeps the
    /// winner's rule.
    #[test]
    fn concurrent_writes_are_serialized_so_none_is_lost() {
        const WRITERS: usize = 4;
        const WRITES: usize = 5;
        let dir = std::env::temp_dir().join(format!(
            "pwrroute-mcp-live-rules-concurrent-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rules.json");
        let rules = Arc::new(LiveRules::new(path.clone(), || None));
        let start = Arc::new(std::sync::Barrier::new(WRITERS));
        let writers: Vec<_> = (0..WRITERS)
            .map(|writer| {
                let rules = Arc::clone(&rules);
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    for n in 0..WRITES {
                        let pattern = format!("w{writer}-{n}.example/*");
                        loop {
                            let snapshot = rules.read().unwrap();
                            let mut doc = snapshot.rules;
                            doc["defaultBrowser"] = json!("Safari");
                            let list = doc["rules"].as_array_mut().unwrap();
                            list.push(json!({ "match": pattern, "browser": "Firefox" }));
                            match rules.write(&snapshot.revision, doc, false) {
                                Ok(_) => break,
                                Err(RulesError::Conflict { .. }) => continue,
                                Err(e) => panic!("{e:?}"),
                            }
                        }
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        let file = RulesFile::load(&path).unwrap();
        assert_eq!(file.rules.len(), WRITERS * WRITES, "every write landed");
        assert_eq!(rules.read().unwrap().revision, revision_of(&path));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_catalog_reads_a_fixture_machine_and_the_given_default() {
        let dir =
            std::env::temp_dir().join(format!("pwrroute-mcp-live-browsers-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let browsers = LiveBrowsers::new(Env::fixture(Platform::MacOs, &dir), || {
            Some(DefaultStatus {
                is_default: false,
                http: Some("com.apple.Safari".into()),
                https: Some("com.apple.Safari".into()),
            })
        });
        let list = browsers.browsers().unwrap();
        assert!(list.browsers.is_empty(), "an empty fixture machine");
        assert!(!list.pwrroute_is_default);
        assert_eq!(list.os_default.as_deref(), Some("com.apple.Safari"));
        fs::remove_dir_all(dir).unwrap();
    }

    /// Attribution (WP-B): a record's trace names who decided, by list and
    /// number, and whose default took an unclaimed link; a record that did
    /// not say is not called a rule.
    #[test]
    fn the_trace_names_who_routed_a_record() {
        use pwrroute_router::{DispatchId, Timings};
        let url = "https://docs.example.com/a";
        let record = |steps: Vec<Step>| DispatchRecord {
            id: DispatchId(7),
            received_at: SystemTime::now(),
            privacy: Privacy::Full,
            link: Link::new(url, Source::CommandLine),
            forwarded: false,
            final_url: Some(url.into()),
            browser: None,
            steps,
            outcome: Outcome::DryRun,
            timings: Timings::default(),
        };
        let last = |steps: Vec<Step>| {
            let trace = trace_of(&record(steps), None);
            let step = trace.steps.last().expect("a step").clone();
            (step.kind, step.index, step.source, step.summary)
        };
        let matched = |by| Step::Matched { by };

        assert_eq!(
            last(vec![matched(Matched::Rule { index: 2 })]),
            (
                StepKind::Handler,
                Some(2),
                Some(StepSource::Rules),
                "rules.json rule 3 matched".to_owned()
            )
        );
        assert_eq!(
            last(vec![matched(Matched::Handler { index: 1 })]),
            (
                StepKind::Handler,
                Some(1),
                Some(StepSource::Config),
                "config handler 2 matched".to_owned()
            )
        );
        assert_eq!(
            last(vec![matched(Matched::Default {
                owner: Some(Owner::PwrRoute)
            })]),
            (
                StepKind::Default,
                None,
                Some(StepSource::Rules),
                "no rule or handler matched: PwrRoute's default (rules.json's defaultBrowser)"
                    .to_owned()
            )
        );
        assert_eq!(
            last(vec![matched(Matched::Default {
                owner: Some(Owner::Config)
            })]),
            (
                StepKind::Default,
                None,
                Some(StepSource::Config),
                "no handler or rule matched: the config's default (its defaultBrowser)".to_owned()
            )
        );
        // With the config in charge named, its default goes by its path,
        // as the app's views say it.
        let config_default = record(vec![matched(Matched::Default {
            owner: Some(Owner::Config),
        })]);
        assert_eq!(
            trace_of(&config_default, Some("~/.finicky.ts"))
                .steps
                .last()
                .map(|step| step.summary.clone()),
            Some(
                "no handler or rule matched: ~/.finicky.ts's default (its defaultBrowser)"
                    .to_owned()
            )
        );
        // A record from before owners: a default, nobody's in particular.
        assert_eq!(
            last(vec![matched(Matched::Default { owner: None })]).3,
            "no rule or handler matched: the default browser"
        );
        // Unreported is not passed off as a rules.json rule.
        let (kind, index, source, summary) = last(vec![matched(Matched::Unreported)]);
        assert_eq!((kind, index, source), (StepKind::Handler, None, None));
        assert!(!summary.contains("rules.json"), "{summary}");
        assert_eq!(
            summary,
            "a rule or handler matched; the router did not say which"
        );

        // Paused: the skipped rule is named from its own list.
        assert_eq!(
            last(vec![
                matched(Matched::Rule { index: 0 }),
                Step::Paused { skipped: None },
            ]),
            (
                StepKind::Default,
                Some(0),
                Some(StepSource::Rules),
                "routing is paused: rules.json rule 1 skipped, the pause target instead".to_owned()
            )
        );
        assert_eq!(
            last(vec![
                matched(Matched::Handler { index: 3 }),
                Step::Paused { skipped: Some(3) },
            ])
            .3,
            "routing is paused: config handler 4 skipped, the pause target instead"
        );
        assert_eq!(
            last(vec![
                matched(Matched::Default {
                    owner: Some(Owner::PwrRoute)
                }),
                Step::Paused { skipped: None },
            ])
            .3,
            "routing is paused: every rule and handler skipped, the pause target instead"
        );
        // Sent: the same, in the user's words and with the count.
        assert_eq!(
            last(vec![
                matched(Matched::Rule { index: 1 }),
                Step::Sent {
                    sent: pwrroute_router::Sent::Next { nth: 1, of: 3 },
                    skipped: None,
                },
            ]),
            (
                StepKind::Default,
                Some(1),
                Some(StepSource::Rules),
                "the user sent this link (next link 1 of 3): rules.json rule 2 skipped, \
                 their destination instead"
                    .to_owned()
            )
        );
        assert_eq!(
            last(vec![
                matched(Matched::Default { owner: None }),
                Step::Sent {
                    sent: pwrroute_router::Sent::All,
                    skipped: None,
                },
            ])
            .3,
            "the user sent this link (all links): every rule and handler skipped, \
             their destination instead"
        );

        // On the wire: `source` says whose list `index` counts.
        let json = serde_json::to_value(trace_of(
            &record(vec![matched(Matched::Rule { index: 4 })]),
            None,
        ))
        .unwrap();
        assert_eq!(json["steps"][0]["source"], "rules");
        assert_eq!(json["steps"][0]["index"], 4);
        let json =
            serde_json::to_value(trace_of(&record(vec![matched(Matched::Unreported)]), None))
                .unwrap();
        assert!(json["steps"][0].get("source").is_none(), "{json}");
    }
}
