//! Installs and updates through Velopack.
//!
//! [`run_startup_hooks`] is Velopack's startup logic, which every Velopack
//! app runs before anything else. [`check_in_background`] asks GitHub
//! Releases for a newer version, at startup, every [`CHECK_EVERY`] and from
//! "Check for Updates…", and downloads it into Velopack's package cache.
//!
//! A downloaded package is never applied at launch. PwrRoute is resident
//! and is launched by a click on a link: Velopack's default swaps the
//! package and restarts the process before AppKit delivers that link, so
//! the click opens nothing and the relaunch pops the window. Instead the
//! package is applied when the user quits ([`apply_pending_on_quit`], from
//! the app menu and the tray) or asks for it (Settings' Restart to update,
//! [`restart_to_update`]).
//!
//! Only a copy Velopack installed can update. `cargo run`, the .deb and the
//! tar.gz have no Velopack manifest, so their check logs that and stops.
//!
//! Which release: the slot of the release channel in force
//! ([`crate::update_channel`]), the Pwr family's Stable or Beta × Latest or
//! Prerelease. A check reads the published list
//! ([`crate::github_releases`]), picks the slot's release, and points
//! Velopack at that one release's files. A release older than this copy is
//! a switch back: only Check now (the menu, Settings) downloads one, and it
//! installs only from Restart to switch, never on quit. A download is
//! recorded with its slot ([`Held`], `updates.json` in the data directory);
//! after another slot is picked it is neither shown nor installed.
//!
//! What a check found is kept in [`state`] for Settings › Updates, which
//! [`install`] redraws when it changes; apart from the held download nothing
//! is persisted, so a fresh launch starts at "not checked yet".

use std::{
    cmp::Ordering as Precedence,
    fmt, fs,
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, SystemTime},
};

use gpui_kit::App;
use serde::{Deserialize, Serialize};

use velopack::{
    UpdateCheck, UpdateManager, UpdateOptions, VelopackApp, VelopackAsset,
    locator::{self, LocationContext, VelopackLocator},
    sources::{HttpSource, NoneSource},
};

use crate::github_releases;
use crate::update_channel::{self, Release, Slot, UpdateSettings};

/// The held download's record, in the data directory.
const HELD_FILE: &str = "updates.json";

/// How often a resident router checks again after the startup check: the
/// design's "At startup and every 6 hours while running". A Velopack
/// install only; a dev build has nothing to check.
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// One check at a time: a second "Check for Updates…" while the startup
/// check is still downloading would only contend for Velopack's lock.
static CHECKING: AtomicBool = AtomicBool::new(false);

/// What [`run_startup_hooks`] tells Velopack, apart from the hooks. Kept
/// as data so a test can hold the policy without a Velopack install.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartupPolicy {
    /// Apply a downloaded package at launch, swapping it in and restarting
    /// the process. Velopack's default is `true`; PwrRoute's is `false`,
    /// since a launch is usually for a link (see the module docs).
    pub auto_apply: bool,
}

/// The policy PwrRoute runs with.
pub const STARTUP: StartupPolicy = StartupPolicy { auto_apply: false };

/// Velopack's startup logic. Call it first in `main`, before the window.
///
/// Setup, updates and the uninstaller start `pwrroute.exe` with a hook
/// argument; this runs that hook and exits the process. Otherwise it
/// returns at once, also for builds Velopack did not install, and also
/// when an update was downloaded on a previous run: that waits for Quit or
/// Restart to update ([`STARTUP`]).
pub fn run_startup_hooks() {
    startup_hooks(STARTUP).run();
}

/// The builder behind [`run_startup_hooks`]: `policy` and the hooks.
fn startup_hooks(policy: StartupPolicy) -> VelopackApp<'static> {
    let app = VelopackApp::build().set_auto_apply_on_startup(policy.auto_apply);
    // Windows only: Velopack has no install or uninstall step on macOS or
    // Linux. Each hook runs in a process of its own, which Velopack kills
    // after 30 s (install, uninstall) or 15 s (update): no UI, no network.
    // Registering makes PwrRoute one of the browsers the user can pick; it
    // never makes PwrRoute the default.
    #[cfg(target_os = "windows")]
    let app = {
        use crate::browser_registration::{register_browser, unregister_browser};
        app.on_after_install_fast_callback(|_| register_browser())
            // An update may change what is registered, so it re-registers.
            .on_after_update_fast_callback(|_| register_browser())
            .on_before_uninstall_fast_callback(|_| unregister_browser())
    };
    app
}

