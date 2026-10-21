//! The engine on a thread of its own: [`Resolver`] and [`RulesTester`]
//! over [`pwrroute_engine::Engine`].
//!
//! An `Engine` is not `Send` (QuickJS), so it lives on a thread and the
//! adapters hand it work over a channel, one job at a time, as the router
//! does. The thread is built around a [`Loader`]: [`EngineResolver`] loads
//! one config file; [`EngineTester`] keeps a [`Reloader`] over the config
//! choice and `rules.json` the router routes by, caught up with the files
//! before each job; the fakes build one from in-memory rules. The trace is
//! the engine's own ([`Engine::trace`]), so it names each rule that fired
//! and cannot disagree with the destination.

use std::{
    path::PathBuf,
    rc::Rc,
    sync::{Arc, mpsc},
    thread,
};

use anyhow::{Context as _, Result, anyhow};
use pwrroute_engine::{
    self as engine, ConfigChoice, Engine, HttpNetwork, Limits, Link, Matcher, NativeHost, Network,
    Opener, Reloader, RulesFile, Source, Sources, Stage, TestReport,
};

use crate::backend::{Resolver, RulesTester, StepKind, Trace, TraceStep};

/// What the thread owns and keeps current. Built on the thread, so it may
/// hold an [`Engine`].
pub trait Loader {
    /// The engine as of now: a loader that follows files catches up here.
    fn engine(&mut self) -> Result<&Engine>;
}

/// One engine, loaded once from a file.
struct Fixed(Engine);

impl Loader for Fixed {
    fn engine(&mut self) -> Result<&Engine> {
        Ok(&self.0)
    }
}

/// A [`Reloader`] over the config choice and `rules.json`, reloaded when
/// either changed since the last job. A reload that fails keeps the last
/// engine that worked, as the router does.
struct Following(Reloader);

impl Loader for Following {
    fn engine(&mut self) -> Result<&Engine> {
        if let Some(Err(e)) = self.0.reload_if_changed() {
            // The last engine that worked goes on answering; say so where
            // a developer looks, without the files' contents.
            eprintln!("pwrroute-mcp: the config did not reload: {e:#}");
        }
        Ok(self.0.engine())
    }
}

type Job = Box<dyn FnOnce(&Engine) -> Result<serde_json::Value> + Send>;

/// Hands jobs to an engine on its own thread, one at a time, and returns
/// each answer as JSON (the engine's types all serialize) so the thread
/// stays generic over what is asked of it.
#[derive(Debug, Clone)]
pub struct EngineThread {
    jobs: mpsc::Sender<Job>,
}

impl EngineThread {
    /// Builds the loader on a new thread with `make` and returns once it
    /// has, or with the error.
    pub fn spawn<L: Loader + 'static>(
        name: &str,
        make: impl FnOnce() -> Result<L> + Send + 'static,
    ) -> Result<Self> {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (ready, loaded) = mpsc::channel::<Result<()>>();
        thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                let mut loader = match make() {
                    Ok(loader) => {
                        let _ = ready.send(Ok(()));
                        loader
                    }
                    Err(e) => {
                        let _ = ready.send(Err(e));
                        return;
                    }
                };
                for job in inbox {
                    // The answer is sent inside the job; a job whose asker
                    // has gone just drops its answer.
                    let _ = loader.engine().and_then(job);
                }
            })
            .context("starting the engine thread")?;
        loaded
            .recv()
            .map_err(|_| anyhow!("the engine thread exited while loading"))??;
        Ok(Self { jobs })
    }

    /// Runs `f` on the thread's engine and waits for its answer.
    pub fn run<T: serde::de::DeserializeOwned + Send + 'static>(
        &self,
        f: impl FnOnce(&Engine) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (reply, answer) = mpsc::channel::<Result<T>>();
        let job: Job = Box::new(move |engine| {
            let result = f(engine);
            let _ = reply.send(result);
            Ok(serde_json::Value::Null)
        });
        self.jobs
            .send(job)
            .map_err(|_| anyhow!("the engine thread has stopped"))?;
        answer
            .recv()
            .map_err(|_| anyhow!("the engine thread has stopped"))?
    }
}

