//! The dispatch thread: normalize, resolve, launch, record.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, PoisonError,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use anyhow::Result;
use pwrroute_engine::{
    AppType, Browser, Engine, HttpNetwork, Link as EngineLink, Network, Opener, ShortLinks, Source,
    Stage, Trace,
};

use crate::{
    ipc::{Request, Response},
    journal::{Journal, JournalConfig, JournalMode},
    launcher::Launcher,
    link::{Link, normalize_url},
    recent::{RecentDispatches, Retention},
    trace::{DispatchId, DispatchRecord, Matched, Outcome, Owner, Privacy, Step, Timings},
};

/// Where links go when no config is loaded, the config sends a link back
/// to PwrRoute, or the chosen browser cannot be opened: the browser every
/// install of the OS has. [`Options::fallback`] replaces it with the
/// user's choice.
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
    /// Resolves a link whose short link, if any, the router already
    /// expanded: `link.url` is what the rules see, `link.original_url` what
    /// arrived.
    fn resolve(&self, link: &EngineLink, opener: Option<&Opener>) -> Result<Resolved>;

    /// Which links the router expands before [`resolve`](Self::resolve), as
    /// the config in charge says; `None` expands nothing. Called on the
    /// dispatch thread before each resolve, and done with before the
    /// expansion runs, so a resolver that borrows an engine to answer
    /// releases it first.
    fn short_links(&self) -> Option<ShortLinks> {
        None
    }

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
    /// `impl Resolver for Engine` fills it from the trace (`rule_steps`).
    pub rule_steps: Vec<Step>,
    /// How much of the link the record may keep: the tightest level of the
    /// rules that routed it, as the engine's `Browser::privacy` carries it.
    /// [`Privacy::Full`] when the resolver does not say.
    pub privacy: Privacy,
}

impl Resolver for Engine {
    fn resolve(&self, link: &EngineLink, opener: Option<&Opener>) -> Result<Resolved> {
        let trace = self.trace(link, opener)?;
        Ok(Resolved {
            rule_steps: rule_steps(&trace),
            privacy: trace.resolution.browser.privacy,
            browser: trace.resolution.browser,
            error: trace.resolution.error,
        })
    }

    fn short_links(&self) -> Option<ShortLinks> {
        Some(Engine::short_links(self).clone())
    }
}

/// The router's rule steps from the engine's trace: each rewrite that ran,
/// then what chose the browser, named by the list it came from, so a
/// record says who decided without comparing browsers:
///
/// - a JS config handler is [`Matched::Handler`] and a `rules.json` handler
///   is [`Matched::Rule`], each by its own place (the engine numbers
///   `rules.json` rules by file position, disabled ones counted), whichever
///   order `rules.json`'s `precedence` ran them in;
/// - a default is [`Matched::Default`] with its owner from the trace step's
///   source: the config's `defaultBrowser` is [`Owner::Config`], and
///   `rules.json`'s (no config in charge, or owning everything else under
///   `"precedence": "pwrroute"`) is [`Owner::PwrRoute`].
///
/// A rewrite's rule number is the config's only: [`Step::Rewrite`] cannot
/// say which list a number belongs to, so a `rules.json` rewrite has none.
fn rule_steps(trace: &Trace) -> Vec<Step> {
    let mut steps: Vec<Step> = trace
        .rewrites_applied()
        .filter_map(|s| {
            Some(Step::Rewrite {
                rule: (s.source == Source::Config).then_some(s.index),
                url: s.rewritten_to.clone()?,
            })
        })
        .collect();
    let by = match trace.winner() {
        None => Matched::Unreported,
        Some(s) => match (s.stage, s.source) {
            (Stage::Default, source) => Matched::Default {
                owner: Some(Owner::from(source)),
            },
            (_, Source::Config) => Matched::Handler { index: s.index },
            (_, Source::Rules) => Matched::Rule { index: s.index },
        },
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

/// Called on the thread that hands a link to the router (AppKit's, for an
/// Apple Event; the IPC server's, for a forwarded link), right after it is
/// queued, with its id and the instant its timings count from. Not for Test
/// a link. Keep it quick: the OS edge waits for it.
pub type ReceivedCallback = Arc<dyn Fn(DispatchId, Instant) + Send + Sync>;

/// A dispatch that took this long, from receipt to the launcher's return, is
/// logged as a warning.
pub const SLOW_DISPATCH: Duration = Duration::from_millis(500);

/// Where the fallback browser comes from: asked each time a link needs it,
/// so a choice saved while the router runs (first run, an edit of
/// `rules.json`) applies from the next link, without a restart. Keep the
/// call cheap; it runs on the dispatch thread, though only when a link
/// falls back.
#[derive(Clone)]
pub struct FallbackSource(Arc<dyn Fn() -> Option<(String, String)> + Send + Sync>);

impl FallbackSource {
    /// Asks `choice` for the browser (an app name, bundle id or `.app`
    /// path) and profile (empty for none). `None`, an empty name, or a name
    /// that is PwrRoute itself means [`FALLBACK_BROWSER`].
    pub fn new(choice: impl Fn() -> Option<(String, String)> + Send + Sync + 'static) -> Self {
        Self(Arc::new(choice))
    }

    /// Always `browser`, in `profile`.
    pub fn fixed(browser: impl Into<String>, profile: impl Into<String>) -> Self {
        let choice = (browser.into(), profile.into());
        Self::new(move || Some(choice.clone()))
    }

    /// The browser and profile now: never empty, never PwrRoute.
    pub fn get(&self) -> (String, String) {
        (self.0)()
            .map(|(browser, profile)| (browser.trim().to_owned(), profile.trim().to_owned()))
            .filter(|(browser, _)| !browser.is_empty() && !is_self_name(browser))
            .unwrap_or_else(|| (FALLBACK_BROWSER.to_owned(), String::new()))
    }
}

impl Default for FallbackSource {
    /// [`FALLBACK_BROWSER`], with no profile.
    fn default() -> Self {
        Self::new(|| None)
    }
}

impl std::fmt::Debug for FallbackSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Not `get()`: asking may read a file.
        f.debug_tuple("FallbackSource").finish_non_exhaustive()
    }
}

