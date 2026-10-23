//! What Test a link draws: a resolution as numbered steps and a result.
//!
//! [`TraceView`] is the screen's model. [`TraceView::from_engine`] fills it
//! from [`pwrroute_engine::Trace`]; anything else that can say which rules
//! ran (the router's dispatch records, fixtures in tests) can fill it the
//! same way, and the view does not change.

use std::time::Duration;

use pwrroute_engine::{AppType, Matcher, RewriteAction, RulesFile, Source, Stage, Step, Trace};

use crate::test_link::OpenerChoice;
use crate::widgets::{NOT_OPENED, NOT_OPENED_BADGE, badge_for};

/// A resolution, step by step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceView {
    pub steps: Vec<TraceStep>,
    pub result: TraceResult,
}

/// `.ts`: one row of the trace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceStep {
    /// The number in the circle; `None` draws the check of the last step.
    pub number: Option<usize>,
    pub state: StepState,
    pub title: String,
    /// The mono line under the title.
    pub detail: String,
    pub body: Vec<StepBody>,
}

/// How a step reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    /// The link as received.
    Plain,
    /// `.ts.hit`: the rule matched.
    Hit,
    /// `.ts.miss`: the rule did not match, dimmed.
    Miss,
    /// `.ts.end`: where the link opens.
    End,
    /// The rule threw; Finicky falls back to the default browser.
    Error,
}

/// What a step's right-hand column shows, top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepBody {
    /// A row of chips, optionally followed by a note.
    Chips(Vec<Chip>, Option<String>),
    /// A URL with the parts this step changed marked.
    Url(Vec<UrlPart>),
    /// Plain text: "No match."
    Text(String),
    /// Mono lines, one under the other: what the config printed.
    Lines(Vec<String>),
    /// "No match." for a run of handlers folded into one step, with a link
    /// that unfolds them.
    Folded,
}

/// A chip in a step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chip {
    /// `.tag.act`: a rewrite action.
    Action(String, String),
    /// `.tag.app`: an app or browser with its badge.
    App(String, String),
    /// `.tag`
    Plain(String),
}

/// A run of a URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrlPart {
    pub text: String,
    pub mark: UrlMark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UrlMark {
    Same,
    /// `.chg`: added by this step.
    Changed,
    /// `.del`: removed by this step.
    Deleted,
}

/// `.res`: where the link opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceResult {
    pub badge: String,
    pub title: String,
    pub url: String,
    /// A matcher or rewrite threw; the result is the default browser.
    pub error: Option<String>,
}

impl TraceView {
    /// The view of an engine trace. `rules` is the `rules.json` the engine
    /// loaded, for what the trace does not carry (rewrite actions and
    /// openers); `config` is the Finicky config in charge as the user knows
    /// it (`~/.finicky.ts`), `None` when `rules.json` is the whole config;
    /// `opener` is the app the link was tested as coming from. Runs of
    /// handlers that did not match fold into one step unless `unfold` is
    /// set.
    pub fn from_engine(
        trace: &Trace,
        rules: &RulesFile,
        config: Option<&str>,
        opener: Option<&OpenerChoice>,
        took: Duration,
        unfold: bool,
    ) -> Self {
        let mut steps = vec![TraceStep {
            number: Some(0),
            state: StepState::Plain,
            title: "Received".into(),
            detail: match opener {
                Some(app) => format!("from {} \u{00b7} {}", app.name, app.bundle_id),
                None => "from no app".into(),
            },
            body: vec![StepBody::Url(vec![UrlPart {
                text: trace.url.clone(),
                mark: UrlMark::Same,
            }])],
        }];
        // A rule that throws stops the handlers; the default follows.
        let threw = trace.steps.iter().any(|s| s.error.is_some());
        let mut misses: Vec<&Step> = Vec::new();
        for step in &trace.steps {
            if step.stage == Stage::Handler && !step.matched && step.error.is_none() && !unfold {
                // A run folds within one list: the config's handlers and
                // rules.json's are numbered separately.
                if misses.last().is_some_and(|last| last.source != step.source) {
                    flush_misses(&mut steps, &mut misses);
                }
                misses.push(step);
                continue;
            }
            flush_misses(&mut steps, &mut misses);
            steps.push(engine_step(step, rules, config, opener, threw, steps.len()));
        }
        flush_misses(&mut steps, &mut misses);
        if !trace.console.is_empty() {
            // What the config printed while these steps ran. The lines are
            // the config's own words, URL and all; they stay on screen.
            let count = trace.console.len();
            steps.push(TraceStep {
                number: Some(steps.len()),
                state: StepState::Plain,
                title: "Config output".into(),
                detail: if count == 1 {
                    "1 console line".into()
                } else {
                    format!("{count} console lines")
                },
                body: vec![StepBody::Lines(
                    trace
                        .console
                        .iter()
                        .map(|line| format!("console.{}: {}", line.level.name(), line.message))
                        .collect(),
                )],
            });
        }

        let browser = &trace.resolution.browser;
        let mut chips = Vec::new();
        let (badge, title) = if browser.app_type == AppType::None {
            (NOT_OPENED_BADGE.to_owned(), NOT_OPENED.to_owned())
        } else {
            let name = browser.name.clone();
            chips.push(Chip::App(badge_for(&name).to_string(), name.clone()));
            if !browser.profile.is_empty() {
                chips.push(Chip::Plain(format!("profile {}", browser.profile)));
            }
            chips.push(Chip::Plain(
                if browser.open_in_background == Some(true) {
                    "background"
                } else {
                    "foreground"
                }
                .into(),
            ));
            let title = if browser.profile.is_empty() {
                name.clone()
            } else {
                format!("{name} \u{00b7} {}", browser.profile)
            };
            (badge_for(&name).to_string(), title)
        };
        steps.push(TraceStep {
            number: None,
            state: StepState::End,
            title: "Opens in".into(),
            detail: format!("resolved in {}", millis(took)),
            body: vec![StepBody::Chips(chips, None)],
        });
        Self {
            steps,
            result: TraceResult {
                badge,
                title,
                url: browser.url.clone(),
                error: trace.resolution.error.clone(),
            },
        }
    }

