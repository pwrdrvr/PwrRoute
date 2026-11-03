//! Suggestion batching through the real MCP protocol, with contrived links.
mod support;

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use pwrroute_engine::{Engine, Limits, RulesFile, TestReport};
use pwrroute_mcp::{EngineTester, EngineThread, Loader, Role, RulesTester, fake::FakeApprover};
use serde_json::{Value, json};
use support::{Harness, call, text};

type Batch = (Vec<String>, Vec<String>);

struct CountingTester {
    inner: Arc<dyn RulesTester>,
    singles: AtomicUsize,
    batches: Mutex<Vec<Batch>>,
    reloads: Arc<AtomicUsize>,
}

impl RulesTester for CountingTester {
    fn test(&self, proposal: Option<&RulesFile>) -> anyhow::Result<TestReport> {
        self.inner.test(proposal)
    }

    fn collisions(&self, pattern: &str, urls: &[String]) -> anyhow::Result<Vec<String>> {
        self.singles.fetch_add(1, Ordering::SeqCst);
        self.inner.collisions(pattern, urls)
    }

    fn collisions_batch(
        &self,
        patterns: &[String],
        urls: &[String],
    ) -> anyhow::Result<Vec<Vec<String>>> {
        self.batches
            .lock()
            .unwrap()
            .push((patterns.to_vec(), urls.to_vec()));
        self.inner.collisions_batch(patterns, urls)
    }
}

struct UnavailableLoader(Arc<AtomicUsize>);

impl Loader for UnavailableLoader {
    fn engine(&mut self) -> anyhow::Result<&Engine> {
        self.0.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("matching the suggestion batch: fixture engine unavailable");
    }
}

struct UnavailableTester(EngineThread);

impl RulesTester for UnavailableTester {
    fn test(&self, _: Option<&RulesFile>) -> anyhow::Result<TestReport> {
        unreachable!()
    }

    fn collisions(&self, _: &str, _: &[String]) -> anyhow::Result<Vec<String>> {
        unreachable!("suggestions must use the batch backend")
    }

    fn collisions_batch(&self, _: &[String], _: &[String]) -> anyhow::Result<Vec<Vec<String>>> {
        self.0.run(|_| Ok(Vec::new()))
    }
}

async fn harness(fail: bool) -> (Harness, Arc<CountingTester>) {
    let endpoint = support::private_endpoint();
    let state = std::env::temp_dir().join(format!("pwrroute-batch-{}-{fail}", std::process::id()));
    let mut tester = None;
    let h = Harness::start_in_with(
        state,
        Arc::new(FakeApprover::approving(Role::Opener)),
        support::config(endpoint, Duration::from_secs(10)),
        |b| {
            let reloads = Arc::new(AtomicUsize::new(0));
            let observed = reloads.clone();
            let inner: Arc<dyn RulesTester> = if fail {
                Arc::new(UnavailableTester(
                    EngineThread::spawn("unavailable", move || Ok(UnavailableLoader(observed)))
                        .unwrap(),
                ))
            } else {
                Arc::new(
                    EngineTester::over(
                        move || {
                            observed.fetch_add(1, Ordering::SeqCst);
                            ("1".to_owned(), RulesFile::default())
                        },
                        Limits::default(),
                    )
                    .unwrap(),
                )
            };
            let counting = Arc::new(CountingTester {
                inner,
                singles: AtomicUsize::new(0),
                batches: Mutex::default(),
                reloads,
            });
            b.tester = counting.clone();
            tester = Some(counting);
        },
    )
    .await;
    (h, tester.unwrap())
}

