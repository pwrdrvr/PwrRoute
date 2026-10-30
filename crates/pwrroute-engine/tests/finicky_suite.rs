//! Finicky's own tests, run through PwrRoute's engine, plus the cases the Go
//! oracle cannot record. `tests/finicky-coverage.md` maps every Finicky test
//! to the PwrRoute test that covers it.
//!
//! - Finicky's config API unit tests (`packages/config-api/src/*.test.ts`)
//!   are ported to `configs/finicky-unit.js` and run here in the engine's
//!   QuickJS context; `oracle/record.py --eval-oracle` runs the same file in
//!   the Go host's goja VM.
//! - The Go tests (`resolver_test.go`, `rules_test.go`) and Finicky's
//!   `testdata/` are suites in `configs/expected.json`, recorded from the Go
//!   host and checked by `finicky_compat.rs`. What a suite cannot hold (no
//!   config at all, `rules.json` on disk, a rule the PwrFinicky router
//!   refuses) is here, each with the Finicky code that decides it.
//! - Wiki recipes the Go router cannot run (a `finicky:` link, a config
//!   Finicky 4 rejects) are here too.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use pwrroute_engine::{
    AppType, DefaultHost, Engine, Host, Limits, LogLevel, ModifierKeys, Opener, PowerInfo, Rule,
    RulesFile, Sources, SystemInfo,
};
use serde::Deserialize;

fn configs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/configs")
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pwrroute-finicky-suite-{}-{name}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_config(name: &str, source: &str) -> PathBuf {
    let path = temp_dir("configs").join(name);
    fs::write(&path, source).unwrap();
    path
}

fn engine_for(config: Option<PathBuf>, rules: RulesFile, host: Rc<dyn Host>) -> Engine {
    Engine::from_sources(&Sources { config, rules }, host, Limits::default())
        .unwrap_or_else(|e| panic!("{e:#}"))
}

fn rules_only(rules: RulesFile) -> Engine {
    engine_for(None, rules, Rc::new(DefaultHost))
}

fn terminal() -> Opener {
    Opener {
        name: "Terminal".into(),
        bundle_id: "com.apple.Terminal".into(),
        path: "/System/Applications/Utilities/Terminal.app".into(),
        window_title: None,
    }
}

// ---- packages/config-api/src/*.test.ts -------------------------------------

#[derive(Deserialize)]
struct UnitResult {
    file: String,
    name: String,
    ok: bool,
    error: Option<String>,
}

