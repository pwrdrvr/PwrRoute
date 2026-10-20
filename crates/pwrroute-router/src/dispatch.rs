//! The dispatch thread: normalize, resolve, launch, record.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Instant, SystemTime},
};

use anyhow::Result;
use pwrroute_engine::{AppType, Browser, Engine, Opener, Source, Stage, Step as TraceStep, Trace};

use crate::{
    ipc::{Request, Response},
    launcher::Launcher,
    link::{Link, normalize_url},
    recent::RecentDispatches,
    trace::{DispatchId, DispatchRecord, Matched, Outcome, Step, Timings},
};

/// Where links go when no config is loaded, or the config sends a link back
/// to PwrRoute: the browser every install of the OS has.
pub const FALLBACK_BROWSER: &str = if cfg!(target_os = "macos") {
    "Safari"
} else if cfg!(windows) {
    "Microsoft Edge"
} else {
    "Firefox"
};

/// Resolves a URL to a browser. [`Engine`] is the real one; tests use fixed
/// answers. Built and used on the dispatch thread only, so it need not be
/// `Send` (QuickJS is not).
pub trait Resolver {
    fn resolve(&self, url: &str, opener: Option<&Opener>) -> Result<Resolved>;

    /// Catches up with the files behind the resolver and says whether
    /// anything is in charge. The router calls it before each resolve, for
    /// its status, and when [`RouterHandle::refresh`] asks. `Err` is why no
    /// config is in charge: the link then goes to the fallback browser with
    /// that reason, and [`RouterStatus::config_error`] carries it. A resolver
    /// built once from a file has nothing to do here.
    fn refresh(&self) -> std::result::Result<(), String> {
        Ok(())
    }
}

/// A resolver's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub browser: Browser,
    /// A matcher, rewrite or browser function threw; `browser` is then the
    /// default, as Finicky does it.
    pub error: Option<String>,
    /// Rule-level steps, when the resolver can say which rules fired:
    /// [`Step::Rewrite`] with a rule number for each rewrite, then one
    /// [`Step::Matched`]. Left empty, the router derives what it can from the
    /// result: a rewrite step when the URL changed, and `Matched::Unreported`.
    pub rule_steps: Vec<Step>,
}

impl Resolver for Engine {
    fn resolve(&self, url: &str, opener: Option<&Opener>) -> Result<Resolved> {
        let trace = self.trace(&pwrroute_engine::Link::new(url), opener)?;
        Ok(Resolved {
            rule_steps: rule_steps(&trace),
            browser: trace.resolution.browser,
            error: trace.resolution.error,
        })
    }
}

/// The router's rule steps from the engine's trace: each rewrite that ran,
/// then what chose the browser. Rule numbers index the config's own lists;
/// `rules.json` entries, which follow them, are traced without a number
/// (`Step` cannot say which list a number belongs to).
fn rule_steps(trace: &Trace) -> Vec<Step> {
    let numbered = |s: &TraceStep| (s.source == Source::Config).then_some(s.index);
    let mut steps: Vec<Step> = trace
        .rewrites_applied()
        .filter_map(|s| {
            Some(Step::Rewrite {
                rule: numbered(s),
                url: s.rewritten_to.clone()?,
            })
        })
        .collect();
    let by = match trace.winner() {
        Some(s) if s.stage == Stage::Default => Matched::Default,
        Some(s) => numbered(s).map_or(Matched::Unreported, |index| Matched::Handler { index }),
        None => Matched::Unreported,
    };
    steps.push(Step::Matched { by });
    steps
}

/// Builds the resolver on the dispatch thread. An error leaves the router on
/// its fallback browser, with the message in every trace.
pub type ResolverFactory = Box<dyn FnOnce() -> Result<Box<dyn Resolver>> + Send>;

/// Called on the dispatch thread after each recorded dispatch (not for Test
/// a link). Keep it quick: the next link waits for it.
pub type DispatchCallback = Arc<dyn Fn(&DispatchRecord) + Send + Sync>;

/// How the router behaves.
#[derive(Clone)]
pub struct Options {
    /// How many dispatches Activity keeps in memory.
    pub capacity: usize,
    /// An app name, used when no config is loaded or the config would loop.
    pub fallback_browser: String,
    pub on_dispatch: Option<DispatchCallback>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            capacity: 200,
            fallback_browser: FALLBACK_BROWSER.to_owned(),
            on_dispatch: None,
        }
    }
}

impl std::fmt::Debug for Options {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Options")
            .field("capacity", &self.capacity)
            .field("fallback_browser", &self.fallback_browser)
            .field("on_dispatch", &self.on_dispatch.is_some())
            .finish()
    }
}

