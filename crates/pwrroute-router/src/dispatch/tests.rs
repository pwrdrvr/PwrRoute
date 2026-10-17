use std::{
    rc::Rc,
    sync::{Arc, Mutex},
};

use pwrroute_engine::{DefaultHost, Limits};

use anyhow::anyhow;

use super::*;
use crate::{
    launcher::DryRunLauncher,
    link::{OpenerInfo, Source},
};

const CONFIG: &str = r#"
export default {
  defaultBrowser: "Safari",
  rewrite: [
    { match: "https://example.com/old*", url: "https://example.com/new" },
  ],
  handlers: [
    { match: "https://*.slack.com/*", browser: "Firefox" },
    {
      match: (url, { opener }) => opener && opener.bundleId === "com.apple.mail",
      browser: { name: "Google Chrome", profile: "Work" },
    },
    { match: "https://blocked.example.com/*", browser: null },
    { match: "https://loop.example.com/*", browser: "PwrRoute" },
  ],
};
"#;

fn engine_factory(dir: &tempfile::TempDir) -> ResolverFactory {
    let path = dir.path().join("finicky.js");
    std::fs::write(&path, CONFIG).unwrap();
    Box::new(move || {
        let engine = Engine::load(&path, Rc::new(DefaultHost), Limits::default())?;
        Ok(Box::new(engine) as Box<dyn Resolver>)
    })
}

fn engine_router(options: Options) -> (tempfile::TempDir, Router, DryRunLauncher) {
    let dir = tempfile::tempdir().unwrap();
    let launcher = DryRunLauncher::new();
    let factory = engine_factory(&dir);
    let router = Router::spawn(options, Some(factory), launcher.clone()).unwrap();
    (dir, router, launcher)
}

fn link(url: &str) -> Link {
    Link::new(url, Source::CommandLine)
}

#[test]
fn a_handler_routes_the_link_and_the_launcher_opens_it() {
    let (_dir, router, launcher) = engine_router(Options::default());
    let record = router
        .handle()
        .dispatch_and_wait(link("https://team.slack.com/archives/1"))
        .unwrap();
    assert_eq!(record.outcome, Outcome::DryRun);
    let browser = record.browser.clone().unwrap();
    assert_eq!(browser.name, "Firefox");
    assert_eq!(browser.url, "https://team.slack.com/archives/1");
    assert_eq!(
        record.steps,
        vec![
            Step::Received {
                url: "https://team.slack.com/archives/1".into()
            },
            Step::Matched {
                by: Matched::Handler { index: 0 }
            },
            Step::Browser { browser },
            Step::Launched { dry_run: true },
        ]
    );
    assert_eq!(launcher.launched().len(), 1);
    assert_eq!(router.handle().recent().len(), 1);
}

#[test]
fn a_rewrite_shows_in_the_trace_and_reaches_the_launcher() {
    let (_dir, router, launcher) = engine_router(Options::default());
    let record = router
        .handle()
        .dispatch_and_wait(link("https://example.com/old/page"))
        .unwrap();
    assert_eq!(record.final_url.as_deref(), Some("https://example.com/new"));
    assert_eq!(
        record.steps[1..3],
        [
            Step::Rewrite {
                rule: Some(0),
                url: "https://example.com/new".into()
            },
            Step::Matched {
                by: Matched::Default
            },
        ]
    );
    assert_eq!(launcher.launched()[0].url, "https://example.com/new");
    assert_eq!(launcher.launched()[0].name, "Safari");
}

#[test]
fn the_opener_reaches_the_config() {
    let (_dir, router, launcher) = engine_router(Options::default());
    let mail = OpenerInfo {
        name: "Mail".into(),
        bundle_id: "com.apple.mail".into(),
        path: "/System/Applications/Mail.app".into(),
        window_title: None,
    };
    let record = router
        .handle()
        .dispatch_and_wait(link("https://example.org/").with_opener(Some(mail)))
        .unwrap();
    assert_eq!(record.browser.unwrap().profile, "Work");
    assert_eq!(launcher.launched()[0].name, "Google Chrome");
}

