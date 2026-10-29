//! Opening a URL in the browser the engine chose.
//!
//! The macOS command line is Finicky's (MIT, Copyright (c) 2015-2025 John
//! Sterling): `open -a NAME` or `open -b BUNDLE_ID`, `-g` to stay in the
//! background, and `-n --args` with the profile flag or the config's own
//! `args`, because `open` drops arguments when the app is already running
//! unless it starts a new instance (which then hands the URL to the running
//! one). A path that starts with `~/` is expanded with the [`Env`]'s home.
//!
//! That new instance is a whole second copy of the browser, started only to
//! pass its command line to the running one over Chromium's singleton
//! socket. For a running Chromium browser with a verified protocol the plan
//! carries that command line ([`Plan::socket`]), and [`Launcher::run`]
//! writes it to the socket itself ([`crate::singleton`]), running `open`
//! only when that fails.
//!
//! Windows and Linux run the browser's executable directly, never "the
//! default browser": that is PwrRoute, and the link would loop back.

use std::sync::Arc;
use std::{
    borrow::Cow,
    fmt,
    io::Read as _,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

use pwrroute_engine::{AppType, Browser};

use crate::{
    catalog::{BrowserEngine, BrowserInfo, Catalog, PrivateFlag, SingletonProtocol},
    desktop_entry, detect,
    env::{Env, Platform},
    profiles::{self, ProfileMatch},
    registry,
    singleton::{self, SOCKET_BUDGET, SocketError, SocketHandOff},
};

/// How long `open` may take on macOS before the launch is reported failed.
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);

/// Settings that come from the config rather than the browser.
#[derive(Debug, Clone, Copy, Default)]
pub struct LaunchOptions {
    /// Finicky's `options.openInBackground`: used when a browser does not
    /// set `openInBackground` itself.
    pub background_by_default: bool,
}

/// Whether an app with a given macOS bundle id is running; see
/// [`Launcher::with_running`].
#[derive(Clone)]
struct RunningApps(Arc<dyn Fn(&str) -> bool + Send + Sync>);

impl fmt::Debug for RunningApps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RunningApps")
    }
}

/// Plans and runs browser launches against one [`Env`].
#[derive(Debug, Clone)]
pub struct Launcher {
    env: Env,
    options: LaunchOptions,
    running: Option<RunningApps>,
    uptime: Option<Uptime>,
}

/// The system's uptime, sleep excluded; see [`Launcher::with_uptime`].
#[derive(Clone)]
struct Uptime(Arc<dyn Fn() -> Option<Duration> + Send + Sync>);

impl fmt::Debug for Uptime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Uptime")
    }
}

/// The exact command a launch runs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Plan {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// What the command could not honor. Never contains the URL.
    pub notes: Vec<Note>,
    /// The bundle id of the running browser this launch hands the link to
    /// (macOS, a Chromium browser with a profile or flags), when the link
    /// is not meant for the background. That copy brings itself forward,
    /// and since macOS 14 it may only once the active app gives way, so
    /// the launching app should. `None` for every other launch.
    pub hand_off_to: Option<String>,
    /// The singleton-socket hand-off, tried before `program`: `Some(Ok)`
    /// for a running Chromium browser with a verified protocol whose socket
    /// passed every check that does not connect, `Some(Err)` with why not
    /// when it is running but its socket did not, `None` for every other
    /// launch.
    pub socket: Option<Result<SocketHandOff, SocketError>>,
    socket_not_used: &'static str,
    platform: Platform,
}

/// URL-free progress from running a launch plan. Command completion only
/// means the launcher returned; it does not confirm a visible browser tab.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RunEvent {
    SocketStarted,
    SocketFinished {
        elapsed: Duration,
        error: Option<SocketError>,
    },
    CommandStarted,
    CommandFinished {
        elapsed: Duration,
        success: bool,
    },
}

impl fmt::Display for RunEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SocketStarted => f.write_str("socket begin"),
            Self::SocketFinished { elapsed, error } => {
                write!(
                    f,
                    "socket returned after {:.1}ms; ",
                    elapsed.as_secs_f64() * 1000.0
                )?;
                match error {
                    Some(error) => write!(f, "fallback_reason={error}"),
                    None => f.write_str("ack=true; tab_readiness=unobserved"),
                }
            }
            Self::CommandStarted => f.write_str("command begin"),
            Self::CommandFinished { elapsed, success } => write!(
                f,
                "command returned after {:.1}ms; success={success}; tab_readiness=unobserved",
                elapsed.as_secs_f64() * 1000.0,
            ),
        }
    }
}

/// A launch that ran: the plan, and which way the link went.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Launched {
    pub plan: Plan,
    pub delivery: Delivery,
}

/// Which way a launched link went.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Delivery {
    /// Over the running browser's singleton socket; `program` never ran.
    Socket,
    /// `program` ran; no socket hand-off was planned.
    Command,
    /// `program` ran because the socket hand-off failed, for this reason.
    CommandAfter(SocketError),
}

/// A few words for a log line. Never the URL.
impl fmt::Display for Delivery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Socket => f.write_str("over the browser's singleton socket"),
            Self::Command => f.write_str("by command"),
            Self::CommandAfter(why) => {
                write!(f, "by command, after the singleton socket failed ({why})")
            }
        }
    }
}

/// Something about a launch worth logging: the URL still opens.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Note {
    /// The live profile index was blocked; saved profile metadata selected it.
    ProfileCached { profile: String },
    /// The browser has no profile with this name or directory; it opens in
    /// whichever profile it picks. `available` lists the names it has.
    ProfileNotFound {
        profile: String,
        available: Vec<String>,
    },
    /// The browser is not in the catalog, or cannot select a profile from
    /// the command line (Safari, Orion, Arc), so the profile is ignored.
    ProfileUnsupported { profile: String },
    /// `openInBackground` was requested on a platform with no way to open a
    /// URL without focusing the browser (Windows, Linux).
    BackgroundUnsupported,
    /// A private window was requested, but the browser is not in the
    /// catalog or has no command-line way to open one (Safari, Orion, Arc),
    /// so the URL opens in a normal window.
    PrivateUnsupported { browser: String },
}

#[derive(Debug)]
#[non_exhaustive]
pub enum LaunchError {
    /// No installed browser matches the name, bundle id or path.
    NotInstalled { name: String },
    /// The program could not be started.
    Spawn {
        program: PathBuf,
        source: std::io::Error,
    },
    /// `open` (macOS) exited with an error. `stderr` is `open`'s message,
    /// which may quote the URL; treat it as private.
    Failed {
        program: PathBuf,
        status: ExitStatus,
        stderr: String,
    },
    /// `open` (macOS) did not finish in time.
    Timeout { program: PathBuf },
}

impl fmt::Display for LaunchError {
    // Deliberately free of URLs and of `open`'s stderr, so it can be logged.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled { name } => write!(f, "browser {name:?} is not installed"),
            Self::Spawn { program, source } => {
                write!(f, "could not start {}: {source}", program.display())
            }
            Self::Failed {
                program, status, ..
            } => write!(f, "{} failed: {status}", program.display()),
            Self::Timeout { program } => {
                write!(
                    f,
                    "{} did not finish in {OPEN_TIMEOUT:?}",
                    program.display()
                )
            }
        }
    }
}

