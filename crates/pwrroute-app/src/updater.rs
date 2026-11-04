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

/// Shutdown prevents new discovery pages and later feed/download phases.
/// In-flight blocking I/O finishes under its timeout; this is cooperative.
static STOPPING: AtomicBool = AtomicBool::new(false);

fn stopping() -> &'static AtomicBool {
    #[cfg(test)]
    if let Some(environment) = properties::environment() {
        return &environment.stopping;
    }
    &STOPPING
}

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
    stopping().store(true, Ordering::Release);
    match apply_pending() {
        QuitApply::Nothing => {}
        QuitApply::Applying(version) => {
            log::info!("updates: installing {version} after this quit");
        }
        QuitApply::Failed(why) => {
            log::warn!("updates: could not install the downloaded update: {why}");
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
    /// Changes with the selected slot, invalidating surfaces' ready cache
    /// even when the running worker and held package have not changed.
    pub selection_generation: u64,
    /// A check is running now.
    pub checking: bool,
    /// The last check for the current selection, its slot, and when.
    pub last: Option<LastCheck>,
    /// The published releases the last check read, or why it could not.
    pub releases: Option<Result<Vec<Release>, String>>,
    /// The package downloaded and not installed yet, and its slot.
    pub held: Option<Held>,
    /// The download running now, if the check found one to fetch.
    pub download: Option<Download>,
    /// How many checks have finished this launch: a check asked for from
    /// the menu waits for the next one (`update_notice`).
    pub finished: u64,
    /// Restart to update was pressed and has not failed: the process is
    /// about to be replaced ([`restart_in_background`]).
    pub restarting: bool,
}

/// A result belongs to the slot that was captured when its check started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastCheck {
    pub at: SystemTime,
    pub slot: Slot,
    pub outcome: Outcome,
}

/// A download in progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Download {
    /// Velopack's version: "0.1.1".
    pub version: String,
    /// Older than this copy: Restart to switch installs it.
    pub switch_back: bool,
    /// 0-100, once Velopack has said.
    pub percent: Option<u8>,
}

/// A downloaded release Restart to update installs now: held for the slot
/// in force and not this very version (what Settings › Updates shows as
/// Ready or Switch ready).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ready {
    pub version: String,
    /// Older than this copy: "Restart to switch".
    pub switch_back: bool,
}

/// The release Restart to update would install, for the slot `selection`
/// resolves to on `installed`; `None` when nothing is held for it.
pub fn ready_in(installed: &str, selection: &UpdateSettings, state: &State) -> Option<Ready> {
    let slot = selection.resolve(installed);
    state
        .held
        .as_ref()
        .filter(|held| held.is_for(slot) && held.version != installed)
        .map(|held| Ready {
            version: held.version.clone(),
            switch_back: held.switch_back,
        })
}

/// [`ready_in`] for this copy as it stands. Only a Velopack install can
/// have one.
pub fn ready() -> Option<Ready> {
    match installation() {
        Installation::Velopack { version } => ready_in(&version, &selection(), &state()),
        Installation::NotInstalled { .. } => None,
    }
}

/// A reservation for one worker. IDs also protect against late download
/// progress; generations distinguish A → B → A selection changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Check {
    id: u64,
    generation: u64,
    slot: Slot,
    trigger: Trigger,
}

/// Owns both admission and publication under one lock. `State` is only a
/// snapshot: its `checking` flag is derived from the active reservation.
#[derive(Default)]
struct Coordinator {
    selection: UpdateSettings,
    generation: u64,
    next_id: u64,
    active: Option<Check>,
    queued: Option<Check>,
    last: Option<LastCheck>,
    releases: Option<Result<Vec<Release>, String>>,
    held: Option<Held>,
    download: Option<Download>,
    finished: u64,
    restarting: bool,
}

