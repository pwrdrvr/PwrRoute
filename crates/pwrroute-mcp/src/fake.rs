//! In-memory stand-ins for the backends still being built, and an approver
//! that answers without a window. Tests use them; so can the app until the
//! real router, launcher and rules store land.

use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use pwrroute_engine::{AppType, Browser, Opener};
use serde_json::{Value, json};

use crate::{
    approval::{ApprovalDecision, ApprovalReply, ApprovalRequest, Approver},
    backend::{
        ActivityEntry, ActivityLog, AgentRef, Backends, BrowserCatalog, BrowserList,
        DetectedBrowser, Resolver, Router, RulesError, RulesSnapshot, RulesSource, RulesStore,
        StepKind, Trace, TraceStep,
    },
    roles::Role,
};

/// Sends every `*.example.org` link to Firefox and the rest to Safari, and
/// rewrites `x.com` to `xcancel.com` first. Contrived, like the app's
/// fixture data.
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
            });
        }
        let host = parsed.host_str().unwrap_or_default();
        let name = if host == "example.org" || host.ends_with(".example.org") {
            steps.push(TraceStep {
                kind: StepKind::Handler,
                index: Some(0),
                summary: "*.example.org/* → Firefox".to_owned(),
                url: None,
            });
            "Firefox"
        } else {
            steps.push(TraceStep {
                kind: StepKind::Default,
                index: None,
                summary: "default → Safari".to_owned(),
                url: None,
            });
            "Safari"
        };
        Ok(Trace {
            url: url.to_owned(),
            steps,
            browser: Browser {
                name: name.to_owned(),
                app_type: AppType::AppName,
                url: parsed.to_string(),
                profile: String::new(),
                args: Vec::new(),
                open_in_background: None,
            },
            error: None,
        })
    }
}

/// A router that resolves with [`FakeResolver`], opens nothing, and keeps
/// what it "opened" in memory for [`FakeActivity`] and for assertions.
#[derive(Debug, Default)]
pub struct FakeRouter {
    opened: Mutex<Vec<(AgentRef, ActivityEntry)>>,
}

impl FakeRouter {
    /// Every open so far, oldest first, with the agent it was charged to.
    pub fn opened(&self) -> Vec<(AgentRef, ActivityEntry)> {
        lock(&self.opened).clone()
    }
}

impl Router for FakeRouter {
    fn open(&self, url: &str, opener: Option<&Opener>, agent: &AgentRef) -> anyhow::Result<Trace> {
        let trace = FakeResolver.resolve(url, opener)?;
        let entry = ActivityEntry {
            at_ms: now_ms(),
            via: Some(agent.name.clone()),
            trace: trace.clone(),
        };
        lock(&self.opened).push((agent.clone(), entry));
        Ok(trace)
    }
}

/// Activity read back from a [`FakeRouter`].
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

/// `rules.json` in memory. Revisions count writes. Valid rules are an object
/// with `rewrite` and `handlers` arrays; anything else is refused, which is
/// enough to exercise the tool's validation path.
#[derive(Debug)]
pub struct FakeRules {
    state: Mutex<RulesSnapshot>,
}

impl FakeRules {
    pub fn new(source: RulesSource) -> Self {
        Self {
            state: Mutex::new(RulesSnapshot {
                revision: "1".to_owned(),
                source,
                rules: json!({
                    "rewrite": [{ "match": "x.com/*", "host": "xcancel.com" }],
                    "handlers": [
                        { "match": "*.example.org/*", "browser": "Firefox" },
                        { "fn": "(url, { opener }) => …", "opaque": true }
                    ],
                    "defaultBrowser": "Safari"
                }),
            }),
        }
    }

    /// The stored revision, for assertions.
    pub fn read_revision(&self) -> String {
        lock(&self.state).revision.clone()
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
        for list in ["rewrite", "handlers"] {
            if !rules.get(list).is_some_and(Value::is_array) {
                return Err(RulesError::Invalid(format!("`{list}` must be a list")));
            }
        }
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

/// Every backend as a fake, sharing one router so opens show up in
/// activity.
pub fn backends() -> (Backends, Arc<FakeRouter>, Arc<FakeRules>) {
    let router = Arc::new(FakeRouter::default());
    let rules = Arc::new(FakeRules::default());
    let backends = Backends {
        resolver: Arc::new(FakeResolver),
        router: router.clone(),
        browsers: Arc::new(FakeBrowsers),
        rules: rules.clone(),
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
