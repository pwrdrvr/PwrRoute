use std::{
    rc::Rc,
    sync::{Arc, Mutex},
};

use pwrroute_engine::{DefaultHost, Limits, Rewrite, RewriteAction, Rule, RulesFile, Sources};

use anyhow::anyhow;

use super::*;
use crate::{
    launcher::DryRunLauncher,
    link::{OpenerInfo, Source},
    trace::WITHHELD,
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
    {
      match: (url) => {
        if (url.host === "throws.example.com") throw new Error("matcher failed");
        return false;
      },
      browser: "Opera",
    },
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
fn a_rule_that_throws_is_traced_to_the_default() {
    let (_dir, router, launcher) = engine_router(Options::default());
    let record = router
        .handle()
        .dispatch_and_wait(link("https://throws.example.com/x"))
        .unwrap();
    assert_eq!(
        record.steps[1],
        Step::Matched {
            by: Matched::Default
        }
    );
    assert!(
        record.steps.iter().any(
            |s| matches!(s, Step::ConfigError { message } if message.contains("matcher failed"))
        )
    );
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
                private: false,
                privacy: pwrroute_engine::Privacy::Full,
            },
            error: None,
            privacy: Privacy::Full,
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
            retention: Retention {
                count: 3,
                hours: 24,
            },
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

/// A resolver whose files come and go: `refresh` reports what is in charge
/// and counts its calls.
struct Watching {
    in_charge: Arc<Mutex<std::result::Result<(), String>>>,
    refreshed: Arc<AtomicU64>,
}

impl Resolver for Watching {
    fn resolve(&self, url: &str, _: Option<&Opener>) -> Result<Resolved> {
        Ok(Resolved {
            browser: Browser {
                name: "Watched Browser".into(),
                app_type: AppType::AppName,
                url: url.to_owned(),
                profile: String::new(),
                args: Vec::new(),
                open_in_background: None,
                private: false,
                privacy: pwrroute_engine::Privacy::Full,
            },
            error: None,
            privacy: Privacy::Full,
            rule_steps: Vec::new(),
        })
    }

    fn refresh(&self) -> std::result::Result<(), String> {
        self.refreshed.fetch_add(1, Ordering::Relaxed);
        self.in_charge.lock().unwrap().clone()
    }
}

#[test]
fn refresh_runs_before_each_link_and_its_reason_is_the_fallback_reason() {
    let in_charge = Arc::new(Mutex::new(Err("no rules.json at /nowhere".to_owned())));
    let refreshed = Arc::new(AtomicU64::new(0));
    let factory: ResolverFactory = {
        let (in_charge, refreshed) = (Arc::clone(&in_charge), Arc::clone(&refreshed));
        Box::new(move || {
            Ok(Box::new(Watching {
                in_charge,
                refreshed,
            }) as Box<dyn Resolver>)
        })
    };
    let launcher = DryRunLauncher::new();
    let router = Router::spawn(Options::default(), Some(factory), launcher.clone()).unwrap();
    let handle = router.handle();

    let record = handle
        .dispatch_and_wait(link("https://example.com/"))
        .unwrap();
    assert!(record.steps.contains(&Step::Fallback {
        reason: "no rules.json at /nowhere".into()
    }));
    assert_eq!(launcher.launched()[0].name, FALLBACK_BROWSER);
    assert_eq!(
        handle.status().unwrap().config_error.as_deref(),
        Some("no rules.json at /nowhere")
    );

    // The file appears: the next link sees it without a replace.
    *in_charge.lock().unwrap() = Ok(());
    let record = handle
        .dispatch_and_wait(link("https://example.com/"))
        .unwrap();
    assert_eq!(record.browser.unwrap().name, "Watched Browser");
    assert_eq!(handle.status().unwrap().config_error, None);

    // A refresh on its own asks the resolver too, and returns at once.
    let before = refreshed.load(Ordering::Relaxed);
    handle.refresh().unwrap();
    handle.status().unwrap();
    assert!(refreshed.load(Ordering::Relaxed) >= before + 2);
}

/// Routes every link to one browser, at the privacy level its host names:
/// `host.example.com` is kept as host only, `none.example.com` is only
/// counted, anything else in full.
struct Leveled;
impl Resolver for Leveled {
    fn resolve(&self, url: &str, _: Option<&Opener>) -> Result<Resolved> {
        let privacy = if url.contains("://host.") {
            Privacy::Host
        } else if url.contains("://none.") {
            Privacy::None
        } else {
            Privacy::Full
        };
        Ok(Resolved {
            browser: Browser {
                name: "Firefox".into(),
                app_type: AppType::AppName,
                url: format!("{url}&rewritten=1"),
                profile: "Work".into(),
                args: vec![format!("--url={url}")],
                open_in_background: None,
                private: false,
                privacy: Privacy::Full,
            },
            error: Some(format!("the rule warned about {url}")),
            privacy,
            rule_steps: Vec::new(),
        })
    }
}

fn leveled_factory() -> ResolverFactory {
    Box::new(|| Ok(Box::new(Leveled) as Box<dyn Resolver>))
}

fn secret_opener() -> OpenerInfo {
    OpenerInfo {
        name: "Secret Opener".into(),
        bundle_id: "com.example.secret".into(),
        path: "/Applications/Secret.app".into(),
        window_title: Some("Inbox".into()),
    }
}

#[test]
fn a_host_rule_reduces_the_record_everywhere_but_the_launcher_opens_the_full_url() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let launcher = DryRunLauncher::new();
    let router = Router::spawn(
        Options {
            on_dispatch: Some(Arc::new(move |r: &DispatchRecord| {
                sink.lock().unwrap().push(r.clone());
            })),
            ..Options::default()
        },
        Some(leveled_factory()),
        launcher.clone(),
    )
    .unwrap();
    let handle = router.handle();
    let secret = "https://host.example.com/reset?token=hunter2";
    let reply = handle
        .dispatch_and_wait(link(secret).with_opener(Some(secret_opener())))
        .unwrap();
    // The browser got the real link.
    assert_eq!(launcher.launched()[0].url, format!("{secret}&rewritten=1"));

    // Every copy outside the dispatch thread is the reduced one.
    let stored = handle.get(reply.id).unwrap();
    let callback = seen.lock().unwrap()[0].clone();
    for record in [&reply, &stored, &callback] {
        assert_eq!(record.privacy, Privacy::Host);
        assert_eq!(record.link.url, "https://host.example.com");
        assert_eq!(
            record.final_url.as_deref(),
            Some("https://host.example.com")
        );
        let opener = record.link.opener.as_ref().unwrap();
        assert_eq!(opener.name, "Secret Opener");
        assert_eq!(opener.window_title, None);
        assert_eq!(record.outcome, Outcome::DryRun);
        assert_eq!(record.browser.as_ref().unwrap().profile, "Work");
        let json = serde_json::to_string(record).unwrap();
        for private in [
            "hunter2",
            "/reset",
            "rewritten",
            "--url",
            "warned about",
            "Inbox",
        ] {
            assert!(!json.contains(private), "{private} leaked into {json}");
        }
        assert!(record.steps.contains(&Step::ConfigError {
            message: WITHHELD.into()
        }));
    }
}

