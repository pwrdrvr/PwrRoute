//! Synced rules in the app: the sync thread, what it last reported, and
//! this machine's layer (`machine.json`).
//!
//! `install` starts a [`pwrroute_sync::Service`] over `rules.json` (and the
//! Finicky config in charge, when Settings › Cloud Sync says so) and the folders
//! `settings.json`'s `sync.targets` lists. Its reports land in the
//! [`SyncFeed`] entity, which Settings › Cloud Sync, the rail foot and the tray
//! read. A pass that applied another machine's file asks every main window
//! to read `rules.json` again (`app::sync_rules_windows`), as an MCP write
//! does; the router's reloader already follows the file.
//!
//! Every writer of `rules.json` (the Rules screen's store, first run,
//! Settings, `pwrroute_rules_apply`) writes the local file as before; the
//! sync thread is the only thing that copies it to or from a folder. A
//! save is a local change the schedule notices within two ticks, and the
//! window coming to the front, an MCP write and Sync now wake it at once.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, PoisonError, RwLock},
};

use anyhow::{Context as _, Result};
use gpui_kit::{App, AppContext as _, Entity, Global};
use pwrroute_engine::{Host as _, MachineFile, NativeHost, RulesFile, discovery};
use pwrroute_sync::{Content, Item, Place, Service, Status, Syncer, SystemClock, Target};

use crate::settings_store::{self, SyncSettings};

/// This machine's layer as last read or saved; empty until `install` and in
/// tests, so a test never reads the real data directory's.
static MACHINE: RwLock<Option<MachineFile>> = RwLock::new(None);

/// What `machine.json` says now, for engines the app builds itself (Test a
/// link, Try it, the test runs); the router's reloader reads the file.
pub fn machine() -> MachineFile {
    MACHINE
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .unwrap_or_default()
}

fn set_machine(machine: MachineFile) {
    *MACHINE.write().unwrap_or_else(PoisonError::into_inner) = Some(machine);
}

pub fn rules_path() -> Option<PathBuf> {
    discovery::rules_path(&discovery::Environment::current())
}

fn machine_path() -> Option<PathBuf> {
    rules_path().map(|p| MachineFile::path_beside(&p))
}

/// Reads `machine.json` again (an editor may have changed it). Returns
/// whether the layer changed; one that does not parse keeps the last.
pub fn reload_machine(cx: &App) -> bool {
    if MACHINE
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .is_none()
    {
        // Not installed (tests): the layer stays empty.
        return false;
    }
    let Some(path) = machine_path() else {
        return false;
    };
    let read = match MachineFile::load(&path) {
        Ok(read) => read,
        Err(e) => {
            log::warn!("machine.json did not load: {e:#}");
            return false;
        }
    };
    if read == machine() {
        return false;
    }
    let name = machine_name(&read);
    set_machine(read);
    if let Some(feed) = feed(cx) {
        feed.read(cx).with_service(|s| s.set_machine_name(&name));
    }
    true
}

/// Edits `machine.json` and saves it. The router's reloader follows the
/// file; the window's engines read [`machine`] on their next build. A file
/// that does not parse is left alone: saving over it would lose the hand
/// edits in it.
pub fn edit_machine(edit: impl FnOnce(&mut MachineFile), cx: &mut App) -> Result<()> {
    let path = machine_path().context("no configuration directory")?;
    let mut machine =
        MachineFile::load(&path).context("machine.json does not load; fix it by hand first")?;
    edit(&mut machine);
    machine.save(&path)?;
    let name = machine_name(&machine);
    set_machine(machine);
    if let Some(feed) = feed(cx) {
        feed.read(cx).with_service(|s| s.set_machine_name(&name));
    }
    crate::app::sync_rules_windows(cx);
    Ok(())
}

/// The computer's own name: macOS's Computer Name, Windows' computer
/// name, Linux's pretty host name, else its host name.
pub fn computer_name() -> String {
    let info = NativeHost::new().system_info();
    if info.localized_name.trim().is_empty() {
        info.name
    } else {
        info.localized_name
    }
}

