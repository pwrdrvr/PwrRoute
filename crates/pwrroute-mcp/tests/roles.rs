//! The Reader / Opener / Editor matrix, enforced on real tool calls.

mod support;

use pwrroute_mcp::{Outcome, Role, RulesSource, fake::FakeAnswer};
use serde_json::{Value, json};
use support::{Harness, call, text};

const LINK: &str = "https://docs.example.org/page?q=1";

fn denied(result: &rmcp::model::CallToolResult, permission: &str) {
    assert_eq!(result.is_error, Some(true));
    let message = text(result);
    assert!(
        message.starts_with("missing_permission") && message.contains(permission),
        "{message}"
    );
}

#[tokio::test]
async fn a_reader_can_look_but_not_open_or_edit() {
    let h = Harness::start(FakeAnswer::Approve(Role::Reader)).await;
    let client = h.client("test-agent").await;

    for (tool, args) in [
        ("pwrroute_resolve", json!({ "url": LINK })),
        ("pwrroute_browsers", json!({})),
        ("pwrroute_rules_list", json!({})),
        ("pwrroute_activity", json!({})),
    ] {
        let result = call(&client, tool, args).await;
        assert_eq!(result.is_error, Some(false), "{tool}: {}", text(&result));
    }
    denied(
        &call(&client, "pwrroute_open", json!({ "url": LINK })).await,
        "links.open",
    );
    denied(
        &call(
            &client,
            "pwrroute_rules_apply",
            json!({ "baseRevision": "1", "patch": { "defaultBrowser": "Firefox" } }),
        )
        .await,
        "rules.write",
    );
    assert!(h.router.opened().is_empty(), "nothing was opened");
    assert_eq!(h.rules.read_revision(), "1", "nothing was written");

    let outcomes: Vec<Outcome> = h.audit.records().iter().map(|r| r.outcome).collect();
    assert_eq!(&outcomes[4..], [Outcome::Denied, Outcome::Denied]);
    client.cancel().await.unwrap();
    h.finish().await;
}

#[tokio::test]
async fn a_reader_sees_activity_as_hosts_only() {
    let h = Harness::start(FakeAnswer::Approve(Role::Opener)).await;
    let opener = h.client("opener-agent").await;
    let result = call(&opener, "pwrroute_open", json!({ "url": LINK })).await;
    assert_eq!(result.is_error, Some(false), "{}", text(&result));
    let full = call(&opener, "pwrroute_activity", json!({})).await;
    let full = full.structured_content.unwrap();
    assert_eq!(full["fullUrls"], true);
    assert_eq!(full["entries"][0]["trace"]["url"], LINK);

    h.approver.set(FakeAnswer::Approve(Role::Reader));
    let reader = h.client("reader-agent").await;
    let hosts = call(&reader, "pwrroute_activity", json!({})).await;
    assert!(!text(&hosts).contains("/page"), "{}", text(&hosts));
    let hosts = hosts.structured_content.unwrap();
    assert_eq!(hosts["fullUrls"], false);
    assert_eq!(hosts["entries"][0]["trace"]["url"], "docs.example.org");
    assert_eq!(
        hosts["entries"][0]["trace"]["browser"]["url"],
        "docs.example.org"
    );
    opener.cancel().await.unwrap();
    reader.cancel().await.unwrap();
    h.finish().await;
}

#[tokio::test]
async fn an_opener_opens_through_the_router_against_its_agent() {
    let h = Harness::start(FakeAnswer::Approve(Role::Opener)).await;
    let client = h.client("test-agent").await;

    let result = call(
        &client,
        "pwrroute_open",
        json!({ "url": LINK, "opener": { "name": "Slack", "bundleId": "com.tinyspeck.slackmacgap" } }),
    )
    .await;
    assert_eq!(result.is_error, Some(false), "{}", text(&result));
    assert_eq!(
        result.structured_content.unwrap()["browser"]["name"],
        "Firefox"
    );
    let opened = h.router.opened();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].0.name, "test-agent");
    assert_eq!(opened[0].1.via.as_deref(), Some("test-agent"));

    denied(
        &call(
            &client,
            "pwrroute_rules_apply",
            json!({ "baseRevision": "1", "patch": {} }),
        )
        .await,
        "rules.write",
    );
    client.cancel().await.unwrap();
    h.finish().await;
}

#[tokio::test]
async fn an_editor_changes_rules_by_revision() {
    let h = Harness::start(FakeAnswer::Approve(Role::Editor)).await;
    let client = h.client("test-agent").await;

    let listed = call(&client, "pwrroute_rules_list", json!({})).await;
    let listed = listed.structured_content.unwrap();
    assert_eq!(listed["revision"], "1");
    assert_eq!(listed["rules"]["handlers"][1]["opaque"], true);

    let apply = |args: Value| call(&client, "pwrroute_rules_apply", args);
    let dry = apply(json!({
        "baseRevision": "1",
        "patch": { "defaultBrowser": "Firefox" },
        "dryRun": true
    }))
    .await;
    assert_eq!(dry.is_error, Some(false), "{}", text(&dry));
    assert_eq!(h.rules.read_revision(), "1", "a dry run writes nothing");

    let written =
        apply(json!({ "baseRevision": "1", "patch": { "defaultBrowser": "Firefox" } })).await;
    let written = written.structured_content.unwrap();
    assert_eq!(written["revision"], "2");
    assert_eq!(written["rules"]["defaultBrowser"], "Firefox");
    assert_eq!(
        written["rules"]["rewrite"][0]["host"], "xcancel.com",
        "the rest is kept"
    );

    let stale =
        apply(json!({ "baseRevision": "1", "patch": { "defaultBrowser": "Safari" } })).await;
    assert_eq!(stale.is_error, Some(true));
    assert!(text(&stale).starts_with("conflict"), "{}", text(&stale));

    let invalid = apply(json!({ "baseRevision": "2", "patch": { "handlers": null } })).await;
    assert!(
        text(&invalid).starts_with("invalid_rules"),
        "{}",
        text(&invalid)
    );

    h.rules.select(RulesSource::FinickyConfig);
    let refused =
        apply(json!({ "baseRevision": "2", "patch": { "defaultBrowser": "Safari" } })).await;
    assert!(
        text(&refused).starts_with("finicky_config_selected"),
        "{}",
        text(&refused)
    );
    client.cancel().await.unwrap();
    h.finish().await;
}
