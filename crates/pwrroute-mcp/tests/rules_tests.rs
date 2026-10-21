//! The route-test tools over the link: `pwrroute_rules_test`,
//! `pwrroute_suggest_rule`, `pwrroute_rules_apply`'s refusal of a change
//! that fails a saved example or test, and what `pwrroute_activity` says at
//! each privacy level.

mod support;

use pwrroute_mcp::{Outcome, Role, fake::FakeAnswer};
use serde_json::{Value, json};
use support::{Harness, call, text};

/// The saved example of the fake rules' one handler.
const EXAMPLE: &str = "https://docs.example.org/page";
/// A link no rule routes: the default takes it.
const UNROUTED: &str = "https://app.metrics.example/dashboard/abc/latency?from=1";

#[tokio::test]
async fn rules_test_runs_the_saved_file_or_a_proposal_without_writing() {
    let h = Harness::start(FakeAnswer::Approve(Role::Reader)).await;
    let client = h.client("test-agent").await;

    // As saved: one rule example, one rewrite example, one test.
    let saved = call(&client, "pwrroute_rules_test", json!({})).await;
    assert_eq!(saved.is_error, Some(false), "{}", text(&saved));
    let saved = saved.structured_content.unwrap();
    assert_eq!(saved["proposal"], false);
    assert_eq!(saved["report"]["total"], 3, "{saved}");
    assert_eq!(saved["report"]["passed"], 3);
    assert_eq!(saved["report"]["failed"], 0);

    // One extra handler, too wide and at the top, with the links it is for:
    // the saved example now lands on it, and each link comes back with the
    // patterns that would catch it.
    let proposed = call(
        &client,
        "pwrroute_rules_test",
        json!({
            "rule": { "match": "*.example.org/*", "browser": "Safari" },
            "at": 0,
            "urls": ["https://wiki.example.org/12345/x"]
        }),
    )
    .await;
    assert_eq!(proposed.is_error, Some(false), "{}", text(&proposed));
    let proposed = proposed.structured_content.unwrap();
    assert_eq!(proposed["proposal"], true);
    assert_eq!(
        proposed["report"]["total"], 4,
        "the extra link is an example"
    );
    assert_eq!(proposed["report"]["failed"], 1, "{proposed}");
    let failure = &proposed["report"]["failures"][0];
    assert_eq!(failure["input"], EXAMPLE);
    assert_eq!(failure["source"]["kind"], "ruleExample");
    assert_eq!(failure["source"]["rule"], 1, "numbered after the insertion");
    let url = &proposed["urls"][0];
    assert_eq!(url["url"], "https://wiki.example.org/12345/x");
    assert_eq!(url["needsJs"], true, "an id in the path");
    assert_eq!(url["candidates"][0], "wiki.example.org/12345/*");
    assert!(url["why"].as_array().unwrap().len() >= 3);
    assert_eq!(h.rules.read_revision(), "1", "nothing was written");

    // A whole proposed file replaces the saved one for the run.
    let whole = call(
        &client,
        "pwrroute_rules_test",
        json!({ "rules": { "defaultBrowser": "Firefox", "rules": [] } }),
    )
    .await;
    let whole = whole.structured_content.unwrap();
    assert_eq!(whole["report"]["total"], 0, "{whole}");

    // Links without a rule to be examples of are an input error.
    let alone = call(
        &client,
        "pwrroute_rules_test",
        json!({ "urls": ["https://a.example/"] }),
    )
    .await;
    assert_eq!(alone.is_error, Some(true));
    assert!(text(&alone).contains("invalid_input"), "{}", text(&alone));
    client.cancel().await.unwrap();
    h.finish().await;
}