/// The name sync shows for this machine: `machine.json`'s, else the
/// computer's.
pub fn machine_name(machine: &MachineFile) -> String {
    if machine.name.trim().is_empty() {
        computer_name()
    } else {
        machine.name.clone()
    }
}

/// What the sync thread last reported, and its handle.
pub struct SyncFeed {
    pub status: Status,
    /// The cloud folders found on this machine, read when Settings › Cloud Sync
    /// opens ([`SyncFeed::look_for_places`]), never at launch.
    pub places: Vec<Place>,
    service: Option<Service>,
    /// The files the thread was last given, by name and path: the config
    /// in charge can change (or be published by the router) after launch.
    sent_items: std::cell::RefCell<Vec<(String, PathBuf)>>,
}

impl SyncFeed {
    fn with_service(&self, f: impl FnOnce(&Service)) {
        if let Some(service) = &self.service {
            f(service);
        }
    }

    /// Hands the thread `items` unless it already has those files.
    fn send_items(&self, items: Vec<Item>) {
        let names: Vec<(String, PathBuf)> = items
            .iter()
            .map(|i| (i.name.clone(), i.local.clone()))
            .collect();
        if *self.sent_items.borrow() == names {
            return;
        }
        *self.sent_items.borrow_mut() = names;
        self.with_service(|service| service.set_items(items));
    }

    /// Settles a conflict on the sync thread; `done` runs there.
    pub fn resolve(
        &self,
        root: PathBuf,
        file: String,
        remote_sha: String,
        choice: pwrroute_sync::Choice,
        done: impl FnOnce(Result<()>) + Send + 'static,
    ) {
        match &self.service {
            Some(service) => service.resolve(root, file, remote_sha, choice, done),
            None => done(Err(anyhow::anyhow!("sync is not running"))),
        }
    }

    /// Looks for iCloud Drive, Google Drive and Dropbox folders. Only exact
    /// paths, and one listing of `~/Library/CloudStorage`.
    pub fn look_for_places(&mut self) {
        self.places = pwrroute_sync::detect(&pwrroute_sync::Env::current());
    }
}

/// How loud the sync line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Quiet,
    Warn,
    Bad,
}

/// Sync's one line for the rail foot and the tray (design: Synced Rules
/// 3a-3c), and the strip chip for the two states that need the reader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// "synced · 2 min ago", "sync conflict".
    pub text: String,
    pub tone: Tone,
    /// "sync · conflict" or "sync · paused".
    pub chip: Option<&'static str>,
    /// The folder that needs the reader: "Dropbox: both changed since the
    /// last sync".
    pub tip: Option<String>,
}

/// The whole app's sync state, worst first (design: Synced Rules 3c): a
/// conflict in any folder, paused (no folder reachable or every one
/// failing), syncing, synced. The rail line and the tray read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Overall {
    /// The first folder in conflict, by label.
    Conflict(String),
    /// No folder in step: the first failing one and why, else the first
    /// unreachable one.
    Paused {
        failing: Option<(String, String)>,
        first: String,
    },
    Syncing,
    Synced {
        /// When the last pass ran, in milliseconds.
        last: u64,
        unavailable: usize,
        failing: usize,
        /// The folders in step, by their short names.
        folders: Vec<String>,
    },
}

