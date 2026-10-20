//! PwrRoute's additions on top of Finicky: `rules.json` extensions the Go
//! host never had, the resolution trace, and short-link expansion through the
//! engine. None of these can be recorded from the Go host, so each case says
//! why its answer is right.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use anyhow::{Result, bail};
use pwrroute_engine::{
    AppType, BrowserSpec, DefaultHost, Engine, FixedBrowser, Limits, Link, Matcher, MatcherSpec,
    Method, Network, Opener, Reply, Rewrite, RewriteAction, Rule, RulesFile, Source, Sources,
    Stage, UrlSpec,
};

fn configs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/configs")
}

fn engine(config: Option<&str>, rules: RulesFile) -> Engine {
    Engine::from_sources(
        &Sources {
            config: config.map(|c| configs().join(c)),
            rules,
        },
        Rc::new(DefaultHost),
        Limits::default(),
    )
    .unwrap()
}

fn rule(patterns: &[&str], browser: &str) -> Rule {
    Rule {
        patterns: patterns.iter().map(|p| (*p).to_owned()).collect(),
        browser: browser.into(),
        ..Rule::default()
    }
}

fn rewrite(patterns: &[&str], actions: Vec<RewriteAction>) -> Rewrite {
    Rewrite {
        patterns: patterns.iter().map(|p| (*p).to_owned()).collect(),
        actions,
        ..Rewrite::default()
    }
}

fn slack() -> Opener {
    Opener {
        name: "Slack".into(),
        bundle_id: "com.tinyspeck.slackmacgap".into(),
        ..Opener::default()
    }
}

// rules.json extensions

#[test]
fn visual_rewrites_apply_in_order_before_handlers() {
    let rules = RulesFile {
        default_browser: "Firefox".into(),
        rewrites: vec![
            rewrite(
                &["x.com/*", "twitter.com/*"],
                vec![RewriteAction::SetHost("xcancel.com".into())],
            ),
            rewrite(
                &["*"],
                vec![RewriteAction::DropQuery(vec!["utm_*".into(), "s".into()])],
            ),
            rewrite(
                &["http://*.internal/*"],
                vec![RewriteAction::SetScheme("https".into())],
            ),
            rewrite(
                &["old.example/*"],
                vec![
                    RewriteAction::Url("https://new.example/start".into()),
                    RewriteAction::ReplacePath("/moved".into()),
                ],
            ),
        ],
        rules: vec![rule(&["xcancel.com/*"], "Safari")],
        ..RulesFile::default()
    };
    let e = engine(None, rules);
    let r = e
        .resolve(
            "https://x.com/pwrdrvr/status/1842?utm_source=slack&s=20&keep=1",
            None,
        )
        .unwrap();
    assert_eq!(
        r.browser.url,
        "https://xcancel.com/pwrdrvr/status/1842?keep=1"
    );
    assert_eq!(r.browser.name, "Safari");
    let r = e.resolve("http://wiki.internal/a", None).unwrap();
    assert_eq!(r.browser.url, "https://wiki.internal/a");
    let r = e.resolve("https://old.example/x", None).unwrap();
    assert_eq!(r.browser.url, "https://new.example/moved");
    assert_eq!(r.browser.name, "Firefox");
}

#[test]
fn an_opener_rule_needs_the_app_and_the_url() {
    let rules = RulesFile {
        default_browser: "Safari".into(),
        rules: vec![Rule {
            opener: vec!["slack".into(), "com.example.other".into()],
            profile: "Work".into(),
            ..rule(&["*.atlassian.net/*"], "Google Chrome")
        }],
        ..RulesFile::default()
    };
    let e = engine(None, rules);
    let url = "https://pwrdrvr.atlassian.net/browse/PG-12";
    let from_slack = e.resolve(url, Some(&slack())).unwrap();
    assert_eq!(from_slack.browser.name, "Google Chrome");
    assert_eq!(from_slack.browser.profile, "Work");
    assert_eq!(e.resolve(url, None).unwrap().browser.name, "Safari");
    let wrong_url = e.resolve("https://example.com/", Some(&slack())).unwrap();
    assert_eq!(wrong_url.browser.name, "Safari");
}

