//! The OAuth 2.1 authorization server: metadata, dynamic client
//! registration, `/authorize` with the in-app approval, `/token` and
//! `/revoke`. Behaviour follows PwrSnap and PwrGit, which run the MCP
//! TypeScript SDK's handlers; where the two differ, or PwrRoute does, the
//! crate's `AGENTS.md` says which and why.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    access::{AccessControl, Client, RegisterError, SessionError, constant_time_eq, random_b64},
    approval::{ApprovalDecision, ApprovalReply, ApprovalRequest, Approver},
    roles::{Permission, Permissions, Role},
};

/// How long an approval may wait, and how long its code then lives.
pub(crate) const APPROVAL_TTL: Duration = Duration::from_secs(5 * 60);
pub(crate) const CODE_TTL: Duration = Duration::from_secs(5 * 60);
/// Approvals waiting in a browser, and codes waiting to be exchanged.
const MAX_PENDING: usize = 64;
/// Approval windows open in the app at once. More are denied at once.
const MAX_PROMPTS: usize = 8;

/// Query parameters that would decide consent if `/authorize` honoured
/// them. It never does; their presence is refused outright.
const DECISION_PARAMS: &[&str] = &[
    "decision",
    "pwrroute_decision",
    "capability",
    "consent_transaction",
];

struct Approval {
    expires: Instant,
    /// Set once the person has answered: where the browser goes next.
    outcome: Option<String>,
}

struct Code {
    client_id: String,
    redirect_uri: String,
    challenge: String,
    scopes: Permissions,
    session_name: String,
    role: Role,
    expires: Instant,
}

#[derive(Default)]
struct Pending {
    approvals: HashMap<String, Approval>,
    codes: HashMap<String, Code>,
    prompts: usize,
}

/// The authorization server's state, shared by its handlers.
pub(crate) struct OAuth {
    pub(crate) access: AccessControl,
    approver: Arc<dyn Approver>,
    /// `http://127.0.0.1:<port>`.
    origin: String,
    pending: Mutex<Pending>,
    pub(crate) limits: Limits,
    /// Cancelled when the listener stops, ending every wait for an answer.
    closed: tokio_util::sync::CancellationToken,
}

/// The SDK's per-endpoint rate limits. Every caller is on loopback, so a
/// per-address limit is a global one; these are global.
pub(crate) struct Limits {
    pub(crate) register: RateLimit,
    pub(crate) authorize: RateLimit,
    pub(crate) token: RateLimit,
    pub(crate) revoke: RateLimit,
}

impl Default for Limits {
    fn default() -> Self {
        let quarter_hour = Duration::from_secs(15 * 60);
        Self {
            register: RateLimit::new(20, Duration::from_secs(60 * 60)),
            authorize: RateLimit::new(100, quarter_hour),
            token: RateLimit::new(50, quarter_hour),
            revoke: RateLimit::new(50, quarter_hour),
        }
    }
}

/// A sliding-window counter.
pub(crate) struct RateLimit {
    limit: usize,
    window: Duration,
    hits: Mutex<VecDeque<Instant>>,
}

impl RateLimit {
    pub(crate) fn new(limit: usize, window: Duration) -> Self {
        Self {
            limit,
            window,
            hits: Mutex::new(VecDeque::new()),
        }
    }

    fn check(&self) -> Result<(), Box<Response>> {
        let now = Instant::now();
        let mut hits = self
            .hits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while hits.front().is_some_and(|t| now - *t >= self.window) {
            hits.pop_front();
        }
        if hits.len() >= self.limit {
            return Err(Box::new(oauth_error(
                StatusCode::TOO_MANY_REQUESTS,
                "too_many_requests",
                "You have exceeded the rate limit for this endpoint",
            )));
        }
        hits.push_back(now);
        Ok(())
    }
}

impl OAuth {
    pub(crate) fn new(access: AccessControl, approver: Arc<dyn Approver>, origin: String) -> Self {
        Self {
            access,
            approver,
            origin,
            pending: Mutex::new(Pending::default()),
            limits: Limits::default(),
            closed: tokio_util::sync::CancellationToken::new(),
        }
    }