#[test]
fn a_none_rule_only_counts_the_dispatch() {
    let launcher = DryRunLauncher::new();
    let router = Router::spawn(
        Options::default(),
        Some(leveled_factory()),
        launcher.clone(),
    )
    .unwrap();
    let handle = router.handle();
    let record = handle
        .dispatch_and_wait(
            link("https://none.example.com/private").with_opener(Some(secret_opener())),
        )
        .unwrap();
    assert_eq!(launcher.launched().len(), 1);
    assert_eq!(record.privacy, Privacy::None);
    assert_eq!(record.outcome, Outcome::DryRun);
    assert!(record.timings.total_us > 0);
    let json = serde_json::to_string(&handle.get(record.id).unwrap()).unwrap();
    for private in [
        "example.com",
        "private",
        "Secret",
        "Inbox",
        "Firefox",
        "Work",
    ] {
        assert!(!json.contains(private), "{private} leaked into {json}");
    }

    // A link the rules leave alone is kept in full, as are rejected ones.
    let record = handle
        .dispatch_and_wait(link("https://full.example.com/page"))
        .unwrap();
    assert_eq!(record.privacy, Privacy::Full);
    assert_eq!(record.link.url, "https://full.example.com/page");
    let record = handle.dispatch_and_wait(link("not a url")).unwrap();
    assert_eq!(record.privacy, Privacy::Full);
}

#[test]
fn test_a_link_reports_the_level_without_reducing() {
    let router = Router::spawn(
        Options::default(),
        Some(leveled_factory()),
        DryRunLauncher::new(),
    )
    .unwrap();
    let record = router
        .handle()
        .test(link("https://host.example.com/reset?token=hunter2"))
        .unwrap();
    assert_eq!(record.privacy, Privacy::Host);
    assert_eq!(record.outcome, Outcome::Tested);
    assert_eq!(
        record.link.url,
        "https://host.example.com/reset?token=hunter2"
    );
    assert!(router.handle().recent().is_empty());
}

