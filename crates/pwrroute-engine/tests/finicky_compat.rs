//! Finicky v4 compatibility: every config in `tests/configs` resolves every URL
//! in `urls.txt` the way the Go host did (see `expected.json`), through both
//! `Engine::resolve` and `Engine::resolve_traced`.

use std::{
    cell::RefCell,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use pwrroute_engine::{
    AppType, ChosenBy, Engine, Host, Limits, LogLevel, ModifierKeys, Opener, RewriteStep,
};
use serde::Deserialize;

fn configs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/configs")
}

fn load(name: &str) -> Engine {
    Engine::load(
        &configs().join(name),
        Rc::new(pwrroute_engine::DefaultHost),
        Limits::default(),
    )
    .unwrap_or_else(|e| panic!("{name}: {e:#}"))
}

#[derive(Deserialize)]
struct Expected {
    configs: std::collections::BTreeMap<String, Vec<Case>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    input: String,
    url: String,
    browser: String,
    profile: String,
    args: Vec<String>,
    open_in_background: bool,
}

#[test]
fn matches_the_go_host() {
    let expected: Expected =
        serde_json::from_str(&fs::read_to_string(configs().join("expected.json")).unwrap())
            .unwrap();
    let mut failures = Vec::new();
    let mut checked = 0;
    for (config, cases) in &expected.configs {
        let engine = load(config);
        for case in cases {
            checked += 1;
            let got = engine
                .resolve(&case.input, None)
                .unwrap_or_else(|e| panic!("{config} {}: {e:#}", case.input));
            // The traced path must give exactly openUrl's result, error text
            // included.
            let traced = engine
                .resolve_traced(&case.input, None)
                .unwrap_or_else(|e| panic!("{config} {} (traced): {e:#}", case.input));
            if traced.resolution != got {
                failures.push(format!(
                    "{config} {}\n  openUrl {got:?}\n  traced  {:?}",
                    case.input, traced.resolution
                ));
            }
            let b = &got.browser;
            let actual = (
                b.url.as_str(),
                b.name.as_str(),
                b.profile.as_str(),
                b.args.as_slice(),
                b.open_in_background.unwrap_or(false),
            );
            let wanted = (
                case.url.as_str(),
                case.browser.as_str(),
                case.profile.as_str(),
                case.args.as_slice(),
                case.open_in_background,
            );
            if actual != wanted {
                failures.push(format!(
                    "{config} {}\n  want {wanted:?}\n  got  {actual:?} {:?}",
                    case.input, got.error
                ));
            }
        }
    }
    assert_eq!(checked, 72, "expected.json lost cases");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn matches_on_the_opener() {
    let engine = load("finicky-example.ts");
    let slack = Opener {
        name: "Slack".into(),
        bundle_id: "com.tinyspeck.slackmacgap".into(),
        ..Opener::default()
    };
    let from_slack = engine
        .resolve("https://example.org/", Some(&slack))
        .unwrap();
    let from_nowhere = engine.resolve("https://example.org/", None).unwrap();
    assert_eq!(from_slack.browser.name, "Firefox");
    assert_eq!(from_nowhere.browser.name, "Google Chrome");
    let traced = engine
        .resolve_traced("https://example.org/", Some(&slack))
        .unwrap();
    assert_eq!(traced.resolution, from_slack);
}

#[test]
fn the_traced_resolve_names_the_rules_that_fired() {
    let engine = load("stress.ts");
    let rewrite = |rule: usize, url: &str| RewriteStep {
        rule,
        url: url.to_owned(),
    };

    // rewrite[0] strips tracking, rewrite[1] upgrades to https, rewrite[3]
    // sends medium.com to scribe.rip, and handlers[6] takes scribe.rip.
    let t = engine
        .resolve_traced("http://blog.medium.com/post?utm_source=x", None)
        .unwrap();
    assert_eq!(
        t.rewrites,
        [
            rewrite(0, "http://blog.medium.com/post"),
            rewrite(1, "https://blog.medium.com/post"),
            rewrite(3, "https://scribe.rip/post"),
        ]
    );
    assert_eq!(t.chosen_by, Some(ChosenBy::Handler { index: 6 }));
    assert_eq!(t.resolution.browser.name, "Firefox");

    let t = engine
        .resolve_traced("https://nothing.example/", None)
        .unwrap();
    assert!(t.rewrites.is_empty());
    assert_eq!(t.chosen_by, Some(ChosenBy::Default));

    // handlers[7] chooses `browser: null`.
    let t = engine
        .resolve_traced("https://printer.local/", None)
        .unwrap();
    assert_eq!(t.chosen_by, Some(ChosenBy::Handler { index: 7 }));
    assert_eq!(t.resolution.browser.app_type, AppType::None);
}

#[test]
fn a_rule_that_throws_is_traced_to_the_default() {
    let path = write_config(
        "throwing-rules.js",
        r#"export default {
  defaultBrowser: "Safari",
  rewrite: [
    { match: "*", url: (url) => url.href.replace("a.example", "b.example") },
    { match: (url) => url.pathname === "/rewrite-throws", url: () => { throw new Error("rewrite failed"); } },
  ],
  handlers: [
    { match: "never", browser: "Opera" },
    { match: (url) => url.pathname === "/browser-throws", browser: () => { throw new Error("browser failed"); } },
    { match: (url) => url.pathname === "/matcher-throws" && missing.value, browser: "Firefox" },
    { match: "*", browser: "Google Chrome" },
  ],
};"#,
    );
    let engine = Engine::load(
        &path,
        Rc::new(pwrroute_engine::DefaultHost),
        Limits::default(),
    )
    .unwrap();
    for (path, error) in [
        ("/rewrite-throws", "rewrite failed"),
        ("/browser-throws", "browser failed"),
        ("/matcher-throws", "missing"),
        ("/fine", ""),
    ] {
        let url = format!("https://a.example{path}");
        let plain = engine.resolve(&url, None).unwrap();
        let traced = engine.resolve_traced(&url, None).unwrap();
        assert_eq!(traced.resolution, plain, "{path}");
        assert_eq!(
            traced.rewrites[0],
            RewriteStep {
                rule: 0,
                url: format!("https://b.example{path}"),
            },
            "{path}"
        );
        if error.is_empty() {
            assert_eq!(traced.chosen_by, Some(ChosenBy::Handler { index: 3 }));
            assert_eq!(traced.resolution.error, None);
        } else {
            assert_eq!(traced.chosen_by, Some(ChosenBy::Default), "{path}");
            assert_eq!(traced.resolution.browser.name, "Safari", "{path}");
            let message = traced.resolution.error.unwrap_or_default();
            assert!(message.contains(error), "{path}: {message}");
        }
    }
}