/// [`Overall`] for `status`; `None` while nothing syncs.
pub fn overall(status: &Status) -> Option<Overall> {
    use pwrroute_sync::{Health, Wait};
    if status.targets.is_empty() {
        return None;
    }
    let worst = |t: &pwrroute_sync::TargetStatus| t.worst().map(|f| f.health.clone());
    if let Some(target) = status
        .targets
        .iter()
        .find(|t| matches!(worst(t), Some(Health::Conflict(_))))
    {
        return Some(Overall::Conflict(target.target.label.clone()));
    }
    let unavailable = status
        .targets
        .iter()
        .filter(|t| matches!(worst(t), Some(Health::Unavailable(_))))
        .count();
    // A write that failed or a file that does not load: the folder is
    // there, so say so rather than "unavailable".
    let failing: Vec<(String, String)> = status
        .targets
        .iter()
        .filter_map(|t| match worst(t) {
            Some(Health::Error(why)) => Some((t.target.label.clone(), why)),
            _ => None,
        })
        .collect();
    if unavailable + failing.len() == status.targets.len() {
        return Some(Overall::Paused {
            failing: failing.into_iter().next(),
            first: status.targets[0].target.label.clone(),
        });
    }
    let settling = status
        .targets
        .iter()
        .any(|t| matches!(worst(t), Some(Health::Waiting(Wait::Settling))));
    let Some(last) = status.last_pass.filter(|_| !settling) else {
        return Some(Overall::Syncing);
    };
    let folders = status
        .targets
        .iter()
        .filter(|t| !matches!(worst(t), Some(Health::Unavailable(_) | Health::Error(_))))
        .map(|t| short_name(&t.target))
        .collect();
    Some(Overall::Synced {
        last,
        unavailable,
        failing: failing.len(),
        folders,
    })
}

/// A folder as the tray lists it: "iCloud", "Dropbox", a folder's name.
fn short_name(target: &Target) -> String {
    match target.kind {
        pwrroute_sync::Kind::ICloud => "iCloud".into(),
        pwrroute_sync::Kind::GoogleDrive => "Google Drive".into(),
        pwrroute_sync::Kind::Dropbox => "Dropbox".into(),
        pwrroute_sync::Kind::Folder => target.label.clone(),
    }
}

/// The rail foot's line and the strip's chip for `status`. `now` and
/// `last_pass` are milliseconds.
pub fn line_of(status: &Status, now: u64) -> Option<Line> {
    Some(match overall(status)? {
        Overall::Conflict(label) => Line {
            text: "sync conflict".into(),
            tone: Tone::Bad,
            chip: Some("sync \u{00b7} conflict"),
            tip: Some(format!("{label}: both changed since the last sync")),
        },
        Overall::Paused {
            failing: Some((label, why)),
            ..
        } => Line {
            text: "sync paused \u{00b7} needs you".into(),
            tone: Tone::Warn,
            chip: Some("sync \u{00b7} paused"),
            tip: Some(format!("{label}: {why}")),
        },
        Overall::Paused {
            failing: None,
            first,
        } => Line {
            text: "sync paused \u{00b7} folder unavailable".into(),
            tone: Tone::Warn,
            chip: Some("sync \u{00b7} paused"),
            tip: Some(format!("{first}: not reachable")),
        },
        Overall::Syncing => Line {
            text: "syncing\u{2026}".into(),
            tone: Tone::Quiet,
            chip: None,
            tip: None,
        },
        Overall::Synced {
            last,
            unavailable,
            failing,
            ..
        } => {
            let mut text = format!("synced \u{00b7} {}", age(now.saturating_sub(last)));
            let mut tone = Tone::Quiet;
            if unavailable > 0 {
                text.push_str(&format!(" \u{00b7} {unavailable} unavailable"));
                tone = Tone::Warn;
            }
            if failing > 0 {
                text.push_str(&format!(" \u{00b7} {failing} need you"));
                tone = Tone::Warn;
            }
            Line {
                text,
                tone,
                chip: None,
                tip: None,
            }
        }
    })
}

/// The tray's sync row (design: Synced Rules 3c) for `status`.
pub fn tray_row_of(status: &Status, now: u64) -> Option<crate::tray::model::SyncRow> {
    use crate::tray::model::{SyncRow, SyncTone};
    Some(match overall(status)? {
        Overall::Conflict(_) => SyncRow {
            title: "Sync conflict".into(),
            sub: Some("Resolve\u{2026}".into()),
            tone: SyncTone::Bad,
        },
        Overall::Paused { failing, .. } => SyncRow {
            title: "Sync paused".into(),
            sub: Some(if failing.is_some() {
                "needs you".into()
            } else {
                "folder unavailable".into()
            }),
            tone: SyncTone::Warn,
        },
        Overall::Syncing => SyncRow {
            title: "Syncing rules\u{2026}".into(),
            sub: None,
            tone: SyncTone::Busy,
        },
        Overall::Synced {
            last,
            unavailable,
            failing,
            folders,
        } => SyncRow {
            title: format!("Rules synced {}", age(now.saturating_sub(last))),
            sub: Some(folders.join(", ")).filter(|s| !s.is_empty()),
            tone: if unavailable + failing > 0 {
                SyncTone::Warn
            } else {
                SyncTone::Ok
            },
        },
    })
}