/// A `rules.json` with no JS config, so every level comes from the engine's
/// own reading of the rules: a `host` handler, a `none` handler, a private
/// handler that says nothing about privacy, a handler with no level behind
/// a `host` rewrite, and a plain handler.
fn leveled_rules() -> RulesFile {
    let handler = |pattern: &str| Rule {
        patterns: vec![pattern.to_owned()],
        browser: "Firefox".into(),
        profile: "Work".into(),
        ..Rule::default()
    };
    RulesFile {
        default_browser: "Safari".into(),
        rewrites: vec![Rewrite {
            patterns: vec!["tracked.example.com/*".into()],
            actions: vec![RewriteAction::DropQuery(vec!["utm_*".into()])],
            privacy: Some(Privacy::Host.into()),
            ..Rewrite::default()
        }],
        rules: vec![
            Rule {
                privacy: Some(Privacy::Host.into()),
                ..handler("host.example.com/*")
            },
            Rule {
                privacy: Some(Privacy::None.into()),
                ..handler("none.example.com/*")
            },
            Rule {
                private: true,
                ..handler("private.example.com/*")
            },
            handler("tracked.example.com/*"),
            handler("full.example.com/*"),
        ],
        ..RulesFile::default()
    }
}

fn rules_factory(rules: RulesFile) -> ResolverFactory {
    Box::new(move || {
        let sources = Sources {
            config: None,
            rules,
        };
        let engine = Engine::from_sources(&sources, Rc::new(DefaultHost), Limits::default())?;
        Ok(Box::new(engine) as Box<dyn Resolver>)
    })
}

fn rules_router(rules: RulesFile) -> (Router, DryRunLauncher) {
    let launcher = DryRunLauncher::new();
    let router = Router::spawn(
        Options::default(),
        Some(rules_factory(rules)),
        launcher.clone(),
    )
    .unwrap();
    (router, launcher)
}

#[test]
fn a_rules_json_host_level_reaches_the_stored_record_through_the_engine() {
    let (router, launcher) = rules_router(leveled_rules());
    let handle = router.handle();
    let secret = "https://host.example.com/reset?token=hunter2";
    let reply = handle
        .dispatch_and_wait(link(secret).with_opener(Some(secret_opener())))
        .unwrap();
    // The browser got the real link, routed by the rule.
    assert_eq!(launcher.launched()[0].url, secret);
    assert_eq!(launcher.launched()[0].profile, "Work");

    let stored = handle.get(reply.id).unwrap();
    for record in [&reply, &stored] {
        assert_eq!(record.privacy, Privacy::Host);
        assert_eq!(record.outcome, Outcome::DryRun);
        assert_eq!(record.link.url, "https://host.example.com");
        assert_eq!(
            record.final_url.as_deref(),
            Some("https://host.example.com")
        );
        assert_eq!(record.link.opener.as_ref().unwrap().name, "Secret Opener");
        let browser = record.browser.as_ref().unwrap();
        assert_eq!(browser.url, "https://host.example.com");
        assert_eq!(browser.privacy, Privacy::Host);
        assert!(!browser.private);
        let json = serde_json::to_string(record).unwrap();
        for private in ["hunter2", "/reset", "token"] {
            assert!(!json.contains(private), "{private} leaked into {json}");
        }
    }
}

#[test]
fn a_rules_json_none_level_only_counts_the_dispatch_through_the_engine() {
    let (router, launcher) = rules_router(leveled_rules());
    let handle = router.handle();
    let secret = "https://none.example.com/private?token=hunter2";
    let reply = handle
        .dispatch_and_wait(link(secret).with_opener(Some(secret_opener())))
        .unwrap();
    assert_eq!(launcher.launched()[0].url, secret);

    let stored = handle.get(reply.id).unwrap();
    for record in [&reply, &stored] {
        assert_eq!(record.privacy, Privacy::None);
        assert_eq!(record.outcome, Outcome::DryRun);
        assert_eq!(record.link.url, "");
        assert!(record.link.opener.is_none());
        assert!(record.final_url.is_none());
        assert!(record.browser.is_none());
        assert!(record.steps.is_empty());
        assert!(record.timings.total_us > 0);
        let json = serde_json::to_string(record).unwrap();
        for private in [
            "example.com",
            "hunter2",
            "Secret",
            "Inbox",
            "Firefox",
            "Work",
        ] {
            assert!(!json.contains(private), "{private} leaked into {json}");
        }
    }
}

