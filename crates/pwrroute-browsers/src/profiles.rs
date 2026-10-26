//! Browser profiles: Chromium's `Local State` (`profile.info_cache`) and
//! Firefox's `profiles.ini`, and mapping a config's profile name to the
//! argument the browser needs. Ported from Finicky's launcher
//! (MIT, Copyright (c) 2015-2025 John Sterling).

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    catalog::{BrowserInfo, ProfileFlag},
    env::Env,
    profile_cache,
};

/// One profile as the browser lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// The name the user sees and a config names (`profile: "Work"`).
    pub name: String,
    /// What identifies it on disk: the Chromium profile directory
    /// (`Default`, `Profile 1`) or the Firefox profile `Path`.
    pub directory: String,
    /// Chromium's last-used profile, or Firefox's default for an install.
    pub default: bool,
}

/// The profiles of `browser` on this machine, in the browser's own order.
/// Empty when the browser has no profile flag, nothing is installed, or the
/// profile index cannot be read and no saved metadata is available.
pub fn list_profiles(browser: &BrowserInfo, env: &Env) -> Vec<Profile> {
    read_profiles(browser, env).unwrap_or_else(|blocked| blocked.cached)
}

/// The profile index is there but this process may not read it: macOS 27
/// keeps another app's data from an app until the user turns it on in
/// Privacy & Security › Files & Folders, and the file's `stat` still works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfilesBlocked {
    /// The file that could not be read (`Local State`, `profiles.ini`).
    pub path: PathBuf,
    /// Last-successful metadata, if caching was enabled for this browser
    /// and index path. The live read remains blocked even when this is usable.
    pub cached: Vec<Profile>,
}

/// [`list_profiles`], telling a profile index that is there but cannot be
/// read (permission denied) from one that is missing or malformed, which
/// are `Ok` and empty.
pub fn read_profiles(browser: &BrowserInfo, env: &Env) -> Result<Vec<Profile>, ProfilesBlocked> {
    read_with(browser, env, |path| fs::read_to_string(path))
}

type ProfileParser = fn(&str) -> Option<Vec<Profile>>;

fn read_with(
    browser: &BrowserInfo,
    env: &Env,
    read: impl FnOnce(&Path) -> io::Result<String>,
) -> Result<Vec<Profile>, ProfilesBlocked> {
    let Some(dir) = browser.profile_dir(env) else {
        return Ok(Vec::new());
    };
    let (path, parse): (_, ProfileParser) = match browser.profile_flag() {
        ProfileFlag::Chromium => (dir.join("Local State"), parse_chromium),
        ProfileFlag::Firefox => (dir.join("profiles.ini"), |text| {
            (!ini_sections(text).is_empty()).then(|| firefox_profiles(text))
        }),
        ProfileFlag::None => return Ok(Vec::new()),
    };
    match read(&path) {
        Ok(text) => {
            let profiles = parse(&text);
            if let (Some(cache), Some(profiles)) = (env.profile_cache(), &profiles) {
                // An unwritable cache must never prevent a live profile launch.
                let _ = profile_cache::remember(cache, &browser.id, &path, profiles);
            }
            Ok(profiles.unwrap_or_default())
        }
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            let cached = env
                .profile_cache()
                .map(|cache| profile_cache::read(cache, &browser.id, &path))
                .unwrap_or_default();
            Err(ProfilesBlocked { path, cached })
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if let Some(cache) = env.profile_cache() {
                let _ = profile_cache::remember(cache, &browser.id, &path, &[]);
            }
            Ok(Vec::new())
        }
        Err(_) => Ok(Vec::new()),
    }
}

/// How a profile request turned out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProfileMatch {
    /// The arguments that select it.
    Found { args: Vec<String>, cached: bool },
    /// The browser has no such profile; these are the names it does have.
    NotFound(Vec<String>),
    /// The browser cannot select a profile from the command line.
    Unsupported,
}

