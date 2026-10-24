//! Whether the helper's own executable is still the file it started from.
//!
//! An update swaps the whole `PwrRoute.app` for a new one (Velopack renames
//! the new bundle over the old and deletes the old), and dragging the app to
//! the Trash moves it. Neither stops a running helper: it would keep
//! forwarding links from a binary that is gone, until logout. So the helper
//! notes its executable's identity when it starts and looks again after each
//! request; once the path names a different file, or nothing, it finishes
//! what is queued and exits, and the next link makes LaunchServices start
//! the helper that is there now.

use std::path::{Path, PathBuf};

/// A file's identity: its device and inode. The same path with a new
/// identity is a new file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileId {
    dev: u64,
    ino: u64,
}

impl FileId {
    #[cfg(test)]
    pub fn new(dev: u64, ino: u64) -> Self {
        Self { dev, ino }
    }
}

/// The identity of the file at `path`, following symlinks. `None` when
/// there is no file there, or off Unix, where the helper does not run.
pub fn file_id(path: &Path) -> Option<FileId> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(path).ok()?;
        Some(FileId {
            dev: meta.dev(),
            ino: meta.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// Replaced once a file was seen at start and the path now names another
/// file or none. Never when the start was not seen: a helper that cannot
/// tell keeps running, as before.
pub fn replaced(at_start: Option<FileId>, now: Option<FileId>) -> bool {
    at_start.is_some_and(|start| now != Some(start))
}

/// The helper's executable, as it was when the helper started.
pub struct SelfWatch {
    path: Option<PathBuf>,
    at_start: Option<FileId>,
}

impl SelfWatch {
    /// Notes the identity of the executable at `path` now.
    pub fn new(path: Option<PathBuf>) -> Self {
        let at_start = path.as_deref().and_then(file_id);
        Self { path, at_start }
    }

    /// Whether the executable at the path is no longer the one noted.
    pub fn replaced(&self) -> bool {
        replaced(self.at_start, self.path.as_deref().and_then(file_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_file_seen_at_start_can_be_replaced() {
        let start = FileId::new(1, 10);
        assert!(!replaced(Some(start), Some(start)));
        assert!(replaced(Some(start), Some(FileId::new(1, 11))));
        assert!(replaced(Some(start), Some(FileId::new(2, 10))));
        assert!(replaced(Some(start), None));
        assert!(!replaced(None, None));
        assert!(!replaced(None, Some(start)));
    }

    #[cfg(unix)]
    #[test]
    fn a_swapped_or_removed_executable_reads_as_replaced() {
        let dir = std::env::temp_dir().join(format!("plh-replaced-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("pwrroute-links");
        std::fs::write(&exe, "old").unwrap();

        let watch = SelfWatch::new(Some(exe.clone()));
        assert!(!watch.replaced());
        // Rewriting in place keeps the file: not an update.
        std::fs::write(&exe, "old, rewritten").unwrap();
        assert!(!watch.replaced());

        // An update renames a new file over the path.
        let new = dir.join("new");
        std::fs::write(&new, "new").unwrap();
        std::fs::rename(&new, &exe).unwrap();
        assert!(watch.replaced());

        // Moved to the Trash, or deleted.
        let watch = SelfWatch::new(Some(exe.clone()));
        std::fs::remove_file(&exe).unwrap();
        assert!(watch.replaced());

        // Nothing there at start: cannot tell, so never replaced.
        assert!(!SelfWatch::new(Some(exe)).replaced());
        assert!(!SelfWatch::new(None).replaced());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
