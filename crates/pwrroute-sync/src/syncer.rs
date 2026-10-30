//! One machine's sync: its folders, its files, and a pass over them.
//!
//! A pass takes each synced file (`rules.json`, and the Finicky config
//! when that syncs too) and each folder in the order the user added them,
//! looks at both sides, and acts on [`decide`]'s answer:
//!
//! - **Push** writes the local file into the folder ([`Folder::write`]:
//!   staged, renamed, then the manifest). The folder's old copy is backed
//!   up first unless that version is already kept.
//! - **Pull** applies the folder's file locally, only once it has
//!   **settled**: its hash is the one the manifest lists, or (no manifest,
//!   or one that never catches up) it looked the same on two passes in a
//!   row. It must also pass the file's check (`rules.json` must parse).
//!   The local file is backed up first. A pull is a local change, so the
//!   folders after it in the same pass, and the next pass, push it on.
//! - **Conflict** writes nothing. Local keeps routing; the folder's version
//!   is backed up and handed to the UI, which calls [`Syncer::resolve`].
//!
//! A folder whose root is gone is unavailable, and a file iCloud has not
//! downloaded is waiting; both keep their base, so nothing is lost, and the
//! next pass tries again. Other folders carry on either way.

use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    decide::{ConflictKind, Decision, Local, decide},
    folder::{Folder, Machine, Manifest, Probe, Stamp, hash, write_atomic},
    places::Kind,
    state::{Backup, State},
};

/// What a synced file's bytes amount to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// Valid, and says nothing yet (`rules.json` with no rules or default).
    Blank,
    Valid,
    /// Not something to apply; the reason in words.
    Invalid(String),
}

pub type Check = Arc<dyn Fn(&[u8]) -> Content + Send + Sync>;

/// A file that syncs: its name in the folder and where it lives here.
#[derive(Clone)]
pub struct Item {
    /// `rules.json`, `finicky.ts`.
    pub name: String,
    pub local: PathBuf,
    pub check: Check,
}

impl std::fmt::Debug for Item {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Item")
            .field("name", &self.name)
            .field("local", &self.local)
            .finish_non_exhaustive()
    }
}

/// A folder the user chose to sync with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub kind: Kind,
    pub label: String,
    /// The cloud root; the folder is `<root>/PwrRoute`.
    pub root: PathBuf,
}

impl Target {
    /// The key its base and status go by: the root as written.
    pub fn id(&self) -> String {
        self.root.display().to_string()
    }

    pub fn folder(&self) -> Folder {
        Folder::new(&self.root)
    }
}

/// How one file stands with one folder after a pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    /// Neither side has the file.
    Absent,
    InSync,
    /// This pass wrote the local file into the folder.
    Pushed,
    /// This pass applied the folder's file here.
    Pulled,
    Waiting(Wait),
    /// The folder cannot be reached; the reason in words.
    Unavailable(String),
    Conflict(Box<Conflict>),
    /// Something to fix by hand: a file that does not parse, a write that
    /// failed. Words for the user.
    Error(String),
}

impl Health {
    /// Whether the two sides hold the same file after this pass.
    pub fn in_step(&self) -> bool {
        matches!(self, Self::Absent | Self::InSync | Self::Pushed | Self::Pulled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    /// iCloud has not downloaded it.
    NotDownloaded,
    /// Another machine's write is still arriving.
    Settling,
}

/// Two versions that both changed; neither is applied until the user
/// chooses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub kind: ConflictKind,
    /// This machine's file (`None`: there is none).
    pub local: Option<Vec<u8>>,
    pub remote: Vec<u8>,
    /// The folder's version, which [`Syncer::resolve`] checks is still the
    /// one shown.
    pub remote_sha: String,
    /// Who wrote the folder's version and when, from the manifest.
    pub remote_by: Option<Machine>,
    pub remote_at: Option<u64>,
}

/// One file in one folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
    pub name: String,
    pub health: Health,
}

/// One folder after a pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetStatus {
    pub target: Target,
    pub files: Vec<FileStatus>,
    /// When every file was last in step with it.
    pub last_synced: Option<u64>,
    /// The manifest's last writer and time.
    pub saved_by: Option<Machine>,
    pub saved_at: Option<u64>,
}

impl TargetStatus {
    /// The file that most needs the reader: a conflict, then an error, then
    /// unavailable, then waiting; else the first.
    pub fn worst(&self) -> Option<&FileStatus> {
        let rank = |h: &Health| match h {
            Health::Conflict(_) => 0,
            Health::Error(_) => 1,
            Health::Unavailable(_) => 2,
            Health::Waiting(_) => 3,
            Health::Pulled | Health::Pushed => 4,
            Health::InSync | Health::Absent => 5,
        };
        self.files.iter().min_by_key(|f| rank(&f.health))
    }
}

