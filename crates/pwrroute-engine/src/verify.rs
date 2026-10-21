//! Running the examples and tests `rules.json` carries: each rule's example
//! URLs must resolve to that rule, each rewrite's examples must come out as
//! written, and each top-level test must end at the browser it names. They
//! run through the live engine, JS config included, so a change to either
//! side is caught before it changes old routing.
//!
//! Every example and test is a URL the user chose to save with the rules;
//! the report repeats them in memory and nothing here writes one anywhere.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{
    Browser, ConsoleLine, ConsoleLog, Engine, Expectation, Link, Opener, RulesFile, Source, Stage,
    Step, Trace,
};

/// What running a file's examples and tests found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestReport {
    /// Examples and tests run. Those of a disabled or incomplete rule or
    /// rewrite are not run, since it cannot fire.
    pub total: usize,
    pub passed: usize,
    pub failures: Vec<TestFailure>,
    /// What the config printed with `console` while the examples and tests
    /// ran, in order, the last [`ConsoleLog::CAPACITY`] lines. In memory
    /// only: a line often quotes the example it was printed for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub console: Vec<ConsoleLine>,
}

impl TestReport {
    /// Every example and test passed.
    pub fn is_ok(&self) -> bool {
        self.failures.is_empty()
    }
}

/// One example or test that did not do what the file says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestFailure {
    pub source: TestSource,
    /// The URL that was resolved.
    pub input: String,
    /// The test's opener, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opener: Option<String>,
    /// What the file asked for, in words.
    pub expected: String,
    /// What the engine did, in words.
    pub actual: String,
    /// The step whose browser was used, when the resolution reached one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub winner: Option<Step>,
}

/// Where in `rules.json` a failing example or test is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum TestSource {
    /// `rules[rule].examples[example]`.
    RuleExample { rule: usize, example: usize },
    /// `rewrites[rewrite].examples[example]`.
    RewriteExample { rewrite: usize, example: usize },
    /// `tests[test]`.
    Test { test: usize },
}

impl RulesFile {
    /// Runs this file's examples and tests through `engine`, which must have
    /// been built from this file (and the JS config in charge, if any), so
    /// that the engine's trace numbers rules the way this file does.
    /// [`Engine::test_rules`] pairs the two for a caller.
    pub fn run_tests(&self, engine: &Engine) -> Result<TestReport> {
        let mut report = TestReport::default();
        let planned_rules = self.planned_rules();
        for (i, rule) in self.rules.iter().enumerate() {
            if !planned_rules.contains(&i) {
                continue;
            }
            for (j, url) in rule.examples.iter().enumerate() {
                let trace = engine.trace(&Link::new(url), None)?;
                report.print(&trace);
                let source = TestSource::RuleExample {
                    rule: i,
                    example: j,
                };
                let outcome = match trace.winner() {
                    Some(step)
                        if step.source == Source::Rules
                            && step.stage == Stage::Handler
                            && step.index == i =>
                    {
                        None
                    }
                    _ => Some(TestFailure {
                        source,
                        input: url.clone(),
                        opener: None,
                        expected: format!("rule {} ({})", i + 1, rule.patterns.join(", ")),
                        actual: describe_winner(&trace),
                        winner: trace.winner().cloned(),
                    }),
                };
                report.record(outcome);
            }
        }
        let planned_rewrites = self.planned_rewrites();
        for (i, rewrite) in self.rewrites.iter().enumerate() {
            if !planned_rewrites.contains(&i) {
                continue;
            }
            for (j, example) in rewrite.examples.iter().enumerate() {
                let trace = engine.trace(&Link::new(&example.url), None)?;
                report.print(&trace);
                let step = trace.steps.iter().find(|s| {
                    s.source == Source::Rules && s.stage == Stage::Rewrite && s.index == i
                });
                let actual = match step {
                    None => Some("the rewrite was not reached".to_owned()),
                    Some(step) if !step.matched => Some(format!(
                        "rewrite {} did not match ({})",
                        i + 1,
                        rewrite.patterns.join(", ")
                    )),
                    Some(step) => match &step.rewritten_to {
                        Some(got) if same_url(got, &example.becomes) => None,
                        Some(got) => Some(got.clone()),
                        None => Some(
                            step.error
                                .clone()
                                .unwrap_or_else(|| "the rewrite produced nothing".to_owned()),
                        ),
                    },
                };
                report.record(actual.map(|actual| TestFailure {
                    source: TestSource::RewriteExample {
                        rewrite: i,
                        example: j,
                    },
                    input: example.url.clone(),
                    opener: None,
                    expected: example.becomes.clone(),
                    actual,
                    winner: trace.winner().cloned(),
                }));
            }
        }
        for (i, test) in self.tests.iter().enumerate() {
            let opener = test.opener.as_deref().map(opener_named);
            let trace = engine.trace(&Link::new(&test.url), opener.as_ref())?;
            report.print(&trace);
            let outcome = if test.expect.is_met_by(&trace.resolution.browser) {
                None
            } else {
                Some(TestFailure {
                    source: TestSource::Test { test: i },
                    input: test.url.clone(),
                    opener: test.opener.clone(),
                    expected: test.expect.to_string(),
                    actual: describe_browser(
                        &trace.resolution.browser,
                        trace.resolution.error.as_deref(),
                    ),
                    winner: trace.winner().cloned(),
                })
            };
            report.record(outcome);
        }
        Ok(report)
    }
}

impl TestReport {
    fn record(&mut self, failure: Option<TestFailure>) {
        self.total += 1;
        match failure {
            None => self.passed += 1,
            Some(failure) => self.failures.push(failure),
        }
    }