#[tokio::test]
async fn suggest_rule_names_patterns_and_what_else_they_catch() {
    let h = Harness::start(FakeAnswer::Approve(Role::Opener)).await;
    // Links a person clicked, kept in full in memory.
    h.router.record(UNROUTED, "Slack").unwrap();
    h.router
        .record("https://app.metrics.example/dashboard/9f2e/x", "Slack")
        .unwrap();
    h.router
        .record("https://app.metrics.example/settings", "Mail")
        .unwrap();
    // One kept as host only: it cannot be matched, so it is not offered.
    h.router
        .record("https://bank.example.org/account?token=1", "Mail")
        .unwrap();
    let client = h.client("test-agent").await;

    let result = call(&client, "pwrroute_suggest_rule", json!({ "url": UNROUTED })).await;
    assert_eq!(result.is_error, Some(false), "{}", text(&result));
    let rendered = text(&result);
    let body = result.structured_content.unwrap();
    assert_eq!(
        body["candidates"],
        json!(["app.metrics.example/dashboard/*", "app.metrics.example/*"])
    );
    assert_eq!(body["needsJs"], false);
    assert_eq!(body["fullUrls"], true);
    let collisions = body["collisions"].as_array().unwrap();
    assert_eq!(collisions[0]["pattern"], "app.metrics.example/dashboard/*");
    assert_eq!(collisions[0]["count"], 1);
    assert_eq!(
        collisions[0]["urls"],
        json!(["https://app.metrics.example/dashboard/9f2e/x"])
    );
    assert_eq!(collisions[1]["count"], 2, "{body}");
    assert!(
        !rendered.contains("token=1"),
        "a host-only record stays out"
    );

    let not_a_url = call(&client, "pwrroute_suggest_rule", json!({ "url": "x/y" })).await;
    let not_a_url = not_a_url.structured_content.unwrap();
    assert_eq!(not_a_url["candidates"], json!([]));
    client.cancel().await.unwrap();
    h.finish().await;
}

#[tokio::test]
async fn apply_refuses_a_change_that_fails_a_saved_test_unless_forced() {
    let h = Harness::start(FakeAnswer::Approve(Role::Editor)).await;
    let client = h.client("test-agent").await;
    let apply = |args: Value| call(&client, "pwrroute_rules_apply", args);
    let listed = call(&client, "pwrroute_rules_list", json!({})).await;
    let listed = listed.structured_content.unwrap();
    let mut rules = listed["rules"]["rules"].as_array().cloned().unwrap();
    // A wider handler ahead of the saved one takes its example.
    rules.insert(
        0,
        json!({ "match": "*.example.org/*", "browser": "Safari" }),
    );

    let refused = apply(json!({ "baseRevision": "1", "patch": { "rules": rules } })).await;
    assert_eq!(refused.is_error, Some(true));
    let message = text(&refused);
    assert!(
        message.starts_with("regressions: 1 saved example or test"),
        "{message}"
    );
    assert!(message.contains("Nothing was written"), "{message}");
    // The body carries the failures so the agent can fix the patch.
    let body: Value = serde_json::from_str(message.split_once('\n').unwrap().1).unwrap();
    assert_eq!(body["regressions"][0]["input"], EXAMPLE);
    assert_eq!(body["report"]["failed"], 1);
    assert_eq!(body["revision"], "1");
    assert_eq!(h.rules.read_revision(), "1", "nothing was written");

    let forced = apply(json!({
        "baseRevision": "1",
        "patch": { "rules": rules },
        "force": true
    }))
    .await;
    assert_eq!(forced.is_error, Some(false), "{}", text(&forced));
    let forced = forced.structured_content.unwrap();
    assert_eq!(forced["written"], true);
    assert_eq!(forced["forced"], true);
    assert_eq!(forced["revision"], "2");
    assert_eq!(forced["regressions"].as_array().unwrap().len(), 1);

    // The audit says what happened, in fixed words, and never a URL.
    let records = h.audit.records();
    let applies: Vec<_> = records
        .iter()
        .filter(|r| r.tool == "pwrroute_rules_apply")
        .collect();
    assert_eq!(applies.len(), 2);
    assert_eq!(applies[0].outcome, Outcome::Failed);
    assert_eq!(
        applies[0].note,
        Some("refused: the change fails saved examples or tests")
    );
    assert_eq!(applies[1].outcome, Outcome::Ok);
    assert_eq!(
        applies[1].note,
        Some("forced: written although saved examples or tests fail")
    );
    let serialized = serde_json::to_string(&records).unwrap();
    assert!(!serialized.contains("example.org"), "{serialized}");
    client.cancel().await.unwrap();
    h.finish().await;
}