/// "just now", "2 min ago", "3 h ago".
fn age(ms: u64) -> String {
    let minutes = ms / 60_000;
    match minutes {
        0 => "just now".into(),
        1..=59 => format!("{minutes} min ago"),
        _ => format!("{} h ago", minutes / 60),
    }
}

/// [`line_of`] for the running app.
pub fn line(cx: &App) -> Option<Line> {
    let feed = feed(cx)?;
    line_of(
        &feed.read(cx).status,
        pwrroute_sync::Clock::now(&SystemClock),
    )
}

struct FeedGlobal(Entity<SyncFeed>);

impl Global for FeedGlobal {}

pub fn feed(cx: &App) -> Option<Entity<SyncFeed>> {
    cx.try_global::<FeedGlobal>().map(|g| g.0.clone())
}

/// Starts the sync thread. Nothing reads a cloud folder until a folder is
/// configured; with none, the thread only ticks.
pub fn install(cx: &mut App) {
    let (Some(rules), Some(data_dir)) = (rules_path(), pwrroute_os::data_dir()) else {
        return;
    };
    let machine = MachineFile::load(&MachineFile::path_beside(&rules)).unwrap_or_else(|e| {
        log::warn!("machine.json did not load: {e:#}");
        MachineFile::default()
    });
    let name = machine_name(&machine);
    set_machine(machine);
    let settings = settings(cx);
    let mut syncer = Syncer::new(&data_dir, &name, env!("CARGO_PKG_VERSION"));
    let first = items(&rules, &settings);
    let sent = first
        .iter()
        .map(|i| (i.name.clone(), i.local.clone()))
        .collect();
    syncer.set_items(first);
    syncer.set_targets(settings.targets.clone());
    let (tx, rx) = async_channel::unbounded::<Status>();
    let service = match Service::spawn(syncer, SystemClock, move |status| {
        let _ = tx.try_send(status.clone());
    }) {
        Ok(service) => Some(service),
        Err(e) => {
            log::error!("sync: could not start: {e:#}");
            None
        }
    };
    let entity = cx.new(|_| SyncFeed {
        status: Status::default(),
        places: Vec::new(),
        service,
        sent_items: std::cell::RefCell::new(sent),
    });
    cx.set_global(FeedGlobal(entity.clone()));
    cx.spawn(async move |cx| {
        while let Ok(status) = rx.recv().await {
            cx.update(|cx| apply(status, &entity, cx));
        }
    })
    .detach();
}

fn apply(status: Status, entity: &Entity<SyncFeed>, cx: &mut App) {
    let pulled = status.targets.iter().any(|t| {
        t.files
            .iter()
            .any(|f| f.health == pwrroute_sync::Health::Pulled)
    });
    entity.update(cx, |feed, cx| {
        feed.status = status;
        cx.notify();
    });
    if pulled {
        crate::app::sync_rules_windows(cx);
    }
    follow_items(cx);
    crate::settings::sync_host::refresh(cx);
}

/// Runs a pass now: Sync now, the window coming to the front, an agent's
/// write.
pub fn wake(cx: &App) {
    follow_items(cx);
    if let Some(feed) = feed(cx) {
        feed.read(cx).with_service(Service::wake);
    }
}

/// Gives the thread the files that sync now: the router publishes the
/// config in charge after launch, and the importer can change it.
fn follow_items(cx: &App) {
    if let (Some(feed), Some(rules)) = (feed(cx), rules_path()) {
        let items = items(&rules, &settings(cx));
        feed.read(cx).send_items(items);
    }
}