#[test]
fn the_traced_resolve_runs_each_config_function_as_open_url_does() {
    let path = write_config(
        "logging-rules.js",
        r#"export default {
  defaultBrowser: (url) => { console.log("default", url.href); return "Safari"; },
  rewrite: [
    { match: (url) => { console.log("rewrite 0 match", url.href); return true; }, url: (url) => { console.log("rewrite 0 url"); return url.href + "x"; } },
    { match: (url) => { console.log("rewrite 1 match", url.href); return false; }, url: () => { console.log("rewrite 1 url"); return "https://never.example/"; } },
  ],
  handlers: [
    { match: (url) => { console.log("handler 0 match", url.href); return url.pathname === "/ax"; }, browser: (url) => { console.log("handler 0 browser"); return "Firefox"; } },
    { match: (url) => { console.log("handler 1 match", url.href); return false; }, browser: "Opera" },
  ],
};"#,
    );
    let host = Rc::new(RecordingHost {
        logs: RefCell::default(),
    });
    let engine = Engine::load(&path, host.clone(), Limits::default()).unwrap();
    for url in ["https://a.example/a", "https://a.example/b"] {
        engine.resolve(url, None).unwrap();
        let plain = host.logs.take();
        engine.resolve_traced(url, None).unwrap();
        let traced = host.logs.take();
        assert!(!plain.is_empty());
        assert_eq!(traced, plain, "{url}");
    }
}

#[test]
fn a_config_made_invalid_at_runtime_fails_the_same_way_traced() {
    let path = write_config(
        "self-breaking.js",
        r#"const config = {
  defaultBrowser: "Safari",
  handlers: [{ match: (url) => { if (url.host === "break.example") config.defaultBrowser = 42; return false; }, browser: "Firefox" }],
};
export default config;"#,
    );
    // Each path on its own engine: the first call breaks its config. The
    // matcher sets defaultBrowser to 42, which then fails as a browser; after
    // that the config is invalid before any rule runs.
    let errors = |traced: bool| {
        let engine = Engine::load(
            &path,
            Rc::new(pwrroute_engine::DefaultHost),
            Limits::default(),
        )
        .unwrap();
        ["https://break.example/", "https://other.example/"].map(|url| {
            let err = if traced {
                engine.resolve_traced(url, None).map(|_| ())
            } else {
                engine.resolve(url, None).map(|_| ())
            };
            // The JS stack after the message names different frames.
            let err = err.unwrap_err().to_string();
            err.split("\n    at ").next().unwrap().to_owned()
        })
    };
    let plain = errors(false);
    assert!(plain[0].contains("Invalid browser option"), "{}", plain[0]);
    assert!(plain[1].contains("Invalid config"), "{}", plain[1]);
    assert_eq!(errors(true), plain);
}