    /// The trace as plain text, for Copy trace.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for step in &self.steps {
            let number = step
                .number
                .map_or_else(|| "\u{2713}".to_owned(), |n| n.to_string());
            out.push_str(&format!("{number}. {} ({})\n", step.title, step.detail));
            for body in &step.body {
                let line = match body {
                    StepBody::Chips(chips, note) => {
                        let mut parts: Vec<String> = chips
                            .iter()
                            .map(|chip| match chip {
                                Chip::Action(kind, value) => format!("{kind} {value}"),
                                Chip::App(_, name) | Chip::Plain(name) => name.clone(),
                            })
                            .collect();
                        parts.extend(note.clone());
                        parts.join(" \u{00b7} ")
                    }
                    StepBody::Url(parts) => parts
                        .iter()
                        .filter(|p| p.mark != UrlMark::Deleted)
                        .map(|p| p.text.as_str())
                        .collect(),
                    StepBody::Text(text) => text.clone(),
                    StepBody::Lines(lines) => lines.join("\n   "),
                    StepBody::Folded => "No match.".into(),
                };
                if !line.is_empty() {
                    out.push_str(&format!("   {line}\n"));
                }
            }
        }
        out.push_str(&format!("{}\n{}\n", self.result.title, self.result.url));
        out
    }
}

/// "0.6 ms", or microseconds under a tenth of a millisecond.
fn millis(took: Duration) -> String {
    let ms = took.as_secs_f64() * 1000.0;
    if ms < 0.1 {
        format!("{} \u{00b5}s", took.as_micros())
    } else {
        format!("{ms:.1} ms")
    }
}

fn matcher_label(matcher: &Matcher) -> String {
    match matcher {
        Matcher::Wildcard(s) | Matcher::Regex(s) | Matcher::Other(s) => s.clone(),
        Matcher::Function(name) if name.is_empty() => "fn".into(),
        Matcher::Function(name) => format!("fn {name}"),
    }
}

/// What a step's source calls its rule: "Rewrite 1", "Handler 3", or with
/// the config's own lists first, "Config handler 2".
fn rule_name(step: &Step, plural: bool) -> String {
    let kind = match (step.stage, plural) {
        (Stage::Rewrite, false) => "Rewrite",
        (Stage::Rewrite, true) => "Rewrites",
        (Stage::Handler, false) => "Handler",
        (Stage::Handler, true) => "Handlers",
        (Stage::Default, _) => "Default",
    };
    match step.source {
        Source::Rules => kind.to_owned(),
        Source::Config => format!("Config {}", kind.to_lowercase()),
    }
}

