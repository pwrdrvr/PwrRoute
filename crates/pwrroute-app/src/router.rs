//! The resident router inside the app: the command line, one router per
//! user, and the hand-off between the OS, the IPC socket and GPUI.
//!
//! [`start`] runs before GPUI. A second launch forwards its links (or asks
//! for the window) and exits without drawing anything; the first one starts
//! the router and keeps it in [`Resident`] for as long as the app lives.

use std::{
    cell::RefCell,
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, LazyLock, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

use pwrroute_browsers::{Env, LaunchError, LaunchOptions, Note};
use pwrroute_engine::{
    ConfigChoice, Engine, Host, Limits, NativeHost, Reloader, RulesFile, ShortLinks, Sources,
    Status, Watcher,
    discovery::{self, Environment},
};
use pwrroute_router::{
    Browser, DispatchRecord, DryRunLauncher, FallbackSource, Launcher, Link, Opener, Options,
    Resolved, Resolver, ResolverFactory, Router, RouterHandle, Source,
    instance::{self, Claim, Endpoint, Server},
    ipc::Request,
};

use crate::settings_store::SettingsStore;

/// How long a second launch waits for the running router to answer.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(3);

/// How often the config, its imports and `rules.json` are rechecked for a
/// change while no link arrives (the Go host's interval). A link always
/// rechecks them first, whatever the timer last saw.
const WATCH_INTERVAL: Duration = Duration::from_millis(500);

/// This process serves the router's socket, so later launches reach it.
/// Set once by [`start`]; Settings › Diagnostics reads it.
static LISTENING: AtomicBool = AtomicBool::new(false);

/// Whether this process listens for other launches' links.
pub fn listening() -> bool {
    LISTENING.load(Ordering::Acquire)
}

/// The router's Unix socket, when the OS gives it a directory. Windows
/// has a named pipe instead, and no file.
pub fn socket_path() -> Option<PathBuf> {
    if cfg!(windows) {
        return None;
    }
    pwrroute_os::router_dir().map(|dir| Endpoint::new(dir).socket_path())
}

/// What the command line asked for.
#[derive(Debug, Default)]
pub struct Args {
    /// Links to route: a Windows ProgID or Linux `.desktop` launch, or
    /// `pwrroute URL…` from a shell.
    pub links: Vec<String>,
    /// Resolve and record, but open nothing.
    pub dry_run: bool,
    /// A Finicky config to load instead of the default locations.
    pub config: Option<PathBuf>,
    /// Open the first-run window (`--first-run`), whatever has run before.
    pub first_run: bool,
}

impl Args {
    pub fn from_env() -> Self {
        Self::parse(env::args_os().collect()).with_e2e_env(|key| env::var_os(key))
    }

    /// Under the E2E hook, a copy the OS starts for a link gets no arguments
    /// of the harness's, so the harness passes `--dry-run` and `--config` in
    /// the environment instead (e2e/CONTRACT.md, R8). Ignored otherwise.
    fn with_e2e_env(mut self, var: impl Fn(&str) -> Option<OsString>) -> Self {
        if var(crate::e2e::SOCKET_ENV).is_none() {
            return self;
        }
        if var(crate::e2e::DRY_RUN_ENV).is_some_and(|v| v == "1") {
            self.dry_run = true;
        }
        if let Some(config) = var(crate::e2e::CONFIG_ENV) {
            self.config = Some(PathBuf::from(config));
        }
        self
    }

    fn parse(raw: Vec<OsString>) -> Self {
        let mut args = Self {
            links: pwrroute_os::links_from_args(raw.iter().cloned()),
            ..Self::default()
        };
        let mut rest = raw.iter().skip(1).filter_map(|a| a.to_str());
        while let Some(arg) = rest.next() {
            match arg {
                "--dry-run" => args.dry_run = true,
                "--first-run" => args.first_run = true,
                "--config" => args.config = rest.next().map(PathBuf::from),
                _ => {
                    if let Some(path) = arg.strip_prefix("--config=") {
                        args.config = Some(PathBuf::from(path));
                    }
                }
            }
        }
        args
    }
}

/// How [`start`] went.
pub enum Start {
    /// Another process routes the links; this one is done.
    Exit,
    /// This process is the router. Boxed: it carries the settings and the
    /// channels, and `Exit` nothing.
    Resident(Box<Resident>),
}

/// The router and its IPC server, alive until the app quits.
pub struct Resident {
    router: Router,
    _server: Option<Server>,
    /// A second launch asked for the window.
    pub activate: async_channel::Receiver<()>,
    /// Every dispatch the router records, as it lands: what Activity and the
    /// tray follow. Memory only, like the router's own buffer.
    pub dispatched: async_channel::Receiver<DispatchRecord>,
    /// The command line carried links, so this launch is for routing, not
    /// for the window.
    pub launched_with_links: bool,
    /// `settings.json` as the router was started from: Settings edits it
    /// and applies the change to the router through its handle.
    pub settings: SettingsStore,
}

impl Resident {
    pub fn handle(&self) -> RouterHandle {
        self.router.handle()
    }
}

/// Becomes the router, or hands the links to the one already running.
pub fn start(args: &Args) -> Start {
    let links: Vec<Link> = args
        .links
        .iter()
        .map(|url| Link::new(url.as_str(), Source::CommandLine))
        .collect();

    // Under the E2E harness the router keeps to the harness's directory, so a
    // test run neither forwards to nor blocks the developer's own PwrRoute.
    let dir = env::var_os(crate::e2e::ROUTER_DIR_ENV)
        .map(PathBuf::from)
        .or_else(pwrroute_os::router_dir);
    let primary = match dir.map(|dir| instance::claim(&Endpoint::new(dir))) {
        Some(Ok(Claim::Primary(primary))) => Some(primary),
        Some(Ok(Claim::Running(client))) => {
            let request = if links.is_empty() {
                Request::Activate
            } else {
                Request::Open {
                    links: links.clone(),
                }
            };
            match client.send(&request, FORWARD_TIMEOUT) {
                Ok(response) if response.ok => return Start::Exit,
                Ok(response) => log::warn!(
                    "the running router refused the request: {}",
                    response.error.unwrap_or_default()
                ),
                Err(e) => log::warn!("the running router did not answer: {e}"),
            }
            // Never lose a link to a stuck router: route it here, then go.
            if !links.is_empty() {
                route_here_and_exit(args, links);
            }
            return Start::Exit;
        }
        Some(Err(e)) => {
            log::warn!("cannot take the router lock ({e}); running without it");
            None
        }
        None => {
            log::warn!("no data directory; running without the router lock");
            None
        }
    };

    let (dispatched_tx, dispatched) = async_channel::unbounded();
    let (router, settings) = spawn_router(args, Some(dispatched_tx));
    let handle = router.handle();
    let (activate_tx, activate) = async_channel::unbounded();
    let server = primary.and_then(|primary| {
        let handle = handle.clone();
        primary
            .serve(move |request| {
                handle.answer(request, || {
                    let _ = activate_tx.try_send(());
                })
            })
            .map_err(|e| log::warn!("cannot listen for other launches: {e}"))
            .ok()
    });
    LISTENING.store(server.is_some(), Ordering::Release);
    let launched_with_links = !links.is_empty();
    for link in links {
        let _ = handle.dispatch(link);
    }
    Start::Resident(Box::new(Resident {
        router,
        _server: server,
        activate,
        dispatched,
        launched_with_links,
        settings,
    }))
}

/// Links the OS handed over while the app runs: GPUI's open-URLs callback.
pub fn dispatch_os_links(handle: &RouterHandle, urls: Vec<String>) {
    // Read while AppKit is still handling the Apple Event that carried them.
    let opener = pwrroute_os::current_opener();
    let source = if cfg!(target_os = "macos") {
        Source::AppleEvent
    } else {
        Source::CommandLine
    };
    for url in urls {
        let _ = handle.dispatch(Link::new(url, source).with_opener(opener.clone()));
    }
}

/// Routes the links in this process without the lock or a window, waits for
/// each to finish, and returns.
fn route_here_and_exit(args: &Args, links: Vec<Link>) {
    let (router, _) = spawn_router(args, None);
    let handle = router.handle();
    for link in links {
        let _ = handle.dispatch_and_wait(link);
    }
}

/// What the router routes with. [`routing_source`] reads it from any thread;
/// the router writes it after every load and reload, so a config that
/// appears, changes or breaks while the app runs is reflected.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoutingSource {
    /// The Finicky config in charge, canonicalized, when one is. While
    /// `config_not_loaded` says the config is in no engine, the config that
    /// failed, so the window shows it with the error.
    pub config: Option<PathBuf>,
    /// `rules.json`, when the engine in charge holds a default browser or
    /// rules from it: combined with the config's own rules, or the whole
    /// config when there is none (or it has not loaded).
    pub rules: Option<PathBuf>,
    /// `config` names a config that has not loaded since it was chosen or
    /// appeared, and `error` says why. The router routes with `rules` alone
    /// until it loads, or with nothing when `fallback` is set too.
    pub config_not_loaded: bool,
    /// Why every link goes to the fallback browser, when nothing is in
    /// charge: no config (or one that has not loaded) and no (or an empty)
    /// `rules.json`, or nothing has ever loaded.
    pub fallback: Option<String>,
    /// Why the last load failed, when it did. Routing goes on with the last
    /// load that worked (or `rules.json` alone, while `config_not_loaded`),
    /// which `config` and `rules` describe.
    pub error: Option<String>,
}

