//! The editor's Try it: what the selected rule makes of a sample link,
//! from the selected rule's point of view.
//!
//! The rail runs its own [`Engine`] over the rules as edited, drafts and
//! all, so a new handler can say "this handler would take it" before it is
//! complete enough to be saved. Test a link resolves the saved file instead.
//!
//! [`TestRuns`] runs the file's examples and tests the same way, over the
//! config in charge and the rules as edited, and keeps the report until the
//! rules change; the Rules screen's chips and Run tests strip read it. The
//! examples and tests are URLs the user saved into `rules.json`; nothing
//! here writes one anywhere.

use std::{path::PathBuf, rc::Rc, time::SystemTime};

use pwrroute_engine::{
    DefaultHost, Engine, Limits, Link, Opener, RulesFile, Source, Stage, Step, TestFailure,
    TestReport, TestSource, Trace,
};

use crate::app::Selection;

/// The dot's color beside the result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The rule takes the link.
    Takes,
    /// An earlier rule takes it first.
    Beaten,
    /// The rule does not match, or is off.
    Passes,
    /// The link could not be resolved, or the rule has problems.
    Broken,
}

/// What the rail says under the sample link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TryResult {
    /// Plain text with an optional run in bold.
    Text {
        verdict: Verdict,
        message: String,
        bold: Option<String>,
    },
    /// A rewrite changed the link: the URL before and after, then who
    /// opens it.
    Rewrote {
        before: String,
        after: String,
        then: String,
    },
}

impl TryResult {
    fn text(verdict: Verdict, message: impl Into<String>) -> Self {
        Self::Text {
            verdict,
            message: message.into(),
            bold: None,
        }
    }
}

/// An engine over the edited rules, rebuilt when they change.
pub struct Probe {
    config: Option<PathBuf>,
    rules: RulesFile,
    engine: Option<Engine>,
}

impl Probe {
    pub fn new(config: Option<PathBuf>, rules: RulesFile) -> Self {
        Self {
            config,
            rules,
            engine: None,
        }
    }

    /// The rules changed: the next probe loads them.
    pub fn set_rules(&mut self, rules: RulesFile) {
        if self.rules != rules {
            self.rules = rules;
            self.engine = None;
        }
    }

    /// The engine over the rules as they stand, built on first use.
    fn engine(&mut self) -> Result<&Engine, String> {
        if self.engine.is_none() {
            self.engine = Some(load(self.config.as_deref(), &self.rules)?);
        }
        self.engine
            .as_ref()
            .ok_or_else(|| "The rules did not load.".to_owned())
    }

    /// What `selection` makes of `sample`, as from `opener`.
    pub fn run(&mut self, sample: &str, opener: Option<&str>, selection: Selection) -> TryResult {
        let url = match pwrroute_router::normalize_url(sample) {
            Ok(url) => url,
            Err(error) => {
                return TryResult::text(
                    Verdict::Broken,
                    format!("Not a link the router accepts: {error}."),
                );
            }
        };
        let opener = opener.map(|app| Opener {
            name: app.to_owned(),
            bundle_id: app.to_owned(),
            ..Opener::default()
        });
        match self.trace(&url, opener.as_ref()) {
            Ok(trace) => interpret(&trace, &self.rules, selection),
            Err(error) => TryResult::text(Verdict::Broken, error),
        }
    }

    /// Resolves `url` (already normalized) as from `opener`, with every
    /// step.
    pub fn trace(&mut self, url: &str, opener: Option<&Opener>) -> Result<Trace, String> {
        self.engine()?
            .trace(&Link::new(url), opener)
            .map_err(|error| format!("The rules failed: {error:#}"))
    }

    /// Which of `urls` `pattern` would match, by Finicky's own `isMatch`:
    /// what a candidate rule would also catch.
    pub fn collisions(&mut self, pattern: &str, urls: &[&str]) -> Result<Vec<String>, String> {
        self.engine()?
            .collisions(pattern, urls)
            .map_err(|error| format!("The pattern could not be matched: {error:#}"))
    }