/// What quitting did about a downloaded update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuitApply {
    /// Nothing downloaded, or not a Velopack install: a plain quit.
    Nothing,
    /// Velopack's updater is waiting for this process to exit, and then
    /// swaps `version` in without starting PwrRoute again.
    Applying(String),
    /// A package was downloaded but the updater could not be started; it
    /// stays downloaded for the next quit or Restart to update.
    Failed(String),
}

/// Applies a downloaded update once this process has exited. Call right
/// before `cx.quit()` from every Quit: the app menu and the tray. The
/// updater waits for the exit on its own, so this returns at once and the
/// quit goes on; PwrRoute is not started again, and the next link opens
/// the new version.
pub fn apply_pending_on_quit() {
    match apply_pending() {
        QuitApply::Nothing => {}
        QuitApply::Applying(version) => {
            eprintln!("pwrroute: updates: installing {version} after this quit");
        }
        QuitApply::Failed(why) => {
            eprintln!("pwrroute: updates: could not install the downloaded update: {why}");
        }
    }
}

fn apply_pending() -> QuitApply {
    // The manager locates the manifest itself and refuses a copy Velopack
    // did not install; no feed is read here.
    let Ok(locator) = locator::auto_locate_app_manifest(LocationContext::FromCurrentExe) else {
        return QuitApply::Nothing;
    };
    let manager = match UpdateManager::new(NoneSource {}, None, None) {
        Ok(manager) => manager,
        Err(_) => return QuitApply::Nothing,
    };
    let installed = locator.get_manifest_version().to_string();
    let slot = selection().resolve(&installed);
    let held = load_held().filter(|held| package_exists(&locator, held));
    let pending = match &held {
        Some(held) => quit_package(&installed, slot, held).map(Held::asset),
        // Nothing recorded: a package an earlier build downloaded, which
        // only ever fetched its own channel's newer releases.
        None => manager.get_update_pending_restart(),
    };
    on_quit(pending, |pending| {
        manager
            // Silent: no progress window over a quit, and no restart.
            .wait_exit_then_apply_updates(pending, true, false, Vec::<String>::new())
            .map_err(|e| e.to_string())
    })
}

/// The quit path's branch: `apply` runs only for a downloaded package.
fn on_quit(
    pending: Option<VelopackAsset>,
    apply: impl FnOnce(&VelopackAsset) -> Result<(), String>,
) -> QuitApply {
    let Some(pending) = pending else {
        return QuitApply::Nothing;
    };
    match apply(&pending) {
        Ok(()) => QuitApply::Applying(pending.Version),
        Err(why) => QuitApply::Failed(why),
    }
}

/// What asked for the check, for the log line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Startup,
    /// Every [`CHECK_EVERY`] while running.
    Timer,
    Menu,
    Settings,
    /// Another slot was picked in Settings › Updates.
    Selection,
}

impl Trigger {
    /// Check now, from the menu or Settings: the one check that may switch
    /// back to an older release.
    pub fn is_manual(self) -> bool {
        matches!(self, Self::Menu | Self::Settings)
    }
}

/// What the updater has said so far this launch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// A check is running now.
    pub checking: bool,
    /// The last check that finished, and when.
    pub last: Option<(SystemTime, Outcome)>,
    /// The published releases the last check read, or why it could not.
    pub releases: Option<Result<Vec<Release>, String>>,
    /// The package downloaded and not installed yet, and its slot.
    pub held: Option<Held>,
}

static STATE: Mutex<State> = Mutex::new(State {
    checking: false,
    last: None,
    releases: None,
    held: None,
});

