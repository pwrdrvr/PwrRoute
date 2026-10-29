//! The MCP tools (the brief's 2d), each gated on a permission.
//!
//! Every tool is listed to every agent, as in PwrSnap and PwrGit; calling
//! one the agent's role does not allow answers with an `isError` result
//! naming the missing permission. The role is read at call time, so a
//! change or revocation in Settings applies to the next call. A client
//! name not yet approved raises the approval window, and its first call
//! waits for the answer. A denial lasts for the connection; a window that
//! closed unanswered does not, and the next call raises it again.

use std::{
    sync::{
        Arc, LazyLock, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use pwrroute_engine::{Opener, Rule, RulesFile, TestFailure, TestReport, suggest_rule};
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        InitializeRequestParams, InitializeResult, JsonObject, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
    },
    service::RequestContext,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use crate::{
    agents::{Agents, Standing, clean_name},
    approval::{ApprovalDecision, Approvals, Verdict, Waiting},
    audit::{AuditRecord, AuditSink, Outcome},
    backend::{
        ActivityEntry, ActivityOutcome, ActivitySource, AgentRef, Backends, ConsoleLine,
        RulesError, host_of,
    },
    roles::{Permission, Permissions, Role},
};

/// The longest URL a tool accepts.
const MAX_URL_BYTES: usize = 16 * 1024;
const DEFAULT_ACTIVITY: usize = 20;
const MAX_ACTIVITY: usize = 200;
/// Extra URLs one `pwrroute_rules_test` or `pwrroute_suggest_rule` call
/// takes.
const MAX_EXTRA_URLS: usize = 50;

const INSTRUCTIONS: &str = "PwrRoute is the OS default browser and routes each link to a browser and profile by rules. \
Use pwrroute_resolve to see where a link would go and why; it never opens anything. \
Every trace names who decided: a rules.json rule or rewrite (`source: rules`, `index` counting rules.json's lists), a Finicky config's handler or rewrite (`source: config`, counting the config's), or whose default browser took a link nothing claimed, PwrRoute's (rules.json's defaultBrowser) or the config's. \
With a Finicky config in charge its handlers run before rules.json's rules and its defaultBrowser takes the rest, unless rules.json says \"precedence\": \"pwrroute\": then rules.json's rules run first and its defaultBrowser takes everything else. \
pwrroute_open dispatches a link through the router as a click would, and is recorded in Activity against this agent. \
pwrroute_activity lists recent dispatches; its `retention` says where they are kept and for how long, each entry's `privacy` says how much of the link it keeps, and `filter: \"unrouted\"` finds the links no rule chose. \
Read the rules with pwrroute_rules_list; pwrroute_suggest_rule proposes narrow patterns for a link and what else they would catch; \
pwrroute_rules_test runs the saved examples and tests against a proposal without writing; \
pwrroute_rules_apply writes a JSON Merge Patch, naming the revision you read, and refuses a change that fails a saved example or test unless force is true. \
Links that pass through these tools stay in memory; only the examples and tests you write into rules.json are saved. \
The first call from a new client waits while the user approves it in PwrRoute's window; \
the window stays up while this connection is open, so after approval_pending simply call again. \
A denial (access_denied) lasts for this connection: reconnect the MCP server to be asked again. \
Which tools work depends on the role the user chose; a refused call names the permission it needs.";

/// The words `pwrroute_activity` and the audit use. Fixed, never link data.
const NOTE_REFUSED: &str = "refused: the change fails saved examples or tests";
const NOTE_FORCED: &str = "forced: written although saved examples or tests fail";

/// Where a link came from, as rules that match on the sending app see it.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OpenerInput {
    /// The sending app's name, e.g. "Slack".
    #[serde(default)]
    name: String,
    /// The sending app's bundle id, e.g. "com.tinyspeck.slackmacgap".
    #[serde(default)]
    bundle_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LinkInput {
    /// An absolute http or https URL.
    url: String,
    /// The app the link came from, for rules that match on it.
    #[serde(default)]
    opener: Option<OpenerInput>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoInput {}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RulesApplyInput {
    /// The `revision` from `pwrroute_rules_list`. The write is refused if the
    /// rules have changed since.
    base_revision: String,
    /// A JSON Merge Patch (RFC 7396) applied to the `rules` object from
    /// `pwrroute_rules_list`. Lists are replaced whole; `null` removes a key.
    /// `"precedence": "pwrroute"` runs these rules before a Finicky
    /// config's handlers and lets `defaultBrowser` take everything else;
    /// absent (or `"config"`), the config's handlers run before these rules
    /// and its `defaultBrowser` takes what neither matched.
    /// A `browser` (a rule's or `defaultBrowser`) is an app name, a bundle
    /// id as `pwrroute_browsers` lists them, or a `.app` path (`~/` allowed);
    /// `profile` is a profile name from the same listing.
    patch: Map<String, Value>,
    /// Validate and test only; write nothing.
    #[serde(default)]
    dry_run: bool,
    /// Write even though a saved example or test that passes today would
    /// fail: the user's old routing changes on purpose.
    #[serde(default)]
    force: bool,
}

/// Which dispatches `pwrroute_activity` lists.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum ActivityFilter {
    #[default]
    All,
    /// No rule or handler chose the browser: a default (PwrRoute's or the
    /// config's) or the fallback did. These are the links a new rule is
    /// for.
    Unrouted,
    Failed,
    /// A rewrite changed the URL.
    Rewritten,
    /// Opened in a private window.
    Private,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ActivityInput {
    /// How many dispatches, newest first, after the filter. Default 20, at
    /// most 200.
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    filter: ActivityFilter,
    /// `memory` (the default) reads the router's buffer, which is gone when
    /// PwrRoute quits. `journal` reads the on-disk journal, when the user
    /// turned it on; it needs the activity.journal permission.
    #[serde(default)]
    source: Option<ActivitySource>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RulesTestInput {
    /// A whole proposed rules.json, in the shape `pwrroute_rules_list`
    /// returns, tested in place of the saved one. Without it (and without
    /// `rule`) the saved rules are tested as they are.
    #[serde(default)]
    rules: Option<Value>,
    /// One extra handler to try: `{ "match": …, "browser": …, … }` as in
    /// rules.json. `browser` is an app name, a bundle id as
    /// `pwrroute_browsers` lists them, or a `.app` path (`~/` allowed);
    /// `profile` is a profile name from the same listing. It is added to the
    /// saved rules (or to `rules`) at `at`.
    #[serde(default)]
    rule: Option<Value>,
    /// Where `rule` goes in the handlers list, from 0. Default: the end.
    #[serde(default)]
    at: Option<usize>,
    /// Links `rule` must route: they are tested as its examples, and each
    /// comes back with the patterns that would catch it and whether a
    /// precise rule needs a JS config. At most 50.
    #[serde(default)]
    urls: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SuggestInput {
    /// The link a rule is wanted for.
    url: String,
}

/// One tool's definition and the permission it needs.
struct Spec {
    name: &'static str,
    permission: Permission,
    tool: Tool,
}

fn spec<T: JsonSchema + 'static>(
    name: &'static str,
    permission: Permission,
    title: &str,
    description: &'static str,
    mut annotations: ToolAnnotations,
) -> Spec {
    annotations.title = Some(title.to_owned());
    Spec {
        name,
        permission,
        tool: Tool::new(name, description, Arc::new(JsonObject::new()))
            .with_input_schema::<T>()
            .with_title(title)
            .with_annotations(annotations),
    }
}

/// Built once: generating the schemas is not free.
static SPECS: LazyLock<Vec<Spec>> = LazyLock::new(specs);

fn spec_named(name: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.name == name)
}

fn specs() -> Vec<Spec> {
    let read_only = || {
        ToolAnnotations::new()
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(false)
    };
    vec![
        spec::<LinkInput>(
            "pwrroute_resolve",
            Permission::Resolve,
            "Resolve a link",
            "Resolve a URL (and optionally the app it came from) to its trace and destination: each rewrite and handler that fired, then the browser and profile, and what the config printed with console when the engine ran it here. Never opens anything.",
            read_only(),
        ),
        spec::<LinkInput>(
            "pwrroute_open",
            Permission::Open,
            "Open a link",
            "Dispatch a URL through PwrRoute's router, as a click would: it is rewritten and opened in the browser the rules choose. Recorded in Activity against this agent. Returns the trace.",
            ToolAnnotations::new()
                .read_only(false)
                .destructive(false)
                .idempotent(false)
                .open_world(true),
        ),
        spec::<NoInput>(
            "pwrroute_browsers",
            Permission::Browsers,
            "List browsers",
            "Detected browsers with their profiles, and whether PwrRoute is the OS default browser.",
            read_only(),
        ),
        spec::<NoInput>(
            "pwrroute_rules_list",
            Permission::RulesRead,
            "Read the rules",
            "The rules as the visual model: rewrites, handlers and the default, with function entries from a Finicky config marked opaque. Includes the revision pwrroute_rules_apply needs.",
            read_only(),
        ),
        spec::<RulesTestInput>(
            "pwrroute_rules_test",
            Permission::RulesRead,
            "Test the rules",
            "Run the examples and tests saved in rules.json through the config and rules in charge, or against a proposal: a whole rules.json, or one extra handler with the links it must route. Reports what passed, each failure, and what the config printed with console. Nothing is written.",
            read_only(),
        ),
        spec::<SuggestInput>(
            "pwrroute_suggest_rule",
            Permission::RulesRead,
            "Suggest a rule",
            "Wildcard patterns that would route a link, narrowest first, whether a precise rule needs a JS config and why, and which recent links each pattern would also catch.",
            read_only(),
        ),
        spec::<RulesApplyInput>(
            "pwrroute_rules_apply",
            Permission::RulesWrite,
            "Change the rules",
            "Validate a JSON Merge Patch to rules.json, run the saved examples and tests against it, and write it if the rules are still at baseRevision. Refused, with the failures, when an example or test that passes today would fail, unless force is true. Use dryRun to validate and test without writing.",
            ToolAnnotations::new()
                .read_only(false)
                .destructive(true)
                .idempotent(false)
                .open_world(false),
        ),
        spec::<ActivityInput>(
            "pwrroute_activity",
            Permission::ActivityHosts,
            "Recent activity",
            "Recent dispatches with their traces, newest first, with where they are kept (retention) and how much of each link is kept (privacy). Filter for the unrouted, failed, rewritten or private ones. Agents without full-URL access see hosts only; the on-disk journal needs activity.journal.",
            read_only(),
        ),
    ]
}

/// What every connection's handler shares.
#[derive(Clone)]
pub(crate) struct Tools {
    pub(crate) backends: Backends,
    pub(crate) agents: Agents,
    pub(crate) audit: Arc<dyn AuditSink>,
    pub(crate) approvals: Arc<Approvals>,
    pub(crate) approval_wait: Duration,
}

impl Tools {
    pub(crate) fn for_connection(&self, connection: Connection) -> Handler {
        Handler {
            tools: self.clone(),
            connection: Arc::new(connection),
        }
    }
}

/// One link connection: which client it turned out to be, the approval it
/// is waiting on or got, and whether the person denied it.
pub(crate) struct Connection {
    id: u64,
    agents: Agents,
    name: OnceLock<String>,
    /// The prompt this connection waits on or got. Held for the life of the
    /// connection: while it is open, the window stays up.
    answer: Mutex<Option<Waiting>>,
    denied: AtomicBool,
}

impl Connection {
    pub(crate) fn new(id: u64, agents: Agents) -> Self {
        Self {
            id,
            agents,
            name: OnceLock::new(),
            answer: Mutex::new(None),
            denied: AtomicBool::new(false),
        }
    }

    /// Files the connection under its client's name, the first time it
    /// says one.
    fn identify(&self, client: &Implementation) -> String {
        let name = clean_name(&client.name);
        let mut first = false;
        let filed = self.name.get_or_init(|| {
            first = true;
            name.clone()
        });
        if first {
            self.agents.connected(filed);
        }
        filed.clone()
    }

    /// The answer this connection waits on: the one asked for when it
    /// connected, while it is still open or was a denial. An approval since
    /// forgotten in Settings, or a window that closed unanswered, is an
    /// empty slot: the person is asked again. `None` when no prompt could
    /// be raised.
    fn answer(
        &self,
        approvals: &Arc<Approvals>,
        client: &Implementation,
    ) -> Option<tokio::sync::watch::Receiver<Option<Verdict>>> {
        let mut slot = self
            .answer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let kept = slot.as_ref().filter(|waiting| {
            matches!(
                *waiting.answer.borrow(),
                None | Some(Verdict::Answered(ApprovalDecision::Deny))
            )
        });
        match kept {
            Some(waiting) => Some(waiting.answer.clone()),
            None => Some(slot.insert(approvals.ask(client)?).answer.clone()),
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if let Some(name) = self.name.get() {
            self.agents.disconnected(name);
        }
    }
}

/// The MCP handler for one connection.
#[derive(Clone)]
pub(crate) struct Handler {
    tools: Tools,
    connection: Arc<Connection>,
}

/// A tool's answer before it becomes a `CallToolResult`.
enum Answer {
    Ok(Value),
    Denied(String),
    Failed(String),
}

impl Handler {
    /// The agent's role now, asking the person first if its name is new.
    /// The wait ends with the call (`cancelled`): a call the client gave up
    /// on, or one whose connection closed, must not keep the window up.
    async fn authorize(
        &self,
        client: &Implementation,
        cancelled: &CancellationToken,
    ) -> Result<Role, String> {
        let name = self.connection.identify(client);
        match self.tools.agents.standing(&name) {
            Standing::Allowed(role) => return Ok(role),
            Standing::Revoked => {
                return Err(format!(
                    "revoked: PwrRoute access for {name} was revoked; the user can allow it again in Settings › Local agents"
                ));
            }
            Standing::Unknown => {}
        }
        if self.connection.denied.load(Ordering::Relaxed) {
            return Err(denied_message(&name));
        }
        let Some(mut answer) = self.connection.answer(&self.tools.approvals, client) else {
            return Err(format!(
                "approval_pending: PwrRoute cannot ask about {name} now (too many approval windows are open); call again later"
            ));
        };
        let decided = tokio::select! {
            seen = tokio::time::timeout(
                self.tools.approval_wait,
                answer.wait_for(Option::is_some),
            ) => seen.map(|seen| seen.ok().and_then(|d| *d)),
            () = cancelled.cancelled() => {
                return Err(format!(
                    "approval_pending: the call was cancelled before the user answered for {name}"
                ));
            }
        };
        match decided {
            Ok(Some(Verdict::Answered(ApprovalDecision::Approve { role }))) => Ok(role),
            // Nobody decided (the server is stopping): not a denial, so the
            // next call asks again.
            Ok(Some(Verdict::Closed)) => Err(format!(
                "approval_closed: PwrRoute's approval window for {name} closed unanswered; call again to reopen it"
            )),
            Ok(_) => {
                self.connection.denied.store(true, Ordering::Relaxed);
                Err(denied_message(&name))
            }
            Err(_) => Err(format!(
                "approval_pending: the user has not yet approved {name} in PwrRoute's window; call again once they have"
            )),
        }
    }

    /// Runs one tool. The note, when there is one, goes to the audit.
    async fn run(&self, name: &str, args: Value, agent: &AgentRef, role: Role) -> Reply {
        let Some(spec) = spec_named(name) else {
            return Reply::of(Answer::Failed(format!(
                "unknown_tool: PwrRoute has no tool named {name}"
            )));
        };
        let permissions = role.permissions();
        if !permissions.contains(&spec.permission) {
            return Reply::of(Answer::Denied(format!(
                "missing_permission: the {role} role does not allow {} (needs {})",
                spec.name, spec.permission
            )));
        }
        let b = self.tools.backends.clone();
        let result = match spec.name {
            "pwrroute_resolve" => match parse::<LinkInput>(args).and_then(link) {
                Ok((url, opener)) => {
                    blocking(move || b.resolver.resolve(&url, opener.as_ref()).map(to_json)).await
                }
                Err(e) => return Reply::of(e),
            },
            "pwrroute_open" => match parse::<LinkInput>(args).and_then(link) {
                Ok((url, opener)) => {
                    let agent = agent.clone();
                    blocking(move || b.router.open(&url, opener.as_ref(), &agent).map(to_json))
                        .await
                }
                Err(e) => return Reply::of(e),
            },
            "pwrroute_browsers" => match parse::<NoInput>(args) {
                Ok(_) => blocking(move || b.browsers.browsers().map(to_json)).await,
                Err(e) => return Reply::of(e),
            },
            "pwrroute_rules_list" => match parse::<NoInput>(args) {
                Ok(_) => blocking(move || b.rules.read().map(to_json)).await,
                Err(e) => return Reply::of(e),
            },
            "pwrroute_rules_test" => match parse::<RulesTestInput>(args) {
                Ok(input) => blocking(move || rules_test(&b, input)).await,
                Err(e) => return Reply::of(e),
            },
            "pwrroute_suggest_rule" => match parse::<SuggestInput>(args) {
                Ok(input) => {
                    let full = permissions.contains(&Permission::ActivityUrls);
                    blocking(move || suggest(&b, &input.url, full)).await
                }
                Err(e) => return Reply::of(e),
            },
            "pwrroute_rules_apply" => match parse::<RulesApplyInput>(args) {
                Ok(input) => return rules_apply(b, input).await,
                Err(e) => return Reply::of(e),
            },
            "pwrroute_activity" => match parse::<ActivityInput>(args) {
                Ok(input) => match activity(b, input, &permissions).await {
                    Ok(answer) => return Reply::of(answer),
                    Err(e) => Err(e),
                },
                Err(e) => return Reply::of(e),
            },
            _ => unreachable!("every spec has an arm"),
        };
        Reply::of(match result {
            Ok(value) => Answer::Ok(value),
            Err(e) => Answer::Failed(format!("failed: {e:#}")),
        })
    }
}

/// A tool's answer and, when there is more to say in the audit than the
/// outcome, a fixed word about it.
struct Reply {
    answer: Answer,
    note: Option<&'static str>,
}

impl Reply {
    fn of(answer: Answer) -> Self {
        Self { answer, note: None }
    }
}

/// The agent's activity: the buffer or the journal, filtered, cut to the
/// role's level.
async fn activity(
    b: Backends,
    input: ActivityInput,
    permissions: &Permissions,
) -> anyhow::Result<Answer> {
    let full = permissions.contains(&Permission::ActivityUrls);
    let source = input.source.unwrap_or(ActivitySource::Memory);
    if source == ActivitySource::Journal && !permissions.contains(&Permission::ActivityJournal) {
        return Ok(Answer::Denied(format!(
            "missing_permission: reading the activity journal needs {}",
            Permission::ActivityJournal
        )));
    }
    let limit = input
        .limit
        .unwrap_or(DEFAULT_ACTIVITY)
        .clamp(1, MAX_ACTIVITY);
    let filter = input.filter;
    blocking(move || {
        let entries = match source {
            ActivitySource::Memory => b.activity.recent(MAX_ACTIVITY)?,
            ActivitySource::Journal => b.activity.journal(MAX_ACTIVITY)?,
        };
        let scanned = entries.len();
        let entries: Vec<ActivityEntry> = entries
            .into_iter()
            .filter(|entry| keeps(filter, entry))
            .map(|entry| if full { entry } else { entry.hosts_only() })
            .take(limit)
            .collect();
        Ok(json!({
            "entries": entries,
            "fullUrls": full,
            "retention": b.activity.retention(),
            "source": source,
            "filter": filter,
            "scanned": scanned,
        }))
    })
    .await
    .map(Answer::Ok)
}

fn keeps(filter: ActivityFilter, entry: &ActivityEntry) -> bool {
    match filter {
        ActivityFilter::All => true,
        ActivityFilter::Unrouted => entry.unrouted,
        ActivityFilter::Failed => entry.outcome == ActivityOutcome::Failed,
        ActivityFilter::Rewritten => entry.rewritten,
        ActivityFilter::Private => entry.private,
    }
}

/// The saved examples and tests, or a proposal's. `rules` and `rule` are
/// JSON from the agent; they must parse as rules.json would.
fn rules_test(b: &Backends, input: RulesTestInput) -> anyhow::Result<Value> {
    if input.urls.len() > MAX_EXTRA_URLS {
        anyhow::bail!("invalid_input: at most {MAX_EXTRA_URLS} urls");
    }
    let proposal = match (input.rules, input.rule) {
        (None, None) if input.urls.is_empty() => None,
        (None, None) => anyhow::bail!("invalid_input: urls need a rule to be examples of"),
        (rules, rule) => {
            let mut file: RulesFile = match rules {
                Some(rules) => serde_json::from_value(rules)
                    .map_err(|e| anyhow::anyhow!("invalid_input: rules: {e}"))?,
                None => serde_json::from_value(b.rules.read()?.rules)?,
            };
            if let Some(rule) = rule {
                let mut rule: Rule = serde_json::from_value(rule)
                    .map_err(|e| anyhow::anyhow!("invalid_input: rule: {e}"))?;
                rule.examples.extend(input.urls.iter().cloned());
                let at = input.at.unwrap_or(file.rules.len()).min(file.rules.len());
                file.rules.insert(at, rule);
            }
            Some(file)
        }
    };
    let report = b.tester.test(proposal.as_ref())?;
    let urls: Vec<Value> = input
        .urls
        .iter()
        .map(|url| {
            let s = suggest_rule(url);
            json!({
                "url": url,
                "candidates": s.candidates,
                "needsJs": s.needs_js,
                "why": s.why,
            })
        })
        .collect();
    Ok(json!({
        "proposal": proposal.is_some(),
        "report": report_json(&report),
        "urls": urls,
    }))
}

fn report_json(report: &TestReport) -> Value {
    let console: Vec<ConsoleLine> = report.console.iter().map(Into::into).collect();
    json!({
        "total": report.total,
        "passed": report.passed,
        "failed": report.failures.len(),
        "failures": report.failures,
        "console": console,
    })
}

/// Patterns for a link, and what else each would catch among the links
/// kept in full in memory (a `host` or `none` record cannot be matched).
/// Without full-URL access the colliding links come back as hosts.
fn suggest(b: &Backends, url: &str, full: bool) -> anyhow::Result<Value> {
    if url.len() > MAX_URL_BYTES {
        anyhow::bail!("invalid_input: the URL is too long");
    }
    let suggestion = suggest_rule(url);
    let mut recent: Vec<String> = b
        .activity
        .recent(MAX_ACTIVITY)?
        .into_iter()
        .filter(|entry| entry.privacy == pwrroute_engine::Privacy::Full)
        .filter_map(|entry| entry.trace.map(|t| t.url))
        .filter(|u| u != url)
        .collect();
    recent.sort();
    recent.dedup();
    let mut collisions = Vec::new();
    for pattern in &suggestion.candidates {
        let hits = b.tester.collisions(pattern, &recent)?;
        let count = hits.len();
        let urls: Vec<String> = if full {
            hits
        } else {
            let mut hosts: Vec<String> = hits.iter().map(|u| host_of(u)).collect();
            hosts.sort();
            hosts.dedup();
            hosts
        };
        collisions.push(json!({ "pattern": pattern, "count": count, "urls": urls }));
    }
    Ok(json!({
        "url": url,
        "candidates": suggestion.candidates,
        "needsJs": suggestion.needs_js,
        "why": suggestion.why,
        "collisions": collisions,
        "fullUrls": full,
    }))
}

/// Patches, validates, tests and writes. A patch that makes a saved example
/// or test fail which passes today is refused, with the failures, unless
/// `force` is set; nothing is written then, and the audit says why. A dry
/// run reports the regressions instead of refusing: it writes nothing
/// either way.
async fn rules_apply(b: Backends, input: RulesApplyInput) -> Reply {
    let force = input.force;
    let dry_run = input.dry_run;
    type Noted<T> = Result<T, (RulesError, Option<&'static str>)>;
    let result = tokio::task::spawn_blocking(move || -> Noted<(Value, Option<&'static str>)> {
        // The file's loader reads any other precedence as the config's
        // order; from an agent, a misspelling is refused rather than
        // written as a change that changes nothing.
        match input.patch.get("precedence") {
            None | Some(Value::Null) => {}
            Some(Value::String(word)) if word == "config" || word == "pwrroute" => {}
            Some(_) => {
                return Err((
                    RulesError::Invalid(
                        "precedence must be \"pwrroute\" or \"config\"".to_owned(),
                    ),
                    None,
                ));
            }
        }
        let current = b.rules.read().map_err(|e| (RulesError::Other(e), None))?;
        let mut rules = current.rules;
        merge_patch(&mut rules, &Value::Object(input.patch));
        let proposal: RulesFile = serde_json::from_value(rules.clone())
            .map_err(|e| (RulesError::Invalid(e.to_string()), None))?;
        if let Some(problem) = proposal.problems().first() {
            return Err((
                RulesError::Invalid(format!("{:?}: {}", problem.place, problem.message)),
                None,
            ));
        }
        let before = b.tester.test(None).map_err(|e| (RulesError::Other(e), None))?;
        let after = b
            .tester
            .test(Some(&proposal))
            .map_err(|e| (RulesError::Other(e), None))?;
        let regressions: Vec<&TestFailure> = after
            .failures
            .iter()
            .filter(|f| !before.failures.iter().any(|b| same_case(b, f)))
            .collect();
        if !regressions.is_empty() && !force && !dry_run {
            let n = regressions.len();
            let body = json!({
                "regressions": regressions,
                "report": report_json(&after),
                "revision": current.revision,
            });
            return Err((
                RulesError::Invalid(format!(
                    "{n} saved {} would change routing. Nothing was written; pass force: true to change it on purpose.\n{body}",
                    if n == 1 { "example or test" } else { "examples or tests" }
                )),
                Some(NOTE_REFUSED),
            ));
        }
        let note = (!regressions.is_empty() && !dry_run).then_some(NOTE_FORCED);
        let snapshot = b
            .rules
            .write(&input.base_revision, rules, dry_run)
            .map_err(|e| (e, note))?;
        let value = json!({
            "revision": snapshot.revision,
            "dryRun": dry_run,
            "written": !dry_run,
            "rules": snapshot.rules,
            "report": report_json(&after),
            "regressions": regressions,
            "forced": note.is_some(),
        });
        Ok((value, note))
    })
    .await;
    match result {
        Ok(Ok((value, note))) => Reply {
            answer: Answer::Ok(value),
            note,
        },
        Ok(Err((e, note))) => Reply {
            answer: Answer::Failed(match &e {
                RulesError::FinickyConfigSelected => format!("finicky_config_selected: {e}"),
                RulesError::Conflict { .. } => format!("conflict: {e}"),
                RulesError::Invalid(m) if note == Some(NOTE_REFUSED) => format!("regressions: {m}"),
                RulesError::Invalid(_) => format!("invalid_rules: {e}"),
                RulesError::Other(e) => format!("failed: {e:#}"),
            }),
            note,
        },
        Err(e) => Reply::of(Answer::Failed(format!("failed: {e}"))),
    }
}

/// Whether two failures are the same example or test: rule numbers move
/// when a handler is inserted, so the URL, opener and expectation decide.
fn same_case(a: &TestFailure, b: &TestFailure) -> bool {
    a.input == b.input && a.opener == b.opener && a.expected == b.expected
}

/// RFC 7396 JSON Merge Patch.
fn merge_patch(target: &mut Value, patch: &Value) {
    let Value::Object(patch) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(Map::new());
    }
    let target = target.as_object_mut().expect("made an object above");
    for (key, value) in patch {
        if value.is_null() {
            target.remove(key);
        } else {
            merge_patch(target.entry(key.clone()).or_insert(Value::Null), value);
        }
    }
}

fn parse<T: DeserializeOwned>(args: Value) -> Result<T, Answer> {
    serde_json::from_value(args).map_err(|e| Answer::Failed(format!("invalid_input: {e}")))
}

/// Checks the URL and converts the opener. Only http and https: the router
/// is the default *browser*, and an agent should not reach other URL
/// handlers through it.
fn link(input: LinkInput) -> Result<(String, Option<Opener>), Answer> {
    let invalid = |m: &str| Answer::Failed(format!("invalid_input: {m}"));
    if input.url.len() > MAX_URL_BYTES {
        return Err(invalid("the URL is too long"));
    }
    let parsed = url::Url::parse(&input.url).map_err(|_| invalid("url must be an absolute URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(invalid("url must be http or https"));
    }
    let opener = input.opener.map(|o| Opener {
        name: o.name,
        bundle_id: o.bundle_id,
        ..Opener::default()
    });
    Ok((input.url, opener))
}

fn to_json(value: impl Serialize) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

async fn blocking(
    f: impl FnOnce() -> anyhow::Result<Value> + Send + 'static,
) -> anyhow::Result<Value> {
    tokio::task::spawn_blocking(f).await?
}

fn denied_message(name: &str) -> String {
    format!(
        "access_denied: the user did not approve {name} in PwrRoute; reconnect the MCP server to be asked again"
    )
}

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("PwrRoute", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }

    /// Raises the approval window as the client connects, so it is up
    /// before the first tool call waits on it.
    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        context.peer.set_peer_info(request.clone());
        let name = self.connection.identify(&request.client_info);
        if self.tools.agents.standing(&name) == Standing::Unknown {
            self.connection
                .answer(&self.tools.approvals, &request.client_info);
        }
        self.negotiate_initialize(&request)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: SPECS.iter().map(|s| s.tool.clone()).collect(),
            ..ListToolsResult::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let started = SystemTime::now();
        let clock = Instant::now();
        let client = context
            .client_info()
            .unwrap_or_else(|| Implementation::new("", ""));
        let agent = AgentRef {
            name: self.connection.identify(&client),
            connection: self.connection.id,
        };
        let args = Value::Object(request.arguments.unwrap_or_default());
        let Reply { answer, note } = match self.authorize(&client, &context.ct).await {
            Ok(role) => self.run(&request.name, args, &agent, role).await,
            Err(message) => Reply::of(Answer::Denied(message)),
        };
        let outcome = match &answer {
            Answer::Ok(_) => Outcome::Ok,
            Answer::Denied(_) => Outcome::Denied,
            Answer::Failed(_) => Outcome::Failed,
        };
        self.tools.agents.record_use(&agent.name);
        self.tools.audit.record(AuditRecord {
            // Only names PwrRoute defines: an unknown one is agent text.
            tool: spec_named(&request.name)
                .map_or("unknown_tool", |s| s.name)
                .to_owned(),
            agent: agent.name.clone(),
            connection: agent.connection,
            started,
            duration: clock.elapsed(),
            outcome,
            note,
        });
        Ok(match answer {
            Answer::Ok(value) => success(value),
            Answer::Denied(message) | Answer::Failed(message) => error_result(&message),
        }
        .into())
    }
}

/// A summary line, then the JSON, with the same JSON as structured content,
/// as the siblings answer.
fn success(value: Value) -> CallToolResult {
    let value = if value.is_object() {
        value
    } else {
        json!({ "value": value })
    };
    let mut result = CallToolResult::structured(value.clone());
    result.content = vec![ContentBlock::text(value.to_string())];
    result
}

fn error_result(message: &str) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message)])
}