    /// The MCP endpoint, which is also the OAuth resource indicator.
    pub(crate) fn resource(&self) -> String {
        format!("{}/mcp", self.origin)
    }

    /// RFC 9728 metadata location, named in every 401.
    pub(crate) fn resource_metadata_url(&self) -> String {
        format!("{}/.well-known/oauth-protected-resource/mcp", self.origin)
    }

    /// Stops every waiting approval and forgets unexchanged codes, as the
    /// siblings do when the listener stops.
    pub(crate) fn close(&self) {
        self.closed.cancel();
        let ids: Vec<String> = {
            let mut pending = self.lock();
            pending.codes.clear();
            pending.prompts = 0;
            pending.approvals.drain().map(|(id, _)| id).collect()
        };
        for id in ids {
            self.approver.cancel(&id);
        }
    }

    fn lock(&self) -> MutexGuard<'_, Pending> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn prune(&self) {
        let now = Instant::now();
        let expired: Vec<String> = {
            let mut pending = self.lock();
            pending.codes.retain(|_, c| c.expires > now);
            let expired = pending
                .approvals
                .iter()
                .filter(|(_, a)| a.expires <= now)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            for id in &expired {
                pending.approvals.remove(id);
            }
            expired
        };
        for id in expired {
            self.approver.cancel(&id);
        }
    }
}

pub(crate) async fn authorization_server_metadata(State(oauth): State<Arc<OAuth>>) -> Response {
    let o = &oauth.origin;
    Json(json!({
        "issuer": format!("{o}/"),
        "authorization_endpoint": format!("{o}/authorize"),
        "token_endpoint": format!("{o}/token"),
        "registration_endpoint": format!("{o}/register"),
        "revocation_endpoint": format!("{o}/revoke"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
        "token_endpoint_auth_methods_supported": ["none"],
        "revocation_endpoint_auth_methods_supported": ["none"],
        "code_challenge_methods_supported": ["S256"],
        "scopes_supported": all_scopes(),
    }))
    .into_response()
}

pub(crate) async fn protected_resource_metadata(State(oauth): State<Arc<OAuth>>) -> Response {
    Json(json!({
        "resource": oauth.resource(),
        "authorization_servers": [format!("{}/", oauth.origin)],
        "scopes_supported": all_scopes(),
        "resource_name": "PwrRoute",
    }))
    .into_response()
}

fn all_scopes() -> Vec<&'static str> {
    Permission::ALL.iter().map(|p| p.scope()).collect()
}

/// `POST /register` (RFC 7591). Every client is public; registration grants
/// nothing.
pub(crate) async fn register(
    State(oauth): State<Arc<OAuth>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if let Err(limited) = oauth.limits.register.check() {
        return *limited;
    }
    if !is_content_type(&headers, "application/json") {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "Client metadata must be JSON",
        );
    }
    let Ok(Value::Object(metadata)) = serde_json::from_slice::<Value>(&body) else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "Client metadata must be a JSON object",
        );
    };
    match oauth.access.register(&metadata) {
        Ok(client) => (StatusCode::CREATED, Json(client.registration())).into_response(),
        Err(RegisterError::InvalidMetadata(m)) => {
            oauth_error(StatusCode::BAD_REQUEST, "invalid_client_metadata", &m)
        }
        Err(e @ RegisterError::Full) => {
            oauth_error(StatusCode::BAD_REQUEST, "invalid_request", &e.to_string())
        }
        Err(e @ RegisterError::Store(_)) => oauth_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            &e.to_string(),
        ),
    }
}

