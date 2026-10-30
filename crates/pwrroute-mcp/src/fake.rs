//! In-memory stand-ins for the backends, and an approver that answers
//! without a window. Tests use them; the app runs [`crate::live`].

use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use pwrroute_engine::{
    AppType, Browser, Expectation, Limits, Opener, Privacy, RouteTest, Rule, RulesFile,
};
use serde_json::Value;

use crate::{
    approval::{ApprovalDecision, ApprovalReply, ApprovalRequest, Approver},
    backend::{
        ActivityEntry, ActivityLog, ActivityOutcome, ActivitySource, AgentRef, Backends,
        BrowserCatalog, BrowserList, DetectedBrowser, Resolver, Router, RulesError, RulesSnapshot,
        RulesSource, RulesStore, StepKind, StepSource, Trace, TraceStep,
    },
    engine::EngineTester,
    roles::Role,
};

/// Sends every `*.example.org` link to Firefox and the rest to Safari, and
/// rewrites `x.com` to `xcancel.com` first. Contrived, like the app's
/// fixture data, and the same routing [`FakeRules::default`] writes down.
#[derive(Debug, Default)]
pub struct FakeResolver;

impl Resolver for FakeResolver {
    fn resolve(&self, url: &str, _opener: Option<&Opener>) -> anyhow::Result<Trace> {
        let mut parsed = url::Url::parse(url)?;
        let mut steps = Vec::new();
        if parsed.host_str() == Some("x.com") {
            parsed.set_host(Some("xcancel.com"))?;
            steps.push(TraceStep {
                kind: StepKind::Rewrite,
                index: Some(0),
                summary: "x.com/* → host xcancel.com".to_owned(),
                url: Some(parsed.to_string()),
                source: Some(StepSource::Rules),
            });
        }
        let host = parsed.host_str().unwrap_or_default();
        let name = if host == "example.org" || host.ends_with(".example.org") {
            steps.push(TraceStep {
                kind: StepKind::Handler,
                index: Some(0),
                summary: "*.example.org/* → Firefox".to_owned(),
                url: None,
                source: Some(StepSource::Rules),
            });
            "Firefox"
        } else {
            steps.push(TraceStep {
                kind: StepKind::Default,
                index: None,
                summary: "default → Safari".to_owned(),
                url: None,
                source: Some(StepSource::Rules),
            });
            "Safari"
        };
        // A link to the bank keeps its host only, as a rule's `privacy`
        // would have it; the router reduces such a record before anyone
        // sees it, and so does this fake.
        let privacy = match host {
            "bank.example.org" => Privacy::Host,
            "secret.example.org" => Privacy::None,
            _ => Privacy::Full,
        };
        Ok(Trace {
            url: url.to_owned(),
            original_url: None,
            steps,
            browser: Browser {
                name: name.to_owned(),
                app_type: AppType::AppName,
                url: parsed.to_string(),
                profile: String::new(),
                args: Vec::new(),
                open_in_background: None,
                private: host == "bank.example.org",
                privacy,
            },
            error: None,
            console: Vec::new(),
        })
    }
}

/// A router that resolves with [`FakeResolver`], opens nothing, and keeps
/// what it "opened" in memory for [`FakeActivity`] and for assertions.
#[derive(Debug, Default)]
pub struct FakeRouter {
    opened: Mutex<Vec<(AgentRef, ActivityEntry)>>,
    hold: Mutex<Option<crate::backend::Hold>>,
}

impl FakeRouter {
    /// Every open so far, oldest first, with the agent it was charged to.
    pub fn opened(&self) -> Vec<(AgentRef, ActivityEntry)> {
        lock(&self.opened).clone()
    }

    /// Records a dispatch that did not come through MCP: a click in another
    /// app, as the router would have reduced it.
    pub fn record(&self, url: &str, via: &str) -> anyhow::Result<()> {
        let trace = FakeResolver.resolve(url, None)?;
        let entry = entry_of(self.next_id(), Some(via.to_owned()), trace);
        lock(&self.opened).push((
            AgentRef {
                name: String::new(),
                connection: 0,
            },
            entry,
        ));
        Ok(())
    }

