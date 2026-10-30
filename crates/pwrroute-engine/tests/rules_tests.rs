//! `rules.json` examples and tests, the privacy of what the rules route, and
//! help for writing a rule: all PwrRoute's own, so each case says why its
//! answer is right. Every URL here is contrived.

use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

use pwrroute_engine::{
    DefaultHost, Engine, Expectation, Limits, Place, Privacy, PrivacySetting, Rewrite,
    RewriteAction, RewriteExample, RouteTest, Rule, RulesFile, Sources, Stage, TestSource,
    suggest_rule,
};

fn configs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/configs")
}

fn engine(config: Option<&str>, rules: RulesFile) -> Engine {
    Engine::from_sources(
        &Sources {
            config: config.map(|c| configs().join(c)),
            rules,
            ..Default::default()
        },
        Rc::new(DefaultHost),
        Limits::default(),
    )
    .unwrap()
}

fn rules(name: &str) -> RulesFile {
    RulesFile::load(&configs().join("rules").join(name)).unwrap()
}

fn rule(patterns: &[&str], browser: &str) -> Rule {
    Rule {
        patterns: patterns.iter().map(|p| (*p).to_owned()).collect(),
        browser: browser.into(),
        ..Rule::default()
    }
}

fn test(url: &str, browser: &str) -> RouteTest {
    RouteTest {
        url: url.into(),
        opener: None,
        expect: Expectation {
            browser: browser.into(),
            ..Expectation::default()
        },
    }
}

// Examples and tests

#[test]
fn the_sample_file_passes_its_examples_and_tests() {
    let file = rules("examples-pass.json");
    assert!(file.problems().is_empty());
    let e = engine(None, file.clone());
    let report = file.run_tests(&e).unwrap();
    assert!(report.is_ok(), "{:#?}", report.failures);
    // One rewrite example, four rule examples (the disabled rule's is not
    // run) and three tests.
    assert_eq!((report.total, report.passed), (8, 8));
    assert_eq!(e.test_rules(None).unwrap(), report);
}

#[test]
fn a_broad_rule_takes_an_example_and_the_report_says_so() {
    let file = rules("examples-fail.json");
    let report = file.run_tests(&engine(None, file.clone())).unwrap();
    assert_eq!((report.total, report.passed), (5, 2));
    let sources: Vec<_> = report.failures.iter().map(|f| f.source).collect();
    assert_eq!(
        sources,
        [
            TestSource::RuleExample {
                rule: 1,
                example: 0
            },
            TestSource::RewriteExample {
                rewrite: 0,
                example: 0
            },
            TestSource::Test { test: 0 },
        ]
    );

    let taken = &report.failures[0];
    assert_eq!(taken.input, "https://acme.atlassian.net/browse/X-1");
    assert_eq!(taken.expected, "rule 2 (acme.atlassian.net/*)");
    assert_eq!(
        taken.actual,
        "rule 1: Arc with https://acme.atlassian.net/browse/X-1"
    );
    let winner = taken.winner.as_ref().unwrap();
    assert_eq!((winner.stage, winner.index), (Stage::Handler, 0));

    let rewrite = &report.failures[1];
    assert_eq!(rewrite.expected, "https://new.example/moved");
    assert_eq!(rewrite.actual, "https://new.example/a");

    let test = &report.failures[2];
    assert_eq!(test.expected, "Google Chrome (Work)");
    assert_eq!(
        test.actual,
        "Arc with https://acme.atlassian.net/browse/X-1"
    );
}

#[test]
fn a_proposal_is_tested_on_a_temporary_engine_and_the_saved_rules_stay() {
    let saved = rules("examples-pass.json");
    let e = engine(None, saved.clone());
    let mut proposal = saved.clone();
    proposal
        .rules
        .insert(0, rule(&["*.atlassian.net/*"], "Arc"));
    let report = e.test_rules(Some(&proposal)).unwrap();
    assert_eq!(report.failures.len(), 3, "{:#?}", report.failures);
    assert!(report.failures.iter().all(|f| matches!(
        f.source,
        TestSource::RuleExample { rule: 1, .. } | TestSource::Test { test: 0 }
    )));
    // The engine itself still routes with the saved rules.
    assert!(e.test_rules(None).unwrap().is_ok());
    assert_eq!(
        e.resolve("https://acme.atlassian.net/browse/X-1", None)
            .unwrap()
            .browser
            .name,
        "Google Chrome"
    );
}