/// `GET /authorize`. Checks the request, asks the app, and answers with a
/// page that waits. Nothing in the request can approve it.
pub(crate) async fn authorize(
    State(oauth): State<Arc<OAuth>>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if let Err(limited) = oauth.limits.authorize.check() {
        return *limited;
    }
    oauth.prune();
    if DECISION_PARAMS.iter().any(|p| query.contains_key(*p)) {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Authorization decisions are made only in PwrRoute's approval window",
        );
    }
    let (Some(client_id), Some(redirect_uri)) = (query.get("client_id"), query.get("redirect_uri"))
    else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "client_id and redirect_uri are required",
        );
    };
    let Some(client) = oauth.access.client(client_id) else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_client",
            "The requesting MCP client is not registered",
        );
    };
    if !client
        .redirect_uris
        .iter()
        .any(|r| redirect_uri_matches(r, redirect_uri))
    {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "redirect_uri is not registered for this client",
        );
    }

    // From here on, errors go back to the client's redirect URI.
    let state = query.get("state").cloned();
    let fail = |error: &str, description: &str| {
        found(&error_url(
            redirect_uri,
            error,
            description,
            state.as_deref(),
        ))
    };
    let challenge = query.get("code_challenge").map(String::as_str);
    if query.get("response_type").map(String::as_str) != Some("code")
        || query.get("code_challenge_method").map(String::as_str) != Some("S256")
        || !challenge.is_some_and(is_s256_challenge)
    {
        return fail(
            "invalid_request",
            "PwrRoute requires the authorization code flow with an S256 PKCE challenge",
        );
    }
    let resource = oauth.resource();
    if query.get("resource") != Some(&resource) {
        return fail("invalid_target", &format!("resource must be {resource}"));
    }
    let scopes = match parse_scopes(query.get("scope").map(String::as_str)) {
        Ok(scopes) => scopes,
        Err(()) => {
            return fail(
                "invalid_scope",
                "The request names an unknown PwrRoute scope",
            );
        }
    };
    let roles = Role::offered_for(&scopes);
    if roles.is_empty() {
        return fail(
            "invalid_scope",
            "The requested scopes do not cover any PwrRoute role",
        );
    }

    let id = random_b64(32);
    {
        let mut pending = oauth.lock();
        if pending.approvals.len() >= MAX_PENDING || pending.codes.len() >= MAX_PENDING {
            return fail("invalid_request", "Too many pending approvals");
        }
        if pending.prompts >= MAX_PROMPTS {
            return fail(
                "access_denied",
                "Too many approval windows are open in PwrRoute",
            );
        }
        pending.prompts += 1;
        pending.approvals.insert(
            id.clone(),
            Approval {
                expires: Instant::now() + APPROVAL_TTL,
                outcome: None,
            },
        );
    }

    let client_name = client.name().unwrap_or("Local MCP client").to_owned();
    let request = ApprovalRequest {
        id: id.clone(),
        suggested_session_name: oauth.access.suggest_session_name(&client_name),
        client_name,
        redirect_uri: redirect_uri.clone(),
        requested: scopes.iter().copied().collect(),
        roles: roles.clone(),
    };
    let (reply, answer) = ApprovalReply::channel();
    oauth.approver.request(request, reply);

    let grant = Grant {
        client,
        redirect_uri: redirect_uri.clone(),
        state,
        challenge: challenge.unwrap_or_default().to_owned(),
        scopes,
        roles,
    };
    let waiting = Arc::clone(&oauth);
    let approval_id = id.clone();
    tokio::spawn(async move {
        let decision = tokio::select! {
            answer = tokio::time::timeout(APPROVAL_TTL, answer) => match answer {
                Ok(Ok(decision)) => Some(decision),
                // The app dropped the reply: a deny.
                Ok(Err(_)) => Some(ApprovalDecision::Deny),
                Err(_) => None,
            },
            () = waiting.closed.cancelled() => return,
        };
        waiting.finish(&approval_id, decision, grant);
    });

    waiting_page(&id)
}

/// What an approval, once given, turns into a code for.
struct Grant {
    client: Client,
    redirect_uri: String,
    state: Option<String>,
    challenge: String,
    scopes: Permissions,
    roles: Vec<Role>,
}

