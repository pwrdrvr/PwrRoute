//! `settings.json` in the data directory: what Settings keeps between
//! launches, as plain data. Today that is the Activity retention and the
//! journal (Settings › Privacy); other panes join as their producers land.
//!
//! [`Settings::load`] reads the file (absent means the defaults) and
//! [`Settings::save`] writes it through a sibling file and a rename, like
//! `rules.json`. [`spawn_router`](crate::router) reads it before the router
//! starts, so the retention and the journal are in force from the first
//! link; a change from Settings is saved here and handed to the running
//! router with `RouterHandle::set_retention` / `set_journal`, so it takes
//! effect at once. Nothing here holds a URL: the file carries numbers and
//! a level, never a link.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result};
use gpui_kit::{App, AppContext as _, Entity, Global};
use pwrroute_router::{JournalConfig, JournalMode, Retention, RouterHandle};
use serde::{Deserialize, Serialize};

/// The file in `pwrroute_os::data_dir()`.
pub const SETTINGS_FILE: &str = "settings.json";

/// The journal's directory under the data directory; its files are
/// `activity-YYYY-MM-DD.jsonl`.
pub const JOURNAL_DIR: &str = "journal";

/// Everything the file holds. Field order is the file's; a missing field
/// is its default, so an older file reads back whole.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub activity: ActivitySettings,
}

/// Settings › Privacy: what the router keeps of each link, where, and for
/// how long.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ActivitySettings {
    /// The in-memory buffer: the newest `count`, each for at most `hours`.
    pub keep: Retention,
    /// The on-disk journal, off by default.
    pub journal: JournalLevel,
    /// How many days of journal files to keep, today included.
    pub journal_days: u32,
}

impl Default for ActivitySettings {
    fn default() -> Self {
        Self {
            keep: Retention::default(),
            journal: JournalLevel::Off,
            journal_days: 7,
        }
    }
}

/// The journal switch: off, or on at one of the router's two modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JournalLevel {
    #[default]
    Off,
    Hosts,
    Full,
}

impl JournalLevel {
    /// The router's mode, when the journal is on.
    pub fn mode(self) -> Option<JournalMode> {
        match self {
            Self::Off => None,
            Self::Hosts => Some(JournalMode::Hosts),
            Self::Full => Some(JournalMode::Full),
        }
    }
}

impl Settings {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join(SETTINGS_FILE)
    }

    /// Reads the file. A missing file is the defaults.
    pub fn load(dir: &Path) -> Result<Self> {
        let path = Self::path(dir);
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).with_context(|| path.display().to_string()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| path.display().to_string()),
        }
    }

    /// Writes the file, creating the directory. Through a sibling file and a
    /// rename, so a crash leaves the old file or the new one, never half.
    pub fn save(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir).with_context(|| dir.display().to_string())?;
        let path = Self::path(dir);
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        let mut staging = path.as_os_str().to_owned();
        staging.push(".saving");
        fs::write(&staging, text).with_context(|| path.display().to_string())?;
        fs::rename(&staging, &path).with_context(|| path.display().to_string())
    }

    /// The router's in-memory retention.
    pub fn retention(&self) -> Retention {
        self.activity.keep
    }

    /// The router's journal, when it is on: in `JOURNAL_DIR` under `dir`.
    pub fn journal(&self, dir: &Path) -> Option<JournalConfig> {
        Some(JournalConfig {
            dir: dir.join(JOURNAL_DIR),
            mode: self.activity.journal.mode()?,
            days: self.activity.journal_days.max(1),
        })
    }
}

/// The settings as loaded, the directory they came from, and the router
/// they are applied to.
#[derive(Clone, Debug)]
pub struct SettingsStore {
    /// The data directory; `None` when the OS gives none, and then nothing
    /// is saved and the journal cannot be turned on.
    dir: Option<PathBuf>,
    settings: Settings,
}

impl SettingsStore {
    /// Reads `settings.json` under `dir`. A file that cannot be read is
    /// reported on stderr and the defaults apply: a bad file must not keep
    /// the router from starting.
    pub fn load(dir: Option<PathBuf>) -> Self {
        let settings = match dir.as_deref().map(Settings::load) {
            Some(Ok(settings)) => settings,
            Some(Err(error)) => {
                eprintln!("pwrroute: cannot read settings: {error:#}; using the defaults");
                Settings::default()
            }
            None => Settings::default(),
        };
        Self { dir, settings }
    }

    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// The router's retention, as set.
    pub fn retention(&self) -> Retention {
        self.settings.retention()
    }

    /// The router's journal config, when the journal is on and there is a
    /// data directory to write it in.
    pub fn journal(&self) -> Option<JournalConfig> {
        self.settings.journal(self.dir.as_deref()?)
    }

    /// Where the journal's files go, on or off.
    pub fn journal_dir(&self) -> Option<PathBuf> {
        self.dir.as_deref().map(|dir| dir.join(JOURNAL_DIR))
    }