    /// Keeps the run's `console` lines, the newest
    /// [`ConsoleLog::CAPACITY`] of them.
    fn print(&mut self, trace: &Trace) {
        self.console.extend(trace.console.iter().cloned());
        let over = self.console.len().saturating_sub(ConsoleLog::CAPACITY);
        if over > 0 {
            self.console.drain(..over);
        }
    }
}

impl Expectation {
    /// Whether `browser` is what this expectation asks for. The browser is
    /// compared ignoring case with [`Browser::name`] as the winning rule or
    /// config wrote it (so `Safari` does not satisfy a rule that says
    /// `com.apple.Safari`); a profile or URL only when given.
    pub fn is_met_by(&self, browser: &Browser) -> bool {
        let (name, profile) = self.name_and_profile();
        name.trim().eq_ignore_ascii_case(browser.name.trim())
            && profile.is_none_or(|p| p == browser.profile)
            && self
                .url
                .as_deref()
                .is_none_or(|u| same_url(u, &browser.url))
    }

    /// `browser` and `profile`, reading `Name:profile` as Finicky's
    /// `resolveBrowserInfo` does when no profile is given on its own.
    fn name_and_profile(&self) -> (&str, Option<&str>) {
        match (&self.profile, self.browser.split_once(':')) {
            (Some(profile), _) => (&self.browser, Some(profile)),
            (None, Some((name, profile))) => (name, Some(profile)),
            (None, None) => (&self.browser, None),
        }
    }
}

impl std::fmt::Display for Expectation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (name, profile) = self.name_and_profile();
        write!(f, "{name}")?;
        if let Some(profile) = profile.filter(|p| !p.is_empty()) {
            write!(f, " ({profile})")?;
        }
        if let Some(url) = &self.url {
            write!(f, " with {url}")?;
        }
        Ok(())
    }
}

/// An opener from a test's `opener`: `Name:bundle.id`, or one word used as
/// both the name and the bundle id, so a rule's `opener` list and a config's
/// `opener.name` both see it.
fn opener_named(spec: &str) -> Opener {
    let (name, bundle_id) = spec.split_once(':').unwrap_or((spec, spec));
    Opener {
        name: name.trim().to_owned(),
        bundle_id: bundle_id.trim().to_owned(),
        ..Opener::default()
    }
}

fn describe_winner(trace: &Trace) -> String {
    let where_ = match trace.winner() {
        Some(step) => match (step.stage, step.source) {
            (Stage::Default, _) => "the default browser".to_owned(),
            (_, Source::Rules) => format!("rule {}", step.index + 1),
            (_, Source::Config) => format!("the config's handler {}", step.index + 1),
        },
        None => "no handler".to_owned(),
    };
    format!(
        "{where_}: {}",
        describe_browser(&trace.resolution.browser, trace.resolution.error.as_deref())
    )
}

fn describe_browser(browser: &Browser, error: Option<&str>) -> String {
    let mut text = browser.name.clone();
    if !browser.profile.is_empty() {
        text.push_str(&format!(" ({})", browser.profile));
    }
    text.push_str(&format!(" with {}", browser.url));
    if let Some(error) = error {
        text.push_str(&format!(", after an error: {error}"));
    }
    text
}

/// Whether two URLs are the same once parsed: the engine re-serializes every
/// URL (`https://example.com` comes back with a trailing slash), so a string
/// comparison would fail a correct example.
fn same_url(a: &str, b: &str) -> bool {
    match (url::Url::parse(a.trim()), url::Url::parse(b.trim())) {
        (Ok(a), Ok(b)) => a == b,
        _ => a.trim() == b.trim(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppType, Privacy};

    fn browser(name: &str, profile: &str, url: &str) -> Browser {
        Browser {
            name: name.into(),
            app_type: AppType::AppName,
            url: url.into(),
            profile: profile.into(),
            args: Vec::new(),
            open_in_background: None,
            private: false,
            privacy: Privacy::Full,
        }
    }

    #[test]
    fn an_expectation_compares_only_what_it_names() {
        let firefox = browser("Firefox", "Work", "https://example.com/a?b=1");
        let only_browser = Expectation {
            browser: "firefox".into(),
            ..Expectation::default()
        };
        assert!(only_browser.is_met_by(&firefox));
        let with_profile = Expectation {
            browser: "Firefox:Work".into(),
            ..Expectation::default()
        };
        assert!(with_profile.is_met_by(&firefox));
        let wrong_profile = Expectation {
            browser: "Firefox".into(),
            profile: Some("Personal".into()),
            ..Expectation::default()
        };
        assert!(!wrong_profile.is_met_by(&firefox));
        let with_url = Expectation {
            browser: "Firefox".into(),
            url: Some("https://example.com/a?b=1".into()),
            ..Expectation::default()
        };
        assert!(with_url.is_met_by(&firefox));
        assert_eq!(
            with_url.to_string(),
            "Firefox with https://example.com/a?b=1"
        );
        assert_eq!(with_profile.to_string(), "Firefox (Work)");
    }

    #[test]
    fn urls_compare_once_parsed() {
        assert!(same_url("https://example.com", "https://example.com/"));
        assert!(same_url("HTTPS://Example.com/x", "https://example.com/x"));
        assert!(!same_url("https://example.com/x", "https://example.com/y"));
        assert!(same_url("not a url", "not a url"));
    }

    #[test]
    fn an_opener_spec_names_the_app_and_its_bundle_id() {
        let both = opener_named("Slack:com.tinyspeck.slackmacgap");
        assert_eq!(
            (both.name.as_str(), both.bundle_id.as_str()),
            ("Slack", "com.tinyspeck.slackmacgap")
        );
        let one = opener_named("Slack");
        assert_eq!(
            (one.name.as_str(), one.bundle_id.as_str()),
            ("Slack", "Slack")
        );
    }
}
