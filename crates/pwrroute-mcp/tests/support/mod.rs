//! A running server on an ephemeral port with fake backends, and an OAuth
//! client that walks the flow the way Claude Code and Codex do: 401,
//! discovery, registration, authorize, wait, token.

#![allow(dead_code)]

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use pwrroute_mcp::{
    AccessControl, Config, McpServer, MemoryAudit, Role,
    fake::{self, FakeApprover, FakeRouter, FakeRules},
};
use reqwest::{StatusCode, header};
use rmcp::{
    RoleClient, ServiceExt as _,
    model::{CallToolRequestParams, CallToolResult},
    service::RunningService,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const REDIRECT_URI: &str = "http://127.0.0.1:9/callback";

pub struct Harness {
    pub server: Option<McpServer>,
    pub base: String,
    pub access: AccessControl,
    pub approver: Arc<FakeApprover>,
    pub audit: Arc<MemoryAudit>,
    pub router: Arc<FakeRouter>,
    pub rules: Arc<FakeRules>,
    pub http: reqwest::Client,
    pub state_dir: PathBuf,
}

impl Harness {
    pub async fn start(role: Role) -> Self {
        let state_dir = std::env::temp_dir().join(format!(
            "pwrroute-mcp-test-{}-{}",
            std::process::id(),
            URL_SAFE_NO_PAD.encode(rand_bytes())
        ));
        Self::start_in(state_dir, Arc::new(FakeApprover::approving(role))).await
    }

    pub async fn start_in(state_dir: PathBuf, approver: Arc<FakeApprover>) -> Self {
        let access = AccessControl::open(Some(&state_dir)).expect("open the state dir");
        let (backends, router, rules) = fake::backends();
        let audit = Arc::new(MemoryAudit::default());
        let server = McpServer::start(
            Config { port: 0 },
            access.clone(),
            backends,
            approver.clone(),
            audit.clone(),
        )
        .await
        .expect("start the server");
        let base = format!("http://{}", server.local_addr());
        Self {
            server: Some(server),
            base,
            access,
            approver,
            audit,
            router,
            rules,
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            state_dir,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    pub fn resource(&self) -> String {
        self.url("/mcp")
    }

    pub async fn stop(mut self) -> PathBuf {
        if let Some(server) = self.server.take() {
            server.stop().await;
        }
        self.state_dir.clone()
    }

    /// Stops the server and removes its state directory.
    pub async fn finish(self) {
        let dir = self.stop().await;
        let _ = std::fs::remove_dir_all(dir);
    }

    pub async fn register(&self) -> String {
        let response = self
            .http
            .post(self.url("/register"))
            .json(&json!({
                "redirect_uris": [REDIRECT_URI],
                "client_name": "Test agent",
                "token_endpoint_auth_method": "none",
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body: Value = response.json().await.unwrap();
        body["client_id"].as_str().unwrap().to_owned()
    }

    /// `GET /authorize`, then polls the waiting page until it redirects.
    /// Returns the redirect's query parameters.
    pub async fn authorize(
        &self,
        client_id: &str,
        pkce: &Pkce,
        scope: Option<&str>,
    ) -> HashMap<String, String> {
        let mut query = vec![
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", REDIRECT_URI),
            ("code_challenge", &pkce.challenge),
            ("code_challenge_method", "S256"),
            ("state", "xyz"),
        ];
        let resource = self.resource();
        query.push(("resource", &resource));
        if let Some(scope) = scope {
            query.push(("scope", scope));
        }
        let response = self.get_with_query("/authorize", &query).await;
        self.follow_waiting_page(response).await
    }

    pub async fn get_with_query(&self, path: &str, query: &[(&str, &str)]) -> reqwest::Response {
        let mut url = url::Url::parse(&self.url(path)).unwrap();
        url.query_pairs_mut().extend_pairs(query);
        self.http.get(url).send().await.unwrap()
    }

    /// Follows the meta refresh until the status URL redirects to the
    /// client, as a browser would.
    pub async fn follow_waiting_page(
        &self,
        mut response: reqwest::Response,
    ) -> HashMap<String, String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if response.status() == StatusCode::FOUND {
                return redirect_params(&response);
            }
            assert_eq!(response.status(), StatusCode::OK, "waiting page");
            let page = response.text().await.unwrap();
            let id = waiting_id(&page).expect("a waiting page");
            assert!(Instant::now() < deadline, "approval never finished");
            tokio::time::sleep(Duration::from_millis(20)).await;
            response = self
                .get_with_query("/authorize/status", &[("id", &id)])
                .await;
        }
    }

    pub async fn exchange(&self, client_id: &str, code: &str, pkce: &Pkce) -> reqwest::Response {
        let resource = self.resource();
        self.http
            .post(self.url("/token"))
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", client_id),
                ("code", code),
                ("code_verifier", &pkce.verifier),
                ("redirect_uri", REDIRECT_URI),
                ("resource", &resource),
            ])
            .send()
            .await
            .unwrap()
    }

    /// The whole flow; returns the access token.
    pub async fn login(&self, scope: Option<&str>) -> String {
        let client_id = self.register().await;
        let pkce = Pkce::new();
        let params = self.authorize(&client_id, &pkce, scope).await;
        let code = params.get("code").expect("a code");
        let response = self.exchange(&client_id, code, &pkce).await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["token_type"], "bearer");
        assert!(body.get("refresh_token").is_none());
        assert!(body.get("expires_in").is_none());
        body["access_token"].as_str().unwrap().to_owned()
    }

    /// An rmcp client, connected and initialized with `token`.
    pub async fn client(&self, token: &str) -> RunningService<RoleClient, ()> {
        let mut config = StreamableHttpClientTransportConfig::with_uri(self.resource());
        config.auth_header = Some(token.to_owned());
        let transport = StreamableHttpClientTransport::from_config(config);
        ().serve(transport).await.expect("initialize")
    }
}

/// Every scope: the client asks for all, so every role can be offered.
pub const ALL_SCOPES: &str =
    "links.resolve browsers.read rules.read activity.hosts activity.urls links.open rules.write";

pub async fn call(
    client: &RunningService<RoleClient, ()>,
    tool: &str,
    args: Value,
) -> CallToolResult {
    let params = CallToolRequestParams::new(tool.to_owned())
        .with_arguments(args.as_object().cloned().unwrap_or_default());
    client.call_tool(params).await.expect("tools/call")
}

pub fn text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn new() -> Self {
        let verifier = URL_SAFE_NO_PAD.encode(rand_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Self {
            verifier,
            challenge,
        }
    }
}

fn rand_bytes() -> [u8; 32] {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).unwrap();
    bytes
}

pub fn waiting_id(page: &str) -> Option<String> {
    let start = page.find("/authorize/status?id=")? + "/authorize/status?id=".len();
    let rest = &page[start..];
    Some(rest[..rest.find('"')?].to_owned())
}

pub fn redirect_params(response: &reqwest::Response) -> HashMap<String, String> {
    let location = response.headers()[header::LOCATION].to_str().unwrap();
    assert!(location.starts_with(REDIRECT_URI), "{location}");
    url::Url::parse(location)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}
