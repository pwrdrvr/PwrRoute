//! Where cloud-drive clients keep their folders, per OS.
//!
//! Only exact paths are checked, with one exception: Google Drive for
//! desktop on macOS names a folder per account
//! (`~/Library/CloudStorage/GoogleDrive-<account>`), so that one directory
//! is listed, once per call. Nothing here watches a directory, and nothing
//! reads inside a cloud folder.
//!
//! | Kind | macOS | Linux | Windows |
//! | --- | --- | --- | --- |
//! | iCloud Drive | `~/Library/Mobile Documents/com~apple~CloudDocs` | none | `%USERPROFILE%\iCloudDrive` |
//! | Google Drive | `~/Library/CloudStorage/GoogleDrive-*/My Drive` | `~/Google Drive`, `~/GoogleDrive` (rclone, Insync) | `G:\My Drive`, `%USERPROFILE%\Google Drive` |
//! | Dropbox | `~/Library/CloudStorage/Dropbox`, `info.json`'s paths, `~/Dropbox` | `info.json`'s paths, `~/Dropbox` | `info.json`'s paths, `%USERPROFILE%\Dropbox` |
//!
//! Dropbox's `info.json` is `~/.dropbox/info.json` (Windows:
//! `%LOCALAPPDATA%\Dropbox\info.json` or `%APPDATA%\Dropbox\info.json`),
//! with a `path` for its `personal` and `business` accounts.
//!
//! Anything else (an rclone or GNOME Online Accounts mount, a network
//! share, OneDrive) is a folder the user picks: [`Kind::Folder`].

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

/// Which client a sync folder belongs to; it decides only the label and
/// how a file that is not downloaded yet looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    ICloud,
    GoogleDrive,
    Dropbox,
    Folder,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::ICloud => "iCloud Drive",
            Self::GoogleDrive => "Google Drive",
            Self::Dropbox => "Dropbox",
            Self::Folder => "Folder",
        }
    }
}

/// A cloud folder found on this machine: where a client keeps its files.
/// PwrRoute syncs to `<root>/PwrRoute`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    pub kind: Kind,
    /// "Google Drive · jo@example.com", "Dropbox", "iCloud Drive".
    pub label: String,
    pub root: PathBuf,
}

/// The OS facts detection needs, so tests can supply their own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    pub os: Os,
    pub home: Option<PathBuf>,
    /// `%USERPROFILE%`, `%LOCALAPPDATA%`, `%APPDATA%` on Windows.
    pub user_profile: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
    pub app_data: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Os {
    #[default]
    MacOs,
    Linux,
    Windows,
}

impl Env {
    /// This process's environment.
    pub fn current() -> Self {
        let var = |key: &str| {
            std::env::var_os(key)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        };
        Self {
            os: if cfg!(target_os = "macos") {
                Os::MacOs
            } else if cfg!(windows) {
                Os::Windows
            } else {
                Os::Linux
            },
            home: std::env::home_dir().filter(|p| !p.as_os_str().is_empty()),
            user_profile: var("USERPROFILE"),
            local_app_data: var("LOCALAPPDATA"),
            app_data: var("APPDATA"),
        }
    }
}

/// The cloud folders present on this machine, in the order the Sync pane
/// offers them: iCloud Drive, Google Drive accounts, Dropbox accounts.
pub fn detect(env: &Env) -> Vec<Place> {
    let mut found: Vec<Place> = Vec::new();
    // Compared by real path: `~/Dropbox` is often a link to the
    // CloudStorage folder, and info.json names the same one.
    let mut real: Vec<PathBuf> = Vec::new();
    let mut add = |kind: Kind, label: String, root: PathBuf| {
        if !root.is_dir() {
            return;
        }
        let canonical = fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
        if !real.contains(&canonical) {
            real.push(canonical);
            found.push(Place { kind, label, root });
        }
    };
    let home = env.home.as_deref();
    let profile = env.user_profile.as_deref().or(home);
    match env.os {
        Os::MacOs => {
            if let Some(home) = home {
                add(
                    Kind::ICloud,
                    Kind::ICloud.name().into(),
                    home.join("Library/Mobile Documents/com~apple~CloudDocs"),
                );
                let storage = home.join("Library/CloudStorage");
                for (account, dir) in google_drive_accounts(&storage) {
                    add(
                        Kind::GoogleDrive,
                        format!("Google Drive · {account}"),
                        dir.join("My Drive"),
                    );
                }
                add(
                    Kind::Dropbox,
                    Kind::Dropbox.name().into(),
                    storage.join("Dropbox"),
                );
            }
        }
        Os::Windows => {
            if let Some(profile) = profile {
                add(
                    Kind::ICloud,
                    Kind::ICloud.name().into(),
                    profile.join("iCloudDrive"),
                );
            }
            add(
                Kind::GoogleDrive,
                Kind::GoogleDrive.name().into(),
                PathBuf::from(r"G:\My Drive"),
            );
            if let Some(profile) = profile {
                add(
                    Kind::GoogleDrive,
                    Kind::GoogleDrive.name().into(),
                    profile.join("Google Drive"),
                );
            }
        }
        Os::Linux => {
            if let Some(home) = home {
                for name in ["Google Drive", "GoogleDrive"] {
                    add(
                        Kind::GoogleDrive,
                        Kind::GoogleDrive.name().into(),
                        home.join(name),
                    );
                }
            }
        }
    }
    let accounts = dropbox_accounts(env);
    let fallback = accounts.is_empty();
    for (label, path) in accounts {
        add(Kind::Dropbox, label, path);
    }
    if fallback && let Some(dir) = profile {
        add(
            Kind::Dropbox,
            Kind::Dropbox.name().into(),
            dir.join("Dropbox"),
        );
    }
    found
}

