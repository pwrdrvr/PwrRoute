//! Last-successful profile metadata, shared by installed and development apps.
//! This contains only our parsed profile fields, never a browser's raw index.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::Profile;

#[derive(Deserialize, Serialize)]
struct Cache {
    version: u32,
    entries: Vec<Entry>,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            version: 1,
            entries: Vec::new(),
        }
    }
}

#[derive(Deserialize, Serialize)]
struct Entry {
    browser: String,
    index: PathBuf,
    profiles: Vec<Profile>,
}

fn load(path: &Path) -> Option<Cache> {
    let cache: Cache = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    (cache.version == 1).then_some(cache)
}

pub(crate) fn read(path: &Path, browser: &str, index: &Path) -> Vec<Profile> {
    load(path)
        .and_then(|cache| {
            cache
                .entries
                .into_iter()
                .find(|e| e.browser == browser && e.index == index)
        })
        .map(|entry| entry.profiles)
        .unwrap_or_default()
}

fn options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
}

pub(crate) fn remember(
    path: &Path,
    browser: &str,
    index: &Path,
    profiles: &[Profile],
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // Serialize read/merge/write across threads and processes sharing this file.
    let lock = options().open(path.with_extension("lock"))?;
    lock.lock()?;
    let mut cache = load(path).unwrap_or_default();
    match cache
        .entries
        .iter_mut()
        .find(|e| e.browser == browser && e.index == index)
    {
        Some(entry) if entry.profiles == profiles => return Ok(()),
        Some(entry) => entry.profiles = profiles.to_vec(),
        None => cache.entries.push(Entry {
            browser: browser.to_owned(),
            index: index.to_owned(),
            profiles: profiles.to_vec(),
        }),
    }
    // The lock makes a fixed sibling staging file safe; a crash leaves the
    // previous complete cache. Readers only ever open the published JSON.
    let staging = path.with_extension("json.tmp");
    let result = (|| {
        let mut file = options().truncate(true).open(&staging)?;
        serde_json::to_writer_pretty(&mut file, &cache)?;
        file.write_all(b"\n")?;
        drop(file);
        fs::rename(&staging, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(staging);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_refreshes_keep_each_browsers_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("profiles.json");
        std::thread::scope(|scope| {
            for i in 0..8 {
                let cache = &cache;
                scope.spawn(move || {
                    remember(
                        cache,
                        &format!("browser-{i}"),
                        Path::new("/profiles/Local State"),
                        &[Profile {
                            name: format!("Work {i}"),
                            directory: "Default".into(),
                            default: true,
                        }],
                    )
                    .unwrap()
                });
            }
        });
        for i in 0..8 {
            assert_eq!(
                read(
                    &cache,
                    &format!("browser-{i}"),
                    Path::new("/profiles/Local State")
                )[0]
                .name,
                format!("Work {i}")
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(cache).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
