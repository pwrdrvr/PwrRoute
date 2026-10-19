//! A running server on a private socket (a pipe on Windows) with fake
//! backends and a fake approver, and MCP clients that reach it over the
//! link directly or through the `pwrroute mcp` proxy binary.

#![allow(dead_code)]

use std::{
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use pwrroute_mcp::{
    Agents, Config, Endpoint, McpServer, MemoryAudit,
    fake::{self, FakeAnswer, FakeApprover, FakeRouter, FakeRules},
    link::{self, ENDPOINT_ENV},
};
use rmcp::{
    RoleClient, ServiceExt as _,
    model::{
        CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, Implementation,
    },
    service::RunningService,
};
use serde_json::Value;

pub type Client = RunningService<RoleClient, ClientConfig>;

pub struct Harness {
    pub server: Option<McpServer>,
    pub endpoint: Endpoint,
    pub agents: Agents,
    pub approver: Arc<FakeApprover>,
    pub audit: Arc<MemoryAudit>,
    pub router: Arc<FakeRouter>,
    pub rules: Arc<FakeRules>,
    pub state_dir: PathBuf,
}

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    format!(
        "{tag}-{}-{}-{nanos}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// A fresh endpoint no other test uses. Socket paths stay short: macOS
/// allows about 100 bytes.
pub fn private_endpoint() -> Endpoint {
    if cfg!(windows) {
        Endpoint::new(format!(r"\\.\pipe\{}", unique("pwrroute-mcp-test")))
    } else {
        Endpoint::new(std::env::temp_dir().join(format!("{}.sock", unique("prm"))))
    }
}

impl Harness {
    pub async fn start(answer: FakeAnswer) -> Self {
        Self::start_with(answer, Duration::from_secs(10)).await
    }

    pub async fn start_with(answer: FakeAnswer, approval_wait: Duration) -> Self {
        let state_dir = std::env::temp_dir().join(unique("pwrroute-mcp-state"));
        Self::start_in(
            state_dir,
            private_endpoint(),
            Arc::new(FakeApprover::new(answer)),
            approval_wait,
        )
        .await
    }

    pub async fn start_in(
        state_dir: PathBuf,
        endpoint: Endpoint,
        approver: Arc<FakeApprover>,
        approval_wait: Duration,
    ) -> Self {
        let agents = Agents::open(Some(&state_dir)).expect("open the state dir");
        let (backends, router, rules) = fake::backends();
        let audit = Arc::new(MemoryAudit::default());
        let server = McpServer::start(
            Config {
                endpoint: endpoint.clone(),
                approval_wait,
            },
            agents.clone(),
            backends,
            approver.clone(),
            audit.clone(),
        )
        .await
        .expect("start the server");
        Self {
            server: Some(server),
            endpoint,
            agents,
            approver,
            audit,
            router,
            rules,
            state_dir,
        }
    }

    /// An MCP client named `name`, connected over the link and initialized.
    pub async fn client(&self, name: &str) -> Client {
        let stream = link::connect(&self.endpoint).await.expect("connect");
        client_config(name).serve(stream).await.expect("initialize")
    }

    /// An MCP client named `name` that reaches the server the way Claude
    /// Code does: by launching `pwrroute mcp` and speaking over its stdio.
    pub async fn client_via_proxy(&self, name: &str) -> (Client, tokio::process::Child) {
        let mut child = proxy_command(&self.endpoint)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn the proxy");
        let stdout = child.stdout.take().unwrap();
        let stdin = child.stdin.take().unwrap();
        let client = client_config(name)
            .serve((stdout, stdin))
            .await
            .expect("initialize through the proxy");
        (client, child)
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
}

pub fn client_config(name: &str) -> ClientConfig {
    ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new(name.to_owned(), "1.0.0"),
    )
}

pub fn proxy_command(endpoint: &Endpoint) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_pwrroute-mcp-proxy"));
    command
        .env(ENDPOINT_ENV, endpoint.path())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

pub async fn call(client: &Client, tool: &str, args: Value) -> CallToolResult {
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

/// Waits until `check` holds, for things another task finishes.
pub async fn eventually(mut check: impl FnMut() -> bool) {
    for _ in 0..200 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(check(), "condition never held");
}
