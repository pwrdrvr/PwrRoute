//! The editor's Try it: what the selected rule makes of a sample link,
//! from the selected rule's point of view.
//!
//! The rail runs its own [`Engine`] over the rules as edited, drafts and
//! all, so a new handler can say "this handler would take it" before it is
//! complete enough to be saved. Test a link resolves the saved file instead.
//!
//! With a Finicky config in charge the result follows `rules.json`'s
//! `precedence`, read from the trace rather than guessed: under the Go
//! order a config handler that wins took the link before this handler ran,
//! and the advice is to let the rules run first, never to move the rule;
//! under `"pwrroute"` the config's handlers run after every rule, so one
//! that wins means this handler did not match. The default is named by its
//! owner: PwrRoute's or the config's.
//!
//! [`TestRuns`] runs the file's examples and tests the same way, over the
//! config in charge and the rules as edited, and keeps the report until the
//! rules change; the Rules screen's chips and Run tests strip read it. The
//! examples and tests are URLs the user saved into `rules.json`; nothing
//! here writes one anywhere.

use std::{
    path::{Path, PathBuf},
    rc::Rc,
    time::SystemTime,
};

use pwrroute_engine::{
    Engine, Limits, Link, NativeHost, Opener, RulesFile, Source, Stage, Step, TestFailure,
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
    /// The config as the user knows it (`~/.finicky.ts`), for the result.
    config_name: Option<String>,
    rules: RulesFile,
    engine: Option<Engine>,
}

impl Probe {
    /// A probe over `config` (when one is in charge) and `rules`, naming
    /// the config by its full path; the window uses [`Probe::at_home`].
    #[cfg(test)]
    pub fn new(config: Option<PathBuf>, rules: RulesFile) -> Self {
        Self::at_home(config, None, rules)
    }

    /// [`Probe::new`], with `home` shortening the config's path to `~/…`
    /// where the result names it, as the window shows it.
    pub fn at_home(config: Option<PathBuf>, home: Option<&Path>, rules: RulesFile) -> Self {
        Self {
            config_name: config
                .as_deref()
                .map(|path| crate::config_view::display_path(path, home)),
            config,
            rules,
            engine: None,
        }
    }