/// Everything sync knows, for the Settings pane and the status chip.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub machine: Machine,
    pub targets: Vec<TargetStatus>,
    /// When the last pass ran.
    pub last_pass: Option<u64>,
    /// Newest first.
    pub backups: Vec<Backup>,
}

/// Which side a conflict's resolution keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// This machine's version goes to the folder.
    KeepLocal,
    /// The folder's version replaces this machine's.
    UseFolder,
}

/// What a pass remembers about a folder's file between passes.
#[derive(Debug, Clone)]
struct Seen {
    stamp: Stamp,
    sha: String,
    bytes: Vec<u8>,
    /// Seen unchanged on an earlier pass too.
    stable: bool,
}

/// Largest file sync reads; a `rules.json` past this is not one PwrRoute
/// wrote.
const MAX_BYTES: u64 = 8 * 1024 * 1024;

pub struct Syncer {
    data_dir: PathBuf,
    machine_name: String,
    app: String,
    items: Vec<Item>,
    targets: Vec<Target>,
    state: State,
    seen: HashMap<(String, String), Seen>,
    status: Status,
}

impl std::fmt::Debug for Syncer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Syncer")
            .field("targets", &self.targets)
            .field("items", &self.items)
            .finish_non_exhaustive()
    }
}

impl Syncer {
    /// `data_dir` is PwrRoute's own; sync keeps `sync/state.json` and
    /// `sync/backups/` there.
    pub fn new(data_dir: &Path, machine_name: &str, app: &str) -> Self {
        let state = State::load(&Self::state_path(data_dir));
        Self {
            data_dir: data_dir.to_path_buf(),
            machine_name: machine_name.to_owned(),
            app: app.to_owned(),
            items: Vec::new(),
            targets: Vec::new(),
            state,
            seen: HashMap::new(),
            status: Status::default(),
        }
    }

    fn state_path(data_dir: &Path) -> PathBuf {
        data_dir.join("sync").join("state.json")
    }

    fn backups_dir(&self) -> PathBuf {
        self.data_dir.join("sync").join("backups")
    }

    pub fn machine(&self) -> Machine {
        Machine {
            id: self.state.machine_id.clone(),
            name: self.machine_name.clone(),
        }
    }

    pub fn set_machine_name(&mut self, name: &str) {
        name.clone_into(&mut self.machine_name);
    }

