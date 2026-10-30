//! One sync folder on disk: `<root>/PwrRoute/`.
//!
//! ```text
//! <root>/PwrRoute/
//!   rules.json   the synced rules, byte for byte
//!   sync.json    the manifest, written after the files it describes
//!   finicky.ts   the Finicky config, when this is synced too
//! ```
//!
//! Every write goes to `.<name>.<machine>.tmp` beside its file and is
//! renamed into place, so a cloud client never uploads half a file. The
//! manifest is written last: a reader that sees a file whose hash the
//! manifest does not list knows another machine's write is still arriving
//! (see [`crate::pass`]'s settling).
//!
//! Only exact paths are read: nothing here lists the folder.

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The folder PwrRoute owns inside a cloud root.
pub const FOLDER: &str = "PwrRoute";
pub const MANIFEST: &str = "sync.json";
/// The manifest format this build writes and reads.
pub const FORMAT: u32 = 1;

/// The hex SHA-256 of some bytes: the one hash every machine agrees on.
pub fn hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// `sync.json`: who saved the folder last, and what each file should be.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: u32,
    /// By file name in the folder.
    #[serde(default)]
    pub files: BTreeMap<String, FileEntry>,
    #[serde(default)]
    pub saved_by: Machine,
    /// Milliseconds since the epoch.
    #[serde(default)]
    pub saved_at: u64,
    /// The PwrRoute version that wrote it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub app: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub sha256: String,
    pub bytes: u64,
    /// Who wrote this file and when, which can differ from the manifest's
    /// own when two files sync.
    #[serde(default)]
    pub saved_by: Machine,
    #[serde(default)]
    pub saved_at: u64,
    /// The config this version ran after on the machine that wrote it (its
    /// shown path, `~/.finicky.js`): a Finicky config's handlers run before
    /// `rules.json`'s rules, so there `rules.json` is only half the routing.
    /// Recorded only when the file's content is written, never on its own,
    /// so two machines with different configs do not rewrite the manifest
    /// back and forth; a pull leaves it as the writer left it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs_after: Option<String>,
}

/// A machine as sync names it: a random id that never changes and the
/// name the user gave it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Machine {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
}

/// What a file's metadata says, to tell whether it changed without reading
/// it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stamp {
    pub len: u64,
    /// Modification time, nanoseconds since the epoch.
    pub modified: u128,
}

impl Stamp {
    pub fn of(meta: &fs::Metadata) -> Self {
        Self {
            len: meta.len(),
            modified: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos()),
        }
    }
}

/// A file in a sync folder (or the local one) as one look found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// The folder is there and the file is not.
    Missing,
    /// The cloud client knows the file but has not downloaded it: iCloud's
    /// `.<name>.icloud` stand-in is there instead.
    NotDownloaded,
    /// The file is there. `dataless` says macOS has evicted its contents
    /// (a File Provider file: iCloud Drive, Google Drive, Dropbox), so
    /// reading it will download it first.
    Present { stamp: Stamp, dataless: bool },
    /// The folder's root is gone (the client is not running, a drive is
    /// not mounted), or the file could not be looked at; the reason in
    /// words for the user.
    Unavailable(String),
}

/// macOS's `SF_DATALESS`: the file's contents live in the cloud only.
#[cfg(target_os = "macos")]
const SF_DATALESS: u32 = 0x4000_0000;

fn dataless(meta: &fs::Metadata) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt as _;
        meta.st_flags() & SF_DATALESS != 0
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = meta;
        false
    }
}

/// Looks at one exact path. `root` is the cloud folder the file belongs
/// under; when it is gone the file is [`Probe::Unavailable`], not missing,
/// so a client that is not running never looks like a deleted file.
pub fn probe(root: &Path, path: &Path) -> Probe {
    if !root.is_dir() {
        return Probe::Unavailable(format!("{} is not there", root.display()));
    }
    match fs::metadata(path) {
        Ok(meta) if meta.is_file() => Probe::Present {
            stamp: Stamp::of(&meta),
            dataless: dataless(&meta),
        },
        Ok(_) => Probe::Unavailable(format!("{} is not a file", path.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if icloud_stand_in(path).is_some_and(|p| p.is_file()) {
                Probe::NotDownloaded
            } else {
                Probe::Missing
            }
        }
        Err(e) => Probe::Unavailable(describe(&e)),
    }
}

/// iCloud's stand-in for a file it has not downloaded: `.<name>.icloud`.
fn icloud_stand_in(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    Some(path.with_file_name(format!(".{name}.icloud")))
}

/// How [`describe`] words a refused read, so the UI can offer the way out.
pub const NO_ACCESS: &str = "PwrRoute may not use this folder";

/// An I/O error in words, without the path (the caller names the folder).
pub fn describe(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::PermissionDenied => NO_ACCESS.into(),
        io::ErrorKind::NotFound => "not there".into(),
        io::ErrorKind::TimedOut => "the cloud client did not answer in time".into(),
        _ => e.to_string(),
    }
}

/// One sync folder: `<root>/PwrRoute`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub root: PathBuf,
}

impl Folder {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn dir(&self) -> PathBuf {
        self.root.join(FOLDER)
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.dir().join(name)
    }

    pub fn probe(&self, name: &str) -> Probe {
        probe(&self.root, &self.file(name))
    }

    /// The manifest, when there is one that parses and a format this build
    /// knows. A newer format is read as no manifest, so its files only
    /// settle by staying put.
    pub fn manifest(&self) -> Option<Manifest> {
        let text = fs::read_to_string(self.file(MANIFEST)).ok()?;
        serde_json::from_str::<Manifest>(&text)
            .ok()
            .filter(|m| m.format == FORMAT)
    }