impl Coordinator {
    fn snapshot(&self) -> State {
        State {
            selection_generation: self.generation,
            checking: self.active.is_some(),
            last: self.last.clone(),
            releases: self.releases.clone(),
            held: self.held.clone(),
            download: self.download.clone(),
            finished: self.finished,
            restarting: self.restarting,
        }
    }

    fn select(&mut self, settings: UpdateSettings, installed: &str) -> Option<Check> {
        let before = self.selection.resolve(installed);
        self.selection = settings;
        if settings.resolve(installed) == before {
            return None;
        }
        self.generation += 1;
        self.last = None;
        self.download = None;
        // A manual request's downgrade permission belongs to its selection.
        self.queued = None;
        self.request(Trigger::Selection, installed)
    }

    /// At most one follow-up, for the latest selection. A manual request
    /// upgrades that follow-up, and automatic requests cannot demote it.
    fn request(&mut self, trigger: Trigger, installed: &str) -> Option<Check> {
        if let Some(active) = self.active {
            let same = active.generation == self.generation;
            if same && (!trigger.is_manual() || active.trigger.is_manual()) {
                return None;
            }
            if let Some(queued) = &mut self.queued {
                if trigger.is_manual() {
                    queued.trigger = trigger;
                }
                return None;
            }
        }
        self.next_id += 1;
        let check = Check {
            id: self.next_id,
            generation: self.generation,
            slot: self.selection.resolve(installed),
            trigger,
        };
        if self.active.is_some() {
            self.queued = Some(check);
            None
        } else {
            self.active = Some(check);
            Some(check)
        }
    }

    fn finish(&mut self, check: Check, outcome: Outcome, at: SystemTime) -> Option<Check> {
        if self.active != Some(check) {
            return None;
        }
        self.download = None;
        self.finished += 1;
        if check.generation == self.generation {
            self.last = Some(LastCheck {
                at,
                slot: check.slot,
                outcome,
            });
        }
        // Reserve the next worker before releasing the lock: no idle gap
        // in which another request or an old completion can take its place.
        self.active = self.queued.take();
        self.active
    }

    fn percent(&mut self, check: Check, percent: u8) {
        if self.active == Some(check)
            && check.generation == self.generation
            && let Some(download) = &mut self.download
        {
            download.percent = Some(percent);
        }
    }

    fn begin_restart(&mut self) -> bool {
        if self.restarting {
            return false;
        }
        self.restarting = true;
        true
    }
}

static UPDATER: OnceLock<Mutex<Coordinator>> = OnceLock::new();

fn coordinator() -> &'static Mutex<Coordinator> {
    #[cfg(test)]
    if let Some(environment) = properties::environment() {
        return &environment.coordinator;
    }
    UPDATER.get_or_init(|| Mutex::new(Coordinator::default()))
}

/// The release channel as `settings.json` has it.
pub fn selection() -> UpdateSettings {
    coordinator()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .selection
}

/// Follows `settings` from now on. A new slot is checked forward only,
/// after the running check if necessary. Held packages retain their slot.
pub fn set_selection(settings: UpdateSettings) {
    let installed = installed_version();
    let check = update(|updater| updater.select(settings, &installed));
    // The inferred/picked label and ready-package filter can change even
    // when the check snapshot itself did not.
    changed(true);
    if let Some(check) = check {
        spawn_check(check);
    }
}

/// The version this copy reports: Velopack's manifest, else the crate's.
pub fn installed_version() -> String {
    match installation() {
        Installation::Velopack { version } => version,
        Installation::NotInstalled { .. } => env!("CARGO_PKG_VERSION").to_owned(),
    }
}

/// Wakes [`install`]'s task after every change to [`state`], saying
/// whether Settings › Updates draws it ([`settings_changed`]).
static CHANGED: OnceLock<async_channel::Sender<bool>> = OnceLock::new();

/// A surface that follows the updater (`update_notice`): called on the
/// GPUI thread after every change to [`state`], it reads it again.
pub type Listener = fn(&mut App);

static LISTENERS: Mutex<Vec<Listener>> = Mutex::new(Vec::new());