impl OAuth {
    fn finish(&self, id: &str, decision: Option<ApprovalDecision>, grant: Grant) {
        let mut pending = self.lock();
        pending.prompts = pending.prompts.saturating_sub(1);
        let Some(decision) = decision else {
            // Timed out: the approval is gone; the window should close.
            pending.approvals.remove(id);
            drop(pending);
            self.approver.cancel(id);
            return;
        };
        if !pending.approvals.contains_key(id) {
            return;
        }
        let denied = |description: &str| {
            error_url(
                &grant.redirect_uri,
                "access_denied",
                description,
                grant.state.as_deref(),
            )
        };
        let outcome = match decision {
            ApprovalDecision::Deny => denied("The user denied access to PwrRoute"),
            ApprovalDecision::Approve { session_name, role } => {
                let name = session_name.trim();
                if name.is_empty()
                    || name.chars().count() > crate::access::MAX_SESSION_NAME
                    || !grant.roles.contains(&role)
                {
                    denied("The approval named no valid Session Name and role")
                } else if pending.codes.len() >= MAX_PENDING {
                    denied("Too many pending authorization codes")
                } else {
                    let code = random_b64(32);
                    pending.codes.insert(
                        code.clone(),
                        Code {
                            client_id: grant.client.client_id.clone(),
                            redirect_uri: grant.redirect_uri.clone(),
                            challenge: grant.challenge.clone(),
                            scopes: grant.scopes.clone(),
                            session_name: name.to_owned(),
                            role,
                            expires: Instant::now() + CODE_TTL,
                        },
                    );
                    let mut params = vec![("code", code.as_str())];
                    if let Some(state) = &grant.state {
                        params.push(("state", state));
                    }
                    with_query(&grant.redirect_uri, &params)
                }
            }
        };
        if let Some(approval) = pending.approvals.get_mut(id) {
            approval.outcome = Some(outcome);
        }
    }
}

/// `GET /authorize/status?id=`: the waiting page polls here. Once the
/// person has answered, it redirects to the client, once.
pub(crate) async fn authorize_status(
    State(oauth): State<Arc<OAuth>>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    oauth.prune();
    let id = query.get("id").map(String::as_str).unwrap_or_default();
    let mut pending = oauth.lock();
    match pending.approvals.get(id) {
        None => page(
            StatusCode::NOT_FOUND,
            "Approval request expired",
            "Return to your MCP client and start a new PwrRoute login.",
            None,
        ),
        Some(Approval { outcome: None, .. }) => waiting_page(id),
        Some(Approval {
            outcome: Some(_), ..
        }) => {
            let approval = pending.approvals.remove(id).expect("checked above");
            found(&approval.outcome.expect("checked above"))
        }
    }
}

/// `POST /token`. Only the authorization code grant; tokens do not expire
/// and are not refreshed, they are revoked.
pub(crate) async fn token(
    State(oauth): State<Arc<OAuth>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if let Err(limited) = oauth.limits.token.check() {
        return *limited;
    }
    let form = match form_body(&headers, &body) {
        Ok(form) => form,
        Err(response) => return *response,
    };
    let field = |name: &str| form.get(name).map(String::as_str);
    let bad =
        |error: &str, description: &str| oauth_error(StatusCode::BAD_REQUEST, error, description);
    let Some(client_id) = field("client_id") else {
        return bad("invalid_request", "client_id is required");
    };
    let Some(client) = oauth.access.client(client_id) else {
        return bad("invalid_client", "Invalid client_id");
    };
    match field("grant_type") {
        Some("authorization_code") => {}
        Some("refresh_token") => {
            return bad("invalid_grant", "PwrRoute tokens do not expire or refresh");
        }
        Some(_) => {
            return bad(
                "unsupported_grant_type",
                "The grant type is not supported by this authorization server.",
            );
        }
        None => return bad("invalid_request", "grant_type is required"),
    }
    let (Some(code), Some(verifier)) = (field("code"), field("code_verifier")) else {
        return bad("invalid_request", "code and code_verifier are required");
    };

    oauth.prune();
    let mut pending = oauth.lock();
    let Some(record) = pending
        .codes
        .get(code)
        .filter(|c| c.client_id == client.client_id)
    else {
        return bad("invalid_grant", "Invalid or expired authorization code");
    };
    // A failed check below leaves the code usable until it expires, as in
    // the siblings: the verifier is 256 bits the attacker would have to guess.
    if !pkce_matches(verifier, &record.challenge) {
        return bad(
            "invalid_grant",
            "code_verifier does not match the challenge",
        );
    }
    if field("redirect_uri") != Some(record.redirect_uri.as_str()) {
        return bad(
            "invalid_grant",
            "redirect_uri does not match the authorization request",
        );
    }
    let resource = oauth.resource();
    if field("resource") != Some(resource.as_str()) {
        return bad("invalid_target", &format!("resource must be {resource}"));
    }
    let record = pending.codes.remove(code).expect("checked above");
    drop(pending);

    match oauth.access.create_session(
        &record.session_name,
        record.role,
        &client,
        record.scopes.clone(),
    ) {
        Ok((_, access_token)) => Json(json!({
            "access_token": access_token,
            "token_type": "bearer",
            "scope": record
                .scopes
                .iter()
                .map(|p| p.scope())
                .collect::<Vec<_>>()
                .join(" "),
        }))
        .into_response(),
        Err(e @ (SessionError::NameTaken(_) | SessionError::BadName)) => {
            bad("invalid_grant", &e.to_string())
        }
        Err(e @ (SessionError::Full | SessionError::Store(_))) => oauth_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            &e.to_string(),
        ),
    }
}

