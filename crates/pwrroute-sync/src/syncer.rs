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
    collections::{BTreeMap, HashMap},
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    LocalFile,
    decide::{ConflictKind, Decision, Local, decide},
    folder::{FileEntry, Folder, Machine, Manifest, Probe, Stamp, hash, write_atomic},
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
    /// The shown path of the config this file runs after on this machine
    /// (`~/.finicky.js`), set by the app for `rules.json` only: a Finicky
    /// config's handlers run before `rules.json`'s rules. Recorded in the
    /// manifest ([`FileEntry::runs_after`]) with each version this machine
    /// writes, so another machine can say so.
    pub runs_after: Option<String>,
}

impl std::fmt::Debug for Item {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Item")
            .field("name", &self.name)
            .field("local", &self.local)
            .field("runs_after", &self.runs_after)
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
        matches!(
            self,
            Self::Absent | Self::InSync | Self::Pushed | Self::Pulled
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    /// iCloud has not downloaded it.
    NotDownloaded,
    /// A remote observation or the local apply snapshot changed; retry.
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
    /// The manifest's files as this pass read them, by name: what the
    /// folder holds (`finicky.js`, say) and who wrote each, with
    /// [`FileEntry::runs_after`]. Empty with no manifest or no folder.
    pub folder_files: BTreeMap<String, FileEntry>,
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

/// A read either observed one metadata revision throughout, or must be
/// retried on another pass. Only coherent observations enter `seen`.
enum Observation {
    Coherent(Seen),
    Settling,
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

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum IoPoint {
    RemoteRead,
    RemoteManifest,
    RemoteWrite,
    LocalBackup,
}

#[cfg(test)]
type IoHook = Arc<dyn Fn(IoPoint, &Path) + Send + Sync>;

pub struct Syncer {
    data_dir: PathBuf,
    machine_name: String,
    app: String,
    items: Vec<Item>,
    targets: Vec<Target>,
    state: State,
    seen: HashMap<(String, String), Seen>,
    status: Status,
    #[cfg(test)]
    io_hook: Option<IoHook>,
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
            #[cfg(test)]
            io_hook: None,
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
    /// adding it back starts as a join (and asks before overwriting), and
    /// leaves the status at once. Returns whether the status lost a folder.
    pub fn set_targets(&mut self, targets: Vec<Target>) -> bool {
        for old in &self.targets {
            if !targets.iter().any(|t| t.id() == old.id()) {
                self.state.forget(&old.id());
                self.seen.retain(|(t, _), _| *t != old.id());
            }
        }
        let before = self.status.targets.len();
        self.status
            .targets
            .retain(|s| targets.iter().any(|t| t.id() == s.target.id()));
        self.targets = targets;
        self.save_state();
        self.status.targets.len() != before
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
                folder_files: BTreeMap::new(),
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
                status.folder_files = manifest.files;
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

        let remote = match folder.probe(&item.name) {
            Probe::Unavailable(why) => return Health::Unavailable(why),
            Probe::NotDownloaded => return Health::Waiting(Wait::NotDownloaded),
            Probe::Missing => {
                self.seen.remove(&(id.clone(), item.name.clone()));
                None
            }
            Probe::Present { stamp, .. } => match self.read_remote(&folder, item, &id, stamp) {
                Ok(Observation::Coherent(seen)) => Some(seen),
                Ok(Observation::Settling) => return Health::Waiting(Wait::Settling),
                // A File Provider file that cannot be downloaded right now
                // (offline) fails to read: wait for it.
                Err(e) => {
                    return Health::Unavailable(match e.root_cause().downcast_ref::<io::Error>() {
                        Some(io) => crate::folder::describe(io),
                        None => format!("{e:#}"),
                    });
                }
            },
        };
        #[cfg(test)]
        if let Some(hook) = &self.io_hook {
            hook(IoPoint::RemoteManifest, &folder.file(crate::MANIFEST));
        }
        let manifest = folder.manifest();
        // Cloud reads can block on a provider. Only after they finish do we
        // enter the local revision/backup/apply boundary shared with app and
        // MCP writers. No cooperating save can slip between check and rename.
        let held = match LocalFile::lock(&item.local) {
            Ok(held) => held,
            Err(e) => return Health::Error(format!("Locking this machine's {}: {e}", item.name)),
        };
        let local = match read_local(held.path()) {
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

        let base = self.state.base(&id, &item.name).map(str::to_owned);
        let decision = decide(
            local_view,
            remote.as_ref().map(|r| r.sha.as_str()),
            base.as_deref(),
        );
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
                // A push only uses this snapshot; release the local lock
                // before writing to a potentially blocked cloud provider.
                drop(held);
                if let Some(old) = &remote {
                    let why = "Replaced by this machine's version";
                    if let Err(e) = self.back_up(&item.name, &old.bytes, &target.label, why, now) {
                        return Health::Error(format!("Backing up before syncing: {e:#}"));
                    }
                }
                #[cfg(test)]
                if let Some(hook) = &self.io_hook {
                    hook(IoPoint::RemoteWrite, &folder.file(&item.name));
                }
                let written = folder.write(
                    &item.name,
                    &bytes,
                    &self.machine(),
                    now,
                    &self.app,
                    item.runs_after.as_deref(),
                );
                match written {
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
                    let why = format!(
                        "Before taking {}'s version",
                        who(manifest.as_ref(), &item.name, &target.label)
                    );
                    if let Err(e) = self.back_up(&item.name, old, "This machine", &why, now) {
                        return Health::Error(format!("Backing up before syncing: {e:#}"));
                    }
                }
                #[cfg(test)]
                if let Some(hook) = &self.io_hook {
                    hook(IoPoint::LocalBackup, &item.local);
                }
                // Hand editors do not take our advisory lock. Refuse an
                // observable edit during validation/backup; a noncooperating
                // write after this check is still outside our guarantee.
                match read_local(held.path()) {
                    Ok(current) if current == local => {}
                    Ok(_) => return Health::Waiting(Wait::Settling),
                    Err(e) => {
                        return Health::Error(format!(
                            "Rechecking this machine's {}: {e:#}",
                            item.name
                        ));
                    }
                }
                match write_local(held.path(), &remote.bytes) {
                    Ok(()) => {
                        self.state.set_base(&id, &item.name, &remote.sha);
                        Health::Pulled
                    }
                    Err(e) => Health::Error(format!("Writing this machine's {}: {e:#}", item.name)),
                }
            }
            Decision::Conflict(kind) => {
                drop(held);
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
    fn read_remote(
        &mut self,
        folder: &Folder,
        item: &Item,
        id: &str,
        stamp: Stamp,
    ) -> Result<Observation> {
        let key = (id.to_owned(), item.name.clone());
        if let Some(seen) = self.seen.get_mut(&key)
            && seen.stamp == stamp
        {
            seen.stable = true;
            return Ok(Observation::Coherent(seen.clone()));
        }
        self.seen.remove(&key);
        if stamp.len > MAX_BYTES {
            bail!("{} is too large to be PwrRoute's", item.name);
        }
        let path = folder.file(&item.name);
        let bytes = fs::read(&path).with_context(|| format!("reading {}", item.name))?;
        #[cfg(test)]
        if let Some(hook) = &self.io_hook {
            hook(IoPoint::RemoteRead, &path);
        }
        let sha = hash(&bytes);
        // Read again by stamp: a file still being written changes under us.
        let after = fs::metadata(&path).map(|m| Stamp::of(&m));
        match after {
            Ok(after) if after == stamp && bytes.len() as u64 == stamp.len => {}
            Ok(_) => return Ok(Observation::Settling),
            Err(e) => {
                return Err(e).with_context(|| format!("rechecking {}", item.name));
            }
        }
        let seen = Seen {
            stamp,
            sha,
            bytes,
            stable: false,
        };
        self.seen.insert(key, seen.clone());
        Ok(Observation::Coherent(seen))
    }

    fn back_up(&mut self, name: &str, bytes: &[u8], from: &str, why: &str, now: u64) -> Result<()> {
        let dir = self.backups_dir();
        self.state
            .back_up(&dir, name, bytes, from, why, now)
            .map(|_| ())
    }

    /// Settles a conflict the UI showed. `remote_sha` is the folder's
    /// version as shown; if the folder changed since, nothing is written
    /// and the next pass shows the new state. The version not kept is
    /// backed up first.
    pub fn resolve(
        &mut self,
        target_root: &Path,
        name: &str,
        remote_sha: &str,
        choice: Choice,
        now: u64,
    ) -> Result<()> {
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
        let remote = fs::read(folder.file(name))
            .with_context(|| format!("reading {name} in {}", target.label))?;
        if hash(&remote) != remote_sha {
            bail!(
                "{name} in {} changed again; look again before choosing",
                target.label
            );
        }
        let held = LocalFile::lock(&item.local)?;
        let local = read_local(held.path())?;
        match choice {
            Choice::KeepLocal => {
                let Some(bytes) = local else {
                    bail!("this machine has no {name} to keep");
                };
                drop(held);
                self.back_up(name, &remote, &target.label, "Not kept in a conflict", now)?;
                let runs_after = item.runs_after.as_deref();
                #[cfg(test)]
                if let Some(hook) = &self.io_hook {
                    hook(IoPoint::RemoteWrite, &folder.file(name));
                }
                folder.write(name, &bytes, &self.machine(), now, &self.app, runs_after)?;
                self.state.set_base(&id, name, &hash(&bytes));
            }
            Choice::UseFolder => {
                if let Content::Invalid(why) = (item.check)(&remote) {
                    bail!("{name} in {} does not load ({why})", target.label);
                }
                if let Some(old) = &local {
                    self.back_up(name, old, "This machine", "Not kept in a conflict", now)?;
                }
                #[cfg(test)]
                if let Some(hook) = &self.io_hook {
                    hook(IoPoint::LocalBackup, &item.local);
                }
                if read_local(held.path())? != local {
                    bail!("{name} on this machine changed again; look again before choosing");
                }
                write_local(held.path(), &remote)?;
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

/// Writes the target resolved by the held LocalFile guard.
fn write_local(path: &Path, bytes: &[u8]) -> Result<()> {
    write_atomic(path, bytes, "sync")
}

#[cfg(test)]
mod revision_tests {
    use super::*;
    use tempfile::TempDir;

    struct Fixture {
        _dir: TempDir,
        syncer: Syncer,
        item: Item,
        target: Target,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let data = dir.path().join("data");
            let root = dir.path().join("cloud");
            fs::create_dir_all(&data).unwrap();
            fs::create_dir_all(&root).unwrap();
            let item = Item {
                name: "finicky.ts".into(),
                local: data.join("finicky.ts"),
                check: Arc::new(|_| Content::Valid),
                runs_after: None,
            };
            fs::write(&item.local, b"base").unwrap();
            let target = Target {
                kind: Kind::Folder,
                label: "Cloud".into(),
                root,
            };
            let mut syncer = Syncer::new(&data, "Here", "test");
            syncer.set_items(vec![item.clone()]);
            syncer.set_targets(vec![target.clone()]);
            assert_eq!(syncer.sync_one(&item, &target, 1), Health::Pushed);
            Self {
                _dir: dir,
                syncer,
                item,
                target,
            }
        }

        fn remote(&self, bytes: &[u8]) {
            self.target
                .folder()
                .write(&self.item.name, bytes, &Machine::default(), 2, "test", None)
                .unwrap();
        }
    }

    #[test]
    fn an_edit_during_remote_io_is_not_overwritten() {
        let mut f = Fixture::new();
        f.remote(b"folder revision");
        let local = f.item.local.clone();
        f.syncer.io_hook = Some(Arc::new(move |point, _| {
            if point == IoPoint::RemoteRead {
                fs::write(&local, b"intervening local edit").unwrap();
            }
        }));
        let health = f.syncer.sync_one(&f.item, &f.target, 3);
        assert_eq!(fs::read(&f.item.local).unwrap(), b"intervening local edit");
        assert!(matches!(health, Health::Conflict(_)), "{health:?}");
        assert_eq!(
            f.syncer.state.base(&f.target.id(), &f.item.name),
            Some(hash(b"base").as_str())
        );
    }

    #[test]
    fn an_edit_after_backup_is_not_overwritten() {
        let mut f = Fixture::new();
        f.remote(b"folder revision");
        f.syncer.io_hook = Some(Arc::new(|point, path| {
            if point == IoPoint::LocalBackup {
                fs::write(path, b"late external edit").unwrap();
            }
        }));
        let health = f.syncer.sync_one(&f.item, &f.target, 3);
        assert_eq!(fs::read(&f.item.local).unwrap(), b"late external edit");
        assert_eq!(health, Health::Waiting(Wait::Settling));
        assert_eq!(
            f.syncer.state.base(&f.target.id(), &f.item.name),
            Some(hash(b"base").as_str())
        );
    }

    #[test]
    fn changed_remote_metadata_cannot_settle_old_bytes() {
        let mut f = Fixture::new();
        f.remote(b"old bytes");
        // No manifest can endorse the read; it has to settle across passes.
        fs::remove_file(f.target.folder().file(crate::MANIFEST)).unwrap();
        f.syncer.io_hook = Some(Arc::new(|point, path| {
            if point == IoPoint::RemoteRead {
                fs::write(path, b"a different remote revision").unwrap();
            }
        }));
        assert_eq!(
            f.syncer.sync_one(&f.item, &f.target, 3),
            Health::Waiting(Wait::Settling)
        );
        f.syncer.io_hook = None;
        assert_eq!(
            f.syncer.sync_one(&f.item, &f.target, 4),
            Health::Waiting(Wait::Settling)
        );
        assert_eq!(fs::read(&f.item.local).unwrap(), b"base");
        assert_eq!(f.syncer.sync_one(&f.item, &f.target, 5), Health::Pulled);
        assert_eq!(
            fs::read(&f.item.local).unwrap(),
            b"a different remote revision"
        );
    }

    #[test]
    fn a_disappearing_remote_cannot_settle_cached_bytes() {
        let mut f = Fixture::new();
        f.remote(b"remote bytes");
        let folder = f.target.folder();
        let Probe::Present { stamp, .. } = folder.probe(&f.item.name) else {
            panic!("present")
        };
        f.syncer.io_hook = Some(Arc::new(|point, path| {
            if point == IoPoint::RemoteRead {
                fs::remove_file(path).unwrap();
            }
        }));
        assert!(
            f.syncer
                .read_remote(&folder, &f.item, &f.target.id(), stamp)
                .is_err()
        );
        assert!(
            !f.syncer
                .seen
                .contains_key(&(f.target.id(), f.item.name.clone()))
        );
    }

    #[test]
    fn a_manifest_cannot_endorse_an_inconsistent_observation() {
        let mut f = Fixture::new();
        f.remote(b"listed revision");
        f.syncer.io_hook = Some(Arc::new(|point, path| {
            if point == IoPoint::RemoteRead {
                fs::write(path, b"different revision after read").unwrap();
            }
        }));
        assert_eq!(
            f.syncer.sync_one(&f.item, &f.target, 3),
            Health::Waiting(Wait::Settling)
        );
        assert_eq!(fs::read(&f.item.local).unwrap(), b"base");
        assert!(f.syncer.seen.is_empty());
    }

    #[test]
    fn a_missing_local_file_created_during_remote_io_is_a_conflict() {
        let mut f = Fixture::new();
        fs::remove_file(&f.item.local).unwrap();
        f.remote(b"folder revision");
        let path = f.item.local.clone();
        f.syncer.io_hook = Some(Arc::new(move |point, _| {
            if point == IoPoint::RemoteRead {
                let held = LocalFile::lock(&path).unwrap();
                fs::write(held.path(), b"new local file").unwrap();
            }
        }));
        assert!(matches!(
            f.syncer.sync_one(&f.item, &f.target, 3),
            Health::Conflict(_)
        ));
        assert_eq!(fs::read(&f.item.local).unwrap(), b"new local file");
    }

    #[test]
    fn pull_and_resolution_hold_the_guard_through_backup_and_apply() {
        for resolve in [false, true] {
            let mut f = Fixture::new();
            f.remote(b"folder revision");
            let path = f.item.local.clone();
            f.syncer.io_hook = Some(Arc::new(move |point, _| {
                if point == IoPoint::LocalBackup {
                    let other = fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(crate::local::lock_path(&path))
                        .unwrap();
                    assert!(matches!(
                        other.try_lock(),
                        Err(fs::TryLockError::WouldBlock)
                    ));
                }
            }));
            if resolve {
                f.syncer
                    .resolve(
                        &f.target.root,
                        &f.item.name,
                        &hash(b"folder revision"),
                        Choice::UseFolder,
                        3,
                    )
                    .unwrap();
            } else {
                assert_eq!(f.syncer.sync_one(&f.item, &f.target, 3), Health::Pulled);
            }
            assert_eq!(fs::read(&f.item.local).unwrap(), b"folder revision");
            assert_eq!(f.syncer.state.backups.len(), 1);
            assert_eq!(fs::read(&f.syncer.state.backups[0].path).unwrap(), b"base");
            assert_eq!(
                f.syncer.state.base(&f.target.id(), &f.item.name),
                Some(hash(b"folder revision").as_str())
            );
        }
    }

    #[test]
    fn resolution_refuses_an_external_edit_after_backup() {
        let mut f = Fixture::new();
        f.remote(b"folder revision");
        f.syncer.io_hook = Some(Arc::new(|point, path| {
            if point == IoPoint::LocalBackup {
                fs::write(path, b"late external edit").unwrap();
            }
        }));
        let error = f
            .syncer
            .resolve(
                &f.target.root,
                &f.item.name,
                &hash(b"folder revision"),
                Choice::UseFolder,
                3,
            )
            .unwrap_err();
        assert!(error.to_string().contains("changed again"));
        assert_eq!(fs::read(&f.item.local).unwrap(), b"late external edit");
        assert_eq!(
            f.syncer.state.base(&f.target.id(), &f.item.name),
            Some(hash(b"base").as_str())
        );
    }

    #[test]
    fn blocked_provider_observations_leave_the_local_writer_guard_available() {
        use std::sync::{Mutex, mpsc};
        for blocked_at in [IoPoint::RemoteRead, IoPoint::RemoteManifest] {
            let mut f = Fixture::new();
            f.remote(b"folder revision");
            let (entered, waiting) = mpsc::channel();
            let (resume, resumed) = mpsc::channel();
            let resumed = Mutex::new(resumed);
            f.syncer.io_hook = Some(Arc::new(move |point, _| {
                if point == blocked_at {
                    entered.send(()).unwrap();
                    resumed.lock().unwrap().recv().unwrap();
                }
            }));
            let path = f.item.local.clone();
            std::thread::scope(|scope| {
                let worker = scope.spawn(|| f.syncer.sync_one(&f.item, &f.target, 3));
                waiting.recv().unwrap();
                // No timeout or sleep: the provider stays blocked until the
                // UI's nonblocking lock attempt and save have completed.
                let saved = LocalFile::try_lock(&path).map(|held| {
                    fs::write(held.path(), b"UI save during provider read").unwrap();
                });
                resume.send(()).unwrap();
                let health = worker.join().unwrap();
                saved.unwrap();
                assert!(matches!(health, Health::Conflict(_)), "{health:?}");
            });
            assert_eq!(fs::read(path).unwrap(), b"UI save during provider read");
        }
    }

    #[test]
    fn pushes_and_keep_local_resolution_release_the_guard_before_remote_writes() {
        for resolve in [false, true] {
            let mut f = Fixture::new();
            fs::write(&f.item.local, b"local edit").unwrap();
            let path = f.item.local.clone();
            f.syncer.io_hook = Some(Arc::new(move |point, _| {
                if point == IoPoint::RemoteWrite {
                    let _held = LocalFile::try_lock(&path).unwrap();
                }
            }));
            if resolve {
                f.remote(b"folder revision");
                f.syncer
                    .resolve(
                        &f.target.root,
                        &f.item.name,
                        &hash(b"folder revision"),
                        Choice::KeepLocal,
                        3,
                    )
                    .unwrap();
            } else {
                assert_eq!(f.syncer.sync_one(&f.item, &f.target, 3), Health::Pushed);
            }
            assert_eq!(
                fs::read(f.target.folder().file(&f.item.name)).unwrap(),
                b"local edit"
            );
        }
    }

    #[test]
    fn a_lock_error_cannot_apply_or_advance_the_base() {
        let mut f = Fixture::new();
        f.remote(b"folder revision");
        // A directory where the lock file should be deterministically refuses
        // opening, without relying on root/user permission differences.
        let lock = crate::local::lock_path(&f.item.local);
        fs::remove_file(&lock).unwrap();
        fs::create_dir(&lock).unwrap();
        assert!(matches!(
            f.syncer.sync_one(&f.item, &f.target, 3),
            Health::Error(_)
        ));
        assert!(
            f.syncer
                .resolve(
                    &f.target.root,
                    &f.item.name,
                    &hash(b"folder revision"),
                    Choice::UseFolder,
                    4
                )
                .is_err()
        );
        assert_eq!(fs::read(&f.item.local).unwrap(), b"base");
        assert_eq!(
            f.syncer.state.base(&f.target.id(), &f.item.name),
            Some(hash(b"base").as_str())
        );
    }
}