#[test]
fn a_private_rule_with_no_level_keeps_the_host_through_the_engine() {
    let (router, launcher) = rules_router(leveled_rules());
    let handle = router.handle();
    let secret = "https://private.example.com/inbox?token=hunter2";
    let reply = handle
        .dispatch_and_wait(link(secret).with_opener(Some(secret_opener())))
        .unwrap();
    // The launcher sees the full URL and the private flag.
    assert_eq!(launcher.launched()[0].url, secret);
    assert!(launcher.launched()[0].private);

    let stored = handle.get(reply.id).unwrap();
    for record in [&reply, &stored] {
        assert_eq!(record.privacy, Privacy::Host);
        assert_eq!(record.link.url, "https://private.example.com");
        assert_eq!(
            record.final_url.as_deref(),
            Some("https://private.example.com")
        );
        let browser = record.browser.as_ref().unwrap();
        assert!(browser.private);
        assert_eq!(browser.privacy, Privacy::Host);
        let json = serde_json::to_string(record).unwrap();
        for private in ["hunter2", "/inbox", "token"] {
            assert!(!json.contains(private), "{private} leaked into {json}");
        }
    }
}

#[test]
fn the_engines_tightest_level_and_an_unmatched_link_come_through_the_router() {
    let (router, launcher) = rules_router(leveled_rules());
    let handle = router.handle();

    // A `host` rewrite behind a handler with no level: the tightest wins,
    // and the rewrite itself still ran for the browser.
    let record = handle
        .dispatch_and_wait(link(
            "https://tracked.example.com/page?utm_source=mail&id=7",
        ))
        .unwrap();
    assert_eq!(
        launcher.launched()[0].url,
        "https://tracked.example.com/page?id=7"
    );
    assert_eq!(record.privacy, Privacy::Host);
    assert_eq!(record.link.url, "https://tracked.example.com");
    assert_eq!(
        record.final_url.as_deref(),
        Some("https://tracked.example.com")
    );

    // A handler with no level, and a link no rule matched: kept in full.
    let record = handle
        .dispatch_and_wait(link("https://full.example.com/page?q=1"))
        .unwrap();
    assert_eq!(record.privacy, Privacy::Full);
    assert_eq!(record.link.url, "https://full.example.com/page?q=1");
    assert_eq!(record.browser.as_ref().unwrap().name, "Firefox");
    let record = handle
        .dispatch_and_wait(link("https://unrouted.example.com/page?q=1"))
        .unwrap();
    assert_eq!(record.privacy, Privacy::Full);
    assert_eq!(record.link.url, "https://unrouted.example.com/page?q=1");
    assert_eq!(
        record.final_url.as_deref(),
        Some("https://unrouted.example.com/page?q=1")
    );
    assert_eq!(record.browser.as_ref().unwrap().name, "Safari");
    assert_eq!(handle.get(record.id).unwrap().privacy, Privacy::Full);
}

#[test]
fn unrouted_links_stay_full() {
    let router = Router::spawn(Options::default(), None, DryRunLauncher::new()).unwrap();
    let record = router
        .handle()
        .dispatch_and_wait(link("https://example.com/page?q=1"))
        .unwrap();
    assert_eq!(record.privacy, Privacy::Full);
    assert_eq!(record.link.url, "https://example.com/page?q=1");
}

#[test]
fn with_the_journal_off_nothing_is_written_and_the_description_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let before: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert!(before.is_empty());
    let router = Router::spawn(
        Options::default(),
        Some(leveled_factory()),
        DryRunLauncher::new(),
    )
    .unwrap();
    let handle = router.handle();
    handle
        .dispatch_and_wait(link("https://full.example.com/page?token=hunter2"))
        .unwrap();
    assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    assert!(handle.journal(10).unwrap().is_empty());
    let retention = handle.retention();
    assert_eq!(retention.memory, Retention::default());
    assert_eq!(retention.journal, None);
    assert_eq!(
        retention.description,
        "Kept in memory only: the last 500 links or 24 hours, cleared when PwrRoute quits."
    );
    assert_eq!(
        Options::default().retention_description(),
        retention.description
    );
    assert_eq!(handle.status().unwrap().journal_error, None);
}