impl std::error::Error for LaunchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl Plan {
    /// Delivery decisions without the program path, arguments or URL.
    pub fn diagnostic_summary(&self) -> String {
        let socket = match &self.socket {
            Some(Ok(handoff)) => format!("ready({})", handoff.protocol.label()),
            Some(Err(error)) => format!("unavailable({error})"),
            None => format!("skipped({})", self.socket_not_used),
        };
        let options: Vec<_> = self.args.iter().take_while(|a| *a != "--args").collect();
        format!(
            "command={} new_instance={} background={} yield_requested={} socket={socket}",
            if self.platform == Platform::MacOs {
                "open"
            } else {
                "executable"
            },
            self.platform == Platform::MacOs && options.iter().any(|a| a.as_str() == "-n"),
            self.platform == Platform::MacOs && options.iter().any(|a| a.as_str() == "-g"),
            self.hand_off_to.is_some(),
        )
    }

    /// The plan as a [`Command`], ready to spawn.
    pub fn to_command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        command
    }

    /// The command line, quoted for reading (not for a shell). It contains
    /// the URL: log it only where URLs may go.
    pub fn command_line(&self) -> String {
        std::iter::once(self.program.to_string_lossy().into_owned())
            .chain(self.args.iter().cloned())
            .map(|a| {
                // Not `{a:?}`: Debug doubles a Windows path's backslashes.
                if a.is_empty() || a.contains([' ', '"', '\'', '\t']) {
                    format!("\"{}\"", a.replace('"', "\\\""))
                } else {
                    a
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl Launcher {
    /// A launcher for this machine.
    pub fn new(options: LaunchOptions) -> Self {
        Self::with_env(Env::system(), options)
    }

    /// A launcher that resolves against `env` (a fixture, in tests).
    pub fn with_env(env: Env, options: LaunchOptions) -> Self {
        Self {
            env,
            options,
            running: None,
            uptime: None,
        }
    }

    /// The launcher, asking `running` whether the app with a bundle id is
    /// running. On macOS a profile, private-window or argument launch of a
    /// running Chromium browser then passes `open -g`: the instance `-n`
    /// starts only hands the link to the running browser and quits, so it
    /// must not take focus on its way (focus would bounce back to the app
    /// that was active, then to the browser). The running browser brings
    /// its own window forward. Without this, nothing counts as running and
    /// `-g` is passed only when the browser asks for the background.
    pub fn with_running(mut self, running: impl Fn(&str) -> bool + Send + Sync + 'static) -> Self {
        self.running = Some(RunningApps(Arc::new(running)));
        self
    }

    /// The launcher, reading the system's uptime with `uptime` (the app
    /// passes `pwrroute_os::system_uptime`): sleep excluded, the clock
    /// Chromium's `TimeTicks` reads on macOS. Edge's singleton message
    /// carries it, so without it Edge gets no socket hand-off.
    pub fn with_uptime(
        mut self,
        uptime: impl Fn() -> Option<Duration> + Send + Sync + 'static,
    ) -> Self {
        self.uptime = Some(Uptime(Arc::new(uptime)));
        self
    }

    pub fn env(&self) -> &Env {
        &self.env
    }

    /// The dry run: the command [`Launcher::launch`] would run for
    /// `browser`, without running it. `Ok(None)` for `AppType::None`
    /// (`browser: null` in the config): do not open the URL at all.
    ///
    /// When `browser.private` is set (the engine's reading of
    /// `rules[].private`, or of a known private flag in a JS config's
    /// `args`), the browser's private-window flag is added once; a browser
    /// without one gets [`Note::PrivateUnsupported`] and still opens.
    pub fn plan(&self, browser: &Browser) -> Result<Option<Plan>, LaunchError> {
        if browser.app_type == AppType::None {
            return Ok(None);
        }
        let expanded = expand_home(browser, &self.env);
        let browser = expanded.as_ref();
        let platform = self.env.platform();
        let entry = catalog_entry(&browser.name, browser.app_type, platform);
        let mut notes = Vec::new();
        let mut flags = self.profile_args(entry, &browser.profile, &mut notes);
        if browser.private
            && let Some(flag) = private_flag(browser, entry, &mut notes)
        {
            flags.get_or_insert_default().push(flag.to_owned());
        }
        let background = browser
            .open_in_background
            .unwrap_or(self.options.background_by_default);
        let mut hand_off_to = None;
        let mut socket = None;
        let mut socket_not_used = "not_applicable";
        let plan = match platform {
            Platform::MacOs => {
                let running = self.hands_off(browser, entry);
                let uses_args = flags.is_some() || !browser.args.is_empty();
                socket_not_used = if !uses_args {
                    "no_browser_arguments"
                } else if entry.is_none() {
                    "unknown_browser"
                } else if entry.is_some_and(|e| e.engine != BrowserEngine::Chromium) {
                    "not_chromium"
                } else if self.running.is_none() {
                    "running_check_unavailable"
                } else {
                    "browser_not_running"
                };
                if uses_args && running.is_some() {
                    if !background {
                        hand_off_to.clone_from(&running);
                    }
                    if let Some(entry) = entry {
                        (socket, socket_not_used) =
                            self.socket_plan(browser, entry, flags.as_deref());
                    }
                }
                macos_plan(browser, background, running.is_some(), flags)
            }
            _ => {
                if background {
                    notes.push(Note::BackgroundUnsupported);
                }
                let mut extra = flags.unwrap_or_default();
                extra.extend(browser_args(browser));
                match platform {
                    Platform::Windows => self.windows_plan(browser, entry, extra)?,
                    _ => self.linux_plan(browser, entry, extra)?,
                }
            }
        };
        let (program, args) = plan;
        Ok(Some(Plan {
            program,
            args,
            notes,
            hand_off_to,
            socket,
            socket_not_used,
            platform,
        }))
    }

    /// [`Launcher::plan`], then [`Launcher::run`].
    pub fn launch(&self, browser: &Browser) -> Result<Option<Launched>, LaunchError> {
        let Some(plan) = self.plan(browser)? else {
            return Ok(None);
        };
        self.run(plan).map(Some)
    }

    /// Runs a plan: the socket hand-off when it has one (waiting up to
    /// [`SOCKET_BUDGET`] for the browser's `ACK`), else, or when that fails,
    /// its command. On macOS this waits for `open` (which returns once the
    /// app has the URL) so its errors are reported; elsewhere the browser
    /// is started and left running.
    pub fn run(&self, plan: Plan) -> Result<Launched, LaunchError> {
        self.run_observed(plan, &mut |_| {})
    }

    /// Runs a plan while reporting URL-free progress, including failures.
    pub fn run_observed(
        &self,
        plan: Plan,
        observe: &mut dyn FnMut(RunEvent),
    ) -> Result<Launched, LaunchError> {
        let delivery = match &plan.socket {
            Some(Ok(handoff)) => {
                observe(RunEvent::SocketStarted);
                let started = Instant::now();
                let uptime = self.uptime.as_ref().map(|u| &*u.0 as &dyn Fn() -> _);
                let result = singleton::send(handoff, uptime, SOCKET_BUDGET);
                observe(RunEvent::SocketFinished {
                    elapsed: started.elapsed(),
                    error: result.as_ref().err().cloned(),
                });
                match result {
                    Ok(()) => Delivery::Socket,
                    Err(why) => Delivery::CommandAfter(why),
                }
            }
            Some(Err(why)) => Delivery::CommandAfter(why.clone()),
            None => Delivery::Command,
        };
        if delivery != Delivery::Socket {
            observe(RunEvent::CommandStarted);
            let started = Instant::now();
            let result = spawn(&plan);
            observe(RunEvent::CommandFinished {
                elapsed: started.elapsed(),
                success: result.is_ok(),
            });
            result?;
        }
        Ok(Launched { plan, delivery })
    }

    /// The singleton-socket hand-off for a running Chromium `browser`: its
    /// user data dir and the command line `open -n --args` would pass, or
    /// `None` when its protocol is unverified or that command line is not
    /// one the socket can carry (a config's `open` options before its own
    /// `--args`, or another `--user-data-dir`, whose browser listens on
    /// another socket).
    fn socket_plan(
        &self,
        browser: &Browser,
        entry: &BrowserInfo,
        flags: Option<&[String]>,
    ) -> (Option<Result<SocketHandOff, SocketError>>, &'static str) {
        // ACK only means the browser accepted a message, not that it read
        // the header it expects: only a protocol the catalog names is used.
        let Some(protocol) = entry.singleton_protocol else {
            return (None, "unverified_protocol");
        };
        if protocol == SingletonProtocol::Edge && self.uptime.is_none() {
            return (None, "no_uptime_clock");
        }
        let own_open_options = browser.args.iter().any(|a| a == "--args");
        let other_data_dir = browser
            .args
            .iter()
            .any(|a| a == "--user-data-dir" || a.starts_with("--user-data-dir="));
        if own_open_options {
            return (None, "custom_open_options");
        }
        if other_data_dir {
            return (None, "custom_user_data_dir");
        }
        let Some(user_data_dir) = entry.profile_dir(&self.env) else {
            return (None, "no_user_data_directory");
        };
        // `hands_off` found the browser by its macOS entry, so it has one.
        let Some(macos) = entry.macos.as_ref() else {
            return (None, "no_macos_catalog_entry");
        };
        let app_name = &macos.app_name;
        let mut argv = vec![macos_program(browser, app_name)];
        argv.extend(flags.unwrap_or_default().iter().cloned());
        argv.extend(browser_args(browser));
        let handoff = SocketHandOff {
            user_data_dir,
            argv,
            protocol,
            temp_dir: self
                .env
                .var("TMPDIR")
                .map_or_else(|| PathBuf::from("/tmp"), PathBuf::from),
            uid: singleton::current_uid(),
        };
        (Some(singleton::locate(&handoff).map(|_| handoff)), "")
    }

    /// The bundle id of the copy a `-n` launch of `browser` only hands the
    /// link to: a Chromium browser that [`Launcher::with_running`] says is
    /// running. Firefox's remote hand-off is not covered: whether the
    /// running Firefox raises its window from a background launch is
    /// untested.
    fn hands_off(&self, browser: &Browser, entry: Option<&BrowserInfo>) -> Option<String> {
        let (Some(running), Some(entry)) = (&self.running, entry) else {
            return None;
        };
        if entry.engine != BrowserEngine::Chromium {
            return None;
        }
        let bundle_id = match browser.app_type {
            AppType::BundleId => Some(browser.name.as_str()),
            _ => entry.macos.as_ref().and_then(|m| m.bundle_id.as_deref()),
        };
        bundle_id.filter(|id| (running.0)(id)).map(str::to_owned)
    }

    fn profile_args(
        &self,
        entry: Option<&BrowserInfo>,
        profile: &str,
        notes: &mut Vec<Note>,
    ) -> Option<Vec<String>> {
        if profile.is_empty() {
            return None;
        }
        let unsupported = || Note::ProfileUnsupported {
            profile: profile.to_owned(),
        };
        let Some(entry) = entry else {
            notes.push(unsupported());
            return None;
        };
        match profiles::resolve(entry, &self.env, profile) {
            ProfileMatch::Found { args, cached } => {
                if cached {
                    notes.push(Note::ProfileCached {
                        profile: profile.to_owned(),
                    });
                }
                Some(args)
            }
            ProfileMatch::NotFound(available) => {
                notes.push(Note::ProfileNotFound {
                    profile: profile.to_owned(),
                    available,
                });
                None
            }
            ProfileMatch::Unsupported => {
                notes.push(unsupported());
                None
            }
        }
    }

    fn windows_plan(
        &self,
        browser: &Browser,
        entry: Option<&BrowserInfo>,
        extra: Vec<String>,
    ) -> Result<(PathBuf, Vec<String>), LaunchError> {
        let env = &self.env;
        let catalog = Catalog::builtin();
        let is_entry = |p: &Path| {
            entry.is_some_and(|e| {
                catalog
                    .find_by_path(Platform::Windows, p)
                    .is_some_and(|f| f.id == e.id)
            })
        };
        let program = match (browser.app_type, entry) {
            (AppType::Path, _) => Some(PathBuf::from(&browser.name)).filter(|p| p.is_file()),
            (_, Some(e)) => detect::windows_exe(env, e)
                .or_else(|| {
                    registry::start_menu_internet(env)
                        .into_iter()
                        .map(|r| r.exe)
                        .find(|exe| is_entry(exe) && exe.is_file())
                })
                .or_else(|| {
                    e.windows.as_ref()?.exe.iter().find_map(|t| {
                        let file = t.rsplit('/').next()?;
                        registry::app_path(env, file).filter(|p| is_entry(p))
                    })
                }),
            (_, None) => registry::start_menu_internet(env)
                .into_iter()
                .find(|r| r.name.eq_ignore_ascii_case(&browser.name) && r.exe.is_file())
                .map(|r| r.exe)
                .or_else(|| {
                    let exe = if browser.name.to_ascii_lowercase().ends_with(".exe") {
                        browser.name.clone()
                    } else {
                        format!("{}.exe", browser.name)
                    };
                    registry::app_path(env, &exe)
                })
                .or_else(|| env.which(&browser.name)),
        };
        let program = program.ok_or_else(|| not_installed(browser))?;
        Ok((program, extra))
    }

    fn linux_plan(
        &self,
        browser: &Browser,
        entry: Option<&BrowserInfo>,
        extra: Vec<String>,
    ) -> Result<(PathBuf, Vec<String>), LaunchError> {
        let env = &self.env;
        let from_desktop = |id: &str| {
            let (_, e) = desktop_entry::find(env, id)?;
            desktop_entry::exec_command(&e.exec, &extra)
        };
        let command = match (browser.app_type, entry) {
            (AppType::Path, _) => {
                let path = Path::new(&browser.name);
                if !path.is_file() {
                    None
                } else if path.extension().is_some_and(|e| e == "desktop") {
                    std::fs::read_to_string(path)
                        .ok()
                        .and_then(|t| desktop_entry::DesktopEntry::parse(&t))
                        .and_then(|e| desktop_entry::exec_command(&e.exec, &extra))
                } else {
                    Some((path.to_owned(), extra.clone()))
                }
            }
            (_, Some(e)) => e.linux.as_ref().and_then(|l| {
                l.desktop_ids
                    .iter()
                    .find_map(|id| from_desktop(id))
                    .or_else(|| {
                        let exe = l.executables.iter().find_map(|x| env.which(x))?;
                        Some((exe, extra.clone()))
                    })
            }),
            (_, None) => from_desktop(&browser.name)
                .or_else(|| Some((env.which(&browser.name)?, extra.clone()))),
        };
        command.ok_or_else(|| not_installed(browser))
    }
}

/// The catalog entry the launcher opens a browser with: a `.app` path by
/// its file, anything else by name, alias or bundle id.
pub fn catalog_entry(
    name: &str,
    app_type: AppType,
    platform: Platform,
) -> Option<&'static BrowserInfo> {
    let catalog = Catalog::builtin();
    match app_type {
        AppType::Path => catalog.find_by_path(platform, Path::new(name)),
        _ => catalog.find(name),
    }
}

/// What a browser asked to open privately ([`Browser::private`]) opens, by
/// the window's name: the catalog's ("Incognito", "InPrivate", "Private
/// window") when it knows the browser, else the word for a private flag
/// its `args` carry, which reach it as written. `None` when a normal
/// window opens: the catalog's browser has no private mode (Safari, Arc;
/// its flag in `args` is ignored), or one it does not know was given none.
pub fn private_window(
    name: &str,
    app_type: AppType,
    args: &[String],
    platform: Platform,
) -> Option<&'static str> {
    match catalog_entry(name, app_type, platform) {
        Some(entry) => entry.private_label(),
        None => {
            let flag = args
                .iter()
                .find_map(|arg| pwrroute_engine::private_flag_name(arg))?;
            match flag {
                "incognito" => PrivateFlag::Incognito,
                "inprivate" => PrivateFlag::InPrivate,
                _ => PrivateFlag::PrivateWindow,
            }
            .label()
        }
    }
}

/// The private-window flag to add for `browser`, or `None` with a note
/// when it has none, or nothing when the config's `args` already carry it
/// (or, outside the catalog, any known private flag).
fn private_flag(
    browser: &Browser,
    entry: Option<&BrowserInfo>,
    notes: &mut Vec<Note>,
) -> Option<&'static str> {
    let own = entry
        .map(BrowserInfo::private_flag)
        .and_then(PrivateFlag::arg);
    let carries = |flag: &str| browser.args.iter().any(|a| a == flag);
    let carries_any = || {
        browser
            .args
            .iter()
            .any(|a| PrivateFlag::from_arg(a).is_some())
    };
    match own {
        Some(flag) if carries(flag) => None,
        Some(flag) => Some(flag),
        None if carries_any() => None,
        None => {
            notes.push(Note::PrivateUnsupported {
                browser: browser.name.clone(),
            });
            None
        }
    }
}

/// The arguments after the profile flag: the config's `args` when it has
/// any, else the URL. As in Finicky, `args` replace the URL rather than
/// adding to it, so a config that sets `args` passes the URL itself.
fn browser_args(browser: &Browser) -> Vec<String> {
    if browser.args.is_empty() {
        vec![browser.url.clone()]
    } else {
        browser.args.clone()
    }
}

/// A `Path` browser whose name starts with `~/`, with the home directory
/// of `env` in its place; any other browser as it is. `open -a` and the
/// catalog's path lookup never expand `~` themselves.
fn expand_home<'a>(browser: &'a Browser, env: &Env) -> Cow<'a, Browser> {
    if browser.app_type == AppType::Path
        && let Some(rest) = browser.name.strip_prefix("~/")
    {
        let mut path = env.home().to_path_buf();
        path.extend(rest.split('/'));
        let mut expanded = browser.clone();
        expanded.name = path.to_string_lossy().into_owned();
        return Cow::Owned(expanded);
    }
    Cow::Borrowed(browser)
}

