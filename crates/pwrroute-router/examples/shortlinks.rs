//! Manual live-network probe. Read a JSON array of {url, host} from stdin:
//! cargo run -p pwrroute-router --example shortlinks
//! Only use public test links. Output contains counts, status codes and
//! timings, never input URLs, redirect destinations or network error text.
//! Uses the real router with rules.json alone, its Activity record and Test
//! a link API, with a dry-run launcher. No browser is opened and no journal
//! or logger is installed. A nonzero exit means a routing check failed.
use anyhow::Result;
use pwrroute_engine::{
    Engine, HttpNetwork, Limits, Method, NativeHost, Network, Reply, Rule, RulesFile, Sources,
};
use pwrroute_router::{
    DispatchRecord, DryRunLauncher, Link, Options, Outcome, Resolver, ResolverFactory, Router,
    Source, Step,
};
use serde::Deserialize;
use std::{
    io,
    rc::Rc,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Deserialize)]
struct Case {
    url: String,
    host: String,
}

struct Probe {
    http: HttpNetwork,
    requests: Mutex<Vec<(Method, Option<u16>, u128)>>,
}
impl Network for Probe {
    fn request(&self, method: Method, url: &str, timeout: Duration) -> Result<Reply> {
        let started = Instant::now();
        let reply = self.http.request(method, url, timeout);
        self.requests.lock().unwrap().push((
            method,
            reply.as_ref().ok().map(|r| r.status),
            started.elapsed().as_millis(),
        ));
        reply
    }
}

fn main() -> Result<()> {
    let cases: Vec<Case> = serde_json::from_reader(io::stdin())
        .map_err(|_| anyhow::anyhow!("expected a JSON array of URL/host pairs on stdin"))?;
    anyhow::ensure!(!cases.is_empty(), "provide at least one case");
    let mut failed = 0;
    for (i, case) in cases.iter().enumerate() {
        let rules = RulesFile {
            default_browser: "Safari".into(),
            rules: vec![Rule {
                patterns: vec![format!("{}/*", case.host)],
                browser: "Firefox".into(),
                ..Rule::default()
            }],
            ..RulesFile::default()
        };
        let factory: ResolverFactory = Box::new(move || {
            let engine = Engine::from_sources(
                &Sources {
                    rules,
                    ..Sources::default()
                },
                Rc::new(NativeHost::new()),
                Limits::default(),
            )?;
            Ok(Box::new(engine) as Box<dyn Resolver>)
        });
        let probe = Arc::new(Probe {
            http: HttpNetwork::new(),
            requests: Mutex::new(Vec::new()),
        });
        let launcher = DryRunLauncher::new();
        let router = Router::spawn(
            Options {
                network: Some(probe.clone()),
                ..Options::default()
            },
            Some(factory),
            launcher.clone(),
        )?;
        let handle = router.handle();
        let started = Instant::now();
        let dispatched = handle.dispatch_and_wait(Link::new(&case.url, Source::Api))?;
        let tested = handle.test(Link::new(&case.url, Source::Api))?;
        let landed = |record: &DispatchRecord| {
            record.link.url == case.url
                && record.browser.as_ref().is_some_and(|b| b.name == "Firefox")
                && record
                    .steps
                    .iter()
                    .any(|s| matches!(s, Step::Expanded { .. }))
        };
        let matched = landed(&dispatched)
            && landed(&tested)
            && dispatched.outcome == Outcome::DryRun
            && tested.outcome == Outcome::Tested
            && handle
                .get(dispatched.id)
                .is_some_and(|record| landed(&record) && record.final_url == dispatched.final_url)
            && matches!(launcher.launched().as_slice(), [browser] if browser.name == "Firefox" && Some(&browser.url) == dispatched.final_url.as_ref());
        println!(
            "case {}: matched={matched} expanded={} elapsed_ms={} requests={:?}",
            i + 1,
            dispatched
                .steps
                .iter()
                .any(|s| matches!(s, Step::Expanded { .. })),
            started.elapsed().as_millis(),
            probe.requests.lock().unwrap()
        );
        failed += usize::from(!matched);
    }
    anyhow::ensure!(
        failed == 0,
        "{failed} cases did not reach their expected host"
    );
    Ok(())
}