fn engine_step(
    step: &Step,
    rules: &RulesFile,
    config: Option<&str>,
    opener: Option<&OpenerChoice>,
    threw: bool,
    number: usize,
) -> TraceStep {
    let patterns = || {
        step.matchers
            .iter()
            .map(matcher_label)
            .collect::<Vec<_>>()
            .join(" \u{00b7} ")
    };
    let matched_pattern = || {
        step.pattern
            .and_then(|i| step.matchers.get(i))
            .or_else(|| (step.matchers.len() == 1).then(|| &step.matchers[0]))
            .map(matcher_label)
    };
    let title = match step.stage {
        Stage::Default => DefaultOwner::of(step, config).title(),
        _ => format!("{} {}", rule_name(step, false), step.index + 1),
    };
    if let Some(error) = &step.error {
        return TraceStep {
            number: Some(number),
            state: StepState::Error,
            title,
            detail: patterns(),
            body: vec![StepBody::Text(error.clone())],
        };
    }
    if !step.matched && step.stage != Stage::Default {
        return TraceStep {
            number: Some(number),
            state: StepState::Miss,
            title,
            detail: patterns(),
            body: vec![StepBody::Text("No match.".into())],
        };
    }
    let rule_opener = match (step.stage, step.source) {
        (Stage::Handler, Source::Rules) => rules
            .rules
            .get(step.index)
            .filter(|rule| !rule.opener.is_empty()),
        _ => None,
    };
    match step.stage {
        Stage::Rewrite => {
            let after = step
                .rewritten_to
                .clone()
                .unwrap_or_else(|| step.url.clone());
            let actions = match step.source {
                Source::Rules => rules
                    .rewrites
                    .get(step.index)
                    .map(|r| r.actions.as_slice())
                    .unwrap_or_default(),
                Source::Config => &[],
            };
            let chips: Vec<Chip> = actions
                .iter()
                .map(|action| action_chip(action, &step.url, &after))
                .collect();
            let mut body = Vec::new();
            if !chips.is_empty() {
                body.push(StepBody::Chips(chips, None));
            }
            if after == step.url {
                body.push(StepBody::Text("Matched; the URL did not change.".into()));
            } else {
                body.push(StepBody::Url(url_diff(&step.url, &after)));
            }
            TraceStep {
                number: Some(number),
                state: StepState::Hit,
                title,
                detail: matched_pattern().map_or_else(patterns, |p| format!("matched {p}")),
                body,
            }
        }
        Stage::Handler => {
            let pattern = matched_pattern();
            let (detail, body) = match (rule_opener, opener) {
                (Some(_), Some(app)) => (
                    format!("sent from {} \u{00b7} first match", app.name),
                    StepBody::Chips(
                        vec![Chip::App(app.badge.to_string(), app.name.to_string())],
                        Some(match &pattern {
                            Some(p) => format!("matched the opener; pattern {p} matched the URL"),
                            None => "matched the opener and the URL".into(),
                        }),
                    ),
                ),
                _ => (
                    match &pattern {
                        Some(p) => format!("matched {p} \u{00b7} first match"),
                        None => "first match".into(),
                    },
                    StepBody::Chips(
                        Vec::new(),
                        Some(match &pattern {
                            Some(p) => format!("pattern {p} matched the URL"),
                            None => "matched the URL".into(),
                        }),
                    ),
                ),
            };
            TraceStep {
                number: Some(number),
                state: StepState::Hit,
                title,
                detail,
                body: vec![body],
            }
        }
        Stage::Default => {
            let owner = DefaultOwner::of(step, config);
            TraceStep {
                number: Some(number),
                state: StepState::Hit,
                title,
                detail: if threw {
                    "a rule threw".into()
                } else if matches!(owner, DefaultOwner::PwrRoute { .. }) {
                    "no rule or config handler matched".into()
                } else {
                    "no handler matched".into()
                },
                body: vec![StepBody::Text(owner.opens_it(threw))],
            }
        }
    }
}