    /// The handler of `rules.json` that takes `url`, as from `opener`, by
    /// its position in the file; `None` when the default, a config handler
    /// or nothing takes it.
    pub fn winning_handler(&mut self, url: &str, opener: Option<&Opener>) -> Option<usize> {
        let trace = self.trace(url, opener).ok()?;
        let winner = trace.winner()?;
        (winner.source == Source::Rules && winner.stage == Stage::Handler).then_some(winner.index)
    }
}

/// An engine over `config` (when one is in charge) and `rules`, as the
/// router loads the two together.
fn load(config: Option<&std::path::Path>, rules: &RulesFile) -> Result<Engine, String> {
    let sources = pwrroute_engine::Sources {
        config: config.map(std::path::Path::to_path_buf),
        rules: rules.clone(),
    };
    Engine::from_sources(&sources, Rc::new(DefaultHost), Limits::default())
        .map_err(|error| format!("The rules did not load: {error:#}"))
}

/// The file's examples and tests, run through an engine over the config in
/// charge and the rules as edited (so a rule's report sits on its row), and
/// the report kept until the rules change.
pub struct TestRuns {
    config: Option<PathBuf>,
    rules: RulesFile,
    report: Option<Result<TestReport, String>>,
    ran_at: Option<SystemTime>,
}

impl TestRuns {
    pub fn new(config: Option<PathBuf>, rules: RulesFile) -> Self {
        Self {
            config,
            rules,
            report: None,
            ran_at: None,
        }
    }

    /// The rules changed: the report is stale until [`TestRuns::run`].
    pub fn set_rules(&mut self, rules: RulesFile) {
        if self.rules != rules {
            self.rules = rules;
            self.report = None;
        }
    }

    /// Runs every example and test now and keeps the report.
    pub fn run(&mut self) -> &Result<TestReport, String> {
        let report = load(self.config.as_deref(), &self.rules).and_then(|engine| {
            engine
                .test_rules(None)
                .map_err(|error| format!("The tests did not run: {error:#}"))
        });
        self.report = Some(report);
        self.ran_at = Some(SystemTime::now());
        self.report.as_ref().expect("just set")
    }

    /// The last report, when the rules have not changed since it ran.
    pub fn report(&self) -> Option<&Result<TestReport, String>> {
        self.report.as_ref()
    }

    /// When the report was made.
    pub fn ran_at(&self) -> Option<SystemTime> {
        self.ran_at
    }

    /// How many examples the rules carry (handlers and rewrites), and how
    /// many saved tests.
    pub fn counts(&self) -> (usize, usize) {
        let examples: usize = self
            .rules
            .rules
            .iter()
            .map(|r| r.examples.len())
            .sum::<usize>()
            + self
                .rules
                .rewrites
                .iter()
                .map(|r| r.examples.len())
                .sum::<usize>();
        (examples, self.rules.tests.len())
    }

    /// The failure of one example or test, if it failed in the last report.
    pub fn failure(&self, source: TestSource) -> Option<&TestFailure> {
        self.report
            .as_ref()?
            .as_ref()
            .ok()?
            .failures
            .iter()
            .find(|f| f.source == source)
    }

    /// Whether `source` ran and passed in the last report: `None` while
    /// there is no report.
    pub fn passed(&self, source: TestSource) -> Option<bool> {
        let report = self.report.as_ref()?.as_ref().ok()?;
        Some(!report.failures.iter().any(|f| f.source == source))
    }
}

