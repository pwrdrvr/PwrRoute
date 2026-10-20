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
    sync::{Arc, LazyLock, Mutex, PoisonError, mpsc},
    time::Duration,
};

use pwrroute_browsers::{Env, LaunchError, LaunchOptions, Note};
use pwrroute_engine::{
    ConfigChoice, DefaultHost, Limits, Reloader, RulesFile, Status, Watcher,
    discovery::{self, Environment},
};
use pwrroute_router::{
    Browser, DispatchRecord, DryRunLauncher, Launcher, Link, Opener, Options, Resolved, Resolver,
    ResolverFactory, Router, RouterHandle, Source,
    instance::{self, Claim, Endpoint, Server},
    ipc::Request,
};

/// How long a second launch waits for the running router to answer.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(3);

/// How often the config, its imports and `rules.json` are rechecked for a
/// change while no link arrives (the Go host's interval). A link always
/// rechecks them first, whatever the timer last saw.
const WATCH_INTERVAL: Duration = Duration::from_millis(500);

/// A dispatch slower than this is reported on stderr, as the Go router did.
const SLOW_DISPATCH_US: u64 = 500_000;

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
        Self::parse(env::args_os().collect())
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
    /// This process is the router.
    Resident(Resident),
}

/// The router and its IPC server, alive until the app quits.
pub struct Resident {
    router: Router,
    _server: Option<Server>,
    /// A second launch asked for the window.
    pub activate: async_channel::Receiver<()>,
    /// The command line carried links, so this launch is for routing, not
    /// for the window.
    pub launched_with_links: bool,
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

    let primary = match pwrroute_os::router_dir().map(|dir| instance::claim(&Endpoint::new(dir))) {
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
                Ok(response) => eprintln!(
                    "pwrroute: the running router refused the request: {}",
                    response.error.unwrap_or_default()
                ),
                Err(e) => eprintln!("pwrroute: the running router did not answer: {e}"),
            }
            // Never lose a link to a stuck router: route it here, then go.
            if !links.is_empty() {
                route_here_and_exit(args, links);
            }
            return Start::Exit;
        }
        Some(Err(e)) => {
            eprintln!("pwrroute: cannot take the router lock ({e}); running without it");
            None
        }
        None => {
            eprintln!("pwrroute: no data directory; running without the router lock");
            None
        }
    };

    let router = spawn_router(args);
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
            .map_err(|e| eprintln!("pwrroute: cannot listen for other launches: {e}"))
            .ok()
    });
    let launched_with_links = !links.is_empty();
    for link in links {
        let _ = handle.dispatch(link);
    }
    Start::Resident(Resident {
        router,
        _server: server,
        activate,
        launched_with_links,
    })
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
    let router = spawn_router(args);
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
    /// `error` says a config did not load and none is in charge, the config
    /// that failed, so the window shows it with the error.
    pub config: Option<PathBuf>,
    /// `rules.json`, when it names a default browser or has rules: appended
    /// after the config's own rules, or the whole config when there is none.
    pub rules: Option<PathBuf>,
    /// Why every link goes to the fallback browser, when nothing is in
    /// charge: no config and no (or an empty) `rules.json`, or nothing has
    /// ever loaded.
    pub fallback: Option<String>,
    /// Why the last reload failed, when it did. Routing goes on with the
    /// last load that worked, which `config` and `rules` describe.
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

/// The Finicky config in charge of routing, if the router found one. The
/// window shows it read-only and tests links against it.
pub fn config_in_charge() -> Option<PathBuf> {
    routing_source().config
}