#[test]
fn a_frozen_config_traces_as_it_resolves() {
    // A Proxy may not report a different value for a frozen property, so the
    // view must not sit directly on frozen objects.
    let path = write_config(
        "frozen.js",
        r#"const freeze = Object.freeze;
export default freeze({
  defaultBrowser: "Safari",
  rewrite: freeze([freeze({ match: "*/old*", url: "https://example.com/new" })]),
  handlers: freeze([freeze({ match: "*slack.com*", browser: "Firefox" })]),
});"#,
    );
    let engine = Engine::load(
        &path,
        Rc::new(pwrroute_engine::DefaultHost),
        Limits::default(),
    )
    .unwrap();
    for (url, rewrites, chosen_by) in [
        (
            "https://team.slack.com/x",
            0,
            ChosenBy::Handler { index: 0 },
        ),
        ("https://example.com/old", 1, ChosenBy::Default),
    ] {
        let plain = engine.resolve(url, None).unwrap();
        let traced = engine.resolve_traced(url, None).unwrap();
        assert_eq!(traced.resolution, plain, "{url}");
        assert_eq!(traced.rewrites.len(), rewrites, "{url}");
        assert_eq!(traced.chosen_by, Some(chosen_by), "{url}");
    }
}

#[test]
fn the_traced_resolve_honours_the_deadline() {
    let path = write_config(
        "spin-traced.js",
        r#"export default {
  defaultBrowser: "Safari",
  handlers: [
    { match: (url) => url.host === "spin.example", browser: () => { while (true) {} } },
  ],
};"#,
    );
    let limits = Limits {
        resolve: Duration::from_millis(200),
        ..Limits::default()
    };
    let engine = Engine::load(&path, Rc::new(pwrroute_engine::DefaultHost), limits).unwrap();
    let err = engine
        .resolve_traced("https://spin.example/", None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("timed out"), "{err}");
    let ok = engine
        .resolve_traced("https://other.example/", None)
        .unwrap();
    assert_eq!(ok.chosen_by, Some(ChosenBy::Default));
}

#[test]
fn null_browser_means_do_not_open() {
    let engine = load("stress.ts");
    let r = engine.resolve("https://printer.local/", None).unwrap();
    assert_eq!(r.browser.app_type, AppType::None);
}

struct RecordingHost {
    logs: RefCell<Vec<(LogLevel, String)>>,
}

impl Host for RecordingHost {
    fn modifier_keys(&self) -> ModifierKeys {
        ModifierKeys {
            shift: true,
            ..ModifierKeys::default()
        }
    }

    fn log(&self, level: LogLevel, message: &str) {
        self.logs.borrow_mut().push((level, message.to_owned()));
    }
}

#[test]
fn host_functions_and_console_reach_the_host() {
    let host = Rc::new(RecordingHost {
        logs: RefCell::default(),
    });
    let engine = Engine::load(
        &configs().join("finicky-example.js"),
        host.clone(),
        Limits::default(),
    )
    .unwrap();
    engine.resolve("https://example.org/", None).unwrap();
    let logs = host.logs.borrow();
    assert!(
        logs.iter()
            .any(|(level, m)| *level == LogLevel::Info && m.contains(r#""shift":true"#)),
        "{logs:?}"
    );
}

fn write_config(name: &str, source: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pwrroute-engine-test-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, source).unwrap();
    path
}

#[test]
fn a_config_that_never_finishes_loading_times_out() {
    let path = write_config("spin-load.js", "while (true) {}\nexport default {};");
    let limits = Limits {
        load: Duration::from_millis(200),
        ..Limits::default()
    };
    let err = Engine::load(&path, Rc::new(pwrroute_engine::DefaultHost), limits)
        .unwrap_err()
        .to_string();
    assert!(err.contains("timed out"), "{err}");
}

#[test]
fn a_matcher_that_never_returns_times_out_and_the_engine_survives() {
    let path = write_config(
        "spin-match.js",
        r#"export default {
  defaultBrowser: "Safari",
  handlers: [
    { match: (url) => { if (url.host === "spin.example") { while (true) {} } return false; }, browser: "Firefox" },
  ],
};"#,
    );
    let limits = Limits {
        resolve: Duration::from_millis(200),
        ..Limits::default()
    };
    let engine = Engine::load(&path, Rc::new(pwrroute_engine::DefaultHost), limits).unwrap();
    let err = engine
        .resolve("https://spin.example/", None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("timed out"), "{err}");
    let ok = engine.resolve("https://other.example/", None).unwrap();
    assert_eq!(ok.browser.name, "Safari");
}

#[test]
fn an_invalid_config_is_rejected_with_a_reason() {
    let path = write_config("invalid.js", "export default { handlers: 42 };");
    let err = Engine::load(
        &path,
        Rc::new(pwrroute_engine::DefaultHost),
        Limits::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.starts_with("validating the config:"), "{err}");
}

#[test]
fn bare_imports_are_refused() {
    let path = write_config(
        "bare-import.js",
        "import x from \"lodash\";\nexport default { defaultBrowser: x };",
    );
    let err = Engine::load(
        &path,
        Rc::new(pwrroute_engine::DefaultHost),
        Limits::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("lodash"), "{err}");
}
