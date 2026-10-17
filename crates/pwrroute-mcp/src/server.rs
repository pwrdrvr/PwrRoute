//! The loopback listener: one HTTP server for the OAuth endpoints and the
//! MCP endpoint, behind a guard against DNS rebinding and cross-site
//! requests.

use std::{net::SocketAddr, sync::Arc};

use anyhow::{Context as _, Result};
use axum::{
    Json, Router,
    body::Body,
    extract::{ConnectInfo, DefaultBodyLimit, FromRef, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{MethodRouter, any, get, post},
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::never::NeverSessionManager,
};
use serde_json::json;
use tokio::{net::TcpListener, sync::Semaphore, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::{
    access::AccessControl,
    approval::Approver,
    audit::AuditSink,
    backend::Backends,
    oauth::{self, OAuth},
    tools::Tools,
};

/// PwrRoute's MCP port. The design's 51731 is a placeholder that PwrGit
/// already listens on (PwrSnap holds 51729); PwrRoute takes the next odd
/// port.
pub const DEFAULT_PORT: u16 = 51733;
/// The largest MCP request body. A rules patch is the biggest input.
const MAX_MCP_BODY: usize = 1024 * 1024;
/// The largest OAuth request body (the SDK's parsers' default).
const MAX_OAUTH_BODY: usize = 100 * 1024;
/// MCP requests in flight at once; more get a 429.
const MAX_IN_FLIGHT: usize = 32;

/// What to listen on.
#[derive(Debug, Clone)]
pub struct Config {
    /// [`DEFAULT_PORT`] in the app; 0 picks a free port, for tests.
    pub port: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self { port: DEFAULT_PORT }
    }
}

#[derive(Clone)]
struct AppState {
    oauth: Arc<OAuth>,
    mcp: StreamableHttpService<Tools, NeverSessionManager>,
    in_flight: Arc<Semaphore>,
}

impl FromRef<AppState> for Arc<OAuth> {
    fn from_ref(state: &AppState) -> Self {
        Arc::clone(&state.oauth)
    }
}

/// A running listener. Dropping it does not stop it; call [`Self::stop`].
pub struct McpServer {
    addr: SocketAddr,
    oauth: Arc<OAuth>,
    shutdown: CancellationToken,
    task: JoinHandle<()>,
}

impl std::fmt::Debug for McpServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpServer")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

impl McpServer {
    /// Binds `127.0.0.1:<port>` and serves until [`Self::stop`]. A busy port
    /// is an error, not a fallback: clients are configured with the fixed
    /// URL. Must be called inside a Tokio runtime.
    pub async fn start(
        config: Config,
        access: AccessControl,
        backends: Backends,
        approver: Arc<dyn Approver>,
        audit: Arc<dyn AuditSink>,
    ) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", config.port))
            .await
            .with_context(|| {
                format!(
                    "PwrRoute could not use local port {}; another process may be using it",
                    config.port
                )
            })?;
        let addr = listener.local_addr()?;
        let authority = format!("127.0.0.1:{}", addr.port());
        let origin = format!("http://{authority}");
        let shutdown = CancellationToken::new();

        let oauth = Arc::new(OAuth::new(access.clone(), approver, origin));
        let tools = Tools {
            backends,
            access,
            audit,
        };
        let mcp = StreamableHttpService::new(
            move || Ok(tools.clone()),
            Arc::new(NeverSessionManager::default()),
            StreamableHttpServerConfig::default()
                .with_legacy_session_mode(false)
                .with_json_response(true)
                .with_allowed_hosts([authority.clone()])
                .with_max_request_body_bytes(MAX_MCP_BODY)
                .with_sse_keep_alive(None)
                .with_cancellation_token(shutdown.child_token()),
        );
        let state = AppState {
            oauth: Arc::clone(&oauth),
            mcp,
            in_flight: Arc::new(Semaphore::new(MAX_IN_FLIGHT)),
        };
        let app = router(state, authority);

        let stop = shutdown.clone();
        let task = tokio::spawn(async move {
            let serve = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(stop.cancelled_owned());
            // The listener only fails when the socket does; there is no one
            // to tell but the next start.
            let _ = serve.await;
        });
        Ok(Self {
            addr,
            oauth,
            shutdown,
            task,
        })
    }

    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// `http://127.0.0.1:<port>/mcp`, what Settings shows and clients add.
    #[must_use]
    pub fn mcp_url(&self) -> String {
        self.oauth.resource()
    }

    /// Stops listening, denies every approval still waiting, and forgets
    /// unexchanged codes. Sessions are kept.
    pub async fn stop(self) {
        self.oauth.close();
        self.shutdown.cancel();
        let _ = self.task.await;
    }
}