/// What the router is running on, for status displays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterStatus {
    /// Why no config is in charge, when none is. Links then go to the
    /// fallback.
    pub config_error: Option<String>,
    pub dry_run: bool,
}

enum Command {
    Dispatch {
        request: Queued,
        reply: Option<mpsc::Sender<DispatchRecord>>,
    },
    Test {
        request: Queued,
        reply: mpsc::Sender<DispatchRecord>,
    },
    Replace {
        factory: ResolverFactory,
        reply: mpsc::Sender<std::result::Result<(), String>>,
    },
    Status {
        reply: mpsc::Sender<RouterStatus>,
    },
    Refresh,
    Shutdown,
}

struct Queued {
    id: DispatchId,
    link: Link,
    forwarded: bool,
    received: Instant,
    received_at: SystemTime,
}

/// The resident router. Dropping it finishes the link in hand and stops the
/// dispatch thread; outstanding [`RouterHandle`]s then report it gone.
pub struct Router {
    handle: RouterHandle,
    thread: Option<thread::JoinHandle<()>>,
}

impl std::fmt::Debug for Router {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Router").finish_non_exhaustive()
    }
}

impl Router {
    /// Starts the dispatch thread. `resolver` runs on it first; `None` means
    /// no config is selected, and every link goes to the fallback browser.
    pub fn spawn(
        options: Options,
        resolver: Option<ResolverFactory>,
        launcher: impl Launcher,
    ) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let recent = Arc::new(RecentDispatches::new(options.capacity));
        let worker_recent = Arc::clone(&recent);
        let thread = thread::Builder::new()
            .name("pwrroute-dispatch".into())
            .spawn(move || {
                Worker {
                    resolver: build(resolver),
                    launcher: Box::new(launcher),
                    recent: worker_recent,
                    options,
                }
                .run(&rx);
            })?;
        Ok(Self {
            handle: RouterHandle {
                tx,
                recent,
                next_id: Arc::new(AtomicU64::new(1)),
            },
            thread: Some(thread),
        })
    }

    pub fn handle(&self) -> RouterHandle {
        self.handle.clone()
    }
}

impl Drop for Router {
    fn drop(&mut self) {
        let _ = self.handle.tx.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The router stopped; nothing can be dispatched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouterGone;

impl std::fmt::Display for RouterGone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the router has stopped")
    }
}

impl std::error::Error for RouterGone {}

/// Feeds and reads a [`Router`] from any thread.
#[derive(Clone)]
pub struct RouterHandle {
    tx: mpsc::Sender<Command>,
    recent: Arc<RecentDispatches>,
    next_id: Arc<AtomicU64>,
}

impl std::fmt::Debug for RouterHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RouterHandle").finish_non_exhaustive()
    }
}

impl RouterHandle {
    /// Queues a link and returns at once. The record lands in [`recent`]
    /// when the dispatch finishes.
    ///
    /// [`recent`]: Self::recent
    pub fn dispatch(&self, link: Link) -> std::result::Result<DispatchId, RouterGone> {
        self.submit(link, false)
    }

    /// Queues a link a second launch forwarded over IPC.
    pub fn dispatch_forwarded(&self, link: Link) -> std::result::Result<DispatchId, RouterGone> {
        self.submit(link, true)
    }

    /// Dispatches a link and waits for its record.
    pub fn dispatch_and_wait(&self, link: Link) -> std::result::Result<DispatchRecord, RouterGone> {
        let (reply, rx) = mpsc::channel();
        let request = self.request(link, false);
        self.send(Command::Dispatch {
            request,
            reply: Some(reply),
        })?;
        rx.recv().map_err(|_| RouterGone)
    }

    /// Test a link: resolves it and returns the trace without opening it or
    /// recording it in Activity.
    pub fn test(&self, link: Link) -> std::result::Result<DispatchRecord, RouterGone> {
        let (reply, rx) = mpsc::channel();
        let request = self.request(link, false);
        self.send(Command::Test { request, reply })?;
        rx.recv().map_err(|_| RouterGone)
    }

    /// Swaps in a new resolver, built on the dispatch thread, after a config
    /// change. If it fails to build, the current one stays and the error is
    /// returned.
    pub fn replace_resolver(
        &self,
        factory: ResolverFactory,
    ) -> std::result::Result<std::result::Result<(), String>, RouterGone> {
        let (reply, rx) = mpsc::channel();
        self.send(Command::Replace { factory, reply })?;
        rx.recv().map_err(|_| RouterGone)
    }