/// Resolves against a Finicky config loaded into an [`Engine`] on its own
/// thread, expanding a short link first as the router does, so the trace
/// matches the router's, with the OS answering the config's `finicky.*`
/// calls ([`NativeHost`]). A run's `console` output comes back in the
/// trace; the config's load-time output is dropped, since it often carries
/// URLs and no result carries it.
pub struct EngineResolver {
    thread: EngineThread,
    /// Expands short links, on the engine's thread: the one request a
    /// resolve makes off the machine, to the shortener's host.
    network: Arc<dyn Network + Send + Sync>,
}

impl std::fmt::Debug for EngineResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineResolver")
            .field("thread", &self.thread)
            .finish_non_exhaustive()
    }
}

impl EngineResolver {
    /// Loads `config` on a new thread and returns once it has loaded, or
    /// with the load error. Short links expand over the real network.
    pub fn spawn(config: PathBuf, limits: Limits) -> Result<Self> {
        Self::spawn_with(config, limits, Arc::new(HttpNetwork::new()))
    }

    /// [`spawn`](Self::spawn) with the network that expands short links;
    /// tests pass a fake.
    pub fn spawn_with(
        config: PathBuf,
        limits: Limits,
        network: Arc<dyn Network + Send + Sync>,
    ) -> Result<Self> {
        let thread = EngineThread::spawn("pwrroute-mcp-engine", move || {
            Ok(Fixed(Engine::load(
                &config,
                Rc::new(NativeHost::new()),
                limits,
            )?))
        })?;
        Ok(Self { thread, network })
    }
}

impl Resolver for EngineResolver {
    fn resolve(&self, url: &str, opener: Option<&Opener>) -> Result<Trace> {
        let url = url.to_owned();
        let opener = opener.cloned();
        let network = Arc::clone(&self.network);
        let trace = self.thread.run(move |engine| {
            let link = engine.short_links().expand(&url, network.as_ref());
            engine.trace(&link, opener.as_ref())
        })?;
        Ok(trace_of(trace))
    }
}

/// Runs `rules.json`'s examples and tests, and candidate patterns, through
/// the same config and rules the router routes by, on a thread of its own
/// that follows the files.
#[derive(Debug, Clone)]
pub struct EngineTester {
    thread: EngineThread,
}

impl EngineTester {
    /// Loads `choice` and `rules_path` on a new thread, as the router's
    /// own resolver does. A config that fails to load leaves a fallback
    /// engine in charge, as it does there.
    pub fn spawn(
        choice: ConfigChoice,
        rules_path: Option<PathBuf>,
        limits: Limits,
    ) -> Result<Self> {
        let thread = EngineThread::spawn("pwrroute-mcp-rules", move || {
            Ok(Following(Reloader::new(
                choice,
                rules_path,
                Rc::new(NativeHost::new()),
                limits,
            )?))
        })?;
        Ok(Self { thread })
    }

    /// A tester over rules held in memory (no config), rebuilt whenever
    /// `rules` answers with a new revision: the fakes' engine.
    pub fn over(
        rules: impl Fn() -> (String, RulesFile) + Send + 'static,
        limits: Limits,
    ) -> Result<Self> {
        let thread = EngineThread::spawn("pwrroute-mcp-rules", move || {
            let (revision, file) = rules();
            let engine = build(&file, limits)?;
            Ok(InMemory {
                rules: Box::new(rules),
                limits,
                revision,
                engine,
            })
        })?;
        Ok(Self { thread })
    }
}

fn build(rules: &RulesFile, limits: Limits) -> Result<Engine> {
    Engine::from_sources(
        &Sources {
            config: None,
            rules: rules.clone(),
        },
        Rc::new(NativeHost::new()),
        limits,
    )
}