/// `flags` are the profile and private-window flags, which must follow
/// `--args`. Whenever `--args` is used, for `flags` or for the config's own
/// `args`, `-n` goes with it: `open` drops arguments for a running app
/// unless it starts a new instance. `hand_off` says that new instance only
/// passes the link to a running copy (see [`Launcher::with_running`]), so it
/// starts with `-g` and leaves bringing a window forward to that copy.
fn macos_plan(
    browser: &Browser,
    background: bool,
    hand_off: bool,
    flags: Option<Vec<String>>,
) -> (PathBuf, Vec<String>) {
    let mut args = vec![
        if browser.app_type == AppType::BundleId {
            "-b"
        } else {
            "-a"
        }
        .to_owned(),
        browser.name.clone(),
    ];
    let uses_args = flags.is_some() || !browser.args.is_empty();
    if background || (hand_off && uses_args) {
        args.push("-g".into());
    }
    if uses_args {
        args.push("-n".into());
    }
    match (flags, uses_args) {
        (None, false) => args.push(browser.url.clone()),
        (flags, _) => {
            let flags = flags.unwrap_or_default();
            // Arguments before a config's own `--args` are `open`'s options
            // (Finicky passed them through); the flags belong after it.
            match browser.args.iter().position(|a| a == "--args") {
                Some(i) => {
                    args.extend_from_slice(&browser.args[..=i]);
                    args.extend(flags);
                    args.extend_from_slice(&browser.args[i + 1..]);
                }
                None => {
                    args.push("--args".into());
                    args.extend(flags);
                    args.extend(browser_args(browser));
                }
            }
        }
    }
    (PathBuf::from("/usr/bin/open"), args)
}

