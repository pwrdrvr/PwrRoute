//! Where to look: the platform, the home directory, environment variables,
//! and the filesystem root. Everything that reads the machine takes an
//! [`Env`], so tests point it at a fixture tree and never see the
//! developer's real browsers or profiles.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

/// The operating systems PwrRoute runs on. Planning and parsing work for any
/// of them on any host; only spawning and the Windows registry are native.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    MacOs,
    Windows,
    Linux,
}

impl Platform {
    /// The platform this binary was built for. Other Unixes are treated as
    /// Linux: they share XDG directories and `.desktop` files.
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// A snapshot of the machine the catalog is resolved against.
#[derive(Debug, Clone)]
pub struct Env {
    platform: Platform,
    home: PathBuf,
    vars: HashMap<String, String>,
    /// Prefix for absolute system paths (`/Applications`, `/usr/share`).
    /// `None` on the real machine.
    root: Option<PathBuf>,
    registry: bool,
}

impl Env {
    /// The real machine: this platform, `$HOME` (or `%USERPROFILE%`), the
    /// process environment and, on Windows, the registry.
    pub fn system() -> Self {
        let platform = Platform::current();
        // Not std::env::vars(): it panics on a variable that is not UTF-8.
        // Such variables are skipped; none this crate reads is ever one.
        let vars: Vec<(String, String)> = std::env::vars_os()
            .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
            .collect();
        let home_var = if platform == Platform::Windows {
            "USERPROFILE"
        } else {
            "HOME"
        };
        let mut env = Self {
            platform,
            home: PathBuf::new(),
            vars: HashMap::new(),
            root: None,
            registry: platform == Platform::Windows,
        };
        for (k, v) in vars {
            env.set_var(&k, v);
        }
        env.home = env.var(home_var).map(PathBuf::from).unwrap_or_default();
        env
    }

    /// A fixture machine under `root`: home is `root/home`, system paths are
    /// resolved inside `root`, the registry is off, and the platform's
    /// directory variables (`APPDATA`, `XDG_CONFIG_HOME`, …) point inside
    /// `root` too. `PATH` is `root/bin` (Unix) or `root\bin`; override any
    /// of these with [`Env::set_var`].
    pub fn fixture(platform: Platform, root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let home = root.join("home");
        let mut env = Self {
            platform,
            home: home.clone(),
            vars: HashMap::new(),
            root: Some(root.clone()),
            registry: false,
        };
        let s = |p: PathBuf| p.to_string_lossy().into_owned();
        // Component by component, so the host's separator is used throughout.
        let under = |base: &Path, rel: &str| {
            let mut p = base.to_path_buf();
            p.extend(rel.split('/'));
            s(p)
        };
        match platform {
            Platform::Windows => {
                env.set_var("USERPROFILE", s(home.clone()));
                env.set_var("APPDATA", under(&home, "AppData/Roaming"));
                env.set_var("LOCALAPPDATA", under(&home, "AppData/Local"));
                env.set_var("PROGRAMFILES", under(&root, "Program Files"));
                env.set_var("PROGRAMFILES(X86)", under(&root, "Program Files (x86)"));
                env.set_var("PATHEXT", ".COM;.EXE;.BAT;.CMD".into());
            }
            Platform::MacOs | Platform::Linux => env.set_var("HOME", s(home.clone())),
        }
        env.set_var("PATH", s(root.join("bin")));
        env
    }