fn settings(cx: &App) -> SyncSettings {
    settings_store::store(cx)
        .map(|store| store.read(cx).settings().sync.clone())
        .unwrap_or_default()
}

/// Saves `settings.json`'s `sync` and hands the change to the thread.
pub fn edit_settings(edit: impl FnOnce(&mut SyncSettings), cx: &mut App) -> Result<()> {
    let store = settings_store::store(cx).context("settings are not loaded")?;
    store.update(cx, |store, _| store.update(|s| edit(&mut s.sync)))?;
    let settings = settings(cx);
    if let Some(feed) = feed(cx) {
        feed.read(cx)
            .with_service(|service| service.set_targets(settings.targets.clone()));
    }
    follow_items(cx);
    Ok(())
}

/// Adds a folder, unless it is already synced.
pub fn add_target(target: Target, cx: &mut App) -> Result<()> {
    edit_settings(
        |sync| {
            if !sync.targets.iter().any(|t| t.root == target.root) {
                sync.targets.push(target);
            }
        },
        cx,
    )
}

/// The files that sync: the Finicky config in charge unless "Also sync"
/// is off, then `rules.json`, which records the config it ran after here.
/// The config goes first, so a folder never lists rules that ran after a
/// config it does not have yet (another machine would warn of it).
fn items(rules: &Path, settings: &SyncSettings) -> Vec<Item> {
    let config = crate::router::config_in_charge();
    let home = std::env::home_dir();
    let runs_after = config
        .as_deref()
        .map(|config| crate::config_view::display_path(config, home.as_deref()));
    let mut items = Vec::new();
    if settings.syncs_config()
        && let Some(config) = config
        && let Some(name) = config_name(&config)
    {
        items.push(Item {
            name,
            local: config,
            check: Arc::new(check_config),
            runs_after: None,
        });
    }
    items.push(Item {
        name: "rules.json".into(),
        local: rules.to_path_buf(),
        check: Arc::new(check_rules),
        runs_after,
    });
    items
}

/// The config's name in the folder: `finicky` and its own extension.
pub fn config_name(config: &Path) -> Option<String> {
    let ext = config.extension()?.to_str()?;
    matches!(ext, "js" | "ts" | "mjs" | "mts").then(|| format!("finicky.{ext}"))
}

/// `rules.json` must load; one with nothing in it is blank, so a new
/// machine takes a folder's rules without asking.
pub fn check_rules(bytes: &[u8]) -> Content {
    match serde_json::from_slice::<RulesFile>(bytes) {
        Err(e) => Content::Invalid(format!("not a rules file: {e}")),
        Ok(file) if file.is_empty() && file.destinations.is_empty() => Content::Blank,
        Ok(_) => Content::Valid,
    }
}

