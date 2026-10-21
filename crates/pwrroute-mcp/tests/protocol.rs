//! MCP over the local link: rmcp clients against a running server, the
//! `pwrroute mcp` proxy end to end, the socket's ownership rules, and the
//! privacy rules.

mod support;

use std::{fs, time::Duration};

use pwrroute_mcp::{Config, McpServer, MemoryAudit, Outcome, Role, fake};
use rmcp::ServiceExt as _;
use serde_json::json;
use support::{Harness, call, eventually, private_endpoint, proxy_command, text};

use pwrroute_mcp::fake::FakeAnswer;

/// A URL nothing else in the test produces, to look for afterwards.
const SECRET_URL: &str = "https://docs.example.org/private/7f3a9c-do-not-persist?token=abc";

#[tokio::test]
async fn a_client_on_the_link_lists_and_calls_tools() {
    let h = Harness::start(FakeAnswer::Approve(Role::Reader)).await;
    let client = h.client("test-agent").await;

    let info = client.peer_info().expect("server info");
    assert_eq!(info.server_info.as_ref().unwrap().name, "PwrRoute");
    assert!(
        info.instructions
            .as_deref()
            .unwrap_or_default()
            .contains("pwrroute_resolve")
    );

    let mut names: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "pwrroute_activity",
            "pwrroute_browsers",
            "pwrroute_open",
            "pwrroute_resolve",
            "pwrroute_rules_apply",
            "pwrroute_rules_list",
            "pwrroute_rules_test",
            "pwrroute_suggest_rule",
        ]
    );

    let result = call(
        &client,
        "pwrroute_resolve",
        json!({ "url": "https://x.com/someone", "opener": { "name": "Slack" } }),
    )
    .await;
    assert_eq!(result.is_error, Some(false), "{}", text(&result));
    let trace = result.structured_content.expect("structured content");
    assert_eq!(trace["url"], "https://x.com/someone");
    assert_eq!(trace["browser"]["name"], "Safari");
    assert_eq!(trace["browser"]["url"], "https://xcancel.com/someone");
    assert_eq!(trace["steps"][0]["kind"], "rewrite");

    let result = call(&client, "pwrroute_browsers", json!({})).await;
    assert_eq!(
        result.structured_content.unwrap()["browsers"][0]["name"],
        "Safari"
    );

    let result = call(
        &client,
        "pwrroute_resolve",
        json!({ "url": "file:///etc/hosts" }),
    )
    .await;
    assert_eq!(result.is_error, Some(true));
    assert!(
        text(&result).starts_with("invalid_input"),
        "{}",
        text(&result)
    );

    let result = call(&client, "pwrroute_nope", json!({})).await;
    assert_eq!(result.is_error, Some(true));
    // The audit names only PwrRoute's tools, never text an agent made up.
    let records = h.audit.records();
    assert_eq!(records.last().unwrap().tool, "unknown_tool");
    client.cancel().await.unwrap();
    h.finish().await;
}

#[tokio::test]
async fn the_proxy_carries_a_client_end_to_end() {
    let h = Harness::start(FakeAnswer::Approve(Role::Opener)).await;
    let (client, mut child) = h.client_via_proxy("claude-code").await;
    assert_eq!(client.list_all_tools().await.unwrap().len(), 8);
    let result = call(
        &client,
        "pwrroute_open",
        json!({ "url": "https://docs.example.org/a" }),
    )
    .await;
    assert_eq!(result.is_error, Some(false), "{}", text(&result));
    assert_eq!(h.router.opened()[0].0.name, "claude-code");
    assert_eq!(h.approver.asked()[0].client_name, "claude-code");

    // The client hanging up ends the proxy cleanly.
    client.cancel().await.unwrap();
    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .expect("the proxy exits")
        .unwrap();
    assert!(status.success(), "{status}");
    h.finish().await;
}

#[tokio::test]
async fn the_proxy_exits_when_the_app_stops() {
    let h = Harness::start(FakeAnswer::Approve(Role::Reader)).await;
    let (client, child) = h.client_via_proxy("claude-code").await;
    h.finish().await;
    let output = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
        .await
        .expect("the proxy exits")
        .unwrap();
    assert!(
        output.status.success(),
        "{}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    drop(client);
}

#[tokio::test]
async fn the_proxy_says_so_when_pwrroute_is_not_running() {
    let output = proxy_command(&private_endpoint())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("PwrRoute is not running"), "{stderr}");
}

#[tokio::test]
async fn a_second_server_does_not_take_over_the_endpoint() {
    let h = Harness::start(FakeAnswer::Approve(Role::Reader)).await;
    let (backends, _, _) = fake::backends();
    let second = McpServer::start(
        Config {
            endpoint: h.endpoint.clone(),
            approval_wait: Duration::from_secs(1),
        },
        h.agents.clone(),
        backends,
        h.approver.clone(),
        std::sync::Arc::new(MemoryAudit::default()),
    )
    .await;
    let error = second.expect_err("the endpoint is taken");
    assert!(
        format!("{error:#}").contains("already serving"),
        "{error:#}"
    );
    // The first still answers.
    let client = h.client("test-agent").await;
    assert_eq!(client.list_all_tools().await.unwrap().len(), 8);
    client.cancel().await.unwrap();
    h.finish().await;
}

