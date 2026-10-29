//! The first-connect approval: one prompt per client name, remembered, and
//! changed, revoked or forgotten from Settings.

mod support;

use std::{sync::Arc, time::Duration};

use pwrroute_mcp::{
    ApprovalDecision, Outcome, Role,
    fake::{FakeAnswer, FakeApprover},
};
use serde_json::json;
use support::{Harness, call, config, eventually, private_endpoint, text};

const LINK: &str = "https://docs.example.org/page";

#[tokio::test]
async fn a_new_client_is_asked_about_once_and_remembered() {
    let h = Harness::start(FakeAnswer::Approve(Role::Opener)).await;
    let first = h.client("claude-code").await;
    // The window goes up as the client connects, before any tool call.
    let approver = h.approver.clone();
    eventually(|| approver.asked().len() == 1).await;
    let asked = &h.approver.asked()[0];
    assert_eq!(asked.client_name, "claude-code");
    assert_eq!(asked.client_version, "1.0.0");
    assert_eq!(asked.roles, Role::ALL);

    let result = call(&first, "pwrroute_open", json!({ "url": LINK })).await;
    assert_eq!(result.is_error, Some(false), "{}", text(&result));

    // Another connection under the same name is not asked about again.
    let second = h.client("claude-code").await;
    let result = call(&second, "pwrroute_resolve", json!({ "url": LINK })).await;
    assert_eq!(result.is_error, Some(false), "{}", text(&result));
    assert_eq!(h.approver.asked().len(), 1);

    let agents = h.agents.agents();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].name, "claude-code");
    assert_eq!(agents[0].role, Role::Opener);
    assert_eq!(agents[0].connections, 2);

    // A different client is its own question.
    let other = h.client("codex-mcp-client").await;
    let approver = h.approver.clone();
    eventually(|| approver.asked().len() == 2).await;
    for client in [first, second, other] {
        client.cancel().await.unwrap();
    }
    h.finish().await;
}

#[tokio::test]
async fn a_denial_refuses_the_connection_and_a_new_one_is_asked_again() {
    let h = Harness::start(FakeAnswer::Deny).await;
    let client = h.client("claude-code").await;
    let result = call(&client, "pwrroute_resolve", json!({ "url": LINK })).await;
    assert_eq!(result.is_error, Some(true));
    assert!(
        text(&result).starts_with("access_denied"),
        "{}",
        text(&result)
    );
    assert!(
        text(&result).contains("reconnect the MCP server"),
        "{}",
        text(&result)
    );
    // Not asked again on this connection.
    call(&client, "pwrroute_browsers", json!({})).await;
    assert_eq!(h.approver.asked().len(), 1);
    assert!(h.agents.agents().is_empty(), "a denial is not remembered");
    assert!(
        h.audit
            .records()
            .iter()
            .all(|r| r.outcome == Outcome::Denied)
    );

    h.approver.set(FakeAnswer::Approve(Role::Reader));
    let again = h.client("claude-code").await;
    let result = call(&again, "pwrroute_resolve", json!({ "url": LINK })).await;
    assert_eq!(result.is_error, Some(false), "{}", text(&result));
    assert_eq!(h.approver.asked().len(), 2);
    // Once the name is approved, the denied connection works too.
    let result = call(&client, "pwrroute_resolve", json!({ "url": LINK })).await;
    assert_eq!(result.is_error, Some(false), "{}", text(&result));
    client.cancel().await.unwrap();
    again.cancel().await.unwrap();
    h.finish().await;
}