#[tokio::test]
async fn suggestion_uses_one_batch_with_ordered_overlaps_and_private_counts() {
    let (h, tester) = harness(false).await;
    let target = "https://app.metrics.example/dashboard/abc/latency?from=1";
    let client = h.client("opener").await;
    // Even with no activity, valid candidates retain zero collision counts.
    let empty = call(&client, "pwrroute_suggest_rule", json!({"url": target})).await;
    assert_eq!(empty.is_error, Some(false), "{}", text(&empty));
    for c in empty.structured_content.unwrap()["candidates"]
        .as_array()
        .unwrap()
    {
        assert_eq!(c["alsoCatches"], json!({"count": 0, "urls": []}));
    }
    assert_eq!(
        tester.singles.load(Ordering::SeqCst),
        0,
        "suggestions must use the batch backend"
    );
    assert_eq!(tester.batches.lock().unwrap().len(), 1);

    let links = [
        target,
        "https://app.metrics.example/dashboard/other",
        "https://app.metrics.example/dashboard/other", // repeated dispatch
        "https://app.metrics.example/settings",
        "https://metrics.example/status",
        "https://bank.example.org/account?token=private",
        "https://secret.example.org/private",
    ];
    for link in links {
        h.router.record(link, "Fixture").unwrap();
    }
    for role in [Role::Opener, Role::Reader] {
        h.agents.set_role("opener", role).unwrap();
        let result = call(&client, "pwrroute_suggest_rule", json!({"url": target})).await;
        assert_eq!(result.is_error, Some(false), "{}", text(&result));
        let rendered: Value = serde_json::from_str(&text(&result)).unwrap();
        assert_eq!(result.structured_content.as_ref(), Some(&rendered));
        assert_eq!(rendered["fullUrls"], role == Role::Opener);
        let candidates = rendered["candidates"].as_array().unwrap();
        assert_eq!(
            candidates
                .iter()
                .map(|c| c["match"].clone())
                .collect::<Vec<_>>(),
            vec![
                json!(["app.metrics.example/dashboard/*"]),
                json!(["app.metrics.example/*"]),
                json!(["metrics.example/*", "*.metrics.example/*"]),
            ]
        );
        assert_eq!(
            candidates
                .iter()
                .map(|c| c["alsoCatches"]["count"].clone())
                .collect::<Vec<_>>(),
            vec![json!(1), json!(2), json!(3)]
        );
        assert_eq!(
            candidates[1]["alsoCatches"]["urls"],
            if role == Role::Reader {
                json!(["app.metrics.example"])
            } else {
                json!([
                    "https://app.metrics.example/dashboard/other",
                    "https://app.metrics.example/settings"
                ])
            }
        );
        assert!(!text(&result).contains("token=private"));
        if role == Role::Reader {
            assert!(!text(&result).contains("/dashboard/other"));
        }
    }
    let batches = tester.batches.lock().unwrap().clone();
    assert_eq!(batches.len(), 3, "one batch per suggestion");
    assert_eq!(
        batches[1].0,
        vec![
            "app.metrics.example/dashboard/*",
            "app.metrics.example/*",
            "metrics.example/*",
            "*.metrics.example/*"
        ]
    );
    assert_eq!(
        batches[1].1,
        vec![
            "https://app.metrics.example/dashboard/other",
            "https://app.metrics.example/settings",
            "https://metrics.example/status"
        ]
    );
    assert_eq!(tester.singles.load(Ordering::SeqCst), 0);
    let invalid = call(&client, "pwrroute_suggest_rule", json!({"url": "x/y"})).await;
    assert_eq!(invalid.structured_content.unwrap()["candidates"], json!([]));
    assert_eq!(
        tester.batches.lock().unwrap().len(),
        3,
        "no pattern means no engine work"
    );
    assert_eq!(
        tester.reloads.load(Ordering::SeqCst),
        4,
        "initial load plus one coherent engine snapshot per suggestion"
    );
    client.cancel().await.unwrap();
    h.finish().await;
}

#[tokio::test]
async fn suggestion_reports_batch_errors_as_protocol_errors() {
    let (h, tester) = harness(true).await;
    let client = h.client("opener").await;
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        call(
            &client,
            "pwrroute_suggest_rule",
            json!({"url": "https://example.org/page"}),
        ),
    )
    .await
    .expect("a loader failure must produce a protocol reply");
    assert_eq!(result.is_error, Some(true), "{}", text(&result));
    assert!(text(&result).contains("matching the suggestion batch: fixture engine unavailable"));
    assert!(result.structured_content.is_none());
    assert_eq!(tester.batches.lock().unwrap().len(), 1);
    assert_eq!(tester.singles.load(Ordering::SeqCst), 0);
    assert_eq!(tester.reloads.load(Ordering::SeqCst), 1);
    client.cancel().await.unwrap();
    h.finish().await;
}

#[test]
fn legacy_tester_batch_reuses_duplicate_patterns_and_preserves_positions() {
    struct Legacy(AtomicUsize);
    impl RulesTester for Legacy {
        fn test(&self, _: Option<&RulesFile>) -> anyhow::Result<TestReport> {
            unreachable!()
        }
        fn collisions(&self, pattern: &str, _: &[String]) -> anyhow::Result<Vec<String>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            if pattern == "error" {
                anyhow::bail!("fixture matcher failed");
            }
            Ok(vec![pattern.to_owned()])
        }
    }
    let tester = Legacy(AtomicUsize::new(0));
    let patterns = ["narrow", "wide", "narrow"].map(str::to_owned);
    assert_eq!(
        tester.collisions_batch(&patterns, &[]).unwrap(),
        vec![vec!["narrow"], vec!["wide"], vec!["narrow"]]
    );
    assert_eq!(
        tester.0.load(Ordering::SeqCst),
        2,
        "evaluate each distinct pattern once"
    );
    assert!(
        tester
            .collisions_batch(&["error".to_owned()], &[])
            .unwrap_err()
            .to_string()
            .contains("fixture matcher failed")
    );
}