/// A [`RoutingSource`] shared between the dispatch thread and the window.
#[derive(Debug, Clone, Default)]
struct SharedSource(Arc<Mutex<RoutingSource>>);

impl SharedSource {
    fn get(&self) -> RoutingSource {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set(&self, source: RoutingSource) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = source;
    }
}

/// The router's source, from the moment it is chosen.
static ROUTING: LazyLock<SharedSource> = LazyLock::new(SharedSource::default);

/// What the router routes with right now.
pub fn routing_source() -> RoutingSource {
    ROUTING.get()
}

/// A source the window chose (the importer's Choose source) for the
/// router to switch to on its next refresh.
static CHOSEN: LazyLock<Mutex<Option<ConfigChoice>>> = LazyLock::new(Mutex::default);

/// Switches the running router to `choice`: no config, one by path, or
/// discovery. The dispatch thread applies it on the refresh this asks for,
/// before the next link, and [`routing_source`] follows. The new source is
/// published here at once, so a window opened meanwhile already shows it.
pub fn choose_source(choice: ConfigChoice, router: Option<&RouterHandle>) {
    ROUTING.set(RoutingSource {
        config: chosen_config(&choice),
        ..ROUTING.get()
    });
    *CHOSEN.lock().unwrap_or_else(PoisonError::into_inner) = Some(choice);
    if let Some(router) = router {
        let _ = router.refresh();
    }
}

/// An engine over `sources` as the router routes with them, for Test a link
/// and the Rules screen's runs: a config the router has left out because it
/// has not loaded is left out here too, so `rules.json` runs alone, as it
/// does for links. Any other failure is the error.
pub fn engine_as_routed(
    sources: &Sources,
    host: Rc<dyn Host>,
    limits: Limits,
) -> anyhow::Result<Engine> {
    build_as_routed(sources, host, limits, |config| {
        let source = routing_source();
        source.config_not_loaded
            && source
                .config
                .as_deref()
                .is_some_and(|left_out| same_file(left_out, config))
    })
}

/// [`engine_as_routed`] with the router's answer to "is this config left
/// out?" passed in.
fn build_as_routed(
    sources: &Sources,
    host: Rc<dyn Host>,
    limits: Limits,
    left_out: impl Fn(&Path) -> bool,
) -> anyhow::Result<Engine> {
    let loaded = Engine::from_sources(sources, Rc::clone(&host), limits);
    if loaded.is_ok() || !sources.config.as_deref().is_some_and(left_out) {
        return loaded;
    }
    let alone = Sources {
        config: None,
        rules: sources.rules.clone(),
    };
    Engine::from_sources(&alone, host, limits).or(loaded)
}

/// The same file, whether or not either path is canonical.
fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || a.canonicalize()
            .ok()
            .zip(b.canonicalize().ok())
            .is_some_and(|(a, b)| a == b)
}

/// The Finicky config in charge of routing, if the router found one. The
/// window shows it read-only and tests links against it.
pub fn config_in_charge() -> Option<PathBuf> {
    routing_source().config
}