    pub fn set_items(&mut self, items: Vec<Item>) {
        self.items = items;
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Sets the folders. A folder no longer in the list is forgotten, so
    /// adding it back starts as a join (and asks before overwriting).
    pub fn set_targets(&mut self, targets: Vec<Target>) {
        for old in &self.targets {
            if !targets.iter().any(|t| t.id() == old.id()) {
                self.state.forget(&old.id());
                self.seen.retain(|(t, _), _| *t != old.id());
            }
        }
        self.targets = targets;
        self.save_state();
    }

    pub fn targets(&self) -> &[Target] {
        &self.targets
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    /// The local files' stamps, for [`crate::Schedule::tick`].
    pub fn local_stamps(&self) -> Vec<Option<Stamp>> {
        self.items
            .iter()
            .map(|item| fs::metadata(&item.local).ok().map(|m| Stamp::of(&m)))
            .collect()
    }

    /// One pass over every file and folder. `now` is milliseconds since the
    /// epoch.
    pub fn pass(&mut self, now: u64) -> &Status {
        let mut statuses: Vec<TargetStatus> = self
            .targets
            .iter()
            .map(|target| TargetStatus {
                target: target.clone(),
                files: Vec::new(),
                last_synced: None,
                saved_by: None,
                saved_at: None,
            })
            .collect();
        for item in self.items.clone() {
            for (index, target) in self.targets.clone().iter().enumerate() {
                let health = self.sync_one(&item, target, now);
                statuses[index].files.push(FileStatus {
                    name: item.name.clone(),
                    health,
                });
            }
        }
        for status in &mut statuses {
            let id = status.target.id();
            if status.files.iter().all(|f| f.health.in_step()) {
                self.state.last_synced.insert(id.clone(), now);
            }
            status.last_synced = self.state.last_synced.get(&id).copied();
            if let Some(manifest) = status.target.folder().manifest() {
                status.saved_by = Some(manifest.saved_by);
                status.saved_at = Some(manifest.saved_at);
            }
        }
        self.save_state();
        self.status = Status {
            machine: self.machine(),
            targets: statuses,
            last_pass: Some(now),
            backups: self.state.backups.iter().rev().cloned().collect(),
        };
        &self.status
    }

    fn sync_one(&mut self, item: &Item, target: &Target, now: u64) -> Health {
        let id = target.id();
        let folder = target.folder();
        let local = match read_local(&item.local) {
            Ok(local) => local,
            Err(e) => return Health::Error(format!("Reading this machine's {}: {e:#}", item.name)),
        };
        let local_content = local.as_deref().map(|b| (item.check)(b));
        if let Some(Content::Invalid(why)) = &local_content {
            return Health::Error(format!(
                "This machine's {} does not load ({why}); it syncs once it does.",
                item.name
            ));
        }
        let local_sha = local.as_deref().map(hash);
        let local_view = match (&local_sha, &local_content) {
            (Some(sha), Some(Content::Blank)) => Local::Blank(sha),
            (Some(sha), _) => Local::Present(sha),
            (None, _) => Local::Missing,
        };

        let remote = match folder.probe(&item.name) {
            Probe::Unavailable(why) => return Health::Unavailable(why),
            Probe::NotDownloaded => return Health::Waiting(Wait::NotDownloaded),
            Probe::Missing => {
                self.seen.remove(&(id.clone(), item.name.clone()));
                None
            }
            Probe::Present { stamp, .. } => match self.read_remote(&folder, item, &id, stamp) {
                Ok(seen) => Some(seen),
                // A File Provider file that cannot be downloaded right now
                // (offline) fails to read: wait for it.
                Err(e) => return Health::Unavailable(format!("{e:#}")),
            },
        };
        let manifest = folder.manifest();
        let base = self.state.base(&id, &item.name).map(str::to_owned);
        let decision = decide(local_view, remote.as_ref().map(|r| r.sha.as_str()), base.as_deref());
        let settled = remote
            .as_ref()
            .is_none_or(|r| r.stable || listed(manifest.as_ref(), &item.name, &r.sha));

        match decision {
            Decision::Nothing => Health::Absent,
            Decision::InSync => {
                if let Some(sha) = &local_sha {
                    self.state.set_base(&id, &item.name, sha);
                }
                Health::InSync
            }
            Decision::Push => {
                let Some(bytes) = local else {
                    return Health::Absent;
                };
                if let Some(old) = &remote {
                    let why = "Replaced by this machine's version";
                    if let Err(e) = self.back_up(&item.name, &old.bytes, &target.label, why, now) {
                        return Health::Error(format!("Backing up before syncing: {e:#}"));
                    }
                }
                match folder.write(&item.name, &bytes, &self.machine(), now, &self.app) {
                    Ok(()) => {
                        let sha = hash(&bytes);
                        self.state.set_base(&id, &item.name, &sha);
                        self.seen.remove(&(id, item.name.clone()));
                        Health::Pushed
                    }
                    Err(e) => Health::Error(format!("Writing to {}: {e:#}", target.label)),
                }
            }
            Decision::Pull | Decision::Conflict(_) if !settled => Health::Waiting(Wait::Settling),
            Decision::Pull => {
                let Some(remote) = remote else {
                    return Health::Absent;
                };
                if let Content::Invalid(why) = (item.check)(&remote.bytes) {
                    return Health::Error(format!(
                        "{} in {} does not load ({why}); it was not applied.",
                        item.name, target.label
                    ));
                }
                if let Some(old) = &local {
                    let why = format!("Before taking {}'s version", who(manifest.as_ref(), &item.name, &target.label));
                    if let Err(e) = self.back_up(&item.name, old, "This machine", &why, now) {
                        return Health::Error(format!("Backing up before syncing: {e:#}"));
                    }
                }
                match write_local(&item.local, &remote.bytes) {
                    Ok(()) => {
                        self.state.set_base(&id, &item.name, &remote.sha);
                        Health::Pulled
                    }
                    Err(e) => Health::Error(format!("Writing this machine's {}: {e:#}", item.name)),
                }
            }
            Decision::Conflict(kind) => {
                let Some(remote) = remote else {
                    return Health::Absent;
                };
                // Keep the folder's version whatever happens to the folder
                // next.
                let why = "Set aside in a conflict";
                if let Err(e) = self.back_up(&item.name, &remote.bytes, &target.label, why, now) {
                    log::warn!("sync: backing up a conflict: {e:#}");
                }
                let entry = manifest.as_ref().and_then(|m| m.files.get(&item.name));
                Health::Conflict(Box::new(Conflict {
                    kind,
                    local,
                    remote_sha: remote.sha.clone(),
                    remote: remote.bytes,
                    remote_by: entry
                        .filter(|e| e.sha256 == remote.sha)
                        .map(|e| e.saved_by.clone()),
                    remote_at: entry.filter(|e| e.sha256 == remote.sha).map(|e| e.saved_at),
                }))
            }
        }
    }

    /// Reads the folder's file, or reuses the last read when its stamp has
    /// not changed (so an evicted file is not downloaded on every pass).
    fn read_remote(&mut self, folder: &Folder, item: &Item, id: &str, stamp: Stamp) -> Result<Seen> {
        let key = (id.to_owned(), item.name.clone());
        if let Some(seen) = self.seen.get_mut(&key)
            && seen.stamp == stamp
        {
            seen.stable = true;
            return Ok(seen.clone());
        }
        if stamp.len > MAX_BYTES {
            bail!("{} is too large to be PwrRoute's", item.name);
        }
        let path = folder.file(&item.name);
        let bytes = fs::read(&path).with_context(|| format!("reading {}", item.name))?;
        let sha = hash(&bytes);
        // Read again by stamp: a file still being written changes under us.
        let after = fs::metadata(&path).map(|m| Stamp::of(&m)).ok();
        let stamp = after.unwrap_or(stamp);
        let stable = self
            .seen
            .get(&key)
            .is_some_and(|old| old.sha == sha && old.stamp == stamp);
        let seen = Seen {
            stamp,
            sha,
            bytes,
            stable,
        };
        self.seen.insert(key, seen.clone());
        Ok(seen)
    }

    fn back_up(&mut self, name: &str, bytes: &[u8], from: &str, why: &str, now: u64) -> Result<()> {
        let dir = self.backups_dir();
        self.state.back_up(&dir, name, bytes, from, why, now).map(|_| ())
    }

    /// Settles a conflict the UI showed. `remote_sha` is the folder's
    /// version as shown; if the folder changed since, nothing is written
    /// and the next pass shows the new state. The version not kept is
    /// backed up first.
    pub fn resolve(&mut self, target_root: &Path, name: &str, remote_sha: &str, choice: Choice, now: u64) -> Result<()> {
        let target = self
            .targets
            .iter()
            .find(|t| t.root == target_root)
            .cloned()
            .context("that folder is no longer synced")?;
        let item = self
            .items
            .iter()
            .find(|i| i.name == name)
            .cloned()
            .context("that file no longer syncs")?;
        let folder = target.folder();
        let id = target.id();
        let remote = fs::read(folder.file(name)).with_context(|| format!("reading {name} in {}", target.label))?;
        if hash(&remote) != remote_sha {
            bail!("{name} in {} changed again; look again before choosing", target.label);
        }
        let local = read_local(&item.local)?;
        match choice {
            Choice::KeepLocal => {
                let Some(bytes) = local else {
                    bail!("this machine has no {name} to keep");
                };
                self.back_up(name, &remote, &target.label, "Not kept in a conflict", now)?;
                folder.write(name, &bytes, &self.machine(), now, &self.app)?;
                self.state.set_base(&id, name, &hash(&bytes));
            }
            Choice::UseFolder => {
                if let Content::Invalid(why) = (item.check)(&remote) {
                    bail!("{name} in {} does not load ({why})", target.label);
                }
                if let Some(old) = &local {
                    self.back_up(name, old, "This machine", "Not kept in a conflict", now)?;
                }
                write_local(&item.local, &remote)?;
                self.state.set_base(&id, name, remote_sha);
            }
        }
        self.seen.remove(&(id, name.to_owned()));
        self.save_state();
        Ok(())
    }

    fn save_state(&self) {
        if let Err(e) = self.state.save(&Self::state_path(&self.data_dir)) {
            log::warn!("sync: saving state: {e:#}");
        }
    }
}

/// Whether the manifest lists this version of the file.
fn listed(manifest: Option<&Manifest>, name: &str, sha: &str) -> bool {
    manifest
        .and_then(|m| m.files.get(name))
        .is_some_and(|e| e.sha256 == sha)
}

/// The machine that wrote the folder's file, by name, else the folder.
fn who(manifest: Option<&Manifest>, name: &str, folder: &str) -> String {
    manifest
        .and_then(|m| m.files.get(name))
        .map(|e| e.saved_by.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| folder.to_owned())
}

fn read_local(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| path.display().to_string()),
    }
}

fn write_local(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| dir.display().to_string())?;
    }
    write_atomic(path, bytes, "sync")
}
