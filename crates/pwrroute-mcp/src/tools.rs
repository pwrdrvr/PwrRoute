//! The MCP tools (the brief's 2d), each gated on a permission.
//!
//! Every tool is listed to every agent, as in PwrSnap and PwrGit; calling
//! one the agent's role does not allow answers with an `isError` result
//! naming the missing permission. The role is read at call time, so a
//! change or revocation in Settings applies to the next call. A client
//! name not yet approved raises the approval window, and its first call
//! waits for the answer.

use std::{
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

use pwrroute_engine::Opener;
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

use crate::{
    agents::{Agents, Standing, clean_name},
    approval::{Answer as Decision, ApprovalDecision, Approvals},
    audit::{AuditRecord, AuditSink, Outcome},
    backend::{AgentRef, Backends, RulesError},
    roles::{Permission, Role},
};

/// The longest URL a tool accepts.
const MAX_URL_BYTES: usize = 16 * 1024;
const DEFAULT_ACTIVITY: usize = 20;
const MAX_ACTIVITY: usize = 200;

const INSTRUCTIONS: &str = "PwrRoute is the OS default browser and routes each link to a browser and profile by rules. \
Use pwrroute_resolve to see where a link would go and why; it never opens anything. \
pwrroute_open dispatches a link through the router as a click would, and is recorded in Activity against this agent. \
Read the rules with pwrroute_rules_list; change them with pwrroute_rules_apply, naming the revision you read. \
The first call from a new client waits while the user approves it in PwrRoute's window. \
Which tools work depends on the role the user chose; a refused call names the permission it needs.";

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
    patch: Map<String, Value>,
    /// Validate only; write nothing.
    #[serde(default)]
    dry_run: bool,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ActivityInput {
    /// How many recent dispatches, newest first. Default 20, at most 200.
    #[serde(default)]
    limit: Option<usize>,
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
            "Resolve a URL (and optionally the app it came from) to its trace and destination: each rewrite and handler that fired, then the browser and profile. Never opens anything.",
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
        spec::<RulesApplyInput>(
            "pwrroute_rules_apply",
            Permission::RulesWrite,
            "Change the rules",
            "Validate and write a JSON Merge Patch to rules.json, if the rules are still at baseRevision. Refused while a Finicky config is selected. Use dryRun to validate without writing.",
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
            "Recent dispatches with their traces, newest first. Agents without full-URL access see hosts only.",
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
    answer: Mutex<Option<Decision>>,
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
    /// connected, unless that was an approval since forgotten in Settings,
    /// in which case the person is asked again.
    fn answer(&self, approvals: &Arc<Approvals>, client: &Implementation) -> Decision {
        let mut slot = self
            .answer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match slot.as_ref() {
            Some(answer) if !matches!(*answer.borrow(), Some(ApprovalDecision::Approve { .. })) => {
                answer.clone()
            }
            _ => slot.insert(approvals.ask(client)).clone(),
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
    async fn authorize(&self, client: &Implementation) -> Result<Role, String> {
        let name = self.connection.identify(client);
        if self.connection.denied.load(Ordering::Relaxed) {
            return Err(denied_message(&name));
        }
        match self.tools.agents.standing(&name) {
            Standing::Allowed(role) => return Ok(role),
            Standing::Revoked => {
                return Err(format!(
                    "revoked: PwrRoute access for {name} was revoked; the user can allow it again in Settings › Local agents"
                ));
            }
            Standing::Unknown => {}
        }
        let mut answer = self.connection.answer(&self.tools.approvals, client);
        let decided =
            tokio::time::timeout(self.tools.approval_wait, answer.wait_for(Option::is_some))
                .await
                .map(|seen| seen.ok().and_then(|d| *d));
        match decided {
            Ok(Some(ApprovalDecision::Approve { role })) => Ok(role),
            Ok(_) => {
                self.connection.denied.store(true, Ordering::Relaxed);
                Err(denied_message(&name))
            }
            Err(_) => Err(format!(
                "approval_pending: the user has not yet approved {name} in PwrRoute's window; call again once they have"
            )),
        }
    }

    async fn run(&self, name: &str, args: Value, agent: &AgentRef, role: Role) -> Answer {
        let Some(spec) = specs().into_iter().find(|s| s.name == name) else {
            return Answer::Failed(format!("unknown_tool: PwrRoute has no tool named {name}"));
        };
        let permissions = role.permissions();
        if !permissions.contains(&spec.permission) {
            return Answer::Denied(format!(
                "missing_permission: the {role} role does not allow {} (needs {})",
                spec.name, spec.permission
            ));
        }
        let b = self.tools.backends.clone();
        let result = match spec.name {
            "pwrroute_resolve" => match parse::<LinkInput>(args).and_then(link) {
                Ok((url, opener)) => {
                    blocking(move || b.resolver.resolve(&url, opener.as_ref()).map(to_json)).await
                }
                Err(e) => return e,
            },
            "pwrroute_open" => match parse::<LinkInput>(args).and_then(link) {
                Ok((url, opener)) => {
                    let agent = agent.clone();
                    blocking(move || b.router.open(&url, opener.as_ref(), &agent).map(to_json))
                        .await
                }
                Err(e) => return e,
            },
            "pwrroute_browsers" => match parse::<NoInput>(args) {
                Ok(_) => blocking(move || b.browsers.browsers().map(to_json)).await,
                Err(e) => return e,
            },
            "pwrroute_rules_list" => match parse::<NoInput>(args) {
                Ok(_) => blocking(move || b.rules.read().map(to_json)).await,
                Err(e) => return e,
            },
            "pwrroute_rules_apply" => match parse::<RulesApplyInput>(args) {
                Ok(input) => return rules_apply(b, input).await,
                Err(e) => return e,
            },
            "pwrroute_activity" => match parse::<ActivityInput>(args) {
                Ok(input) => {
                    let full = permissions.contains(&Permission::ActivityUrls);
                    let limit = input
                        .limit
                        .unwrap_or(DEFAULT_ACTIVITY)
                        .clamp(1, MAX_ACTIVITY);
                    blocking(move || {
                        let mut entries = b.activity.recent(limit)?;
                        if !full {
                            for entry in &mut entries {
                                entry.trace = entry.trace.hosts_only();
                            }
                        }
                        Ok(json!({ "entries": entries, "fullUrls": full }))
                    })
                    .await
                }
                Err(e) => return e,
            },
            _ => unreachable!("every spec has an arm"),
        };
        match result {
            Ok(value) => Answer::Ok(value),
            Err(e) => Answer::Failed(format!("failed: {e:#}")),
        }
    }
}

async fn rules_apply(b: Backends, input: RulesApplyInput) -> Answer {
    let result = tokio::task::spawn_blocking(move || {
        let current = b.rules.read().map_err(RulesError::Other)?;
        let mut rules = current.rules;
        merge_patch(&mut rules, &Value::Object(input.patch));
        b.rules.write(&input.base_revision, rules, input.dry_run)
    })
    .await;
    match result {
        Ok(Ok(snapshot)) => Answer::Ok(json!({
            "revision": snapshot.revision,
            "dryRun": input.dry_run,
            "written": !input.dry_run,
            "rules": snapshot.rules,
        })),
        Ok(Err(e @ RulesError::FinickyConfigSelected)) => {
            Answer::Failed(format!("finicky_config_selected: {e}"))
        }
        Ok(Err(e @ RulesError::Conflict { .. })) => Answer::Failed(format!("conflict: {e}")),
        Ok(Err(e @ RulesError::Invalid(_))) => Answer::Failed(format!("invalid_rules: {e}")),
        Ok(Err(RulesError::Other(e))) => Answer::Failed(format!("failed: {e:#}")),
        Err(e) => Answer::Failed(format!("failed: {e}")),
    }
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
    format!("access_denied: the user did not approve {name} in PwrRoute")
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
            tools: specs().into_iter().map(|s| s.tool).collect(),
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
        let answer = match self.authorize(&client).await {
            Ok(role) => self.run(&request.name, args, &agent, role).await,
            Err(message) => Answer::Denied(message),
        };
        let outcome = match &answer {
            Answer::Ok(_) => Outcome::Ok,
            Answer::Denied(_) => Outcome::Denied,
            Answer::Failed(_) => Outcome::Failed,
        };
        self.tools.agents.record_use(&agent.name);
        self.tools.audit.record(AuditRecord {
            tool: request.name.to_string(),
            agent: agent.name.clone(),
            connection: agent.connection,
            started,
            duration: clock.elapsed(),
            outcome,
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
    specs()
        .into_iter()
        .map(|s| (s.name, s.permission))
        .collect()
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