/// `POST /revoke` (RFC 7009). Revokes the token's session when the calling
/// client owns it; answers 200 either way.
pub(crate) async fn revoke(
    State(oauth): State<Arc<OAuth>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if let Err(limited) = oauth.limits.revoke.check() {
        return *limited;
    }
    let form = match form_body(&headers, &body) {
        Ok(form) => form,
        Err(response) => return *response,
    };
    let Some(client_id) = form.get("client_id") else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "client_id is required",
        );
    };
    if oauth.access.client(client_id).is_none() {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_client",
            "Invalid client_id",
        );
    }
    let Some(token) = form.get("token") else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "token is required",
        );
    };
    if oauth.access.revoke_token(client_id, token).is_err() {
        return oauth_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "Could not store the revocation",
        );
    }
    Json(json!({})).into_response()
}

fn form_body(headers: &HeaderMap, body: &[u8]) -> Result<HashMap<String, String>, Box<Response>> {
    if !is_content_type(headers, "application/x-www-form-urlencoded") {
        return Err(Box::new(oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "The body must be application/x-www-form-urlencoded",
        )));
    }
    let mut form = HashMap::new();
    for (key, value) in url::form_urlencoded::parse(body) {
        // A repeated parameter is ambiguous (RFC 6749 §3.1).
        if form.insert(key.into_owned(), value.into_owned()).is_some() {
            return Err(Box::new(oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "A parameter is repeated",
            )));
        }
    }
    Ok(form)
}

fn is_content_type(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|v| v.trim().eq_ignore_ascii_case(expected))
}

/// Exact match, or, for loopback redirect URIs, any port (RFC 8252 §7.3)
/// with everything else equal. `localhost` and `127.0.0.1` do not match each
/// other.
fn redirect_uri_matches(registered: &str, requested: &str) -> bool {
    if registered == requested {
        return true;
    }
    let (Ok(r), Ok(q)) = (url::Url::parse(registered), url::Url::parse(requested)) else {
        return false;
    };
    let loopback = |u: &url::Url| matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    loopback(&r)
        && loopback(&q)
        && r.scheme() == q.scheme()
        && r.host_str() == q.host_str()
        && r.path() == q.path()
        && r.query() == q.query()
}

/// A base64url SHA-256 digest: 43 characters, no padding.
fn is_s256_challenge(challenge: &str) -> bool {
    challenge.len() == 43
        && challenge
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// RFC 7636: the verifier is 43–128 unreserved characters, and its SHA-256
/// is the challenge.
fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    let well_formed = (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b));
    let computed = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    well_formed & constant_time_eq(computed.as_bytes(), challenge.as_bytes())
}