#[test]
fn examples_and_tests_run_through_the_js_config_too() {
    // stress.ts's own handler takes github.com before any rules.json rule,
    // and its rewrites turn medium.com into scribe.rip.
    let file = RulesFile {
        rules: vec![Rule {
            examples: vec!["https://github.com/pwrdrvr/PwrRoute".into()],
            ..rule(&["github.com/*"], "Arc")
        }],
        tests: vec![
            test("https://github.com/pwrdrvr/PwrRoute", "Firefox"),
            RouteTest {
                expect: Expectation {
                    browser: "Firefox".into(),
                    url: Some("https://scribe.rip/post-1".into()),
                    ..Expectation::default()
                },
                ..test("https://someone.medium.com/post-1", "Firefox")
            },
            test("https://nothing.example/", "Safari"),
        ],
        ..RulesFile::default()
    };
    let report = file
        .run_tests(&engine(Some("stress.ts"), file.clone()))
        .unwrap();
    assert_eq!(
        (report.total, report.passed),
        (4, 3),
        "{:#?}",
        report.failures
    );
    let failure = &report.failures[0];
    assert_eq!(
        failure.source,
        TestSource::RuleExample {
            rule: 0,
            example: 0
        }
    );
    assert_eq!(
        failure.actual,
        "the config's handler 3: Firefox with https://github.com/pwrdrvr/PwrRoute"
    );
}

#[test]
fn a_test_can_name_the_opener() {
    let file = RulesFile {
        default_browser: "Safari".into(),
        rules: vec![Rule {
            opener: vec!["com.tinyspeck.slackmacgap".into()],
            ..rule(&["*.atlassian.net/*"], "Google Chrome")
        }],
        tests: vec![
            RouteTest {
                opener: Some("Slack:com.tinyspeck.slackmacgap".into()),
                ..test("https://acme.atlassian.net/browse/X-1", "Google Chrome")
            },
            RouteTest {
                opener: Some("Slack".into()),
                ..test("https://acme.atlassian.net/browse/X-1", "Safari")
            },
            test("https://acme.atlassian.net/browse/X-1", "Safari"),
        ],
        ..RulesFile::default()
    };
    // The rule wants the bundle id; "Slack" alone is used as name and id,
    // so it is not that app.
    let report = file.run_tests(&engine(None, file.clone())).unwrap();
    assert!(report.is_ok(), "{:#?}", report.failures);
    assert_eq!(report.total, 3);

    // opener.js's first handler reads opener.bundleId.
    let file = RulesFile {
        tests: vec![RouteTest {
            opener: Some("Slack:com.tinyspeck.slackmacgap".into()),
            ..test("https://anything.example/", "Google Chrome:Work")
        }],
        ..RulesFile::default()
    };
    let e = engine(Some("opener.js"), file.clone());
    let report = file.run_tests(&e).unwrap();
    assert!(report.is_ok(), "{:#?}", report.failures);
    let failure = e
        .test_rules(Some(&RulesFile {
            tests: vec![RouteTest {
                opener: None,
                ..file.tests[0].clone()
            }],
            ..file
        }))
        .unwrap();
    assert_eq!(
        failure.failures[0].actual,
        "Safari with https://anything.example/"
    );
    assert_eq!(failure.failures[0].opener, None);
}

