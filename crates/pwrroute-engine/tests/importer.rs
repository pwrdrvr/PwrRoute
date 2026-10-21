//! The Finicky importer's review of a config (`Engine::review_import`):
//! which rules a visual rule holds exactly, which stay JS, and which get a
//! suggestion found by probing the function beside a candidate pattern.

use std::{path::Path, rc::Rc};

use pwrroute_engine::{
    DefaultHost, Engine, Limits, Review, Rewrite, RewriteAction, Rule, RulesFile, Sources,
    Suggested, Verdict, Visual,
};

fn review(recent: &[&str]) -> Review {
    let config = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/configs/importer.ts");
    let engine = Engine::load(&config, Rc::new(DefaultHost), Limits::default()).unwrap();
    engine.review_import(recent).unwrap().unwrap()
}

fn handler(patterns: &[&str], browser: &str, profile: &str) -> Rule {
    Rule {
        patterns: patterns.iter().map(|p| (*p).to_owned()).collect(),
        browser: browser.into(),
        profile: profile.into(),
        ..Rule::default()
    }
}

fn suggestion(review: &Review, index: usize) -> Option<&Suggested> {
    match &review.handlers[index].verdict {
        Verdict::StaysJs { suggestion, .. } => suggestion.as_ref(),
        Verdict::Imports(_) => panic!("handler {index} imports"),
    }
}

fn stays(review: &Review, index: usize) -> bool {
    matches!(review.handlers[index].verdict, Verdict::StaysJs { .. })
}

#[test]
fn wildcards_browser_names_string_rewrites_and_the_default_import_exactly() {
    let review = review(&[]);
    assert_eq!(
        review.rewrites[0].verdict,
        Verdict::Imports(Visual::Rewrite(Rewrite {
            patterns: vec!["short.example/*".into()],
            actions: vec![RewriteAction::Url("https://long.example/".into())],
            ..Rewrite::default()
        }))
    );
    assert!(matches!(
        review.rewrites[1].verdict,
        Verdict::StaysJs {
            suggestion: None,
            ..
        }
    ));
    assert_eq!(
        review.handlers[0].verdict,
        Verdict::Imports(Visual::Handler(handler(
            &["plain.example/*"],
            "Firefox",
            ""
        )))
    );
    // "Firefox:work" is Firefox with the work profile, as Finicky reads it.
    assert_eq!(
        review.handlers[1].verdict,
        Verdict::Imports(Visual::Handler(handler(
            &["one.example/*", "*.one.example/*"],
            "Firefox",
            "work"
        )))
    );
    assert_eq!(
        review.default_browser.verdict,
        Verdict::Imports(Visual::Default {
            browser: "Google Chrome".into(),
            profile: "Personal".into()
        })
    );
}

#[test]
fn regexes_functions_browser_functions_and_arguments_stay_js() {
    let review = review(&[]);
    // rules.json holds no regex.
    assert!(stays(&review, 2));
    // A browser function, and launch arguments, have no visual form.
    assert!(stays(&review, 9));
    assert!(suggestion(&review, 9).is_none());
    assert!(stays(&review, 10));
    // A function on the opener has no literal that names a link.
    assert!(suggestion(&review, 8).is_none());
}

#[test]
fn helper_matchers_get_the_pattern_they_agree_with_on_every_probe() {
    let review = review(&[]);
    let alpha = suggestion(&review, 3).expect("hostnameIs");
    assert_eq!(
        alpha.rule,
        handler(&["alpha.example/*"], "Google Chrome", "Work")
    );
    assert_eq!(alpha.agreed, review.probes);
    assert!(alpha.matched >= 3, "the host, a deep link and http");
    assert_eq!(
        suggestion(&review, 4).expect("githubOrg").rule,
        handler(&["github.com/acme/*"], "Google Chrome", "Work")
    );
    assert_eq!(
        suggestion(&review, 5).expect("hostname ===").rule,
        handler(&["beta.example/*"], "Firefox", "default-release")
    );
    assert_eq!(review.recent, 0);
}

#[test]
fn a_function_that_disagrees_on_a_negative_probe_gets_no_suggestion() {
    let review = review(&[]);
    // endsWith also takes notgamma.example, which neither gamma.example/*
    // nor *.gamma.example/* does.
    assert!(stays(&review, 6));
    assert!(suggestion(&review, 6).is_none());
}

#[test]
fn recent_links_join_the_probes() {
    // Built from its literals alone, delta's function agrees with
    // delta.example/*; a recent link under /admin shows it does not.
    let quiet = review(&[]);
    assert!(suggestion(&quiet, 7).is_some());
    let review = review(&["https://delta.example/admin/x", "not a url"]);
    assert!(suggestion(&review, 7).is_none());
    assert_eq!(review.recent, 1);
    assert_eq!(review.probes, quiet.probes + 1);
    // The others still agree, on one more link.
    assert_eq!(suggestion(&review, 3).unwrap().agreed, review.probes);
}

#[test]
fn each_rule_shows_its_own_source_lines() {
    let review = review(&[]);
    let plain = review.handlers[0].source.as_ref().unwrap();
    let hit: Vec<&str> = plain
        .hit
        .clone()
        .map(|line| plain.lines[line - plain.first].trim())
        .collect();
    assert_eq!(
        hit,
        ["match: \"plain.example/*\",", "browser: \"Firefox\","]
    );
    let alpha = review.handlers[3].source.as_ref().unwrap();
    assert!(alpha.lines[*alpha.hit.start() - alpha.first].contains("alpha.example"));
    let default = review.default_browser.source.as_ref().unwrap();
    assert!(default.lines[*default.hit.start() - default.first].contains("defaultBrowser"));
}

#[test]
fn rules_json_alone_has_nothing_to_review() {
    let engine = Engine::from_sources(
        &Sources {
            config: None,
            rules: RulesFile::default(),
        },
        Rc::new(DefaultHost),
        Limits::default(),
    )
    .unwrap();
    assert!(engine.review_import(&[]).unwrap().is_none());
}

#[test]
fn an_imported_rule_routes_as_the_config_did() {
    // The imported handlers and default, on their own in rules.json, send
    // the config's own links where the config sent them.
    let review = review(&[]);
    let mut rules = RulesFile::default();
    for reviewed in &review.handlers {
        if let Verdict::Imports(Visual::Handler(rule)) = &reviewed.verdict {
            rules.rules.push(rule.clone());
        }
    }
    if let Verdict::Imports(Visual::Default { browser, profile }) = &review.default_browser.verdict
    {
        rules.default_browser = browser.clone();
        rules.default_profile = profile.clone();
    }
    let visual = Engine::from_sources(
        &Sources {
            config: None,
            rules,
        },
        Rc::new(DefaultHost),
        Limits::default(),
    )
    .unwrap();
    let config = Engine::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/configs/importer.ts"),
        Rc::new(DefaultHost),
        Limits::default(),
    )
    .unwrap();
    for url in [
        "https://plain.example/x",
        "https://a.one.example/y",
        "https://elsewhere.example/",
    ] {
        let want = config.resolve(url, None).unwrap().browser;
        let got = visual.resolve(url, None).unwrap().browser;
        assert_eq!((got.name, got.profile), (want.name, want.profile), "{url}");
    }
}