/// How the router behaves.
#[derive(Clone, Default)]
pub struct Options {
    /// How many dispatches Activity keeps in memory, and for how long.
    pub retention: Retention,
    /// The on-disk journal of dispatches. `None`, the default, writes
    /// nothing: links live in memory only.
    pub journal: Option<JournalConfig>,
    /// The browser and profile used when no config is loaded, the config
    /// would loop, or the browser a rule chose cannot be opened.
    /// [`FALLBACK_BROWSER`] by default.
    pub fallback: FallbackSource,
    pub on_dispatch: Option<DispatchCallback>,
    pub on_received: Option<ReceivedCallback>,
    /// The diagnostic log, when the app writes one. While links are kept at
    /// all (in memory or the journal), the dispatch thread logs one URL-free
    /// line per dispatch to the `log` facade ([`DispatchRecord::log_line`]),
    /// and the retention description names this path.
    pub diagnostic_log: Option<PathBuf>,
    /// What expands short links: the one request a dispatch makes off the
    /// machine, to the shortener's host, as Finicky made it. `None`, the
    /// default, is the real network ([`HttpNetwork`]), built on the dispatch
    /// thread the first time a short link arrives; tests pass a fake.
    pub network: Option<Arc<dyn Network + Send + Sync>>,
}

impl Options {
    /// Where dispatches are kept and for how long, in words the Activity
    /// screen and an agent can show verbatim.
    pub fn retention_description(&self) -> String {
        describe_retention(
            self.retention,
            self.journal.as_ref(),
            self.diagnostic_log.as_deref(),
        )
    }
}

impl std::fmt::Debug for Options {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Options")
            .field("retention", &self.retention)
            .field("journal", &self.journal)
            .field("fallback", &self.fallback)
            .field("on_dispatch", &self.on_dispatch.is_some())
            .field("on_received", &self.on_received.is_some())
            .field("diagnostic_log", &self.diagnostic_log)
            .field("network", &self.network.is_some())
            .finish()
    }
}

fn describe_retention(
    retention: Retention,
    journal: Option<&JournalConfig>,
    log: Option<&Path>,
) -> String {
    let kept = describe_kept(retention, journal);
    match log {
        Some(log) if keeps_links(retention, journal) => format!(
            "{kept} The diagnostic log, {}, notes each link's timings and the apps it came from and opened in, never its address.",
            log.display()
        ),
        _ => kept,
    }
}

/// Links are kept somewhere: in memory, or in the journal.
fn keeps_links(retention: Retention, journal: Option<&JournalConfig>) -> bool {
    retention.count > 0 || journal.is_some()
}