#[test]
fn a_disabled_rule_is_skipped() {
    let rules = RulesFile {
        default_browser: "Safari".into(),
        rules: vec![
            Rule {
                enabled: false,
                ..rule(&["github.com/*"], "Firefox")
            },
            rule(&["github.com/*"], "Arc"),
        ],
        rewrites: vec![Rewrite {
            enabled: false,
            ..rewrite(
                &["github.com/*"],
                vec![RewriteAction::SetHost("gitlab.com".into())],
            )
        }],
        ..RulesFile::default()
    };
    let r = engine(None, rules)
        .resolve("https://github.com/x", None)
        .unwrap();
    assert_eq!(
        (r.browser.name.as_str(), r.browser.url.as_str()),
        ("Arc", "https://github.com/x")
    );
}

#[test]
fn visual_rewrites_follow_the_configs_own() {
    // stress.ts upgrades http to https; a rules.json rewrite after it sees
    // the upgraded URL, and its handlers come after the config's.
    let rules = RulesFile {
        rewrites: vec![rewrite(
            &["https://example.com/*"],
            vec![RewriteAction::ReplacePath("/after-config".into())],
        )],
        rules: vec![rule(&["https://example.com/after-config"], "Vivaldi")],
        ..RulesFile::default()
    };
    let r = engine(Some("stress.ts"), rules)
        .resolve("http://example.com/insecure", None)
        .unwrap();
    assert_eq!(r.browser.url, "https://example.com/after-config");
    assert_eq!(r.browser.name, "Vivaldi");
}

#[test]
fn a_config_keeps_its_default_browser_and_options() {
    let rules = RulesFile {
        default_browser: "Vivaldi".into(),
        ..RulesFile::default()
    };
    let r = engine(Some("legacy-commonjs.js"), rules)
        .resolve("https://unmatched.example/", None)
        .unwrap();
    assert_eq!(r.browser.name, "Safari");
}

#[test]
fn a_bad_static_rewrite_is_flagged_and_reported_when_it_applies() {
    let rules = RulesFile {
        rewrites: vec![rewrite(
            &["*"],
            vec![RewriteAction::Url("not a url".into())],
        )],
        ..RulesFile::default()
    };
    // A static rewrite URL is only parsed when it applies, so this loads; the
    // editor's problems() is what catches it.
    assert!(!rules.problems().is_empty());
    let e = engine(None, rules);
    assert!(
        e.resolve("https://example.com/", None)
            .unwrap()
            .error
            .is_some()
    );
}

// Trace

#[test]
fn a_trace_lists_each_step_and_the_winner() {
    let e = engine(Some("stress.ts"), RulesFile::default());
    let trace = e
        .trace(&Link::new("https://someone.medium.com/post-123"), None)
        .unwrap();
    let rewrites: Vec<_> = trace
        .steps
        .iter()
        .filter(|s| s.stage == Stage::Rewrite)
        .collect();
    assert_eq!(rewrites.len(), 5, "every rewrite is evaluated");
    let applied: Vec<_> = trace.rewrites_applied().collect();
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].index, 3);
    assert_eq!(
        applied[0].matchers,
        [Matcher::Wildcard("*.medium.com/*".into())]
    );
    assert_eq!(
        applied[0].rewritten_to.as_deref(),
        Some("https://scribe.rip/post-123")
    );
    let winner = trace.winner().unwrap();
    assert_eq!(
        (winner.stage, winner.source, winner.index),
        (Stage::Handler, Source::Config, 6)
    );
    assert_eq!(winner.url, "https://scribe.rip/post-123");
    let handlers = trace
        .steps
        .iter()
        .filter(|s| s.stage == Stage::Handler)
        .count();
    assert_eq!(handlers, 7, "handlers stop at the winner");
    assert_eq!(trace.resolution.browser.name, "Firefox");
}

#[test]
fn a_trace_says_which_pattern_and_source_matched() {
    let rules = RulesFile::load(&configs().join("rules/go-schema.json")).unwrap();
    let e = engine(Some("legacy-commonjs.js"), rules);
    let trace = e
        .trace(&Link::new("https://linear.app/pwrdrvr/issue/1"), None)
        .unwrap();
    let winner = trace.winner().unwrap();
    // legacy-commonjs.js's own /linear\.app/ wins before rules.json's.
    assert_eq!((winner.source, winner.index), (Source::Config, 1));
    assert_eq!(winner.matchers, [Matcher::Regex("/linear\\.app/".into())]);

    let trace = e
        .trace(&Link::new("https://acme.atlassian.net/browse/X-1"), None)
        .unwrap();
    let winner = trace.winner().unwrap();
    assert_eq!((winner.source, winner.index), (Source::Rules, 1));
    assert_eq!(winner.pattern, Some(0));
    assert_eq!(
        winner.matchers,
        [
            Matcher::Wildcard("*.atlassian.net/*".into()),
            Matcher::Wildcard("linear.app/*".into())
        ]
    );
    let sources: Vec<_> = trace.steps.iter().map(|s| (s.source, s.index)).collect();
    assert_eq!(
        sources,
        [
            (Source::Config, 0),
            (Source::Config, 1),
            (Source::Rules, 0),
            (Source::Rules, 1)
        ]
    );
}

