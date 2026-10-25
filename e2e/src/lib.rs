//! Desktop E2E for PwrRoute: launch the built app, drive it through its test
//! hook, and read back what it drew and what the router decided.
//!
//! The app half is `crates/pwrroute-app/src/e2e.rs`; `CONTRACT.md` beside
//! this crate is the protocol between them and what the router must report.
//! [`os`] holds the checks made from outside the app: which windows the OS
//! sees, and screenshots.
//!
//! A test starts with [`harness!`], which returns from the test unless
//! `cargo xtask e2e` named an app to launch. Each [`App`] gets its own temp
//! directory for the control socket and the router's lock, and an empty home
//! directory, so a run never meets the developer's own PwrRoute or config.

pub mod os;

use std::{
    env, fs,
    io::{BufRead as _, BufReader, Write as _},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicU32, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use interprocess::local_socket::{ConnectOptions, Stream, prelude::*};
use pwrroute_engine::discovery;
use serde_json::{Value, json};

/// The app to launch: the `pwrroute` executable, or on macOS the
/// `PwrRoute.app` bundle. Set by `cargo xtask e2e`; unset, every test skips.
pub const APP_ENV: &str = "PWRROUTE_E2E_APP";
/// Where each test writes its app log, state dumps and screenshots.
pub const ARTIFACTS_ENV: &str = "PWRROUTE_E2E_ARTIFACTS";
/// `1` to take screenshots. Off by default: on a developer's desktop a
/// screenshot can catch other windows, and URLs are private data.
pub const SCREENSHOTS_ENV: &str = "PWRROUTE_E2E_SCREENSHOTS";
/// `1` only on a throwaway guest or CI runner: allows the scenarios that
/// register PwrRoute with the OS or make it the default browser.
pub const DISPOSABLE_ENV: &str = "PWRROUTE_E2E_DISPOSABLE";

/// The hook's environment, as `crates/pwrroute-app/src/e2e.rs` reads it.
const SOCKET_ENV: &str = "PWRROUTE_E2E_SOCKET";
const ROUTER_DIR_ENV: &str = "PWRROUTE_E2E_ROUTER_DIR";
/// `--dry-run` and `--config` for a copy the OS starts, which gets no
/// arguments of ours (CONTRACT.md, R8).
const DRY_RUN_ENV: &str = "PWRROUTE_E2E_DRY_RUN";
const CONFIG_ENV: &str = "PWRROUTE_E2E_CONFIG";

/// How long the app may take to answer on its socket after launch. A cold
/// start under CI load, with a debug build, is slow.
pub const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the app may take to paint its main window's first frame, once
/// its socket answers. Waited out through `ping`, which never needs the UI
/// thread; see [`App::wait_for_main_window`].
pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
/// The default for [`App::wait_for`].
pub const WAIT: Duration = Duration::from_secs(10);
/// Where the app serves local agents (`pwrroute_mcp::link::ENDPOINT_ENV`).
const MCP_ENDPOINT_ENV: &str = "PWRROUTE_MCP_ENDPOINT";
/// One request's exchange, once connected.
const IO_TIMEOUT: Duration = Duration::from_secs(10);
/// The registration actions' exchange: they run the OS's tools, and
/// xdg-utils' generic `xdg-settings` alone sleeps 12 s while it sets the
/// default browser.
const OS_ACTION_TIMEOUT: Duration = Duration::from_secs(60);

/// Returns from the calling test unless an app was named to launch.
#[macro_export]
macro_rules! harness {
    ($test:expr) => {
        match $crate::Harness::from_env($test) {
            Some(harness) => harness,
            None => {
                eprintln!("skipped {}: no app to launch; run `cargo xtask e2e`", $test);
                return;
            }
        }
    };
}

/// One test's view of the build under test.
#[derive(Debug, Clone)]
pub struct Harness {
    test: String,
    executable: PathBuf,
    bundle: Option<PathBuf>,
    artifacts: PathBuf,
    screenshots: bool,
}

impl Harness {
    /// `None` when no app was named, so the test skips.
    pub fn from_env(test: &str) -> Option<Self> {
        let app = PathBuf::from(env::var_os(APP_ENV)?);
        let (executable, bundle) = if app.extension().is_some_and(|e| e == "app") {
            (app.join("Contents/MacOS/pwrroute"), Some(app))
        } else {
            (app, None)
        };
        let artifacts = env::var_os(ARTIFACTS_ENV)
            .map_or_else(
                || Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/e2e-artifacts"),
                PathBuf::from,
            )
            .join(test);
        Some(Self {
            test: test.to_owned(),
            executable,
            bundle,
            artifacts,
            screenshots: env::var_os(SCREENSHOTS_ENV).is_some_and(|v| v == "1"),
        })
    }

    pub fn test(&self) -> &str {
        &self.test
    }

    /// The `.app` bundle the executable lives in, on macOS.
    pub fn bundle(&self) -> Option<&Path> {
        self.bundle.as_deref()
    }

    /// The per-test artifact directory.
    pub fn artifacts(&self) -> &Path {
        &self.artifacts
    }

    /// True on a throwaway guest or runner, where a scenario may register
    /// PwrRoute with the OS.
    pub fn disposable() -> bool {
        env::var_os(DISPOSABLE_ENV).is_some_and(|v| v == "1")
    }

    /// Launches the router with its window, isolated, in dry run and with no
    /// config: the window shows its own rules.
    pub fn launch(&self) -> Result<App> {
        self.launch_with(Launch::default())
    }

    pub fn launch_with(&self, launch: Launch) -> Result<App> {
        App::launch(self.clone(), launch)
    }

    /// The environment a copy the OS starts must see to serve the hook, run
    /// dry and load the fixture config (CONTRACT.md, "Registration"). Pass it
    /// to whatever asks the OS to open a link: `xdg-open` and `start` hand
    /// it down, and macOS takes it as `open --env`.
    pub fn session_env(&self) -> Vec<(&'static str, String)> {
        vec![
            (SOCKET_ENV, session_socket()),
            (DRY_RUN_ENV, "1".to_owned()),
            (CONFIG_ENV, fixture("routing.js").display().to_string()),
        ]
    }

    /// Connects to a copy the OS started with [`session_env`](Self::session_env).
    /// Quits it when dropped.
    pub fn attach_to_session_router(&self) -> Result<App> {
        fs::create_dir_all(&self.artifacts)?;
        let dir = tempfile::Builder::new().prefix("pwrroute-e2e").tempdir()?;
        let socket = session_socket();
        let deadline = Instant::now() + LAUNCH_TIMEOUT;
        let pid = loop {
            match request(&socket, &json!({ "cmd": "ping" }), IO_TIMEOUT) {
                Ok(answer) => break answer["pid"].as_u64().context("ping without a pid")?,
                Err(e) if Instant::now() >= deadline => {
                    return Err(e.context("no router started by the OS answered"));
                }
                Err(_) => thread::sleep(Duration::from_millis(100)),
            }
        };
        Ok(App {
            harness: self.clone(),
            child: None,
            pid: u32::try_from(pid)?,
            socket,
            env: self.session_env(),
            config: Some(fixture("routing.js")),
            _dir: dir,
            router_dir: None,
            shots: AtomicU32::new(0),
            attached: true,
            launched: Instant::now(),
            answered: Some(Duration::ZERO),
        })
    }
}

/// How to launch the app.
#[derive(Debug, Clone)]
pub struct Launch {
    /// Links on the command line, as a ProgID or `.desktop` launch passes them.
    pub links: Vec<String>,
    /// Keep the router's lock and socket in the test's temp directory. Off
    /// only for disposable-guest scenarios, where a launch the OS starts must
    /// find this router in the usual place.
    pub isolated: bool,
    /// The Finicky config to pass as `--config`. With none the router finds
    /// none either, since home is empty, and the window shows its own rules;
    /// with one it shows the config read-only.
    pub config: Option<PathBuf>,
    /// A `rules.json` to put where the app looks for it in the isolated
    /// home. Defaults to `fixtures/rules.json`, so the Rules screen has rules
    /// to select and edit.
    pub rules: Option<PathBuf>,
}

impl Default for Launch {
    fn default() -> Self {
        Self {
            links: Vec::new(),
            isolated: true,
            config: None,
            rules: Some(fixture("rules.json")),
        }
    }
}

impl Launch {
    /// Routes by `fixtures/routing.js` alone, as the routing tests expect:
    /// no `rules.json`, whose handlers would follow the config's.
    pub fn routing() -> Self {
        Self {
            config: Some(fixture("routing.js")),
            rules: None,
            ..Self::default()
        }
    }
}

/// A fixture file's absolute path.
pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

/// A running copy of the app and its control socket.
pub struct App {
    harness: Harness,
    child: Option<Child>,
    pid: u32,
    socket: String,
    env: Vec<(&'static str, String)>,
    config: Option<PathBuf>,
    _dir: tempfile::TempDir,
    router_dir: Option<PathBuf>,
    shots: AtomicU32,
    /// Started by the OS, not by this harness: quit it rather than kill it.
    attached: bool,
    /// When this harness spawned it, or attached to it.
    launched: Instant,
    /// When its socket first answered, after `launched`.
    answered: Option<Duration>,
}

impl App {
    fn launch(harness: Harness, launch: Launch) -> Result<Self> {
        ensure!(
            harness.executable.is_file(),
            "no app at {}; run `cargo xtask e2e`",
            harness.executable.display()
        );
        ensure!(
            launch.isolated || (launch.config.is_some() && launch.rules.is_none()),
            "a shared launch reads the real home: give it a config, and no rules.json"
        );
        fs::create_dir_all(&harness.artifacts)?;
        // Short: a Unix socket path is limited to about 100 bytes.
        let dir = tempfile::Builder::new().prefix("pwrroute-e2e").tempdir()?;
        let socket = socket_name(dir.path());
        let router_dir = launch.isolated.then(|| dir.path().join("router"));
        let mut env = vec![(SOCKET_ENV, socket.clone())];
        if let Some(router_dir) = &router_dir {
            env.push((ROUTER_DIR_ENV, router_dir.display().to_string()));
            // Local agents are on by default, and their endpoint sits beside
            // the per-user router socket (Linux) or is a per-user pipe
            // (Windows): give this copy its own, or it would take the
            // developer's running PwrRoute's.
            let mcp = if cfg!(windows) {
                format!(r"\\.\pipe\{socket}-mcp")
            } else {
                dir.path().join("mcp.sock").display().to_string()
            };
            env.push((MCP_ENDPOINT_ENV, mcp));
            // An empty home, so the router never finds the developer's
            // Finicky config or rules.json. `env::home_dir` reads HOME, or
            // USERPROFILE on Windows; on Linux the XDG variables win over
            // HOME when set, as on GitHub's runners; on Windows the engine
            // looks under APPDATA, which USERPROFILE does not move. Not for
            // a shared launch:
            // the copy the OS starts has the real home, and must find the
            // same router directory and registration
            // (`.local/share/applications`, `%APPDATA%`).
            let home = dir.path().join("home");
            fs::create_dir(&home)?;
            if cfg!(windows) {
                env.push(("USERPROFILE", home.display().to_string()));
                for (var, sub) in [
                    ("APPDATA", r"AppData\Roaming"),
                    ("LOCALAPPDATA", r"AppData\Local"),
                ] {
                    env.push((var, home.join(sub).display().to_string()));
                }
            } else {
                env.push(("HOME", home.display().to_string()));
                for (var, sub) in [
                    ("XDG_CONFIG_HOME", ".config"),
                    ("XDG_DATA_HOME", ".local/share"),
                ] {
                    env.push((var, home.join(sub).display().to_string()));
                }
            }
            if let Some(rules) = &launch.rules {
                // Where the app will look, by the engine's own rule.
                let var = |key: &str| {
                    env.iter()
                        .find(|(k, _)| *k == key)
                        .map(|(_, v)| PathBuf::from(v))
                };
                let target = discovery::rules_path(&discovery::Environment {
                    home: Some(home.clone()),
                    xdg_config_home: var("XDG_CONFIG_HOME"),
                    app_data: var("APPDATA"),
                })
                .context("no place for rules.json")?;
                fs::create_dir_all(target.parent().context("rules.json has no directory")?)?;
                fs::copy(rules, &target).with_context(|| format!("seed {}", target.display()))?;
            }
        }
        let log_file = fs::File::create(harness.artifacts.join("app.log"))?;
        let mut command = Command::new(&harness.executable);
        command
            .args(router_args(launch.config.as_deref()))
            .args(&launch.links)
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            // One file description for both, so they share an offset and
            // neither overwrites the other.
            .stdout(Stdio::from(log_file.try_clone()?))
            .stderr(Stdio::from(log_file));
        let child = command
            .spawn()
            .with_context(|| format!("launch {}", harness.executable.display()))?;
        let launched = Instant::now();
        let pid = child.id();
        let mut app = Self {
            harness,
            child: Some(child),
            pid,
            socket,
            env,
            config: launch.config,
            _dir: dir,
            router_dir,
            shots: AtomicU32::new(0),
            attached: false,
            launched,
            answered: None,
        };
        app.wait_for_socket()?;
        Ok(app)
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn harness(&self) -> &Harness {
        &self.harness
    }

    /// The isolated router directory, when the launch was isolated.
    pub fn router_dir(&self) -> Option<&Path> {
        self.router_dir.as_deref()
    }

    fn wait_for_socket(&mut self) -> Result<()> {
        let deadline = Instant::now() + LAUNCH_TIMEOUT;
        loop {
            match self.request(&json!({ "cmd": "ping" })) {
                Ok(answer) => {
                    ensure!(
                        answer["pid"] == self.pid,
                        "another process answered on the socket: {answer}"
                    );
                    self.answered = Some(self.launched.elapsed());
                    return Ok(());
                }
                Err(e) if Instant::now() >= deadline => {
                    return Err(e.context(format!(
                        "the app did not open its control socket in {LAUNCH_TIMEOUT:?}; see {}",
                        self.harness.artifacts.join("app.log").display()
                    )));
                }
                Err(_) => {
                    if let Some(status) = self.exited() {
                        bail!(
                            "the app exited during launch ({status}); see {}",
                            self.harness.artifacts.join("app.log").display()
                        );
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }

    /// Sends one request and returns the answer, which must say `ok`.
    pub fn call(&self, request: &Value) -> Result<Value> {
        self.call_within(request, IO_TIMEOUT)
    }

    fn call_within(&self, request: &Value, timeout: Duration) -> Result<Value> {
        let answer = self::request(&self.socket, request, timeout)?;
        ensure!(
            answer["ok"] == true,
            "{request} failed: {}",
            answer["error"].as_str().unwrap_or("no error message")
        );
        Ok(answer)
    }

    /// Sends one request and returns the raw answer.
    pub fn request(&self, request: &Value) -> Result<Value> {
        self::request(&self.socket, request, IO_TIMEOUT)
    }

    /// The app's windows, the main window's painted regions and view state,
    /// and the router's status.
    pub fn state(&self) -> Result<Value> {
        self.call(&json!({ "cmd": "state" }))
    }

    /// Polls [`state`](Self::state) until `check` returns a value. On timeout
    /// the last state is saved to the artifacts and quoted in the error.
    pub fn wait_for<T>(
        &self,
        what: &str,
        timeout: Duration,
        mut check: impl FnMut(&Value) -> Option<T>,
    ) -> Result<T> {
        let deadline = Instant::now() + timeout;
        loop {
            // A UI thread still busy (a cold first launch compiling shaders
            // under software Vulkan) is something to wait out, not a failure.
            let state = match self.state() {
                Ok(state) => state,
                Err(e) if is_busy(&e) && Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
                Err(e) => return Err(e.context(format!("waiting for {what}"))),
            };
            if let Some(found) = check(&state) {
                return Ok(found);
            }
            if Instant::now() >= deadline {
                let dump = self.harness.artifacts.join("state-on-timeout.json");
                let _ = fs::write(&dump, serde_json::to_string_pretty(&state)?);
                self.screenshot("timeout");
                bail!("timed out after {timeout:?} waiting for {what}; last state:\n{state:#}");
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// Waits for the main window to open and paint, and returns its state.
    ///
    /// First the app's startup, through `ping` ([`wait_for_first_paint`](
    /// Self::wait_for_first_paint)), then the window's state from the UI
    /// thread, which by then is serving the hook.
    ///
    /// A launch from a test starts in the background, where another window
    /// may cover it, so a main window the platform reports hidden is brought
    /// forward once, as the Dock icon would. Hidden alone does not fail the
    /// wait: macOS reports a covered window hidden and GPUI still draws it.
    pub fn wait_for_main_window(&self) -> Result<Value> {
        self.wait_for_first_paint()?;
        let state = self.wait_for("the main window to paint", LAUNCH_TIMEOUT, |state| {
            let painted = state["main"]["frames"].as_u64().unwrap_or(0) > 0
                && state["main"]["regions"]["titleStrip"].is_object();
            painted.then(|| state.clone())
        })?;
        if main_visible(&state) {
            return Ok(state);
        }
        self.action("showWindow")?;
        match self.wait_for("the main window to be shown", WAIT, |state| {
            main_visible(state).then(|| state.clone())
        }) {
            Ok(state) => Ok(state),
            Err(_) => {
                eprintln!(
                    "{}: the main window stays hidden after showWindow",
                    self.harness.test
                );
                self.state()
            }
        }
    }

    /// The startup stages the app has reached (`ping`, CONTRACT.md), each
    /// `{"stage": …, "ms": …}` from the top of its `main`.
    pub fn startup(&self) -> Result<Vec<Value>> {
        let answer = self.call(&json!({ "cmd": "ping" }))?;
        Ok(answer["startup"].as_array().cloned().unwrap_or_default())
    }

    /// Waits until the app reports its main window's first frame, asking
    /// with `ping`, which is answered off the UI thread.
    ///
    /// Until then the UI thread is still starting: GPUI's platform, the
    /// window's renderer, the first layout. On a fresh CI machine that takes
    /// anywhere from a second to most of a minute (the first launch only),
    /// and a `state` sent meanwhile only queues behind it. So startup gets
    /// its own budget, [`STARTUP_TIMEOUT`], and a stall says which stage it
    /// stopped at; once the window has painted, a UI thread that does not
    /// answer is a hang and fails on the usual timeouts.
    ///
    /// Where the time went is saved as `startup.json` in the artifacts.
    pub fn wait_for_first_paint(&self) -> Result<()> {
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            let stages = self
                .startup()
                .context("waiting for the main window's first frame")?;
            let painted = stages.iter().any(|s| s["stage"] == "painted");
            if painted || Instant::now() >= deadline {
                let report = json!({
                    "socketMs": self.answered.map(|d| d.as_millis() as u64),
                    "waitedMs": self.launched.elapsed().as_millis() as u64,
                    "stages": stages,
                });
                let _ = fs::write(
                    self.harness.artifacts.join("startup.json"),
                    serde_json::to_string_pretty(&report)?,
                );
                if painted {
                    return Ok(());
                }
                self.screenshot("startup-timeout");
                let last = stages
                    .last()
                    .map_or_else(|| "none".to_owned(), |s| s["stage"].to_string());
                bail!(
                    "the main window had not painted {STARTUP_TIMEOUT:?} after the socket answered; \
                     the last startup stage reached was {last}: {report}; see {}",
                    self.harness.artifacts.join("app.log").display()
                );
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// Types keystrokes into the main window: `"down"`, `"cmd-q"`.
    pub fn keys(&self, keys: &str) -> Result<()> {
        self.call(&json!({ "cmd": "key", "keys": keys })).map(drop)
    }

    /// Clicks the center of a painted region.
    pub fn click(&self, region: &str) -> Result<()> {
        self.call(&json!({ "cmd": "click", "region": region }))
            .map(drop)
    }

    /// Dispatches an app action: `quit`, `closeWindow`, `showWindow`, or a
    /// registration action (CONTRACT.md, "Registration").
    pub fn action(&self, name: &str) -> Result<()> {
        let timeout = match name {
            "registerBrowser" | "setDefaultBrowser" | "unregisterBrowser" => OS_ACTION_TIMEOUT,
            _ => IO_TIMEOUT,
        };
        self.call_within(&json!({ "cmd": "action", "name": name }), timeout)
            .map(drop)
    }

    /// Which app handles web links now (`{"defaultBrowser": bool, …}`).
    pub fn os_state(&self) -> Result<Value> {
        self.call(&json!({ "cmd": "os" }))
    }

    /// The router's recent dispatches, newest first.
    pub fn dispatches(&self) -> Result<Vec<Value>> {
        let answer = self.call(&json!({ "cmd": "dispatches" }))?;
        Ok(answer["dispatches"].as_array().cloned().unwrap_or_default())
    }

    /// Polls the dispatches until one matches `check`.
    pub fn wait_for_dispatch(
        &self,
        what: &str,
        mut check: impl FnMut(&Value) -> bool,
    ) -> Result<Value> {
        let deadline = Instant::now() + WAIT;
        loop {
            let dispatches = self.dispatches()?;
            if let Some(found) = dispatches.iter().find(|d| check(d)) {
                return Ok(found.clone());
            }
            if Instant::now() >= deadline {
                bail!(
                    "timed out waiting for a dispatch of {what}; recent: {}",
                    Value::Array(dispatches)
                );
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// Runs the app again with this one's environment, as a second launch
    /// from a shell would, and waits for it to exit.
    pub fn second_launch(&self, args: &[&str]) -> Result<ExitStatus> {
        let mut child = Command::new(&self.harness.executable)
            .args(router_args(self.config.as_deref()))
            .args(args)
            // Not the hook's socket: the second launch must not serve it.
            .envs(self.env.iter().filter(|(k, _)| *k != SOCKET_ENV).map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(
                fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(self.harness.artifacts.join("second-launch.log"))?,
            ))
            .spawn()?;
        wait_exit(&mut child, WAIT)
    }

    /// Waits for the app to exit, as it should after a quit.
    pub fn wait_exit(&mut self, timeout: Duration) -> Result<ExitStatus> {
        let child = self.child.as_mut().context("the app was already reaped")?;
        let status = wait_exit(child, timeout)?;
        self.child = None;
        Ok(status)
    }

    fn exited(&mut self) -> Option<ExitStatus> {
        self.child.as_mut()?.try_wait().ok().flatten()
    }

    /// Saves a screenshot of the app's window to the artifacts, when
    /// screenshots are on. Best effort: a failure is logged, not raised.
    pub fn screenshot(&self, label: &str) {
        if !self.harness.screenshots {
            return;
        }
        let n = self.shots.fetch_add(1, Ordering::Relaxed);
        let path = self.harness.artifacts.join(format!("{n:02}-{label}.png"));
        if let Err(e) = os::screenshot(self.pid, &path) {
            eprintln!("screenshot {label} failed: {e:#}");
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if self.attached {
            // Attached to a copy the OS started: ask it to go, and wait until
            // it has, so the next launch does not find its router lock held.
            let _ = self.request(&json!({ "cmd": "action", "name": "quit" }));
            let deadline = Instant::now() + WAIT;
            while Instant::now() < deadline && self.request(&json!({ "cmd": "ping" })).is_ok() {
                thread::sleep(Duration::from_millis(100));
            }
            return;
        }
        if let Some(mut child) = self.child.take() {
            if thread::panicking() {
                self.screenshot("failure");
                if let Ok(state) = self.request(&json!({ "cmd": "state" })) {
                    let _ = fs::write(
                        self.harness.artifacts.join("state-on-failure.json"),
                        serde_json::to_string_pretty(&state).unwrap_or_default(),
                    );
                }
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn router_args(config: Option<&Path>) -> Vec<String> {
    let mut args = vec!["--dry-run".to_owned()];
    if let Some(config) = config {
        args.extend(["--config".to_owned(), config.display().to_string()]);
    }
    args
}

fn wait_exit(child: &mut Child, timeout: Duration) -> Result<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("the process did not exit within {timeout:?}; killed it");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// The hook's socket: a file in `dir` on Unix, a named pipe on Windows.
fn socket_name(dir: &Path) -> String {
    if cfg!(windows) {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        format!(
            "pwrroute-e2e-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )
    } else {
        dir.join("e2e.sock").display().to_string()
    }
}

/// Whether a request failed only because the app's UI thread did not get to
/// it in time (`e2e.rs`, `UI_TIMEOUT`).
fn is_busy(error: &anyhow::Error) -> bool {
    format!("{error:#}").contains("the UI thread did not answer in time")
}

/// Whether `state`'s main window is presented. `false` only when the hook
/// says so.
fn main_visible(state: &Value) -> bool {
    state["windows"]
        .as_array()
        .and_then(|w| w.iter().find(|w| w["main"] == true))
        .is_some_and(|w| w["visible"] != false)
}

/// One request on the hook's socket: a line of JSON each way.
fn request(socket: &str, request: &Value, timeout: Duration) -> Result<Value> {
    let conn = connect(socket)?;
    let line = format!("{request}\n");
    // On a thread with a deadline: a named pipe cannot time out a read.
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut conn = BufReader::new(conn);
        let result = conn.get_mut().write_all(line.as_bytes()).and_then(|()| {
            let mut answer = String::new();
            conn.read_line(&mut answer).map(|_| answer)
        });
        let _ = tx.send(result);
    });
    let answer = rx
        .recv_timeout(timeout)
        .map_err(|_| anyhow!("no answer to {request} in {timeout:?}"))??;
    serde_json::from_str(&answer).with_context(|| format!("bad answer {answer:?}"))
}

/// The socket a copy the OS starts serves, per user and fixed, since the
/// harness cannot hand it a temp directory.
fn session_socket() -> String {
    if cfg!(windows) {
        "pwrroute-e2e-session".to_owned()
    } else {
        env::temp_dir()
            .join("pwrroute-e2e-session.sock")
            .display()
            .to_string()
    }
}

fn connect(name: &str) -> Result<Stream> {
    #[cfg(windows)]
    let name = {
        use interprocess::local_socket::GenericNamespaced;
        name.to_owned().to_ns_name::<GenericNamespaced>()?
    };
    #[cfg(not(windows))]
    let name = {
        use interprocess::local_socket::GenericFilePath;
        name.to_owned().to_fs_name::<GenericFilePath>()?
    };
    Ok(ConnectOptions::new().name(name).connect_sync()?)
}

/// A painted region's rectangle: `(x, y, width, height)`.
pub fn rect(region: &Value) -> Option<(f64, f64, f64, f64)> {
    Some((
        region["x"].as_f64()?,
        region["y"].as_f64()?,
        region["width"].as_f64()?,
        region["height"].as_f64()?,
    ))
}

/// The keystroke that quits on this OS, as the app menu binds it.
pub fn quit_keystroke() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd-q"
    } else {
        "ctrl-q"
    }
}
