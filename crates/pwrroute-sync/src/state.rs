//! What this machine remembers between runs: `<data dir>/sync/state.json`.
//!
//! It never syncs. It holds this machine's sync id, the **base** for each
//! folder and file (the hash both last agreed on, which
//! [`crate::decide`] needs), when each folder last synced, and the list of
//! backups, so pruning them never lists a directory.

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::folder::{hash, write_atomic};

/// How many backups are kept; the oldest go first.
pub const KEEP_BACKUPS: usize = 20;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    /// This machine's id in manifests: random, made once.
    #[serde(default)]
    pub machine_id: String,
    /// By target id, then file name: the hash both sides last agreed on.
    #[serde(default)]
    pub bases: BTreeMap<String, BTreeMap<String, String>>,
    /// By target id: when a pass last found every file in step.
    #[serde(default)]
    pub last_synced: BTreeMap<String, u64>,
    /// Oldest first.
    #[serde(default)]
    pub backups: Vec<Backup>,
}

/// A copy of a version that sync replaced or set aside.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    /// The file it is a version of: `rules.json`, `finicky.ts`.
    pub name: String,
    /// Where it came from: "This machine", or the folder's label.
    pub from: String,
    /// Why it was kept, in words: "Before taking MacBook Air's rules".
    pub why: String,
    /// Milliseconds since the epoch.
    pub at: u64,
    pub sha256: String,
    /// The copy, under the backups directory.
    pub path: PathBuf,
}

impl State {
    /// Reads the state; a missing or broken file is a fresh one, with a new
    /// machine id. Bases lost that way only mean the next pass asks about
    /// differences it cannot place: never a silent overwrite.
    pub fn load(path: &Path) -> Self {
        let mut state: Self = match fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                log::warn!("sync: state.json does not parse, starting fresh: {e}");
                Self::default()
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                log::warn!("sync: reading state.json: {e}");
                Self::default()
            }
        };
        if state.machine_id.is_empty() {
            state.machine_id = new_id();
        }
        state
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).with_context(|| dir.display().to_string())?;
        }
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        write_atomic(path, text.as_bytes(), "state")
    }

    pub fn base(&self, target: &str, name: &str) -> Option<&str> {
        self.bases.get(target)?.get(name).map(String::as_str)
    }

    pub fn set_base(&mut self, target: &str, name: &str, sha: &str) {
        self.bases
            .entry(target.to_owned())
            .or_default()
            .insert(name.to_owned(), sha.to_owned());
    }

    /// Forgets a folder the user removed, so adding it again asks first.
    pub fn forget(&mut self, target: &str) {
        self.bases.remove(target);
        self.last_synced.remove(target);
    }

    /// Copies `bytes` into `dir` unless a backup of the same file with the
    /// same hash is already kept, then drops the oldest past
    /// [`KEEP_BACKUPS`]. Returns whether a copy was written.
    pub fn back_up(
        &mut self,
        dir: &Path,
        name: &str,
        bytes: &[u8],
        from: &str,
        why: &str,
        at: u64,
    ) -> Result<bool> {
        let sha = hash(bytes);
        if self
            .backups
            .iter()
            .any(|b| b.name == name && b.sha256 == sha)
        {
            return Ok(false);
        }
        fs::create_dir_all(dir).with_context(|| dir.display().to_string())?;
        let (stem, ext) = match name.rsplit_once('.') {
            Some((stem, ext)) => (stem, format!(".{ext}")),
            None => (name, String::new()),
        };
        let path = dir.join(format!("{at}-{stem}-{}{ext}", &sha[..8]));
        write_atomic(&path, bytes, "backup")?;
        self.backups.push(Backup {
            name: name.to_owned(),
            from: from.to_owned(),
            why: why.to_owned(),
            at,
            sha256: sha,
            path,
        });
        while self.backups.len() > KEEP_BACKUPS {
            let old = self.backups.remove(0);
            if let Err(e) = fs::remove_file(&old.path)
                && e.kind() != io::ErrorKind::NotFound
            {
                log::warn!("sync: removing an old backup: {e}");
            }
        }
        Ok(true)
    }
}

/// A random-enough id for a machine: the hash of the time, the process and
/// a stack address, cut to 16 hex digits. It names a machine in manifests;
/// nothing depends on it being secret.
fn new_id() -> String {
    let marker = 0_u8;
    let seed = format!(
        "{:?}-{}-{:p}-{:?}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default(),
        std::process::id(),
        &marker,
        std::env::home_dir(),
    );
    hash(seed.as_bytes())[..16].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_state_gets_an_id_that_survives_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sync/state.json");
        let mut state = State::load(&path);
        assert_eq!(state.machine_id.len(), 16);
        state.set_base("t", "rules.json", "abc");
        state.save(&path).unwrap();
        let again = State::load(&path);
        assert_eq!(again.machine_id, state.machine_id);
        assert_eq!(again.base("t", "rules.json"), Some("abc"));
        fs::write(&path, "{ broken").unwrap();
        assert!(State::load(&path).bases.is_empty());
    }

    #[test]
    fn backups_dedupe_by_hash_and_keep_the_newest() {
        let dir = tempfile::tempdir().unwrap();
        let backups = dir.path().join("backups");
        let mut state = State::default();
        assert!(
            state
                .back_up(&backups, "rules.json", b"a", "This machine", "why", 1)
                .unwrap()
        );
        assert!(
            !state
                .back_up(&backups, "rules.json", b"a", "This machine", "why", 2)
                .unwrap()
        );
        for i in 0..KEEP_BACKUPS + 3 {
            state
                .back_up(
                    &backups,
                    "rules.json",
                    i.to_string().as_bytes(),
                    "Dropbox",
                    "why",
                    10 + i as u64,
                )
                .unwrap();
        }
        assert_eq!(state.backups.len(), KEEP_BACKUPS);
        assert!(!state.backups.iter().any(|b| b.sha256 == hash(b"a")));
        let on_disk = fs::read_dir(&backups).unwrap().count();
        assert_eq!(on_disk, KEEP_BACKUPS, "pruned files are gone too");
        let newest = state.backups.last().unwrap();
        assert!(newest.path.to_string_lossy().ends_with(".json"));
        assert_eq!(
            fs::read(&newest.path).unwrap(),
            (KEEP_BACKUPS + 2).to_string().as_bytes()
        );
    }
}