/// The trace from the selected rule's point of view.
pub fn interpret(trace: &Trace, rules: &RulesFile, selection: Selection) -> TryResult {
    let browser = {
        let b = &trace.resolution.browser;
        if b.profile.is_empty() {
            b.name.clone()
        } else {
            format!("{} \u{00b7} {}", b.name, b.profile)
        }
    };
    let winner = trace.winner();
    let winner_name = winner.map_or_else(|| "Everything else".to_owned(), step_name);
    match selection {
        Selection::Handler(h) => {
            let enabled = rules.rules.get(h).is_none_or(|r| r.enabled);
            if !enabled {
                return TryResult::text(
                    Verdict::Passes,
                    format!("Switched off: {winner_name} opens that link in {browser}."),
                );
            }
            match winner {
                Some(step) if is_rule(step, Stage::Handler, h) => TryResult::text(
                    Verdict::Takes,
                    format!("This handler opens that link in {browser}."),
                ),
                Some(step) if step.stage == Stage::Handler && step_before(step, h) => {
                    TryResult::Text {
                        verdict: Verdict::Beaten,
                        message: format!(
                            "{winner_name} wins first: {browser}. Move this rule above it to \
                             change that."
                        ),
                        bold: Some(browser),
                    }
                }
                _ => TryResult::text(
                    Verdict::Passes,
                    format!("No match. {winner_name} opens that link in {browser}."),
                ),
            }
        }
        Selection::Rewrite(r) => {
            let enabled = rules.rewrites.get(r).is_none_or(|w| w.enabled);
            if !enabled {
                return TryResult::text(
                    Verdict::Passes,
                    "Switched off: the link passes through unchanged.",
                );
            }
            let step = trace.steps.iter().find(|s| is_rule(s, Stage::Rewrite, r));
            match step.and_then(|s| s.rewritten_to.as_deref().map(|to| (s, to))) {
                Some((step, to)) if to != step.url => TryResult::Rewrote {
                    before: step.url.clone(),
                    after: to.to_owned(),
                    then: format!("{winner_name} \u{00b7} {browser} opens it."),
                },
                Some(_) => TryResult::text(
                    Verdict::Takes,
                    format!(
                        "Matches, and changes nothing here. {winner_name} \u{00b7} {browser} opens it."
                    ),
                ),
                None => TryResult::text(
                    Verdict::Passes,
                    "No match: the link passes through unchanged.",
                ),
            }
        }
    }
}

/// A rule of `rules.json` at position `index` in its list.
fn is_rule(step: &Step, stage: Stage, index: usize) -> bool {
    step.source == Source::Rules && step.stage == stage && step.index == index
}

/// A handler that runs before handler `h` of `rules.json`: the config's
/// all do.
fn step_before(step: &Step, h: usize) -> bool {
    match step.source {
        Source::Config => true,
        Source::Rules => step.index < h,
    }
}

/// How a step is named in the result: "Handler 2", "Config handler 1".
fn step_name(step: &Step) -> String {
    match (step.stage, step.source) {
        (Stage::Default, _) => "Everything else".into(),
        (Stage::Handler, Source::Rules) => format!("Handler {}", step.index + 1),
        (Stage::Handler, Source::Config) => format!("Config handler {}", step.index + 1),
        (Stage::Rewrite, Source::Rules) => format!("Rewrite {}", step.index + 1),
        (Stage::Rewrite, Source::Config) => format!("Config rewrite {}", step.index + 1),
    }
}