/// Every `it` in Finicky's four config API test files (79 at upstream
/// `main`) and the two PwrFinicky added for the bundle's validation, run
/// against the bundle in the engine's context.
#[test]
fn finickys_config_api_unit_tests_pass_in_the_engine() {
    let engine = Engine::from_sources(
        &Sources::default(),
        Rc::new(DefaultHost),
        Limits {
            resolve: Duration::from_secs(20),
            ..Limits::default()
        },
    )
    .unwrap();
    let script = fs::read_to_string(configs().join("finicky-unit.js")).unwrap();
    let value = engine.evaluate(&script, "finickyUnit.run()").unwrap();
    let results: Vec<UnitResult> = serde_json::from_value(value).unwrap();

    let mut per_file = BTreeMap::<&str, usize>::new();
    for r in &results {
        *per_file.entry(r.file.as_str()).or_default() += 1;
    }
    assert_eq!(
        per_file,
        BTreeMap::from([
            ("FinickyURL.test.ts", 10),
            ("config.test.ts", 23),
            ("config.test.ts (PwrFinicky)", 2),
            ("utils.test.ts", 33),
            ("wildcard.test.ts", 13),
        ]),
        "finicky-unit.js lost or gained cases"
    );
    let failures: Vec<String> = results
        .iter()
        .filter(|r| !r.ok)
        .map(|r| {
            format!(
                "{} › {}: {}",
                r.file,
                r.name,
                r.error.as_deref().unwrap_or("")
            )
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The harness itself can fail: a wrong expectation is reported, not
/// swallowed.
#[test]
fn the_unit_harness_reports_a_wrong_expectation() {
    let engine = rules_only(RulesFile::default());
    let script = fs::read_to_string(configs().join("finicky-unit.js"))
        .unwrap()
        .replace(
            r#"expect(url.port).toBe("8080");"#,
            r#"expect(url.port).toBe("8081");"#,
        );
    let value = engine.evaluate(&script, "finickyUnit.run()").unwrap();
    let results: Vec<UnitResult> = serde_json::from_value(value).unwrap();
    let failed: Vec<&UnitResult> = results.iter().filter(|r| !r.ok).collect();
    assert_eq!(failed.len(), 1);
    assert!(
        failed[0]
            .name
            .ends_with("should inherit all URL properties")
    );
    assert!(failed[0].error.as_deref().unwrap().contains("8081"));
}

/// config.test.ts "validation", as a config file: Finicky's schema rejects
/// `defaultBrowser: 123` and the engine refuses to load it.
#[test]
fn an_invalid_default_browser_is_rejected_as_finicky_rejects_it() {
    let path = write_config("default-123.js", "export default { defaultBrowser: 123 };");
    let err = Engine::load(&path, Rc::new(DefaultHost), Limits::default())
        .unwrap_err()
        .to_string();
    assert!(err.starts_with("validating the config:"), "{err}");
    assert!(err.contains("defaultBrowser"), "{err}");
}

// ---- apps/finicky/src/resolver/resolver_test.go ----------------------------

/// TestResolveURL_NoConfig: with no config at all, `ResolveURL` answers
/// `com.apple.Safari` by bundle id. With no config and an empty `rules.json`
/// the engine does the same (`FALLBACK_BROWSER`, as the Go host's
/// `ToJSConfigScript` falls back).
#[test]
fn no_config_resolves_to_safari_by_bundle_id() {
    let engine = rules_only(RulesFile::default());
    let r = engine.resolve("https://example.com", None).unwrap();
    assert_eq!(r.browser.name, "com.apple.Safari");
    assert_eq!(r.browser.app_type, AppType::BundleId);
    assert_eq!(r.browser.url, "https://example.com/");
    assert_eq!(r.error, None);
}

/// TestResolveURL_OpenInBackground, the engine's half: `mergeBackground`
/// keeps a config's `openInBackground` and uses the host's request only when
/// the config leaves it unset, so the engine must hand back `None` for
/// "unset". (Finicky's host asks for the background when Finicky is not the
/// frontmost app; PwrRoute's launcher does not infer that, see the coverage
/// map.)
#[test]
fn open_in_background_is_unset_unless_the_config_sets_it() {
    let path = write_config(
        "background.js",
        r#"export default {
  defaultBrowser: "Safari",
  handlers: [
    { match: "https://bg.example/*", browser: { name: "Firefox", openInBackground: true } },
    { match: "https://fg.example/*", browser: { name: "Firefox", openInBackground: false } },
  ],
};"#,
    );
    let engine = engine_for(Some(path), RulesFile::default(), Rc::new(DefaultHost));
    let bg = |url: &str| {
        engine
            .resolve(url, None)
            .unwrap()
            .browser
            .open_in_background
    };
    assert_eq!(bg("https://bg.example/"), Some(true));
    assert_eq!(bg("https://fg.example/"), Some(false));
    assert_eq!(bg("https://other.example/"), None);
}

// ---- apps/finicky/src/rules/rules_test.go ----------------------------------

fn rule(patterns: &[&str], browser: &str, profile: &str) -> Rule {
    Rule {
        patterns: patterns.iter().map(|p| (*p).to_owned()).collect(),
        browser: browser.into(),
        profile: profile.into(),
        ..Rule::default()
    }
}

/// TestToJSHandlers_Empty and TestToJSConfigScript_DefaultBrowserFallback: no
/// rules and no default browser route everything to `com.apple.Safari`.
#[test]
fn an_empty_rules_file_falls_back_to_safari() {
    let engine = rules_only(RulesFile {
        rules: vec![rule(&["example.com/*"], "Firefox", "")],
        ..RulesFile::default()
    });
    assert_eq!(
        engine
            .resolve("https://other.example/", None)
            .unwrap()
            .browser
            .name,
        "com.apple.Safari"
    );
    assert_eq!(
        engine
            .resolve("https://example.com/a", None)
            .unwrap()
            .browser
            .name,
        "Firefox"
    );
}

/// TestToJSHandlers_SkipsIncompleteRules: a rule whose patterns are all
/// empty, or that names no browser, is skipped; the rest still route. (The
/// PwrFinicky router refuses such a file outright, so no suite records it;
/// upstream's `ToJSHandlers` skips, and so does the engine.)
#[test]
fn incomplete_rules_are_skipped_and_the_rest_route() {
    let engine = rules_only(RulesFile {
        default_browser: "Arc".into(),
        rules: vec![
            rule(&[""], "Firefox", ""),
            rule(&["skipped.example/*"], "", ""),
            rule(&["", "kept.example/*"], "Safari", ""),
        ],
        ..RulesFile::default()
    });
    let browser = |url: &str| engine.resolve(url, None).unwrap().browser.name;
    assert_eq!(browser("https://skipped.example/x"), "Arc");
    assert_eq!(browser("https://kept.example/x"), "Safari");
    assert_eq!(browser("https://any.example/"), "Arc");
}

/// TestToJSHandlers_StringBrowser, _WithProfile and _MultipleRules (order
/// preserved: the first match wins), and TestToJSConfigScript_ExplicitDefault.
#[test]
fn rules_keep_their_browser_profile_and_order() {
    let engine = rules_only(RulesFile {
        default_browser: "Firefox".into(),
        rules: vec![
            rule(&["*github.com/*"], "Firefox", ""),
            rule(&["https://linear.app/*"], "Google Chrome", "Work"),
            rule(&["*github.com/*", "example.com"], "Safari", ""),
        ],
        ..RulesFile::default()
    });
    let github = engine
        .resolve("https://github.com/x", None)
        .unwrap()
        .browser;
    assert_eq!(
        (github.name.as_str(), github.profile.as_str()),
        ("Firefox", "")
    );
    let linear = engine
        .resolve("https://linear.app/t/1", None)
        .unwrap()
        .browser;
    assert_eq!(
        (linear.name.as_str(), linear.profile.as_str()),
        ("Google Chrome", "Work")
    );
    let other = engine
        .resolve("https://other.example/", None)
        .unwrap()
        .browser;
    assert_eq!(other.name, "Firefox");
}

/// TestLoadSave_RoundTrip, on Go's own field names: what Go wrote loads, and
/// what PwrRoute saves from it is what Go wrote (a single pattern as a
/// string, `profile` and `defaultProfile` only when set).
#[test]
fn a_go_rules_file_round_trips_byte_for_byte_in_shape() {
    let go_written = serde_json::json!({
        "defaultBrowser": "Firefox",
        "defaultProfile": "Work",
        "rules": [
            { "match": "*github.com/*", "browser": "Google Chrome", "profile": "Personal" },
            { "match": "https://linear.app/*", "browser": "Safari" },
            { "match": ["a.example/*", "b.example/*"], "browser": "Arc" }
        ]
    });
    let dir = temp_dir("roundtrip");
    let path = dir.join("rules.json");
    fs::write(&path, serde_json::to_string_pretty(&go_written).unwrap()).unwrap();
    let loaded = RulesFile::load(&path).unwrap();
    assert_eq!(loaded.default_browser, "Firefox");
    assert_eq!(loaded.default_profile, "Work");
    assert_eq!(loaded.rules[0].patterns, ["*github.com/*"]);
    assert_eq!(loaded.rules[0].profile, "Personal");
    assert_eq!(loaded.rules[2].patterns, ["a.example/*", "b.example/*"]);
    loaded.save(&path).unwrap();
    let saved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved, go_written);
    assert_eq!(RulesFile::load(&path).unwrap(), loaded);
}

/// TestLoad_MissingFile and TestLoad_EmptyRulesNotNil.
#[test]
fn a_missing_file_or_absent_rules_load_empty() {
    let dir = temp_dir("load");
    assert_eq!(
        RulesFile::load(&dir.join("nonexistent.json")).unwrap(),
        RulesFile::default()
    );
    let path = dir.join("rules.json");
    fs::write(&path, r#"{"defaultBrowser":"Safari"}"#).unwrap();
    let file = RulesFile::load(&path).unwrap();
    assert!(file.rules.is_empty());
    assert_eq!(file.default_browser, "Safari");
}

/// Finicky's `testdata/rules.json`, read from disk exactly as Finicky 4.4's
/// visual editor saved it, loads unchanged; `finicky_compat` checks how it
/// routes against the Go host.
#[test]
fn finickys_testdata_rules_json_loads_unchanged() {
    let file = RulesFile::load(&configs().join("finicky/testdata-rules.json")).unwrap();
    assert_eq!(file.default_browser, "Google Chrome");
    assert_eq!(file.rules.len(), 2);
    assert_eq!(file.rules[0].patterns, ["example.com/*", "github.com/*"]);
    assert_eq!(file.rules[1].patterns, ["linear.app/*"]);
    assert!(file.problems().is_empty(), "{:?}", file.problems());
}

// ---- Finicky's `finicky` global and `console` -------------------------------

#[derive(Default)]
struct FakeHost {
    logs: RefCell<Vec<(LogLevel, String)>>,
}

impl Host for FakeHost {
    fn modifier_keys(&self) -> ModifierKeys {
        ModifierKeys {
            command: true,
            fn_key: true,
            function_key: true,
            ..ModifierKeys::default()
        }
    }

    fn system_info(&self) -> SystemInfo {
        SystemInfo {
            name: "work-mac".into(),
            localized_name: "Work Mac".into(),
            platform: "macos".into(),
        }
    }

    fn power_info(&self) -> PowerInfo {
        PowerInfo {
            is_charging: Some(true),
            is_connected: Some(true),
            percentage: Some(80.0),
        }
    }

    fn is_app_running(&self, identifier: &str) -> bool {
        identifier == "com.tinyspeck.slackmacgap" || identifier == "Slack"
    }

    fn log(&self, level: LogLevel, message: &str) {
        self.logs.borrow_mut().push((level, message.to_owned()));
    }
}

/// The documented utilities (wiki "Configuration (v4)" › Utility Functions
/// and `finicky.d.ts`): each native one reaches the host with the documented
/// shape, and the API's own `matchHostnames`, `matchDomains`, `notify` and
/// `getBattery` behave as the API defines them.
#[test]
fn the_finicky_global_answers_every_documented_utility() {
    let path = write_config(
        "utilities.js",
        r#"export default {
  defaultBrowser: "Safari",
  handlers: [
    { match: () => finicky.getModifierKeys().command && finicky.getModifierKeys().fn && !finicky.getModifierKeys().shift, browser: "Modifier Keys" },
  ],
};
export const probe = () => ({
  keys: finicky.getModifierKeys(),
  system: finicky.getSystemInfo(),
  power: finicky.getPowerInfo(),
  slack: finicky.isAppRunning("com.tinyspeck.slackmacgap"),
  arc: finicky.isAppRunning("Arc"),
  hostnames: finicky.matchHostnames(["example.com", /\.example\.org$/])(new URL("https://a.example.org/")),
  domains: finicky.matchDomains("example.com")(new URL("https://example.com/x")),
  notify: finicky.notify("hello") === undefined,
  battery: finicky.getBattery(),
});"#,
    );
    let host = Rc::new(FakeHost::default());
    let engine = engine_for(Some(path), RulesFile::default(), host.clone());
    assert_eq!(
        engine
            .resolve("https://example.com/", None)
            .unwrap()
            .browser
            .name,
        "Modifier Keys"
    );
    let got = engine
        .evaluate("", "finickyConfig.probe()")
        .unwrap_or_else(|e| panic!("{e:#}"));
    assert_eq!(
        got["keys"],
        serde_json::json!({
            "shift": false, "option": false, "command": true, "control": false,
            "capsLock": false, "fn": true, "function": true
        })
    );
    assert_eq!(got["system"]["name"], "work-mac");
    assert_eq!(got["system"]["localizedName"], "Work Mac");
    assert_eq!(
        got["power"],
        serde_json::json!({ "isCharging": true, "isConnected": true, "percentage": 80 })
    );
    assert_eq!(got["slack"], true);
    assert_eq!(got["arc"], false);
    assert_eq!(got["hostnames"], true);
    assert_eq!(got["domains"], true);
    assert_eq!(got["notify"], true);
    assert_eq!(
        got["battery"],
        serde_json::json!({ "isCharging": false, "isPluggedIn": false, "chargePercentage": 0 })
    );
    let logs = host.logs.borrow();
    let said =
        |level: LogLevel, text: &str| logs.iter().any(|(l, m)| *l == level && m.contains(text));
    assert!(
        said(LogLevel::Warn, "finicky.matchDomains is deprecated"),
        "{logs:?}"
    );
    assert!(
        said(LogLevel::Error, "finicky.notify is unavailable"),
        "{logs:?}"
    );
    assert!(
        said(LogLevel::Error, "finicky.getBattery is unavailable"),
        "{logs:?}"
    );
}

/// `console.log`, `warn` and `error` (the three the wiki documents and the Go
/// host provides) reach the host at their levels; a CommonJS config loads
/// with the API's deprecation warning, as under Finicky 4.
#[test]
fn console_levels_and_the_commonjs_warning_reach_the_host() {
    let path = write_config(
        "console.js",
        r#"console.log("log line"); console.warn("warn line"); console.error("error line");
module.exports = { defaultBrowser: "Safari" };"#,
    );
    let host = Rc::new(FakeHost::default());
    engine_for(Some(path), RulesFile::default(), host.clone());
    let logs = host.logs.borrow();
    assert!(
        logs.contains(&(LogLevel::Info, "log line".into())),
        "{logs:?}"
    );
    assert!(
        logs.contains(&(LogLevel::Warn, "warn line".into())),
        "{logs:?}"
    );
    assert!(
        logs.contains(&(LogLevel::Error, "error line".into())),
        "{logs:?}"
    );
    assert!(
        logs.iter().any(|(l, m)| *l == LogLevel::Warn
            && m.contains("No default export found for configuration namespace")),
        "{logs:?}"
    );
}

/// Finicky 3's `({ url })` and `({ opener })` callbacks read FinickyURL's
/// legacy getters, which warn once per read; the warning reaches the trace's
/// console like any other line.
#[test]
fn legacy_url_arguments_work_and_warn() {
    let path = write_config(
        "legacy-args.js",
        r#"export default {
  defaultBrowser: "Safari",
  handlers: [
    { match: ({ url }) => url.host === "legacy.example" && url.protocol === "https", browser: "Legacy URL" },
    { match: ({ opener }) => opener !== null && opener.name === "Terminal", browser: "Legacy Opener" },
  ],
};"#,
    );
    let engine = engine_for(Some(path), RulesFile::default(), Rc::new(DefaultHost));
    let link = pwrroute_engine::Link::new("https://legacy.example/a");
    let trace = engine.trace(&link, None).unwrap();
    assert_eq!(trace.resolution.browser.name, "Legacy URL");
    assert!(
        trace
            .console
            .iter()
            .any(|l| l.level == LogLevel::Warn && l.message.contains(r#"legacy property "url""#)),
        "{:?}",
        trace.console
    );
    let r = engine
        .resolve("https://other.example/", Some(&terminal()))
        .unwrap();
    assert_eq!(r.browser.name, "Legacy Opener");
}

// ---- Finicky's wiki, where the Go router cannot record ----------------------

fn wiki_ideas() -> Engine {
    engine_for(
        Some(configs().join("finicky/wiki-ideas.js")),
        RulesFile::default(),
        Rc::new(DefaultHost),
    )
}

/// "MacOS: Create bookmarks that open Finicky": `finicky://open?url=…` is
/// handed to the config as it is (upstream's `HandleURL` only decodes
/// `finicky://open/<base64>`), and the recipe's rewrite returns the target.
/// The PwrFinicky router refuses `finicky:` links, so this is not a suite.
#[test]
fn the_bookmark_recipe_rewrites_a_finicky_link_to_its_target() {
    let r = wiki_ideas()
        .resolve(
            "finicky://open?url=https%3A%2F%2Fgithub.com%2Fx",
            Some(&terminal()),
        )
        .unwrap();
    assert_eq!(r.browser.url, "https://github.com/x");
    assert_eq!(r.browser.name, "Safari");
    assert_eq!(r.error, None);
}

/// "Match multiple conditions" reads `opener.bundleId` without a null check:
/// with no opener the matcher throws, and Finicky's `openUrl` reports the
/// error and falls back to the default browser. The thrown TypeError's text
/// is the JS engine's own, so the suite cannot compare it with Go's.
#[test]
fn a_recipe_that_assumes_an_opener_falls_back_without_one() {
    let r = wiki_ideas()
        .resolve("https://drive.google.com/x", None)
        .unwrap();
    assert_eq!(r.browser.name, "Safari");
    assert!(r.error.is_some());
}

/// Finicky 3 recipes that Finicky 4's schema rejects, because a rewrite's
/// `url` is a string, a URL or a function (never a partial object): the
/// config does not load, with Finicky's validation message, as under the
/// Go host (`validateConfig` fails, and the app keeps its previous rules).
#[test]
fn v3_partial_url_objects_fail_validation_as_in_finicky_4() {
    for (name, source) in [
        (
            "smile.js",
            r#"export default { defaultBrowser: "Safari", rewrite: [{ match: "amazon.com/*", url: { host: "smile.amazon.com" } }] };"#,
        ),
        (
            "spamcop.js",
            r#"export default { defaultBrowser: "Safari", rewrite: [{ match: finicky.matchHostnames("www.spamcop.net"), url: { hash: "report" } }] };"#,
        ),
    ] {
        let path = write_config(name, source);
        let err = Engine::load(&path, Rc::new(DefaultHost), Limits::default())
            .unwrap_err()
            .to_string();
        assert!(err.starts_with("validating the config:"), "{name}: {err}");
        assert!(err.contains("rewrite"), "{name}: {err}");
    }
}

/// The Troubleshooting page's Finicky 3 `finicky.log(…)` is gone in
/// Finicky 4: the matcher throws, the error is reported and the default
/// browser opens the link.
#[test]
fn finicky_log_from_v3_is_reported_as_an_error() {
    let path = write_config(
        "finicky-log.js",
        r#"module.exports = {
  defaultBrowser: "Safari",
  rewrite: [{ match(all) { finicky.log(JSON.stringify(all, null, 2)); return false; }, url: ({ url }) => url }],
};"#,
    );
    let engine = engine_for(Some(path), RulesFile::default(), Rc::new(DefaultHost));
    let r = engine.resolve("https://example.com/", None).unwrap();
    assert_eq!(r.browser.name, "Safari");
    assert!(r.error.is_some());
}

/// Every option `ConfigOptions` declares validates and loads (only
/// `urlShorteners` changes what the engine does; `features.rs` covers it).
#[test]
fn every_documented_option_loads() {
    let path = write_config(
        "options.js",
        r#"export default {
  defaultBrowser: "Safari",
  options: { urlShorteners: ["s.example"], logRequests: true, checkForUpdates: false, keepRunning: true, hideIcon: true },
};"#,
    );
    let engine = engine_for(Some(path), RulesFile::default(), Rc::new(DefaultHost));
    assert!(engine.short_links().is_short("https://s.example/abc"));
    assert_eq!(
        engine
            .resolve("https://example.com/", None)
            .unwrap()
            .browser
            .name,
        "Safari"
    );
}
