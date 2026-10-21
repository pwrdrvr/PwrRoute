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
        ("pwrroute_rules_test", json!({})),
        ("pwrroute_suggest_rule", json!({ "url": LINK })),
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
    // The journal is the Editor's: a Reader is refused by name.
    denied(
        &call(&client, "pwrroute_activity", json!({ "source": "journal" })).await,
        "activity.journal",
    );
    assert!(h.router.opened().is_empty(), "nothing was opened");
    assert_eq!(h.rules.read_revision(), "1", "nothing was written");

    let outcomes: Vec<Outcome> = h.audit.records().iter().map(|r| r.outcome).collect();
    assert_eq!(
        &outcomes[6..],
        [Outcome::Denied, Outcome::Denied, Outcome::Denied]
    );
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
    // A Reader's suggestion names what a pattern would catch by host only:
    // the link it asked about is its own, the others are not.
    let suggested = call(
        &reader,
        "pwrroute_suggest_rule",
        json!({ "url": "https://docs.example.org/other" }),
    )
    .await;
    assert!(!text(&suggested).contains("/page"), "{}", text(&suggested));
    let suggested = suggested.structured_content.unwrap();
    assert_eq!(suggested["fullUrls"], false);
    assert_eq!(suggested["collisions"][1]["count"], 1, "{suggested}");
    assert_eq!(
        suggested["collisions"][1]["urls"],
        json!(["docs.example.org"])
    );
    opener.cancel().await.unwrap();
    reader.cancel().await.unwrap();
    h.finish().await;
}

#[tokio::test]
async fn only_an_editor_reads_the_journal() {
    let h = Harness::start(FakeAnswer::Approve(Role::Opener)).await;
    let opener = h.client("opener-agent").await;
    denied(
        &call(&opener, "pwrroute_activity", json!({ "source": "journal" })).await,
        "activity.journal",
    );
    h.approver.set(FakeAnswer::Approve(Role::Editor));
    let editor = h.client("editor-agent").await;
    let result = call(&editor, "pwrroute_activity", json!({ "source": "journal" })).await;
    assert_eq!(result.is_error, Some(false), "{}", text(&result));
    let body = result.structured_content.unwrap();
    assert_eq!(body["source"], "journal");
    assert_eq!(body["entries"], json!([]), "the fake journal is off");
    assert!(body["retention"].as_str().unwrap().contains("memory"));
    opener.cancel().await.unwrap();
    editor.cancel().await.unwrap();
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
    assert_eq!(listed["rules"]["rules"][0]["match"], "*.example.org/*");
    assert_eq!(
        listed["rules"]["rules"][0]["examples"][0],
        "https://docs.example.org/page"
    );

    let apply = |args: Value| call(&client, "pwrroute_rules_apply", args);
    // The saved test says example.net goes to Safari, the default: a dry
    // run of a new default reports the regression and writes nothing.
    let dry = apply(json!({
        "baseRevision": "1",
        "patch": { "defaultBrowser": "Firefox" },
        "dryRun": true
    }))
    .await;
    assert_eq!(dry.is_error, Some(false), "{}", text(&dry));
    let dry = dry.structured_content.unwrap();
    assert_eq!(dry["written"], false);
    assert_eq!(dry["regressions"].as_array().unwrap().len(), 1, "{dry}");
    assert_eq!(h.rules.read_revision(), "1", "a dry run writes nothing");

    // A change that keeps every saved example and test is written.
    let written = apply(json!({
        "baseRevision": "1",
        "patch": { "defaultBrowser": "Safari", "defaultProfile": "Work" }
    }))
    .await;
    assert_eq!(written.is_error, Some(false), "{}", text(&written));
    let written = written.structured_content.unwrap();
    assert_eq!(written["revision"], "2");
    assert_eq!(written["rules"]["defaultProfile"], "Work");
    assert_eq!(
        written["rules"]["rewrites"][0]["actions"][0]["setHost"], "xcancel.com",
        "the rest is kept"
    );
    assert_eq!(written["forced"], false);
    assert_eq!(written["report"]["failed"], 0);

    let stale =
        apply(json!({ "baseRevision": "1", "patch": { "defaultBrowser": "Safari" } })).await;
    assert_eq!(stale.is_error, Some(true));
    assert!(text(&stale).starts_with("conflict"), "{}", text(&stale));

    let invalid = apply(json!({ "baseRevision": "2", "patch": { "rules": "none" } })).await;
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
