//! A router in dry run, fed by other processes, as the OS would: a second
//! launch forwards its links over the endpoint and exits, and a cold launch
//! with nothing running routes them itself.
//!
//! The other process is this test binary, re-run with an environment
//! variable that turns one of the `child_*` tests into the launch. Without
//! the variable those tests do nothing.

use std::{
    env, fs,
    path::Path,
    process::{Command, Output},
    rc::Rc,
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

use pwrroute_engine::{DefaultHost, Engine, Limits};
use pwrroute_router::{
    DispatchRecord, DryRunLauncher, Link, Options, Outcome, Resolver, ResolverFactory, Router,
    RouterHandle, Source,
    instance::{self, Claim, Endpoint},
};

const ENDPOINT_VAR: &str = "PWRROUTE_TEST_ENDPOINT";
const LINKS_VAR: &str = "PWRROUTE_TEST_LINKS";

const CONFIG: &str = r#"
export default {
  defaultBrowser: "Safari",
  rewrite: [{ match: "https://example.com/old*", url: "https://example.com/new" }],
  handlers: [{ match: "https://*.slack.com/*", browser: "Firefox" }],
};
"#;

/// Serializes the tests that spawn processes, so a slow CI runner doesn't
/// time out under a pile of concurrent builds of the same binary.
static SERIAL: Mutex<()> = Mutex::new(());

fn temp_endpoint() -> (tempfile::TempDir, Endpoint) {
    // Short: a Unix socket path is limited to about 100 bytes.
    let dir = tempfile::Builder::new().prefix("pr").tempdir().unwrap();
    let endpoint = Endpoint::new(dir.path().join("r"));
    (dir, endpoint)
}

fn engine_factory(dir: &Path) -> ResolverFactory {
    let path = dir.join("finicky.js");
    fs::write(&path, CONFIG).unwrap();
    Box::new(move || {
        let engine = Engine::load(&path, Rc::new(DefaultHost), Limits::default())?;
        Ok(Box::new(engine) as Box<dyn Resolver>)
    })
}

/// Runs `child` from this test binary as a separate process.
fn launch(child: &str, endpoint: &Endpoint, links: &[&str]) -> Output {
    Command::new(env::current_exe().unwrap())
        .args([child, "--exact", "--nocapture", "--test-threads=1"])
        .env(ENDPOINT_VAR, endpoint.dir())
        .env(LINKS_VAR, links.join("\n"))
        .output()
        .unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "child failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn wait_for(handle: &RouterHandle, count: usize) -> Vec<DispatchRecord> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let recent = handle.recent();
        if recent.len() >= count || Instant::now() > deadline {
            return recent;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// The child's side: what a launch reads from its environment.
fn child_env() -> Option<(Endpoint, Vec<Link>)> {
    let dir = env::var_os(ENDPOINT_VAR)?;
    let links = env::var(LINKS_VAR)
        .unwrap_or_default()
        .lines()
        .map(|url| Link::new(url, Source::CommandLine))
        .collect();
    Some((Endpoint::new(dir), links))
}

#[test]
fn a_second_launch_forwards_its_links_to_the_running_router() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (dir, endpoint) = temp_endpoint();
    let Claim::Primary(primary) = instance::claim(&endpoint).unwrap() else {
        panic!("nothing else holds this endpoint");
    };
    let launcher = DryRunLauncher::new();
    let router = Router::spawn(
        Options::default(),
        Some(engine_factory(dir.path())),
        launcher.clone(),
    )
    .unwrap();
    let handle = router.handle();
    let server = {
        let handle = handle.clone();
        primary
            .serve(move |request| handle.answer(request, || ()))
            .unwrap()
    };

    let output = launch(
        "child_forwards_and_exits",
        &endpoint,
        &[
            "https://team.slack.com/archives/1",
            "https://example.com/old/page",
        ],
    );
    assert_success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("forwarded 2"));

    let recent = wait_for(&handle, 2);
    assert_eq!(recent.len(), 2);
    // Newest first.
    let (rewritten, slack) = (&recent[0], &recent[1]);
    assert!(slack.forwarded && rewritten.forwarded);
    assert_eq!(slack.outcome, Outcome::DryRun);
    assert_eq!(slack.browser.as_ref().unwrap().name, "Firefox");
    assert_eq!(
        rewritten.final_url.as_deref(),
        Some("https://example.com/new")
    );
    let launched: Vec<_> = launcher
        .launched()
        .into_iter()
        .map(|b| (b.name, b.url))
        .collect();
    assert_eq!(
        launched,
        [
            (
                "Firefox".to_owned(),
                "https://team.slack.com/archives/1".to_owned()
            ),
            ("Safari".to_owned(), "https://example.com/new".to_owned()),
        ]
    );

    // Several launches at once all get through.
    let children: Vec<_> = (0..4)
        .map(|n| {
            let endpoint = endpoint.clone();
            thread::spawn(move || {
                launch(
                    "child_forwards_and_exits",
                    &endpoint,
                    &[&format!("https://example.org/{n}")],
                )
            })
        })
        .collect();
    for child in children {
        assert_success(&child.join().unwrap());
    }
    assert_eq!(wait_for(&handle, 6).len(), 6);

    drop(server);
    drop(router);
}

#[test]
fn a_cold_launch_routes_its_links_when_nothing_is_running() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (_dir, endpoint) = temp_endpoint();
    let output = launch("child_cold_start", &endpoint, &["https://example.com/cold"]);
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("primary"), "{stdout}");
    assert!(stdout.contains("outcome dryRun"), "{stdout}");
    // It exited, and with it went the lock.
    assert!(matches!(
        instance::claim(&endpoint).unwrap(),
        Claim::Primary(_)
    ));
}

/// A second launch: finds the router running, forwards, exits.
#[test]
fn child_forwards_and_exits() {
    let Some((endpoint, links)) = child_env() else {
        return;
    };
    let Claim::Running(client) = instance::claim(&endpoint).unwrap() else {
        panic!("the parent test holds the router lock");
    };
    let count = links.len();
    let response = client.open(links, Duration::from_secs(5)).unwrap();
    assert_eq!(response.dispatched.len(), count);
    assert_ne!(response.pid, std::process::id());
    println!("forwarded {count}");
}

/// A cold launch: nothing running, so it becomes the router, routes its
/// links in dry run, and exits.
#[test]
fn child_cold_start() {
    let Some((endpoint, links)) = child_env() else {
        return;
    };
    let Claim::Primary(primary) = instance::claim(&endpoint).unwrap() else {
        panic!("nothing should be running");
    };
    println!("primary");
    let router = Router::spawn(Options::default(), None, DryRunLauncher::new()).unwrap();
    let handle = router.handle();
    let _server = {
        let handle = handle.clone();
        primary
            .serve(move |request| handle.answer(request, || ()))
            .unwrap()
    };
    for link in links {
        let record = handle.dispatch_and_wait(link).unwrap();
        let outcome = serde_json::to_value(record.outcome).unwrap();
        println!("outcome {}", outcome.as_str().unwrap());
    }
}