/// Whose default took the link: the trace step's source says, never the
/// browser. Only beside a config is there an owner to name: `rules.json`'s
/// default is PwrRoute's under `"precedence": "pwrroute"`, otherwise the
/// config's. With `rules.json` alone the step is plain "Everything else",
/// as Try it says, since an empty default there means Safari until the
/// user picks one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DefaultOwner<'a> {
    /// No config in charge: the default browser, unnamed.
    Alone,
    /// PwrRoute's default, beside the config in charge (whose own
    /// `defaultBrowser` is then not used).
    PwrRoute { beside: &'a str },
    /// The config's `defaultBrowser`, by the config's shown name.
    Config(&'a str),
}

impl<'a> DefaultOwner<'a> {
    fn of(step: &Step, config: Option<&'a str>) -> Self {
        match (step.source, config) {
            (_, None) => Self::Alone,
            (Source::Rules, Some(config)) => Self::PwrRoute { beside: config },
            (Source::Config, Some(config)) => Self::Config(config),
        }
    }

    /// "Everything else", "Everything else · PwrRoute's default",
    /// "Everything else · ~/.finicky.ts's default".
    fn title(self) -> String {
        match self {
            Self::Alone => "Everything else".into(),
            Self::PwrRoute { .. } => "Everything else \u{00b7} PwrRoute's default".into(),
            Self::Config(config) => format!("Everything else \u{00b7} {config}'s default"),
        }
    }

    /// The step's sentence: who opens the link, and after an error, why.
    fn opens_it(self, threw: bool) -> String {
        let who = match self {
            Self::Alone => "the default browser".to_owned(),
            Self::PwrRoute { .. } => "PwrRoute's default".to_owned(),
            Self::Config(config) => format!("{config}'s defaultBrowser"),
        };
        match self {
            _ if threw => format!("Finicky stops at the error and {who} opens it."),
            Self::Alone => "The default browser opens it.".into(),
            Self::PwrRoute { beside } => {
                format!("{who} opens it; {beside}'s defaultBrowser is not used.")
            }
            Self::Config(_) => format!("{who} opens it."),
        }
    }
}

/// A rewrite action as the step's chip. A query drop names the keys it
/// actually removed, when it removed any.
fn action_chip(action: &RewriteAction, before: &str, after: &str) -> Chip {
    let (kind, value) = match action {
        RewriteAction::Url(url) => ("url", url.clone()),
        RewriteAction::SetScheme(scheme) => ("scheme", scheme.clone()),
        RewriteAction::SetHost(host) => ("host", host.clone()),
        RewriteAction::ReplacePath(path) => ("path", path.clone()),
        RewriteAction::DropQuery(keys) => {
            let kept = query_keys(after);
            let dropped: Vec<String> = query_keys(before)
                .into_iter()
                .filter(|key| !kept.contains(key))
                .collect();
            let value = if dropped.is_empty() {
                keys.join(" ")
            } else {
                dropped.join(" ")
            };
            ("drop", value)
        }
    };
    Chip::Action(kind.into(), value)
}

fn query_keys(url: &str) -> Vec<String> {
    let Some((_, query)) = url.split_once('?') else {
        return Vec::new();
    };
    let query = query.split('#').next().unwrap_or_default();
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| pair.split('=').next().unwrap_or_default().to_owned())
        .collect()
}

/// Folds a run of missed handlers into one step: "Handlers 1 and 2".
fn flush_misses(steps: &mut Vec<TraceStep>, misses: &mut Vec<&Step>) {
    let Some(first) = misses.first() else {
        return;
    };
    let number = steps.len();
    if misses.len() == 1 {
        let step = misses[0];
        steps.push(TraceStep {
            number: Some(number),
            state: StepState::Miss,
            title: format!("{} {}", rule_name(step, false), step.index + 1),
            detail: step
                .matchers
                .iter()
                .map(matcher_label)
                .collect::<Vec<_>>()
                .join(" \u{00b7} "),
            body: vec![StepBody::Text("No match.".into())],
        });
        misses.clear();
        return;
    }
    let last = misses[misses.len() - 1];
    let (a, b) = (first.index + 1, last.index + 1);
    let range = if misses.len() == 2 {
        format!("{a} and {b}")
    } else {
        format!("{a}\u{2013}{b}")
    };
    let firsts: Vec<String> = misses
        .iter()
        .filter_map(|s| s.matchers.first().map(matcher_label))
        .collect();
    let more = misses.iter().any(|s| s.matchers.len() > 1);
    let mut detail = firsts.join(" \u{00b7} ");
    if more {
        detail.push_str(" \u{2026}");
    }
    steps.push(TraceStep {
        number: Some(number),
        state: StepState::Miss,
        title: format!("{} {range}", rule_name(first, true)),
        detail,
        body: vec![StepBody::Folded],
    });
    misses.clear();
}

/// Splits a URL into words and the delimiters between them, so a change
/// marks a whole host or query pair (`utm_source=slack`) rather than a few
/// letters.
fn url_tokens(url: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start = 0;
    for (i, ch) in url.char_indices() {
        if matches!(ch, '/' | ':' | '?' | '&' | '#') {
            if start < i {
                tokens.push(&url[start..i]);
            }
            tokens.push(&url[i..i + ch.len_utf8()]);
            start = i + ch.len_utf8();
        }
    }
    if start < url.len() {
        tokens.push(&url[start..]);
    }
    tokens
}