    pub fn platform(&self) -> Platform {
        self.platform
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Sets an environment variable as this `Env` sees it (not the process's).
    /// Windows names are case-insensitive, as they are on Windows.
    pub fn set_var(&mut self, name: &str, value: String) {
        self.vars.insert(self.key(name), value);
    }

    /// Removes an environment variable from this `Env`.
    pub fn remove_var(&mut self, name: &str) {
        self.vars.remove(&self.key(name));
    }

    /// A non-empty environment variable.
    pub fn var(&self, name: &str) -> Option<&str> {
        self.vars
            .get(&self.key(name))
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    /// Whether to read the Windows registry. Off for fixtures.
    pub(crate) fn registry(&self) -> bool {
        self.registry && cfg!(windows)
    }

    fn key(&self, name: &str) -> String {
        if self.platform == Platform::Windows {
            name.to_ascii_uppercase()
        } else {
            name.to_owned()
        }
    }

    /// An absolute system path (`/Applications`, `/usr/share/applications`),
    /// moved under the fixture root when there is one.
    pub(crate) fn system_path(&self, path: &str) -> PathBuf {
        match &self.root {
            Some(root) => root.join(path.trim_start_matches('/')),
            None => PathBuf::from(path),
        }
    }

    /// `$PATH`, split the host's way: a real `Env` is the host, and a
    /// fixture's `PATH` holds host paths (`C:\…` on a Windows runner).
    pub(crate) fn path_dirs(&self) -> Vec<PathBuf> {
        self.var("PATH")
            .map(|p| {
                std::env::split_paths(p)
                    .filter(|d| !d.as_os_str().is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Looks `name` up on `$PATH` (with `%PATHEXT%` on Windows). A name that
    /// already contains a separator is returned as is when it is a file.
    pub(crate) fn which(&self, name: &str) -> Option<PathBuf> {
        if name.contains('/') || name.contains('\\') {
            let p = PathBuf::from(name);
            return p.is_file().then_some(p);
        }
        let exts: Vec<String> =
            if self.platform == Platform::Windows && Path::new(name).extension().is_none() {
                self.var("PATHEXT")
                    .unwrap_or(".EXE")
                    .split(';')
                    .map(|e| e.to_ascii_lowercase())
                    .collect()
            } else {
                vec![String::new()]
            };
        self.path_dirs().into_iter().find_map(|dir| {
            exts.iter()
                .map(|ext| dir.join(format!("{name}{ext}")))
                .find(|p| p.is_file())
        })
    }

    /// Expands a catalog path template. Templates use `/` and one leading
    /// token naming a base directory; `{home}` everywhere; `{app_support}` on macOS; `{appdata}`,
    /// `{localappdata}`, `{programfiles}` and `{programfiles_x86}` on Windows;
    /// `{xdg_config}` and `{xdg_data}` on Linux. `None` when the token's
    /// variable is unset, or the token is unknown or for another platform.
    pub fn expand(&self, template: &str) -> Option<PathBuf> {
        let (token, rest) = split_token(template)?;
        let base = self.token_dir(token)?;
        let mut path = base;
        for part in rest.split('/').filter(|p| !p.is_empty()) {
            path.push(part);
        }
        Some(path)
    }

    fn token_dir(&self, token: &str) -> Option<PathBuf> {
        if !token_allowed(token, self.platform) {
            return None;
        }
        let from_var = |name: &str| self.var(name).map(PathBuf::from);
        // XDG base directories must be absolute; relative values are ignored.
        let xdg_var = |name: &str| from_var(name).filter(|p| p.is_absolute());
        // No home directory (HOME unset) means no home-relative paths, rather
        // than paths relative to whatever the current directory is.
        let home = Some(self.home.clone()).filter(|h| !h.as_os_str().is_empty());
        match token {
            "home" => home,
            "app_support" => home.map(|h| h.join("Library").join("Application Support")),
            "appdata" => from_var("APPDATA"),
            "localappdata" => from_var("LOCALAPPDATA"),
            "programfiles" => from_var("PROGRAMFILES"),
            "programfiles_x86" => from_var("PROGRAMFILES(X86)"),
            "xdg_config" => xdg_var("XDG_CONFIG_HOME").or_else(|| home.map(|h| h.join(".config"))),
            "xdg_data" => {
                xdg_var("XDG_DATA_HOME").or_else(|| home.map(|h| h.join(".local").join("share")))
            }
            _ => None,
        }
    }
}

/// The base-directory tokens a catalog path template may start with, and the
/// platforms each belongs to.
const TOKENS: &[(&str, &[Platform])] = &[
    (
        "home",
        &[Platform::MacOs, Platform::Windows, Platform::Linux],
    ),
    ("app_support", &[Platform::MacOs]),
    ("appdata", &[Platform::Windows]),
    ("localappdata", &[Platform::Windows]),
    ("programfiles", &[Platform::Windows]),
    ("programfiles_x86", &[Platform::Windows]),
    ("xdg_config", &[Platform::Linux]),
    ("xdg_data", &[Platform::Linux]),
];

/// `{token}/rest` → `("token", "rest")`.
pub(crate) fn split_token(template: &str) -> Option<(&str, &str)> {
    let rest = template.strip_prefix('{')?;
    let (token, rest) = rest.split_once('}')?;
    Some((token, rest.strip_prefix('/').unwrap_or(rest)))
}

/// Whether `token` may be used in templates for `platform`.
pub(crate) fn token_allowed(token: &str, platform: Platform) -> bool {
    TOKENS
        .iter()
        .any(|(t, platforms)| *t == token && platforms.contains(&platform))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_tokens_per_platform() {
        let env = Env::fixture(Platform::Windows, "/r");
        assert_eq!(
            env.expand("{localappdata}/Google/Chrome/User Data"),
            Some(PathBuf::from(
                "/r/home/AppData/Local/Google/Chrome/User Data"
            ))
        );
        assert_eq!(env.expand("{xdg_config}/chromium"), None);
        assert_eq!(env.expand("{nope}/x"), None);
        assert_eq!(env.expand("no-token"), None);

        let mut env = Env::fixture(Platform::Linux, "/r");
        assert_eq!(
            env.expand("{xdg_config}/chromium"),
            Some(PathBuf::from("/r/home/.config/chromium"))
        );
        env.set_var("XDG_CONFIG_HOME", "/cfg".into());
        assert_eq!(
            env.expand("{xdg_config}/chromium"),
            Some(PathBuf::from("/cfg/chromium"))
        );
        env.set_var("XDG_CONFIG_HOME", "relative/cfg".into());
        assert_eq!(
            env.expand("{xdg_config}/chromium"),
            Some(PathBuf::from("/r/home/.config/chromium"))
        );
    }

    #[test]
    fn no_home_means_no_home_paths() {
        let mut env = Env::fixture(Platform::MacOs, "/r");
        env.home = PathBuf::new();
        assert_eq!(env.expand("{app_support}/Google/Chrome"), None);
        assert_eq!(env.expand("{home}/x"), None);
    }

    #[test]
    fn windows_variables_ignore_case() {
        let mut env = Env::fixture(Platform::Windows, "/r");
        env.set_var("LocalAppData", "/x".into());
        assert_eq!(env.var("LOCALAPPDATA"), Some("/x"));
    }
}