    /// A probe over the same config and `rules` instead: what a change
    /// would do before it is made.
    pub fn with_rules(&self, rules: RulesFile) -> Self {
        Self {
            config: self.config.clone(),
            config_name: self.config_name.clone(),
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
            Ok(trace) => interpret(&trace, &self.rules, self.config_name.as_deref(), selection),
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
    // The OS answers the config's finicky.* calls, as it does for the
    // router, so an example that depends on them is tested as it routes.
    Engine::from_sources(&sources, Rc::new(NativeHost::new()), Limits::default())
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

/// The trace from the selected rule's point of view. `config` is the
/// config in charge as the user knows it (`~/.finicky.ts`), `None` when
/// `rules.json` is the whole config.
pub fn interpret(
    trace: &Trace,
    rules: &RulesFile,
    config: Option<&str>,
    selection: Selection,
) -> TryResult {
    let browser = {
        let b = &trace.resolution.browser;
        if b.profile.is_empty() {
            b.name.clone()
        } else {
            format!("{} \u{00b7} {}", b.name, b.profile)
        }
    };
    let winner = trace.winner();
    let winner_name = winner.map_or_else(
        || EVERYTHING_ELSE.to_owned(),
        |step| step_name(step, config),
    );
    match selection {
        Selection::Handler(h) => {
            let enabled = rules.rules.get(h).is_none_or(|r| r.enabled);
            if !enabled {
                return TryResult::text(
                    Verdict::Passes,
                    format!("Switched off: {winner_name} opens that link in {browser}."),
                );
            }
            // Whether this handler was asked at all: openUrl stops at the
            // first match, or at a matcher that throws.
            let ran = trace.steps.iter().any(|s| is_rule(s, Stage::Handler, h));
            let threw = trace.steps.iter().find(|s| s.error.is_some());
            match winner {
                Some(step) if is_rule(step, Stage::Handler, h) => TryResult::text(
                    Verdict::Takes,
                    format!("This handler opens that link in {browser}."),
                ),
                Some(step)
                    if step.stage == Stage::Handler
                        && step_before(step, h, rules.rules_first()) =>
                {
                    let advice = match step.source {
                        Source::Rules => "Move this rule above it to change that.".to_owned(),
                        // The Go order: no place in rules.json is above it.
                        Source::Config => format!(
                            "{}'s handlers run before your rules; let PwrRoute's default \
                             take everything else to run yours first.",
                            config.unwrap_or("The config")
                        ),
                    };
                    TryResult::Text {
                        verdict: Verdict::Beaten,
                        message: format!("{winner_name} wins first: {browser}. {advice}"),
                        bold: Some(browser),
                    }
                }
                // A matcher before this one threw: Finicky stopped there.
                _ => match threw.filter(|_| !ran) {
                    Some(threw) => TryResult::text(
                        Verdict::Beaten,
                        format!(
                            "{} threw before this handler ran: {}. {winner_name} opens that \
                             link in {browser}.",
                            step_name(threw, config),
                            threw.error.as_deref().unwrap_or_default()
                        ),
                    ),
                    None => TryResult::text(
                        Verdict::Passes,
                        format!("No match. {winner_name} opens that link in {browser}."),
                    ),
                },
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

/// The default's name with no config in charge, where it is the only one.
const EVERYTHING_ELSE: &str = "Everything else";

/// A rule of `rules.json` at position `index` in its list.
fn is_rule(step: &Step, stage: Stage, index: usize) -> bool {
    step.source == Source::Rules && step.stage == stage && step.index == index
}

/// Whether `step`, a handler, runs before handler `h` of `rules.json`: an
/// earlier rule, or any of the config's under the Go order. With
/// `rules_first` (`"precedence": "pwrroute"`) the config's run after every
/// rule, so one that wins only took what no rule matched.
fn step_before(step: &Step, h: usize, rules_first: bool) -> bool {
    match step.source {
        Source::Config => !rules_first,
        Source::Rules => step.index < h,
    }
}

/// How a step is named in the result: "Handler 2", "Config handler 1", and
/// the default by its owner when a config is in charge: "PwrRoute's
/// default", "~/.finicky.ts's default".
fn step_name(step: &Step, config: Option<&str>) -> String {
    match (step.stage, step.source) {
        (Stage::Default, source) => match (config, source) {
            (None, _) => EVERYTHING_ELSE.into(),
            (Some(_), Source::Rules) => "PwrRoute's default".into(),
            (Some(config), Source::Config) => format!("{config}'s default"),
        },
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

    // Precedence: with a Finicky config in charge the advice follows who
    // actually goes first.

    /// A config beside the rules: handlers for both.example.com and
    /// config.example.com, one that throws for boom.example.com, Safari for
    /// the rest. Written under `home`, so the result names it `~/finicky.js`.
    fn config_beside(home: &Path) -> PathBuf {
        let path = home.join("finicky.js");
        std::fs::write(
            &path,
            r#"export default {
  defaultBrowser: "Safari",
  handlers: [
    { match: "https://both.example.com/*", browser: "Firefox" },
    { match: "https://config.example.com/*", browser: "Safari" },
    {
      match: (url) => {
        if (url.hostname === "boom.example.com") throw new Error("handler matcher threw");
        return false;
      },
      browser: "Opera",
    },
  ],
};"#,
        )
        .expect("write the config");
        path
    }

    /// Rules for both.example.com and other.example.com, Vivaldi · Work for
    /// everything else, in `precedence` order.
    fn rules_beside(precedence: pwrroute_engine::Precedence) -> RulesFile {
        let rule = |pattern: &str| pwrroute_engine::Rule {
            patterns: vec![pattern.into()],
            browser: "Arc".into(),
            ..pwrroute_engine::Rule::default()
        };
        RulesFile {
            default_browser: "Vivaldi".into(),
            default_profile: "Work".into(),
            precedence,
            rules: vec![
                rule("https://both.example.com/*"),
                rule("https://other.example.com/*"),
            ],
            ..RulesFile::default()
        }
    }

    fn home(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("pwrroute-try-it-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        dir
    }

    #[test]
    fn a_config_handler_first_is_not_a_rule_to_move() {
        let home = home("go-order");
        let config = config_beside(&home);
        let mut probe = Probe::at_home(
            Some(config),
            Some(&home),
            rules_beside(pwrroute_engine::Precedence::Config),
        );
        // The config's handler takes it before this rule is asked: no
        // place in rules.json is above it, so the advice is the switch.
        let beaten = probe.run("https://both.example.com/x", None, Selection::Handler(0));
        assert_eq!(
            beaten,
            TryResult::Text {
                verdict: Verdict::Beaten,
                message: "Config handler 1 wins first: Firefox. ~/finicky.js's handlers run \
                          before your rules; let PwrRoute's default take everything else to \
                          run yours first."
                    .into(),
                bold: Some("Firefox".into()),
            }
        );
        // Everything else is the config's.
        assert_eq!(
            probe.run(
                "https://unmatched.example.com/",
                None,
                Selection::Handler(1)
            ),
            TryResult::text(
                Verdict::Passes,
                "No match. ~/finicky.js's default opens that link in Safari."
            )
        );
        // A config matcher that throws stops Finicky before the rules run.
        assert_eq!(
            probe.run("https://boom.example.com/", None, Selection::Handler(1)),
            TryResult::text(
                Verdict::Beaten,
                "Config handler 3 threw before this handler ran: handler matcher threw. \
                 ~/finicky.js's default opens that link in Safari."
            )
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn under_pwrroute_the_rules_go_first_and_pwrroutes_default_takes_the_rest() {
        let home = home("pwrroute");
        let config = config_beside(&home);
        let mut probe = Probe::at_home(
            Some(config),
            Some(&home),
            rules_beside(pwrroute_engine::Precedence::PwrRoute),
        );
        assert_eq!(
            probe.run("https://both.example.com/x", None, Selection::Handler(0)),
            TryResult::text(Verdict::Takes, "This handler opens that link in Arc.")
        );
        // An earlier rule still wins first, and moving this one fixes that.
        let mut beaten = probe.with_rules(RulesFile {
            rules: vec![
                rules_beside(pwrroute_engine::Precedence::PwrRoute).rules[0].clone(),
                pwrroute_engine::Rule {
                    patterns: vec!["https://both.example.com/x".into()],
                    browser: "Firefox".into(),
                    ..pwrroute_engine::Rule::default()
                },
            ],
            ..rules_beside(pwrroute_engine::Precedence::PwrRoute)
        });
        assert_eq!(
            beaten.run("https://both.example.com/x", None, Selection::Handler(1)),
            TryResult::Text {
                verdict: Verdict::Beaten,
                message: "Handler 1 wins first: Arc. Move this rule above it to change that."
                    .into(),
                bold: Some("Arc".into()),
            }
        );
        // The config's handlers run after every rule: one that wins only
        // took what this rule did not match.
        assert_eq!(
            probe.run("https://config.example.com/x", None, Selection::Handler(1)),
            TryResult::text(
                Verdict::Passes,
                "No match. Config handler 2 opens that link in Safari."
            )
        );
        assert_eq!(
            probe.run(
                "https://unmatched.example.com/",
                None,
                Selection::Handler(1)
            ),
            TryResult::text(
                Verdict::Passes,
                "No match. PwrRoute's default opens that link in Vivaldi \u{00b7} Work."
            )
        );
        // The rules ran before the config's matcher threw.
        assert_eq!(
            probe.run("https://boom.example.com/", None, Selection::Handler(1)),
            TryResult::text(
                Verdict::Passes,
                "No match. PwrRoute's default opens that link in Vivaldi \u{00b7} Work."
            )
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_config_handler_runs_before_a_rule_only_in_the_go_order() {
        let step = |source, index| Step {
            stage: Stage::Handler,
            source,
            index,
            matchers: Vec::new(),
            url: "https://a.example.com/".into(),
            matched: true,
            pattern: None,
            rewritten_to: None,
            won: true,
            error: None,
        };
        assert!(step_before(&step(Source::Config, 5), 0, false));
        assert!(!step_before(&step(Source::Config, 0), 3, true));
        assert!(step_before(&step(Source::Rules, 1), 2, true));
        assert!(!step_before(&step(Source::Rules, 2), 2, false));
    }
}
