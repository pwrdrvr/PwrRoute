//! Installs and updates through Velopack.
//!
//! [`run_startup_hooks`] is Velopack's startup logic, which every Velopack
//! app runs before anything else. [`check_in_background`] asks GitHub
//! Releases for a newer version, at startup and from "Check for Updates…",
//! and downloads it into Velopack's package cache. The next launch applies
//! it: `VelopackApp::run` finds the downloaded package, swaps it in and
//! restarts. Until the update design lands, a log line is the whole UI.
//!
//! Only a copy Velopack installed can update. `cargo run`, the .deb and the
//! tar.gz have no Velopack manifest, so their check logs that and stops.
//!
//! What a check found is kept in [`state`] for Settings › Updates, which
//! [`install`] redraws when it changes; nothing is persisted, so a fresh
//! launch starts at "not checked yet".

use std::{
    fmt,
    sync::{
        Mutex, OnceLock, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::SystemTime,
};

use gpui_kit::App;

use velopack::{
    UpdateCheck, UpdateManager, VelopackApp,
    locator::{self, LocationContext},
    sources::GithubSource,
};

/// Where releases are published. `release.yml` uploads Velopack's feed
/// (`releases.<channel>.json` and the `.nupkg` packages) to each release.
const RELEASES: &str = "https://github.com/pwrdrvr/PwrRoute";

/// One check at a time: a second "Check for Updates…" while the startup
/// check is still downloading would only contend for Velopack's lock.
static CHECKING: AtomicBool = AtomicBool::new(false);

/// Velopack's startup logic. Call it first in `main`, before the window.
///
/// Setup, updates and the uninstaller start `pwrroute.exe` with a hook
/// argument; this runs that hook and exits the process. When an update was
/// downloaded on a previous run, this applies it and restarts into the new
/// version. Otherwise it returns at once, also for builds Velopack did not
/// install.
pub fn run_startup_hooks() {
    let mut app = VelopackApp::build();
    // Windows only: Velopack has no install or uninstall step on macOS or
    // Linux. Each hook runs in a process of its own, which Velopack kills
    // after 30 s (install, uninstall) or 15 s (update): no UI, no network.
    // Registering makes PwrRoute one of the browsers the user can pick; it
    // never makes PwrRoute the default.
    #[cfg(target_os = "windows")]
    {
        use crate::browser_registration::{register_browser, unregister_browser};
        app = app
            .on_after_install_fast_callback(|_| register_browser())
            // An update may change what is registered, so it re-registers.
            .on_after_update_fast_callback(|_| register_browser())
            .on_before_uninstall_fast_callback(|_| unregister_browser());
    }
    app.run();
}

/// What asked for the check, for the log line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Startup,
    Menu,
    Settings,
}

/// What the updater has said so far this launch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// A check is running now.
    pub checking: bool,
    /// The last check that finished, and when.
    pub last: Option<(SystemTime, Outcome)>,
}

static STATE: Mutex<State> = Mutex::new(State {
    checking: false,
    last: None,
});

/// Wakes [`install`]'s task after every change to [`state`].
static CHANGED: OnceLock<async_channel::Sender<()>> = OnceLock::new();

/// The updater's state as it stands.
pub fn state() -> State {
    STATE.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

fn set_state(edit: impl FnOnce(&mut State)) {
    edit(&mut STATE.lock().unwrap_or_else(PoisonError::into_inner));
    if let Some(changed) = CHANGED.get() {
        let _ = changed.try_send(());
    }
}

/// Redraws Settings › Updates whenever a check starts or finishes. Call
/// once, after `settings` is installed.
pub fn install(cx: &mut App) {
    let (tx, rx) = async_channel::unbounded::<()>();
    if CHANGED.set(tx).is_err() {
        return;
    }
    cx.spawn(async move |cx| {
        while rx.recv().await.is_ok() {
            cx.update(|cx| {
                let updates = crate::settings::updates_host::model();
                crate::settings::update(cx, move |model| model.updates = updates);
            });
        }
    })
    .detach();
}

/// Where this copy came from, as far as Velopack can tell without the
/// network: its manifest version, or why there is none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Installation {
    /// Velopack installed this copy at `version`; it can update itself.
    Velopack { version: String },
    /// No Velopack manifest: `cargo run`, the .deb or the tar.gz.
    NotInstalled { why: String },
}

/// Reads the manifest beside the executable. Local files only.
pub fn installation() -> Installation {
    match locator::auto_locate_app_manifest(LocationContext::FromCurrentExe) {
        Ok(locator) => Installation::Velopack {
            version: locator.get_manifest_version().to_string(),
        },
        Err(error) => Installation::NotInstalled {
            why: error.to_string(),
        },
    }
}

/// Which releases a version follows: see [`includes_prereleases`].
pub fn channel(version: &str) -> &'static str {
    if includes_prereleases(version) {
        "prerelease"
    } else {
        "stable"
    }
}

