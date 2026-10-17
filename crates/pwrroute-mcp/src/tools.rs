//! The MCP tools (the brief's 2d), each gated on a permission.
//!
//! Every tool is listed to every session, as in PwrSnap and PwrGit; calling
//! one the session's role does not allow answers with an `isError` result
//! naming the missing permission. The check runs against the session as it
//! is at call time, so a role change or revocation applies to the next
//! call.

use std::{
    sync::Arc,
    time::{Instant, SystemTime},
};

use pwrroute_engine::Opener;
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        JsonObject, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
        Tool, ToolAnnotations,
    },
    service::RequestContext,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};

use crate::{
    access::{AccessControl, Denied, Principal},
    audit::{AuditRecord, AuditSink, Outcome},
    backend::{Backends, RulesError},
    roles::Permission,
};

/// The longest URL a tool accepts.
const MAX_URL_BYTES: usize = 16 * 1024;
const DEFAULT_ACTIVITY: usize = 20;
const MAX_ACTIVITY: usize = 200;

const INSTRUCTIONS: &str = "PwrRoute is the OS default browser and routes each link to a browser and profile by rules. \
Use pwrroute_resolve to see where a link would go and why; it never opens anything. \
pwrroute_open dispatches a link through the router as a click would, and is recorded in Activity against this session. \
Read the rules with pwrroute_rules_list; change them with pwrroute_rules_apply, naming the revision you read. \
Which tools work depends on the role the user approved for this session; a refused call names the permission it needs. \
This HTTP transport is stateless.";

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
            "Dispatch a URL through PwrRoute's router, as a click would: it is rewritten and opened in the browser the rules choose. Recorded in Activity against this session. Returns the trace.",
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
            "Recent dispatches with their traces, newest first. Sessions without full-URL access see hosts only.",
            read_only(),
        ),
    ]
}

/// The MCP handler. One is built per request (the transport is
/// stateless); it is a few `Arc`s.
#[derive(Clone)]
pub(crate) struct Tools {
    pub(crate) backends: Backends,
    pub(crate) access: AccessControl,
    pub(crate) audit: Arc<dyn AuditSink>,
}

/// A tool's answer before it becomes a `CallToolResult`.
enum Answer {
    Ok(Value),
    Denied(String),
    Failed(String),
}

impl Tools {
    async fn run(&self, name: &str, args: Value, principal: &Principal) -> Answer {
        let Some(spec) = specs().into_iter().find(|s| s.name == name) else {
            return Answer::Failed(format!("unknown_tool: PwrRoute has no tool named {name}"));
        };
        if !principal.permissions.contains(&spec.permission) {
            return Answer::Denied(
                Denied::MissingPermission {
                    tool: spec.name,
                    permission: spec.permission,
                }
                .to_string(),
            );
        }
        let b = self.backends.clone();
        let result = match spec.name {
            "pwrroute_resolve" => match parse::<LinkInput>(args).and_then(link) {
                Ok((url, opener)) => {
                    blocking(move || b.resolver.resolve(&url, opener.as_ref()).map(to_json)).await
                }
                Err(e) => return e,
            },
            "pwrroute_open" => match parse::<LinkInput>(args).and_then(link) {
                Ok((url, opener)) => {
                    let session = principal.session.clone();
                    blocking(move || b.router.open(&url, opener.as_ref(), &session).map(to_json))
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
                    let full = principal.permissions.contains(&Permission::ActivityUrls);
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

/// The caller of this request, as the HTTP layer authenticated it, checked
/// again against the store now.
fn principal(access: &AccessControl, context: &RequestContext<RoleServer>) -> Option<Principal> {
    let parts = context.extensions.get::<axum::http::request::Parts>()?;
    let authenticated = parts.extensions.get::<Principal>()?;
    access.principal(&authenticated.session.id).ok()
}

impl ServerHandler for Tools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("PwrRoute", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
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
        let Some(principal) = principal(&self.access, &context) else {
            return Ok(error_result(&Denied::InvalidSession.to_string()).into());
        };
        let args = Value::Object(request.arguments.unwrap_or_default());
        let answer = self.run(&request.name, args, &principal).await;
        let outcome = match &answer {
            Answer::Ok(_) => Outcome::Ok,
            Answer::Denied(_) => Outcome::Denied,
            Answer::Failed(_) => Outcome::Failed,
        };
        self.access.record_use(&principal.session.id);
        self.audit.record(AuditRecord {
            tool: request.name.to_string(),
            session_id: principal.session.id.clone(),
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
