use super::*;
use proptest::prelude::*;
use std::sync::Mutex;

#[derive(Clone, Debug)]
struct Script {
    full: usize,
    short: usize,
    anchor: Option<usize>,
    kinds: Vec<u8>,
    latencies: Vec<u64>,
}

fn script(max_latency: u64) -> impl Strategy<Value = Script> {
    (
        0usize..12,
        0usize..30,
        prop::option::of(1usize..13),
        prop::collection::vec(0u8..16, 13),
        prop::collection::vec(0u64..=max_latency, 13),
    )
        .prop_map(|(full, short, anchor, kinds, latencies)| Script {
            full,
            short,
            anchor,
            kinds,
            latencies,
        })
}

impl Script {
    fn response(&self, request: Request) -> Result<Option<String>, String> {
        let index = match request {
            Request::Latest => 0,
            Request::Page(p) => p,
        };
        match self.kinds[index] {
            0 => return Ok(None),
            1 => return Err("rate limited".into()),
            2 => return Err("unreachable".into()),
            _ => {}
        }
        match request {
            Request::Latest => Ok(Some(r#"{"tag_name":"anchor"}"#.into())),
            Request::Page(p) => {
                let count = if p <= self.full {
                    30
                } else if p == self.full + 1 {
                    self.short
                } else {
                    0
                };
                let entries: Vec<_> = (0..count).map(|i| serde_json::json!({
                    "tag_name": if self.anchor == Some(p) && i == 0 { "anchor".into() } else { format!("v1.{p}.{i}") },
                    "draft": i % 7 == 6, "prerelease": i % 2 == 0,
                    "assets": [{"name": if (p + i) % 3 == 0 { "releases.win.json" } else { "releases.osx.json" }}]
                })).collect();
                Ok(Some(serde_json::to_string(&entries).unwrap()))
            }
        }
    }
}

// Straight serial oracle: request the anchor, walk pages in order, filter
// drafts/feed membership, stop on a short page or the anchor. It intentionally
// does not call parse_page/latest_tag or fetch_with.
fn serial(s: &Script) -> Result<Vec<Release>, String> {
    let anchor = s.response(Request::Latest)?.map(|body| {
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["tag_name"]
            .as_str()
            .unwrap()
            .to_owned()
    });
    let mut result = Vec::new();
    for p in 1..=10 {
        let body = s
            .response(Request::Page(p))?
            .ok_or_else(|| "GitHub has no release list for PwrRoute".to_owned())?;
        let rows: Vec<serde_json::Value> = serde_json::from_str(&body).unwrap();
        for row in &rows {
            if row["draft"] == false
                && row["assets"].as_array().unwrap().iter().any(|a| {
                    a["name"]
                        .as_str()
                        .unwrap()
                        .eq_ignore_ascii_case("releases.osx.json")
                })
            {
                result.push(Release::new(
                    row["tag_name"].as_str().unwrap(),
                    row["prerelease"].as_bool().unwrap(),
                ));
            }
        }
        if rows.len() < 30
            || anchor
                .as_ref()
                .is_none_or(|a| rows.iter().any(|r| r["tag_name"].as_str() == Some(a)))
        {
            break;
        }
    }
    Ok(result)
}

fn run(s: &Script) -> (Result<Vec<Release>, String>, Duration) {
    // Virtual event clock. The first two requests start at t=0 even if one
    // OS thread enters later; subsequent pages start after both have joined.
    let clock = Mutex::new(Duration::ZERO);
    let initial = std::sync::Barrier::new(2);
    let result = fetch_with(
        "osx",
        &AtomicBool::new(false),
        &|request, timeout| {
            let index = match request {
                Request::Latest => 0,
                Request::Page(p) => p,
            };
            if index <= 1 {
                initial.wait();
            }
            let start = if index <= 1 {
                Duration::ZERO
            } else {
                *clock.lock().unwrap()
            };
            let latency = Duration::from_millis(s.latencies[index]);
            let end = start + latency.min(timeout);
            let mut now = clock.lock().unwrap();
            *now = (*now).max(end);
            drop(now);
            if latency >= timeout {
                Err("transport timeout".into())
            } else {
                s.response(request)
            }
        },
        &|| *clock.lock().unwrap(),
    );
    (result, clock.into_inner().unwrap())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn discovery_has_one_total_twenty_second_budget(s in script(15000)) {
        let (_, elapsed) = run(&s);
        prop_assert!(elapsed <= Duration::from_secs(20));
    }
    #[test]
    fn zero_latency_discovery_matches_the_serial_walk(s in script(0)) {
        prop_assert_eq!(run(&s).0, serial(&s));
    }
}

#[test]
fn two_slow_full_pages_share_the_discovery_deadline() {
    let s = Script {
        full: 3,
        short: 0,
        anchor: None,
        kinds: vec![15; 13],
        latencies: vec![15000; 13],
    };
    let (result, elapsed) = run(&s);
    assert!(result.is_err());
    assert_eq!(elapsed, Duration::from_secs(20));
}