/// An engine over in-memory rules, rebuilt when their revision changes.
struct InMemory {
    rules: Box<dyn Fn() -> (String, RulesFile) + Send>,
    limits: Limits,
    revision: String,
    engine: Engine,
}

impl Loader for InMemory {
    fn engine(&mut self) -> Result<&Engine> {
        let (revision, file) = (self.rules)();
        if revision != self.revision {
            self.engine = build(&file, self.limits)?;
            self.revision = revision;
        }
        Ok(&self.engine)
    }
}

impl RulesTester for EngineTester {
    fn test(&self, proposal: Option<&RulesFile>) -> Result<TestReport> {
        let proposal = proposal.cloned();
        self.thread
            .run(move |engine| engine.test_rules(proposal.as_ref()))
    }

    fn collisions(&self, pattern: &str, urls: &[String]) -> Result<Vec<String>> {
        let pattern = pattern.to_owned();
        let urls = urls.to_vec();
        self.thread.run(move |engine| {
            let refs: Vec<&str> = urls.iter().map(String::as_str).collect();
            engine.collisions(&pattern, &refs)
        })
    }
}

/// The engine's trace as the tools show it: the rewrites that matched, in
/// order, then the handler or default that chose the browser. Rules that
/// were evaluated but did not match are left out.
pub(crate) fn trace_of(trace: engine::Trace) -> Trace {
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
    let original_url = (trace.original_url != trace.url).then_some(trace.original_url);
    Trace {
        url: trace.url,
        original_url,
        steps,
        browser: trace.resolution.browser,
        error,
        console: trace.console.iter().map(Into::into).collect(),
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
    use std::{fs, sync::Mutex, time::Duration};

    use pwrroute_engine::{Expectation, Method, Reply, RouteTest, Rule};

    use super::*;

    /// A shortener that sends one link to GitHub and counts what it was
    /// asked; anything else is offline.
    #[derive(Default)]
    struct FakeShortener {
        asked: Mutex<Vec<String>>,
    }

    impl Network for FakeShortener {
        fn request(&self, method: Method, url: &str, _timeout: Duration) -> Result<Reply> {
            self.asked.lock().unwrap().push(format!("{method:?} {url}"));
            match url {
                "https://bit.ly/gh" => Ok(Reply {
                    status: 301,
                    location: Some("https://github.com/pwrdrvr/PwrRoute".into()),
                }),
                "https://github.com/pwrdrvr/PwrRoute" => Ok(Reply {
                    status: 200,
                    location: None,
                }),
                _ => Err(anyhow!("offline")),
            }
        }
    }

    #[test]
    fn a_short_link_is_expanded_before_the_trace_and_both_urls_are_reported() {
        let dir =
            std::env::temp_dir().join(format!("pwrroute-mcp-engine-short-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let config = dir.join("config.js");
        fs::write(
            &config,
            r#"export default {
                defaultBrowser: "Safari",
                handlers: [{ match: "github.com/*", browser: "Firefox" }],
            };"#,
        )
        .unwrap();
        let net = Arc::new(FakeShortener::default());
        let resolver = EngineResolver::spawn_with(
            config,
            Limits::default(),
            Arc::clone(&net) as Arc<dyn Network + Send + Sync>,
        )
        .unwrap();

        let trace = resolver.resolve("https://bit.ly/gh", None).unwrap();
        assert_eq!(trace.url, "https://github.com/pwrdrvr/PwrRoute");
        assert_eq!(trace.original_url.as_deref(), Some("https://bit.ly/gh"));
        assert_eq!(trace.browser.name, "Firefox");
        assert_eq!(trace.steps[0].kind, StepKind::Handler);
        assert_eq!(
            *net.asked.lock().unwrap(),
            [
                "Head https://bit.ly/gh",
                "Head https://github.com/pwrdrvr/PwrRoute"
            ]
        );
        let json = serde_json::to_string(&trace).unwrap();
        assert!(
            json.contains(r#""originalUrl":"https://bit.ly/gh""#),
            "{json}"
        );

        // Not short: no request, no originalUrl.
        let trace = resolver.resolve("https://github.com/direct", None).unwrap();
        assert_eq!(trace.original_url, None);
        assert_eq!(net.asked.lock().unwrap().len(), 2);
        let json = serde_json::to_string(&trace).unwrap();
        assert!(!json.contains("originalUrl"), "{json}");
        fs::remove_dir_all(dir).unwrap();
    }

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
                    { match: "x.com/*", url: (u) => { console.log("unshortening", u.href); u.host = "xcancel.com"; return u; } },
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
        assert_eq!(
            trace.console,
            [crate::backend::ConsoleLine {
                level: "log".to_owned(),
                message: "unshortening https://x.com/a".to_owned(),
            }],
            "the run's console output rides along"
        );

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
        assert!(trace.console.is_empty(), "{trace:?}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_tester_reports_what_the_config_printed() {
        let dir = std::env::temp_dir().join(format!(
            "pwrroute-mcp-tester-console-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let config = dir.join("config.js");
        fs::write(
            &config,
            r#"export default {
                defaultBrowser: "Safari",
                handlers: [{ match: (u) => { console.warn("checking", u.href); return false; }, browser: "Chrome" }],
            };"#,
        )
        .unwrap();
        let path = dir.join("rules.json");
        rules().save(&path).unwrap();
        let tester = EngineTester::spawn(
            ConfigChoice::Path(config),
            Some(path.clone()),
            Limits::default(),
        )
        .unwrap();
        let report = tester.test(None).unwrap();
        assert_eq!((report.total, report.passed), (2, 2), "{report:?}");
        let lines: Vec<(&str, &str)> = report
            .console
            .iter()
            .map(|l| (l.level.name(), l.message.as_str()))
            .collect();
        assert_eq!(
            lines,
            [
                ("warn", "checking https://docs.example.org/a"),
                ("warn", "checking https://example.net/"),
            ]
        );
        fs::remove_dir_all(dir).unwrap();
    }

    fn rules() -> RulesFile {
        RulesFile {
            default_browser: "Safari".into(),
            rules: vec![Rule {
                patterns: vec!["*.example.org/*".into()],
                browser: "Firefox".into(),
                examples: vec!["https://docs.example.org/a".into()],
                ..Rule::default()
            }],
            tests: vec![RouteTest {
                url: "https://example.net/".into(),
                opener: None,
                expect: Expectation {
                    browser: "Safari".into(),
                    ..Expectation::default()
                },
            }],
            ..RulesFile::default()
        }
    }

    #[test]
    fn a_tester_follows_the_files_and_tests_a_proposal_without_writing() {
        let dir = std::env::temp_dir().join(format!("pwrroute-mcp-tester-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rules.json");
        rules().save(&path).unwrap();
        let tester =
            EngineTester::spawn(ConfigChoice::None, Some(path.clone()), Limits::default()).unwrap();

        let report = tester.test(None).unwrap();
        assert_eq!((report.total, report.passed), (2, 2), "{report:?}");

        // A proposal that catches the example with a wider rule up front.
        let mut proposal = rules();
        proposal.rules.insert(
            0,
            Rule {
                patterns: vec!["*/*".into()],
                browser: "Chrome".into(),
                ..Rule::default()
            },
        );
        let report = tester.test(Some(&proposal)).unwrap();
        assert_eq!(report.failures.len(), 2, "{report:?}");
        assert_eq!(
            RulesFile::load(&path).unwrap(),
            rules(),
            "a proposal is never written"
        );

        // A change to the file is seen on the next job.
        proposal.save(&path).unwrap();
        let report = tester.test(None).unwrap();
        assert_eq!(report.failures.len(), 2, "{report:?}");

        assert_eq!(
            tester
                .collisions(
                    "*.example.org/*",
                    &["https://a.example.org/x".into(), "https://b.net/".into()]
                )
                .unwrap(),
            ["https://a.example.org/x"]
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