    pub fn status(&self) -> std::result::Result<RouterStatus, RouterGone> {
        let (reply, rx) = mpsc::channel();
        self.send(Command::Status { reply })?;
        rx.recv().map_err(|_| RouterGone)
    }

    /// Asks the resolver to catch up with its files now ([`Resolver::refresh`])
    /// rather than at the next link, so [`status`] is current. Returns at
    /// once; a file watcher calls it from its own thread.
    ///
    /// [`status`]: Self::status
    pub fn refresh(&self) -> std::result::Result<(), RouterGone> {
        self.send(Command::Refresh)
    }

    /// Recent dispatches, newest first.
    pub fn recent(&self) -> Vec<DispatchRecord> {
        self.recent.snapshot()
    }

    /// One recent dispatch, while it is still in the buffer.
    pub fn get(&self, id: DispatchId) -> Option<DispatchRecord> {
        self.recent.get(id)
    }

    pub fn clear_recent(&self) {
        self.recent.clear();
    }

    /// Answers a second launch's IPC request: queues forwarded links, and
    /// calls `on_activate` when it asks for the window. The app's IPC server
    /// is `move |request| handle.answer(request, …)`.
    pub fn answer(&self, request: Request, on_activate: impl FnOnce()) -> Response {
        match request {
            Request::Open { links } => {
                let mut response = Response::ok();
                for link in links {
                    match self.dispatch_forwarded(link) {
                        Ok(id) => response.dispatched.push(id),
                        Err(e) => return Response::error(e.to_string()),
                    }
                }
                response
            }
            Request::Activate => {
                on_activate();
                Response::ok()
            }
            Request::Ping => Response::ok(),
        }
    }

    fn submit(&self, link: Link, forwarded: bool) -> std::result::Result<DispatchId, RouterGone> {
        let request = self.request(link, forwarded);
        let id = request.id;
        self.send(Command::Dispatch {
            request,
            reply: None,
        })?;
        Ok(id)
    }

    fn request(&self, link: Link, forwarded: bool) -> Queued {
        Queued {
            id: DispatchId(self.next_id.fetch_add(1, Ordering::Relaxed)),
            link,
            forwarded,
            received: Instant::now(),
            received_at: SystemTime::now(),
        }
    }

    fn send(&self, command: Command) -> std::result::Result<(), RouterGone> {
        self.tx.send(command).map_err(|_| RouterGone)
    }
}

fn build(factory: Option<ResolverFactory>) -> std::result::Result<Box<dyn Resolver>, String> {
    let factory = factory.ok_or_else(|| "no config is selected".to_owned())?;
    factory().map_err(|e| format!("{e:#}"))
}

struct Worker {
    resolver: std::result::Result<Box<dyn Resolver>, String>,
    launcher: Box<dyn Launcher>,
    recent: Arc<RecentDispatches>,
    options: Options,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Dispatch,
    Test,
}

impl Worker {
    fn run(mut self, rx: &mpsc::Receiver<Command>) {
        while let Ok(command) = rx.recv() {
            match command {
                Command::Dispatch { request, reply } => {
                    let record = self.process(request, Mode::Dispatch);
                    self.recent.push(record.clone());
                    if let Some(callback) = &self.options.on_dispatch {
                        callback(&record);
                    }
                    if let Some(reply) = reply {
                        let _ = reply.send(record);
                    }
                }
                Command::Test { request, reply } => {
                    let _ = reply.send(self.process(request, Mode::Test));
                }
                Command::Replace { factory, reply } => {
                    let result = match build(Some(factory)) {
                        Ok(resolver) => {
                            self.resolver = Ok(resolver);
                            Ok(())
                        }
                        Err(e) => Err(e),
                    };
                    let _ = reply.send(result);
                }
                Command::Status { reply } => {
                    let _ = reply.send(RouterStatus {
                        config_error: self.in_charge().err(),
                        dry_run: self.launcher.is_dry_run(),
                    });
                }
                Command::Refresh => {
                    let _ = self.in_charge();
                }
                Command::Shutdown => break,
            }
        }
    }