    /// Records a dispatch a rule kept from opening (Don't open), as the
    /// router would have kept it: the rule's trace, a browser that opens
    /// nothing, and the not-opened outcome.
    pub fn record_not_opened(&self, url: &str, via: &str) -> anyhow::Result<()> {
        let mut trace = FakeResolver.resolve(url, None)?;
        trace.steps.retain(|s| s.kind == StepKind::Rewrite);
        trace.steps.push(TraceStep {
            kind: StepKind::Handler,
            index: Some(1),
            summary: format!("rules.json rule 2 matched{}", crate::engine::NOT_OPENED),
            url: None,
            source: Some(StepSource::Rules),
        });
        trace.browser.name = String::new();
        trace.browser.app_type = AppType::None;
        trace.browser.private = false;
        let mut entry = entry_of(self.next_id(), Some(via.to_owned()), trace);
        entry.outcome = ActivityOutcome::NotOpened;
        entry.reason = Some(crate::backend::NotOpenedReason::Rule);
        entry.unrouted = false;
        lock(&self.opened).push((
            AgentRef {
                name: String::new(),
                connection: 0,
            },
            entry,
        ));
        Ok(())
    }

    /// Records a link the user held: the rules' trace, not opened, the
    /// hold's reason.
    pub fn record_held(&self, url: &str, via: &str) -> anyhow::Result<()> {
        let trace = FakeResolver.resolve(url, None)?;
        let mut entry = entry_of(self.next_id(), Some(via.to_owned()), trace);
        entry.outcome = ActivityOutcome::NotOpened;
        entry.reason = Some(crate::backend::NotOpenedReason::Hold);
        lock(&self.opened).push((
            AgentRef {
                name: String::new(),
                connection: 0,
            },
            entry,
        ));
        Ok(())
    }

    /// Holds links, or stops (`None`), as the tray would.
    pub fn set_hold(&self, hold: Option<crate::backend::Hold>) {
        *lock(&self.hold) = hold;
    }

    fn next_id(&self) -> u64 {
        lock(&self.opened).len() as u64 + 1
    }
}

impl Router for FakeRouter {
    fn open(&self, url: &str, opener: Option<&Opener>, agent: &AgentRef) -> anyhow::Result<Trace> {
        let trace = FakeResolver.resolve(url, opener)?;
        let entry = entry_of(self.next_id(), Some(agent.name.clone()), trace.clone());
        lock(&self.opened).push((agent.clone(), entry));
        Ok(trace)
    }
}

/// An entry reduced to its trace's privacy, as the router reduces a record
/// before the buffer sees it.
fn entry_of(id: u64, via: Option<String>, trace: Trace) -> ActivityEntry {
    let privacy = trace.browser.privacy;
    let unrouted = trace.steps.iter().any(|s| s.kind == StepKind::Default);
    let rewritten = trace.steps.iter().any(|s| s.kind == StepKind::Rewrite);
    let private = trace.browser.private;
    let trace = match privacy {
        Privacy::Full => Some(trace),
        Privacy::Host => Some(trace.hosts_only()),
        Privacy::None => None,
    };
    ActivityEntry {
        id,
        at_ms: now_ms(),
        via: via.filter(|_| privacy != Privacy::None),
        privacy,
        source: ActivitySource::Memory,
        outcome: ActivityOutcome::DryRun,
        unrouted,
        rewritten,
        private,
        reason: None,
        waiting: false,
        trace,
    }
}

/// Activity read back from a [`FakeRouter`]: memory only, no journal.
#[derive(Debug)]
pub struct FakeActivity(pub Arc<FakeRouter>);

impl ActivityLog for FakeActivity {
    fn recent(&self, limit: usize) -> anyhow::Result<Vec<ActivityEntry>> {
        Ok(self
            .0
            .opened()
            .into_iter()
            .rev()
            .take(limit)
            .map(|(_, entry)| entry)
            .collect())
    }

    fn journal(&self, _limit: usize) -> anyhow::Result<Vec<ActivityEntry>> {
        Ok(Vec::new())
    }

    fn retention(&self) -> String {
        "Kept in memory only: the last 500 links or 24 hours, cleared when PwrRoute quits.".into()
    }

    /// Every not-opened entry waits: nothing here opens one.
    fn waiting(&self) -> Vec<u64> {
        self.0
            .opened()
            .into_iter()
            .filter(|(_, entry)| entry.outcome == ActivityOutcome::NotOpened)
            .map(|(_, entry)| entry.id)
            .collect()
    }