#[test]
fn a_trace_numbers_rules_json_entries_by_their_place_in_the_file() {
    // A switched-off rewrite and handler are not loaded, but the ones after
    // them keep their file positions in the trace.
    let rules = RulesFile {
        rewrites: vec![
            Rewrite {
                enabled: false,
                ..rewrite(&["*"], vec![RewriteAction::SetHost("off.example".into())])
            },
            rewrite(&["*"], vec![RewriteAction::DropQuery(vec!["utm_*".into()])]),
        ],
        rules: vec![
            Rule {
                enabled: false,
                ..rule(&["*"], "Safari")
            },
            rule(&["a.example/*"], "Firefox"),
        ],
        ..RulesFile::default()
    };
    let trace = engine(None, rules)
        .trace(&Link::new("https://a.example/x?utm_source=m"), None)
        .unwrap();
    let steps: Vec<_> = trace
        .steps
        .iter()
        .map(|s| (s.stage, s.index, s.matched))
        .collect();
    assert_eq!(
        steps,
        [(Stage::Rewrite, 1, true), (Stage::Handler, 1, true)]
    );
    assert_eq!(trace.resolution.browser.url, "https://a.example/x");
}

#[test]
fn a_trace_ends_at_the_default_when_nothing_matches() {
    let rules = RulesFile::load(&configs().join("rules/go-schema.json")).unwrap();
    let trace = engine(None, rules)
        .trace(&Link::new("https://other.example/"), None)
        .unwrap();
    let last = trace.steps.last().unwrap();
    assert_eq!(
        (last.stage, last.source, last.won),
        (Stage::Default, Source::Rules, true)
    );
    assert!(
        trace.steps[..trace.steps.len() - 1]
            .iter()
            .all(|s| !s.matched)
    );
    assert_eq!(trace.resolution.browser.name, "Firefox");
}

#[test]
fn a_trace_records_the_step_that_threw() {
    let e = engine(Some("functions.js"), RulesFile::default());
    let trace = e
        .trace(&Link::new("https://handler-boom.example/"), None)
        .unwrap();
    let threw = trace.steps.iter().find(|s| s.error.is_some()).unwrap();
    assert_eq!(threw.stage, Stage::Handler);
    assert!(matches!(threw.matchers[0], Matcher::Function(_)));
    assert_eq!(threw.error.as_deref(), Some("handler matcher threw"));
    assert_eq!(trace.winner().unwrap().stage, Stage::Default);
    assert_eq!(
        trace.resolution.error.as_deref(),
        Some("handler matcher threw")
    );
}

#[test]
fn a_trace_sees_the_opener() {
    let e = engine(Some("opener.js"), RulesFile::default());
    let trace = e
        .trace(&Link::new("https://example.com/a"), Some(&slack()))
        .unwrap();
    let winner = trace.winner().unwrap();
    assert_eq!((winner.source, winner.index), (Source::Config, 0));
    assert_eq!(trace.resolution.browser.profile, "Work");
}

// Short links

/// Redirects every bit.ly link to one place and records what was asked.
#[derive(Default)]
struct FakeShortener {
    asked: RefCell<Vec<String>>,
}

impl Network for FakeShortener {
    fn request(&self, method: Method, url: &str, _timeout: Duration) -> Result<Reply> {
        self.asked.borrow_mut().push(format!("{method:?} {url}"));
        match url {
            "https://bit.ly/gh" => Ok(Reply {
                status: 301,
                location: Some("https://github.com/pwrdrvr/PwrRoute".into()),
            }),
            "https://go.corp.example/wiki" => Ok(Reply {
                status: 302,
                location: Some("https://corp.example.com/wiki".into()),
            }),
            "https://github.com/pwrdrvr/PwrRoute" | "https://corp.example.com/wiki" => Ok(Reply {
                status: 200,
                location: None,
            }),
            _ => bail!("offline"),
        }
    }
}