#[test]
fn a_rewrite_example_checks_that_rewrite_alone() {
    // The second rewrite sees the first's output; its example is about what
    // it does, not the whole chain.
    let file = RulesFile {
        rewrites: vec![
            Rewrite {
                examples: vec![RewriteExample {
                    url: "https://old.example/a?utm_source=x".into(),
                    becomes: "https://new.example/a?utm_source=x".into(),
                }],
                ..Rewrite {
                    patterns: vec!["old.example/*".into()],
                    actions: vec![RewriteAction::SetHost("new.example".into())],
                    ..Rewrite::default()
                }
            },
            Rewrite {
                examples: vec![
                    RewriteExample {
                        url: "https://old.example/a?utm_source=x".into(),
                        becomes: "https://new.example/a".into(),
                    },
                    RewriteExample {
                        url: "https://other.example/".into(),
                        becomes: "https://other.example/".into(),
                    },
                ],
                ..Rewrite {
                    patterns: vec!["new.example/*".into()],
                    actions: vec![RewriteAction::DropQuery(vec!["utm_*".into()])],
                    ..Rewrite::default()
                }
            },
        ],
        ..RulesFile::default()
    };
    let report = file.run_tests(&engine(None, file.clone())).unwrap();
    assert_eq!((report.total, report.passed), (3, 2));
    let failure = &report.failures[0];
    assert_eq!(
        failure.source,
        TestSource::RewriteExample {
            rewrite: 1,
            example: 1
        }
    );
    assert_eq!(failure.actual, "rewrite 2 did not match (new.example/*)");
}

#[test]
fn problems_flag_bad_examples_tests_and_privacy_levels() {
    let file = RulesFile {
        default_browser: "Firefox".into(),
        rules: vec![Rule {
            examples: vec!["github.com/x".into()],
            privacy: Some(PrivacySetting::Unknown("secret".into())),
            ..rule(&["github.com/*"], "Safari")
        }],
        rewrites: vec![Rewrite {
            patterns: vec!["x.com/*".into()],
            actions: vec![RewriteAction::SetHost("xcancel.com".into())],
            examples: vec![RewriteExample {
                url: "https://x.com/a".into(),
                becomes: "xcancel.com/a".into(),
            }],
            ..Rewrite::default()
        }],
        tests: vec![
            test("not a url", "Firefox"),
            test("https://ok.example/", " "),
            RouteTest {
                expect: Expectation {
                    browser: "Firefox".into(),
                    url: Some("relative".into()),
                    ..Expectation::default()
                },
                ..test("https://ok.example/", "Firefox")
            },
        ],
        ..RulesFile::default()
    };
    let problems = file.problems();
    let problems: Vec<(Place, &str)> = problems
        .iter()
        .map(|p| (p.place, p.message.as_str()))
        .collect();
    assert_eq!(
        problems,
        [
            (Place::Rule(0), "Privacy must be full, host or none."),
            (
                Place::Rule(0),
                "Examples must be full URLs, such as https://example.com/path."
            ),
            (
                Place::Rewrite(0),
                "Examples must be full URLs, such as https://example.com/path."
            ),
            (
                Place::Test(0),
                "Examples must be full URLs, such as https://example.com/path."
            ),
            (Place::Test(1), "Name the browser the link must open in."),
            (Place::Test(2), "The expected URL must be a full URL."),
        ]
    );
}

// Privacy

fn privacy_of(e: &Engine, url: &str) -> (Privacy, bool) {
    let b = e.resolve(url, None).unwrap().browser;
    (b.privacy, b.private)
}

#[test]
fn the_rules_that_fired_set_the_results_privacy() {
    let e = engine(None, rules("examples-pass.json"));
    // A plain rule, and the default: everything may be kept.
    assert_eq!(
        privacy_of(&e, "https://acme.atlassian.net/browse/X-1"),
        (Privacy::Full, false)
    );
    assert_eq!(
        privacy_of(&e, "https://unrouted.example/"),
        (Privacy::Full, false)
    );
    // A private rule keeps the host unless it says otherwise, and opens a
    // private window; the host-level rewrite that matched agrees.
    assert_eq!(
        privacy_of(&e, "https://x.com/pwrdrvr/status/1"),
        (Privacy::Host, true)
    );
    assert_eq!(
        privacy_of(&e, "https://bank.example/login"),
        (Privacy::None, false)
    );
    // The trace carries the same answer.
    let trace = e
        .trace(
            &pwrroute_engine::Link::new("https://bank.example/login"),
            None,
        )
        .unwrap();
    assert_eq!(trace.resolution.browser.privacy, Privacy::None);
}