/// A config must be text. Whether it loads is the reloader's to say: a
/// broken one is left out and `rules.json` keeps routing.
pub fn check_config(bytes: &[u8]) -> Content {
    match std::str::from_utf8(bytes) {
        Ok(text) if text.trim().is_empty() => Content::Invalid("empty".into()),
        Ok(_) => Content::Valid,
        Err(_) => Content::Invalid("not text".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_are_checked_before_they_sync() {
        assert_eq!(check_rules(b"{}"), Content::Blank);
        assert_eq!(
            check_rules(br#"{"defaultBrowser":"","rules":null}"#),
            Content::Blank
        );
        assert_eq!(
            check_rules(br#"{"defaultBrowser":"Safari"}"#),
            Content::Valid
        );
        assert!(matches!(check_rules(b"{ half"), Content::Invalid(_)));
        assert!(matches!(
            check_rules(br#"{"rules":"x"}"#),
            Content::Invalid(_)
        ));
        assert!(matches!(check_config(b"  "), Content::Invalid(_)));
        assert_eq!(check_config(b"export default {}"), Content::Valid);
    }

    #[test]
    fn a_config_syncs_under_finickys_name_with_its_own_extension() {
        assert_eq!(
            config_name(Path::new("/h/.finicky.ts")).as_deref(),
            Some("finicky.ts")
        );
        assert_eq!(
            config_name(Path::new("/h/.config/finicky/finicky.js")).as_deref(),
            Some("finicky.js")
        );
        assert_eq!(config_name(Path::new("/h/finicky")), None);
    }

    fn status(healths: &[pwrroute_sync::Health], last_pass: Option<u64>) -> Status {
        Status {
            targets: healths
                .iter()
                .enumerate()
                .map(|(i, health)| pwrroute_sync::TargetStatus {
                    target: Target {
                        kind: pwrroute_sync::Kind::Dropbox,
                        label: format!("Folder {i}"),
                        root: PathBuf::from(format!("/f{i}")),
                    },
                    files: vec![pwrroute_sync::FileStatus {
                        name: "rules.json".into(),
                        health: health.clone(),
                    }],
                    last_synced: last_pass,
                    saved_by: None,
                    saved_at: None,
                    folder_files: Default::default(),
                })
                .collect(),
            last_pass,
            ..Status::default()
        }
    }

    #[test]
    fn one_line_says_how_sync_stands_worst_first() {
        use pwrroute_sync::{Health, Wait};
        assert_eq!(
            line_of(&Status::default(), 0),
            None,
            "nothing syncs: no line"
        );
        let ok = line_of(&status(&[Health::InSync, Health::Pushed], Some(0)), 120_000).unwrap();
        assert_eq!(
            (ok.text.as_str(), ok.tone, ok.chip),
            ("synced \u{00b7} 2 min ago", Tone::Quiet, None)
        );
        let one_down = line_of(
            &status(
                &[Health::InSync, Health::Unavailable("gone".into())],
                Some(0),
            ),
            1_000,
        )
        .unwrap();
        assert_eq!(
            one_down.text,
            "synced \u{00b7} just now \u{00b7} 1 unavailable"
        );
        assert_eq!(one_down.tone, Tone::Warn);
        let paused = line_of(&status(&[Health::Unavailable("gone".into())], Some(0)), 0).unwrap();
        assert_eq!(paused.chip, Some("sync \u{00b7} paused"));
        assert_eq!(paused.text, "sync paused \u{00b7} folder unavailable");
        // A file that does not load is not an unreachable folder.
        let broken = line_of(
            &status(&[Health::Error("does not load".into())], Some(0)),
            0,
        )
        .unwrap();
        assert_eq!(broken.text, "sync paused \u{00b7} needs you");
        assert_eq!(broken.tip.as_deref(), Some("Folder 0: does not load"));
        let one_broken = line_of(
            &status(&[Health::InSync, Health::Error("x".into())], Some(0)),
            0,
        )
        .unwrap();
        assert_eq!(
            one_broken.text,
            "synced \u{00b7} just now \u{00b7} 1 need you"
        );
        let syncing = line_of(&status(&[Health::Waiting(Wait::Settling)], Some(0)), 0).unwrap();
        assert_eq!(syncing.text, "syncing\u{2026}");
        let conflict = pwrroute_sync::Conflict {
            kind: pwrroute_sync::ConflictKind::BothChanged,
            local: None,
            remote: Vec::new(),
            remote_sha: String::new(),
            remote_by: None,
            remote_at: None,
        };
        let bad = line_of(
            &status(
                &[
                    Health::Unavailable("gone".into()),
                    Health::Conflict(Box::new(conflict)),
                ],
                Some(0),
            ),
            0,
        )
        .unwrap();
        assert_eq!((bad.text.as_str(), bad.tone), ("sync conflict", Tone::Bad));
        assert_eq!(
            bad.tip.as_deref(),
            Some("Folder 1: both changed since the last sync")
        );
        assert_eq!(age(3 * 3_600_000), "3 h ago");
    }

    #[test]
    fn the_machine_layer_is_empty_until_installed() {
        assert!(machine().is_empty());
    }
}