/// Check for an update, and download it, on a thread of its own: Velopack's
/// client blocks, and a download can take a while.
pub fn check_in_background(trigger: Trigger) {
    if CHECKING.swap(true, Ordering::AcqRel) {
        log(trigger, &Outcome::AlreadyChecking);
        return;
    }
    set_state(|state| state.checking = true);
    let spawned = thread::Builder::new()
        .name("pwrroute-updater".to_owned())
        .spawn(move || {
            let outcome = check_and_download();
            CHECKING.store(false, Ordering::Release);
            log(trigger, &outcome);
            set_state(|state| {
                state.checking = false;
                state.last = Some((SystemTime::now(), outcome));
            });
        });
    if let Err(error) = spawned {
        CHECKING.store(false, Ordering::Release);
        let outcome = Outcome::Failed(error.to_string());
        log(trigger, &outcome);
        set_state(|state| {
            state.checking = false;
            state.last = Some((SystemTime::now(), outcome));
        });
    }
}

/// Installs the downloaded release and starts PwrRoute again: Settings'
/// "Restart to update". Velopack swaps the package and relaunches, so this
/// returns only when there is nothing to apply or it could not start.
pub fn restart_to_update() -> Result<(), String> {
    // The manager locates the manifest itself and refuses a copy Velopack
    // did not install; the feed is never read here.
    let source = GithubSource::new(RELEASES, None, false);
    let manager = UpdateManager::new(source, None, None).map_err(|e| e.to_string())?;
    let pending = manager
        .get_update_pending_restart()
        .ok_or_else(|| "no downloaded update to install".to_owned())?;
    manager
        .apply_updates_and_restart(&pending)
        .map_err(|e| e.to_string())
}

fn check_and_download() -> Outcome {
    // The installed version (Velopack's sq.version), not the one compiled
    // in: packages are stamped separately from Cargo.toml.
    let installed = match locator::auto_locate_app_manifest(LocationContext::FromCurrentExe) {
        Ok(locator) => locator.get_manifest_version().to_string(),
        Err(error) => return Outcome::NotInstalled(error.to_string()),
    };
    let source = GithubSource::new(RELEASES, None, includes_prereleases(&installed));
    let manager = match UpdateManager::new(source, None, None) {
        Ok(manager) => manager,
        Err(error) => return Outcome::NotInstalled(error.to_string()),
    };
    if let Some(pending) = manager.get_update_pending_restart() {
        return Outcome::Pending(pending.Version);
    }
    match manager.check_for_updates() {
        Ok(UpdateCheck::UpdateAvailable(update)) => {
            let version = update.TargetFullRelease.Version.clone();
            match manager.download_updates(&update, None) {
                Ok(()) => Outcome::Downloaded(version),
                Err(error) => Outcome::Failed(error.to_string()),
            }
        }
        Ok(UpdateCheck::NoUpdateAvailable | UpdateCheck::RemoteIsEmpty) => {
            Outcome::UpToDate(manager.get_current_version_as_string())
        }
        Err(error) => Outcome::Failed(error.to_string()),
    }
}

/// Releases are born as GitHub Pre-releases and promoted to Latest by hand.
/// A prerelease install follows every release; a stable install only
/// promoted ones.
fn includes_prereleases(version: &str) -> bool {
    version.contains('-')
}

/// How a check went. The version strings are Velopack's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    AlreadyChecking,
    NotInstalled(String),
    UpToDate(String),
    Downloaded(String),
    Pending(String),
    Failed(String),
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyChecking => write!(f, "a check is already running"),
            Self::NotInstalled(why) => {
                write!(f, "not a Velopack install, so no updates here ({why})")
            }
            Self::UpToDate(version) => write!(f, "{version} is the latest"),
            Self::Downloaded(version) => {
                write!(
                    f,
                    "downloaded {version}; it installs when PwrRoute restarts"
                )
            }
            Self::Pending(version) => {
                write!(
                    f,
                    "{version} is downloaded and installs when PwrRoute restarts"
                )
            }
            Self::Failed(why) => write!(f, "check failed: {why}"),
        }
    }
}

fn log(trigger: Trigger, outcome: &Outcome) {
    let trigger = match trigger {
        Trigger::Startup => "startup",
        Trigger::Menu => "menu",
        Trigger::Settings => "settings",
    };
    eprintln!("pwrroute: updates ({trigger}): {outcome}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prerelease_builds_follow_prereleases() {
        assert!(includes_prereleases("0.0.1-alpha.0"));
        assert!(includes_prereleases("1.2.0-beta.3"));
        assert!(!includes_prereleases("1.2.0"));
        assert_eq!(channel("0.0.1-alpha.0"), "prerelease");
        assert_eq!(channel("1.2.0"), "stable");
    }

    #[test]
    fn a_dev_build_has_no_installation_and_nothing_to_restart_into() {
        assert!(matches!(installation(), Installation::NotInstalled { .. }));
        assert!(restart_to_update().is_err());
    }

    #[test]
    fn a_dev_build_reports_it_is_not_installed() {
        // The test binary is not in a Velopack layout, so the manager cannot
        // load and nothing reaches the network.
        assert!(matches!(check_and_download(), Outcome::NotInstalled(_)));
    }

    #[test]
    fn outcomes_read_as_log_lines() {
        assert_eq!(
            Outcome::AlreadyChecking.to_string(),
            "a check is already running"
        );
        assert_eq!(
            Outcome::Downloaded("0.0.2".to_owned()).to_string(),
            "downloaded 0.0.2; it installs when PwrRoute restarts"
        );
    }
}