#[test]
fn test_a_link_resolves_without_opening_or_recording() {
    let (_dir, router, launcher) = engine_router(Options::default());
    let record = router
        .handle()
        .test(link("https://team.slack.com/x"))
        .unwrap();
    assert_eq!(record.outcome, Outcome::Tested);
    assert_eq!(record.browser.unwrap().name, "Firefox");
    assert!(launcher.launched().is_empty());
    assert!(router.handle().recent().is_empty());
}

#[test]
fn browser_null_opens_nothing() {
    let (_dir, router, launcher) = engine_router(Options::default());
    let record = router
        .handle()
        .dispatch_and_wait(link("https://blocked.example.com/ad"))
        .unwrap();
    assert_eq!(record.outcome, Outcome::NotOpened);
    assert!(record.browser.is_none());
    assert_eq!(record.steps.last(), Some(&Step::NotOpened));
    assert!(record.steps.contains(&Step::Matched {
        by: Matched::Handler { index: 2 }
    }));
    assert!(launcher.launched().is_empty());
}

#[test]
fn a_route_back_to_pwrroute_goes_to_the_fallback() {
    let (_dir, router, launcher) = engine_router(Options {
        fallback_browser: "Fallback Browser".into(),
        ..Options::default()
    });
    let record = router
        .handle()
        .dispatch_and_wait(link("https://loop.example.com/x"))
        .unwrap();
    assert!(record.steps.contains(&Step::RefusedLoop {
        browser: "PwrRoute".into()
    }));
    assert_eq!(launcher.launched()[0].name, "Fallback Browser");
    assert_eq!(launcher.launched()[0].url, "https://loop.example.com/x");
}

#[test]
fn with_no_config_links_go_to_the_fallback() {
    let launcher = DryRunLauncher::new();
    let router = Router::spawn(Options::default(), None, launcher.clone()).unwrap();
    let record = router
        .handle()
        .dispatch_and_wait(link("https://example.com/"))
        .unwrap();
    assert!(record.steps.contains(&Step::Fallback {
        reason: "no config is selected".into()
    }));
    assert_eq!(launcher.launched()[0].name, FALLBACK_BROWSER);
    assert_eq!(
        router.handle().status().unwrap().config_error.as_deref(),
        Some("no config is selected")
    );
}

#[test]
fn a_config_that_fails_to_load_leaves_the_fallback_with_the_reason() {
    let factory: ResolverFactory = Box::new(|| Err(anyhow!("syntax error at line 3")));
    let launcher = DryRunLauncher::new();
    let router = Router::spawn(Options::default(), Some(factory), launcher.clone()).unwrap();
    let record = router
        .handle()
        .dispatch_and_wait(link("https://example.com/"))
        .unwrap();
    assert!(record.steps.contains(&Step::Fallback {
        reason: "syntax error at line 3".into()
    }));
    assert_eq!(launcher.launched().len(), 1);
}

struct Failing;
impl Resolver for Failing {
    fn resolve(&self, _: &str, _: Option<&Opener>) -> Result<Resolved> {
        Err(anyhow!("resolving the URL: timed out"))
    }
}

#[test]
fn a_resolve_error_is_traced_and_the_link_still_opens() {
    let factory: ResolverFactory = Box::new(|| Ok(Box::new(Failing) as Box<dyn Resolver>));
    let launcher = DryRunLauncher::new();
    let router = Router::spawn(Options::default(), Some(factory), launcher.clone()).unwrap();
    let record = router
        .handle()
        .dispatch_and_wait(link("https://example.com/"))
        .unwrap();
    assert!(record.steps.contains(&Step::ConfigError {
        message: "resolving the URL: timed out".into()
    }));
    assert_eq!(record.outcome, Outcome::DryRun);
    assert_eq!(launcher.launched()[0].name, FALLBACK_BROWSER);
}

