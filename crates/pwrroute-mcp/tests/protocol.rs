//! MCP over the wire: an rmcp client against a running server, plus the
//! HTTP-level guards and the privacy rules.

mod support;

use std::fs;

use pwrroute_mcp::{Outcome, Role};
use reqwest::{StatusCode, header};
use serde_json::{Value, json};
use support::{ALL_SCOPES, Harness, call, text};

/// A URL nothing else in the test produces, to look for afterwards.
const SECRET_URL: &str = "https://docs.example.org/private/7f3a9c-do-not-persist?token=abc";

#[tokio::test]
async fn an_mcp_client_lists_and_calls_tools() {
    let h = Harness::start(Role::Reader).await;
    let token = h.login(None).await;
    let client = h.client(&token).await;

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
    let browsers = result.structured_content.unwrap();
    assert_eq!(browsers["browsers"][0]["name"], "Safari");

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
    client.cancel().await.unwrap();
    h.finish().await;
}

#[tokio::test]
async fn the_transport_is_stateless_json() {
    let h = Harness::start(Role::Reader).await;
    let token = h.login(None).await;
    let response = h
        .http
        .post(h.resource())
        .bearer_auth(&token)
        .header(header::ACCEPT, "application/json, text/event-stream")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "raw", "version": "0" }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get("mcp-session-id").is_none());
    assert!(
        response.headers()[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("application/json")
    );
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["result"]["serverInfo"]["name"], "PwrRoute");

    // No session to carry: the next POST stands alone, as Claude Code and
    // Codex send it on the 2025 protocol versions.
    let response = h
        .http
        .post(h.resource())
        .bearer_auth(&token)
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-06-18")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "pwrroute_resolve", "arguments": { "url": "https://example.org/" } }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["result"]["isError"], false, "{body}");
    assert_eq!(
        body["result"]["structuredContent"]["browser"]["name"],
        "Firefox"
    );

    for method in [reqwest::Method::GET, reqwest::Method::DELETE] {
        let response = h
            .http
            .request(method.clone(), h.resource())
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "{method}"
        );
        assert_eq!(response.headers()[header::ALLOW], "POST");
    }
    h.finish().await;
}

#[tokio::test]
async fn host_and_origin_must_be_loopback() {
    let h = Harness::start(Role::Reader).await;
    let port = h.server.as_ref().unwrap().local_addr().port();
    let get = |host: String, origin: Option<&'static str>| {
        let h = &h;
        async move {
            let mut request = h
                .http
                .get(h.url("/.well-known/oauth-authorization-server"))
                .header(header::HOST, host);
            if let Some(origin) = origin {
                request = request.header(header::ORIGIN, origin);
            }
            let response = request.send().await.unwrap();
            let status = response.status();
            let body = response.json::<Value>().await.unwrap_or(Value::Null);
            (status, body)
        }
    };
    let host = format!("127.0.0.1:{port}");
    assert_eq!(get(host.clone(), None).await.0, StatusCode::OK);
    assert_eq!(
        get(host.clone(), Some("http://localhost:6274")).await.0,
        StatusCode::OK
    );
    // A DNS-rebinding page arrives with its own name as the Host.
    for bad_host in [
        format!("localhost:{port}"),
        format!("attacker.example:{port}"),
        "127.0.0.1".to_owned(),
    ] {
        let (status, body) = get(bad_host, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "invalid_host");
    }
    for origin in [
        "https://attacker.example",
        "null",
        "file://",
        "ftp://localhost",
    ] {
        let (status, body) = get(host.clone(), Some(origin)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{origin}");
        assert_eq!(body["error"], "invalid_origin");
    }
    h.finish().await;
}

#[tokio::test]
async fn oversized_and_encoded_bodies_are_refused() {
    let h = Harness::start(Role::Reader).await;
    let token = h.login(None).await;
    let huge = "x".repeat(1024 * 1024 + 1);
    let response = h
        .http
        .post(h.resource())
        .bearer_auth(&token)
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header(header::CONTENT_TYPE, "application/json")
        .body(huge)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let response = h
        .http
        .post(h.resource())
        .bearer_auth(&token)
        .header(header::CONTENT_ENCODING, "gzip")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    h.finish().await;
}

#[tokio::test]
async fn an_unknown_token_gets_the_401_challenge() {
    let h = Harness::start(Role::Reader).await;
    for auth in ["Bearer prmcp_not-a-token", "Basic Zm9vOmJhcg==", "Bearer "] {
        let response = h
            .http
            .post(h.resource())
            .header(header::AUTHORIZATION, auth)
            .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{auth}");
        assert!(response.headers().contains_key(header::WWW_AUTHENTICATE));
    }
    h.finish().await;
}

#[tokio::test]
async fn urls_stay_off_disk_and_out_of_the_audit_log() {
    let h = Harness::start(Role::Opener).await;
    let token = h.login(Some(ALL_SCOPES)).await;
    let client = h.client(&token).await;
    for tool in ["pwrroute_resolve", "pwrroute_open"] {
        let result = call(&client, tool, json!({ "url": SECRET_URL })).await;
        assert_eq!(result.is_error, Some(false), "{}", text(&result));
    }
    let result = call(&client, "pwrroute_activity", json!({})).await;
    assert!(text(&result).contains(SECRET_URL), "Opener sees full URLs");
    client.cancel().await.unwrap();

    let records = h.audit.records();
    let tools: Vec<&str> = records.iter().map(|r| r.tool.as_str()).collect();
    assert_eq!(
        tools,
        ["pwrroute_resolve", "pwrroute_open", "pwrroute_activity"]
    );
    let session = &h.access.sessions()[0];
    assert!(records.iter().all(|r| r.session_id == session.id));
    assert!(records.iter().all(|r| r.outcome == Outcome::Ok));
    assert_eq!(session.calls, 3);
    assert!(session.last_used_at.is_some());
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
        ["duration", "outcome", "sessionId", "started", "tool"]
    );

    let dir = h.stop().await;
    let mut files = 0;
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let contents = fs::read_to_string(&path).unwrap();
        assert!(!contents.contains("example.org"), "{}", path.display());
        assert!(!contents.contains(&token), "{}", path.display());
        files += 1;
    }
    assert_eq!(files, 2, "only the client and session stores");
    fs::remove_dir_all(dir).unwrap();
}