#[test]
fn the_tightest_level_of_every_matching_rewrite_and_the_handler_wins() {
    let file = RulesFile {
        default_browser: "Firefox".into(),
        rewrites: vec![
            Rewrite {
                privacy: Some(Privacy::None.into()),
                ..Rewrite {
                    patterns: vec!["*".into()],
                    actions: vec![RewriteAction::DropQuery(vec!["utm_*".into()])],
                    ..Rewrite::default()
                }
            },
            Rewrite {
                privacy: Some(Privacy::Host.into()),
                ..Rewrite {
                    patterns: vec!["never.example/*".into()],
                    actions: vec![RewriteAction::SetHost("x.example".into())],
                    ..Rewrite::default()
                }
            },
        ],
        rules: vec![
            Rule {
                private: true,
                privacy: Some(Privacy::Full.into()),
                ..rule(&["loud.example/*"], "Safari")
            },
            Rule {
                privacy: Some(PrivacySetting::Unknown("secret".into())),
                ..rule(&["odd.example/*"], "Safari")
            },
        ],
        ..RulesFile::default()
    };
    let e = engine(None, file);
    // The first rewrite matches everything and keeps nothing; a private rule
    // that says `full` cannot loosen that, but does open a private window.
    assert_eq!(
        privacy_of(&e, "https://loud.example/"),
        (Privacy::None, true)
    );
    assert_eq!(
        privacy_of(&e, "https://other.example/"),
        (Privacy::None, false)
    );
    // An unknown level keeps nothing.
    let e = engine(
        None,
        RulesFile {
            rules: vec![Rule {
                privacy: Some(PrivacySetting::Unknown("secret".into())),
                ..rule(&["odd.example/*"], "Safari")
            }],
            ..RulesFile::default()
        },
    );
    assert_eq!(
        privacy_of(&e, "https://odd.example/"),
        (Privacy::None, false)
    );
    // A private rule without a level keeps the host; with `full`, the URL.
    let e = engine(
        None,
        RulesFile {
            rules: vec![
                Rule {
                    private: true,
                    ..rule(&["a.example/*"], "Safari")
                },
                Rule {
                    private: true,
                    privacy: Some(Privacy::Full.into()),
                    ..rule(&["b.example/*"], "Safari")
                },
            ],
            ..RulesFile::default()
        },
    );
    assert_eq!(privacy_of(&e, "https://a.example/"), (Privacy::Host, true));
    assert_eq!(privacy_of(&e, "https://b.example/"), (Privacy::Full, true));
}

#[test]
fn a_js_config_is_full_and_private_only_when_its_args_say_so() {
    // functions.js's args.example handler passes --incognito itself.
    let e = engine(Some("functions.js"), RulesFile::default());
    let b = e
        .resolve("https://args.example/?q=search", None)
        .unwrap()
        .browser;
    assert_eq!(b.args[0], "--incognito");
    assert_eq!((b.privacy, b.private), (Privacy::Full, true));
    assert_eq!(
        privacy_of(&e, "https://exact.example/"),
        (Privacy::Full, false)
    );
    // rules.json rules behind a config still tighten what is kept.
    let e = engine(
        Some("functions.js"),
        RulesFile {
            rules: vec![Rule {
                privacy: Some(Privacy::Host.into()),
                ..rule(&["quiet.example/*"], "Safari")
            }],
            ..RulesFile::default()
        },
    );
    assert_eq!(
        privacy_of(&e, "https://quiet.example/"),
        (Privacy::Host, false)
    );
}

#[test]
fn privacy_fields_round_trip_through_the_file_and_stay_absent_when_unused() {
    let file = rules("examples-pass.json");
    assert_eq!(
        file.rewrites[0].privacy,
        Some(PrivacySetting::Level(Privacy::Host))
    );
    assert!(file.rules[1].private);
    assert_eq!(file.rules[1].privacy, None);
    assert_eq!(file.rules[1].effective_privacy(), Privacy::Host);
    assert_eq!(file.rules[2].effective_privacy(), Privacy::None);
    let text = serde_json::to_string(&file).unwrap();
    assert_eq!(serde_json::from_str::<RulesFile>(&text).unwrap(), file);

    // The Go-schema file comes back exactly as the Go host wrote it.
    let go = rules("go-schema.json");
    let value = serde_json::to_value(&go).unwrap();
    for rule in value["rules"].as_array().unwrap() {
        for key in ["private", "privacy", "examples"] {
            assert!(rule.get(key).is_none(), "{key} appeared");
        }
    }
    assert!(value.get("tests").is_none());
    assert!(value.get("rewrites").is_none());

    // An unknown level is kept as written, so a save does not drop it.
    let odd: RulesFile = serde_json::from_str(
        r#"{ "rules": [{ "match": "a/*", "browser": "Safari", "privacy": "secret" }] }"#,
    )
    .unwrap();
    assert_eq!(
        odd.rules[0].privacy,
        Some(PrivacySetting::Unknown("secret".into()))
    );
    assert_eq!(
        serde_json::to_value(&odd).unwrap()["rules"][0]["privacy"],
        "secret"
    );
}