    fn hold(&self) -> Option<crate::backend::Hold> {
        *lock(&self.0.hold)
    }
}

#[derive(Debug, Default)]
pub struct FakeBrowsers;

impl BrowserCatalog for FakeBrowsers {
    fn browsers(&self) -> anyhow::Result<BrowserList> {
        let browser = |name: &str, bundle_id: &str, profiles: &[&str]| DetectedBrowser {
            name: name.to_owned(),
            bundle_id: Some(bundle_id.to_owned()),
            path: None,
            profiles: profiles.iter().map(|p| (*p).to_owned()).collect(),
            hidden: false,
        };
        Ok(BrowserList {
            browsers: vec![
                browser("Safari", "com.apple.Safari", &["Personal", "Work"]),
                browser("Firefox", "org.mozilla.firefox", &["default-release"]),
            ],
            pwrroute_is_default: false,
            os_default: Some("com.apple.Safari".to_owned()),
        })
    }
}

/// `rules.json` in memory, in the file's own schema. Revisions count
/// writes. A write must parse as a [`RulesFile`] with no problems.
#[derive(Debug)]
pub struct FakeRules {
    state: Mutex<RulesSnapshot>,
}

impl FakeRules {
    /// The routing [`FakeResolver`] does, written as rules with an example
    /// and a test, so the saved tests pass against it.
    pub fn new(source: RulesSource) -> Self {
        Self::with(source, Self::file())
    }

    pub fn with(source: RulesSource, file: RulesFile) -> Self {
        Self {
            state: Mutex::new(RulesSnapshot {
                revision: "1".to_owned(),
                source,
                config: None,
                rules: serde_json::to_value(file).unwrap_or(Value::Null),
            }),
        }
    }

    /// The default rules: `x.com` rewritten to `xcancel.com`, `*.example.org`
    /// to Firefox, Safari otherwise; one example and one test.
    pub fn file() -> RulesFile {
        RulesFile {
            default_browser: "Safari".into(),
            rewrites: vec![pwrroute_engine::Rewrite {
                patterns: vec!["x.com/*".into()],
                actions: vec![pwrroute_engine::RewriteAction::SetHost(
                    "xcancel.com".into(),
                )],
                examples: vec![pwrroute_engine::RewriteExample {
                    url: "https://x.com/someone".into(),
                    becomes: "https://xcancel.com/someone".into(),
                }],
                ..pwrroute_engine::Rewrite::default()
            }],
            rules: vec![Rule {
                patterns: vec!["*.example.org/*".into()],
                browser: "Firefox".into(),
                examples: vec!["https://docs.example.org/page".into()],
                ..Rule::default()
            }],
            tests: vec![RouteTest {
                original_url: None,
                url: "https://example.net/".into(),
                opener: None,
                expect: Expectation {
                    browser: "Safari".into(),
                    ..Expectation::default()
                },
            }],
            ..RulesFile::default()
        }
    }

    /// The stored revision, for assertions.
    pub fn read_revision(&self) -> String {
        lock(&self.state).revision.clone()
    }

    /// The stored revision and rules, for the tester's engine.
    pub fn current(&self) -> (String, RulesFile) {
        let state = lock(&self.state);
        (
            state.revision.clone(),
            serde_json::from_value(state.rules.clone()).unwrap_or_default(),
        )
    }

    /// Switches the source, as Settings › Configuration would.
    pub fn select(&self, source: RulesSource) {
        lock(&self.state).source = source;
    }
}

impl Default for FakeRules {
    fn default() -> Self {
        Self::new(RulesSource::Visual)
    }
}

impl RulesStore for FakeRules {
    fn read(&self) -> anyhow::Result<RulesSnapshot> {
        Ok(lock(&self.state).clone())
    }

