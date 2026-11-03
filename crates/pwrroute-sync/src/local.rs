//! The local revision/write boundary shared by sync and PwrRoute's writers.
//!
//! Lock a persistent sibling file, not the data file (which is replaced by
//! rename). Hold the guard across the revision check, backup and write. The
//! lock is advisory: hand editors and cloud clients need not honor it. Reads
//! after slow provider I/O and just before applying also catch their observable
//! edits, but cannot provide a filesystem compare-and-swap against them.

use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

/// Exclusive access among cooperating PwrRoute writers, including separate
/// processes. Dropping the guard releases the lock. Never unlink its sidecar:
/// waiters must continue to lock the same file after a data-file rename.
#[derive(Debug)]
pub struct LocalFile {
    path: PathBuf,
    _lock: File,
}

impl LocalFile {
    pub fn lock(path: &Path) -> io::Result<Self> {
        let file = Self::open(path)?;
        file._lock.lock()?;
        Ok(file)
    }

    /// UI writers must not wait on the sync worker. On contention the edit
    /// stays unsaved and the caller reports an error; the next save can retry.
    pub fn try_lock(path: &Path) -> io::Result<Self> {
        let file = Self::open(path)?;
        match file._lock.try_lock() {
            Ok(()) => Ok(file),
            Err(fs::TryLockError::WouldBlock) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Another PwrRoute write is in progress; try saving again",
            )),
            Err(fs::TryLockError::Error(error)) => Err(error),
        }
    }

    fn open(path: &Path) -> io::Result<Self> {
        // Follow symlinks so aliases use the target's lock, and applying a
        // synced Finicky config preserves the symlink. A dangling link is an
        // error, as it was for sync's write_local.
        let path = match fs::symlink_metadata(path) {
            Ok(m) if m.file_type().is_symlink() => fs::canonicalize(path)?,
            Ok(_) => path.to_path_buf(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => path.to_path_buf(),
            Err(e) => return Err(e),
        };
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        // Canonicalize the parent too: symlinked data directories and relative
        // paths must share a lock. The file itself may not exist yet.
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "a file name"))?;
        let path = fs::canonicalize(parent)?.join(name);
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path(&path))?;
        Ok(Self { path, _lock: lock })
    }

    /// Resolved write target. Writers use this path so replacing a symlink
    /// cannot split the lock identity while another writer waits.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub(crate) fn lock_path(path: &Path) -> PathBuf {
    let mut name = std::ffi::OsString::from(".");
    name.push(path.file_name().expect("a local file name"));
    name.push(".pwrroute.lock");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guard_survives_replacing_the_data_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rules.json");
        let held = LocalFile::lock(&path).unwrap();
        crate::folder::write_atomic(held.path(), b"new", "test").unwrap();
        let other = OpenOptions::new()
            .read(true)
            .write(true)
            .open(lock_path(held.path()))
            .unwrap();
        assert!(matches!(
            other.try_lock(),
            Err(fs::TryLockError::WouldBlock)
        ));
        drop(held);
        other.try_lock().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_uses_its_targets_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("finicky.ts");
        fs::write(&path, b"base").unwrap();
        let link = dir.path().join("linked.ts");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        let held = LocalFile::lock(&link).unwrap();
        let other = OpenOptions::new()
            .read(true)
            .write(true)
            .open(lock_path(&path))
            .unwrap();
        assert!(matches!(
            other.try_lock(),
            Err(fs::TryLockError::WouldBlock)
        ));
        assert_eq!(held.path(), fs::canonicalize(path).unwrap());
    }
}