fn spawn_router(args: &Args) -> Router {
    let environment = Environment::current();
    let choice = match &args.config {
        Some(path) => ConfigChoice::Path(path.clone()),
        None => ConfigChoice::Discover(discovery::config_candidates(&environment)),
    };
    let rules_path = discovery::rules_path(&environment);
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
        let mut resolver =
            LiveResolver::new(choice, rules_path, source.clone()).inspect_err(|e| {
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
    let report_all = env::var_os("PWRROUTE_DIAGNOSTICS").is_some_and(|v| v == "1");
    let options = Options {
        on_dispatch: Some(Arc::new(move |record| report_timing(record, report_all))),
        ..Options::default()
    };
    let router =
        Router::spawn(options, Some(factory), launcher).expect("start the dispatch thread");
    let _ = handle_tx.send(router.handle());
    router
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
/// keeps the last engine that worked; the window learns of it through
/// [`RoutingSource::error`].
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
    ) -> anyhow::Result<Self> {
        let reloader = Reloader::new(
            choice,
            rules_path.clone(),
            Rc::new(DefaultHost),
            Limits::default(),
        )?;
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
        .map_err(|e| eprintln!("pwrroute: cannot watch the config for changes: {e}"))
        .ok();
    }

    fn publish(&self, reloader: &Reloader) {
        let last = self.source.get();
        self.source.set(describe_source(
            reloader.status(),
            chosen_config(reloader.choice()),
            self.rules_path.as_deref(),
            &last,
        ));
    }
}

impl Resolver for LiveResolver {
    fn resolve(&self, url: &str, opener: Option<&Opener>) -> anyhow::Result<Resolved> {
        // The trait's resolve, not the engine's own: it carries the trace's
        // rule steps.
        Resolver::resolve(self.reloader.borrow().engine(), url, opener)
    }

    fn refresh(&self) -> Result<(), String> {
        let mut reloader = self.reloader.borrow_mut();
        if let Some(result) = reloader.reload_if_changed() {
            if let Err(e) = result {
                eprintln!("pwrroute: the config did not reload: {e:#}");
            }
            self.publish(&reloader);
        }
        self.source.get().fallback.map_or(Ok(()), Err)
    }
}

/// The source as the reloader's status and `rules.json` describe it.
/// `named` is the config the choice names right now: the status only knows
/// the config an engine was built from, so a config that never loaded is
/// taken from here. `last` is the source as last published: after a failed
/// reload the engine in charge is still the one `last` describes.
fn describe_source(
    status: &Status,
    named: Option<PathBuf>,
    rules_path: Option<&Path>,
    last: &RoutingSource,
) -> RoutingSource {
    let rules = if status.error.is_none() {
        // The engine was built from the files as they are: say whether
        // rules.json had anything in it.
        rules_path
            .filter(|path| RulesFile::load(path).is_ok_and(|file| !file.is_empty()))
            .map(Path::to_path_buf)
    } else {
        last.rules.clone()
    };
    let fallback = if status.generation == 0 {
        // Nothing has ever loaded: the reloader's stand-in engine is not in
        // charge, so links fall back with the load error, as before.
        Some(
            status
                .error
                .clone()
                .unwrap_or_else(|| "no config has loaded".to_owned()),
        )
    } else if status.config.is_none() && rules.is_none() {
        Some(match rules_path {
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
    // A config that did not load (at start, or one that appeared broken
    // while rules.json was in charge) is in no engine, so the status does
    // not name it; the window still shows it, with the error.
    let config = status
        .config
        .clone()
        .or_else(|| status.error.as_ref().and(named));
    RoutingSource {
        config,
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
        Self::with_env(Env::system())
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
                eprintln!("pwrroute: {}", describe_note(note));
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

/// Slow dispatches, or all with `PWRROUTE_DIAGNOSTICS=1`, as one line of
/// URL-free JSON on stderr.
fn report_timing(record: &DispatchRecord, report_all: bool) {
    if (report_all || record.timings.total_us >= SLOW_DISPATCH_US)
        && let Ok(json) = serde_json::to_string(&record.timing())
    {
        eprintln!("pwrroute dispatch {json}");
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use pwrroute_browsers::Platform;
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
        }
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

        fn write(&self, name: &str, text: &str) -> PathBuf {
            let path = self.path(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, text).unwrap();
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
            LiveResolver::new(
                self.choice(),
                Some(self.rules_path()),
                SharedSource::default(),
            )
            .unwrap()
        }
    }

    const RULES: &str = r#"{
      "defaultBrowser": "Firefox",
      "rules": [{ "match": "github.com/*", "browser": "Arc" }]
    }"#;

    fn resolve(resolver: &LiveResolver, url: &str) -> String {
        resolver.resolve(url, None).unwrap().browser.name
    }

    #[test]
    fn rules_json_alone_routes_links_and_is_the_source() {
        let home = Home::new();
        let rules = home.write("PwrRoute/rules.json", RULES);
        let resolver = home.resolver();
        assert_eq!(resolver.refresh(), Ok(()));
        assert_eq!(resolve(&resolver, "https://github.com/pwrdrvr/x"), "Arc");
        assert_eq!(resolve(&resolver, "https://example.com/"), "Firefox");
        assert_eq!(
            resolver.source.get(),
            RoutingSource {
                config: None,
                rules: Some(rules),
                fallback: None,
                error: None,
            }
        );
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
        assert_eq!(resolve(&resolver, "https://github.com/x"), "Arc");

        home.write(
            "PwrRoute/rules.json",
            r#"{ "defaultBrowser": "Firefox", "rules": [{ "match": "github.com/*", "browser": "Orion" }] }"#,
        );
        resolver.refresh().unwrap();
        assert_eq!(resolve(&resolver, "https://github.com/x"), "Orion");

        // A broken edit keeps the last working rules and says why.
        home.write("PwrRoute/rules.json", "{ not json");
        resolver.refresh().unwrap();
        assert_eq!(resolve(&resolver, "https://github.com/x"), "Orion");
        let source = resolver.source.get();
        assert!(source.error.is_some(), "{source:?}");
        assert_eq!(source.fallback, None);
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
        assert_eq!(resolve(&resolver, "https://github.com/x"), "Arc");
        let source = resolver.source.get();
        assert_eq!(source.config, Some(config.canonicalize().unwrap()));
        assert_eq!(source.rules, Some(home.rules_path()));
        assert_eq!(source.error, None);
    }

    #[test]
    fn a_config_that_is_broken_at_start_is_named_with_its_error() {
        let home = Home::new();
        home.write("PwrRoute/rules.json", RULES);
        let config = home.write(".finicky.js", "export default {\n");
        let resolver = home.resolver();

        // Nothing has loaded: links fall back with the load error, and the
        // window is told which config it was, so it can show the error.
        let reason = resolver.refresh().unwrap_err();
        assert!(reason.contains(".finicky.js"), "{reason}");
        let source = resolver.source.get();
        assert_eq!(source.config, Some(config.canonicalize().unwrap()));
        assert_eq!(source.fallback.as_deref(), Some(reason.as_str()));
        assert_eq!(source.error.as_deref(), Some(reason.as_str()));
        assert_eq!(source.rules, None);

        // Fixing it takes charge on the next refresh.
        home.write(
            ".finicky.js",
            "export default { defaultBrowser: \"Arc\" };\n",
        );
        resolver.refresh().unwrap();
        let source = resolver.source.get();
        assert_eq!(source.config, Some(config.canonicalize().unwrap()));
        assert_eq!(source.rules, Some(home.rules_path()));
        assert_eq!(source.fallback, None);
        assert_eq!(source.error, None);
        assert_eq!(resolve(&resolver, "https://example.com/"), "Arc");
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
        assert_eq!(source.fallback, None);
        assert!(
            source
                .error
                .as_deref()
                .is_some_and(|e| e.contains(".finicky.js")),
            "{source:?}"
        );
    }

    #[test]
    fn through_the_router_a_file_change_is_seen_without_a_restart() {
        let home = Home::new();
        let source = SharedSource::default();
        let factory: ResolverFactory = {
            let (choice, rules, source) = (home.choice(), home.rules_path(), source.clone());
            Box::new(move || {
                Ok(Box::new(LiveResolver::new(choice, Some(rules), source)?) as Box<dyn Resolver>)
            })
        };
        let launcher = DryRunLauncher::new();
        let router = Router::spawn(Options::default(), Some(factory), launcher.clone()).unwrap();
        let handle = router.handle();

        let record = handle
            .dispatch_and_wait(Link::new("https://github.com/x", Source::CommandLine))
            .unwrap();
        let reason = source.get().fallback.expect("nothing in charge");
        assert!(record.steps.contains(&Step::Fallback {
            reason: reason.clone()
        }));
        assert_eq!(launcher.launched()[0].name, FALLBACK_BROWSER);
        assert_eq!(handle.status().unwrap().config_error, Some(reason));

        home.write("PwrRoute/rules.json", RULES);
        let record = handle
            .dispatch_and_wait(Link::new("https://github.com/x", Source::CommandLine))
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
                let mut resolver = LiveResolver::new(choice, Some(rules), source)?;
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