/// Reports rule-level steps, as a tracing engine would.
struct Tracing;
impl Resolver for Tracing {
    fn resolve(&self, url: &str, _: Option<&Opener>) -> Result<Resolved> {
        Ok(Resolved {
            browser: Browser {
                name: "Firefox".into(),
                app_type: AppType::AppName,
                url: format!("{url}?rewritten"),
                profile: String::new(),
                args: Vec::new(),
                open_in_background: None,
            },
            error: None,
            rule_steps: vec![
                Step::Rewrite {
                    rule: Some(1),
                    url: format!("{url}?rewritten"),
                },
                Step::Matched {
                    by: Matched::Handler { index: 3 },
                },
            ],
        })
    }
}

#[test]
fn rule_steps_from_the_resolver_replace_the_derived_ones() {
    let factory: ResolverFactory = Box::new(|| Ok(Box::new(Tracing) as Box<dyn Resolver>));
    let router = Router::spawn(Options::default(), Some(factory), DryRunLauncher::new()).unwrap();
    let record = router
        .handle()
        .dispatch_and_wait(link("https://example.com/"))
        .unwrap();
    assert_eq!(
        record.steps[1..3],
        [
            Step::Rewrite {
                rule: Some(1),
                url: "https://example.com/?rewritten".into()
            },
            Step::Matched {
                by: Matched::Handler { index: 3 }
            },
        ]
    );
}

struct Broken;
impl Launcher for Broken {
    fn launch(&mut self, _: &Browser) -> Result<()> {
        Err(anyhow!("Firefox is not installed"))
    }
}

#[test]
fn a_launch_failure_is_recorded() {
    let router = Router::spawn(Options::default(), None, Broken).unwrap();
    let record = router
        .handle()
        .dispatch_and_wait(link("https://example.com/"))
        .unwrap();
    assert_eq!(record.outcome, Outcome::Failed);
    assert_eq!(
        record.steps.last(),
        Some(&Step::LaunchFailed {
            message: "Firefox is not installed".into()
        })
    );
}

#[test]
fn a_bad_link_is_rejected_and_recorded() {
    let launcher = DryRunLauncher::new();
    let router = Router::spawn(Options::default(), None, launcher.clone()).unwrap();
    let record = router
        .handle()
        .dispatch_and_wait(link("not a url"))
        .unwrap();
    assert_eq!(record.outcome, Outcome::Failed);
    assert!(matches!(record.steps[..], [Step::Rejected { .. }]));
    assert!(launcher.launched().is_empty());
    assert_eq!(router.handle().recent().len(), 1);
}

#[test]
fn the_recent_buffer_keeps_the_newest_up_to_its_capacity() {
    let router = Router::spawn(
        Options {
            capacity: 3,
            ..Options::default()
        },
        None,
        DryRunLauncher::new(),
    )
    .unwrap();
    let handle = router.handle();
    let mut ids = Vec::new();
    for n in 0..5 {
        ids.push(
            handle
                .dispatch(link(&format!("https://example.com/{n}")))
                .unwrap(),
        );
    }
    // A waited dispatch is queued behind the others, so they are all done.
    let last = handle
        .dispatch_and_wait(link("https://example.com/5"))
        .unwrap();
    ids.push(last.id);
    let recent: Vec<_> = handle.recent().iter().map(|r| r.id).collect();
    assert_eq!(recent, vec![ids[5], ids[4], ids[3]]);
    assert!(handle.get(ids[0]).is_none());
    assert_eq!(
        handle.get(ids[4]).unwrap().link.url,
        "https://example.com/4"
    );
    handle.clear_recent();
    assert!(handle.recent().is_empty());
}

#[test]
fn ids_are_unique_and_increasing() {
    let router = Router::spawn(Options::default(), None, DryRunLauncher::new()).unwrap();
    let handle = router.handle();
    let a = handle.dispatch(link("https://example.com/a")).unwrap();
    let b = handle
        .dispatch_forwarded(link("https://example.com/b"))
        .unwrap();
    assert!(b > a);
    let c = handle
        .dispatch_and_wait(link("https://example.com/c"))
        .unwrap();
    assert!(c.id > b);
    assert!(handle.get(b).unwrap().forwarded);
    assert!(!handle.get(a).unwrap().forwarded);
}

