use std::{
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

use pwrroute_engine::{
    DefaultHost, Limits, Method, Reply, Rewrite, RewriteAction, Rule, RulesFile, Sources,
};

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
    { match: "https://*.chat.example.com/*", browser: "Firefox" },
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
        .dispatch_and_wait(link("https://team.chat.example.com/archives/1"))
        .unwrap();
    assert_eq!(record.outcome, Outcome::DryRun);
    let browser = record.browser.clone().unwrap();
    assert_eq!(browser.name, "Firefox");
    assert_eq!(browser.url, "https://team.chat.example.com/archives/1");
    assert_eq!(
        record.steps,
        vec![
            Step::Received {
                url: "https://team.chat.example.com/archives/1".into()
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
                by: Matched::Default {
                    owner: Some(Owner::Config)
                }
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
            by: Matched::Default {
                owner: Some(Owner::Config)
            }
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
        .test(link("https://team.chat.example.com/x"))
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
        fallback: FallbackSource::fixed("Fallback Browser", ""),
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

/// The macOS link helper is PwrRoute too: a rule, a fallback or a pause
/// that names it, by bundle id, name or path, would hand the link back.
#[test]
fn the_link_helper_is_pwrroute() {
    for name in [
        "PwrRoute",
        "com.pwrdrvr.pwrroute",
        "com.pwrdrvr.pwrroute.links",
        "/Applications/PwrRoute.app/Contents/Helpers/PwrRoute.app",
    ] {
        assert!(is_self_name(name), "{name}");
    }
    assert!(!is_self_name("com.pwrdrvr.pwrfinicky"));
    let fallback = FallbackSource::fixed("com.pwrdrvr.pwrroute.links", "Work");
    assert_eq!(fallback.get(), (FALLBACK_BROWSER.to_owned(), String::new()));
}

fn pause_target() -> PauseTarget {
    PauseTarget {
        browser: "Google Chrome".into(),
        profile: "Personal".into(),
    }
}

#[test]
fn paused_every_link_opens_in_the_pause_target_and_the_skipped_handler_is_traced() {
    let (_dir, router, launcher) = engine_router(Options::default());
    let handle = router.handle();
    assert_eq!(handle.paused(), None);
    handle.set_paused(Some(pause_target()));
    assert_eq!(handle.paused(), Some(pause_target()));

    let record = handle
        .dispatch_and_wait(link("https://team.chat.example.com/archives/1"))
        .unwrap();
    assert_eq!(record.outcome, Outcome::DryRun);
    let browser = record.browser.clone().unwrap();
    assert_eq!(browser.name, "Google Chrome");
    assert_eq!(browser.profile, "Personal");
    assert_eq!(browser.url, "https://team.chat.example.com/archives/1");
    assert_eq!(
        record.steps,
        vec![
            Step::Received {
                url: "https://team.chat.example.com/archives/1".into()
            },
            Step::Matched {
                by: Matched::Handler { index: 0 }
            },
            Step::Paused { skipped: Some(0) },
            Step::Browser { browser },
            Step::Launched { dry_run: true },
        ]
    );
    assert_eq!(launcher.launched()[0].name, "Google Chrome");
    assert_eq!(launcher.launched()[0].profile, "Personal");

    // Resumed: the handler routes again, and nothing says paused.
    handle.set_paused(None);
    assert_eq!(handle.paused(), None);
    let record = handle
        .dispatch_and_wait(link("https://team.chat.example.com/archives/2"))
        .unwrap();
    assert_eq!(record.browser.unwrap().name, "Firefox");
    assert!(
        !record
            .steps
            .iter()
            .any(|s| matches!(s, Step::Paused { .. }))
    );
    assert_eq!(launcher.launched()[1].name, "Firefox");
}

#[test]
fn paused_rewrites_still_run_and_the_default_skips_no_handler() {
    let (_dir, router, launcher) = engine_router(Options::default());
    let handle = router.handle();
    handle.set_paused(Some(pause_target()));
    let record = handle
        .dispatch_and_wait(link("https://example.com/old/page"))
        .unwrap();
    assert_eq!(record.final_url.as_deref(), Some("https://example.com/new"));
    assert_eq!(
        record.steps[1..4],
        [
            Step::Rewrite {
                rule: Some(0),
                url: "https://example.com/new".into()
            },
            Step::Matched {
                by: Matched::Default {
                    owner: Some(Owner::Config)
                }
            },
            Step::Paused { skipped: None },
        ]
    );
    assert_eq!(launcher.launched()[0].url, "https://example.com/new");
    assert_eq!(launcher.launched()[0].name, "Google Chrome");
}

#[test]
fn paused_a_browser_null_handler_is_skipped_too() {
    let (_dir, router, launcher) = engine_router(Options::default());
    let handle = router.handle();
    handle.set_paused(Some(pause_target()));
    let record = handle
        .dispatch_and_wait(link("https://blocked.example.com/ad"))
        .unwrap();
    assert_eq!(record.outcome, Outcome::DryRun);
    assert!(record.steps.contains(&Step::Paused { skipped: Some(2) }));
    assert!(!record.steps.contains(&Step::NotOpened));
    assert_eq!(record.browser.unwrap().name, "Google Chrome");
    assert_eq!(launcher.launched()[0].url, "https://blocked.example.com/ad");
}

#[test]
fn paused_a_private_rule_opens_a_normal_window_and_still_keeps_the_host() {
    let (router, launcher) = rules_router(leveled_rules());
    let handle = router.handle();
    handle.set_paused(Some(pause_target()));
    let secret = "https://private.example.com/inbox?token=hunter2";
    let reply = handle.dispatch_and_wait(link(secret)).unwrap();
    // The launcher opens the full URL in the pause target, not privately.
    assert_eq!(launcher.launched()[0].url, secret);
    assert_eq!(launcher.launched()[0].name, "Google Chrome");
    assert!(!launcher.launched()[0].private);
    // A rules.json handler has no number to skip.
    assert!(reply.steps.contains(&Step::Paused { skipped: None }));
    // The rule's level stands: the record keeps the host only.
    let stored = handle.get(reply.id).unwrap();
    for record in [&reply, &stored] {
        assert_eq!(record.privacy, Privacy::Host);
        assert_eq!(record.link.url, "https://private.example.com");
        let browser = record.browser.as_ref().unwrap();
        assert!(!browser.private);
        assert_eq!(browser.privacy, Privacy::Host);
        assert_eq!(browser.url, "https://private.example.com");
        let json = serde_json::to_string(record).unwrap();
        for private in ["hunter2", "/inbox", "token"] {
            assert!(!json.contains(private), "{private} leaked into {json}");
        }
    }
}

#[test]
fn paused_test_a_link_still_answers_by_the_rules() {
    let (_dir, router, launcher) = engine_router(Options::default());
    let handle = router.handle();
    handle.set_paused(Some(pause_target()));
    let record = handle
        .test(link("https://team.chat.example.com/x"))
        .unwrap();
    assert_eq!(record.outcome, Outcome::Tested);
    assert_eq!(record.browser.unwrap().name, "Firefox");
    assert!(
        !record
            .steps
            .iter()
            .any(|s| matches!(s, Step::Paused { .. }))
    );
    assert!(launcher.launched().is_empty());
}

#[test]
fn paused_with_no_config_the_pause_target_takes_the_fallbacks_links() {
    let launcher = DryRunLauncher::new();
    let router = Router::spawn(Options::default(), None, launcher.clone()).unwrap();
    let handle = router.handle();
    handle.set_paused(Some(pause_target()));
    let record = handle
        .dispatch_and_wait(link("https://example.com/"))
        .unwrap();
    assert!(record.steps.contains(&Step::Fallback {
        reason: "no config is selected".into()
    }));
    assert!(record.steps.contains(&Step::Paused { skipped: None }));
    assert_eq!(launcher.launched()[0].name, "Google Chrome");
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
    fn resolve(&self, _: &EngineLink, _: Option<&Opener>) -> Result<Resolved> {
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
    fn resolve(&self, link: &EngineLink, _: Option<&Opener>) -> Result<Resolved> {
        let url = &link.url;
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

/// Fails every browser whose name is in `missing`, as `open` does for an app
/// that is not installed, and records every attempt.
#[derive(Clone)]
struct Broken {
    missing: Vec<&'static str>,
    attempts: Arc<Mutex<Vec<Browser>>>,
}

impl Broken {
    fn new(missing: &[&'static str]) -> Self {
        Self {
            missing: missing.to_vec(),
            attempts: Arc::default(),
        }
    }

    fn attempts(&self) -> Vec<Browser> {
        self.attempts.lock().unwrap().clone()
    }
}

impl Launcher for Broken {
    fn launch(&mut self, browser: &Browser) -> Result<()> {
        self.attempts.lock().unwrap().push(browser.clone());
        if self.missing.contains(&browser.name.as_str()) {
            Err(anyhow!("browser {:?} is not installed", browser.name))
        } else {
            Ok(())
        }
    }
}

#[test]
fn a_browser_that_cannot_open_hands_the_link_to_the_fallback() {
    let launcher = Broken::new(&["Firefox"]);
    let dir = tempfile::tempdir().unwrap();
    let router = Router::spawn(
        Options {
            fallback: FallbackSource::fixed("Safari", ""),
            ..Options::default()
        },
        Some(engine_factory(&dir)),
        launcher.clone(),
    )
    .unwrap();
    let record = router
        .handle()
        .dispatch_and_wait(link("https://team.chat.example.com/x"))
        .unwrap();
    assert_eq!(record.outcome, Outcome::Opened);
    // The handler chose Firefox; the record names what opened the link.
    let opened = record.browser.clone().unwrap();
    assert_eq!(
        (opened.name.as_str(), opened.app_type),
        ("Safari", AppType::AppName)
    );
    assert_eq!(opened.url, "https://team.chat.example.com/x");
    assert_eq!(
        record.steps[3..],
        [
            Step::LaunchFailed {
                message: "browser \"Firefox\" is not installed".into()
            },
            Step::Fallback {
                reason: "Firefox could not be opened".into()
            },
            Step::Launched { dry_run: false },
        ]
    );
    // The handler's choice stays in the trace.
    assert!(matches!(&record.steps[2], Step::Browser { browser } if browser.name == "Firefox"));
    let attempts = launcher.attempts();
    let attempts: Vec<&str> = attempts.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(attempts, ["Firefox", "Safari"]);
}

#[test]
fn when_the_fallback_fails_too_the_dispatch_fails_without_a_loop() {
    let launcher = Broken::new(&["Firefox", "Safari"]);
    let dir = tempfile::tempdir().unwrap();
    let router = Router::spawn(
        Options {
            fallback: FallbackSource::fixed("Safari", ""),
            ..Options::default()
        },
        Some(engine_factory(&dir)),
        launcher.clone(),
    )
    .unwrap();
    let record = router
        .handle()
        .dispatch_and_wait(link("https://team.chat.example.com/x"))
        .unwrap();
    assert_eq!(record.outcome, Outcome::Failed);
    assert_eq!(record.browser.unwrap().name, "Firefox");
    assert_eq!(
        record.steps[3..],
        [
            Step::LaunchFailed {
                message: "browser \"Firefox\" is not installed".into()
            },
            Step::Fallback {
                reason: "Firefox could not be opened".into()
            },
            Step::LaunchFailed {
                message: "browser \"Safari\" is not installed".into()
            },
        ]
    );
    assert_eq!(launcher.attempts().len(), 2);
}

#[test]
fn the_fallback_itself_failing_is_not_retried() {
    let launcher = Broken::new(&["Safari"]);
    let router = Router::spawn(
        Options {
            fallback: FallbackSource::fixed("Safari", ""),
            ..Options::default()
        },
        None,
        launcher.clone(),
    )
    .unwrap();
    let record = router
        .handle()
        .dispatch_and_wait(link("https://example.com/"))
        .unwrap();
    assert_eq!(record.outcome, Outcome::Failed);
    assert_eq!(
        record.steps.last(),
        Some(&Step::LaunchFailed {
            message: "browser \"Safari\" is not installed".into()
        })
    );
    assert_eq!(
        record
            .steps
            .iter()
            .filter(|s| matches!(s, Step::Fallback { .. }))
            .count(),
        1,
        "only the no-config fallback, not a launch fallback: {:?}",
        record.steps
    );
    assert_eq!(launcher.attempts().len(), 1);
}

#[test]
fn the_fallback_is_asked_for_each_time_a_link_needs_it() {
    // The app saves a new choice (first run, an edit of rules.json) while
    // the router runs: the next link that falls back follows it.
    // Every link resolves to Brave, which is never the OS's own browser
    // (on Linux that is Firefox, which the config would pick).
    let choice: Arc<Mutex<Option<(String, String)>>> = Arc::default();
    let launcher = Broken::new(&["Brave"]);
    let router = Router::spawn(
        Options {
            fallback: FallbackSource::new({
                let choice = Arc::clone(&choice);
                move || choice.lock().unwrap().clone()
            }),
            ..Options::default()
        },
        Some(Box::new(|| Ok(Box::new(ToBrave) as Box<dyn Resolver>))),
        launcher.clone(),
    )
    .unwrap();
    let handle = router.handle();
    let opened = |handle: &RouterHandle| {
        let record = handle
            .dispatch_and_wait(link("https://team.chat.example.com/x"))
            .unwrap();
        assert_eq!(record.outcome, Outcome::Opened, "{:?}", record.steps);
        let browser = record.browser.unwrap();
        (browser.name, browser.app_type, browser.profile)
    };
    assert_eq!(
        opened(&handle),
        (FALLBACK_BROWSER.to_owned(), AppType::AppName, String::new())
    );
    *choice.lock().unwrap() = Some(("com.google.Chrome".into(), "Work".into()));
    assert_eq!(
        opened(&handle),
        (
            "com.google.Chrome".to_owned(),
            AppType::BundleId,
            "Work".to_owned()
        )
    );
    // Cleared again, or naming PwrRoute: the OS's own browser.
    *choice.lock().unwrap() = Some(("com.pwrdrvr.pwrroute".into(), "Work".into()));
    assert_eq!(
        opened(&handle),
        (FALLBACK_BROWSER.to_owned(), AppType::AppName, String::new())
    );
    *choice.lock().unwrap() = Some(("  ".into(), String::new()));
    assert_eq!(opened(&handle).0, FALLBACK_BROWSER);
}

/// Routes every link to Brave, a browser no OS has as its own.
struct ToBrave;
impl Resolver for ToBrave {
    fn resolve(&self, link: &EngineLink, _: Option<&Opener>) -> Result<Resolved> {
        Ok(Resolved {
            browser: Browser {
                name: "Brave".into(),
                app_type: AppType::AppName,
                url: link.url.clone(),
                profile: String::new(),
                args: Vec::new(),
                open_in_background: None,
                private: false,
                privacy: Privacy::Full,
            },
            error: None,
            privacy: Privacy::Full,
            rule_steps: Vec::new(),
        })
    }
}

/// Routes every link to a private Firefox window.
struct PrivateFirefox;
impl Resolver for PrivateFirefox {
    fn resolve(&self, link: &EngineLink, _: Option<&Opener>) -> Result<Resolved> {
        Ok(Resolved {
            browser: Browser {
                name: "Firefox".into(),
                app_type: AppType::AppName,
                url: link.url.clone(),
                profile: String::new(),
                args: Vec::new(),
                open_in_background: None,
                private: true,
                privacy: Privacy::Full,
            },
            error: None,
            privacy: Privacy::Full,
            rule_steps: Vec::new(),
        })
    }
}

#[test]
fn a_private_link_stays_private_in_the_fallback() {
    let launcher = Broken::new(&["Firefox"]);
    let factory: ResolverFactory = Box::new(|| Ok(Box::new(PrivateFirefox) as Box<dyn Resolver>));
    let router = Router::spawn(
        Options {
            fallback: FallbackSource::fixed("Safari", ""),
            ..Options::default()
        },
        Some(factory),
        launcher.clone(),
    )
    .unwrap();
    let record = router
        .handle()
        .dispatch_and_wait(link("https://example.com/"))
        .unwrap();
    assert_eq!(record.outcome, Outcome::Opened);
    let attempts = launcher.attempts();
    assert_eq!(
        attempts
            .iter()
            .map(|b| (b.name.as_str(), b.private))
            .collect::<Vec<_>>(),
        [("Firefox", true), ("Safari", true)]
    );
    assert!(record.browser.unwrap().private);
}

#[test]
fn the_fallback_browser_and_profile_come_from_the_options() {
    let launcher = DryRunLauncher::new();
    let router = Router::spawn(
        Options {
            fallback: FallbackSource::fixed("com.google.Chrome", "Work"),
            ..Options::default()
        },
        None,
        launcher.clone(),
    )
    .unwrap();
    router
        .handle()
        .dispatch_and_wait(link("https://example.com/"))
        .unwrap();
    let launched = &launcher.launched()[0];
    assert_eq!(launched.name, "com.google.Chrome");
    assert_eq!(launched.app_type, AppType::BundleId);
    assert_eq!(launched.profile, "Work");
    assert_eq!(app_type_of("Google Chrome"), AppType::AppName);
    assert_eq!(app_type_of("org.mozilla.firefox"), AppType::BundleId);
    assert_eq!(app_type_of("/Applications/Arc.app"), AppType::Path);
    assert_eq!(app_type_of("~/Applications/Arc.app"), AppType::Path);
    assert_eq!(app_type_of(""), AppType::AppName);
    // Neither a bundle id nor an .app path: Finicky calls it an app name.
    assert_eq!(app_type_of("Brave Browser (Beta)"), AppType::AppName);
    assert_eq!(app_type_of("Chrome_Canary"), AppType::AppName);
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
        .dispatch_and_wait(link("https://team.chat.example.com/x"))
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
    fn resolve(&self, link: &EngineLink, _: Option<&Opener>) -> Result<Resolved> {
        Ok(Resolved {
            browser: Browser {
                name: "Watched Browser".into(),
                app_type: AppType::AppName,
                url: link.url.clone(),
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
    fn resolve(&self, link: &EngineLink, _: Option<&Opener>) -> Result<Resolved> {
        let url = &link.url;
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
fn the_default_names_its_owner_and_a_rules_json_handler_its_place() {
    // A JS config's default is the config's (the tests above); with
    // rules.json alone, its default is PwrRoute's.
    let (router, _launcher) = rules_router(leveled_rules());
    let handle = router.handle();
    let record = handle
        .dispatch_and_wait(link("https://unmatched.example.com/"))
        .unwrap();
    assert!(record.steps.contains(&Step::Matched {
        by: Matched::Default {
            owner: Some(Owner::PwrRoute)
        }
    }));
    // A rules.json handler is numbered by its place in the file.
    let record = handle
        .dispatch_and_wait(link("https://full.example.com/"))
        .unwrap();
    assert!(record.steps.contains(&Step::Matched {
        by: Matched::Rule { index: 4 }
    }));
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

/// A shortener that answers from a table and counts what it was asked;
/// anything else is offline. `Send + Sync`, as the router's network must be.
#[derive(Default)]
struct FakeShortener {
    redirects: Mutex<Vec<(String, String)>>,
    asked: Mutex<Vec<String>>,
}

impl FakeShortener {
    fn redirect(self, from: &str, to: &str) -> Self {
        self.redirects
            .lock()
            .unwrap()
            .push((from.to_owned(), to.to_owned()));
        self
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl Network for FakeShortener {
    fn request(&self, method: Method, url: &str, _timeout: Duration) -> Result<Reply> {
        self.asked.lock().unwrap().push(format!("{method:?} {url}"));
        let redirects = self.redirects.lock().unwrap();
        if let Some((_, to)) = redirects.iter().find(|(from, _)| from == url) {
            return Ok(Reply {
                status: 301,
                location: Some(to.clone()),
            });
        }
        if redirects.iter().any(|(_, to)| to == url) {
            return Ok(Reply {
                status: 200,
                location: None,
            });
        }
        Err(anyhow!("offline"))
    }
}

/// `rules.json` alone: the engine's built-in shortener list is in force,
/// and `github.com` goes to Firefox.
fn github_rules() -> RulesFile {
    RulesFile {
        default_browser: "Safari".into(),
        rules: vec![
            Rule {
                patterns: vec!["github.com/*".into()],
                browser: "Firefox".into(),
                ..Rule::default()
            },
            Rule {
                patterns: vec!["host.example.com/*".into()],
                browser: "Firefox".into(),
                privacy: Some(Privacy::Host.into()),
                ..Rule::default()
            },
        ],
        ..RulesFile::default()
    }
}

fn shortener_router(rules: RulesFile, network: Arc<FakeShortener>) -> (Router, DryRunLauncher) {
    let launcher = DryRunLauncher::new();
    let router = Router::spawn(
        Options {
            network: Some(network),
            ..Options::default()
        },
        Some(rules_factory(rules)),
        launcher.clone(),
    )
    .unwrap();
    (router, launcher)
}

#[test]
fn a_short_link_is_routed_by_where_it_lands_and_the_record_keeps_both() {
    let net = Arc::new(
        FakeShortener::default().redirect("https://bit.ly/gh", "https://github.com/pwrdrvr/x"),
    );
    let (router, launcher) = shortener_router(github_rules(), Arc::clone(&net));
    let handle = router.handle();
    let record = handle.dispatch_and_wait(link("https://bit.ly/gh")).unwrap();
    assert_eq!(record.outcome, Outcome::DryRun);
    let browser = record.browser.clone().unwrap();
    assert_eq!(browser.name, "Firefox", "{:?}", record.steps);
    assert_eq!(browser.url, "https://github.com/pwrdrvr/x");
    assert_eq!(launcher.launched()[0].url, "https://github.com/pwrdrvr/x");
    // The record shows the short link and where it landed.
    assert_eq!(record.link.url, "https://bit.ly/gh");
    assert_eq!(
        record.final_url.as_deref(),
        Some("https://github.com/pwrdrvr/x")
    );
    assert_eq!(
        record.steps,
        vec![
            Step::Received {
                url: "https://bit.ly/gh".into()
            },
            Step::Expanded {
                url: "https://github.com/pwrdrvr/x".into()
            },
            Step::Matched {
                by: Matched::Rule { index: 0 }
            },
            Step::Browser { browser },
            Step::Launched { dry_run: true },
        ]
    );
    assert_eq!(record.timing().rewrites, 0, "an expansion is not a rewrite");
    // Only the shortener and its target were asked, HEAD first.
    assert_eq!(
        net.asked(),
        [
            "Head https://bit.ly/gh",
            "Head https://github.com/pwrdrvr/x"
        ]
    );

    // Test a link expands the same way, and makes the same requests.
    let tested = handle.test(link("https://bit.ly/gh")).unwrap();
    assert_eq!(tested.outcome, Outcome::Tested);
    assert_eq!(tested.browser.unwrap().name, "Firefox");
    assert!(tested.steps.contains(&Step::Expanded {
        url: "https://github.com/pwrdrvr/x".into()
    }));
    assert_eq!(net.asked().len(), 4);

    // A link that is not short is never requested, and has no step.
    let record = handle
        .dispatch_and_wait(link("https://github.com/direct"))
        .unwrap();
    assert!(
        !record
            .steps
            .iter()
            .any(|s| matches!(s, Step::Expanded { .. }))
    );
    assert_eq!(net.asked().len(), 4);
}

#[test]
fn an_unanswered_short_link_goes_to_the_rules_as_it_is() {
    let net = Arc::new(FakeShortener::default());
    let (router, launcher) = shortener_router(github_rules(), Arc::clone(&net));
    let record = router
        .handle()
        .dispatch_and_wait(link("https://bit.ly/offline"))
        .unwrap();
    assert_eq!(record.outcome, Outcome::DryRun);
    assert_eq!(record.browser.unwrap().name, "Safari");
    assert_eq!(launcher.launched()[0].url, "https://bit.ly/offline");
    assert!(
        !record
            .steps
            .iter()
            .any(|s| matches!(s, Step::Expanded { .. }))
    );
    // HEAD, then GET, and nothing else.
    assert_eq!(
        net.asked(),
        ["Head https://bit.ly/offline", "Get https://bit.ly/offline"]
    );
}

#[test]
fn a_host_rule_withholds_the_short_link_and_where_it_landed_alike() {
    let secret = "https://host.example.com/reset?token=hunter2";
    let net = Arc::new(FakeShortener::default().redirect("https://bit.ly/s3cr3t", secret));
    let (router, launcher) = shortener_router(github_rules(), net);
    let handle = router.handle();
    let reply = handle
        .dispatch_and_wait(link("https://bit.ly/s3cr3t").with_opener(Some(secret_opener())))
        .unwrap();
    // The browser got the real destination, by the host rule.
    assert_eq!(launcher.launched()[0].url, secret);
    assert_eq!(launcher.launched()[0].name, "Firefox");

    let stored = handle.get(reply.id).unwrap();
    for record in [&reply, &stored] {
        assert_eq!(record.privacy, Privacy::Host);
        assert_eq!(record.link.url, "https://bit.ly");
        assert_eq!(
            record.final_url.as_deref(),
            Some("https://host.example.com")
        );
        assert_eq!(
            record.steps[..2],
            [
                Step::Received {
                    url: "https://bit.ly".into()
                },
                Step::Expanded {
                    url: "https://host.example.com".into()
                },
            ]
        );
        let json = serde_json::to_string(record).unwrap();
        for private in ["s3cr3t", "hunter2", "/reset", "token"] {
            assert!(!json.contains(private), "{private} leaked into {json}");
        }
    }
}

#[test]
fn nothing_is_requested_when_no_config_is_in_charge_or_expansion_is_off() {
    let net = Arc::new(
        FakeShortener::default().redirect("https://bit.ly/gh", "https://github.com/pwrdrvr/x"),
    );
    // No config: the fallback takes the short link as it is.
    let launcher = DryRunLauncher::new();
    let router = Router::spawn(
        Options {
            network: Some(Arc::clone(&net) as Arc<dyn Network + Send + Sync>),
            ..Options::default()
        },
        None,
        launcher.clone(),
    )
    .unwrap();
    let record = router
        .handle()
        .dispatch_and_wait(link("https://bit.ly/gh"))
        .unwrap();
    assert_eq!(launcher.launched()[0].url, "https://bit.ly/gh");
    assert!(record.steps.contains(&Step::Fallback {
        reason: "no config is selected".into()
    }));
    assert!(net.asked().is_empty());

    // A config whose `urlShorteners: []` turned expansion off.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("finicky.js");
    std::fs::write(
        &path,
        r#"export default {
  defaultBrowser: "Safari",
  options: { urlShorteners: [] },
  handlers: [{ match: "github.com/*", browser: "Firefox" }],
};"#,
    )
    .unwrap();
    let factory: ResolverFactory = Box::new(move || {
        let engine = Engine::load(&path, Rc::new(DefaultHost), Limits::default())?;
        Ok(Box::new(engine) as Box<dyn Resolver>)
    });
    let launcher = DryRunLauncher::new();
    let router = Router::spawn(
        Options {
            network: Some(Arc::clone(&net) as Arc<dyn Network + Send + Sync>),
            ..Options::default()
        },
        Some(factory),
        launcher.clone(),
    )
    .unwrap();
    let record = router
        .handle()
        .dispatch_and_wait(link("https://bit.ly/gh"))
        .unwrap();
    assert_eq!(record.browser.unwrap().name, "Safari");
    assert_eq!(launcher.launched()[0].url, "https://bit.ly/gh");
    assert!(net.asked().is_empty());
}

// Precedence (WP-A): the record says who decided, whichever order
// rules.json's `precedence` runs the two lists in.

/// [`CONFIG`] with `rules`, as the router's `Reloader` loads them together.
fn config_and_rules_router(rules: RulesFile) -> (tempfile::TempDir, Router, DryRunLauncher) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("finicky.js");
    std::fs::write(&path, CONFIG).unwrap();
    let factory: ResolverFactory = Box::new(move || {
        let sources = Sources {
            config: Some(path),
            rules,
        };
        let engine = Engine::from_sources(&sources, Rc::new(DefaultHost), Limits::default())?;
        Ok(Box::new(engine) as Box<dyn Resolver>)
    });
    let launcher = DryRunLauncher::new();
    let router = Router::spawn(Options::default(), Some(factory), launcher.clone()).unwrap();
    (dir, router, launcher)
}

/// rules.json beside [`CONFIG`]: a switched-off rule, a rule for the
/// config's chat links and one for its own host, and Vivaldi · Work for
/// everything else.
fn beside_the_config(precedence: pwrroute_engine::Precedence) -> RulesFile {
    let handler = |pattern: &str, browser: &str| Rule {
        patterns: vec![pattern.to_owned()],
        browser: browser.into(),
        ..Rule::default()
    };
    RulesFile {
        default_browser: "Vivaldi".into(),
        default_profile: "Work".into(),
        precedence,
        rules: vec![
            Rule {
                enabled: false,
                ..handler("*", "Safari")
            },
            handler("https://*.chat.example.com/*", "Arc"),
            handler("https://rules.example.com/*", "Arc"),
        ],
        ..RulesFile::default()
    }
}

fn matched(record: &DispatchRecord) -> Option<Matched> {
    record.steps.iter().find_map(|step| match step {
        Step::Matched { by } => Some(*by),
        _ => None,
    })
}

#[test]
fn the_go_order_names_config_handlers_rules_and_the_configs_default() {
    let (_dir, router, launcher) =
        config_and_rules_router(beside_the_config(pwrroute_engine::Precedence::Config));
    let handle = router.handle();
    let decided = |url: &str| matched(&handle.dispatch_and_wait(link(url)).unwrap());
    // The config's chat handler runs first and takes it.
    assert_eq!(
        decided("https://team.chat.example.com/a"),
        Some(Matched::Handler { index: 0 })
    );
    assert_eq!(
        decided("https://rules.example.com/a"),
        Some(Matched::Rule { index: 2 })
    );
    assert_eq!(
        decided("https://unmatched.example.com/"),
        Some(Matched::Default {
            owner: Some(Owner::Config)
        })
    );
    let names: Vec<String> = launcher.launched().iter().map(|b| b.name.clone()).collect();
    assert_eq!(names, ["Firefox", "Arc", "Safari"]);
}

#[test]
fn pwrroute_precedence_names_its_rules_first_and_its_own_default() {
    let (_dir, router, launcher) =
        config_and_rules_router(beside_the_config(pwrroute_engine::Precedence::PwrRoute));
    let handle = router.handle();
    let decided = |url: &str| matched(&handle.dispatch_and_wait(link(url)).unwrap());
    // rules.json's chat rule now runs before the config's handler.
    assert_eq!(
        decided("https://team.chat.example.com/a"),
        Some(Matched::Rule { index: 1 })
    );
    // A config handler that matches still decides, `browser: null` too.
    assert_eq!(
        decided("https://blocked.example.com/a"),
        Some(Matched::Handler { index: 2 })
    );
    // Everything else is PwrRoute's: so is a config matcher that throws.
    let pwrroute = Some(Matched::Default {
        owner: Some(Owner::PwrRoute),
    });
    assert_eq!(decided("https://unmatched.example.com/"), pwrroute);
    let threw = handle
        .dispatch_and_wait(link("https://throws.example.com/"))
        .unwrap();
    assert_eq!(matched(&threw), pwrroute);
    let launched: Vec<(String, String)> = launcher
        .launched()
        .iter()
        .map(|b| (b.name.clone(), b.profile.clone()))
        .collect();
    assert_eq!(
        launched,
        [
            ("Arc".to_owned(), String::new()),
            ("Vivaldi".to_owned(), "Work".to_owned()),
            ("Vivaldi".to_owned(), "Work".to_owned()),
        ]
    );
}

#[test]
fn paused_a_skipped_rules_json_rule_is_the_records_own_rule() {
    let (_dir, router, launcher) =
        config_and_rules_router(beside_the_config(pwrroute_engine::Precedence::PwrRoute));
    let handle = router.handle();
    handle.set_paused(Some(pause_target()));
    let record = handle
        .dispatch_and_wait(link("https://team.chat.example.com/a"))
        .unwrap();
    // The rule that would have taken it, then the pause: `skipped` numbers
    // config handlers only, so it is None and the rule says which.
    assert_eq!(
        record.steps[1..3],
        [
            Step::Matched {
                by: Matched::Rule { index: 1 }
            },
            Step::Paused { skipped: None },
        ]
    );
    // A config handler passed over is still numbered.
    let record = handle
        .dispatch_and_wait(link("https://blocked.example.com/a"))
        .unwrap();
    assert!(record.steps.contains(&Step::Paused { skipped: Some(2) }));
    assert!(
        launcher
            .launched()
            .iter()
            .all(|b| b.name == "Google Chrome")
    );
}

#[test]
fn skipped_handler_numbers_only_the_configs_handlers() {
    let with = |by| vec![Step::Matched { by }];
    assert_eq!(
        skipped_handler(&with(Matched::Handler { index: 3 })),
        Some(3)
    );
    for by in [
        Matched::Rule { index: 3 },
        Matched::Default {
            owner: Some(Owner::PwrRoute),
        },
        Matched::Default { owner: None },
        Matched::Unreported,
    ] {
        assert_eq!(skipped_handler(&with(by)), None, "{by:?}");
    }
    assert_eq!(skipped_handler(&[]), None);
}