/// The release channel as `settings.json` has it; [`set_selection`] keeps
/// it current. `None` until the settings are read: the defaults.
static SELECTION: Mutex<Option<UpdateSettings>> = Mutex::new(None);

/// The release channel as `settings.json` has it.
pub fn selection() -> UpdateSettings {
    SELECTION
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .unwrap_or_default()
}

/// Follows `settings` from now on. When the slot in force changes, the
/// new slot is checked at once, forward only, and the window redrawn: a
/// download held for the old slot is neither shown nor installed now.
pub fn set_selection(settings: UpdateSettings) {
    let installed = installed_version();
    let before = selection().resolve(&installed);
    *SELECTION.lock().unwrap_or_else(PoisonError::into_inner) = Some(settings);
    if settings.resolve(&installed) != before {
        check_in_background(Trigger::Selection);
    }
    set_state(|_| {});
}

/// The version this copy reports: Velopack's manifest, else the crate's.
pub fn installed_version() -> String {
    match installation() {
        Installation::Velopack { version } => version,
        Installation::NotInstalled { .. } => env!("CARGO_PKG_VERSION").to_owned(),
    }
}

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

/// Redraws Settings › Updates whenever a check starts or finishes, and
/// checks again every [`CHECK_EVERY`] on a Velopack install. Call once,
/// after `settings` is installed.
pub fn install(cx: &mut App) {
    let (tx, rx) = async_channel::unbounded::<()>();
    if CHANGED.set(tx).is_err() {
        return;
    }
    if let Some(store) = crate::settings_store::store(cx) {
        let settings = store.read(cx).settings().updates;
        *SELECTION.lock().unwrap_or_else(PoisonError::into_inner) = Some(settings);
    }
    if let Some(held) = load_held() {
        set_state(|state| state.held = Some(held));
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
    if let Some(every) = check_every(&installation()) {
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(every).await;
                check_in_background(Trigger::Timer);
            }
        })
        .detach();
    }
}

/// How often `installation` checks again on its own: [`CHECK_EVERY`] for
/// a Velopack install, never for a copy that cannot update itself.
pub fn check_every(installation: &Installation) -> Option<Duration> {
    match installation {
        Installation::Velopack { .. } => Some(CHECK_EVERY),
        Installation::NotInstalled { .. } => None,
    }
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
            let outcome = check_and_download(trigger);
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
/// "Restart to update" (or "Restart to switch", for an older release).
/// Velopack swaps the package and relaunches, so this returns only when
/// there is nothing to apply or it could not start.
pub fn restart_to_update() -> Result<(), String> {
    let locator = locator::auto_locate_app_manifest(LocationContext::FromCurrentExe)
        .map_err(|e| e.to_string())?;
    // The manager refuses a copy Velopack did not install; no feed is read.
    let manager = UpdateManager::new(NoneSource {}, None, None).map_err(|e| e.to_string())?;
    let installed = locator.get_manifest_version().to_string();
    let slot = selection().resolve(&installed);
    let pending = match load_held().filter(|held| package_exists(&locator, held)) {
        Some(held) => restart_package(&installed, slot, &held)
            .map(Held::asset)
            .ok_or_else(|| "the downloaded update is not for the selected channel".to_owned())?,
        None => manager
            .get_update_pending_restart()
            .ok_or_else(|| "no downloaded update to install".to_owned())?,
    };
    manager
        .apply_updates_and_restart(&pending)
        .map_err(|e| e.to_string())
}

/// A downloaded package and the slot it was downloaded for. Velopack keeps
/// one package at a time (a download deletes the others), so one record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Held {
    /// Velopack's version: "0.1.1".
    pub version: String,
    /// The full package's file in Velopack's packages directory.
    pub file_name: String,
    /// The slot's key ([`Slot::key`]): "stable:latest".
    pub slot: String,
    /// Older than the copy that downloaded it: installed only from Restart
    /// to switch.
    #[serde(default)]
    pub switch_back: bool,
}

impl Held {
    /// Held for `slot`.
    pub fn is_for(&self, slot: Slot) -> bool {
        Slot::from_key(&self.slot) == Some(slot)
    }