#[tokio::test]
async fn the_first_call_waits_for_the_window_and_connections_share_one_prompt() {
    let h = Harness::start(FakeAnswer::Hold).await;
    let first = h.client("claude-code").await;
    let second = h.client("claude-code").await;
    let first_call = tokio::spawn(async move {
        let result = call(&first, "pwrroute_resolve", json!({ "url": LINK })).await;
        (first, result)
    });
    let second_call = tokio::spawn(async move {
        let result = call(&second, "pwrroute_browsers", json!({})).await;
        (second, result)
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!first_call.is_finished(), "the call waits for the person");

    let mut held = h.approver.take_pending();
    assert_eq!(held.len(), 1, "one window for one name");
    let (_, reply) = held.pop().unwrap();
    reply.approve(Role::Reader);

    for task in [first_call, second_call] {
        let (client, result) = task.await.unwrap();
        assert_eq!(result.is_error, Some(false), "{}", text(&result));
        client.cancel().await.unwrap();
    }
    h.finish().await;
}

#[tokio::test]
async fn an_unanswered_window_reports_pending_then_works_once_answered() {
    let h = Harness::start_with(FakeAnswer::Hold, Duration::from_millis(200)).await;
    let client = h.client("claude-code").await;
    let result = call(&client, "pwrroute_resolve", json!({ "url": LINK })).await;
    assert!(
        text(&result).starts_with("approval_pending"),
        "{}",
        text(&result)
    );
    let (_, reply) = h.approver.take_pending().pop().unwrap();
    reply.approve(Role::Reader);
    let agents = h.agents.clone();
    eventually(|| !agents.agents().is_empty()).await;
    let result = call(&client, "pwrroute_resolve", json!({ "url": LINK })).await;
    assert_eq!(result.is_error, Some(false), "{}", text(&result));
    client.cancel().await.unwrap();
    h.finish().await;
}

#[tokio::test]
async fn a_window_closes_when_its_caller_goes_and_is_asked_again_on_return() {
    let h = Harness::start_with(FakeAnswer::Hold, Duration::from_millis(200)).await;
    let client = h.client("claude-code").await;
    let approver = h.approver.clone();
    eventually(|| approver.asked().len() == 1).await;
    // No time limit: a call that gives up waiting leaves the window up.
    let result = call(&client, "pwrroute_resolve", json!({ "url": LINK })).await;
    assert!(
        text(&result).starts_with("approval_pending"),
        "{}",
        text(&result)
    );
    assert!(h.approver.cancelled().is_empty());

    // The caller goes away: nobody is left to answer for, so it closes.
    client.cancel().await.unwrap();
    let id = h.approver.asked()[0].id.clone();
    eventually(|| approver.cancelled() == [id.clone()]).await;
    assert!(h.agents.agents().is_empty(), "a closed window is no answer");

    // A late reply to the closed window changes nothing.
    for (_, reply) in h.approver.take_pending() {
        reply.approve(Role::Editor);
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(h.agents.agents().is_empty());

    // Back again, it is asked again, and the answer holds: a closed window
    // was never a denial.
    h.approver.set(FakeAnswer::Approve(Role::Reader));
    let client = h.client("claude-code").await;
    let result = call(&client, "pwrroute_resolve", json!({ "url": LINK })).await;
    assert_eq!(result.is_error, Some(false), "{}", text(&result));
    assert_eq!(h.approver.asked().len(), 2);
    assert_eq!(h.agents.agents()[0].role, Role::Reader);
    client.cancel().await.unwrap();
    h.finish().await;
}

#[tokio::test]
async fn a_window_closes_when_its_caller_goes_mid_call() {
    // The call is still waiting when the client goes: rmcp cancels it once
    // its drain gives up (5 s), and the window closes with it.
    let h = Harness::start_with(FakeAnswer::Hold, Duration::from_secs(60)).await;
    let client = h.client("claude-code").await;
    let approver = h.approver.clone();
    eventually(|| approver.asked().len() == 1).await;
    let peer = client.peer().clone();
    let waiting = tokio::spawn(async move {
        let _ = peer
            .call_tool(
                rmcp::model::CallToolRequestParams::new("pwrroute_resolve")
                    .with_arguments(json!({ "url": LINK }).as_object().unwrap().clone()),
            )
            .await;
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    client.cancel().await.unwrap();
    let id = h.approver.asked()[0].id.clone();
    let closed = tokio::time::timeout(Duration::from_secs(15), async {
        while approver.cancelled() != [id.clone()] {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(closed.is_ok(), "the window stayed up after its caller left");
    waiting.abort();
    h.finish().await;
}

#[tokio::test]
async fn a_window_stays_while_any_caller_under_the_name_waits() {
    let h = Harness::start_with(FakeAnswer::Hold, Duration::from_millis(200)).await;
    let first = h.client("claude-code").await;
    let second = h.client("claude-code").await;
    let approver = h.approver.clone();
    eventually(|| approver.asked().len() == 1).await;
    // One prompt for the name, shared.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(h.approver.asked().len(), 1);

    first.cancel().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(h.approver.cancelled().is_empty(), "the second still waits");

    // The person answers for the one still here.
    let (_, reply) = h.approver.take_pending().pop().unwrap();
    reply.approve(Role::Opener);
    let result = call(&second, "pwrroute_resolve", json!({ "url": LINK })).await;
    assert_eq!(result.is_error, Some(false), "{}", text(&result));
    assert_eq!(h.agents.agents()[0].role, Role::Opener);
    assert!(h.approver.cancelled().is_empty());
    second.cancel().await.unwrap();
    h.finish().await;
}

#[tokio::test]
async fn past_the_prompt_cap_a_client_waits_rather_than_counting_as_denied() {
    let h = Harness::start_with(FakeAnswer::Hold, Duration::from_millis(200)).await;
    let mut clients = Vec::new();
    for n in 0..8 {
        clients.push(h.client(&format!("agent-{n}")).await);
    }
    let approver = h.approver.clone();
    eventually(|| approver.asked().len() == 8).await;
    let ninth = h.client("agent-8").await;
    let result = call(&ninth, "pwrroute_resolve", json!({ "url": LINK })).await;
    assert!(
        text(&result).starts_with("approval_pending") && text(&result).contains("too many"),
        "{}",
        text(&result)
    );
    assert_eq!(h.approver.asked().len(), 8, "the ninth is not asked");

    // A window closes; the ninth is asked on its next call.
    let (_, reply) = h.approver.take_pending().remove(0);
    reply.deny();
    let mut asked = false;
    for _ in 0..50 {
        call(&ninth, "pwrroute_browsers", json!({})).await;
        if h.approver.asked().len() == 9 {
            asked = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(asked, "the ninth was never asked");
    clients.push(ninth);
    for client in clients {
        client.cancel().await.unwrap();
    }
    h.finish().await;
}

#[tokio::test]
async fn settings_can_change_revoke_allow_and_forget() {
    let h = Harness::start(FakeAnswer::Approve(Role::Opener)).await;
    let client = h.client("claude-code").await;
    let open = || call(&client, "pwrroute_open", json!({ "url": LINK }));
    assert_eq!(open().await.is_error, Some(false));

    // A role change applies to the next call on the same connection.
    h.agents.set_role("claude-code", Role::Reader).unwrap();
    let result = open().await;
    assert!(
        text(&result).starts_with("missing_permission") && text(&result).contains("links.open"),
        "{}",
        text(&result)
    );

    // Revoked: refused, and not asked about again.
    h.agents.revoke("claude-code").unwrap();
    let result = call(&client, "pwrroute_resolve", json!({ "url": LINK })).await;
    assert!(text(&result).starts_with("revoked"), "{}", text(&result));
    assert_eq!(h.approver.asked().len(), 1);

    h.agents.allow("claude-code", Role::Opener).unwrap();
    assert_eq!(open().await.is_error, Some(false));

    // Forgotten: the next call asks again.
    h.agents.forget("claude-code").unwrap();
    assert_eq!(open().await.is_error, Some(false));
    assert_eq!(h.approver.asked().len(), 2);
    client.cancel().await.unwrap();
    h.finish().await;
}

#[tokio::test]
async fn the_window_may_grant_any_role() {
    let h = Harness::start(FakeAnswer::Decide(ApprovalDecision::Approve {
        role: Role::Editor,
    }))
    .await;
    let client = h.client("claude-code").await;
    let result = call(&client, "pwrroute_rules_list", json!({})).await;
    assert_eq!(result.is_error, Some(false));
    assert_eq!(h.agents.agents()[0].role, Role::Editor);
    client.cancel().await.unwrap();
    h.finish().await;
}

#[tokio::test]
async fn approvals_survive_a_restart() {
    let h = Harness::start(FakeAnswer::Approve(Role::Opener)).await;
    let client = h.client("claude-code").await;
    call(&client, "pwrroute_browsers", json!({})).await;
    client.cancel().await.unwrap();
    let dir = h.stop().await;

    let h = Harness::start_in(
        dir,
        Arc::new(FakeApprover::new(FakeAnswer::Deny)),
        config(private_endpoint(), Duration::from_secs(10)),
    )
    .await;
    let client = h.client("claude-code").await;
    let result = call(&client, "pwrroute_open", json!({ "url": LINK })).await;
    assert_eq!(result.is_error, Some(false), "{}", text(&result));
    assert!(h.approver.asked().is_empty());
    client.cancel().await.unwrap();
    h.finish().await;
}

#[tokio::test]
async fn stopping_closes_open_windows() {
    let h = Harness::start(FakeAnswer::Hold).await;
    let client = h.client("claude-code").await;
    let approver = h.approver.clone();
    eventually(|| approver.asked().len() == 1).await;
    let id = approver.asked()[0].id.clone();
    let (_, reply) = approver.take_pending().pop().unwrap();
    h.finish().await;
    eventually(|| approver.cancelled() == [id.clone()]).await;
    eventually(|| !reply.is_pending()).await;
    drop(client);
}