/// A link to start Try it with, from a rule's first pattern: `x.com/*`
/// gives `https://x.com/`.
pub fn sample_url(patterns: &[String]) -> String {
    let pattern = patterns
        .iter()
        .map(|p| p.trim())
        .find(|p| !p.is_empty())
        .unwrap_or("example.com/*");
    if pattern.starts_with('/') {
        // A regex: nothing to derive a link from.
        return "https://example.com/".into();
    }
    let (scheme, rest) = match pattern.split_once("://") {
        Some((scheme, rest)) if !scheme.contains('*') => (scheme, rest),
        _ => ("https", pattern.trim_start_matches("*://")),
    };
    let mut rest = rest
        .replace(":*", ":3000")
        .replace("*.", "www.")
        .replace('*', "");
    if rest.is_empty() || rest == "www." {
        rest = "example.com/".into();
    }
    if !rest.contains('/') {
        rest.push('/');
    }
    format!("{scheme}://{rest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe() -> Probe {
        Probe::new(None, crate::fixtures::rules_file())
    }

    #[test]
    fn a_handler_says_who_takes_the_link() {
        let mut probe = probe();
        // Handler 2 takes Atlassian links.
        let taken = probe.run(
            "https://example.atlassian.net/browse/PG-12",
            None,
            Selection::Handler(1),
        );
        assert_eq!(
            taken,
            TryResult::text(
                Verdict::Takes,
                "This handler opens that link in Google Chrome \u{00b7} Work."
            )
        );
        // Handler 4 (Social) loses to handler 2 on that link.
        let beaten = probe.run(
            "https://example.atlassian.net/browse/PG-12",
            None,
            Selection::Handler(3),
        );
        assert_eq!(
            beaten,
            TryResult::Text {
                verdict: Verdict::Beaten,
                message: "Handler 2 wins first: Google Chrome \u{00b7} Work. Move this rule \
                          above it to change that."
                    .into(),
                bold: Some("Google Chrome \u{00b7} Work".into()),
            }
        );
        // Handler 1 does not match it: a later handler does.
        let passes = probe.run(
            "https://example.atlassian.net/browse/PG-12",
            None,
            Selection::Handler(0),
        );
        assert_eq!(
            passes,
            TryResult::text(
                Verdict::Passes,
                "No match. Handler 2 opens that link in Google Chrome \u{00b7} Work."
            )
        );
        // Handler 3 needs Slack as the opener: the probe passes its app.
        let slack = probe.run(
            "https://example.com/",
            Some("com.tinyspeck.slackmacgap"),
            Selection::Handler(2),
        );
        assert!(
            matches!(
                slack,
                TryResult::Text {
                    verdict: Verdict::Takes,
                    ..
                }
            ),
            "{slack:?}"
        );
        // Handler 7 is off.
        let off = probe.run("https://figma.com/file/1", None, Selection::Handler(6));
        assert_eq!(
            off,
            TryResult::text(
                Verdict::Passes,
                "Switched off: Everything else opens that link in Google Chrome \u{00b7} Personal."
            )
        );
    }

    #[test]
    fn a_rewrite_shows_before_and_after() {
        let mut probe = probe();
        let rewrote = probe.run(
            "https://x.com/pwrdrvr/status/1842?s=20",
            None,
            Selection::Rewrite(0),
        );
        assert_eq!(
            rewrote,
            TryResult::Rewrote {
                before: "https://x.com/pwrdrvr/status/1842?s=20".into(),
                after: "https://xcancel.com/pwrdrvr/status/1842?s=20".into(),
                then: "Everything else \u{00b7} Google Chrome \u{00b7} Personal opens it.".into(),
            }
        );
        let passes = probe.run("https://example.com/", None, Selection::Rewrite(0));
        assert_eq!(
            passes,
            TryResult::text(
                Verdict::Passes,
                "No match: the link passes through unchanged."
            )
        );
        let not_a_link = probe.run("not a link", None, Selection::Rewrite(0));
        assert!(matches!(
            not_a_link,
            TryResult::Text {
                verdict: Verdict::Broken,
                ..
            }
        ));
    }

    #[test]
    fn a_draft_handler_is_probed_as_edited() {
        let mut rules = crate::fixtures::rules_file();
        rules.rules.push(pwrroute_engine::Rule {
            patterns: vec!["notion.so/*".into()],
            browser: "Firefox".into(),
            ..pwrroute_engine::Rule::default()
        });
        let mut probe = Probe::new(None, RulesFile::default());
        probe.set_rules(rules);
        let result = probe.run(
            "https://notion.so/pwrdrvr/Roadmap",
            None,
            Selection::Handler(7),
        );
        assert_eq!(
            result,
            TryResult::text(Verdict::Takes, "This handler opens that link in Firefox.")
        );
    }

    #[test]
    fn test_runs_report_each_example_and_test_by_its_place() {
        let mut rules = crate::fixtures::rules_file();
        rules.rules[1].examples = vec![
            "https://pwrdrvr.atlassian.net/browse/PG-12".into(),
            "https://github.com/pwrdrvr/PwrGit/pull/361".into(),
        ];
        rules.rewrites[0].examples = vec![pwrroute_engine::RewriteExample {
            url: "https://x.com/a".into(),
            becomes: "https://xcancel.com/a".into(),
        }];
        rules.tests = vec![pwrroute_engine::RouteTest {
            url: "https://www.apple.com/macos/".into(),
            opener: None,
            expect: pwrroute_engine::Expectation {
                browser: "Google Chrome".into(),
                profile: Some("Personal".into()),
                url: None,
            },
        }];
        let mut runs = TestRuns::new(None, rules.clone());
        assert!(runs.report().is_none());
        assert_eq!(runs.counts(), (3, 1));
        let report = runs.run().as_ref().expect("the tests ran");
        assert_eq!((report.total, report.passed), (4, 4));
        let example = TestSource::RuleExample {
            rule: 1,
            example: 1,
        };
        assert_eq!(runs.passed(example), Some(true));

        // A new handler above takes the GitHub example: that one fails.
        rules.rules.insert(
            1,
            pwrroute_engine::Rule {
                patterns: vec!["github.com/*".into()],
                browser: "Firefox".into(),
                ..pwrroute_engine::Rule::default()
            },
        );
        runs.set_rules(rules);
        assert!(runs.report().is_none(), "stale until run");
        let report = runs.run().as_ref().expect("the tests ran");
        assert_eq!((report.total, report.passed), (4, 3));
        let example = TestSource::RuleExample {
            rule: 2,
            example: 1,
        };
        assert_eq!(runs.passed(example), Some(false));
        let failure = runs.failure(example).expect("the failure");
        assert_eq!(failure.input, "https://github.com/pwrdrvr/PwrGit/pull/361");
        assert_eq!(failure.winner.as_ref().map(|w| w.index), Some(1));
        assert!(runs.ran_at().is_some());
    }

    #[test]
    fn the_probe_finds_collisions_and_the_winning_handler() {
        let mut probe = probe();
        let recent = [
            "https://github.com/pwrdrvr/PwrGit/pull/361",
            "https://github.com/rust-lang/rust",
            "https://example.com/",
            "not a url",
        ];
        assert_eq!(
            probe.collisions("github.com/*", &recent).unwrap(),
            [
                "https://github.com/pwrdrvr/PwrGit/pull/361",
                "https://github.com/rust-lang/rust"
            ]
        );
        assert_eq!(
            probe.winning_handler("https://github.com/pwrdrvr/PwrGit/pull/361", None),
            Some(1)
        );
        assert_eq!(probe.winning_handler("https://example.com/", None), None);
    }

    #[test]
    fn samples_come_from_the_first_pattern() {
        let s = |patterns: &[&str]| {
            sample_url(&patterns.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>())
        };
        assert_eq!(s(&["x.com/*", "twitter.com/*"]), "https://x.com/");
        assert_eq!(s(&["*.atlassian.net/*"]), "https://www.atlassian.net/");
        assert_eq!(s(&["http://*.internal/*"]), "http://www.internal/");
        assert_eq!(s(&["localhost:*"]), "https://localhost:3000/");
        assert_eq!(s(&["*"]), "https://example.com/");
        assert_eq!(s(&[]), "https://example.com/");
        assert_eq!(
            s(&["", "github.com/pwrdrvr/*"]),
            "https://github.com/pwrdrvr/"
        );
        assert_eq!(s(&["/^https:\\/\\/news/"]), "https://example.com/");
    }
}
