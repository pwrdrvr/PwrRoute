//! Opening a URL in the browser the engine chose.
//!
//! The macOS command line is Finicky's (MIT, Copyright (c) 2015-2025 John
//! Sterling): `open -a NAME` or `open -b BUNDLE_ID`, `-g` to stay in the
//! background, and `-n --args` with the profile flag, because `open` drops
//! arguments when the app is already running unless it starts a new
//! instance (which then hands the URL to the running one).
//!
//! Windows and Linux run the browser's executable directly, never "the
//! default browser": that is PwrRoute, and the link would loop back.

use std::{
    fmt,
    io::Read as _,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

use pwrroute_engine::{AppType, Browser};

use crate::{
    catalog::{BrowserInfo, Catalog, PrivateFlag},
    desktop_entry, detect,
    env::{Env, Platform},
    profiles::{self, ProfileMatch},
    registry,
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

/// Plans and runs browser launches against one [`Env`].
#[derive(Debug, Clone)]
pub struct Launcher {
    env: Env,
    options: LaunchOptions,
}

/// The exact command a launch runs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Plan {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// What the command could not honor. Never contains the URL.
    pub notes: Vec<Note>,
    platform: Platform,
}

/// Something about a launch worth logging: the URL still opens.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Note {
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
        Self { env, options }
    }

    pub fn env(&self) -> &Env {
        &self.env
    }

    /// The dry run: the command [`Launcher::launch`] would run for
    /// `browser`, without running it. `Ok(None)` for `AppType::None`
    /// (`browser: null` in the config): do not open the URL at all.
    pub fn plan(&self, browser: &Browser) -> Result<Option<Plan>, LaunchError> {
        self.plan_in(browser, requests_private(browser))
    }

    /// [`Launcher::plan`] with the private-window request spelled out, so
    /// it can be exercised before the engine carries it.
    fn plan_in(&self, browser: &Browser, private: bool) -> Result<Option<Plan>, LaunchError> {
        if browser.app_type == AppType::None {
            return Ok(None);
        }
        let catalog = Catalog::builtin();
        let platform = self.env.platform();
        let entry = match browser.app_type {
            AppType::Path => catalog.find_by_path(platform, Path::new(&browser.name)),
            _ => catalog.find(&browser.name),
        };
        let mut notes = Vec::new();
        let mut flags = self.profile_args(entry, &browser.profile, &mut notes);
        if private && let Some(flag) = private_flag(browser, entry, &mut notes) {
            flags.get_or_insert_default().push(flag.to_owned());
        }
        let background = browser
            .open_in_background
            .unwrap_or(self.options.background_by_default);
        let plan = match platform {
            Platform::MacOs => macos_plan(browser, background, flags),
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
            platform,
        }))
    }

    /// Runs [`Launcher::plan`]'s command and returns it. On macOS this waits
    /// for `open` (which returns once the app has the URL) so its errors are
    /// reported; elsewhere the browser is started and left running.
    pub fn launch(&self, browser: &Browser) -> Result<Option<Plan>, LaunchError> {
        let Some(plan) = self.plan(browser)? else {
            return Ok(None);
        };
        spawn(&plan)?;
        Ok(Some(plan))
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
            ProfileMatch::Found(args) => Some(args),
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

/// Whether the rule that chose `browser` asked for a private window.
///
/// `pwrroute_engine::Browser` is gaining `pub private: bool` (set by the
/// engine from `rules[].private`, or from a known private flag in a JS
/// config's `args`). Until that field lands this is `false`, and only args
/// that already carry the flag open a private window; flip the body to
/// `browser.private` when it does.
fn requests_private(_browser: &Browser) -> bool {
    false
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

/// `flags` are the profile and private-window flags, which must follow
/// `--args` (and need `-n`: `open` drops arguments for a running app
/// unless it starts a new instance).
fn macos_plan(
    browser: &Browser,
    background: bool,
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
    if background {
        args.push("-g".into());
    }
    if flags.is_some() {
        args.push("-n".into());
    }
    match (flags, browser.args.is_empty()) {
        (None, true) => args.push(browser.url.clone()),
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
    fn command_line_keeps_backslashes() {
        let plan = Plan {
            program: PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe"),
            args: vec![
                "--profile-directory=Profile 1".into(),
                r#"say "hi""#.into(),
                "ü".into(),
            ],
            notes: Vec::new(),
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

    fn private_plan(launcher: &Launcher, b: &Browser) -> Plan {
        launcher.plan_in(b, true).unwrap().expect("a plan")
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
    fn private_flag_is_not_added_twice() {
        let (_dir, launcher) = fixture(Platform::MacOs, &[]);
        let mut b = browser("Google Chrome", AppType::AppName);
        b.args = ["--incognito", URL].map(String::from).to_vec();
        let p = private_plan(&launcher, &b);
        assert_eq!(
            args(&p),
            ["-a", "Google Chrome", "--args", "--incognito", URL]
        );
        assert!(p.notes.is_empty());

        // A config's own `--args`, with the flag after it.
        b.args = ["-F", "--args", "--incognito", URL]
            .map(String::from)
            .to_vec();
        let p = private_plan(&launcher, &b);
        assert_eq!(
            args(&p),
            ["-a", "Google Chrome", "-F", "--args", "--incognito", URL]
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
            ["-a", "Microsoft Edge", "--args", "--inprivate", URL]
        );

        // Without the request, args are passed through as they are.
        let mut b = browser("Microsoft Edge", AppType::AppName);
        b.args = ["--incognito", URL].map(String::from).to_vec();
        let p = launcher.plan_in(&b, false).unwrap().unwrap();
        assert_eq!(
            args(&p),
            ["-a", "Microsoft Edge", "--args", "--incognito", URL]
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

    /// Until the engine's `Browser` carries `private`, a plan never asks
    /// for a private window on its own.
    #[test]
    fn plan_reads_the_request_from_the_browser() {
        let b = browser("Google Chrome", AppType::AppName);
        assert!(!requests_private(&b));
        let (_dir, launcher) = fixture(Platform::MacOs, &[]);
        let p = launcher.plan(&b).unwrap().unwrap();
        assert_eq!(args(&p), ["-a", "Google Chrome", URL]);
    }

    #[test]
    fn spawn_errors_name_the_program() {
        let plan = Plan {
            program: PathBuf::from("/nonexistent/browser"),
            args: Vec::new(),
            notes: Vec::new(),
            platform: Platform::Linux,
        };
        let err = spawn(&plan).unwrap_err();
        assert!(matches!(err, LaunchError::Spawn { .. }), "{err}");
    }
}