/// The program a socket hand-off names first, as a second instance's own
/// command line would: the bundle's executable. Chromium reads it only as
/// the program's name and never runs it, so it is not looked up on disk.
fn macos_program(browser: &Browser, app_name: &str) -> String {
    let bundle = match browser.app_type {
        AppType::Path => PathBuf::from(&browser.name),
        _ => PathBuf::from(format!("/Applications/{app_name}.app")),
    };
    bundle
        .join("Contents/MacOS")
        .join(app_name)
        .to_string_lossy()
        .into_owned()
}

fn not_installed(browser: &Browser) -> LaunchError {
    LaunchError::NotInstalled {
        name: browser.name.clone(),
    }
}

fn spawn(plan: &Plan) -> Result<(), LaunchError> {
    let spawn_error = |source| LaunchError::Spawn {
        program: plan.program.clone(),
        source,
    };
    let mut command = plan.to_command();
    command.stdin(Stdio::null()).stdout(Stdio::null());
    if plan.platform != Platform::MacOs {
        let mut child = command.stderr(Stdio::null()).spawn().map_err(spawn_error)?;
        // The browser may stay running; reap it whenever it exits.
        std::thread::spawn(move || child.wait());
        return Ok(());
    }
    let mut child = command
        .stderr(Stdio::piped())
        .spawn()
        .map_err(spawn_error)?;
    let deadline = Instant::now() + OPEN_TIMEOUT;
    let status = loop {
        match child.try_wait().map_err(spawn_error)? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(LaunchError::Timeout {
                    program: plan.program.clone(),
                });
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    if status.success() {
        return Ok(());
    }
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    Err(LaunchError::Failed {
        program: plan.program.clone(),
        status,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    /// A stand-in for `open`: does nothing in a normal run; started by the
    /// tests below with `-- fail` or `-- ok`, it exits accordingly.
    #[test]
    fn fake_open() {
        let args: Vec<String> = std::env::args().collect();
        let Some(i) = args.iter().position(|a| a == "--") else {
            return;
        };
        match args.get(i + 1).map(String::as_str) {
            Some("fail") => {
                // Not eprint!: libtest captures that, and exit discards it.
                let _ = std::io::stderr().write_all(b"open: no such app");
                std::process::exit(3);
            }
            Some("ok") => std::process::exit(0),
            _ => {}
        }
    }

    fn fake_open_plan(outcome: &str) -> Plan {
        Plan {
            program: std::env::current_exe().unwrap(),
            args: ["--exact", "launch::tests::fake_open", "--", outcome]
                .map(String::from)
                .to_vec(),
            notes: Vec::new(),
            hand_off_to: None,
            socket: None,
            socket_not_used: "test",
            platform: Platform::MacOs,
        }
    }

    #[test]
    fn open_is_waited_for_and_its_failure_reported() {
        spawn(&fake_open_plan("ok")).unwrap();
        match spawn(&fake_open_plan("fail")) {
            Err(LaunchError::Failed { status, stderr, .. }) => {
                assert_eq!(status.code(), Some(3));
                assert_eq!(stderr, "open: no such app");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn command_progress_includes_failed_launches_without_arguments() {
        let (_dir, launcher) = fixture(Platform::MacOs, &[]);
        for outcome in ["ok", "fail"] {
            let mut events = Vec::new();
            let result = launcher.run_observed(fake_open_plan(outcome), &mut |e| events.push(e));
            assert_eq!(result.is_ok(), outcome == "ok");
            assert!(
                matches!(events.as_slice(), [RunEvent::CommandStarted, RunEvent::CommandFinished { success, .. }] if *success == (outcome == "ok"))
            );
            let lines = events
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(lines.contains("tab_readiness=unobserved"));
            assert!(!lines.contains(URL));
        }
    }

    #[test]
    fn command_line_keeps_backslashes() {
        let plan = Plan {
            program: PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
            args: vec![
                "--profile-directory=Profile 1".into(),
                r#"say "hi""#.into(),
                "ü".into(),
            ],
            notes: Vec::new(),
            hand_off_to: None,
            socket: None,
            socket_not_used: "test",
            platform: Platform::Windows,
        };
        assert_eq!(
            plan.command_line(),
            r#""C:\Program Files\Google\Chrome\Application\chrome.exe" "--profile-directory=Profile 1" "say \"hi\"" ü"#
        );
    }

    const URL: &str = "https://example.com/path?q=a%20b";

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

    /// A fixture machine, kept alive with its temp dir.
    fn fixture(platform: Platform, files: &[&str]) -> (tempfile::TempDir, Launcher) {
        let dir = tempfile::tempdir().unwrap();
        for rel in files {
            let mut path = dir.path().to_path_buf();
            path.extend(rel.split('/'));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "").unwrap();
        }
        let env = Env::fixture(platform, dir.path());
        (dir, Launcher::with_env(env, LaunchOptions::default()))
    }

    /// The plan for `b` with a private window requested.
    fn private_plan(launcher: &Launcher, b: &Browser) -> Plan {
        let b = Browser {
            private: true,
            ..b.clone()
        };
        launcher.plan(&b).unwrap().expect("a plan")
    }

    fn args(p: &Plan) -> Vec<&str> {
        p.args.iter().map(String::as_str).collect()
    }

    fn unsupported(name: &str) -> Vec<Note> {
        vec![Note::PrivateUnsupported {
            browser: name.into(),
        }]
    }

    #[test]
    fn macos_private_window_goes_through_open_args() {
        let (_dir, launcher) = fixture(Platform::MacOs, &[]);
        for (name, flag) in [
            ("Google Chrome", "--incognito"),
            ("Microsoft Edge", "--inprivate"),
            ("Firefox", "--private-window"),
        ] {
            let p = private_plan(&launcher, &browser(name, AppType::AppName));
            assert_eq!(p.program, PathBuf::from("/usr/bin/open"));
            assert_eq!(args(&p), ["-a", name, "-n", "--args", flag, URL], "{name}");
            assert!(p.notes.is_empty(), "{name}");
        }
        // By bundle id too, in the background, and with a profile: the
        // private flag follows the profile flag.
        let mut b = browser("com.google.Chrome", AppType::BundleId);
        b.open_in_background = Some(true);
        let p = private_plan(&launcher, &b);
        assert_eq!(
            args(&p),
            [
                "-b",
                "com.google.Chrome",
                "-g",
                "-n",
                "--args",
                "--incognito",
                URL
            ]
        );

        let dir = tempfile::tempdir().unwrap();
        let local_state = dir
            .path()
            .join("home/Library/Application Support/Google/Chrome/Local State");
        std::fs::create_dir_all(local_state.parent().unwrap()).unwrap();
        std::fs::write(
            &local_state,
            include_str!("../tests/fixtures/chromium-local-state.json"),
        )
        .unwrap();
        let launcher = Launcher::with_env(
            Env::fixture(Platform::MacOs, dir.path()),
            LaunchOptions::default(),
        );
        let mut b = browser("Google Chrome", AppType::AppName);
        b.profile = "Personal".into();
        let p = private_plan(&launcher, &b);
        assert_eq!(
            args(&p),
            [
                "-a",
                "Google Chrome",
                "-n",
                "--args",
                "--profile-directory=Profile 1",
                "--incognito",
                URL
            ]
        );
        assert!(p.notes.is_empty());
    }

    #[test]
    fn a_running_chromium_browser_gets_the_link_without_a_focus_bounce() {
        let (_dir, launcher) = fixture(Platform::MacOs, &[]);
        let env = launcher.env().clone();
        let running = Launcher::with_env(env.clone(), LaunchOptions::default())
            .with_running(|id| id == "com.google.Chrome" || id == "org.mozilla.firefox");
        let chrome = browser("Google Chrome", AppType::AppName);
        // The new instance only hands over: it starts in the background,
        // and the running copy is the one to bring forward.
        let handed = private_plan(&running, &chrome);
        assert_eq!(handed.hand_off_to.as_deref(), Some("com.google.Chrome"));
        assert_eq!(
            args(&handed),
            [
                "-a",
                "Google Chrome",
                "-g",
                "-n",
                "--args",
                "--incognito",
                URL
            ]
        );
        let by_id = browser("com.google.Chrome", AppType::BundleId);
        assert_eq!(
            args(&private_plan(&running, &by_id)),
            [
                "-b",
                "com.google.Chrome",
                "-g",
                "-n",
                "--args",
                "--incognito",
                URL
            ]
        );
        // A plain link starts no instance and keeps `open`'s activation.
        let plain = running.plan(&chrome).unwrap().unwrap();
        assert_eq!(args(&plain), ["-a", "Google Chrome", URL]);
        assert_eq!(plain.hand_off_to, None);
        // Asked for the background as well: `-g` once, and nothing to
        // bring forward, whether the link or the config asked.
        let mut quiet = chrome.clone();
        quiet.open_in_background = Some(true);
        let quiet_plan = private_plan(&running, &quiet);
        assert_eq!(quiet_plan.hand_off_to, None);
        let by_default = Launcher::with_env(
            env.clone(),
            LaunchOptions {
                background_by_default: true,
            },
        )
        .with_running(|id| id == "com.google.Chrome");
        assert_eq!(private_plan(&by_default, &chrome).hand_off_to, None);
        assert_eq!(
            args(&quiet_plan),
            [
                "-a",
                "Google Chrome",
                "-g",
                "-n",
                "--args",
                "--incognito",
                URL
            ]
        );
        // Not running: the new instance is the browser and comes forward.
        let idle =
            Launcher::with_env(env.clone(), LaunchOptions::default()).with_running(|_| false);
        assert_eq!(
            args(&private_plan(&idle, &chrome)),
            ["-a", "Google Chrome", "-n", "--args", "--incognito", URL]
        );
        // Firefox is not a Chromium hand-off: unchanged even while running.
        let firefox = browser("Firefox", AppType::AppName);
        assert_eq!(
            args(&private_plan(&running, &firefox)),
            ["-a", "Firefox", "-n", "--args", "--private-window", URL]
        );
        // Without a running check (tests, other callers): unchanged.
        assert_eq!(
            args(&private_plan(&launcher, &chrome)),
            ["-a", "Google Chrome", "-n", "--args", "--incognito", URL]
        );
    }

    /// A fixture Mac whose Chrome is running with a singleton socket, the
    /// launcher told so, and `open` swapped for the fake one.
    #[cfg(unix)]
    fn running_chrome() -> (crate::singleton::fake::Fake, Launcher) {
        let fake =
            crate::singleton::fake::Fake::new("home/Library/Application Support/Google/Chrome");
        std::fs::write(
            fake.data.join("Local State"),
            include_str!("../tests/fixtures/chromium-local-state.json"),
        )
        .unwrap();
        let mut env = Env::fixture(Platform::MacOs, fake.root());
        env.set_var("TMPDIR", fake.temp.to_string_lossy().into_owned());
        let launcher = Launcher::with_env(env, LaunchOptions::default())
            .with_running(|id| id == "com.google.Chrome" || id == "org.mozilla.firefox");
        (fake, launcher)
    }

    /// `plan` with its command replaced by the fake `open`, so a fallback
    /// never starts anything real.
    #[cfg(unix)]
    fn with_fake_open(mut plan: Plan, outcome: &str) -> Plan {
        let fake = fake_open_plan(outcome);
        plan.program = fake.program;
        plan.args = fake.args;
        plan
    }

    #[cfg(unix)]
    #[test]
    fn a_running_chromium_browser_gets_the_link_over_its_socket() {
        let (mut fake, launcher) = running_chrome();
        let mut b = browser("Google Chrome", AppType::AppName);
        b.profile = "Personal".into();
        let plan = launcher.plan(&b).unwrap().unwrap();
        // The dry run says which way: the socket, with `open` behind it.
        let Some(Ok(handoff)) = &plan.socket else {
            panic!("expected a socket hand-off, got {:?}", plan.socket);
        };
        assert_eq!(handoff.user_data_dir, fake.data);
        assert_eq!(
            handoff.argv,
            [
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "--profile-directory=Profile 1",
                URL
            ]
        );
        assert_eq!(
            args(&plan),
            [
                "-a",
                "Google Chrome",
                "-g",
                "-n",
                "--args",
                "--profile-directory=Profile 1",
                URL
            ]
        );
        assert_eq!(plan.hand_off_to.as_deref(), Some("com.google.Chrome"));

        let got = fake.serve(Some(b"ACK"));
        let expected = crate::singleton::encode("/", &[], &handoff.argv).unwrap();
        // The fake `open` fails: had it run, `run` would say so.
        let mut events = Vec::new();
        let launched = launcher
            .run_observed(with_fake_open(plan, "fail"), &mut |e| events.push(e))
            .unwrap();
        assert_eq!(launched.delivery, Delivery::Socket);
        assert!(matches!(
            events.as_slice(),
            [
                RunEvent::SocketStarted,
                RunEvent::SocketFinished { error: None, .. }
            ]
        ));
        assert_eq!(got.recv().unwrap(), expected);
    }

    #[cfg(unix)]
    #[test]
    fn edge_defaults_keep_the_profile_and_url_on_every_launch() {
        use std::rc::Rc;

        use pwrroute_engine::{DefaultHost, Engine, Limits, RulesFile, Sources};

        // Edge 156's native sender puts two timestamps between cwd and
        // argv. The upstream header lost the profile and URL even though
        // Edge replied ACK, so Edge's own header goes out instead.
        let edge = || {
            let fake = crate::singleton::fake::Fake::new(
                "home/Library/Application Support/Microsoft Edge",
            );
            std::fs::write(
                fake.data.join("Local State"),
                include_str!("../tests/fixtures/chromium-local-state.json")
                    .replace(r#""last_used": "Profile 1""#, r#""last_used": "Profile 3""#),
            )
            .unwrap();
            let mut env = Env::fixture(Platform::MacOs, fake.root());
            env.set_var("TMPDIR", fake.temp.to_string_lossy().into_owned());
            let launcher = Launcher::with_env(env, LaunchOptions::default()).with_running(|_| true);
            (fake, launcher)
        };

        for (name, app_type, selector) in [
            ("Microsoft Edge", AppType::AppName, "-a"),
            ("com.microsoft.edgemac", AppType::BundleId, "-b"),
            ("/Applications/Microsoft Edge.app", AppType::Path, "-a"),
        ] {
            // No Finicky config and no rules: only the saved default.
            let engine = Engine::from_sources(
                &Sources {
                    config: None,
                    rules: RulesFile {
                        default_browser: name.into(),
                        default_profile: "Personal".into(),
                        ..RulesFile::default()
                    },
                },
                Rc::new(DefaultHost),
                Limits::default(),
            )
            .unwrap();
            for url in [URL, "https://example.com/second?x=1&y=2"] {
                let b = engine.resolve(url, None).unwrap().browser;
                assert_eq!(b.app_type, app_type);
                assert_eq!(b.profile, "Personal");

                // Without an uptime clock there is no Edge header: `open`.
                let (_fake, launcher) = edge();
                let plan = launcher.plan(&b).unwrap().unwrap();
                assert!(plan.notes.is_empty());
                assert_eq!(plan.socket, None);
                assert!(
                    plan.diagnostic_summary()
                        .contains("skipped(no_uptime_clock)")
                );
                assert_eq!(plan.hand_off_to.as_deref(), Some("com.microsoft.edgemac"));
                assert_eq!(
                    args(&plan),
                    [
                        selector,
                        name,
                        "-g",
                        "-n",
                        "--args",
                        "--profile-directory=Profile 1",
                        url,
                    ]
                );
                let launched = launcher.run(with_fake_open(plan, "ok")).unwrap();
                assert_eq!(launched.delivery, Delivery::Command);

                // With one, the socket gets Edge's header and the profile and
                // URL arrive in argv where Edge reads them.
                let (mut fake, launcher) = edge();
                let launcher = launcher.with_uptime(|| Some(Duration::from_micros(7_000_001)));
                let got = fake.serve(Some(b"ACK"));
                let plan = launcher.plan(&b).unwrap().unwrap();
                assert!(matches!(plan.socket, Some(Ok(_))), "{:?}", plan.socket);
                assert!(plan.diagnostic_summary().contains("socket=ready(edge)"));
                let launched = launcher.run(with_fake_open(plan, "fail")).unwrap();
                assert_eq!(launched.delivery, Delivery::Socket);
                let message = got.recv().unwrap();
                let tokens: Vec<&[u8]> = message.split(|b| *b == 0).collect();
                assert_eq!(tokens.len(), 7, "{name}");
                assert_eq!(tokens[..2], [b"START".as_slice(), b"/"]);
                assert!(tokens[2].iter().all(u8::is_ascii_digit));
                assert_eq!(tokens[3], b"7000001");
                assert!(tokens[4].ends_with(b"/Contents/MacOS/Microsoft Edge"));
                assert_eq!(tokens[5], b"--profile-directory=Profile 1");
                assert_eq!(tokens[6], url.as_bytes());
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_socket_hand_off_falls_back_to_open() {
        let (mut fake, launcher) = running_chrome();
        let b = Browser {
            private: true,
            ..browser("Google Chrome", AppType::AppName)
        };
        let _got = fake.serve(Some(b"SHUTDOWN"));
        let plan = launcher.plan(&b).unwrap().unwrap();
        assert!(matches!(plan.socket, Some(Ok(_))));
        let mut events = Vec::new();
        let launched = launcher
            .run_observed(with_fake_open(plan, "ok"), &mut |e| events.push(e))
            .unwrap();
        assert!(matches!(
            events.as_slice(),
            [
                RunEvent::SocketStarted,
                RunEvent::SocketFinished {
                    error: Some(SocketError::ShuttingDown),
                    ..
                },
                RunEvent::CommandStarted,
                RunEvent::CommandFinished { success: true, .. }
            ]
        ));
        assert_eq!(
            launched.delivery,
            Delivery::CommandAfter(SocketError::ShuttingDown)
        );
        // And `open`'s own failure is still the launch's.
        let (mut fake, launcher) = running_chrome();
        let _got = fake.serve(Some(b"SHUTDOWN"));
        let plan = launcher.plan(&b).unwrap().unwrap();
        assert!(matches!(
            launcher.run(with_fake_open(plan, "fail")),
            Err(LaunchError::Failed { .. })
        ));

        // A socket that fails its checks at planning is never connected to.
        let (fake, launcher) = running_chrome();
        std::fs::remove_file(fake.sock_dir.join("SingletonCookie")).unwrap();
        let plan = launcher.plan(&b).unwrap().unwrap();
        assert_eq!(plan.socket, Some(Err(SocketError::CookieMismatch)));
        let launched = launcher.run(with_fake_open(plan, "ok")).unwrap();
        assert_eq!(
            launched.delivery,
            Delivery::CommandAfter(SocketError::CookieMismatch)
        );
    }

    #[test]
    fn diagnostic_plans_explain_edge_and_never_print_custom_arguments() {
        let (_dir, launcher) = fixture(Platform::MacOs, &[]);
        let launcher = Launcher::with_env(launcher.env().clone(), LaunchOptions::default())
            .with_running(|_| true);
        let mut b = browser("Microsoft Edge", AppType::AppName);
        b.args = vec![
            "--new-window".into(),
            "https://secret.example.com/?token=private".into(),
        ];
        b.profile = "Secret Profile".into();
        let summary = launcher.plan(&b).unwrap().unwrap().diagnostic_summary();
        assert!(summary.contains("socket=skipped(no_uptime_clock)"));
        let mut brave = b.clone();
        brave.name = "Brave Browser".into();
        let summary = launcher.plan(&brave).unwrap().unwrap().diagnostic_summary();
        assert!(summary.contains("socket=skipped(unverified_protocol)"));
        assert!(summary.contains("new_instance=true background=true yield_requested=true"));
        for secret in [&b.url, &b.profile, &b.name, &b.args[1]] {
            assert!(!summary.contains(secret), "{summary}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_socket_is_planned_only_for_what_it_can_carry() {
        let (_fake, launcher) = running_chrome();
        let socket_of = |b: &Browser| launcher.plan(b).unwrap().unwrap().socket;
        let chrome = browser("Google Chrome", AppType::AppName);
        // A plain link starts no second instance: nothing to hand off.
        assert_eq!(socket_of(&chrome), None);
        // By bundle id, in the background, with a config's own arguments.
        let mut by_id = browser("com.google.Chrome", AppType::BundleId);
        by_id.private = true;
        by_id.open_in_background = Some(true);
        let plan = launcher.plan(&by_id).unwrap().unwrap();
        assert_eq!(plan.hand_off_to, None, "the background yields nothing");
        let Some(Ok(handoff)) = plan.socket else {
            panic!("expected a socket hand-off");
        };
        assert_eq!(
            handoff.argv,
            [
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "--incognito",
                URL
            ]
        );
        let mut own_args = chrome.clone();
        own_args.args = ["--new-window", URL].map(String::from).to_vec();
        let Some(Ok(handoff)) = socket_of(&own_args) else {
            panic!("expected a socket hand-off");
        };
        assert_eq!(&handoff.argv[1..], ["--new-window", URL]);
        // `open` options of the config's own, or another user data dir:
        // only `open` gets those right.
        own_args.args = ["-F", "--args", URL].map(String::from).to_vec();
        assert_eq!(socket_of(&own_args), None);
        own_args.args = ["--user-data-dir=/tmp/x", URL].map(String::from).to_vec();
        assert_eq!(socket_of(&own_args), None);
        // Firefox is no Chromium; a Chrome that is not running has no socket.
        let firefox = Browser {
            private: true,
            ..browser("Firefox", AppType::AppName)
        };
        assert_eq!(socket_of(&firefox), None);
        let idle = Launcher::with_env(launcher.env().clone(), LaunchOptions::default())
            .with_running(|_| false);
        assert_eq!(idle.plan(&by_id).unwrap().unwrap().socket, None);
    }

    #[test]
    fn private_flag_is_not_added_twice() {
        let (_dir, launcher) = fixture(Platform::MacOs, &[]);
        let mut b = browser("Google Chrome", AppType::AppName);
        b.args = ["--incognito", URL].map(String::from).to_vec();
        let p = private_plan(&launcher, &b);
        assert_eq!(
            args(&p),
            ["-a", "Google Chrome", "-n", "--args", "--incognito", URL]
        );
        assert!(p.notes.is_empty());

        // A config's own `--args`, with the flag after it.
        b.args = ["-F", "--args", "--incognito", URL]
            .map(String::from)
            .to_vec();
        let p = private_plan(&launcher, &b);
        assert_eq!(
            args(&p),
            [
                "-a",
                "Google Chrome",
                "-n",
                "-F",
                "--args",
                "--incognito",
                URL
            ]
        );

        // Edge's flag is --inprivate: --incognito in the args is not it.
        let mut b = browser("Microsoft Edge", AppType::AppName);
        b.args = ["--incognito", URL].map(String::from).to_vec();
        let p = private_plan(&launcher, &b);
        assert_eq!(
            args(&p),
            [
                "-a",
                "Microsoft Edge",
                "-n",
                "--args",
                "--inprivate",
                "--incognito",
                URL
            ]
        );
        let mut b = browser("Microsoft Edge", AppType::AppName);
        b.args = ["--inprivate", URL].map(String::from).to_vec();
        let p = private_plan(&launcher, &b);
        assert_eq!(
            args(&p),
            ["-a", "Microsoft Edge", "-n", "--args", "--inprivate", URL]
        );

        // Without the request, args are passed through as they are.
        let mut b = browser("Microsoft Edge", AppType::AppName);
        b.args = ["--incognito", URL].map(String::from).to_vec();
        let p = launcher.plan(&b).unwrap().unwrap();
        assert_eq!(
            args(&p),
            ["-a", "Microsoft Edge", "-n", "--args", "--incognito", URL]
        );
    }

    #[test]
    fn browsers_without_a_private_window_still_open() {
        let (_dir, launcher) = fixture(Platform::MacOs, &[]);
        for name in ["Safari", "Orion", "Arc", "Some Unknown Browser"] {
            let p = private_plan(&launcher, &browser(name, AppType::AppName));
            assert_eq!(args(&p), ["-a", name, URL], "{name}");
            assert_eq!(p.notes, unsupported(name), "{name}");
        }
        let mut b = browser("com.apple.Safari", AppType::BundleId);
        b.profile = "Work".into();
        let p = private_plan(&launcher, &b);
        assert_eq!(args(&p), ["-b", "com.apple.Safari", URL]);
        assert_eq!(
            p.notes,
            [
                Note::ProfileUnsupported {
                    profile: "Work".into()
                },
                Note::PrivateUnsupported {
                    browser: "com.apple.Safari".into()
                }
            ]
        );

        // Outside the catalog, args that carry a known flag are trusted.
        let mut b = browser("Some Unknown Browser", AppType::AppName);
        b.args = ["--private-window", URL].map(String::from).to_vec();
        let p = private_plan(&launcher, &b);
        assert_eq!(
            args(&p),
            [
                "-a",
                "Some Unknown Browser",
                "-n",
                "--args",
                "--private-window",
                URL
            ]
        );
        assert!(p.notes.is_empty());
    }

    #[test]
    fn windows_and_linux_pass_the_flag_directly() {
        let (dir, launcher) = fixture(
            Platform::Windows,
            &["Program Files/Microsoft/Edge/Application/msedge.exe"],
        );
        let p = private_plan(&launcher, &browser("Microsoft Edge", AppType::AppName));
        assert!(p.program.starts_with(dir.path()));
        assert!(p.program.ends_with("msedge.exe"));
        assert_eq!(args(&p), ["--inprivate", URL]);
        assert!(p.notes.is_empty());

        let (_dir, launcher) = fixture(Platform::Linux, &["bin/firefox", "bin/epiphany"]);
        let p = private_plan(&launcher, &browser("Firefox", AppType::AppName));
        assert!(p.program.ends_with("firefox"));
        assert_eq!(args(&p), ["--private-window", URL]);
        assert!(p.notes.is_empty());

        let mut b = browser("Firefox", AppType::AppName);
        b.args = ["--private-window", "-new-window", URL]
            .map(String::from)
            .to_vec();
        let p = private_plan(&launcher, &b);
        assert_eq!(args(&p), ["--private-window", "-new-window", URL]);

        let p = private_plan(&launcher, &browser("epiphany", AppType::AppName));
        assert_eq!(args(&p), [URL]);
        assert_eq!(p.notes, unsupported("epiphany"));
    }

    /// `Browser::private` is the request: a plan adds the flag only when
    /// the engine set it, never on its own.
    #[test]
    fn plan_reads_the_request_from_the_browser() {
        let (_dir, launcher) = fixture(Platform::MacOs, &[]);
        let b = browser("Google Chrome", AppType::AppName);
        assert!(!b.private);
        let p = launcher.plan(&b).unwrap().unwrap();
        assert_eq!(args(&p), ["-a", "Google Chrome", URL]);
        assert!(p.notes.is_empty());

        let b = Browser { private: true, ..b };
        let p = launcher.plan(&b).unwrap().unwrap();
        assert_eq!(
            args(&p),
            ["-a", "Google Chrome", "-n", "--args", "--incognito", URL]
        );
        assert!(p.notes.is_empty());
    }

    #[test]
    fn spawn_errors_name_the_program() {
        let plan = Plan {
            program: PathBuf::from("/nonexistent/browser"),
            args: Vec::new(),
            notes: Vec::new(),
            hand_off_to: None,
            socket: None,
            socket_not_used: "test",
            platform: Platform::Linux,
        };
        let err = spawn(&plan).unwrap_err();
        assert!(matches!(err, LaunchError::Spawn { .. }), "{err}");
    }

    /// A private window opens by the launcher's lookup: the catalog's word
    /// for a browser it knows, none for one without a private mode, and a
    /// flag in `args` only for a browser it does not know.
    #[test]
    fn private_window_follows_the_launchers_lookup() {
        let mac = Platform::MacOs;
        let none: &[String] = &[];
        let flag = |arg: &str| vec![arg.to_owned()];
        let at = |name: &str, app_type: AppType, args: &[String]| {
            private_window(name, app_type, args, mac)
        };
        assert_eq!(
            at("Google Chrome", AppType::AppName, none),
            Some("Incognito")
        );
        assert_eq!(
            at("com.microsoft.edgemac", AppType::BundleId, none),
            Some("InPrivate")
        );
        assert_eq!(
            at("/Applications/Firefox.app", AppType::Path, none),
            Some("Private window")
        );
        assert_eq!(
            at("Safari", AppType::AppName, &flag("--private-window")),
            None
        );
        assert_eq!(at("Nightly Build", AppType::AppName, none), None);
        assert_eq!(
            at("Nightly Build", AppType::AppName, &flag("-InPrivate")),
            Some("InPrivate")
        );
        // A path the engine reads as an app name is looked up by name.
        assert_eq!(
            at(
                "Apps/Google Chrome.app",
                AppType::AppName,
                &flag("--incognito")
            ),
            Some("Incognito")
        );
        assert_eq!(
            at("/Applications/Google Chrome.app", AppType::AppName, none),
            None
        );
    }
}
