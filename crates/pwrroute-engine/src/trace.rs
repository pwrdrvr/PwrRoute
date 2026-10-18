//! What happened while resolving a link: each rewrite and handler that was
//! evaluated, which ones matched, and which one won. The design's Test a link
//! and Activity screens, and the MCP resolve tool, show this.
//!
//! The trace comes from Finicky's own `openUrl`, run over a copy of the
//! config whose matchers and rewrites report to the host before deferring to
//! the API's exported `isMatch` and `rewriteUrl` (see `js/host.js`). The
//! bundle is not changed, and the result in a trace is the one `openUrl`
//! returned, so it cannot disagree with [`Engine::resolve`](crate::Engine::resolve).

use serde::{Deserialize, Serialize};

use crate::Resolution;

/// One resolution, step by step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trace {
    /// The URL the rules started from (after short-link expansion).
    pub url: String,
    /// The URL as clicked. Differs from `url` when a short link was expanded.
    pub original_url: String,
    /// Steps in evaluation order. Every rewrite is evaluated; handlers are
    /// evaluated until one matches, so handlers after the winner are absent.
    pub steps: Vec<Step>,
    pub resolution: Resolution,
}

impl Trace {
    /// The step whose browser was used: a handler, or the default.
    pub fn winner(&self) -> Option<&Step> {
        self.steps.iter().find(|s| s.won)
    }

    /// The rewrites that matched, in order.
    pub fn rewrites_applied(&self) -> impl Iterator<Item = &Step> {
        self.steps
            .iter()
            .filter(|s| s.stage == Stage::Rewrite && s.matched)
    }
}

/// One rewrite or handler being evaluated, or the default browser used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub stage: Stage,
    /// The JS config or `rules.json`.
    pub source: Source,
    /// The position in its source's `rewrite` or `handlers` list (or the
    /// rules file's `rewrites` or `rules`). Always 0 for the default.
    pub index: usize,
    /// The step's `match`, described. Empty for the default.
    pub matchers: Vec<Matcher>,
    /// The URL this step saw.
    pub url: String,
    pub matched: bool,
    /// For a list of matchers, which one matched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<usize>,
    /// For a rewrite that matched, the URL it produced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewritten_to: Option<String>,
    /// This step's browser opened the link.
    #[serde(default)]
    pub won: bool,
    /// The matcher or rewrite threw. Finicky then stops and uses the default
    /// browser, which follows as its own step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Rewrite,
    Handler,
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// The JS or TS config.
    Config,
    /// `rules.json`.
    Rules,
}

/// A `match` entry as the UI can show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Matcher {
    /// A Finicky wildcard pattern.
    Wildcard(String),
    /// A regular expression, as `/source/flags`.
    Regex(String),
    /// A JS function, by name when it has one.
    Function(String),
    /// Something Finicky would not match with.
    Other(String),
}

/// The shape `__pwrroute.trace` returns.
#[derive(Deserialize)]
pub(crate) struct Raw {
    pub steps: Vec<Step>,
    pub result: Resolution,
}
