//! Browser profiles: Chromium's `Local State` (`profile.info_cache`) and
//! Firefox's `profiles.ini`, and mapping a config's profile name to the
//! argument the browser needs. Ported from Finicky's launcher
//! (MIT, Copyright (c) 2015-2025 John Sterling).

use std::{fs, path::Path};

use crate::{
    catalog::{BrowserInfo, ProfileFlag},
    env::Env,
};

/// One profile as the browser lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
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
/// profile index cannot be read.
pub fn list_profiles(browser: &BrowserInfo, env: &Env) -> Vec<Profile> {
    let Some(dir) = browser.profile_dir(env) else {
        return Vec::new();
    };
    match browser.profile_flag() {
        ProfileFlag::Chromium => read(&dir.join("Local State"))
            .map(|s| chromium_profiles(&s))
            .unwrap_or_default(),
        ProfileFlag::Firefox => read(&dir.join("profiles.ini"))
            .map(|s| firefox_profiles(&s))
            .unwrap_or_default(),
        ProfileFlag::None => Vec::new(),
    }
}

/// How a profile request turned out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProfileMatch {
    /// The arguments that select it.
    Found(Vec<String>),
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
    let profiles = list_profiles(browser, env);
    let found = profiles
        .iter()
        .find(|p| p.name == wanted)
        .or_else(|| profiles.iter().find(|p| p.directory == wanted));
    match (found, flag) {
        (Some(p), ProfileFlag::Chromium) => {
            ProfileMatch::Found(vec![format!("--profile-directory={}", p.directory)])
        }
        // `-P` takes the profile's name, not its path.
        (Some(p), _) => ProfileMatch::Found(vec!["-P".into(), p.name.clone()]),
        (None, _) => ProfileMatch::NotFound(profiles.into_iter().map(|p| p.name).collect()),
    }
}

fn read(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

/// `profile.info_cache` from a Chromium `Local State`, ordered by
/// `profile.profiles_order` when present, then by directory.
pub(crate) fn chromium_profiles(local_state: &str) -> Vec<Profile> {
    let Ok(state) = serde_json::from_str::<serde_json::Value>(local_state) else {
        return Vec::new();
    };
    let profile = &state["profile"];
    let Some(cache) = profile["info_cache"].as_object() else {
        return Vec::new();
    };
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
    profiles
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