/// `dispatched`, when given, receives a copy of every record the router
/// keeps, for the window. It is wired before the router takes its first
/// link, so the window seeds Activity from this channel alone: nothing the
/// router routed before the window came up is missing from it, and nothing
/// is on it twice.
///
/// `settings.json` is read first: the retention and the journal it names
/// are the router's `Options` from the first link, and the store comes back
/// with the router so Settings edits the same values.
fn spawn_router(
    args: &Args,
    dispatched: Option<async_channel::Sender<DispatchRecord>>,
) -> (Router, SettingsStore) {
    let settings = SettingsStore::load(pwrroute_os::data_dir());
    let environment = Environment::current();
    let choice = match &args.config {
        Some(path) => ConfigChoice::Path(path.clone()),
        None => settings
            .settings()
            .source
            .choice(discovery::config_candidates(&environment)),
    };
    let rules_path = discovery::rules_path(&environment);
    let fallback = fallback_source(rules_path.clone());
    // The window may open before the engine has loaded: say now which
    // config it will load.
    let source = ROUTING.clone();
    source.set(RoutingSource {
        config: chosen_config(&choice),
        ..RoutingSource::default()
    });

    // The resolver is built on the dispatch thread, after the router exists;
    // it gets the router's handle from here to wake it when a file changes.
    let (handle_tx, handle_rx) = mpsc::channel::<RouterHandle>();
    let factory: ResolverFactory = Box::new(move || {
        // The OS answers the config's finicky.* calls: the modifier keys
        // held, the apps running. Its console output stays with the
        // engine's trace, which the router does not keep.
        let host = Rc::new(NativeHost::new());
        let mut resolver = LiveResolver::new(choice, rules_path, source.clone(), host)
            .inspect_err(|e| {
                // The router falls back with this reason; say so where the
                // window looks, next to the config it was going to load.
                source.set(RoutingSource {
                    fallback: Some(format!("{e:#}")),
                    ..source.get()
                });
            })?;
        if let Ok(handle) = handle_rx.recv() {
            resolver.watch(handle);
        }
        Ok(Box::new(resolver) as Box<dyn Resolver>)
    });
    let launcher: Box<dyn Launcher> = if args.dry_run {
        Box::new(DryRunLauncher::new())
    } else {
        Box::new(BrowserLauncher::new())
    };
    let slot = crate::handoff::HandleSlot::default();
    let watcher = slot.clone();
    let options = Options {
        retention: settings.retention(),
        journal: settings.journal(),
        fallback,
        on_dispatch: Some(Arc::new(move |record| {
            if let Some(dispatched) = &dispatched {
                // Unbounded: never blocks the dispatch thread. Nobody
                // listening (a router without a window) just drops it.
                let _ = dispatched.try_send(record.clone());
            }
        })),
        on_received: Some(Arc::new(move |id, received| {
            crate::handoff::watch(id, received, watcher.clone());
        })),
        diagnostic_log: crate::diag_log::path().map(Path::to_path_buf),
        network: None,
    };
    let router =
        Router::spawn(options, Some(factory), launcher).expect("start the dispatch thread");
    let _ = slot.set(router.handle());
    let _ = handle_tx.send(router.handle());
    (router, settings)
}

/// The router's fallback browser and profile: where a link goes when
/// nothing is in charge (a config broken at launch with no `rules.json` to
/// route with), or the browser a rule
/// chose cannot be opened. `rules.json`'s `defaultBrowser` and
/// `defaultProfile`, which first run writes from the user's choice, read
/// each time a link falls back, so a choice saved while the router runs
/// (first run, the Rules screen, an agent, an editor) applies to the next
/// link. The router uses the OS's own browser when the file names none or
/// names PwrRoute.
fn fallback_source(rules_path: Option<PathBuf>) -> FallbackSource {
    FallbackSource::new(move || fallback_from_rules(rules_path.as_deref()))
}

/// `rules.json`'s `defaultBrowser` and `defaultProfile`; `None` when there
/// is no file or it cannot be read.
fn fallback_from_rules(rules_path: Option<&Path>) -> Option<(String, String)> {
    let rules = RulesFile::load(rules_path?).ok()?;
    Some((rules.default_browser, rules.default_profile))
}

/// The config a choice names right now, before anything is loaded.
fn chosen_config(choice: &ConfigChoice) -> Option<PathBuf> {
    match choice {
        ConfigChoice::Path(path) => Some(path.canonicalize().unwrap_or_else(|_| path.clone())),
        ConfigChoice::Discover(candidates) => discovery::find_config(candidates),
        ConfigChoice::None => None,
    }
}

/// The router's resolver: a [`Reloader`] over the chosen config and
/// `rules.json`, caught up with the files before each link. A failed reload
/// keeps the last engine that worked, and a config that has not loaded yet
/// is left out so `rules.json` routes alone; the window learns of either
/// through [`RoutingSource::error`] and [`RoutingSource::config_not_loaded`]. The [`Host`] answers the config's `finicky.*`
/// calls: the OS in the app, a fixed answer in a test.
struct LiveResolver {
    reloader: RefCell<Reloader>,
    rules_path: Option<PathBuf>,
    source: SharedSource,
    /// Rechecks the files on its own thread while no link arrives.
    watcher: Option<Watcher>,
}

impl LiveResolver {
    fn new(
        choice: ConfigChoice,
        rules_path: Option<PathBuf>,
        source: SharedSource,
        host: Rc<dyn Host>,
    ) -> anyhow::Result<Self> {
        let reloader = Reloader::new(choice, rules_path.clone(), host, Limits::default())?;
        let resolver = Self {
            reloader: RefCell::new(reloader),
            rules_path,
            source,
            watcher: None,
        };
        resolver.publish(&resolver.reloader.borrow());
        Ok(resolver)
    }

    /// Starts rechecking the files on a timer. A settled change asks the
    /// router to refresh, so the status is current without waiting for a
    /// link.
    fn watch(&mut self, handle: RouterHandle) {
        let watch = self.reloader.borrow().watch_set();
        self.watcher = Watcher::spawn(watch, WATCH_INTERVAL, move || {
            let _ = handle.refresh();
        })
        .map_err(|e| log::warn!("cannot watch the config for changes: {e}"))
        .ok();
    }

    fn publish(&self, reloader: &Reloader) {
        let rules_in_charge = !reloader.engine().rules().is_empty();
        self.source.set(describe_source(
            reloader.status(),
            chosen_config(reloader.choice()),
            self.rules_path.as_deref(),
            rules_in_charge,
        ));
    }
}

impl Resolver for LiveResolver {
    fn resolve(
        &self,
        link: &pwrroute_engine::Link,
        opener: Option<&Opener>,
    ) -> anyhow::Result<Resolved> {
        // The trait's resolve, not the engine's own: it carries the trace's
        // rule steps.
        Resolver::resolve(self.reloader.borrow().engine(), link, opener)
    }

    fn short_links(&self) -> Option<ShortLinks> {
        // A clone, so the borrow ends here: the router expands the link
        // (a network request) after this returns, never inside it.
        Some(self.reloader.borrow().engine().short_links().clone())
    }

    fn refresh(&self) -> Result<(), String> {
        let mut reloader = self.reloader.borrow_mut();
        let chosen = CHOSEN.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(choice) = chosen {
            if let Err(e) = reloader.set_choice(choice) {
                log::warn!("the chosen source did not load: {e:#}");
            }
            self.publish(&reloader);
        }
        if let Some(result) = reloader.reload_if_changed() {
            if let Err(e) = result {
                log::warn!("the config did not reload: {e:#}");
            }
            self.publish(&reloader);
        }
        self.source.get().fallback.map_or(Ok(()), Err)
    }
}