/// Calls `listener` after every change to [`state`] from now on.
pub fn on_change(listener: Listener) {
    LISTENERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(listener);
}

/// The updater's state as it stands.
pub fn state() -> State {
    coordinator()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .snapshot()
}

fn changed(settings: bool) {
    if let Some(changed) = CHANGED.get() {
        let _ = changed.try_send(settings);
    }
}

fn update<R>(edit: impl FnOnce(&mut Coordinator) -> R) -> R {
    let (result, settings) = {
        let mut updater = coordinator().lock().unwrap_or_else(PoisonError::into_inner);
        let before = updater.snapshot();
        let result = edit(&mut updater);
        (result, settings_changed(&before, &updater.snapshot()))
    };
    changed(settings);
    result
}

/// Worker writes must still own the active reservation. A held package
/// remains real even when its selection generation is no longer current.
fn update_check(check: Check, edit: impl FnOnce(&mut Coordinator)) {
    update(|updater| {
        if updater.active == Some(check) {
            edit(updater);
        }
    });
}

/// Percent ticks only wake the toast; they avoid cloning the release list.
fn set_percent(check: Check, percent: u8) {
    coordinator()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .percent(check, percent);
    changed(false);
}

/// Whether going from `before` to `after` changes what Settings › Updates
/// draws: everything but the download in flight, which only the update
/// toast shows (`update_notice`). A download reports every percent, so
/// those changes skip rebuilding the pane, which reads the install's
/// manifest from disk.
pub fn settings_changed(before: &State, after: &State) -> bool {
    // Every field by name, so a new one has to be sorted here.
    let State {
        selection_generation,
        checking,
        last,
        releases,
        held,
        download: _,
        finished,
        restarting,
    } = before;
    *selection_generation != after.selection_generation
        || *checking != after.checking
        || *last != after.last
        || *releases != after.releases
        || *held != after.held
        || *finished != after.finished
        || *restarting != after.restarting
}