/// Finicky's lookup: the profile whose name is `wanted`, else the profile
/// whose directory is `wanted` (Chromium `Profile 1`).
pub(crate) fn resolve(browser: &BrowserInfo, env: &Env, wanted: &str) -> ProfileMatch {
    let flag = browser.profile_flag();
    if flag == ProfileFlag::None {
        return ProfileMatch::Unsupported;
    }
    let (profiles, cached) = match read_profiles(browser, env) {
        Ok(profiles) => (profiles, false),
        Err(blocked) => (blocked.cached, true),
    };
    let found = profiles
        .iter()
        .find(|p| p.name == wanted)
        .or_else(|| profiles.iter().find(|p| p.directory == wanted));
    match (found, flag) {
        (Some(p), ProfileFlag::Chromium) => ProfileMatch::Found {
            args: vec![format!("--profile-directory={}", p.directory)],
            cached,
        },
        // `-P` takes the profile's name, not its path.
        (Some(p), _) => ProfileMatch::Found {
            args: vec!["-P".into(), p.name.clone()],
            cached,
        },
        (None, _) => ProfileMatch::NotFound(profiles.into_iter().map(|p| p.name).collect()),
    }
}

/// `profile.info_cache` from a Chromium `Local State`, ordered by
/// `profile.profiles_order` when present, then by directory.
#[cfg(test)]
fn chromium_profiles(local_state: &str) -> Vec<Profile> {
    parse_chromium(local_state).unwrap_or_default()
}

fn parse_chromium(local_state: &str) -> Option<Vec<Profile>> {
    let state: serde_json::Value = serde_json::from_str(local_state).ok()?;
    let profile = &state["profile"];
    let cache = profile["info_cache"].as_object()?;
    let last_used = profile["last_used"].as_str();
    let order: Vec<&str> = profile["profiles_order"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let mut profiles: Vec<Profile> = cache
        .iter()
        .filter_map(|(dir, info)| {
            Some(Profile {
                name: info["name"].as_str()?.to_owned(),
                directory: dir.clone(),
                default: last_used == Some(dir.as_str()),
            })
        })
        .collect();
    profiles.sort_by_key(|p| {
        (
            order
                .iter()
                .position(|d| *d == p.directory)
                .unwrap_or(usize::MAX),
            p.directory.clone(),
        )
    });
    if last_used.is_none()
        && let Some(p) = profiles.iter_mut().find(|p| p.directory == "Default")
    {
        p.default = true;
    }
    Some(profiles)
}

/// The `[ProfileN]` sections of a Firefox `profiles.ini`, in file order. The
/// default is the `[Install…]` section's `Default=` path when there is one
/// (Firefox 67+), else the profile marked `Default=1`.
pub(crate) fn firefox_profiles(ini: &str) -> Vec<Profile> {
    let mut profiles = Vec::new();
    let mut install_defaults = Vec::new();
    let mut legacy_default = None;
    for (section, entries) in ini_sections(ini) {
        let get = |key: &str| {
            entries
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v.clone())
        };
        if section.starts_with("Install") {
            install_defaults.extend(get("Default"));
        } else if section.starts_with("Profile")
            && let Some(name) = get("Name")
        {
            let directory = get("Path").unwrap_or_default();
            if get("Default").as_deref() == Some("1") {
                legacy_default = Some(directory.clone());
            }
            profiles.push(Profile {
                name,
                directory,
                default: false,
            });
        }
    }
    for p in &mut profiles {
        p.default = if install_defaults.is_empty() {
            legacy_default.as_deref() == Some(p.directory.as_str())
        } else {
            install_defaults.contains(&p.directory)
        };
    }
    profiles
}

