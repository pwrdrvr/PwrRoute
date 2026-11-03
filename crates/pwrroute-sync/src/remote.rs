//! Cloud I/O boundary. Requests contain no local paths or mutable sync state.
use std::{
    fs,
    io::{self, Read},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::{
    Health, Target, Wait,
    folder::{self, FileEntry, Machine, Manifest, Probe, Stamp, hash},
    syncer::{MAX_BYTES, Seen},
};
use anyhow::{Context as _, Result, bail};

#[derive(Clone, Default)]
pub(crate) struct Cancel(Arc<AtomicBool>, Option<Arc<AtomicBool>>);
impl Cancel {
    pub fn child(&self) -> Self {
        Self(Arc::new(AtomicBool::new(false)), Some(self.0.clone()))
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn check(&self) -> Result<()> {
        if self.0.load(Ordering::Acquire)
            || self.1.as_ref().is_some_and(|s| s.load(Ordering::Acquire))
        {
            bail!("sync operation was retired");
        }
        Ok(())
    }
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

pub(crate) struct Observation {
    pub file: Result<Option<Seen>, Health>,
    pub manifest: Option<Manifest>,
}

#[derive(Clone)]
pub(crate) struct Write {
    pub name: String,
    pub bytes: Vec<u8>,
    pub expected: Option<String>,
    pub by: Machine,
    pub at: u64,
    pub app: String,
    pub runs_after: Option<String>,
}

pub(crate) trait Remote: Send + Sync + 'static {
    fn read(
        &self,
        target: &Target,
        name: &str,
        cached: Option<Seen>,
        cancel: &Cancel,
    ) -> Observation;
    fn write(&self, target: &Target, write: &Write, cancel: &Cancel) -> Result<Manifest> {
        Disk.write(target, write, cancel)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum IoPoint {
    RemoteRead,
    RemoteManifest,
    #[cfg(test)]
    RemoteWrite,
    #[cfg(test)]
    LocalBackup,
}
#[cfg(test)]
pub(crate) type IoHook = Arc<dyn Fn(IoPoint, &Path) + Send + Sync>;

pub(crate) struct Disk;

/// Check the actual byte count as well as metadata: a provider can grow a file
/// after the probe. This also bounds bytes retained by a stuck/retired worker.
fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        bail!("sync file is too large");
    }
    Ok(bytes)
}

fn manifest(target: &Target) -> Option<Manifest> {
    let bytes = read_limited(&target.folder().file(folder::MANIFEST), MAX_BYTES).ok()?;
    serde_json::from_slice::<Manifest>(&bytes)
        .ok()
        .filter(|m| m.format == folder::FORMAT)
}

impl Disk {
    pub(crate) fn read_with(
        &self,
        target: &Target,
        name: &str,
        cached: Option<Seen>,
        cancel: &Cancel,
        mut hook: impl FnMut(IoPoint, &Path),
    ) -> Observation {
        let read = || -> Result<Result<Option<Seen>, Health>> {
            cancel.check()?;
            let folder = target.folder();
            Ok(match folder.probe(name) {
                Probe::Unavailable(why) => Err(Health::Unavailable(why)),
                Probe::NotDownloaded => Err(Health::Waiting(Wait::NotDownloaded)),
                Probe::Missing => Ok(None),
                Probe::Present { stamp, .. } => {
                    cancel.check()?;
                    if let Some(mut seen) = cached.filter(|s| s.stamp == stamp) {
                        seen.stable = true;
                        Ok(Some(seen))
                    } else if stamp.len > MAX_BYTES {
                        Err(Health::Unavailable(format!(
                            "{name} is too large to be PwrRoute's"
                        )))
                    } else {
                        match read_limited(&folder.file(name), MAX_BYTES) {
                            Ok(bytes) => {
                                hook(IoPoint::RemoteRead, &folder.file(name));
                                cancel.check()?;
                                let after =
                                    fs::metadata(folder.file(name)).ok().map(|m| Stamp::of(&m));
                                if after != Some(stamp) || bytes.len() as u64 != stamp.len {
                                    return Ok(Err(Health::Waiting(Wait::Settling)));
                                }
                                Ok(Some(Seen {
                                    stamp,
                                    sha: hash(&bytes),
                                    bytes,
                                    stable: false,
                                }))
                            }
                            Err(e) => {
                                Err(Health::Unavailable(match e.downcast_ref::<io::Error>() {
                                    Some(e) => folder::describe(e),
                                    None => e.to_string(),
                                }))
                            }
                        }
                    }
                }
            })
        };
        let file = read().unwrap_or_else(|e| Err(Health::Unavailable(e.to_string())));
        let manifest = if cancel.check().is_ok() {
            hook(
                IoPoint::RemoteManifest,
                &target.folder().file(folder::MANIFEST),
            );
            manifest(target)
        } else {
            None
        };
        Observation { file, manifest }
    }
}

impl Remote for Disk {
    fn read(
        &self,
        target: &Target,
        name: &str,
        cached: Option<Seen>,
        cancel: &Cancel,
    ) -> Observation {
        self.read_with(target, name, cached, cancel, |_, _| {})
    }

    fn write(&self, target: &Target, write: &Write, cancel: &Cancel) -> Result<Manifest> {
        // Read every cloud precondition before the first live-file mutation.
        // In particular, do not block on a manifest read after replacing bytes.
        let observed = self.read(target, &write.name, None, cancel);
        let remote = observed
            .file
            .map_err(|_| anyhow::anyhow!("folder is unavailable"))?;
        if remote.as_ref().map(|s| &s.sha) != write.expected.as_ref() {
            bail!(
                "{} changed again; sync will look again before writing",
                write.name
            );
        }
        let mut manifest = observed.manifest.unwrap_or_default();
        manifest.format = folder::FORMAT;
        manifest.saved_by = write.by.clone();
        manifest.saved_at = write.at;
        manifest.app.clone_from(&write.app);
        manifest.files.insert(
            write.name.clone(),
            FileEntry {
                sha256: hash(&write.bytes),
                bytes: write.bytes.len() as u64,
                saved_by: write.by.clone(),
                saved_at: write.at,
                runs_after: write.runs_after.clone(),
            },
        );
        let mut text = serde_json::to_vec_pretty(&manifest)?;
        text.push(b'\n');
        commit(target, write, &text, cancel, |_| {})?;
        Ok(manifest)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Directory,
    WriteContent,
    RenameContent,
    WriteManifest,
    RenameManifest,
}

fn commit(
    target: &Target,
    write: &Write,
    manifest: &[u8],
    cancel: &Cancel,
    mut before: impl FnMut(Stage),
) -> Result<()> {
    // The callback is a deterministic syscall-boundary seam for tests.
    before(Stage::Directory);
    cancel.check()?;
    if !target.root.is_dir() {
        bail!("sync root is no longer there");
    }
    cancel.check()?;
    match fs::create_dir(target.folder().dir()) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    guarded_atomic(
        &target.folder().file(&write.name),
        &write.bytes,
        &write.by.id,
        cancel,
        &mut before,
        [Stage::WriteContent, Stage::RenameContent],
    )?;
    guarded_atomic(
        &target.folder().file(folder::MANIFEST),
        manifest,
        &write.by.id,
        cancel,
        &mut before,
        [Stage::WriteManifest, Stage::RenameManifest],
    )
}

fn guarded_atomic(
    path: &Path,
    bytes: &[u8],
    tag: &str,
    cancel: &Cancel,
    before: &mut impl FnMut(Stage),
    stages: [Stage; 2],
) -> Result<()> {
    let name = path.file_name().context("a file name")?.to_string_lossy();
    let short: String = tag
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(8)
        .collect();
    let staging = path.with_file_name(format!(".{name}.{short}.tmp"));
    before(stages[0]);
    cancel.check()?;
    fs::write(&staging, bytes)?;
    before(stages[1]);
    cancel.check()?;
    fs::rename(&staging, path)?;
    // A syscall already entered cannot be cancelled. Cancellation here prevents
    // the *next* stage, including the manifest, and leaves settling to readers.
    cancel.check()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Kind;

    fn target(root: &Path) -> Target {
        Target {
            kind: Kind::Folder,
            label: "test".into(),
            root: root.into(),
        }
    }
    fn write(expected: Option<String>) -> Write {
        Write {
            name: "rules.json".into(),
            bytes: b"new".to_vec(),
            expected,
            by: Machine {
                id: "test".into(),
                name: "test".into(),
            },
            at: 1,
            app: "test".into(),
            runs_after: Some("~/.finicky.ts".into()),
        }
    }

    #[test]
    fn cancellation_at_each_controllable_stage_prevents_later_file_mutations() {
        let stages = [
            Stage::Directory,
            Stage::WriteContent,
            Stage::RenameContent,
            Stage::WriteManifest,
            Stage::RenameManifest,
        ];
        for (index, stop_at) in stages.into_iter().enumerate() {
            let dir = tempfile::tempdir().unwrap();
            let target = target(dir.path());
            let folder = target.folder();
            fs::create_dir(folder.dir()).unwrap();
            fs::write(folder.file("rules.json"), b"old").unwrap();
            fs::write(folder.file(folder::MANIFEST), b"old manifest").unwrap();
            let cancel = Cancel::default();
            let mut entered = Vec::new();
            assert!(
                commit(&target, &write(None), b"new manifest", &cancel, |stage| {
                    entered.push(stage);
                    if stage == stop_at {
                        cancel.cancel();
                    }
                })
                .is_err()
            );
            assert_eq!(entered, stages[..=index]);
            assert_eq!(
                fs::read(folder.file("rules.json")).unwrap(),
                if index > 2 { b"new" } else { b"old" }
            );
            assert_eq!(
                fs::read(folder.file(folder::MANIFEST)).unwrap(),
                b"old manifest"
            );
        }
    }

    #[test]
    fn a_stale_observation_cannot_overwrite_a_new_remote_edit() {
        let dir = tempfile::tempdir().unwrap();
        let target = target(dir.path());
        let folder = target.folder();
        folder
            .write(
                "rules.json",
                b"latest",
                &Machine::default(),
                1,
                "test",
                None,
            )
            .unwrap();
        assert!(
            Disk.write(&target, &write(Some(hash(b"old"))), &Cancel::default())
                .is_err()
        );
        assert_eq!(fs::read(folder.file("rules.json")).unwrap(), b"latest");
        assert_eq!(
            folder.manifest().unwrap().files["rules.json"].sha256,
            hash(b"latest")
        );
    }
}