    /// The package, as Velopack's apply takes it: by its file name.
    fn asset(&self) -> VelopackAsset {
        VelopackAsset {
            Version: self.version.clone(),
            FileName: self.file_name.clone(),
            Type: "Full".to_owned(),
            ..VelopackAsset::default()
        }
    }
}

/// What quitting installs: the held package when it is for `slot`, newer
/// than `installed` and not a switch back.
fn quit_package<'a>(installed: &str, slot: Slot, held: &'a Held) -> Option<&'a Held> {
    let newer = matches!(compare(&held.version, installed), Some(Precedence::Greater));
    (held.is_for(slot) && newer && !held.switch_back).then_some(held)
}

/// What Restart to update installs: the held package when it is for
/// `slot` and not this very version.
fn restart_package<'a>(installed: &str, slot: Slot, held: &'a Held) -> Option<&'a Held> {
    let other = !matches!(compare(&held.version, installed), Some(Precedence::Equal));
    (held.is_for(slot) && other).then_some(held)
}

/// `a` against `b` by SemVer precedence; `None` when either does not parse.
fn compare(a: &str, b: &str) -> Option<Precedence> {
    Some(update_channel::parse(a)?.cmp(&update_channel::parse(b)?))
}

/// What a check does once it knows the slot's release.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Plan {
    /// Nothing to do: `Outcome` says why.
    Done(Outcome),
    /// Download `tag`; `switch_back` for an older release.
    Download { tag: String, switch_back: bool },
}

/// The check's decision, apart from Velopack and the network: `target` is
/// the slot's release, `held` what is downloaded already.
fn plan(
    installed: &str,
    slot: Slot,
    target: Option<&Release>,
    held: Option<&Held>,
    trigger: Trigger,
) -> Plan {
    let Some(target) = target else {
        return Plan::Done(Outcome::NothingPublished(slot.to_string()));
    };
    let version = target.tag.trim_start_matches('v').to_owned();
    let held = held.filter(|held| held.is_for(slot));
    // A switch back already downloaded stays until Restart to switch; only
    // Check now looks past it.
    if let Some(held) = held.filter(|held| held.switch_back && !trigger.is_manual()) {
        return Plan::Done(Outcome::SwitchReady(held.version.clone()));
    }
    match compare(&version, installed) {
        // A tag that does not parse is never taken for an older release.
        None | Some(Precedence::Equal) => Plan::Done(Outcome::UpToDate(installed.to_owned())),
        Some(Precedence::Less) if !trigger.is_manual() => {
            Plan::Done(Outcome::UpToDate(installed.to_owned()))
        }
        Some(Precedence::Less) => match held {
            Some(held) if held.switch_back && held.version == version => {
                Plan::Done(Outcome::SwitchReady(held.version.clone()))
            }
            _ => Plan::Download {
                tag: target.tag.clone(),
                switch_back: true,
            },
        },
        Some(Precedence::Greater) => match held {
            // As new or newer is downloaded already for this slot.
            Some(held)
                if !held.switch_back
                    && matches!(
                        compare(&held.version, &version),
                        Some(Precedence::Greater | Precedence::Equal)
                    ) =>
            {
                Plan::Done(Outcome::Pending(held.version.clone()))
            }
            _ => Plan::Download {
                tag: target.tag.clone(),
                switch_back: false,
            },
        },
    }
}