#[test]
fn with_the_journal_on_each_dispatch_is_a_line_at_its_level() {
    let dir = tempfile::tempdir().unwrap();
    let journal = JournalConfig {
        days: 3,
        ..JournalConfig::new(dir.path().join("activity"), JournalMode::Full)
    };
    let options = Options {
        retention: Retention {
            count: 1,
            hours: 24,
        },
        journal: Some(journal.clone()),
        ..Options::default()
    };
    assert_eq!(
        options.retention_description(),
        format!(
            "Kept in memory for the last 1 link or 24 hours, cleared when PwrRoute quits, \
             and journaled with full URLs (except links whose rule keeps less) to {} for 3 days.",
            dir.path()
                .join("activity")
                .join("activity-YYYY-MM-DD.jsonl")
                .display()
        )
    );
    let router = Router::spawn(options, Some(leveled_factory()), DryRunLauncher::new()).unwrap();
    let handle = router.handle();
    let full = handle
        .dispatch_and_wait(link("https://full.example.com/page?token=hunter2"))
        .unwrap();
    let host = handle
        .dispatch_and_wait(link("https://host.example.com/page?token=hunter2"))
        .unwrap();
    let none = handle
        .dispatch_and_wait(link("https://none.example.com/page?token=hunter2"))
        .unwrap();
    handle
        .test(link("https://full.example.com/tested"))
        .unwrap();
    assert_eq!(handle.status().unwrap().journal_error, None);

    // Memory holds one; the journal holds all three, newest first.
    assert_eq!(handle.recent().len(), 1);
    let journaled = handle.journal(10).unwrap();
    assert_eq!(
        journaled.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![none.id, host.id, full.id]
    );
    assert_eq!(journaled[0].privacy, Privacy::None);
    assert_eq!(journaled[1].privacy, Privacy::Host);
    assert_eq!(journaled[2].privacy, Privacy::Full);
    assert_eq!(
        journaled[2].link.url,
        "https://full.example.com/page?token=hunter2"
    );
    assert_eq!(handle.journal(1).unwrap()[0].id, none.id);

    let text = std::fs::read_dir(dir.path().join("activity"))
        .unwrap()
        .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
        .collect::<String>();
    assert_eq!(text.lines().count(), 3);
    assert!(!text.contains("tested"));
    assert!(!text.contains("host.example.com/page"));
    assert!(!text.contains("none.example.com"));
    assert!(!text.contains("warned about"), "error text in the journal");

    let retention = handle.retention();
    assert_eq!(retention.journal, Some(journal));
    assert_eq!(
        retention.description,
        Options {
            retention: Retention {
                count: 1,
                hours: 24
            },
            journal: retention.journal.clone(),
            ..Options::default()
        }
        .retention_description()
    );
}

#[test]
fn a_hosts_journal_keeps_no_full_url_and_a_failing_one_does_not_stop_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let router = Router::spawn(
        Options {
            journal: Some(JournalConfig::new(dir.path(), JournalMode::Hosts)),
            ..Options::default()
        },
        Some(leveled_factory()),
        DryRunLauncher::new(),
    )
    .unwrap();
    let handle = router.handle();
    let record = handle
        .dispatch_and_wait(link("https://full.example.com/page?token=hunter2"))
        .unwrap();
    // In memory it is full; on disk it is host only.
    assert_eq!(record.privacy, Privacy::Full);
    let journaled = handle.journal(10).unwrap();
    assert_eq!(journaled[0].privacy, Privacy::Host);
    assert_eq!(journaled[0].link.url, "https://full.example.com");
    assert_eq!(
        handle.retention().description,
        format!(
            "Kept in memory for the last 500 links or 24 hours, cleared when PwrRoute quits, \
             and journaled as scheme and host only to {} for 7 days.",
            dir.path().join("activity-YYYY-MM-DD.jsonl").display()
        )
    );

    // A journal that cannot be written: dispatch goes on, status says why.
    let blocked = dir.path().join("blocked");
    std::fs::write(&blocked, "").unwrap();
    let launcher = DryRunLauncher::new();
    let router = Router::spawn(
        Options {
            journal: Some(JournalConfig::new(&blocked, JournalMode::Hosts)),
            ..Options::default()
        },
        Some(leveled_factory()),
        launcher.clone(),
    )
    .unwrap();
    let handle = router.handle();
    let record = handle
        .dispatch_and_wait(link("https://full.example.com/page?token=hunter2"))
        .unwrap();
    assert_eq!(record.outcome, Outcome::DryRun);
    assert_eq!(launcher.launched().len(), 1);
    assert_eq!(handle.recent().len(), 1);
    let error = handle
        .status()
        .unwrap()
        .journal_error
        .expect("the write failed");
    assert!(
        !error.contains("example.com") && !error.contains("hunter2"),
        "{error}"
    );
}

