//! Synced rules in the app: the sync thread, what it last reported, and
//! this machine's layer (`machine.json`).
//!
//! `install` starts a [`pwrroute_sync::Service`] over `rules.json` (and the
//! Finicky config in charge, when Settings › Sync says so) and the folders
//! `settings.json`'s `sync.targets` lists. Its reports land in the
//! [`SyncFeed`] entity, which Settings › Sync, the rail foot and the tray
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

/// Edits `machine.json` and saves it. The router's reloader follows the
/// file; the window's engines read [`machine`] on their next build.
pub fn edit_machine(edit: impl FnOnce(&mut MachineFile), cx: &mut App) -> Result<()> {
    let path = rules_path()
        .map(|p| MachineFile::path_beside(&p))
        .context("no configuration directory")?;
    let mut machine = MachineFile::load(&path).unwrap_or_else(|_| machine());
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
    /// The cloud folders found on this machine, read when Settings › Sync
    /// opens ([`SyncFeed::look_for_places`]), never at launch.
    pub places: Vec<Place>,
    service: Option<Service>,
}

impl SyncFeed {
    fn with_service(&self, f: impl FnOnce(&Service)) {
        if let Some(service) = &self.service {
            f(service);
        }
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

/// The whole app's sync state, worst first: a conflict in any folder,
/// paused (every folder unavailable), syncing, synced. `None` while
/// nothing syncs. `now` and `last_pass` are milliseconds.
pub fn line_of(status: &Status, now: u64) -> Option<Line> {
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
        return Some(Line {
            text: "sync conflict".into(),
            tone: Tone::Bad,
            chip: Some("sync \u{00b7} conflict"),
            tip: Some(format!(
                "{}: both changed since the last sync",
                target.target.label
            )),
        });
    }
    let down = |t: &&pwrroute_sync::TargetStatus| {
        matches!(worst(t), Some(Health::Unavailable(_) | Health::Error(_)))
    };
    let unavailable = status.targets.iter().filter(down).count();
    if unavailable == status.targets.len() {
        return Some(Line {
            text: "sync paused \u{00b7} folder unavailable".into(),
            tone: Tone::Warn,
            chip: Some("sync \u{00b7} paused"),
            tip: status
                .targets
                .first()
                .map(|t| format!("{}: not reachable", t.target.label)),
        });
    }
    let settling = status
        .targets
        .iter()
        .any(|t| matches!(worst(t), Some(Health::Waiting(Wait::Settling))));
    let Some(last) = status.last_pass.filter(|_| !settling) else {
        return Some(Line {
            text: "syncing\u{2026}".into(),
            tone: Tone::Quiet,
            chip: None,
            tip: None,
        });
    };
    let mut text = format!("synced \u{00b7} {}", age(now.saturating_sub(last)));
    let mut tone = Tone::Quiet;
    if unavailable > 0 {
        text.push_str(&format!(" \u{00b7} {unavailable} unavailable"));
        tone = Tone::Warn;
    }
    Some(Line {
        text,
        tone,
        chip: None,
        tip: None,
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
    syncer.set_items(items(&rules, &settings));
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
    crate::settings::sync_host::refresh(cx);
}

/// Runs a pass now: Sync now, the window coming to the front, an agent's
/// write.
pub fn wake(cx: &App) {
    if let Some(feed) = feed(cx) {
        feed.read(cx).with_service(Service::wake);
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
    if let (Some(feed), Some(rules)) = (feed(cx), rules_path()) {
        let items = items(&rules, &settings);
        feed.read(cx).with_service(|service| {
            service.set_items(items);
            service.set_targets(settings.targets.clone());
        });
    }
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

/// The files that sync: `rules.json`, and the Finicky config in charge
/// when `config` is on.
fn items(rules: &Path, settings: &SyncSettings) -> Vec<Item> {
    let mut items = vec![Item {
        name: "rules.json".into(),
        local: rules.to_path_buf(),
        check: Arc::new(check_rules),
    }];
    if settings.config
        && let Some(config) = crate::router::config_in_charge()
        && let Some(name) = config_name(&config)
    {
        items.push(Item {
            name,
            local: config,
            check: Arc::new(check_config),
        });
    }
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
fn check_config(bytes: &[u8]) -> Content {
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