    fn write(
        &self,
        base_revision: &str,
        rules: Value,
        dry_run: bool,
    ) -> Result<RulesSnapshot, RulesError> {
        let mut state = lock(&self.state);
        if state.source == RulesSource::FinickyConfig {
            return Err(RulesError::FinickyConfigSelected);
        }
        if state.revision != base_revision {
            return Err(RulesError::Conflict {
                base: base_revision.to_owned(),
                current: state.revision.clone(),
            });
        }
        let rules = validate(rules)?;
        if dry_run {
            return Ok(RulesSnapshot {
                rules,
                ..state.clone()
            });
        }
        let next = state.revision.parse::<u64>().unwrap_or(0) + 1;
        state.revision = next.to_string();
        state.rules = rules;
        Ok(state.clone())
    }
}

/// A rules document as a [`RulesFile`] with no problems, re-serialized so
/// the store holds exactly what the file would.
pub(crate) fn validate(rules: Value) -> Result<Value, RulesError> {
    let mut file: RulesFile =
        serde_json::from_value(rules).map_err(|e| RulesError::Invalid(e.to_string()))?;
    // A rule that names a destination carries its synced browser too, for
    // readers that ignore names (the Go host), as the editor writes it.
    file.mirror_destinations();
    let problems = file.problems();
    if let Some(problem) = problems.first() {
        return Err(RulesError::Invalid(format!(
            "{:?}: {}",
            problem.place, problem.message
        )));
    }
    serde_json::to_value(file).map_err(|e| RulesError::Invalid(e.to_string()))
}

/// Every backend as a fake, sharing one router so opens show up in
/// activity, and a real engine over the fake rules so `pwrroute_rules_test`
/// and `pwrroute_rules_apply` test what the fake store holds.
pub fn backends() -> (Backends, Arc<FakeRouter>, Arc<FakeRules>) {
    let router = Arc::new(FakeRouter::default());
    let rules = Arc::new(FakeRules::default());
    let tester = {
        let rules = Arc::clone(&rules);
        EngineTester::over(move || rules.current(), Limits::default())
            .expect("an engine over the fake rules")
    };
    let backends = Backends {
        resolver: Arc::new(FakeResolver),
        router: router.clone(),
        browsers: Arc::new(FakeBrowsers),
        rules: rules.clone(),
        tester: Arc::new(tester),
        activity: Arc::new(FakeActivity(router.clone())),
    };
    (backends, router, rules)
}

/// How [`FakeApprover`] answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeAnswer {
    Approve(Role),
    /// Approve with this exact decision, valid or not.
    Decide(ApprovalDecision),
    Deny,
    /// Keep the reply unanswered; [`FakeApprover::take_pending`] hands it out.
    Hold,
}

/// Answers approval prompts without a window, and remembers what it was
/// asked and what it was told to cancel.
#[derive(Debug)]
pub struct FakeApprover {
    answer: Mutex<FakeAnswer>,
    asked: Mutex<Vec<ApprovalRequest>>,
    held: Mutex<Vec<(ApprovalRequest, ApprovalReply)>>,
    cancelled: Mutex<Vec<String>>,
}

impl FakeApprover {
    pub fn new(answer: FakeAnswer) -> Self {
        Self {
            answer: Mutex::new(answer),
            asked: Mutex::default(),
            held: Mutex::default(),
            cancelled: Mutex::default(),
        }
    }

    pub fn approving(role: Role) -> Self {
        Self::new(FakeAnswer::Approve(role))
    }

    pub fn set(&self, answer: FakeAnswer) {
        *lock(&self.answer) = answer;
    }

    pub fn asked(&self) -> Vec<ApprovalRequest> {
        lock(&self.asked).clone()
    }

    pub fn cancelled(&self) -> Vec<String> {
        lock(&self.cancelled).clone()
    }

    /// Prompts held by [`FakeAnswer::Hold`], oldest first.
    pub fn take_pending(&self) -> Vec<(ApprovalRequest, ApprovalReply)> {
        std::mem::take(&mut *lock(&self.held))
    }
}

impl Approver for FakeApprover {
    fn request(&self, request: ApprovalRequest, reply: ApprovalReply) {
        lock(&self.asked).push(request.clone());
        match lock(&self.answer).clone() {
            FakeAnswer::Approve(role) => reply.approve(role),
            FakeAnswer::Decide(decision) => reply.send(decision),
            FakeAnswer::Deny => reply.deny(),
            FakeAnswer::Hold => lock(&self.held).push((request, reply)),
        }
    }

    fn cancel(&self, id: &str) {
        lock(&self.cancelled).push(id.to_owned());
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