fn describe_kept(retention: Retention, journal: Option<&JournalConfig>) -> String {
    let links = |n: usize| if n == 1 { "link" } else { "links" };
    let hours = |n: u64| if n == 1 { "hour" } else { "hours" };
    // "the last 500 links or 24 hours", or "" when nothing is kept.
    let span = match (retention.count, retention.hours) {
        (0, _) => String::new(),
        (count, 0) => format!("the last {count} {}", links(count)),
        (count, h) => format!("the last {count} {} or {h} {}", links(count), hours(h)),
    };
    let what = |journal: &JournalConfig| {
        let days = journal.days.max(1);
        format!(
            "journaled {} to {} for {days} {}",
            match journal.mode {
                JournalMode::Hosts => "as scheme and host only",
                JournalMode::Full => "with full URLs (except links whose rule keeps less)",
            },
            journal.file_pattern().display(),
            if days == 1 { "day" } else { "days" }
        )
    };
    match (span.is_empty(), journal) {
        (true, None) => "Nothing is kept: not in memory, and nothing is written to disk.".into(),
        (true, Some(journal)) => {
            format!("Nothing is kept in memory; each link is {}.", what(journal))
        }
        (false, None) => format!("Kept in memory only: {span}, cleared when PwrRoute quits."),
        (false, Some(journal)) => format!(
            "Kept in memory for {span}, cleared when PwrRoute quits, and {}.",
            what(journal)
        ),
    }
}

/// Where dispatches are kept, as [`RouterHandle::retention`] reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionInfo {
    pub memory: Retention,
    /// The journal, when it is on.
    pub journal: Option<JournalConfig>,
    /// The same, in words to show verbatim.
    pub description: String,
}

impl RetentionInfo {
    /// Links are kept somewhere, in memory or the journal. While they are
    /// not, nothing about them is logged either.
    pub fn keeps_links(&self) -> bool {
        keeps_links(self.memory, self.journal.as_ref())
    }
}

/// What the router is running on, for status displays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterStatus {
    /// Why no config is in charge, when none is. Links then go to the
    /// fallback.
    pub config_error: Option<String>,
    pub dry_run: bool,
    /// Why the journal's last write failed, when it is on and did. Never
    /// quotes a link.
    pub journal_error: Option<String>,
}

/// Where every link goes while routing is paused: the browser the user
/// chose as the fallback, with its profile. Rewrites still run; every
/// handler is passed over, `browser: null` and `private` ones included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PauseTarget {
    /// An app name, bundle id or path, as `rules.json`'s `defaultBrowser`
    /// or a config's `defaultBrowser` names it.
    pub browser: String,
    /// Empty for the browser's own profile.
    pub profile: String,
}

impl PauseTarget {
    /// The target as the launcher opens it, for `url` (the URL after every
    /// rewrite) at the level the rules gave the link. Nothing else a
    /// handler said carries over: no extra arguments, no private window.
    fn browser_for(&self, url: &str, privacy: Privacy) -> Browser {
        Browser {
            name: self.browser.clone(),
            app_type: AppType::AppName,
            url: url.to_owned(),
            profile: self.profile.clone(),
            args: Vec::new(),
            open_in_background: None,
            private: false,
            privacy,
        }
    }
}

/// Where the user sent links without a rule: a browser, a profile and
/// whether to open a private window. Like a pause target, it takes a link
/// after the rules ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    /// An app name, bundle id or path.
    pub browser: String,
    /// Empty for the browser's own profile.
    pub profile: String,
    /// Open a private window, where the browser has one.
    pub private: bool,
}

impl Destination {
    /// The destination as the launcher opens it, for `url` (after every
    /// rewrite). A private window keeps at most the host, as a private
    /// rule does.
    fn browser_for(&self, url: &str, privacy: Privacy) -> Browser {
        Browser {
            app_type: app_type_of(&self.browser),
            name: self.browser.clone(),
            url: url.to_owned(),
            profile: self.profile.clone(),
            args: Vec::new(),
            open_in_background: None,
            private: self.private,
            privacy: if self.private {
                privacy.tighter(Privacy::Host)
            } else {
                privacy
            },
        }
    }
}

/// How many links a send takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendScope {
    /// The next `left` links, of `of` asked for; then the rules again.
    Next { left: u32, of: u32 },
    /// Every link until switched back, or until `until`.
    All { until: Option<SystemTime> },
}