#[cfg(unix)]
#[tokio::test]
async fn the_socket_is_owner_only_replaces_a_stale_one_and_is_removed_on_stop() {
    use std::os::unix::fs::PermissionsExt as _;

    let endpoint = private_endpoint();
    // A socket file left behind by a PwrRoute that crashed: nothing answers.
    drop(std::os::unix::net::UnixListener::bind(endpoint.path()).unwrap());
    assert!(endpoint.path().exists());
    // A process another test is spawning can hold a copy of that listener
    // until it execs; wait until the file is truly stale.
    let path = endpoint.path().to_path_buf();
    eventually(|| std::os::unix::net::UnixStream::connect(&path).is_err()).await;

    let h = Harness::start_in(
        std::env::temp_dir().join(format!("pwrroute-mcp-stale-{}", std::process::id())),
        endpoint.clone(),
        std::sync::Arc::new(fake::FakeApprover::new(FakeAnswer::Approve(Role::Reader))),
        Duration::from_secs(10),
    )
    .await;
    let mode = fs::metadata(endpoint.path()).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let client = h.client("test-agent").await;
    client.cancel().await.unwrap();
    h.finish().await;
    assert!(!endpoint.path().exists());
}

#[tokio::test]
async fn stopping_hangs_up_on_connected_clients() {
    let h = Harness::start(FakeAnswer::Approve(Role::Reader)).await;
    let client = h.client("test-agent").await;
    assert!(client.list_all_tools().await.is_ok());
    h.finish().await;
    let after = tokio::time::timeout(Duration::from_secs(10), client.list_all_tools())
        .await
        .expect("the call fails rather than hangs");
    assert!(after.is_err());
}

#[tokio::test]
async fn urls_stay_off_disk_and_out_of_the_audit_log() {
    let h = Harness::start(FakeAnswer::Approve(Role::Opener)).await;
    let client = h.client("test-agent").await;
    for tool in ["pwrroute_resolve", "pwrroute_open"] {
        let result = call(&client, tool, json!({ "url": SECRET_URL })).await;
        assert_eq!(result.is_error, Some(false), "{}", text(&result));
    }
    let result = call(&client, "pwrroute_activity", json!({})).await;
    assert!(text(&result).contains(SECRET_URL), "Opener sees full URLs");

    let records = h.audit.records();
    let tools: Vec<&str> = records.iter().map(|r| r.tool.as_str()).collect();
    assert_eq!(
        tools,
        ["pwrroute_resolve", "pwrroute_open", "pwrroute_activity"]
    );
    assert!(records.iter().all(|r| r.agent == "test-agent"));
    assert!(records.iter().all(|r| r.outcome == Outcome::Ok));
    let agent = &h.agents.agents()[0];
    assert_eq!(agent.calls, 3);
    assert_eq!(agent.connections, 1);
    assert!(agent.last_used_at.is_some());
    let serialized = serde_json::to_string(&records).unwrap();
    assert!(!serialized.contains("example.org"), "{serialized}");
    // Sorted: the workspace build turns on serde_json's preserve_order.
    let mut fields: Vec<String> = serde_json::to_value(&records[0])
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    fields.sort();
    assert_eq!(
        fields,
        [
            "agent",
            "connection",
            "duration",
            "outcome",
            "started",
            "tool"
        ],
        "no note on an ordinary call, and never the arguments"
    );

    client.cancel().await.unwrap();
    let agents = h.agents.clone();
    eventually(|| agents.agents()[0].connections == 0).await;
    let dir = h.stop().await;
    let mut files = Vec::new();
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let contents = fs::read_to_string(&path).unwrap();
        assert!(!contents.contains("example.org"), "{}", path.display());
        files.push(path.file_name().unwrap().to_string_lossy().into_owned());
    }
    assert_eq!(files, ["mcp-agents.json"], "only the approval store");
    fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn a_2025_protocol_client_speaks_plain_json_lines() {
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

    let h = Harness::start(FakeAnswer::Approve(Role::Reader)).await;
    let stream = pwrroute_mcp::link::connect(&h.endpoint).await.unwrap();
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();
    let mut send = async |message: serde_json::Value| {
        write
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
    };

    send(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "claude-code", "version": "2.1.0" }
        }
    }))
    .await;
    let reply: serde_json::Value =
        serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(reply["result"]["serverInfo"]["name"], "PwrRoute");

    send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await;
    send(json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "pwrroute_resolve", "arguments": { "url": "https://example.org/" } }
    }))
    .await;
    let reply: serde_json::Value =
        serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(reply["id"], 2);
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    assert_eq!(
        reply["result"]["structuredContent"]["browser"]["name"],
        "Firefox"
    );
    assert_eq!(h.approver.asked()[0].client_version, "2.1.0");
    h.finish().await;
}

#[tokio::test]
async fn a_host_thread_serves_until_stopped() {
    let endpoint = private_endpoint();
    let state_dir = std::env::temp_dir().join(format!("pwrroute-mcp-host-{}", std::process::id()));
    let agents = pwrroute_mcp::Agents::open(Some(&state_dir)).unwrap();
    let (backends, _, _) = fake::backends();
    let host = tokio::task::spawn_blocking({
        let endpoint = endpoint.clone();
        move || {
            pwrroute_mcp::McpHost::start(
                Config {
                    endpoint,
                    approval_wait: Duration::from_secs(1),
                },
                agents,
                backends,
                std::sync::Arc::new(fake::FakeApprover::approving(Role::Reader)),
                std::sync::Arc::new(MemoryAudit::default()),
            )
        }
    })
    .await
    .unwrap()
    .expect("the host starts");
    assert_eq!(host.endpoint(), &endpoint);
    let stream = pwrroute_mcp::link::connect(&endpoint).await.unwrap();
    let client = support::client_config("test-agent")
        .serve(stream)
        .await
        .unwrap();
    assert_eq!(client.list_all_tools().await.unwrap().len(), 8);
    client.cancel().await.unwrap();
    tokio::task::spawn_blocking(move || host.stop())
        .await
        .unwrap();
    assert!(pwrroute_mcp::link::connect(&endpoint).await.is_err());
    let _ = fs::remove_dir_all(state_dir);
}