/// The source as the reloader's status describes it. `named` is the config
/// the choice names right now: the status only knows the config an engine
/// was built from, so a config that has not loaded is taken from here.
/// `rules_in_charge` says the engine in charge holds a default browser or
/// rules from `rules.json` (after a failed reload, the files as they last
/// loaded, not as they are now).
fn describe_source(
    status: &Status,
    named: Option<PathBuf>,
    rules_path: Option<&Path>,
    rules_in_charge: bool,
) -> RoutingSource {
    let rules = rules_path
        .filter(|_| status.generation > 0 && rules_in_charge)
        .map(Path::to_path_buf);
    // A config that did not load (at start, or one that appeared broken
    // while rules.json was in charge) is in no engine, so the status does
    // not name it; the window still shows it, with the error. The reloader
    // left it out and routes with rules.json alone.
    let not_loaded = status
        .config
        .is_none()
        .then(|| status.error.as_ref().and(named))
        .flatten();
    let fallback = if status.generation == 0 {
        // Nothing has ever loaded, not even rules.json alone: the
        // reloader's stand-in engine is not in charge, so links fall back
        // with the load error.
        Some(
            status
                .error
                .clone()
                .unwrap_or_else(|| "no config has loaded".to_owned()),
        )
    } else if status.config.is_none() && rules.is_none() {
        Some(match rules_path {
            // The config's error says more than that rules.json is empty.
            _ if not_loaded.is_some() => status.error.clone().unwrap_or_default(),
            Some(path) if path.exists() => format!(
                "no Finicky config, and {} names no default browser and has no rules",
                path.display()
            ),
            Some(path) => format!("no Finicky config, and no rules.json at {}", path.display()),
            None => "no Finicky config, and no home directory for rules.json".to_owned(),
        })
    } else {
        None
    };
    RoutingSource {
        config_not_loaded: not_loaded.is_some(),
        config: status.config.clone().or(not_loaded),
        rules,
        fallback,
        error: status.error.clone(),
    }
}

/// Opens browsers with pwrroute-browsers. What reaches stderr and the trace
/// never carries the URL: [`LaunchError`]'s `Display` and every [`Note`] are
/// free of it, and `open`'s stderr, which may quote it, stays unread.
pub struct BrowserLauncher {
    inner: pwrroute_browsers::Launcher,
}

impl BrowserLauncher {
    /// A launcher for this machine.
    pub fn new() -> Self {
        let mut launcher = Self::with_env(Env::system());
        // A profile or private launch of a running Chromium browser then
        // hands the link over without the new instance taking focus.
        launcher.inner = launcher.inner.with_running(pwrroute_os::app_is_running);
        launcher
    }

    /// A launcher over `env` (a fixture machine, in tests). Finicky v4 has
    /// no `options.openInBackground`; a browser's own `openInBackground`
    /// rides on the engine's [`Browser`], so there is nothing to pass here.
    pub fn with_env(env: Env) -> Self {
        Self {
            inner: pwrroute_browsers::Launcher::with_env(env, LaunchOptions::default()),
        }
    }

    /// The command [`Launcher::launch`] would run for `browser`, without
    /// running it. `None` for `browser: null`.
    #[cfg(test)]
    fn plan(&self, browser: &Browser) -> anyhow::Result<Option<pwrroute_browsers::Plan>> {
        self.inner.plan(browser).map_err(launch_error)
    }
}

impl Launcher for BrowserLauncher {
    fn launch(&mut self, browser: &Browser) -> anyhow::Result<()> {
        if let Some(plan) = self.inner.launch(browser).map_err(launch_error)? {
            for note in &plan.notes {
                log::warn!("{}", describe_note(note));
            }
            // A hand-off's browser brings itself forward, which macOS 14
            // allows only once the active app gives way. When a link
            // activated PwrRoute, that is PwrRoute: give way, or the person
            // watches PwrRoute's window until macOS relents.
            if let Some(id) = &plan.hand_off_to {
                let started = std::time::Instant::now();
                let passed = pwrroute_os::pass_front_to(id);
                log::debug!(
                    "passed the front to {id}: {passed} ({:.1}ms)",
                    started.elapsed().as_secs_f64() * 1000.0
                );
            }
        }
        Ok(())
    }
}

/// Only the error's `Display`: `LaunchError::Failed` also carries `open`'s
/// stderr, which may quote the URL, and `anyhow`'s `Debug` would print it.
fn launch_error(error: LaunchError) -> anyhow::Error {
    anyhow::anyhow!("{error}")
}

