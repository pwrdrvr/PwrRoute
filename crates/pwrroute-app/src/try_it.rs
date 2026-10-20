//! The editor's Try it: what the selected rule makes of a sample link,
//! from the selected rule's point of view.
//!
//! The rail runs its own [`Engine`] over the rules as edited, drafts and
//! all, so a new handler can say "this handler would take it" before it is
//! complete enough to be saved. Test a link resolves the saved file instead.

use std::{path::PathBuf, rc::Rc};

use pwrroute_engine::{
    DefaultHost, Engine, Limits, Link, Opener, RulesFile, Source, Stage, Step, Trace,
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
        if self.engine.is_none() {
            let sources = pwrroute_engine::Sources {
                config: self.config.clone(),
                rules: self.rules.clone(),
            };
            match Engine::from_sources(&sources, Rc::new(DefaultHost), Limits::default()) {
                Ok(engine) => self.engine = Some(engine),
                Err(error) => {
                    return TryResult::text(
                        Verdict::Broken,
                        format!("The rules did not load: {error:#}"),
                    );
                }
            }
        }
        let Some(engine) = &self.engine else {
            return TryResult::text(Verdict::Broken, "The rules did not load.");
        };
        let opener = opener.map(|app| Opener {
            name: app.to_owned(),
            bundle_id: app.to_owned(),
            ..Opener::default()
        });
        match engine.trace(&Link::new(url), opener.as_ref()) {
            Ok(trace) => interpret(&trace, &self.rules, selection),
            Err(error) => TryResult::text(Verdict::Broken, format!("The rules failed: {error:#}")),
        }
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