// Suggesting a rule

#[test]
fn every_candidate_matches_its_own_link_by_finickys_is_match() {
    let e = engine(None, RulesFile::default());
    for url in [
        "https://github.com/pwrdrvr/PwrRoute/pull/1",
        "https://github.com/pwrdrvr?tab=repositories",
        "https://example.com",
        "http://localhost:3000/app/x?y=1#z",
        "vscode://file/Users/x/y.rs",
        "slack://channel?id=1",
        "mailto:alice@example.com",
        "https://a.example/q*",
        "https://issues.example/12345/comments",
        "https://user@host.example/a/b",
        "https://user@host.example/",
        "https://user:s3cret@host.example/a",
        "https://user:s3cret@host.example/a/b?q=1#f",
        "http://user:s3cret@localhost:3000/x",
        "https://user:s3cret@host.example/a?q=1#f",
        "https://auth.example.com/authorize?client_id=demo&state=one",
        "https://example.com/page#top",
        "mailto:alice@example.com?subject=hi",
    ] {
        let suggestion = suggest_rule(url);
        assert!(!suggestion.candidates.is_empty(), "{url}");
        for candidate in &suggestion.candidates {
            assert_eq!(
                e.collisions(candidate, &[url]).unwrap(),
                [url],
                "{candidate} should match {url}"
            );
            assert!(
                !candidate.contains("s3cret"),
                "{candidate} carries a password"
            );
        }
    }
}

#[test]
fn a_page_with_a_query_is_caught_again_with_another_query() {
    let e = engine(None, RulesFile::default());
    let first = "https://auth.example.com/authorize?client_id=demo&state=one";
    let again = "https://auth.example.com/authorize?client_id=demo&state=two";
    let pattern = &suggest_rule(first).candidates[0];
    assert_eq!(e.collisions(pattern, &[again]).unwrap(), [again]);
}

#[test]
fn collisions_say_what_else_a_pattern_would_catch() {
    let e = engine(None, RulesFile::default());
    let recent = [
        "https://github.com/pwrdrvr/PwrRoute",
        "https://github.com/other/repo/issues/1",
        "https://gist.github.com/x",
        "https://example.com/github.com/",
        "not a url",
    ];
    assert_eq!(
        e.collisions("github.com/pwrdrvr/*", &recent).unwrap(),
        ["https://github.com/pwrdrvr/PwrRoute"]
    );
    assert_eq!(
        e.collisions("github.com/*", &recent).unwrap(),
        [
            "https://github.com/pwrdrvr/PwrRoute",
            "https://github.com/other/repo/issues/1"
        ]
    );
    assert_eq!(
        e.collisions("*.github.com/*", &recent).unwrap(),
        ["https://gist.github.com/x"]
    );
    // What a handler with this pattern would do, exactly: the rule agrees.
    let routed = engine(
        None,
        RulesFile {
            default_browser: "Firefox".into(),
            rules: vec![rule(&["github.com/*"], "Safari")],
            ..RulesFile::default()
        },
    );
    for url in &recent[..4] {
        let by_rule = routed.resolve(url, None).unwrap().browser.name == "Safari";
        let by_pattern = !e.collisions("github.com/*", &[url]).unwrap().is_empty();
        assert_eq!(by_rule, by_pattern, "{url}");
    }
    assert!(e.collisions("", &recent).unwrap().is_empty());
    assert!(e.collisions("github.com/*", &[]).unwrap().is_empty());
}