/// `GoogleDrive-<account>` folders in `~/Library/CloudStorage`, by account.
fn google_drive_accounts(storage: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(storage) else {
        return Vec::new();
    };
    let mut accounts: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let account = name.strip_prefix("GoogleDrive-")?.to_owned();
            Some((account, entry.path()))
        })
        .collect();
    accounts.sort();
    accounts
}

/// The accounts Dropbox's `info.json` names.
fn dropbox_accounts(env: &Env) -> Vec<(String, PathBuf)> {
    let candidates: Vec<PathBuf> = match env.os {
        Os::Windows => [&env.local_app_data, &env.app_data]
            .into_iter()
            .flatten()
            .map(|d| d.join("Dropbox").join("info.json"))
            .collect(),
        _ => env
            .home
            .iter()
            .map(|h| h.join(".dropbox").join("info.json"))
            .collect(),
    };
    for path in candidates {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(info) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let mut accounts = Vec::new();
        for (key, label) in [("personal", "Dropbox"), ("business", "Dropbox · business")] {
            if let Some(p) = info[key]["path"].as_str() {
                accounts.push((label.to_owned(), PathBuf::from(p)));
            }
        }
        return accounts;
    }
    Vec::new()
}

/// Names a folder the user picked, by the client whose folder holds it,
/// so a Dropbox folder picked by hand still reads "Dropbox".
pub fn classify(path: &Path, env: &Env) -> Kind {
    detect(env)
        .into_iter()
        .find(|place| path.starts_with(&place.root))
        .map_or(Kind::Folder, |place| place.kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mac(home: &Path) -> Env {
        Env {
            os: Os::MacOs,
            home: Some(home.to_path_buf()),
            ..Env::default()
        }
    }

    #[test]
    fn finds_the_macos_clients_present_and_skips_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        assert!(detect(&mac(home)).is_empty());
        for sub in [
            "Library/Mobile Documents/com~apple~CloudDocs",
            "Library/CloudStorage/GoogleDrive-jo@example.com/My Drive",
            "Library/CloudStorage/GoogleDrive-al@example.org/My Drive",
            "Library/CloudStorage/GoogleDrive-broken@example.org",
            "Library/CloudStorage/OneDrive-Personal",
            "Library/CloudStorage/Dropbox",
        ] {
            fs::create_dir_all(home.join(sub)).unwrap();
        }
        let labels: Vec<(Kind, String)> = detect(&mac(home))
            .into_iter()
            .map(|p| (p.kind, p.label))
            .collect();
        assert_eq!(
            labels,
            vec![
                (Kind::ICloud, "iCloud Drive".into()),
                (Kind::GoogleDrive, "Google Drive · al@example.org".into()),
                (Kind::GoogleDrive, "Google Drive · jo@example.com".into()),
                (Kind::Dropbox, "Dropbox".into()),
            ]
        );
    }

    #[test]
    fn dropbox_info_json_names_the_accounts() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let personal = home.join("Dropbox (Personal)");
        let business = home.join("Dropbox (Example Co)");
        fs::create_dir_all(&personal).unwrap();
        fs::create_dir_all(&business).unwrap();
        fs::create_dir_all(home.join(".dropbox")).unwrap();
        fs::write(
            home.join(".dropbox/info.json"),
            serde_json::json!({
                "personal": { "path": personal },
                "business": { "path": business },
            })
            .to_string(),
        )
        .unwrap();
        let env = Env {
            os: Os::Linux,
            home: Some(home.to_path_buf()),
            ..Env::default()
        };
        let found = detect(&env);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].root, personal);
        assert_eq!(found[1].label, "Dropbox · business");
        assert_eq!(classify(&personal.join("PwrRoute"), &env), Kind::Dropbox);
        assert_eq!(classify(&home.join("elsewhere"), &env), Kind::Folder);
    }

    #[test]
    fn linux_and_windows_paths() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        fs::create_dir_all(home.join("Dropbox")).unwrap();
        fs::create_dir_all(home.join("GoogleDrive")).unwrap();
        fs::create_dir_all(home.join("iCloudDrive")).unwrap();
        let linux = Env {
            os: Os::Linux,
            home: Some(home.to_path_buf()),
            ..Env::default()
        };
        let kinds: Vec<Kind> = detect(&linux).into_iter().map(|p| p.kind).collect();
        assert_eq!(
            kinds,
            vec![Kind::GoogleDrive, Kind::Dropbox],
            "no iCloud on Linux"
        );
        let windows = Env {
            os: Os::Windows,
            user_profile: Some(home.to_path_buf()),
            ..Env::default()
        };
        let kinds: Vec<Kind> = detect(&windows).into_iter().map(|p| p.kind).collect();
        assert_eq!(kinds, vec![Kind::ICloud, Kind::Dropbox]);
    }
}