#[tokio::test]
async fn activity_keeps_each_record_at_its_level_and_says_where_it_is_kept() {
    let h = Harness::start(FakeAnswer::Approve(Role::Editor)).await;
    h.router.record(UNROUTED, "Slack").unwrap();
    h.router
        .record("https://bank.example.org/account?token=hunter2", "Mail")
        .unwrap();
    h.router
        .record("https://secret.example.org/private/x", "Mail")
        .unwrap();
    h.router.record("https://x.com/someone", "Slack").unwrap();
    let client = h.client("test-agent").await;

    let result = call(&client, "pwrroute_activity", json!({})).await;
    assert_eq!(result.is_error, Some(false), "{}", text(&result));
    let body = result.structured_content.unwrap();
    assert_eq!(body["fullUrls"], true);
    assert_eq!(body["source"], "memory");
    assert_eq!(body["filter"], "all");
    assert_eq!(body["scanned"], 4);
    assert!(
        body["retention"]
            .as_str()
            .unwrap()
            .starts_with("Kept in memory only"),
        "{body}"
    );
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 4);

    let rewritten = &entries[0];
    assert_eq!(rewritten["privacy"], "full");
    assert_eq!(rewritten["rewritten"], true);
    assert_eq!(
        rewritten["trace"]["browser"]["url"],
        "https://xcancel.com/someone"
    );

    let none = &entries[1];
    assert_eq!(none["privacy"], "none");
    assert_eq!(none["source"], "memory");
    assert!(none.get("trace").is_none(), "{none}");
    assert!(none.get("via").is_none(), "{none}");
    let none_text = none.to_string();
    assert!(
        !none_text.contains("secret") && !none_text.contains("Mail"),
        "{none_text}"
    );

    let host = &entries[2];
    assert_eq!(host["privacy"], "host");
    assert_eq!(host["private"], true);
    assert_eq!(host["trace"]["url"], "bank.example.org");
    let host_text = host.to_string();
    assert!(
        !host_text.contains("/account") && !host_text.contains("hunter2"),
        "{host_text}"
    );

    let unrouted = &entries[3];
    assert_eq!(unrouted["unrouted"], true);
    assert_eq!(unrouted["via"], "Slack");
    assert_eq!(unrouted["trace"]["url"], UNROUTED);

    // The filters, by what the trace says: the rewritten x.com link still
    // went to the default, so it is unrouted too; the limit cuts the newest.
    let only = call(
        &client,
        "pwrroute_activity",
        json!({ "filter": "unrouted" }),
    )
    .await;
    let only = only.structured_content.unwrap();
    assert_eq!(only["entries"].as_array().unwrap().len(), 2, "{only}");
    assert_eq!(only["entries"][1]["trace"]["url"], UNROUTED);
    let one = call(
        &client,
        "pwrroute_activity",
        json!({ "filter": "unrouted", "limit": 1 }),
    )
    .await;
    let one = one.structured_content.unwrap();
    assert_eq!(one["entries"].as_array().unwrap().len(), 1);
    assert_eq!(one["scanned"], 4, "the limit applies after the filter");
    let private = call(&client, "pwrroute_activity", json!({ "filter": "private" })).await;
    let private = private.structured_content.unwrap();
    assert_eq!(private["entries"].as_array().unwrap().len(), 1);
    assert_eq!(private["entries"][0]["privacy"], "host");
    client.cancel().await.unwrap();
    h.finish().await;
}