/// Links sent to a [`Destination`] instead of where the rules would send
/// them, for the next few or for all of them. In memory only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendTo {
    pub destination: Destination,
    pub scope: SendScope,
}

impl SendTo {
    /// The next link, once.
    pub fn next(destination: Destination) -> Self {
        Self {
            destination,
            scope: SendScope::Next { left: 1, of: 1 },
        }
    }

    /// Whether an `All` with a time limit has run out at `now`.
    pub fn expired(&self, now: SystemTime) -> bool {
        matches!(self.scope, SendScope::All { until: Some(until) } if until <= now)
    }
}

/// What takes links from the rules, when something does: a pause or a
/// send. One slot, so turning one on turns the other off.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Hold {
    Paused(PauseTarget),
    Sending(SendTo),
}

/// The hold the handle writes and the dispatch thread reads.
type Held = Arc<Mutex<Option<Hold>>>;

/// What the dispatch thread took from the hold for one link.
enum Took {
    Paused(PauseTarget),
    Sent(Destination, crate::trace::Sent),
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
    SetRetention {
        retention: Retention,
        reply: mpsc::Sender<()>,
    },
    SetJournal {
        config: Option<JournalConfig>,
        reply: mpsc::Sender<()>,
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
        let on_received = options.on_received.clone();
        let diagnostic_log = options.diagnostic_log.clone();
        let recent = Arc::new(RecentDispatches::new(options.retention));
        let worker_recent = Arc::clone(&recent);
        let journal = Arc::new(Mutex::new(options.journal.clone()));
        let worker_journal = Arc::clone(&journal);
        let held: Held = Arc::default();
        let worker_held = Arc::clone(&held);
        let thread = thread::Builder::new()
            .name("pwrroute-dispatch".into())
            .spawn(move || {
                let network = OnceLock::new();
                if let Some(fake) = options.network.clone() {
                    let _ = network.set(fake);
                }
                Worker {
                    resolver: build(resolver),
                    launcher: Box::new(launcher),
                    recent: worker_recent,
                    journal: options.journal.clone().map(Journal::new),
                    journal_config: worker_journal,
                    held: worker_held,
                    network,
                    options,
                }
                .run(&rx);
            })?;
        Ok(Self {
            handle: RouterHandle {
                tx,
                recent,
                journal,
                held,
                next_id: Arc::new(AtomicU64::new(1)),
                on_received,
                diagnostic_log,
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
    /// The journal as it is on: what [`retention`](Self::retention) and
    /// [`journal`](Self::journal) describe. The dispatch thread writes it
    /// when [`set_journal`](Self::set_journal) takes effect.
    journal: Arc<Mutex<Option<JournalConfig>>>,
    /// A pause or a send, when one is on. Read by the dispatch thread for
    /// each link; written by [`set_paused`] and [`set_send_to`].
    ///
    /// [`set_paused`]: Self::set_paused
    /// [`set_send_to`]: Self::set_send_to
    held: Held,
    next_id: Arc<AtomicU64>,
    on_received: Option<ReceivedCallback>,
    diagnostic_log: Option<PathBuf>,
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
        let (id, received) = (request.id, request.received);
        self.send(Command::Dispatch {
            request,
            reply: Some(reply),
        })?;
        self.received(id, received);
        rx.recv().map_err(|_| RouterGone)
    }

    /// Test a link: resolves it and returns the trace without opening it or
    /// recording it in Activity. The rules answer, paused or not: a pause
    /// only changes where dispatches go.
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

    /// Where dispatches are kept and for how long: the in-memory retention,
    /// the journal when it is on, and both in words. Describes the values
    /// in force now, after any [`set_retention`](Self::set_retention) or
    /// [`set_journal`](Self::set_journal).
    pub fn retention(&self) -> RetentionInfo {
        let memory = self.recent.retention();
        let journal = self.journal_config();
        RetentionInfo {
            description: describe_retention(
                memory,
                journal.as_ref(),
                self.diagnostic_log.as_deref(),
            ),
            memory,
            journal,
        }
    }

    /// Journaled dispatches, newest first, at most `limit`. Empty when the
    /// journal is off. Each is reduced to what the journal kept, with
    /// [`DispatchRecord::privacy`] saying how much.
    pub fn journal(&self, limit: usize) -> std::io::Result<Vec<DispatchRecord>> {
        match self.journal_config() {
            Some(journal) => journal.read(limit),
            None => Ok(Vec::new()),
        }
    }

    /// Changes how many dispatches stay in memory, and for how long, from
    /// now on: the buffer drops at once whatever the new retention does not
    /// keep. Waits until the dispatch thread has applied it, so
    /// [`retention`](Self::retention) describes the new values on return.
    pub fn set_retention(&self, retention: Retention) -> std::result::Result<(), RouterGone> {
        let (reply, rx) = mpsc::channel();
        self.send(Command::SetRetention { retention, reply })?;
        rx.recv().map_err(|_| RouterGone)
    }

    /// Turns the journal on, changes it, or turns it off (`None`), from the
    /// next dispatch on. Turning it off closes the day's file and deletes
    /// nothing; the files stay until they are pruned or cleared. Waits
    /// until the dispatch thread has applied it, so
    /// [`retention`](Self::retention) and [`journal`](Self::journal) see the
    /// change on return.
    pub fn set_journal(
        &self,
        config: Option<JournalConfig>,
    ) -> std::result::Result<(), RouterGone> {
        let (reply, rx) = mpsc::channel();
        self.send(Command::SetJournal { config, reply })?;
        rx.recv().map_err(|_| RouterGone)
    }

    /// Pauses routing: from the next dispatch on, every link goes to
    /// `target` after its rewrites, with [`Step::Paused`] in its trace
    /// naming the handler it skipped. `None` resumes. The record keeps the
    /// privacy level the rules gave it, since the rewrites still ran and a
    /// rule that keeps less of a link keeps less of it paused too.
    /// Pausing ends a send; resuming leaves a send alone.
    pub fn set_paused(&self, target: Option<PauseTarget>) {
        let mut held = self.held();
        match target {
            Some(target) => *held = Some(Hold::Paused(target)),
            None if matches!(*held, Some(Hold::Paused(_))) => *held = None,
            None => {}
        }
    }

    /// Where links go while paused, or `None` while routing.
    pub fn paused(&self) -> Option<PauseTarget> {
        match &*self.held() {
            Some(Hold::Paused(target)) => Some(target.clone()),
            _ => None,
        }
    }

    /// Sends links to a destination instead of where the rules would: the
    /// next few, or all of them for a while. From the next dispatch on,
    /// each link goes there after its rewrites, with [`Step::Sent`] in its
    /// trace, and a `Next` counts down one per link on the dispatch thread
    /// (forwarded links included) until the rules decide again. Sending
    /// ends a pause; `None` ends a send and leaves a pause alone.
    pub fn set_send_to(&self, send: Option<SendTo>) {
        let mut held = self.held();
        match send {
            Some(send) => *held = Some(Hold::Sending(send)),
            None if matches!(*held, Some(Hold::Sending(_))) => *held = None,
            None => {}
        }
    }

    /// The send in force, or `None`. An `All` whose time ran out is ended
    /// here, so a reader never sees it.
    pub fn send_to(&self) -> Option<SendTo> {
        let mut held = self.held();
        match &*held {
            Some(Hold::Sending(send)) if send.expired(SystemTime::now()) => {
                *held = None;
                None
            }
            Some(Hold::Sending(send)) => Some(send.clone()),
            _ => None,
        }
    }

    /// Points the send in force at `destination`, keeping its count or
    /// time. `false` when nothing is being sent.
    pub fn retarget_send(&self, destination: Destination) -> bool {
        match &mut *self.held() {
            Some(Hold::Sending(send)) => {
                send.destination = destination;
                true
            }
            _ => false,
        }
    }

    /// Changes how many links a `Next` send has left by `delta`, never
    /// below one, and the number asked for with it. Read and written under
    /// one lock, so a link dispatched meanwhile is never counted twice.
    /// Returns the send as it is now.
    pub fn add_to_next(&self, delta: i64) -> Option<SendTo> {
        match &mut *self.held() {
            Some(Hold::Sending(send)) => {
                if let SendScope::Next { left, of } = &mut send.scope {
                    let new_left = (i64::from(*left) + delta).clamp(1, i64::from(MAX_NEXT));
                    let change = new_left - i64::from(*left);
                    *left = u32::try_from(new_left).unwrap_or(1);
                    *of = u32::try_from((i64::from(*of) + change).max(new_left)).unwrap_or(*left);
                }
                Some(send.clone())
            }
            _ => None,
        }
    }

    fn held(&self) -> std::sync::MutexGuard<'_, Option<Hold>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn journal_config(&self) -> Option<JournalConfig> {
        self.journal
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
        let (id, received) = (request.id, request.received);
        self.send(Command::Dispatch {
            request,
            reply: None,
        })?;
        self.received(id, received);
        Ok(id)
    }

    fn received(&self, id: DispatchId, at: Instant) {
        if let Some(callback) = &self.on_received {
            callback(id, at);
        }
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
    journal: Option<Journal>,
    /// The handle's copy of the journal's config, kept in step with
    /// `journal` so a reader never sees a journal that is not the one being
    /// written.
    journal_config: Arc<Mutex<Option<JournalConfig>>>,
    /// The handle's pause or send, read once per dispatch.
    held: Held,
    /// `Options::network`, or the real one once a short link needed it.
    network: OnceLock<Arc<dyn Network + Send + Sync>>,
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
                    let mut record = self.process(request, Mode::Dispatch);
                    // Reduced once, here, before anything outside this
                    // thread sees it: the buffer, the journal, the callback
                    // and the reply all get the same record.
                    record.reduce(record.privacy);
                    self.log(&record);
                    self.recent.push(record.clone());
                    if let Some(journal) = &mut self.journal {
                        journal.append(&record);
                    }
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
                        journal_error: self
                            .journal
                            .as_ref()
                            .and_then(|j| j.error().map(str::to_owned)),
                    });
                }
                Command::SetRetention { retention, reply } => {
                    self.recent.set_retention(retention);
                    self.options.retention = retention;
                    let _ = reply.send(());
                }
                Command::SetJournal { config, reply } => {
                    // The old writer is dropped first: its day's file closes
                    // before a new one could open the same path.
                    self.journal = None;
                    self.journal = config.clone().map(Journal::new);
                    *self
                        .journal_config
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner) = config.clone();
                    self.options.journal = config;
                    let _ = reply.send(());
                }
                Command::Refresh => {
                    let _ = self.in_charge();
                }
                Command::Shutdown => break,
            }
        }
    }

    /// The record's line in the diagnostic log, while links are kept at all:
    /// "Nothing is kept" means nothing, here too.
    fn log(&self, record: &DispatchRecord) {
        let kept = self.recent.retention().count > 0 || self.journal.is_some();
        if self.options.diagnostic_log.is_none() || !kept {
            return;
        }
        let slow = record.timings.total_us >= Timings::micros(SLOW_DISPATCH);
        let level = if slow {
            log::Level::Warn
        } else {
            log::Level::Info
        };
        log::log!(target: "pwrroute::link", level, "{}", record.log_line());
    }

    fn process(&mut self, request: Queued, mode: Mode) -> DispatchRecord {
        let dequeued = Instant::now();
        let mut record = DispatchRecord {
            id: request.id,
            received_at: request.received_at,
            privacy: Privacy::Full,
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

        let (mut browser, privacy) =
            self.resolve(&url, &record.link, &mut record.steps, &mut record.timings);
        // Paused: the rules ran (so the rewrites and the privacy level
        // stand), and the pause target takes the link from whatever they
        // chose. A test still reports what the rules say.
        let mut privacy = privacy;
        if mode == Mode::Dispatch {
            match self.take_hold() {
                Some(Took::Paused(target)) => {
                    record.steps.push(Step::Paused {
                        skipped: skipped_handler(&record.steps),
                    });
                    browser = target.browser_for(&browser.url, privacy);
                }
                Some(Took::Sent(destination, sent)) => {
                    record.steps.push(Step::Sent {
                        sent,
                        skipped: skipped_handler(&record.steps),
                    });
                    browser = destination.browser_for(&browser.url, privacy);
                    privacy = browser.privacy;
                }
                None => {}
            }
        }
        record.timings.resolve_us = Timings::micros(dequeued.elapsed());
        record.final_url = Some(browser.url.clone());
        // Noted here; the record is reduced when it is dispatched, not
        // when it is tested.
        record.privacy = privacy;

        if browser.app_type == AppType::None {
            record.steps.push(Step::NotOpened);
            record.outcome = Outcome::NotOpened;
        } else {
            record.steps.push(Step::Browser {
                browser: browser.clone(),
            });
            let mut opened = None;
            record.outcome = match mode {
                Mode::Test => Outcome::Tested,
                Mode::Dispatch => {
                    let launch = Instant::now();
                    let launched = self.launch(&browser, &mut record.steps, record.id);
                    record.timings.launch_us = Timings::micros(launch.elapsed());
                    opened = launched.opened;
                    launched.outcome
                }
            };
            // The browser that took the link: the fallback when the chosen
            // one could not be opened, so Activity and the tray name it.
            record.browser = Some(opened.unwrap_or(browser));
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

    /// The browser for `url`, with every step that chose it, and how much
    /// of the link its rule lets the record keep. A link no rule routed is
    /// kept in full. A short link is expanded first, so the rules match
    /// where it lands.
    fn resolve(
        &self,
        url: &str,
        link: &Link,
        steps: &mut Vec<Step>,
        timings: &mut Timings,
    ) -> (Browser, Privacy) {
        let resolver = match self.in_charge() {
            Ok(resolver) => resolver,
            Err(reason) => {
                steps.push(Step::Fallback { reason });
                return (self.fallback(url), Privacy::Full);
            }
        };
        let opener = link.opener.as_ref().map(Opener::from);
        let expanded = self.expand(resolver, url, steps, timings);
        let url = expanded.url.as_str();
        let mut privacy = Privacy::Full;
        let mut browser = match resolver.resolve(&expanded, opener.as_ref()) {
            Ok(resolved) => {
                privacy = resolved.privacy;
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
        (browser, privacy)
    }

    /// Expands `url` when the resolver's shortener list says it is a short
    /// link: the one request a dispatch makes off the machine, made here on
    /// the dispatch thread, after the resolver's list is read and before
    /// its resolve. The link goes on unchanged when it is not short, the
    /// list is empty, or the shortener does not answer; nothing is logged.
    fn expand(
        &self,
        resolver: &dyn Resolver,
        url: &str,
        steps: &mut Vec<Step>,
        timings: &mut Timings,
    ) -> EngineLink {
        let Some(short_links) = resolver.short_links() else {
            return EngineLink::new(url);
        };
        if !short_links.is_short(url) {
            return EngineLink::new(url);
        }
        let network = self
            .network
            .get_or_init(|| Arc::new(HttpNetwork::new()) as Arc<dyn Network + Send + Sync>);
        let started = Instant::now();
        let link = short_links.expand(url, network.as_ref());
        // At least 1 µs, so the log can tell "tried" from "not short".
        timings.expand_us = Timings::micros(started.elapsed()).max(1);
        if link.was_expanded() {
            steps.push(Step::Expanded {
                url: link.url.clone(),
            });
        }
        link
    }

    /// Opens the link in `browser`, or, when that fails and `browser` is not
    /// the fallback already, in the fallback browser: a rule that names a
    /// browser this machine does not have must not lose the link. The
    /// fallback is tried once; `Failed` means both failed.
    fn launch(&mut self, browser: &Browser, steps: &mut Vec<Step>, id: DispatchId) -> Launched {
        if is_self(browser) {
            steps.push(Step::LaunchFailed {
                message: "the fallback browser is PwrRoute itself".into(),
            });
            return Launched::failed();
        }
        if let Ok(outcome) = self.try_launch(browser, steps, id, 1) {
            return Launched {
                outcome,
                opened: None,
            };
        }
        let mut fallback = self.fallback(&browser.url);
        if same_app(browser, &fallback) || is_self(&fallback) {
            return Launched::failed();
        }
        // A private window was asked for: the fallback opens one too, so a
        // missing browser never puts a private link in a normal window.
        fallback.private = browser.private;
        // The browser's name only: the launcher's message is withheld in a
        // host-only record, this reason is kept.
        steps.push(Step::Fallback {
            reason: format!("{} could not be opened", browser.name),
        });
        match self.try_launch(&fallback, steps, id, 2) {
            Ok(outcome) => Launched {
                outcome,
                opened: Some(fallback),
            },
            Err(()) => Launched::failed(),
        }
    }

    /// One launch, traced: `Launched` on success, `LaunchFailed` with the
    /// launcher's message otherwise.
    fn try_launch(
        &mut self,
        browser: &Browser,
        steps: &mut Vec<Step>,
        id: DispatchId,
        attempt: u8,
    ) -> Result<Outcome, ()> {
        let dry_run = self.launcher.is_dry_run();
        let kept = self.recent.retention().count > 0 || self.journal.is_some();
        let result = if self.options.diagnostic_log.is_some() && kept {
            self.launcher.launch_observed(browser, &mut |detail| {
                log::info!(target: "pwrroute::link", "link {} launch attempt={attempt}: {detail}", id.0);
            })
        } else {
            self.launcher.launch(browser)
        };
        match result {
            Ok(()) => {
                steps.push(Step::Launched { dry_run });
                Ok(if dry_run {
                    Outcome::DryRun
                } else {
                    Outcome::Opened
                })
            }
            Err(e) => {
                steps.push(Step::LaunchFailed {
                    message: format!("{e:#}"),
                });
                Err(())
            }
        }
    }

    /// What takes this link from the rules, if anything. A `Next` send
    /// counts the link here, under the lock, and ends when it reaches
    /// zero; an `All` whose time ran out ends without taking it.
    fn take_hold(&self) -> Option<Took> {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        match &mut *held {
            None => None,
            Some(Hold::Paused(target)) => Some(Took::Paused(target.clone())),
            Some(Hold::Sending(send)) if send.expired(SystemTime::now()) => {
                *held = None;
                None
            }
            Some(Hold::Sending(send)) => {
                let destination = send.destination.clone();
                let sent = match &mut send.scope {
                    SendScope::All { .. } => crate::trace::Sent::All,
                    SendScope::Next { left, of } => {
                        let nth = of.saturating_sub(*left) + 1;
                        let of = *of;
                        *left = left.saturating_sub(1);
                        if *left == 0 {
                            *held = None;
                        }
                        crate::trace::Sent::Next { nth, of }
                    }
                };
                Some(Took::Sent(destination, sent))
            }
        }
    }

    /// The fallback browser with `url`: [`Options::fallback`] as it is now,
    /// typed as Finicky types a browser string.
    fn fallback(&self, url: &str) -> Browser {
        let (name, profile) = self.options.fallback.get();
        Browser {
            app_type: app_type_of(&name),
            name,
            url: url.to_owned(),
            profile,
            args: Vec::new(),
            open_in_background: None,
            private: false,
            privacy: Privacy::Full,
        }
    }
}

/// The most links one `Next` send can take.
pub const MAX_NEXT: u32 = 99;

/// The config handler the trace says chose the browser, for
/// [`Step::Paused`]'s and [`Step::Sent`]'s `skipped`, which numbers the config's handlers only
/// (records journaled before `rules.json` rules were numbered read the
/// same). `None` when the default chose it, when nothing was in charge, or
/// when a `rules.json` rule did: that rule is the record's own
/// [`Matched::Rule`], just before the pause, so a reader names it from
/// there ("paused · skipped rule 2").
fn skipped_handler(steps: &[Step]) -> Option<usize> {
    steps.iter().find_map(|step| match step {
        Step::Matched { by } => match by {
            Matched::Handler { index } => Some(*index),
            Matched::Rule { .. } | Matched::Default { .. } | Matched::Unreported => None,
        },
        _ => None,
    })
}

/// What [`Worker::launch`] did.
struct Launched {
    outcome: Outcome,
    /// The fallback browser, when it opened the link instead of the one
    /// chosen.
    opened: Option<Browser>,
}

impl Launched {
    fn failed() -> Self {
        Self {
            outcome: Outcome::Failed,
            opened: None,
        }
    }
}

/// Whether two browsers name the same app the same way (type and name,
/// whatever the profile): a launch that failed in one would fail in the
/// other. The same app named two ways (`Google Chrome`, `com.google.Chrome`)
/// is not caught, and costs one more attempt.
fn same_app(a: &Browser, b: &Browser) -> bool {
    a.app_type == b.app_type && a.name.eq_ignore_ascii_case(&b.name)
}

/// How Finicky types a browser string (`autodetectAppStringType`): letters,
/// digits and spaces are an app name, letters, digits, dots and dashes a
/// bundle id, an absolute or `~/` path ending in `.app` a path, and
/// anything else an app name.
fn app_type_of(name: &str) -> AppType {
    let name = name.trim();
    let is_app_path = (name.starts_with('/') || name.starts_with("~/"))
        && std::path::Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("app"));
    if name.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ') {
        AppType::AppName
    } else if name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
    {
        AppType::BundleId
    } else if is_app_path {
        AppType::Path
    } else {
        AppType::AppName
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
    is_self_name(&browser.name)
}

/// Whether a browser name (an app name, bundle id or path) is PwrRoute.
fn is_self_name(name: &str) -> bool {
    name.to_ascii_lowercase().contains("pwrroute")
}

#[cfg(test)]
mod tests;