    /// The bytes of `name` once they have settled: the manifest lists the
    /// name and the file's hash is the one it lists. Otherwise an error in
    /// words for the user: the file is still arriving, or is not in this
    /// folder. Reads the exact path only.
    pub fn settled(&self, name: &str) -> Result<Vec<u8>> {
        let arriving = || anyhow::anyhow!("{name} is still arriving in this folder");
        let absent = || anyhow::anyhow!("{name} is not in this folder");
        if !self.root.is_dir() {
            anyhow::bail!("{} is not there", self.root.display());
        }
        let Some(entry) = self.manifest().and_then(|mut m| m.files.remove(name)) else {
            return Err(absent());
        };
        match self.probe(name) {
            Probe::Present { .. } => {}
            // Listed but not here yet: the client has not brought it down.
            Probe::Missing | Probe::NotDownloaded => return Err(arriving()),
            Probe::Unavailable(why) => anyhow::bail!("{name}: {why}"),
        }
        let bytes = match fs::read(self.file(name)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(arriving()),
            Err(e) => anyhow::bail!("{name}: {}", describe(&e)),
        };
        if hash(&bytes) != entry.sha256 {
            return Err(arriving());
        }
        Ok(bytes)
    }

    /// Writes `bytes` as `name`, then records it in the manifest with
    /// `runs_after` ([`FileEntry::runs_after`]). Creates `PwrRoute/` inside
    /// the root, never the root itself.
    pub fn write(
        &self,
        name: &str,
        bytes: &[u8],
        by: &Machine,
        at: u64,
        app: &str,
        runs_after: Option<&str>,
    ) -> Result<()> {
        if !self.root.is_dir() {
            anyhow::bail!("{} is not there", self.root.display());
        }
        let dir = self.dir();
        fs::create_dir_all(&dir).with_context(|| dir.display().to_string())?;
        write_atomic(&self.file(name), bytes, &by.id)?;
        let mut manifest = self.manifest().unwrap_or_default();
        manifest.format = FORMAT;
        manifest.files.insert(
            name.to_owned(),
            FileEntry {
                sha256: hash(bytes),
                bytes: bytes.len() as u64,
                saved_by: by.clone(),
                saved_at: at,
                runs_after: runs_after.map(str::to_owned),
            },
        );
        manifest.saved_by = by.clone();
        manifest.saved_at = at;
        app.clone_into(&mut manifest.app);
        let mut text = serde_json::to_string_pretty(&manifest)?;
        text.push('\n');
        write_atomic(&self.file(MANIFEST), text.as_bytes(), &by.id)
    }
}

/// Writes through `.<name>.<tag>.tmp` beside the file, then renames it into
/// place.
pub fn write_atomic(path: &Path, bytes: &[u8], tag: &str) -> Result<()> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .context("a file name")?;
    let short: String = tag
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(8)
        .collect();
    let staging = path.with_file_name(format!(".{name}.{short}.tmp"));
    fs::write(&staging, bytes).with_context(|| path.display().to_string())?;
    fs::rename(&staging, path).with_context(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> Machine {
        Machine {
            id: "0123456789abcdef".into(),
            name: "Studio Mac".into(),
        }
    }

    #[test]
    fn hash_is_sha256_hex() {
        assert_eq!(
            hash(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn a_write_lands_whole_and_is_recorded_in_the_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let folder = Folder::new(dir.path());
        assert_eq!(folder.probe("rules.json"), Probe::Missing);
        assert!(folder.manifest().is_none());
        folder
            .write("rules.json", b"{}\n", &me(), 42, "0.8.0", None)
            .unwrap();
        assert_eq!(fs::read(folder.file("rules.json")).unwrap(), b"{}\n");
        let manifest = folder.manifest().unwrap();
        assert_eq!(manifest.files["rules.json"].sha256, hash(b"{}\n"));
        assert_eq!(manifest.files["rules.json"].bytes, 3);
        assert_eq!(manifest.saved_by.name, "Studio Mac");
        assert_eq!(manifest.saved_at, 42);
        assert!(
            matches!(folder.probe("rules.json"), Probe::Present { stamp, .. } if stamp.len == 3)
        );
        let leftovers: Vec<_> = fs::read_dir(folder.dir())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "no staging file left behind");
        // A second file keeps the first's entry.
        folder
            .write("finicky.ts", b"x", &me(), 43, "0.8.0", None)
            .unwrap();
        let manifest = folder.manifest().unwrap();
        assert_eq!(manifest.files.len(), 2);
        assert_eq!(manifest.files["rules.json"].saved_at, 42);
    }

    #[test]
    fn a_missing_root_is_unavailable_and_never_created() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("CloudStorage/Dropbox");
        let folder = Folder::new(&gone);
        assert!(matches!(folder.probe("rules.json"), Probe::Unavailable(_)));
        assert!(
            folder
                .write("rules.json", b"{}", &me(), 1, "", None)
                .is_err()
        );
        assert!(!gone.exists());
    }

    #[test]
    fn an_icloud_stand_in_is_not_downloaded() {
        let dir = tempfile::tempdir().unwrap();
        let folder = Folder::new(dir.path());
        fs::create_dir_all(folder.dir()).unwrap();
        fs::write(folder.file(".rules.json.icloud"), b"plist").unwrap();
        assert_eq!(folder.probe("rules.json"), Probe::NotDownloaded);
    }

    #[test]
    fn a_manifest_from_a_newer_format_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let folder = Folder::new(dir.path());
        fs::create_dir_all(folder.dir()).unwrap();
        fs::write(folder.file(MANIFEST), br#"{"format": 2, "files": {}}"#).unwrap();
        assert!(folder.manifest().is_none());
        fs::write(folder.file(MANIFEST), b"{ half").unwrap();
        assert!(folder.manifest().is_none());
    }
}