fn check_and_download(trigger: Trigger) -> Outcome {
    // The installed version (Velopack's sq.version), not the one compiled
    // in: packages are stamped separately from Cargo.toml.
    let locator = match locator::auto_locate_app_manifest(LocationContext::FromCurrentExe) {
        Ok(locator) => locator,
        Err(error) => return Outcome::NotInstalled(error.to_string()),
    };
    let installed = locator.get_manifest_version().to_string();
    let slot = selection().resolve(&installed);
    let releases = github_releases::fetch(&locator.get_manifest_channel());
    set_state(|state| state.releases = Some(releases.clone()));
    let releases = match releases {
        Ok(releases) => releases,
        Err(error) => return Outcome::Failed(error),
    };
    let target = update_channel::select(&releases, slot);
    let held = load_held().filter(|held| package_exists(&locator, held));
    // A package Velopack no longer has is not ready to install.
    set_state(|state| state.held = held.clone());
    let (tag, switch_back) = match plan(&installed, slot, target.as_ref(), held.as_ref(), trigger) {
        Plan::Done(outcome) => return outcome,
        Plan::Download { tag, switch_back } => (tag, switch_back),
    };
    let options = UpdateOptions {
        AllowVersionDowngrade: switch_back,
        ..UpdateOptions::default()
    };
    let source = HttpSource::new(github_releases::download_base(&tag));
    let manager = match UpdateManager::new(source, Some(options), None) {
        Ok(manager) => manager,
        Err(error) => return Outcome::NotInstalled(error.to_string()),
    };
    match manager.check_for_updates() {
        Ok(UpdateCheck::UpdateAvailable(update)) => {
            let target = &update.TargetFullRelease;
            if let Err(error) = manager.download_updates(&update, None) {
                return Outcome::Failed(error.to_string());
            }
            let held = Held {
                version: target.Version.clone(),
                file_name: target.FileName.clone(),
                slot: slot.key(),
                switch_back,
            };
            save_held(&held);
            set_state(|state| state.held = Some(held.clone()));
            if switch_back {
                Outcome::SwitchReady(held.version)
            } else {
                Outcome::Downloaded(held.version)
            }
        }
        Ok(UpdateCheck::NoUpdateAvailable | UpdateCheck::RemoteIsEmpty) => {
            Outcome::UpToDate(installed)
        }
        Err(error) => Outcome::Failed(error.to_string()),
    }
}

/// Whether `held`'s package is still in Velopack's packages directory.
fn package_exists(locator: &VelopackLocator, held: &Held) -> bool {
    locator.get_packages_dir().join(&held.file_name).is_file()
}

fn held_path() -> Option<PathBuf> {
    pwrroute_os::data_dir().map(|dir| dir.join(HELD_FILE))
}

/// The file's shape: room for more than the held download later.
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct HeldFile {
    held: Option<Held>,
}

fn load_held() -> Option<Held> {
    read_held(&held_path()?)
}

fn read_held(path: &Path) -> Option<Held> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str::<HeldFile>(&text).ok()?.held
}

fn save_held(held: &Held) {
    let Some(path) = held_path() else {
        return;
    };
    if let Err(error) = write_held(&path, held) {
        eprintln!("pwrroute: updates: could not record the download: {error}");
    }
}

fn write_held(path: &Path, held: &Held) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let file = HeldFile {
        held: Some(held.clone()),
    };
    let mut text = serde_json::to_string_pretty(&file).map_err(std::io::Error::other)?;
    text.push('\n');
    fs::write(path, text)
}

/// How a check went. The version strings are Velopack's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    AlreadyChecking,
    NotInstalled(String),
    UpToDate(String),
    /// The slot (by name) has no release published yet.
    NothingPublished(String),
    Downloaded(String),
    Pending(String),
    /// An older release is downloaded; Restart to switch installs it.
    SwitchReady(String),
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
                    "downloaded {version}; it installs when PwrRoute quits or restarts"
                )
            }
            Self::Pending(version) => {
                write!(
                    f,
                    "{version} is downloaded and installs when PwrRoute quits or restarts"
                )
            }
            Self::NothingPublished(slot) => write!(f, "nothing is published on {slot} yet"),
            Self::SwitchReady(version) => {
                write!(f, "{version} is downloaded; Restart to switch installs it")
            }
            Self::Failed(why) => write!(f, "check failed: {why}"),
        }
    }
}