/// The URL after a step, with what it added marked; or, when the step only
/// removed something, the URL before with that marked.
pub fn url_diff(before: &str, after: &str) -> Vec<UrlPart> {
    let a = url_tokens(before);
    let b = url_tokens(after);
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let part = |tokens: &[&str], mark| UrlPart {
        text: tokens.concat(),
        mark,
    };
    let (source, mark) = if b.len() > prefix + suffix {
        (&b, UrlMark::Changed)
    } else {
        (&a, UrlMark::Deleted)
    };
    let middle_end = source.len() - suffix;
    [
        part(&source[..prefix], UrlMark::Same),
        part(&source[prefix..middle_end], mark),
        part(&source[middle_end..], UrlMark::Same),
    ]
    .into_iter()
    .filter(|p| !p.text.is_empty())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(diff: &[UrlPart]) -> Vec<(&str, UrlMark)> {
        diff.iter().map(|p| (p.text.as_str(), p.mark)).collect()
    }

    #[test]
    fn a_new_host_is_marked_whole() {
        let diff = url_diff(
            "https://x.com/pwrdrvr/status/1842?utm_source=slack&s=20",
            "https://xcancel.com/pwrdrvr/status/1842?utm_source=slack&s=20",
        );
        assert_eq!(
            parts(&diff),
            [
                ("https://", UrlMark::Same),
                ("xcancel.com", UrlMark::Changed),
                ("/pwrdrvr/status/1842?utm_source=slack&s=20", UrlMark::Same),
            ]
        );
    }

    #[test]
    fn a_dropped_query_pair_is_struck_from_the_old_url() {
        let diff = url_diff(
            "https://xcancel.com/pwrdrvr/status/1842?utm_source=slack&s=20",
            "https://xcancel.com/pwrdrvr/status/1842?s=20",
        );
        assert_eq!(
            parts(&diff),
            [
                ("https://xcancel.com/pwrdrvr/status/1842?", UrlMark::Same),
                ("utm_source=slack&", UrlMark::Deleted),
                ("s=20", UrlMark::Same),
            ]
        );
    }

    fn miss(source: Source, index: usize) -> Step {
        Step {
            stage: Stage::Handler,
            source,
            index,
            matchers: vec![Matcher::Wildcard(format!("h{index}.example/*"))],
            url: "https://a.example/".into(),
            matched: false,
            pattern: None,
            rewritten_to: None,
            won: false,
            error: None,
        }
    }

    #[test]
    fn the_default_after_a_thrown_rule_says_so() {
        let trace: Trace = serde_json::from_value(serde_json::json!({
            "url": "https://a.example/",
            "originalUrl": "https://a.example/",
            "steps": [],
            "resolution": {"browser": {
                "name": "Safari", "appType": "appName", "url": "https://a.example/"
            }},
        }))
        .expect("a trace");
        let thrown = Step {
            error: Some("TypeError: boom".into()),
            ..miss(Source::Config, 0)
        };
        let default = Step {
            stage: Stage::Default,
            matchers: Vec::new(),
            won: true,
            ..miss(Source::Config, 0)
        };
        let trace = Trace {
            steps: vec![thrown, default],
            ..trace
        };
        let view = TraceView::from_engine(
            &trace,
            &RulesFile::default(),
            None,
            None,
            Duration::ZERO,
            false,
        );
        let fallback = &view.steps[2];
        assert_eq!(fallback.title, "Everything else");
        assert_eq!(fallback.detail, "a rule threw");
    }

    #[test]
    fn missed_handlers_fold_within_their_own_list() {
        let trace: Trace = serde_json::from_value(serde_json::json!({
            "url": "https://a.example/",
            "originalUrl": "https://a.example/",
            "steps": [],
            "resolution": {"browser": {
                "name": "Safari", "appType": "appName", "url": "https://a.example/"
            }},
        }))
        .expect("a trace");
        let trace = Trace {
            steps: vec![
                miss(Source::Config, 0),
                miss(Source::Config, 1),
                miss(Source::Rules, 0),
                miss(Source::Rules, 1),
            ],
            ..trace
        };
        let view = TraceView::from_engine(
            &trace,
            &RulesFile::default(),
            None,
            None,
            Duration::ZERO,
            false,
        );
        let titles: Vec<&str> = view.steps.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Received",
                "Config handlers 1 and 2",
                "Handlers 1 and 2",
                "Opens in"
            ]
        );
    }

    #[test]
    fn the_configs_console_output_is_a_row_before_the_result() {
        let trace: Trace = serde_json::from_value(serde_json::json!({
            "url": "https://a.example/",
            "originalUrl": "https://a.example/",
            "steps": [],
            "resolution": {"browser": {
                "name": "Safari", "appType": "appName", "url": "https://a.example/"
            }},
            "console": [
                {"level": "info", "message": "keys {\"option\":true}"},
                {"level": "warn", "message": "saw https://a.example/"},
            ],
        }))
        .expect("a trace");
        let view = TraceView::from_engine(
            &trace,
            &RulesFile::default(),
            None,
            None,
            Duration::ZERO,
            false,
        );
        let titles: Vec<&str> = view.steps.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["Received", "Config output", "Opens in"]);
        let output = &view.steps[1];
        assert_eq!(output.number, Some(1));
        assert_eq!(output.state, StepState::Plain);
        assert_eq!(output.detail, "2 console lines");
        assert_eq!(
            output.body,
            [StepBody::Lines(vec![
                "console.log: keys {\"option\":true}".into(),
                "console.warn: saw https://a.example/".into(),
            ])]
        );
        let text = view.to_text();
        assert!(
            text.contains(
                "1. Config output (2 console lines)\n   console.log: keys {\"option\":true}\n   console.warn: saw https://a.example/\n"
            ),
            "{text}"
        );

        // A quiet run has no such row.
        let quiet = Trace {
            console: Vec::new(),
            ..trace
        };
        let view = TraceView::from_engine(
            &quiet,
            &RulesFile::default(),
            None,
            None,
            Duration::ZERO,
            false,
        );
        assert_eq!(view.steps.len(), 2);
    }

    #[test]
    fn everything_else_names_whose_default_took_the_link() {
        let trace: Trace = serde_json::from_value(serde_json::json!({
            "url": "https://a.example.com/",
            "originalUrl": "https://a.example.com/",
            "steps": [],
            "resolution": {"browser": {
                "name": "Vivaldi", "appType": "appName", "url": "https://a.example.com/"
            }},
        }))
        .expect("a trace");
        let default = |source| Step {
            stage: Stage::Default,
            matchers: Vec::new(),
            matched: true,
            won: true,
            ..miss(source, 0)
        };
        let last = |steps: Vec<Step>, config: Option<&str>| {
            let trace = Trace {
                steps,
                ..trace.clone()
            };
            let view = TraceView::from_engine(
                &trace,
                &RulesFile::default(),
                config,
                None,
                Duration::ZERO,
                false,
            );
            let step = view.steps[view.steps.len() - 2].clone();
            (step.title, step.detail, step.body)
        };
        let text = |s: &str| vec![StepBody::Text(s.into())];
        let config = Some("~/.finicky.ts");
        // `"precedence": "pwrroute"` beside a config.
        assert_eq!(
            last(vec![default(Source::Rules)], config),
            (
                "Everything else \u{00b7} PwrRoute's default".into(),
                "no rule or config handler matched".into(),
                text("PwrRoute's default opens it; ~/.finicky.ts's defaultBrowser is not used.")
            )
        );
        // The Go order: the config's own default.
        assert_eq!(
            last(vec![default(Source::Config)], config),
            (
                "Everything else \u{00b7} ~/.finicky.ts's default".into(),
                "no handler matched".into(),
                text("~/.finicky.ts's defaultBrowser opens it.")
            )
        );
        // rules.json alone: no owner to name, as Try it says.
        assert_eq!(
            last(vec![default(Source::Rules)], None),
            (
                "Everything else".into(),
                "no handler matched".into(),
                text("The default browser opens it.")
            )
        );
        // A config matcher threw: Finicky falls to whoever owns the default.
        let thrown = Step {
            error: Some("handler matcher threw".into()),
            ..miss(Source::Config, 0)
        };
        assert_eq!(
            last(vec![thrown, default(Source::Rules)], config),
            (
                "Everything else \u{00b7} PwrRoute's default".into(),
                "a rule threw".into(),
                text("Finicky stops at the error and PwrRoute's default opens it.")
            )
        );
    }

    #[test]
    fn a_new_scheme_is_marked() {
        let diff = url_diff("http://wiki.internal/start", "https://wiki.internal/start");
        assert_eq!(
            parts(&diff),
            [
                ("https", UrlMark::Changed),
                ("://wiki.internal/start", UrlMark::Same),
            ]
        );
    }
}