/// The requested scopes, or the Reader's when none were asked for.
fn parse_scopes(scope: Option<&str>) -> Result<Permissions, ()> {
    let scope = scope.map(str::trim).unwrap_or_default();
    if scope.is_empty() {
        return Ok(Permissions::from(Permission::DEFAULT_SCOPES));
    }
    scope
        .split_ascii_whitespace()
        .map(|s| Permission::from_scope(s).ok_or(()))
        .collect()
}

fn with_query(base: &str, params: &[(&str, &str)]) -> String {
    let mut url = url::Url::parse(base).expect("redirect URIs are validated at registration");
    url.query_pairs_mut().extend_pairs(params);
    url.into()
}

fn error_url(redirect_uri: &str, error: &str, description: &str, state: Option<&str>) -> String {
    let mut params = vec![("error", error), ("error_description", description)];
    if let Some(state) = state {
        params.push(("state", state));
    }
    with_query(redirect_uri, &params)
}

pub(crate) fn oauth_error(status: StatusCode, error: &str, description: &str) -> Response {
    (
        status,
        Json(json!({ "error": error, "error_description": description })),
    )
        .into_response()
}

/// A 302, as the siblings redirect.
fn found(location: &str) -> Response {
    match HeaderValue::from_str(location) {
        Ok(location) => (StatusCode::FOUND, [(header::LOCATION, location)]).into_response(),
        Err(_) => oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "The redirect URI cannot be used",
        ),
    }
}

/// The page the browser shows while the approval window is up. It cannot
/// approve anything: no form, no script, only a refresh to the status URL.
fn waiting_page(id: &str) -> Response {
    page(
        StatusCode::OK,
        "Continue in PwrRoute",
        "PwrRoute opened its approval window. Review the Session Name and role there to continue.",
        Some(id),
    )
}

fn page(status: StatusCode, title: &str, message: &str, refresh_id: Option<&str>) -> Response {
    let refresh = refresh_id
        .map(|id| {
            format!(
                r#"<meta http-equiv="refresh" content="1;url=/authorize/status?id={}">"#,
                html_escape(id)
            )
        })
        .unwrap_or_default();
    let footer = if refresh_id.is_some() {
        "<p>Waiting for PwrRoute…</p><p>This page cannot approve access. Approval is accepted only in PwrRoute.</p>"
    } else {
        ""
    };
    let html = format!(
        r#"<!doctype html><html><head><meta charset="utf-8">{refresh}<title>{title}</title><style>body{{font:15px system-ui,sans-serif;max-width:32em;margin:4em auto;padding:0 1em}}</style></head><body><h1>{title}</h1><p>{message}</p>{footer}</body></html>"#,
        title = html_escape(title),
        message = html_escape(message),
    );
    let mut response = (status, html).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'",
        ),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    response
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_redirects_match_on_any_port_only() {
        let registered = "http://127.0.0.1:1234/callback";
        assert!(redirect_uri_matches(
            registered,
            "http://127.0.0.1:9999/callback"
        ));
        assert!(!redirect_uri_matches(
            registered,
            "http://localhost:1234/callback"
        ));
        assert!(!redirect_uri_matches(
            registered,
            "http://127.0.0.1:1234/other"
        ));
        assert!(!redirect_uri_matches(
            "https://example.com/cb",
            "https://example.com:8443/cb"
        ));
    }

    #[test]
    fn pkce_needs_a_well_formed_verifier() {
        let verifier = "a".repeat(43);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        assert!(is_s256_challenge(&challenge));
        assert!(pkce_matches(&verifier, &challenge));
        assert!(!pkce_matches(&"a".repeat(44), &challenge));
        let short = "a".repeat(42);
        let short_challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(short.as_bytes()));
        assert!(!pkce_matches(&short, &short_challenge));
    }

    #[test]
    fn scopes_default_to_the_reader() {
        assert_eq!(parse_scopes(None).unwrap(), Role::Reader.permissions());
        assert_eq!(
            parse_scopes(Some("links.open rules.write")).unwrap(),
            Permissions::from([Permission::Open, Permission::RulesWrite])
        );
        assert!(parse_scopes(Some("links.open admin")).is_err());
    }
}