    /// Edits the settings and writes the file. The edit is kept in memory
    /// even when the write fails, so the running router and the window
    /// agree; the error says the next launch will not.
    pub fn update(&mut self, edit: impl FnOnce(&mut Settings)) -> Result<()> {
        edit(&mut self.settings);
        match &self.dir {
            Some(dir) => self.settings.save(dir),
            None => Err(anyhow::anyhow!("no data directory: settings are not saved")),
        }
    }
}

/// The one store and the router it configures, set by [`install`].
struct StoreGlobal {
    store: Entity<SettingsStore>,
    router: Option<RouterHandle>,
}

impl Global for StoreGlobal {}

/// Makes `store` the one Settings edits, applied to `router` when there is
/// one (the window harness runs without).
pub fn install(store: SettingsStore, router: Option<RouterHandle>, cx: &mut App) {
    let store = cx.new(|_| store);
    cx.set_global(StoreGlobal { store, router });
}

/// The store, once [`install`] has run.
pub fn store(cx: &App) -> Option<Entity<SettingsStore>> {
    cx.try_global::<StoreGlobal>().map(|g| g.store.clone())
}

/// The router the settings apply to, once [`install`] has run with one.
pub fn router(cx: &App) -> Option<RouterHandle> {
    cx.try_global::<StoreGlobal>()
        .and_then(|g| g.router.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_file_is_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Settings::load(dir.path()).unwrap();
        assert_eq!(settings, Settings::default());
        assert_eq!(settings.retention(), Retention::default());
        assert_eq!(settings.activity.journal, JournalLevel::Off);
        assert_eq!(settings.activity.journal_days, 7);
        assert_eq!(settings.journal(dir.path()), None);
        // Nothing was written by reading.
        assert!(!Settings::path(dir.path()).exists());

        let store = SettingsStore::load(Some(dir.path().to_path_buf()));
        assert_eq!(store.journal(), None);
        assert_eq!(store.journal_dir(), Some(dir.path().join("journal")));
        let none = SettingsStore::load(None);
        assert_eq!(none.journal_dir(), None);
        assert_eq!(none.retention(), Retention::default());
    }

    #[test]
    fn settings_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SettingsStore::load(Some(dir.path().to_path_buf()));
        store
            .update(|settings| {
                settings.activity.keep = Retention {
                    count: 200,
                    hours: 72,
                };
                settings.activity.journal = JournalLevel::Hosts;
                settings.activity.journal_days = 3;
            })
            .unwrap();
        let text = fs::read_to_string(Settings::path(dir.path())).unwrap();
        assert_eq!(
            text,
            "{\n  \"activity\": {\n    \"keep\": {\n      \"count\": 200,\n      \"hours\": 72\n    },\n    \"journal\": \"hosts\",\n    \"journalDays\": 3\n  }\n}\n"
        );
        assert!(!dir.path().join("settings.json.saving").exists());

        let again = SettingsStore::load(Some(dir.path().to_path_buf()));
        assert_eq!(again.settings(), store.settings());
        assert_eq!(
            again.journal(),
            Some(JournalConfig {
                dir: dir.path().join("journal"),
                mode: JournalMode::Hosts,
                days: 3,
            })
        );

        // Full URLs, then off: the mode follows the level, and off is no
        // config at all.
        store
            .update(|settings| settings.activity.journal = JournalLevel::Full)
            .unwrap();
        assert_eq!(store.journal().unwrap().mode, JournalMode::Full);
        store
            .update(|settings| settings.activity.journal = JournalLevel::Off)
            .unwrap();
        assert_eq!(store.journal(), None);
        assert_eq!(
            SettingsStore::load(Some(dir.path().to_path_buf())).journal(),
            None
        );
    }

    #[test]
    fn a_partial_or_older_file_fills_in_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            Settings::path(dir.path()),
            r#"{ "activity": { "journal": "full" } }"#,
        )
        .unwrap();
        let settings = Settings::load(dir.path()).unwrap();
        assert_eq!(settings.activity.journal, JournalLevel::Full);
        assert_eq!(settings.activity.keep, Retention::default());
        assert_eq!(settings.activity.journal_days, 7);
        // Zero days would be no journal file at all; the router keeps one.
        fs::write(
            Settings::path(dir.path()),
            r#"{ "activity": { "journal": "hosts", "journalDays": 0 } }"#,
        )
        .unwrap();
        assert_eq!(
            Settings::load(dir.path())
                .unwrap()
                .journal(dir.path())
                .unwrap()
                .days,
            1
        );
    }

    #[test]
    fn a_bad_file_reports_and_keeps_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(Settings::path(dir.path()), "{ not json").unwrap();
        assert!(Settings::load(dir.path()).is_err());
        let store = SettingsStore::load(Some(dir.path().to_path_buf()));
        assert_eq!(store.settings(), &Settings::default());
        // Without a directory the edit holds in memory and the save says so.
        let mut none = SettingsStore::load(None);
        let result = none.update(|settings| settings.activity.journal_days = 2);
        assert!(result.is_err());
        assert_eq!(none.settings().activity.journal_days, 2);
    }
}