    fn process(&mut self, request: Queued, mode: Mode) -> DispatchRecord {
        let dequeued = Instant::now();
        let mut record = DispatchRecord {
            id: request.id,
            received_at: request.received_at,
            link: request.link,
            forwarded: request.forwarded,
            final_url: None,
            browser: None,
            steps: Vec::new(),
            outcome: Outcome::Failed,
            timings: Timings {
                queue_us: Timings::micros(dequeued - request.received),
                ..Timings::default()
            },
        };

        let url = match normalize_url(&record.link.url) {
            Ok(url) => url,
            Err(e) => {
                record.steps.push(Step::Rejected {
                    reason: e.to_string(),
                });
                record.timings.total_us = Timings::micros(request.received.elapsed());
                return record;
            }
        };
        record.steps.push(Step::Received { url: url.clone() });

        let browser = self.resolve(&url, &record.link, &mut record.steps);
        record.timings.resolve_us = Timings::micros(dequeued.elapsed());
        record.final_url = Some(browser.url.clone());

        if browser.app_type == AppType::None {
            record.steps.push(Step::NotOpened);
            record.outcome = Outcome::NotOpened;
        } else {
            record.steps.push(Step::Browser {
                browser: browser.clone(),
            });
            record.outcome = match mode {
                Mode::Test => Outcome::Tested,
                Mode::Dispatch => {
                    let launch = Instant::now();
                    let outcome = self.launch(&browser, &mut record.steps);
                    record.timings.launch_us = Timings::micros(launch.elapsed());
                    outcome
                }
            };
            record.browser = Some(browser);
        }
        record.timings.total_us = Timings::micros(request.received.elapsed());
        record
    }

    /// The resolver, caught up with its files; or why links go to the
    /// fallback browser instead.
    fn in_charge(&self) -> std::result::Result<&dyn Resolver, String> {
        let resolver = self.resolver.as_ref().map_err(Clone::clone)?;
        resolver.refresh()?;
        Ok(resolver.as_ref())
    }

    /// The browser for `url`, with every step that chose it.
    fn resolve(&self, url: &str, link: &Link, steps: &mut Vec<Step>) -> Browser {
        let resolver = match self.in_charge() {
            Ok(resolver) => resolver,
            Err(reason) => {
                steps.push(Step::Fallback { reason });
                return self.fallback(url);
            }
        };
        let opener = link.opener.as_ref().map(Opener::from);
        let mut browser = match resolver.resolve(url, opener.as_ref()) {
            Ok(resolved) => {
                if resolved.rule_steps.is_empty() {
                    if !same_url(&resolved.browser.url, url) {
                        steps.push(Step::Rewrite {
                            rule: None,
                            url: resolved.browser.url.clone(),
                        });
                    }
                    steps.push(Step::Matched {
                        by: Matched::Unreported,
                    });
                } else {
                    steps.extend(resolved.rule_steps);
                }
                if let Some(message) = resolved.error {
                    steps.push(Step::ConfigError { message });
                }
                resolved.browser
            }
            Err(e) => {
                steps.push(Step::ConfigError {
                    message: format!("{e:#}"),
                });
                steps.push(Step::Fallback {
                    reason: "the config could not resolve the link".into(),
                });
                self.fallback(url)
            }
        };
        if browser.app_type != AppType::None && is_self(&browser) {
            steps.push(Step::RefusedLoop {
                browser: browser.name.clone(),
            });
            browser = self.fallback(&browser.url);
        }
        browser
    }

    fn launch(&mut self, browser: &Browser, steps: &mut Vec<Step>) -> Outcome {
        if is_self(browser) {
            steps.push(Step::LaunchFailed {
                message: "the fallback browser is PwrRoute itself".into(),
            });
            return Outcome::Failed;
        }
        let dry_run = self.launcher.is_dry_run();
        match self.launcher.launch(browser) {
            Ok(()) => {
                steps.push(Step::Launched { dry_run });
                if dry_run {
                    Outcome::DryRun
                } else {
                    Outcome::Opened
                }
            }
            Err(e) => {
                steps.push(Step::LaunchFailed {
                    message: format!("{e:#}"),
                });
                Outcome::Failed
            }
        }
    }

    fn fallback(&self, url: &str) -> Browser {
        Browser {
            name: self.options.fallback_browser.clone(),
            app_type: AppType::AppName,
            url: url.to_owned(),
            profile: String::new(),
            args: Vec::new(),
            open_in_background: None,
        }
    }
}

/// Whether two URLs are the same once parsed. The engine re-serializes
/// every URL (`https://example.com` comes back with a trailing slash), so a
/// string comparison would report a rewrite that never ran.
fn same_url(a: &str, b: &str) -> bool {
    a == b || matches!((url::Url::parse(a), url::Url::parse(b)), (Ok(a), Ok(b)) if a == b)
}

/// Whether a browser is PwrRoute, by name, bundle id or path. Routing a link
/// there would bounce it straight back.
fn is_self(browser: &Browser) -> bool {
    browser.name.to_ascii_lowercase().contains("pwrroute")
}

#[cfg(test)]
mod tests;