#[test]
fn a_short_link_resolves_where_it_lands() {
    let e = engine(Some("stress.ts"), RulesFile::default());
    let net = FakeShortener::default();
    let r = e
        .expand_and_resolve("https://bit.ly/gh", None, &net)
        .unwrap();
    assert_eq!(
        r.browser.name, "Firefox",
        "stress.ts sends github.com to Firefox"
    );
    assert_eq!(r.browser.url, "https://github.com/pwrdrvr/PwrRoute");
    assert_eq!(
        *net.asked.borrow(),
        [
            "Head https://bit.ly/gh",
            "Head https://github.com/pwrdrvr/PwrRoute"
        ]
    );
}

#[test]
fn a_config_sees_the_short_link_as_original_url() {
    let e = engine(Some("functions.js"), RulesFile::default());
    // functions.js's first handler matches on options.originalUrl's host.
    let link = Link {
        url: "https://elsewhere.example/".into(),
        original_url: "https://chain.example/short".into(),
    };
    let r = e.resolve_link(&link, None).unwrap();
    assert_eq!(r.browser.name, "Firefox");
    assert_eq!(r.browser.profile, "chain");
    let trace = e.trace(&link, None).unwrap();
    assert_eq!(trace.original_url, "https://chain.example/short");
    assert_eq!(trace.url, "https://elsewhere.example/");
}

#[test]
fn a_config_adds_shorteners_and_the_example_keeps_the_defaults() {
    let path = std::env::temp_dir().join(format!(
        "pwrroute-features-shorteners-{}.js",
        std::process::id()
    ));
    std::fs::write(
        &path,
        r#"export default {
  defaultBrowser: "Safari",
  options: { urlShorteners: ["go.corp.example"] },
  handlers: [{ match: "corp.example.com/*", browser: "Google Chrome" }],
};"#,
    )
    .unwrap();
    let e = Engine::load(&path, Rc::new(DefaultHost), Limits::default()).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(e.short_links().is_short("https://go.corp.example/wiki"));
    assert!(e.short_links().is_short("https://bit.ly/gh"));
    let r = e
        .expand_and_resolve(
            "https://go.corp.example/wiki",
            None,
            &FakeShortener::default(),
        )
        .unwrap();
    assert_eq!(r.browser.name, "Google Chrome");

    // Finicky's example sets `urlShorteners: []`; the Go host expanded the
    // built-in list anyway, and so does the engine.
    let example = engine(Some("finicky-example.js"), RulesFile::default());
    assert!(example.short_links().is_short("https://t.co/x"));
}

#[test]
fn an_unreachable_shortener_routes_the_short_link() {
    let e = engine(Some("stress.ts"), RulesFile::default());
    let r = e
        .expand_and_resolve("https://bit.ly/unknown", None, &FakeShortener::default())
        .unwrap();
    assert_eq!(r.browser.url, "https://bit.ly/unknown");
    assert_eq!(r.browser.name, "Safari");
}

#[test]
fn a_handler_whose_browser_fails_does_not_win() {
    // functions.js handler 4 matches but its browser function returns
    // undefined; Finicky then opens the default browser.
    let e = engine(Some("functions.js"), RulesFile::default());
    let trace = e
        .trace(&Link::new("https://undefined.example/"), None)
        .unwrap();
    let winners: Vec<_> = trace.steps.iter().filter(|s| s.won).collect();
    assert_eq!(winners.len(), 1, "{:?}", trace.steps);
    assert_eq!(winners[0].stage, Stage::Default);
    let failed = trace
        .steps
        .iter()
        .find(|s| s.stage == Stage::Handler && s.matched)
        .unwrap();
    assert_eq!(failed.index, 4);
    assert_eq!(failed.error, trace.resolution.error);
    assert!(
        failed
            .error
            .as_deref()
            .unwrap()
            .contains("cannot be undefined")
    );
}

// Describing a config

fn source(spec: &MatcherSpec) -> &str {
    match spec {
        MatcherSpec::Function(f) => f.source.as_deref().expect("QuickJS kept the source"),
        other => panic!("not a function: {other:?}"),
    }
}

fn fixed(name: &str, app_type: AppType, profile: &str) -> BrowserSpec {
    BrowserSpec::Browser(FixedBrowser {
        name: name.into(),
        app_type,
        profile: profile.into(),
        args: Vec::new(),
        open_in_background: None,
    })
}

