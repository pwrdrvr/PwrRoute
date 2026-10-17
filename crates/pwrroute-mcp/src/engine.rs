//! [`Resolver`] over today's [`pwrroute_engine::Engine`].
//!
//! An `Engine` is not `Send` (QuickJS), so it lives on a thread of its own
//! and the resolver hands it URLs over a channel, one at a time, as the
//! router will.

use std::{path::PathBuf, rc::Rc, sync::mpsc, thread};

use anyhow::{Context as _, Result, anyhow};
use pwrroute_engine::{DefaultHost, Engine, Limits, Opener, Resolution};

use crate::backend::{Resolver, StepKind, Trace, TraceStep};

type Job = (String, Option<Opener>, mpsc::Sender<Result<Resolution>>);

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
                    let _ = reply.send(engine.resolve(&url, opener.as_ref()));
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
        let resolution = answer
            .recv()
            .map_err(|_| anyhow!("the engine thread has stopped"))??;
        Ok(trace_of(url, resolution))
    }
}

/// The engine answers with the destination only. Until the router records
/// each rule that fired, the trace shows the rewrite, if the URL changed,
/// and the destination.
fn trace_of(url: &str, resolution: Resolution) -> Trace {
    let mut steps = Vec::new();
    if resolution.browser.url != url {
        steps.push(TraceStep {
            kind: StepKind::Rewrite,
            index: None,
            summary: "rewritten by the config".to_owned(),
            url: Some(resolution.browser.url.clone()),
        });
    }
    Trace {
        url: url.to_owned(),
        steps,
        browser: resolution.browser,
        error: resolution.error,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn resolves_through_a_loaded_config() {
        let dir = std::env::temp_dir().join(format!("pwrroute-mcp-engine-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let config = dir.join("config.js");
        fs::write(
            &config,
            r#"export default {
                defaultBrowser: "Safari",
                rewrite: [{ match: "x.com/*", url: (u) => { u.host = "xcancel.com"; return u; } }],
                handlers: [{ match: "*.example.org/*", browser: "Firefox" }],
            };"#,
        )
        .unwrap();
        let resolver = EngineResolver::spawn(config, Limits::default()).unwrap();

        let trace = resolver.resolve("https://x.com/a", None).unwrap();
        assert_eq!(trace.browser.name, "Safari");
        assert_eq!(trace.browser.url, "https://xcancel.com/a");
        assert_eq!(trace.steps.len(), 1);

        let trace = resolver
            .resolve("https://docs.example.org/b", None)
            .unwrap();
        assert_eq!(trace.browser.name, "Firefox");
        assert!(trace.steps.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }
}