#[test]
fn replacing_the_resolver_keeps_the_old_one_when_the_new_one_fails() {
    let (dir, router, launcher) = engine_router(Options::default());
    let handle = router.handle();
    let err = handle
        .replace_resolver(Box::new(|| Err(anyhow!("bad config"))))
        .unwrap()
        .unwrap_err();
    assert_eq!(err, "bad config");
    handle
        .dispatch_and_wait(link("https://team.slack.com/x"))
        .unwrap();
    assert_eq!(launcher.launched()[0].name, "Firefox");

    // And a good one replaces it.
    handle
        .replace_resolver(engine_factory(&dir))
        .unwrap()
        .unwrap();
    assert_eq!(handle.status().unwrap().config_error, None);
}

#[test]
fn on_dispatch_sees_each_recorded_dispatch() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let router = Router::spawn(
        Options {
            on_dispatch: Some(Arc::new(move |r: &DispatchRecord| {
                sink.lock().unwrap().push(r.id);
            })),
            ..Options::default()
        },
        None,
        DryRunLauncher::new(),
    )
    .unwrap();
    let handle = router.handle();
    let record = handle
        .dispatch_and_wait(link("https://example.com/"))
        .unwrap();
    handle.test(link("https://example.com/")).unwrap();
    assert_eq!(*seen.lock().unwrap(), vec![record.id]);
}

#[test]
fn the_disk_view_of_a_dispatch_carries_no_link_data() {
    let (_dir, router, _launcher) = engine_router(Options::default());
    let secret_url = "https://example.com/old/reset?token=hunter2";
    let record = router
        .handle()
        .dispatch_and_wait(link(secret_url).with_opener(Some(OpenerInfo {
            name: "Secret Opener".into(),
            bundle_id: "com.example.secret".into(),
            path: "/Applications/Secret.app".into(),
            window_title: Some("Inbox".into()),
        })))
        .unwrap();
    let timing = record.timing();
    assert_eq!(timing.rewrites, 1);
    let json = serde_json::to_string(&timing).unwrap();
    for private in [
        "example.com",
        "hunter2",
        "Secret",
        "com.example",
        "Inbox",
        "Safari",
    ] {
        assert!(!json.contains(private), "{private} leaked into {json}");
    }
    // The in-memory record has it all.
    let full = serde_json::to_string(&record).unwrap();
    assert!(full.contains("hunter2") && full.contains("Secret Opener"));
}

#[test]
fn dropping_the_router_stops_it() {
    let router = Router::spawn(Options::default(), None, DryRunLauncher::new()).unwrap();
    let handle = router.handle();
    drop(router);
    assert_eq!(
        handle.dispatch(link("https://example.com/")),
        Err(RouterGone)
    );
}

#[test]
fn a_re_serialized_url_is_not_a_rewrite() {
    let (_dir, router, _launcher) = engine_router(Options::default());
    let record = router
        .handle()
        .dispatch_and_wait(link("https://example.org"))
        .unwrap();
    // The engine hands back `https://example.org/`; no rewrite ran.
    assert!(
        !record
            .steps
            .iter()
            .any(|s| matches!(s, Step::Rewrite { .. })),
        "{:?}",
        record.steps
    );
    assert_eq!(record.timing().rewrites, 0);
}

#[test]
fn answer_queues_forwarded_links_and_reports_activate() {
    use crate::ipc::Request;

    let router = Router::spawn(Options::default(), None, DryRunLauncher::new()).unwrap();
    let handle = router.handle();
    let response = handle.answer(
        Request::Open {
            links: vec![link("https://example.com/a"), link("https://example.com/b")],
        },
        || panic!("not an activate"),
    );
    assert!(response.ok);
    assert_eq!(response.dispatched.len(), 2);
    handle
        .dispatch_and_wait(link("https://example.com/c"))
        .unwrap();
    assert!(handle.get(response.dispatched[0]).unwrap().forwarded);

    let mut activated = false;
    assert!(handle.answer(Request::Activate, || activated = true).ok);
    assert!(activated);

    drop(router);
    let response = handle.answer(
        Request::Open {
            links: vec![link("https://example.com/d")],
        },
        || (),
    );
    assert!(!response.ok);
}