fn router(state: AppState, authority: String) -> Router {
    let cors = |route: MethodRouter<AppState>, allow: &'static str| {
        route
            .options(preflight)
            .fallback(move || async move { method_not_allowed(allow) })
            .layer(middleware::map_response(allow_any_origin))
    };
    Router::new()
        .route("/mcp", any(mcp).layer(DefaultBodyLimit::max(MAX_MCP_BODY)))
        .route(
            "/authorize",
            get(oauth::authorize).fallback(|| async {
                json_error(
                    StatusCode::METHOD_NOT_ALLOWED,
                    "method_not_allowed",
                    "Authorization decisions are accepted only in PwrRoute",
                    Some("GET"),
                )
            }),
        )
        .route(
            "/authorize/status",
            get(oauth::authorize_status).fallback(|| async { method_not_allowed("GET") }),
        )
        .route("/register", cors(post(oauth::register), "POST, OPTIONS"))
        .route("/token", cors(post(oauth::token), "POST, OPTIONS"))
        .route("/revoke", cors(post(oauth::revoke), "POST, OPTIONS"))
        .route(
            "/.well-known/oauth-authorization-server",
            cors(get(oauth::authorization_server_metadata), "GET, OPTIONS"),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            cors(get(oauth::protected_resource_metadata), "GET, OPTIONS"),
        )
        .fallback(|| async {
            (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response()
        })
        .layer(DefaultBodyLimit::max(MAX_OAUTH_BODY))
        .layer(middleware::from_fn_with_state(Arc::new(authority), guard))
        .with_state(state)
}

/// Every request: from loopback, addressed to `127.0.0.1:<port>` exactly
/// (a rebinding page's Host is its own name), and, if a browser sent an
/// Origin, from a loopback page.
async fn guard(
    State(authority): State<Arc<String>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    let forbidden =
        |error: &str| (StatusCode::FORBIDDEN, Json(json!({ "error": error }))).into_response();
    if !peer.ip().is_loopback() {
        return forbidden("non_loopback_client");
    }
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok());
    if host != Some(authority.as_str()) {
        return forbidden("invalid_host");
    }
    if !origin_allowed(request.headers()) {
        return forbidden("invalid_origin");
    }
    if request
        .headers()
        .get(header::CONTENT_ENCODING)
        .is_some_and(|e| !e.as_bytes().eq_ignore_ascii_case(b"identity"))
    {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Json(json!({ "error": "unsupported_media_type" })),
        )
            .into_response();
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// No Origin (a native client), or an http(s) page on a loopback host, any
/// port. `null` and anything unparseable are refused.
fn origin_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Some(url) = origin.to_str().ok().and_then(|o| url::Url::parse(o).ok()) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
        && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
}

/// `POST /mcp`, bearer token required. The token's session rides along to
/// the tools in the request's extensions.
async fn mcp(State(state): State<AppState>, mut request: Request) -> Response {
    if request.method() != Method::POST {
        return json_error(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "PwrRoute's MCP endpoint accepts only POST",
            Some("POST"),
        );
    }
    let principal = bearer(request.headers()).and_then(|t| state.oauth.access.authenticate(t));
    let Some(principal) = principal else {
        let mut response = (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "unauthorized" })),
        )
            .into_response();
        let challenge = format!(
            r#"Bearer resource_metadata="{}""#,
            state.oauth.resource_metadata_url()
        );
        if let Ok(value) = HeaderValue::from_str(&challenge) {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, value);
        }
        return response;
    };
    let Ok(_permit) = state.in_flight.try_acquire() else {
        return json_error(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_requests",
            "Too many MCP requests are in flight",
            None,
        );
    };
    request.extensions_mut().insert(principal);
    state.mcp.handle(request).await.map(Body::new)
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?.trim();
    let (scheme, token) = value.split_once(char::is_whitespace)?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

async fn preflight() -> Response {
    (
        StatusCode::NO_CONTENT,
        [
            (header::ACCESS_CONTROL_ALLOW_METHODS, "GET, POST, OPTIONS"),
            (
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                "Content-Type, Authorization, MCP-Protocol-Version",
            ),
        ],
    )
        .into_response()
}

/// The SDK's `cors()` on metadata, registration, token and revocation. The
/// guard has already refused any non-loopback Origin.
async fn allow_any_origin(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response
}

fn method_not_allowed(allow: &'static str) -> Response {
    json_error(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "The method is not allowed for this endpoint",
        Some(allow),
    )
}

fn json_error(
    status: StatusCode,
    error: &str,
    description: &str,
    allow: Option<&'static str>,
) -> Response {
    let mut response = (
        status,
        Json(json!({ "error": error, "error_description": description })),
    )
        .into_response();
    if let Some(allow) = allow {
        response
            .headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static(allow));
    }
    response
}
