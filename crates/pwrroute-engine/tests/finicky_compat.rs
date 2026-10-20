//! Finicky v4 compatibility: every config in `tests/configs` resolves every URL
//! in `urls.txt` the way the Go host did, and so does every suite (a config,
//! `rules.json`, or both, with openers) in `expected.json`. Every case is also
//! run through the API's plain `openUrl`, which must decide as the traced
//! run `resolve` is built on does.

use std::{
    cell::RefCell,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use pwrroute_engine::{
    AppType, Browser, Engine, Host, Limits, Link, LogLevel, ModifierKeys, Opener, Privacy,
    Resolution, RulesFile, Sources,
};
use serde::Deserialize;

fn configs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/configs")
}

/// Finicky never had privacy levels or private windows: a config's result
/// keeps everything, and opens a private window only when the config put
/// the flag in `args` itself.
fn privacy_as_finicky(b: &Browser) -> bool {
    let flagged = b.args.iter().any(|a| a == "--incognito");
    b.privacy == Privacy::Full && b.private == flagged
}

/// The plain `openUrl` run leaves privacy at its defaults; the traced run
/// sets it, so it is copied over before the two are compared.
fn plain_agrees(engine: &Engine, input: &str, opener: Option<&Opener>, got: &Resolution) -> bool {
    let mut plain = engine
        .resolve_plain(&Link::new(input), opener)
        .unwrap_or_else(|e| panic!("{input}: {e:#}"));
    plain.browser.private = got.browser.private;
    plain.browser.privacy = got.browser.privacy;
    plain == *got
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
    suites: Vec<Suite>,
}

fn expected() -> Expected {
    serde_json::from_str(&fs::read_to_string(configs().join("expected.json")).unwrap()).unwrap()
}

#[derive(Deserialize)]
struct Suite {
    name: String,
    config: Option<String>,
    rules: Option<String>,
    cases: Vec<SuiteCase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SuiteCase {
    input: String,
    opener: Option<Opener>,
    url: String,
    browser: String,
    profile: String,
    args: Vec<String>,
    open_in_background: bool,
    /// Finicky reported this error. The Go host then substituted its own
    /// fallback browser, so only the error is compared.
    error: Option<String>,
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
    let expected = expected();
    let mut failures = Vec::new();
    let mut checked = 0;
    for (config, cases) in &expected.configs {
        let engine = load(config);
        for case in cases {
            checked += 1;
            let got = engine
                .resolve(&case.input, None)
                .unwrap_or_else(|e| panic!("{config} {}: {e:#}", case.input));
            if !plain_agrees(&engine, &case.input, None, &got) {
                failures.push(format!(
                    "{config} {}: the trace ends elsewhere than openUrl",
                    case.input
                ));
            }
            let traced = engine
                .trace(&Link::new(&case.input), None)
                .unwrap_or_else(|e| panic!("{config} {} trace: {e:#}", case.input));
            if traced.winner().is_none() {
                failures.push(format!(
                    "{config} {}: the trace names no winner",
                    case.input
                ));
            }
            if !privacy_as_finicky(&got.browser) {
                failures.push(format!("{config} {}: privacy differs", case.input));
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
fn suites_match_the_go_host_and_their_traces() {
    let expected = expected();
    let mut failures = Vec::new();
    let mut checked = 0;
    for suite in &expected.suites {
        let sources = Sources {
            config: suite.config.as_ref().map(|c| configs().join(c)),
            rules: suite
                .rules
                .as_ref()
                .map(|r| RulesFile::load(&configs().join(r)).unwrap())
                .unwrap_or_default(),
        };
        let engine = Engine::from_sources(
            &sources,
            Rc::new(pwrroute_engine::DefaultHost),
            Limits::default(),
        )
        .unwrap_or_else(|e| panic!("{}: {e:#}", suite.name));
        for case in &suite.cases {
            checked += 1;
            let label = format!("{} {} {:?}", suite.name, case.input, case.opener);
            let got = engine
                .resolve(&case.input, case.opener.as_ref())
                .unwrap_or_else(|e| panic!("{label}: {e:#}"));
            if !plain_agrees(&engine, &case.input, case.opener.as_ref(), &got) {
                failures.push(format!("{label}: the trace ends elsewhere than openUrl"));
            }
            let trace = engine
                .trace(&Link::new(&case.input), case.opener.as_ref())
                .unwrap_or_else(|e| panic!("{label} trace: {e:#}"));
            if trace.winner().is_none() {
                failures.push(format!("{label}: the trace names no winner"));
            }
            if suite.rules.is_none() && !privacy_as_finicky(&got.browser) {
                failures.push(format!("{label}: privacy differs"));
            }
            if let Some(error) = &case.error {
                if got.error.as_deref() != Some(error.as_str()) {
                    failures.push(format!(
                        "{label}\n  want error {error:?}\n  got {:?}",
                        got.error
                    ));
                }
                continue;
            }
            let b = &got.browser;
            let actual = (
                b.url.as_str(),
                b.name.as_str(),
                b.profile.as_str(),
                b.args.as_slice(),
                b.open_in_background.unwrap_or(false),
                got.error.as_deref(),
            );
            let wanted = (
                case.url.as_str(),
                case.browser.as_str(),
                case.profile.as_str(),
                case.args.as_slice(),
                case.open_in_background,
                None,
            );
            if actual != wanted {
                failures.push(format!("{label}\n  want {wanted:?}\n  got  {actual:?}"));
            }
        }
    }
    assert_eq!(checked, 75, "expected.json lost suite cases");
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