/// For tests: the permission each tool needs.
#[cfg(test)]
pub(crate) fn tool_permissions() -> Vec<(&'static str, Permission)> {
    SPECS.iter().map(|s| (s.name, s.permission)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_patch_follows_rfc_7396() {
        let mut doc = json!({ "a": "b", "c": { "d": "e", "f": "g" } });
        merge_patch(&mut doc, &json!({ "a": "z", "c": { "f": null } }));
        assert_eq!(doc, json!({ "a": "z", "c": { "d": "e" } }));
    }

    #[test]
    fn every_tool_needs_the_brief_permission() {
        assert_eq!(
            tool_permissions(),
            [
                ("pwrroute_resolve", Permission::Resolve),
                ("pwrroute_open", Permission::Open),
                ("pwrroute_browsers", Permission::Browsers),
                ("pwrroute_rules_list", Permission::RulesRead),
                ("pwrroute_rules_test", Permission::RulesRead),
                ("pwrroute_suggest_rule", Permission::RulesRead),
                ("pwrroute_rules_apply", Permission::RulesWrite),
                ("pwrroute_activity", Permission::ActivityHosts),
            ]
        );
    }

    #[test]
    fn only_web_urls_are_accepted() {
        let input = |url: &str| LinkInput {
            url: url.to_owned(),
            opener: None,
        };
        assert!(link(input("https://example.org/a")).is_ok());
        assert!(link(input("file:///etc/passwd")).is_err());
        assert!(link(input("not a url")).is_err());
    }
}