#[test]
fn a_description_lists_the_configs_rules_in_order() {
    let d = engine(Some("functions.js"), RulesFile::default())
        .describe()
        .unwrap()
        .unwrap();
    assert_eq!(d.rewrites.len(), 5);
    assert_eq!(d.handlers.len(), 12);

    // Rewrites: a wildcard to a function, and a wildcard to a fixed URL.
    assert_eq!(
        d.rewrites[0].matchers,
        [MatcherSpec::Wildcard("*://chain.example/*".into())]
    );
    let UrlSpec::Function(chain) = &d.rewrites[0].url else {
        panic!("a function rewrite: {:?}", d.rewrites[0].url);
    };
    assert!(chain.source.as_deref().unwrap().contains("\"/one\""));
    assert!(source(&d.rewrites[3].matchers[0]).contains("rewrite matcher threw"));
    assert_eq!(
        d.rewrites[4].url,
        UrlSpec::Url("https://static-target.example/landing".into())
    );

    // Handlers: what each browser opens as, from the API's createBrowserConfig.
    assert!(source(&d.handlers[0].matchers[0]).contains("originalUrl.hostname"));
    assert_eq!(
        d.handlers[0].browser,
        fixed("Firefox", AppType::AppName, "chain")
    );
    assert_eq!(
        d.handlers[1].matchers,
        [MatcherSpec::Regex(
            "/^https:\\/\\/CASE\\.example\\//i".into()
        )]
    );
    assert_eq!(
        d.handlers[1].browser,
        fixed("Google Chrome", AppType::AppName, "Profile 2")
    );
    assert_eq!(
        d.handlers[2].browser,
        fixed("com.google.Chrome", AppType::BundleId, "")
    );
    assert_eq!(
        d.handlers[3].browser,
        fixed("/Applications/Firefox.app", AppType::Path, "")
    );
    assert!(matches!(d.handlers[4].browser, BrowserSpec::Function(_)));
    assert_eq!(
        d.handlers[8].matchers,
        [
            MatcherSpec::Wildcard("https://first.example/*".into()),
            MatcherSpec::Wildcard("https://second.example/*".into()),
        ]
    );
    let BrowserSpec::Browser(safari) = &d.handlers[10].browser else {
        panic!("a fixed browser");
    };
    assert_eq!(safari.open_in_background, Some(false));

    // The default browser is a function here.
    let BrowserSpec::Function(default) = &d.default_browser else {
        panic!("a function default: {:?}", d.default_browser);
    };
    assert!(
        default
            .source
            .as_deref()
            .unwrap()
            .contains("Firefox Developer Edition")
    );
}

#[test]
fn a_description_shows_typescript_functions_without_their_types() {
    let d = engine(Some("finicky-example.ts"), RulesFile::default())
        .describe()
        .unwrap()
        .unwrap();
    assert!(d.rewrites.is_empty());
    assert_eq!(d.handlers.len(), 2);
    // A named const's arrow function, its `url: URL` annotation stripped.
    let workplace = source(&d.handlers[0].matchers[0]);
    assert!(
        workplace.contains("url.host.includes(\"workplace\")"),
        "{workplace}"
    );
    assert!(!workplace.contains(": URL"), "{workplace}");
    assert!(source(&d.handlers[1].matchers[0]).contains("opener?.name.includes(\"Slack\")"));
    assert_eq!(
        d.handlers[1].browser,
        fixed("Firefox", AppType::AppName, "")
    );
    assert_eq!(
        d.default_browser,
        fixed("Google Chrome", AppType::AppName, "")
    );
}

#[test]
fn a_description_shows_null_browsers_and_functions_without_source() {
    let d = engine(Some("describe.js"), RulesFile::default())
        .describe()
        .unwrap()
        .unwrap();
    // A bound function's toString is native code: no source, just its name.
    let MatcherSpec::Function(bound) = &d.handlers[0].matchers[0] else {
        panic!("a function matcher: {:?}", d.handlers[0].matchers);
    };
    assert_eq!(bound.source, None);
    assert!(bound.name.contains("test"), "{}", bound.name);
    // `browser: null` opens nothing, as createBrowserConfig spells it.
    assert_eq!(d.handlers[1].browser, fixed("", AppType::None, ""));
    assert_eq!(d.default_browser, fixed("", AppType::None, ""));
}

#[test]
fn a_description_leaves_out_rules_json() {
    let rules = RulesFile {
        default_browser: "Firefox".into(),
        rules: vec![rule(&["example.com/*"], "Safari")],
        ..RulesFile::default()
    };
    // rules.json alone: there is no config to describe.
    assert_eq!(engine(None, rules.clone()).describe().unwrap(), None);
    // With a config, only the config's own handlers.
    let d = engine(Some("finicky-example.ts"), rules)
        .describe()
        .unwrap()
        .unwrap();
    assert_eq!(d.handlers.len(), 2);
}
