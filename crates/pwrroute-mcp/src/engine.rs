//! [`Resolver`] over today's [`pwrroute_engine::Engine`].
//!
//! An `Engine` is not `Send` (QuickJS), so it lives on a thread of its own
//! and the resolver hands it URLs over a channel, one at a time, as the
//! router will. The trace is the engine's own ([`Engine::trace`]), so it
//! names each rule that fired and cannot disagree with the destination.

use std::{path::PathBuf, rc::Rc, sync::mpsc, thread};

use anyhow::{Context as _, Result, anyhow};
use pwrroute_engine::{
    self as engine, DefaultHost, Engine, Limits, Link, Matcher, Opener, Source, Stage,
};

use crate::backend::{Resolver, StepKind, Trace, TraceStep};

type Job = (String, Option<Opener>, mpsc::Sender<Result<engine::Trace>>);

/// Resolves against a Finicky config loaded into an [`Engine`] on its own
/// thread. Config `console` output is dropped ([`DefaultHost`]): it often
/// carries URLs.
#[derive(Debug)]
pub struct EngineResolver {
    jobs: mpsc::Sender<Job>,
}

impl EngineResolver {
    /// Loads `config` on a new thread and returns once it has loaded, or
    /// with the load error.
    pub fn spawn(config: PathBuf, limits: Limits) -> Result<Self> {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (ready, loaded) = mpsc::channel::<Result<()>>();
        thread::Builder::new()
            .name("pwrroute-mcp-engine".into())
            .spawn(move || {
                let engine = match Engine::load(&config, Rc::new(DefaultHost), limits) {
                    Ok(engine) => {
                        let _ = ready.send(Ok(()));
                        engine
                    }
                    Err(e) => {
                        let _ = ready.send(Err(e));
                        return;
                    }
                };
                for (url, opener, reply) in inbox {
                    let _ = reply.send(engine.trace(&Link::new(url), opener.as_ref()));
                }
            })
            .context("starting the engine thread")?;
        loaded
            .recv()
            .map_err(|_| anyhow!("the engine thread exited while loading"))??;
        Ok(Self { jobs })
    }
}

impl Resolver for EngineResolver {
    fn resolve(&self, url: &str, opener: Option<&Opener>) -> Result<Trace> {
        let (reply, answer) = mpsc::channel();
        self.jobs
            .send((url.to_owned(), opener.cloned(), reply))
            .map_err(|_| anyhow!("the engine thread has stopped"))?;
        let trace = answer
            .recv()
            .map_err(|_| anyhow!("the engine thread has stopped"))??;
        Ok(trace_of(trace))
    }
}

/// The engine's trace as the tools show it: the rewrites that matched, in
/// order, then the handler or default that chose the browser. Rules that
/// were evaluated but did not match are left out.
fn trace_of(trace: engine::Trace) -> Trace {
    let error = trace
        .resolution
        .error
        .or_else(|| trace.steps.iter().find_map(|s| s.error.clone()));
    let steps = trace
        .steps
        .iter()
        .filter(|s| (s.stage == Stage::Rewrite && s.matched) || s.won)
        .map(|s| TraceStep {
            kind: match s.stage {
                Stage::Rewrite => StepKind::Rewrite,
                Stage::Handler => StepKind::Handler,
                Stage::Default => StepKind::Default,
            },
            index: (s.stage != Stage::Default).then_some(s.index),
            summary: summary(s),
            url: s.rewritten_to.clone(),
        })
        .collect();
    Trace {
        url: trace.url,
        steps,
        browser: trace.resolution.browser,
        error,
    }
}

/// How the Rules view words a step: which list, which entry, and the
/// pattern that matched.
fn summary(step: &engine::Step) -> String {
    let list = match (step.source, step.stage) {
        (_, Stage::Default) => return "no handler matched: the default browser".to_owned(),
        (Source::Config, Stage::Rewrite) => "config rewrite",
        (Source::Config, Stage::Handler) => "config handler",
        (Source::Rules, Stage::Rewrite) => "rules.json rewrite",
        (Source::Rules, Stage::Handler) => "rules.json rule",
    };
    let matcher = step
        .matchers
        .get(step.pattern.unwrap_or(0))
        .map(|m| match m {
            Matcher::Wildcard(p) | Matcher::Regex(p) | Matcher::Other(p) => p.clone(),
            Matcher::Function(name) if name.is_empty() => "a function".to_owned(),
            Matcher::Function(name) => format!("function {name}"),
        });
    match matcher {
        Some(m) => format!("{list} {} matched {m}", step.index + 1),
        None => format!("{list} {} matched", step.index + 1),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn traces_each_rule_that_fired_through_a_loaded_config() {
        let dir = std::env::temp_dir().join(format!("pwrroute-mcp-engine-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let config = dir.join("config.js");
        fs::write(
            &config,
            r#"export default {
                defaultBrowser: "Safari",
                rewrite: [
                    { match: "nothing.invalid/*", url: "https://elsewhere.invalid/" },
                    { match: "x.com/*", url: (u) => { u.host = "xcancel.com"; return u; } },
                ],
                handlers: [
                    { match: "*.example.org/*", browser: "Firefox" },
                    { match: function thrower() { throw new Error("boom"); }, browser: "Chrome" },
                ],
            };"#,
        )
        .unwrap();
        let resolver = EngineResolver::spawn(config, Limits::default()).unwrap();

        let trace = resolver.resolve("https://x.com/a", None).unwrap();
        assert_eq!(trace.url, "https://x.com/a");
        assert_eq!(trace.browser.name, "Safari");
        assert_eq!(trace.browser.url, "https://xcancel.com/a");
        assert_eq!(
            trace
                .steps
                .iter()
                .map(|s| (s.kind, s.index))
                .collect::<Vec<_>>(),
            [(StepKind::Rewrite, Some(1)), (StepKind::Default, None)],
            "{trace:?}"
        );
        assert_eq!(trace.steps[0].summary, "config rewrite 2 matched x.com/*");
        assert_eq!(trace.steps[0].url.as_deref(), Some("https://xcancel.com/a"));
        assert!(trace.error.is_some(), "the throwing matcher is reported");

        let trace = resolver
            .resolve("https://docs.example.org/b", None)
            .unwrap();
        assert_eq!(trace.browser.name, "Firefox");
        assert_eq!(trace.steps.len(), 1, "{trace:?}");
        assert_eq!(trace.steps[0].kind, StepKind::Handler);
        assert_eq!(trace.steps[0].index, Some(0));
        assert_eq!(
            trace.steps[0].summary,
            "config handler 1 matched *.example.org/*"
        );
        assert!(trace.error.is_none());
        fs::remove_dir_all(dir).unwrap();
    }
}