#[test]
fn set_retention_takes_effect_at_once_and_the_description_follows() {
    let router = Router::spawn(
        Options::default(),
        Some(leveled_factory()),
        DryRunLauncher::new(),
    )
    .unwrap();
    let handle = router.handle();
    for n in 0..5 {
        handle
            .dispatch_and_wait(link(&format!("https://full.example.com/{n}")))
            .unwrap();
    }
    assert_eq!(handle.recent().len(), 5);

    let tighter = Retention { count: 2, hours: 1 };
    handle.set_retention(tighter).unwrap();
    // The buffer dropped the oldest three on the spot, newest first stays.
    let kept: Vec<String> = handle.recent().iter().map(|r| r.link.url.clone()).collect();
    assert_eq!(
        kept,
        ["https://full.example.com/4", "https://full.example.com/3"]
    );
    let retention = handle.retention();
    assert_eq!(retention.memory, tighter);
    assert_eq!(
        retention.description,
        "Kept in memory only: the last 2 links or 1 hour, cleared when PwrRoute quits."
    );
    // Later dispatches honor it.
    handle
        .dispatch_and_wait(link("https://full.example.com/5"))
        .unwrap();
    assert_eq!(handle.recent().len(), 2);
    assert_eq!(handle.recent()[0].link.url, "https://full.example.com/5");

    // Nothing kept: the buffer empties and stays empty.
    handle
        .set_retention(Retention { count: 0, hours: 0 })
        .unwrap();
    assert!(handle.recent().is_empty());
    handle
        .dispatch_and_wait(link("https://full.example.com/6"))
        .unwrap();
    assert!(handle.recent().is_empty());
    assert_eq!(
        handle.retention().description,
        "Nothing is kept: not in memory, and nothing is written to disk."
    );
}

#[test]
fn set_journal_opens_and_closes_the_journal_from_the_next_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let router = Router::spawn(
        Options::default(),
        Some(leveled_factory()),
        DryRunLauncher::new(),
    )
    .unwrap();
    let handle = router.handle();
    handle
        .dispatch_and_wait(link("https://full.example.com/before?token=hunter2"))
        .unwrap();
    assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    assert_eq!(handle.retention().journal, None);

    // On, hosts only: the next dispatch is a line, the earlier one is not.
    let hosts = JournalConfig::new(dir.path().join("journal"), JournalMode::Hosts);
    handle.set_journal(Some(hosts.clone())).unwrap();
    let retention = handle.retention();
    assert_eq!(retention.journal, Some(hosts.clone()));
    assert!(
        retention
            .description
            .contains("journaled as scheme and host only"),
        "{}",
        retention.description
    );
    handle
        .dispatch_and_wait(link("https://full.example.com/during?token=hunter2"))
        .unwrap();
    let journaled = handle.journal(10).unwrap();
    assert_eq!(journaled.len(), 1);
    assert_eq!(journaled[0].privacy, Privacy::Host);
    assert_eq!(journaled[0].link.url, "https://full.example.com");
    assert_eq!(handle.status().unwrap().journal_error, None);

    // Changed to full URLs: the same directory, the next line in full.
    let full = JournalConfig::new(dir.path().join("journal"), JournalMode::Full);
    handle.set_journal(Some(full.clone())).unwrap();
    assert_eq!(handle.retention().journal, Some(full));
    handle
        .dispatch_and_wait(link("https://full.example.com/after?token=hunter2"))
        .unwrap();
    let journaled = handle.journal(10).unwrap();
    assert_eq!(journaled.len(), 2);
    assert_eq!(
        journaled[0].link.url,
        "https://full.example.com/after?token=hunter2"
    );

    // Off: nothing more is written, the files stay, and reading reports
    // nothing (there is no journal to read).
    handle.set_journal(None).unwrap();
    assert_eq!(handle.retention().journal, None);
    assert!(handle.journal(10).unwrap().is_empty());
    handle
        .dispatch_and_wait(link("https://full.example.com/off?token=hunter2"))
        .unwrap();
    let text = std::fs::read_dir(dir.path().join("journal"))
        .unwrap()
        .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
        .collect::<String>();
    assert_eq!(text.lines().count(), 2);
    assert!(!text.contains("/off"));
    assert!(!text.contains("/before"));
    assert_eq!(
        handle.retention().description,
        Options::default().retention_description()
    );
}