fn log(trigger: Trigger, outcome: &Outcome) {
    let trigger = match trigger {
        Trigger::Startup => "startup",
        Trigger::Timer => "timer",
        Trigger::Menu => "menu",
        Trigger::Settings => "settings",
        Trigger::Selection => "channel",
    };
    eprintln!("pwrroute: updates ({trigger}): {outcome}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dev_build_has_no_installation_and_nothing_to_restart_into() {
        assert!(matches!(installation(), Installation::NotInstalled { .. }));
        assert!(restart_to_update().is_err());
    }

    #[test]
    fn startup_never_applies_a_downloaded_update() {
        // A launch is usually for a link: the package waits for Quit or
        // Restart to update. The builder takes the policy as given; a
        // Velopack install is needed to see it act, so the policy is what
        // the test can hold.
        assert_eq!(STARTUP, StartupPolicy { auto_apply: false });
        // The builder is made, hooks and all, without a Velopack install.
        let _off = startup_hooks(STARTUP);
        let _on = startup_hooks(StartupPolicy { auto_apply: true });
    }

    #[test]
    fn quitting_with_nothing_downloaded_applies_nothing() {
        let mut applied = 0;
        let outcome = on_quit(None, |_| {
            applied += 1;
            Ok(())
        });
        assert_eq!(outcome, QuitApply::Nothing);
        assert_eq!(applied, 0);
        // A test binary is not a Velopack install: the real path says the
        // same, without starting an updater.
        assert_eq!(apply_pending(), QuitApply::Nothing);
    }

    #[test]
    fn quitting_with_a_download_hands_it_to_the_updater() {
        let asset = || VelopackAsset {
            Version: "0.2.1".to_owned(),
            ..VelopackAsset::default()
        };
        assert_eq!(
            on_quit(Some(asset()), |pending| {
                assert_eq!(pending.Version, "0.2.1");
                Ok(())
            }),
            QuitApply::Applying("0.2.1".to_owned())
        );
        assert_eq!(
            on_quit(Some(asset()), |_| Err("no updater".to_owned())),
            QuitApply::Failed("no updater".to_owned())
        );
    }

    #[test]
    fn only_a_velopack_install_checks_on_a_timer() {
        assert_eq!(
            check_every(&Installation::Velopack {
                version: "0.2.0".to_owned()
            }),
            Some(CHECK_EVERY)
        );
        assert_eq!(
            check_every(&Installation::NotInstalled {
                why: "no manifest".to_owned()
            }),
            None
        );
        assert_eq!(CHECK_EVERY, Duration::from_secs(6 * 60 * 60));
    }

    #[test]
    fn a_dev_build_reports_it_is_not_installed() {
        // The test binary is not in a Velopack layout, so the manager cannot
        // load and nothing reaches the network.
        assert!(matches!(
            check_and_download(Trigger::Menu),
            Outcome::NotInstalled(_)
        ));
    }

    fn held(version: &str, slot: Slot, switch_back: bool) -> Held {
        Held {
            version: version.to_owned(),
            file_name: format!("PwrRoute-{version}-osx-full.nupkg"),
            slot: slot.key(),
            switch_back,
        }
    }

    fn download(tag: &str, switch_back: bool) -> Plan {
        Plan::Download {
            tag: tag.to_owned(),
            switch_back,
        }
    }

    #[test]
    fn a_check_downloads_a_newer_release_for_the_slot() {
        let target = Release::new("v0.1.1", true);
        let slot = Slot::STABLE_PRERELEASE;
        assert_eq!(
            plan("0.1.0", slot, Some(&target), None, Trigger::Timer),
            download("v0.1.1", false)
        );
        // Already downloaded for this slot: kept, not fetched again.
        let same = held("0.1.1", slot, false);
        assert_eq!(
            plan("0.1.0", slot, Some(&target), Some(&same), Trigger::Startup),
            Plan::Done(Outcome::Pending("0.1.1".into()))
        );
        // A newer release replaces an older download.
        let older = held("0.1.0-rc.9", slot, false);
        assert_eq!(
            plan(
                "0.1.0-rc.3",
                slot,
                Some(&target),
                Some(&older),
                Trigger::Timer
            ),
            download("v0.1.1", false)
        );
        // A download for another slot does not count.
        let other = held("0.1.1", Slot::BETA_LATEST, false);
        assert_eq!(
            plan("0.1.0", slot, Some(&target), Some(&other), Trigger::Timer),
            download("v0.1.1", false)
        );
    }

    #[test]
    fn only_check_now_switches_back_to_an_older_release() {
        let target = Release::new("v0.1.0", false);
        let slot = Slot::STABLE_LATEST;
        for trigger in [Trigger::Startup, Trigger::Timer, Trigger::Selection] {
            assert_eq!(
                plan("0.2.0-alpha.1", slot, Some(&target), None, trigger),
                Plan::Done(Outcome::UpToDate("0.2.0-alpha.1".into())),
                "{trigger:?}"
            );
        }
        for trigger in [Trigger::Menu, Trigger::Settings] {
            assert_eq!(
                plan("0.2.0-alpha.1", slot, Some(&target), None, trigger),
                download("v0.1.0", true),
                "{trigger:?}"
            );
        }
        // Downloaded already: ready to switch, and a background check
        // leaves it alone.
        let back = held("0.1.0", slot, true);
        assert_eq!(
            plan(
                "0.2.0-alpha.1",
                slot,
                Some(&target),
                Some(&back),
                Trigger::Menu
            ),
            Plan::Done(Outcome::SwitchReady("0.1.0".into()))
        );
        let newer = Release::new("v0.2.0", false);
        assert_eq!(
            plan(
                "0.2.0-alpha.1",
                slot,
                Some(&newer),
                Some(&back),
                Trigger::Timer
            ),
            Plan::Done(Outcome::SwitchReady("0.1.0".into()))
        );
    }

    #[test]
    fn the_same_version_or_an_empty_slot_downloads_nothing() {
        let slot = Slot::STABLE_LATEST;
        assert_eq!(
            plan(
                "0.1.0",
                slot,
                Some(&Release::new("v0.1.0", false)),
                None,
                Trigger::Menu
            ),
            Plan::Done(Outcome::UpToDate("0.1.0".into()))
        );
        assert_eq!(
            plan("0.1.0", slot, None, None, Trigger::Menu),
            Plan::Done(Outcome::NothingPublished("Stable \u{00b7} Latest".into()))
        );
        // A tag that does not parse is never taken for an older release.
        assert_eq!(
            plan(
                "0.1.0",
                slot,
                Some(&Release::new("nightly", false)),
                None,
                Trigger::Menu
            ),
            Plan::Done(Outcome::UpToDate("0.1.0".into()))
        );
    }

    #[test]
    fn quit_installs_only_a_newer_download_for_the_slot_in_force() {
        let slot = Slot::STABLE_LATEST;
        let newer = held("0.1.1", slot, false);
        assert_eq!(quit_package("0.1.0", slot, &newer), Some(&newer));
        assert_eq!(quit_package("0.1.0", Slot::BETA_LATEST, &newer), None);
        assert_eq!(quit_package("0.1.1", slot, &newer), None);
        let back = held("0.0.9", slot, true);
        assert_eq!(quit_package("0.1.0", slot, &back), None);
        // Restart to switch installs it; Restart never reinstalls this copy.
        assert_eq!(restart_package("0.1.0", slot, &back), Some(&back));
        assert_eq!(restart_package("0.1.0", Slot::BETA_LATEST, &back), None);
        assert_eq!(restart_package("0.1.1", slot, &newer), None);
    }

    #[test]
    fn the_held_download_is_recorded_beside_the_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HELD_FILE);
        assert_eq!(read_held(&path), None);
        let back = held("0.0.9", Slot::BETA_PRERELEASE, true);
        write_held(&path, &back).unwrap();
        assert_eq!(read_held(&path), Some(back.clone()));
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"slot\": \"beta:prerelease\""), "{text}");
        assert!(text.contains("\"switchBack\": true"), "{text}");
        assert_eq!(back.asset().FileName, "PwrRoute-0.0.9-osx-full.nupkg");
        fs::write(&path, "{").unwrap();
        assert_eq!(read_held(&path), None, "a bad file is no download");
    }

    #[test]
    fn outcomes_read_as_log_lines() {
        assert_eq!(
            Outcome::AlreadyChecking.to_string(),
            "a check is already running"
        );
        assert_eq!(
            Outcome::Downloaded("0.0.2".to_owned()).to_string(),
            "downloaded 0.0.2; it installs when PwrRoute quits or restarts"
        );
    }
}