/// A note as one line of stderr. Profile names, never URLs.
fn describe_note(note: &Note) -> String {
    match note {
        Note::ProfileNotFound { profile, available } if available.is_empty() => {
            format!("profile {profile:?} not found; the browser picks one")
        }
        Note::ProfileNotFound { profile, available } => format!(
            "profile {profile:?} not found; the browser has {}",
            available.join(", ")
        ),
        Note::ProfileUnsupported { profile } => format!(
            "profile {profile:?} ignored: this browser cannot select a profile from the command line"
        ),
        Note::BackgroundUnsupported => {
            "openInBackground is not possible on this platform; the browser comes to the front"
                .to_owned()
        }
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use pwrroute_browsers::Platform;
    use pwrroute_engine::{DefaultHost, ModifierKeys};
    use pwrroute_router::{AppType, FALLBACK_BROWSER, Outcome, Step};

    use super::*;

    const URL: &str = "https://secret.example.com/path?token=abc";

    fn parse(args: &[&str]) -> Args {
        Args::parse(args.iter().map(OsString::from).collect())
    }

    #[test]
    fn the_command_line_carries_links_and_flags() {
        let args = parse(&[
            "pwrroute",
            "--dry-run",
            "--config",
            "/tmp/finicky.ts",
            "https://example.com/a",
        ]);
        assert!(args.dry_run);
        assert_eq!(args.config.as_deref(), Some(Path::new("/tmp/finicky.ts")));
        assert_eq!(args.links, ["https://example.com/a"]);

        let args = parse(&["pwrroute", "--config=/c.js"]);
        assert_eq!(args.config.as_deref(), Some(Path::new("/c.js")));
        assert!(args.links.is_empty() && !args.dry_run && !args.first_run);

        assert!(parse(&["pwrroute", "--first-run"]).first_run);
    }

    fn browser(name: &str, app_type: AppType) -> Browser {
        Browser {
            name: name.into(),
            app_type,
            url: URL.into(),
            profile: String::new(),
            args: Vec::new(),
            open_in_background: None,
            private: false,
            privacy: pwrroute_engine::Privacy::Full,
        }
    }

    #[test]
    fn the_e2e_environment_stands_in_for_flags_only_under_the_hook() {
        fn env<'a>(vars: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
            move |key| {
                vars.iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| OsString::from(v))
            }
        }
        let flags = [
            ("PWRROUTE_E2E_DRY_RUN", "1"),
            ("PWRROUTE_E2E_CONFIG", "/e2e/routing.js"),
        ];
        let args = parse(&["pwrroute", "https://example.com/a"]).with_e2e_env(env(&flags));
        assert!(!args.dry_run && args.config.is_none(), "no hook, no effect");

        let hooked = [
            ("PWRROUTE_E2E_SOCKET", "/tmp/e2e.sock"),
            ("PWRROUTE_E2E_DRY_RUN", "1"),
            ("PWRROUTE_E2E_CONFIG", "/e2e/routing.js"),
        ];
        let args = parse(&["pwrroute", "https://example.com/a"]).with_e2e_env(env(&hooked));
        assert!(args.dry_run);
        assert_eq!(args.config.as_deref(), Some(Path::new("/e2e/routing.js")));
        assert_eq!(args.links, ["https://example.com/a"]);
    }

    #[test]
    fn the_launcher_plans_the_browsers_crate_command_for_the_engines_browser() {
        let dir = tempfile::tempdir().unwrap();
        let launcher = BrowserLauncher::with_env(Env::fixture(Platform::MacOs, dir.path()));

        let plan = launcher
            .plan(&browser("Google Chrome", AppType::AppName))
            .unwrap()
            .expect("a plan");
        assert_eq!(plan.program, PathBuf::from("/usr/bin/open"));
        assert_eq!(plan.args, ["-a", "Google Chrome", URL]);
        assert!(plan.notes.is_empty());

        let mut background = browser("com.apple.Safari", AppType::BundleId);
        background.open_in_background = Some(true);
        let plan = launcher.plan(&background).unwrap().expect("a plan");
        assert_eq!(plan.args, ["-b", "com.apple.Safari", "-g", URL]);

        assert!(
            launcher
                .plan(&browser("", AppType::None))
                .unwrap()
                .is_none()
        );

        // A profile the browser cannot select is noted, URL-free.
        let mut with_profile = browser("Safari", AppType::AppName);
        with_profile.profile = "Work".into();
        let plan = launcher.plan(&with_profile).unwrap().expect("a plan");
        let notes: Vec<String> = plan.notes.iter().map(describe_note).collect();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("\"Work\""), "{notes:?}");
        assert!(!notes[0].contains("secret"), "{notes:?}");
    }

    #[test]
    fn launch_errors_name_the_browser_and_never_the_url() {
        let dir = tempfile::tempdir().unwrap();
        // A fixture Linux machine with no browsers at all.
        let launcher = BrowserLauncher::with_env(Env::fixture(Platform::Linux, dir.path()));
        let err = launcher
            .plan(&browser("Nope Browser", AppType::AppName))
            .unwrap_err();
        assert_eq!(
            format!("{err:#}"),
            "browser \"Nope Browser\" is not installed"
        );

        // `open`'s stderr quotes the URL; the error that reaches the trace
        // and stderr does not.
        let failed = LaunchError::Failed {
            program: PathBuf::from("/usr/bin/open"),
            status: exit_status(3),
            stderr: format!("open: cannot open {URL}"),
        };
        let err = launch_error(failed);
        let message = format!("{err:#}");
        assert!(message.starts_with("/usr/bin/open failed"), "{message}");
        assert!(!message.contains("secret"), "{message}");
        assert!(!format!("{err:?}").contains("secret"));
    }

    #[cfg(unix)]
    fn exit_status(code: i32) -> std::process::ExitStatus {
        std::os::unix::process::ExitStatusExt::from_raw(code << 8)
    }

    #[cfg(windows)]
    fn exit_status(code: i32) -> std::process::ExitStatus {
        std::os::windows::process::ExitStatusExt::from_raw(code as u32)
    }

    /// A contrived home directory: the config candidates and `rules.json`
    /// of a user, never the developer's own.
    struct Home {
        dir: tempfile::TempDir,
    }

    impl Home {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
            }
        }

        fn path(&self, name: &str) -> PathBuf {
            self.dir.path().join(name)
        }

        /// Writes `name` the way [`RulesFile::save`] does: to a sibling file
        /// renamed into place. The router reads `rules.json` on its own
        /// thread, and a plain write truncates the file first, so it could
        /// read an empty one and keep that parse error.
        fn write(&self, name: &str, text: &str) -> PathBuf {
            let path = self.path(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut staging = path.as_os_str().to_owned();
            staging.push(".saving");
            fs::write(&staging, text).unwrap();
            fs::rename(&staging, &path).unwrap();
            path
        }

        fn choice(&self) -> ConfigChoice {
            ConfigChoice::Discover(discovery::config_candidates(&Environment {
                home: Some(self.dir.path().to_path_buf()),
                ..Environment::default()
            }))
        }

        fn rules_path(&self) -> PathBuf {
            self.path("PwrRoute/rules.json")
        }

        fn resolver(&self) -> LiveResolver {
            self.resolver_with(Rc::new(DefaultHost))
        }

        fn resolver_with(&self, host: Rc<dyn Host>) -> LiveResolver {
            LiveResolver::new(
                self.choice(),
                Some(self.rules_path()),
                SharedSource::default(),
                host,
            )
            .unwrap()
        }
    }

    /// A host with a fixed answer to `finicky.getModifierKeys()`.
    struct KeysHost {
        option: bool,
    }

    impl Host for KeysHost {
        fn modifier_keys(&self) -> ModifierKeys {
            ModifierKeys {
                option: self.option,
                ..ModifierKeys::default()
            }
        }
    }

    const RULES: &str = r#"{
      "defaultBrowser": "Firefox",
      "rules": [{ "match": "docs.example.com/*", "browser": "Arc" }]
    }"#;

    fn resolve(resolver: &LiveResolver, url: &str) -> String {
        resolver
            .resolve(&pwrroute_engine::Link::new(url), None)
            .unwrap()
            .browser
            .name
    }

    #[test]
    fn rules_json_alone_routes_links_and_is_the_source() {
        let home = Home::new();
        let rules = home.write("PwrRoute/rules.json", RULES);
        let resolver = home.resolver();
        assert_eq!(resolver.refresh(), Ok(()));
        assert_eq!(resolve(&resolver, "https://docs.example.com/x"), "Arc");
        assert_eq!(resolve(&resolver, "https://example.com/"), "Firefox");
        assert_eq!(
            resolver.source.get(),
            RoutingSource {
                config: None,
                rules: Some(rules),
                config_not_loaded: false,
                fallback: None,
                error: None,
            }
        );
    }

    #[test]
    fn a_config_sees_the_hosts_modifier_keys() {
        // Finicky's idiom: hold Option to open the link elsewhere.
        let home = Home::new();
        home.write(
            ".finicky.js",
            r#"export default {
              defaultBrowser: "Safari",
              handlers: [{ match: () => finicky.getModifierKeys().option, browser: "Firefox" }],
            };"#,
        );
        let held = home.resolver_with(Rc::new(KeysHost { option: true }));
        assert_eq!(held.refresh(), Ok(()));
        assert_eq!(resolve(&held, "https://example.com/"), "Firefox");

        let plain = home.resolver_with(Rc::new(KeysHost { option: false }));
        assert_eq!(plain.refresh(), Ok(()));
        assert_eq!(resolve(&plain, "https://example.com/"), "Safari");
    }

    #[test]
    fn a_missing_or_empty_rules_json_is_the_fallback_with_a_reason() {
        let home = Home::new();
        let resolver = home.resolver();
        let reason = resolver.refresh().unwrap_err();
        assert!(
            reason.starts_with("no Finicky config, and no rules.json at"),
            "{reason}"
        );
        assert!(reason.contains("rules.json"), "{reason}");
        assert_eq!(
            resolver.source.get().fallback.as_deref(),
            Some(reason.as_str())
        );

        home.write("PwrRoute/rules.json", r#"{ "rules": [] }"#);
        let reason = resolver.refresh().unwrap_err();
        assert!(
            reason.contains("names no default browser and has no rules"),
            "{reason}"
        );
        assert_eq!(resolver.source.get().rules, None);
    }

    #[test]
    fn a_rules_json_edit_takes_effect_on_the_next_link() {
        let home = Home::new();
        home.write("PwrRoute/rules.json", RULES);
        let resolver = home.resolver();
        resolver.refresh().unwrap();
        assert_eq!(resolve(&resolver, "https://docs.example.com/x"), "Arc");

        home.write(
            "PwrRoute/rules.json",
            r#"{ "defaultBrowser": "Firefox", "rules": [{ "match": "docs.example.com/*", "browser": "Orion" }] }"#,
        );
        resolver.refresh().unwrap();
        assert_eq!(resolve(&resolver, "https://docs.example.com/x"), "Orion");

        // A broken edit keeps the last working rules and says why.
        home.write("PwrRoute/rules.json", "{ not json");
        resolver.refresh().unwrap();
        assert_eq!(resolve(&resolver, "https://docs.example.com/x"), "Orion");
        let source = resolver.source.get();
        assert!(source.error.is_some(), "{source:?}");
        assert_eq!(source.fallback, None);
    }

    /// The switch's write reaches the router on the next link: rules.json's
    /// `precedence` puts its rule before the config's handler and its
    /// default in place of the config's, and taking it out puts the Go
    /// order back.
    #[test]
    fn a_precedence_flip_takes_effect_on_the_next_link() {
        let home = Home::new();
        home.write(
            ".finicky.js",
            r#"export default {
              defaultBrowser: "Safari",
              handlers: [{ match: "https://both.example.com/*", browser: "Arc" }],
            };"#,
        );
        let rules = |precedence: &str| {
            format!(
                r#"{{ {precedence} "defaultBrowser": "Firefox",
                  "rules": [{{ "match": "https://both.example.com/*", "browser": "Orion" }}] }}"#
            )
        };
        home.write("PwrRoute/rules.json", &rules(""));
        let resolver = home.resolver();
        let routes = |resolver: &LiveResolver| {
            resolver.refresh().unwrap();
            (
                resolve(resolver, "https://both.example.com/a"),
                resolve(resolver, "https://unmatched.example.com/"),
            )
        };
        assert_eq!(routes(&resolver), ("Arc".into(), "Safari".into()));
        home.write(
            "PwrRoute/rules.json",
            &rules(r#""precedence": "pwrroute","#),
        );
        assert_eq!(routes(&resolver), ("Orion".into(), "Firefox".into()));
        home.write("PwrRoute/rules.json", &rules(""));
        assert_eq!(routes(&resolver), ("Arc".into(), "Safari".into()));
    }

    #[test]
    fn a_config_that_appears_later_takes_charge() {
        let home = Home::new();
        home.write("PwrRoute/rules.json", RULES);
        let resolver = home.resolver();
        resolver.refresh().unwrap();
        assert_eq!(resolver.source.get().config, None);
        assert_eq!(resolve(&resolver, "https://example.com/"), "Firefox");

        let config = home.write(
            ".finicky.js",
            "export default { defaultBrowser: \"Arc\" };\n",
        );
        resolver.refresh().unwrap();
        assert_eq!(resolve(&resolver, "https://example.com/"), "Arc");
        // rules.json's handlers follow the config's.
        assert_eq!(resolve(&resolver, "https://docs.example.com/x"), "Arc");
        let source = resolver.source.get();
        assert_eq!(source.config, Some(config.canonicalize().unwrap()));
        assert_eq!(source.rules, Some(home.rules_path()));
        assert_eq!(source.error, None);
    }

    /// A config that never loaded is left out: rules.json's rules and
    /// default route alone, and the window is told which config failed.
    #[test]
    fn a_config_that_never_loaded_leaves_rules_json_routing() {
        let home = Home::new();
        let rules = home.write("PwrRoute/rules.json", RULES);
        let config = home.write(".finicky.js", "export default {\n");
        let resolver = home.resolver();

        assert_eq!(resolver.refresh(), Ok(()));
        assert_eq!(resolve(&resolver, "https://docs.example.com/x"), "Arc");
        assert_eq!(resolve(&resolver, "https://example.com/"), "Firefox");
        let source = resolver.source.get();
        assert_eq!(source.config, Some(config.canonicalize().unwrap()));
        assert_eq!(source.rules, Some(rules));
        assert!(source.config_not_loaded, "{source:?}");
        assert_eq!(source.fallback, None);
        assert!(
            source
                .error
                .as_deref()
                .is_some_and(|e| e.contains(".finicky.js")),
            "{source:?}"
        );

        // An edit to rules.json applies while the config is still broken.
        home.write(
            "PwrRoute/rules.json",
            r#"{ "defaultBrowser": "Firefox", "rules": [{ "match": "docs.example.com/*", "browser": "Orion" }] }"#,
        );
        assert_eq!(resolver.refresh(), Ok(()));
        assert_eq!(resolve(&resolver, "https://docs.example.com/x"), "Orion");
        assert!(resolver.source.get().config_not_loaded);
    }

    /// With no rules.json to route with, a config that never loaded leaves
    /// nothing in charge: links fall back, with the config's error.
    #[test]
    fn a_config_that_never_loaded_without_rules_json_is_the_fallback() {
        let home = Home::new();
        let config = home.write(".finicky.js", "export default {\n");
        let resolver = home.resolver();

        let reason = resolver.refresh().unwrap_err();
        assert!(reason.contains(".finicky.js"), "{reason}");
        let source = resolver.source.get();
        assert_eq!(source.config, Some(config.canonicalize().unwrap()));
        assert!(source.config_not_loaded, "{source:?}");
        assert_eq!(source.fallback.as_deref(), Some(reason.as_str()));
        assert_eq!(source.error.as_deref(), Some(reason.as_str()));
        assert_eq!(source.rules, None);

        // An empty rules.json is no better: still the config's reason.
        home.write("PwrRoute/rules.json", r#"{ "rules": [] }"#);
        assert_eq!(resolver.refresh().unwrap_err(), reason);
    }

    /// Once the config loads it takes charge, with rules.json after it.
    #[test]
    fn a_config_that_never_loaded_takes_charge_once_it_loads() {
        let home = Home::new();
        let rules = home.write("PwrRoute/rules.json", RULES);
        let config = home.write(".finicky.js", "export default {\n");
        let resolver = home.resolver();
        resolver.refresh().unwrap();
        assert_eq!(resolve(&resolver, "https://example.com/"), "Firefox");

        home.write(
            ".finicky.js",
            "export default { defaultBrowser: \"Safari\" };\n",
        );
        assert_eq!(resolver.refresh(), Ok(()));
        assert_eq!(
            resolver.source.get(),
            RoutingSource {
                config: Some(config.canonicalize().unwrap()),
                rules: Some(rules),
                config_not_loaded: false,
                fallback: None,
                error: None,
            }
        );
        // The config's default takes what its handlers and the rules leave.
        assert_eq!(resolve(&resolver, "https://example.com/"), "Safari");
        assert_eq!(resolve(&resolver, "https://docs.example.com/x"), "Arc");
    }

    #[test]
    fn a_config_that_appears_broken_is_named_while_rules_json_stays_in_charge() {
        let home = Home::new();
        let rules = home.write("PwrRoute/rules.json", RULES);
        let resolver = home.resolver();
        resolver.refresh().unwrap();
        assert_eq!(resolver.source.get().config, None);

        let config = home.write(".finicky.js", "export default {\n");
        // rules.json goes on routing; the window learns of the config and
        // why it did not load.
        resolver.refresh().unwrap();
        assert_eq!(resolve(&resolver, "https://example.com/"), "Firefox");
        let source = resolver.source.get();
        assert_eq!(source.config, Some(config.canonicalize().unwrap()));
        assert_eq!(source.rules, Some(rules));
        assert!(source.config_not_loaded, "{source:?}");
        assert_eq!(source.fallback, None);
        assert!(
            source
                .error
                .as_deref()
                .is_some_and(|e| e.contains(".finicky.js")),
            "{source:?}"
        );
    }

    /// A failed reload of a config that did load is not left out: its last
    /// load goes on routing, rules.json with it.
    /// Test a link and the test runs build as the router routes: a config
    /// it has left out runs rules.json alone; one it has not is the error.
    #[test]
    fn engines_for_the_window_leave_out_the_config_the_router_left_out() {
        let home = Home::new();
        let config = home.write(".finicky.js", "export default {\n");
        let sources = Sources {
            config: Some(config.clone()),
            rules: serde_json::from_str(RULES).unwrap(),
        };
        let build = |left_out: bool| {
            build_as_routed(&sources, Rc::new(DefaultHost), Limits::default(), |path| {
                assert!(same_file(path, &config));
                left_out
            })
        };
        let engine = build(true).expect("rules.json alone");
        assert_eq!(engine.config_path(), None);
        let resolved = engine
            .resolve("https://docs.example.com/x", None)
            .unwrap()
            .browser
            .name;
        assert_eq!(resolved, "Arc");
        let error = build(false).expect_err("the config's error");
        assert!(format!("{error:#}").contains(".finicky.js"), "{error:#}");
    }

    #[test]
    fn a_config_that_loaded_once_keeps_routing_when_a_reload_fails() {
        let home = Home::new();
        home.write("PwrRoute/rules.json", RULES);
        home.write(
            ".finicky.js",
            "export default { defaultBrowser: \"Safari\" };\n",
        );
        let resolver = home.resolver();
        resolver.refresh().unwrap();

        home.write(".finicky.js", "export default {\n");
        assert_eq!(resolver.refresh(), Ok(()));
        assert_eq!(resolve(&resolver, "https://example.com/"), "Safari");
        let source = resolver.source.get();
        assert!(!source.config_not_loaded, "{source:?}");
        assert!(source.error.is_some(), "{source:?}");
        assert_eq!(source.fallback, None);
    }

    #[test]
    fn the_fallback_is_rules_json_s_default_browser_or_the_os_s_own() {
        let home = Home::new();
        let source = fallback_source(Some(home.rules_path()));
        let os_browser = (FALLBACK_BROWSER.to_owned(), String::new());
        // No path, no file, an empty one, a broken one, and one naming
        // PwrRoute.
        assert_eq!(fallback_source(None).get(), os_browser);
        assert_eq!(source.get(), os_browser);
        home.write("PwrRoute/rules.json", r#"{ "rules": [] }"#);
        assert_eq!(source.get(), os_browser);
        home.write("PwrRoute/rules.json", "{ not json");
        assert_eq!(fallback_from_rules(Some(&home.rules_path())), None);
        assert_eq!(source.get(), os_browser);
        home.write(
            "PwrRoute/rules.json",
            r#"{ "defaultBrowser": "com.pwrdrvr.pwrroute" }"#,
        );
        assert_eq!(source.get(), os_browser);

        // First run's choice: a bundle id, with a profile when one is set,
        // read again on every ask.
        home.write(
            "PwrRoute/rules.json",
            r#"{ "defaultBrowser": "com.google.Chrome", "defaultProfile": "Work" }"#,
        );
        assert_eq!(
            source.get(),
            ("com.google.Chrome".to_owned(), "Work".to_owned())
        );
    }

    /// Fails every browser named in `missing`, as the real launcher does
    /// for an app that is not installed, and records every attempt.
    #[derive(Clone)]
    struct Missing {
        missing: Vec<&'static str>,
        attempts: Arc<std::sync::Mutex<Vec<Browser>>>,
    }

    impl Missing {
        fn new(missing: &[&'static str]) -> Self {
            Self {
                missing: missing.to_vec(),
                attempts: Arc::default(),
            }
        }

        fn attempts(&self) -> Vec<Browser> {
            self.attempts.lock().unwrap().clone()
        }
    }

    impl Launcher for Missing {
        fn launch(&mut self, browser: &Browser) -> anyhow::Result<()> {
            self.attempts.lock().unwrap().push(browser.clone());
            if self.missing.contains(&browser.name.as_str()) {
                anyhow::bail!("browser {:?} is not installed", browser.name)
            }
            Ok(())
        }
    }

    #[test]
    fn a_choice_saved_after_the_router_starts_is_the_next_fallback() {
        // The router starts before first run, as `main` does it: no
        // rules.json yet, and a config whose browser is not installed (Brave:
        // never the OS's own browser, which the fallback would be).
        let home = Home::new();
        home.write(
            ".finicky.js",
            "export default {\n  defaultBrowser: \"Brave\",\n  handlers: [{ match: () => true, \
             browser: \"Brave\" }],\n};\n",
        );
        let source = SharedSource::default();
        let factory: ResolverFactory = {
            let (choice, rules, source) = (home.choice(), home.rules_path(), source.clone());
            Box::new(move || {
                Ok(Box::new(LiveResolver::new(
                    choice,
                    Some(rules),
                    source,
                    Rc::new(DefaultHost),
                )?) as Box<dyn Resolver>)
            })
        };
        let launcher = Missing::new(&["Brave"]);
        let router = Router::spawn(
            Options {
                fallback: fallback_source(Some(home.rules_path())),
                ..Options::default()
            },
            Some(factory),
            launcher.clone(),
        )
        .unwrap();
        let handle = router.handle();
        let dispatch = || {
            handle
                .dispatch_and_wait(Link::new("https://example.com/x", Source::CommandLine))
                .unwrap()
        };

        let record = dispatch();
        assert_eq!(record.outcome, Outcome::Opened, "{:?}", record.steps);
        assert_eq!(record.browser.unwrap().name, FALLBACK_BROWSER);

        // First run saves the user's choice while the router runs.
        home.write(
            "PwrRoute/rules.json",
            r#"{ "defaultBrowser": "com.google.Chrome", "defaultProfile": "Work" }"#,
        );
        let record = dispatch();
        assert_eq!(record.outcome, Outcome::Opened, "{:?}", record.steps);
        assert!(record.steps.contains(&Step::Fallback {
            reason: "Brave could not be opened".into()
        }));
        let opened = record.browser.unwrap();
        assert_eq!(
            (
                opened.name.as_str(),
                opened.app_type,
                opened.profile.as_str()
            ),
            ("com.google.Chrome", AppType::BundleId, "Work")
        );
        let attempts: Vec<String> = launcher.attempts().into_iter().map(|b| b.name).collect();
        assert_eq!(
            attempts,
            ["Brave", FALLBACK_BROWSER, "Brave", "com.google.Chrome"]
        );
    }

    /// Through the router: a config broken at launch with no rules.json
    /// falls back to the OS's browser; once first run writes a choice, that
    /// rules.json routes alone, the config still left out.
    #[test]
    fn a_config_broken_at_launch_routes_with_the_rules_json_first_run_writes() {
        let home = Home::new();
        home.write(".finicky.js", "export default {\n");
        let source = SharedSource::default();
        let factory: ResolverFactory = {
            let (choice, rules, source) = (home.choice(), home.rules_path(), source.clone());
            Box::new(move || {
                Ok(Box::new(LiveResolver::new(
                    choice,
                    Some(rules),
                    source,
                    Rc::new(DefaultHost),
                )?) as Box<dyn Resolver>)
            })
        };
        let launcher = DryRunLauncher::new();
        let router = Router::spawn(
            Options {
                fallback: fallback_source(Some(home.rules_path())),
                ..Options::default()
            },
            Some(factory),
            launcher.clone(),
        )
        .unwrap();
        let handle = router.handle();
        let dispatch = || {
            handle
                .dispatch_and_wait(Link::new("https://example.com/x", Source::CommandLine))
                .unwrap()
        };

        // Nothing to route with: the link falls back with the config's error.
        let record = dispatch();
        let reason = source.get().fallback.expect("nothing in charge");
        assert!(reason.contains(".finicky.js"), "{reason}");
        assert!(record.steps.contains(&Step::Fallback { reason }));
        assert_eq!(launcher.launched()[0].name, FALLBACK_BROWSER);

        // Written after the router started, as first run does.
        home.write(
            "PwrRoute/rules.json",
            r#"{ "defaultBrowser": "com.google.Chrome", "defaultProfile": "Work",
                 "rules": [{ "match": "https://docs.example.com/*", "browser": "Firefox" }] }"#,
        );
        let record = dispatch();
        assert_eq!(record.outcome, Outcome::DryRun);
        assert!(
            !record
                .steps
                .iter()
                .any(|step| matches!(step, Step::Fallback { .. })),
            "{:?}",
            record.steps
        );
        // PwrRoute's default, typed as a bundle id so the launcher can open
        // it, with its profile.
        let launched = &launcher.launched()[1];
        assert_eq!(launched.name, "com.google.Chrome");
        assert_eq!(launched.app_type, AppType::BundleId);
        assert_eq!(launched.profile, "Work");
        let record = handle
            .dispatch_and_wait(Link::new("https://docs.example.com/a", Source::CommandLine))
            .unwrap();
        assert_eq!(record.browser.unwrap().name, "Firefox");
        assert_eq!(handle.status().unwrap().config_error, None);
        assert!(source.get().config_not_loaded);
    }

    #[test]
    fn through_the_router_a_file_change_is_seen_without_a_restart() {
        let home = Home::new();
        let source = SharedSource::default();
        let factory: ResolverFactory = {
            let (choice, rules, source) = (home.choice(), home.rules_path(), source.clone());
            Box::new(move || {
                let host = Rc::new(DefaultHost);
                Ok(
                    Box::new(LiveResolver::new(choice, Some(rules), source, host)?)
                        as Box<dyn Resolver>,
                )
            })
        };
        let launcher = DryRunLauncher::new();
        let router = Router::spawn(Options::default(), Some(factory), launcher.clone()).unwrap();
        let handle = router.handle();

        let record = handle
            .dispatch_and_wait(Link::new("https://docs.example.com/x", Source::CommandLine))
            .unwrap();
        let reason = source.get().fallback.expect("nothing in charge");
        assert!(record.steps.contains(&Step::Fallback {
            reason: reason.clone()
        }));
        assert_eq!(launcher.launched()[0].name, FALLBACK_BROWSER);
        assert_eq!(handle.status().unwrap().config_error, Some(reason));

        home.write("PwrRoute/rules.json", RULES);
        let record = handle
            .dispatch_and_wait(Link::new("https://docs.example.com/x", Source::CommandLine))
            .unwrap();
        assert_eq!(record.outcome, Outcome::DryRun);
        assert_eq!(record.browser.unwrap().name, "Arc");
        assert_eq!(handle.status().unwrap().config_error, None);
        assert_eq!(source.get().rules, Some(home.rules_path()));
    }

    #[test]
    fn the_watcher_refreshes_the_router_without_a_link() {
        let home = Home::new();
        let source = SharedSource::default();
        let (handle_tx, handle_rx) = mpsc::channel::<RouterHandle>();
        let factory: ResolverFactory = {
            let (choice, rules, source) = (home.choice(), home.rules_path(), source.clone());
            Box::new(move || {
                let mut resolver =
                    LiveResolver::new(choice, Some(rules), source, Rc::new(DefaultHost))?;
                if let Ok(handle) = handle_rx.recv() {
                    resolver.watch(handle);
                }
                Ok(Box::new(resolver) as Box<dyn Resolver>)
            })
        };
        let router =
            Router::spawn(Options::default(), Some(factory), DryRunLauncher::new()).unwrap();
        handle_tx.send(router.handle()).unwrap();
        assert!(router.handle().status().unwrap().config_error.is_some());

        home.write("PwrRoute/rules.json", RULES);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while source.get().rules.is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "the watcher never reloaded"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        drop(router);
    }
}