/// A minimal INI reader: `[section]`, `key=value`, `;`/`#` comments.
fn ini_sections(ini: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for line in ini.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push((name.trim().to_owned(), Vec::new()));
        } else if let (Some((key, value)), Some((_, entries))) =
            (line.split_once('='), sections.last_mut())
        {
            entries.push((key.trim().to_owned(), value.trim().to_owned()));
        }
    }
    sections
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Catalog, Platform};

    const WORK: &str = r#"{"profile":{"info_cache":{"Profile 2":{"name":"Work","user_name":"private@example.com"}},"last_used":"Profile 2"},"secret":"not cached"}"#;

    fn denied(_: &Path) -> io::Result<String> {
        Err(io::ErrorKind::PermissionDenied.into())
    }

    fn fixture(root: &Path) -> Env {
        Env::fixture(Platform::MacOs, root).with_profile_cache(root.join("profiles.json"))
    }

    #[test]
    fn a_later_process_uses_saved_metadata_only_when_access_is_denied() {
        let dir = tempfile::tempdir().unwrap();
        let chrome = Catalog::builtin().find("chrome").unwrap();
        let first = read_with(chrome, &fixture(dir.path()), |_| Ok(WORK.into())).unwrap();
        let saved = fs::read_to_string(dir.path().join("profiles.json")).unwrap();
        assert!(!saved.contains("private@example.com") && !saved.contains("not cached"));
        let next = fixture(dir.path());
        assert_eq!(read_with(chrome, &next, denied).unwrap_err().cached, first);
        assert!(
            read_with(chrome, &next, |_| Err(io::ErrorKind::Interrupted.into()))
                .unwrap()
                .is_empty()
        );
        assert_eq!(read_with(chrome, &next, denied).unwrap_err().cached, first);
    }

    #[test]
    fn fresh_metadata_replaces_renames_and_removals_without_resurrecting_them() {
        let dir = tempfile::tempdir().unwrap();
        let env = fixture(dir.path());
        let chrome = Catalog::builtin().find("chrome").unwrap();
        read_with(chrome, &env, |_| Ok(WORK.into())).unwrap();
        let renamed = read_with(chrome, &env, |_| Ok(WORK.replace("Work", "Travel"))).unwrap();
        assert_eq!(read_with(chrome, &env, denied).unwrap_err().cached, renamed);
        read_with(chrome, &env, |_| {
            Ok(r#"{"profile":{"info_cache":{}}}"#.into())
        })
        .unwrap();
        assert!(
            read_with(chrome, &env, denied)
                .unwrap_err()
                .cached
                .is_empty()
        );
        read_with(chrome, &env, |_| Ok(WORK.into())).unwrap();
        read_with(chrome, &env, |_| Err(io::ErrorKind::NotFound.into())).unwrap();
        assert!(
            read_with(chrome, &env, denied)
                .unwrap_err()
                .cached
                .is_empty()
        );
    }

    #[test]
    fn malformed_live_data_does_not_overwrite_the_last_successful_scan() {
        let dir = tempfile::tempdir().unwrap();
        let env = fixture(dir.path());
        let chrome = Catalog::builtin().find("chrome").unwrap();
        let saved = read_with(chrome, &env, |_| Ok(WORK.into())).unwrap();
        assert!(
            read_with(chrome, &env, |_| Ok("broken".into()))
                .unwrap()
                .is_empty()
        );
        assert_eq!(read_with(chrome, &env, denied).unwrap_err().cached, saved);
    }

    #[test]
    fn profiles_are_isolated_by_browser_and_index_location() {
        let dir = tempfile::tempdir().unwrap();
        let env = fixture(dir.path());
        let chrome = Catalog::builtin().find("chrome").unwrap();
        read_with(chrome, &env, |_| Ok(WORK.into())).unwrap();
        let beta = Catalog::builtin().find("chrome-beta").unwrap();
        assert!(read_with(beta, &env, denied).unwrap_err().cached.is_empty());
        let elsewhere = Env::fixture(Platform::MacOs, dir.path().join("another-home"))
            .with_profile_cache(dir.path().join("profiles.json"));
        assert!(
            read_with(chrome, &elsewhere, denied)
                .unwrap_err()
                .cached
                .is_empty()
        );
        let saved = read_with(beta, &env, |_| Ok(WORK.replace("Work", "Beta"))).unwrap();
        assert_eq!(read_with(beta, &env, denied).unwrap_err().cached, saved);
        assert_eq!(
            read_with(chrome, &env, denied).unwrap_err().cached[0].name,
            "Work"
        );
    }

    #[test]
    fn firefox_profiles_survive_a_later_blocked_read() {
        let dir = tempfile::tempdir().unwrap();
        let env = fixture(dir.path());
        let firefox = Catalog::builtin().find("firefox").unwrap();
        let saved = read_with(firefox, &env, |_| {
            Ok("[Profile0]\nName=Work\nPath=Profiles/work\nDefault=1\n".into())
        })
        .unwrap();
        assert_eq!(
            read_with(firefox, &fixture(dir.path()), denied)
                .unwrap_err()
                .cached,
            saved
        );
    }

    #[test]
    fn missing_corrupt_and_unwritable_caches_do_not_break_live_reads() {
        let dir = tempfile::tempdir().unwrap();
        let env = fixture(dir.path());
        let chrome = Catalog::builtin().find("chrome").unwrap();
        assert!(
            read_with(chrome, &env, denied)
                .unwrap_err()
                .cached
                .is_empty()
        );
        fs::write(dir.path().join("profiles.json"), "broken").unwrap();
        assert!(
            read_with(chrome, &env, denied)
                .unwrap_err()
                .cached
                .is_empty()
        );
        let profiles = read_with(chrome, &env, |_| Ok(WORK.into())).unwrap();
        assert_eq!(
            read_with(chrome, &env, denied).unwrap_err().cached,
            profiles
        );
        fs::write(dir.path().join("not-a-directory"), "file").unwrap();
        let unwritable = Env::fixture(Platform::MacOs, dir.path())
            .with_profile_cache(dir.path().join("not-a-directory/profiles.json"));
        assert_eq!(
            read_with(chrome, &unwritable, |_| Ok(WORK.into())).unwrap(),
            profiles
        );
    }

    #[test]
    fn persistence_is_explicitly_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let env = Env::fixture(Platform::MacOs, dir.path());
        let chrome = Catalog::builtin().find("chrome").unwrap();
        read_with(chrome, &env, |_| Ok(WORK.into())).unwrap();
        assert!(
            read_with(chrome, &env, denied)
                .unwrap_err()
                .cached
                .is_empty()
        );
        assert!(!dir.path().join("profiles.json").exists());
    }

    #[test]
    fn chromium_orders_and_defaults() {
        let state = r#"{"profile":{
            "info_cache":{
                "Profile 2":{"name":"Personal"},
                "Default":{"name":"Work"},
                "Profile 1":{"name":"Side Project"},
                "Broken":{"no_name":true}
            },
            "last_used":"Profile 1",
            "profiles_order":["Default","Profile 1"]
        }}"#;
        let p = chromium_profiles(state);
        let got: Vec<_> = p
            .iter()
            .map(|p| (p.name.as_str(), p.directory.as_str(), p.default))
            .collect();
        assert_eq!(
            got,
            [
                ("Work", "Default", false),
                ("Side Project", "Profile 1", true),
                ("Personal", "Profile 2", false),
            ]
        );
    }

    #[test]
    fn chromium_tolerates_garbage() {
        assert!(chromium_profiles("").is_empty());
        assert!(chromium_profiles("[]").is_empty());
        assert!(chromium_profiles(r#"{"profile":{}}"#).is_empty());
    }

    #[test]
    fn firefox_install_default_wins() {
        let ini = "\u{feff}[Install308046B0AF4A39CB]\r\nDefault=Profiles/abc.work\r\nLocked=1\r\n\r\n\
                   [Profile1]\r\nName=Personal\r\nIsRelative=1\r\nPath=Profiles/xyz.default\r\nDefault=1\r\n\r\n\
                   [Profile0]\r\nName=Work Stuff\r\nIsRelative=1\r\nPath=Profiles/abc.work\r\n\r\n\
                   [General]\r\nStartWithLastProfile=1\r\nVersion=2\r\n";
        let p = firefox_profiles(ini);
        assert_eq!(p.len(), 2);
        assert_eq!((p[0].name.as_str(), p[0].default), ("Personal", false));
        assert_eq!(
            (p[1].name.as_str(), p[1].directory.as_str(), p[1].default),
            ("Work Stuff", "Profiles/abc.work", true)
        );
    }

    #[test]
    fn firefox_legacy_default() {
        let p = firefox_profiles("[Profile0]\nName=default\nPath=x.default\nDefault=1\n");
        assert!(p[0].default);
    }
}