/// Redraws Settings › Updates whenever what it draws changes, tells the
/// [`on_change`] listeners about every change (a download's percent too),
/// and
/// checks again every [`CHECK_EVERY`] on a Velopack install. Call once,
/// after `settings` is installed.
pub fn install(cx: &mut App) {
    let (tx, rx) = async_channel::unbounded::<bool>();
    if CHANGED.set(tx).is_err() {
        return;
    }
    cx.on_app_quit(|_| {
        stopping().store(true, Ordering::Release);
        async {}
    })
    .detach();
    if let Some(store) = crate::settings_store::store(cx) {
        let settings = store.read(cx).settings().updates;
        update(|updater| updater.selection = settings);
    }
    if let Some(held) = load_held() {
        update(|updater| updater.held = Some(held));
    }
    cx.spawn(async move |cx| {
        while let Ok(first) = rx.recv().await {
            // A burst of changes (a fast download's percents) is one pass.
            let mut settings = first;
            while let Ok(more) = rx.try_recv() {
                settings |= more;
            }
            cx.update(|cx| {
                if settings {
                    crate::settings::reload(cx, crate::settings::Pane::Updates);
                }
                let listeners = LISTENERS
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone();
                for listener in listeners {
                    listener(cx);
                }
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
    if stopping().load(Ordering::Acquire) {
        return;
    }
    let installed = installed_version();
    match update(|updater| updater.request(trigger, &installed)) {
        Some(check) => spawn_check(check),
        None => log(trigger, &Outcome::AlreadyChecking),
    }
}

fn finish_check(check: Check, outcome: Outcome) -> Option<Check> {
    log(check.trigger, &outcome);
    update(|updater| updater.finish(check, outcome, SystemTime::now()))
}

fn spawn_check(mut check: Check) {
    #[cfg(test)]
    let environment = properties::environment();
    loop {
        let spawned = thread::Builder::new()
            .name("pwrroute-updater".to_owned())
            .spawn(move || {
                #[cfg(test)]
                properties::install_environment(environment);
                let mut check = check;
                loop {
                    #[cfg(test)]
                    let outcome =
                        properties::fetch(check).unwrap_or_else(|| check_and_download(check));
                    #[cfg(not(test))]
                    let outcome = check_and_download(check);
                    let next = finish_check(check, outcome);
                    #[cfg(test)]
                    properties::finished(next);
                    match next {
                        Some(next) => check = next,
                        None => break,
                    }
                }
            });
        match spawned {
            Ok(_) => break,
            Err(error) => match finish_check(check, Outcome::Failed(error.to_string())) {
                Some(next) => check = next,
                None => break,
            },
        }
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

/// [`restart_to_update`] on a thread of its own: Velopack exits this
/// process once the package is applied, so nothing waits on it. Settings,
/// the tray, the strip and the update toast all restart through this.
/// One restart at a time ([`State::restarting`]); a failed one clears
/// it, so the surfaces offer Restart again.
pub fn restart_in_background() {
    if !update(Coordinator::begin_restart) {
        return;
    }
    let failed = |error: String| {
        log::warn!("updates: could not restart to update: {error}");
        update(|updater| updater.restarting = false);
    };
    let spawned = thread::Builder::new()
        .name("pwrroute-restart".to_owned())
        .spawn(move || {
            if let Err(error) = restart_to_update() {
                failed(error);
            }
        });
    if let Err(error) = spawned {
        failed(error.to_string());
    }
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

fn check_and_download(check: Check) -> Outcome {
    if stopping().load(Ordering::Acquire) {
        return Outcome::Failed("update check was cancelled during shutdown".into());
    }
    // The installed version (Velopack's sq.version), not the one compiled
    // in: packages are stamped separately from Cargo.toml.
    let locator = match locator::auto_locate_app_manifest(LocationContext::FromCurrentExe) {
        Ok(locator) => locator,
        Err(error) => return Outcome::NotInstalled(error.to_string()),
    };
    let installed = locator.get_manifest_version().to_string();
    let slot = check.slot;
    let releases = github_releases::fetch(&locator.get_manifest_channel(), stopping());
    update_check(check, |updater| updater.releases = Some(releases.clone()));
    let releases = match releases {
        Ok(releases) => releases,
        Err(error) => return Outcome::Failed(error),
    };
    let target = update_channel::select(&releases, slot);
    let held = load_held().filter(|held| package_exists(&locator, held));
    // A package Velopack no longer has is not ready to install.
    update_check(check, |updater| updater.held = held.clone());
    let (tag, switch_back) = match plan(
        &installed,
        slot,
        target.as_ref(),
        held.as_ref(),
        check.trigger,
    ) {
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
    if stopping().load(Ordering::Acquire) {
        return Outcome::Failed("update check was cancelled during shutdown".into());
    }
    match manager.check_for_updates() {
        Ok(UpdateCheck::UpdateAvailable(update)) => {
            if stopping().load(Ordering::Acquire) {
                return Outcome::Failed("update check was cancelled during shutdown".into());
            }
            let target = &update.TargetFullRelease;
            if let Err(error) = download(check, &manager, &update, switch_back) {
                return Outcome::Failed(error.to_string());
            }
            let held = Held {
                version: target.Version.clone(),
                file_name: target.FileName.clone(),
                slot: slot.key(),
                switch_back,
            };
            save_held(&held);
            update_check(check, |updater| updater.held = Some(held.clone()));
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

/// Downloads `update`, keeping [`State::download`]'s percent current.
fn download(
    check: Check,
    manager: &UpdateManager,
    update: &velopack::UpdateInfo,
    switch_back: bool,
) -> Result<(), velopack::Error> {
    update_check(check, |updater| {
        if check.generation == updater.generation {
            updater.download = Some(Download {
                version: update.TargetFullRelease.Version.clone(),
                switch_back,
                percent: None,
            });
        }
    });
    let (progress, percents) = std::sync::mpsc::channel::<i16>();
    // Ends when Velopack drops the sender, at the end of the download.
    let follow = thread::Builder::new()
        .name("pwrroute-download".to_owned())
        .spawn(move || {
            let mut last = None;
            for percent in percents {
                let percent = u8::try_from(percent.clamp(0, 100)).unwrap_or(0);
                if last != Some(percent) {
                    last = Some(percent);
                    set_percent(check, percent);
                }
            }
        });
    let progress = follow.is_ok().then_some(progress);
    manager.download_updates(update, progress)
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
        log::warn!("updates: could not record the download: {error}");
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
    log::info!("updates ({trigger}): {outcome}");
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSTALLED: &str = "0.2.0";

    fn finish(updater: &mut Coordinator, check: Check, outcome: Outcome) -> Option<Check> {
        updater.finish(check, outcome, SystemTime::UNIX_EPOCH)
    }

    #[test]
    fn a_selection_change_in_flight_checks_the_new_slot_before_settling() {
        let mut updater = Coordinator::default();
        let old = updater.request(Trigger::Startup, INSTALLED).unwrap();
        assert_eq!(old.slot, Slot::STABLE_LATEST);
        updater.download = Some(Download {
            version: "0.3.0".into(),
            switch_back: false,
            percent: Some(20),
        });
        let before = updater.snapshot();
        assert_eq!(
            updater.select(UpdateSettings::picked(Slot::BETA_LATEST), INSTALLED),
            None
        );
        assert!(updater.snapshot().checking);
        assert!(
            settings_changed(&before, &updater.snapshot()),
            "selection invalidates the ready cache"
        );
        updater.percent(old, 100);
        assert_eq!(
            updater.download, None,
            "the old generation's progress is hidden"
        );
        // The old download can still finish physically. It stays recorded
        // for its own slot while the selected slot's follow-up runs.
        updater.held = Some(held("0.3.0", old.slot, false));
        let new = finish(&mut updater, old, Outcome::UpToDate(INSTALLED.into())).unwrap();
        assert_eq!(new.slot, Slot::BETA_LATEST);
        assert_eq!(new.trigger, Trigger::Selection);
        assert!(
            updater.snapshot().checking,
            "the next worker is already reserved"
        );
        assert_eq!(
            updater.last, None,
            "the old slot cannot answer for the new one"
        );
        assert!(updater.held.as_ref().unwrap().is_for(old.slot));
        assert_eq!(
            ready_in(INSTALLED, &updater.selection, &updater.snapshot()),
            None
        );
        assert_eq!(
            finish(
                &mut updater,
                new,
                Outcome::NothingPublished(new.slot.to_string())
            ),
            None
        );
        let state = updater.snapshot();
        assert!(!state.checking);
        assert_eq!(state.last.unwrap().slot, Slot::BETA_LATEST);
        assert_eq!(state.finished, 2);
    }

    #[test]
    fn an_idle_selection_change_invalidates_the_previous_result() {
        let mut updater = Coordinator::default();
        let check = updater.request(Trigger::Menu, INSTALLED).unwrap();
        finish(&mut updater, check, Outcome::UpToDate(INSTALLED.into()));
        // An explicit pick of the inferred slot only changes the label.
        assert_eq!(
            updater.select(UpdateSettings::picked(check.slot), INSTALLED),
            None
        );
        assert!(updater.last.is_some());
        assert!(!updater.snapshot().checking);
        let next = updater
            .select(UpdateSettings::picked(Slot::BETA_LATEST), INSTALLED)
            .unwrap();
        assert_eq!(updater.last, None);
        assert!(updater.snapshot().checking);
        finish(&mut updater, next, Outcome::Failed("offline".into()));
        assert_eq!(updater.last.unwrap().slot, next.slot);
    }

    #[test]
    fn rapid_selection_changes_coalesce_even_when_returning_to_the_original_slot() {
        let mut updater = Coordinator::default();
        let old = updater.request(Trigger::Startup, INSTALLED).unwrap();
        for slot in [
            Slot::BETA_LATEST,
            Slot::BETA_PRERELEASE,
            Slot::STABLE_LATEST,
        ] {
            assert_eq!(
                updater.select(UpdateSettings::picked(slot), INSTALLED),
                None
            );
            assert_eq!(updater.queued.unwrap().slot, slot);
        }
        let new = finish(&mut updater, old, Outcome::UpToDate(INSTALLED.into())).unwrap();
        assert_eq!(new.slot, old.slot);
        assert_ne!(new.generation, old.generation);
        assert_eq!(
            updater.last, None,
            "returning to A does not revive A's old check"
        );
        assert_eq!(updater.queued, None, "only the final selection is checked");
        finish(&mut updater, new, Outcome::UpToDate(INSTALLED.into()));
        assert_eq!(updater.finished, 2);
    }

    #[test]
    fn manual_requests_during_auto_checks_retain_downgrade_permission() {
        let target = Release::new("v0.1.0", false);
        for automatic in [Trigger::Startup, Trigger::Timer, Trigger::Selection] {
            for manual in [Trigger::Menu, Trigger::Settings] {
                let mut updater = Coordinator::default();
                let old = updater.request(automatic, INSTALLED).unwrap();
                assert_eq!(
                    plan(INSTALLED, old.slot, Some(&target), None, old.trigger),
                    Plan::Done(Outcome::UpToDate(INSTALLED.into()))
                );
                assert_eq!(updater.request(manual, INSTALLED), None);
                let queued = updater.queued.unwrap();
                // Repeated clicks and timer ticks cannot add more work or
                // strip the manual request's permission.
                updater.request(manual, INSTALLED);
                updater.request(Trigger::Timer, INSTALLED);
                assert_eq!(updater.queued, Some(queued));
                let next = finish(&mut updater, old, Outcome::UpToDate(INSTALLED.into())).unwrap();
                assert_eq!(next, queued);
                assert_eq!(
                    plan(INSTALLED, next.slot, Some(&target), None, next.trigger),
                    Plan::Download {
                        tag: target.tag.clone(),
                        switch_back: true
                    }
                );
                assert_eq!(updater.request(manual, INSTALLED), None);
                assert_eq!(
                    updater.queued, None,
                    "an active manual check can answer repeated clicks"
                );
            }
        }
    }

    #[test]
    fn a_manual_follow_up_is_upgraded_only_for_its_own_selection() {
        let mut updater = Coordinator::default();
        let old = updater.request(Trigger::Startup, INSTALLED).unwrap();
        updater.select(UpdateSettings::picked(Slot::BETA_LATEST), INSTALLED);
        updater.request(Trigger::Menu, INSTALLED);
        assert!(updater.queued.unwrap().trigger.is_manual());
        updater.select(UpdateSettings::picked(Slot::BETA_PRERELEASE), INSTALLED);
        let next = finish(&mut updater, old, Outcome::UpToDate(INSTALLED.into())).unwrap();
        assert_eq!(next.slot, Slot::BETA_PRERELEASE);
        assert_eq!(
            next.trigger,
            Trigger::Selection,
            "manual permission cannot cross slots"
        );
    }

    #[test]
    fn late_completions_and_progress_cannot_clobber_a_new_worker() {
        let mut updater = Coordinator::default();
        let old = updater.request(Trigger::Startup, INSTALLED).unwrap();
        updater.request(Trigger::Menu, INSTALLED);
        let next = finish(&mut updater, old, Outcome::UpToDate(INSTALLED.into())).unwrap();
        updater.download = Some(Download {
            version: "0.1.0".into(),
            switch_back: true,
            percent: Some(20),
        });
        let before = updater.snapshot();
        assert_eq!(
            finish(&mut updater, old, Outcome::Failed("late failure".into())),
            None
        );
        updater.percent(old, 100);
        assert_eq!(updater.snapshot(), before);
        updater.percent(next, 40);
        assert_eq!(updater.download.as_ref().unwrap().percent, Some(40));
        finish(&mut updater, next, Outcome::SwitchReady("0.1.0".into()));
        let before = updater.snapshot();
        // The other ordering: an old completion after the new one ended.
        finish(&mut updater, old, Outcome::Failed("late failure".into()));
        assert_eq!(updater.snapshot(), before);
        // And after an independent new request has already started.
        let newest = updater.request(Trigger::Timer, INSTALLED).unwrap();
        let before = updater.snapshot();
        finish(&mut updater, next, Outcome::Failed("late failure".into()));
        assert_eq!(updater.snapshot(), before);
        assert_eq!(updater.active, Some(newest));
    }

    #[test]
    fn checking_and_restarting_do_not_own_the_held_package() {
        let package = held("0.3.0", Slot::STABLE_LATEST, false);
        let mut updater = Coordinator {
            held: Some(package.clone()),
            ..Coordinator::default()
        };
        let check = updater.request(Trigger::Timer, INSTALLED).unwrap();
        assert!(ready_in(INSTALLED, &updater.selection, &updater.snapshot()).is_some());
        assert!(updater.begin_restart());
        assert!(!updater.begin_restart());
        finish(&mut updater, check, Outcome::Failed("offline".into()));
        assert_eq!(updater.held, Some(package.clone()));
        assert!(updater.snapshot().restarting);
        updater.restarting = false;
        assert!(updater.begin_restart(), "a failed restart can be retried");
        let new = updater
            .select(UpdateSettings::picked(Slot::BETA_LATEST), INSTALLED)
            .unwrap();
        assert_eq!(updater.held, Some(package));
        assert_eq!(
            ready_in(INSTALLED, &updater.selection, &updater.snapshot()),
            None
        );
        finish(&mut updater, new, Outcome::Failed("offline".into()));
        assert!(updater.held.is_some());
    }

    /// A download's percent is the toast's alone; everything else is
    /// Settings › Updates' too, so only those changes rebuild the pane.
    #[test]
    fn only_changes_settings_draws_rebuild_settings() {
        let downloading = State {
            checking: true,
            download: Some(Download {
                version: "0.15.0".into(),
                switch_back: false,
                percent: Some(41),
            }),
            ..State::default()
        };
        let mut ticked = downloading.clone();
        if let Some(download) = &mut ticked.download {
            download.percent = Some(42);
        }
        assert!(!settings_changed(&downloading, &ticked), "a percent tick");
        let mut started = downloading.clone();
        started.download = None;
        assert!(
            !settings_changed(&started, &downloading),
            "a download starts"
        );

        let mut done = downloading.clone();
        done.checking = false;
        assert!(settings_changed(&downloading, &done), "checking");
        let mut held = downloading.clone();
        held.held = Some(Held {
            version: "0.15.0".into(),
            file_name: "PwrRoute-0.15.0-full.nupkg".into(),
            slot: "stable:latest".into(),
            switch_back: false,
        });
        assert!(settings_changed(&downloading, &held), "held");
        let mut answered = downloading.clone();
        answered.last = Some(LastCheck {
            at: SystemTime::UNIX_EPOCH,
            slot: Slot::STABLE_LATEST,
            outcome: Outcome::UpToDate("0.14.0".into()),
        });
        assert!(settings_changed(&downloading, &answered), "last");
        let mut restarting = downloading.clone();
        restarting.restarting = true;
        assert!(settings_changed(&downloading, &restarting), "restarting");
        assert!(
            !settings_changed(&downloading, &downloading.clone()),
            "no change"
        );
    }

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
        let check = Coordinator::default()
            .request(Trigger::Menu, INSTALLED)
            .unwrap();
        assert!(matches!(
            check_and_download(check),
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

#[cfg(test)]
#[path = "property_tests/updater.rs"]
mod properties;
