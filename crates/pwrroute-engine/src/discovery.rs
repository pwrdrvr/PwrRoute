//! Where the config and `rules.json` live.
//!
//! The config is the first of these that is a file (a symlink is followed),
//! the Go host's list in its order, on every OS:
//!
//! 1. `~/.finicky.js`, `~/.finicky.ts`
//! 2. `~/.config/finicky.js`, `~/.config/finicky.ts`
//! 3. `~/.config/finicky/finicky.js`, `~/.config/finicky/finicky.ts`
//!
//! then, where the platform keeps configuration elsewhere:
//!
//! - Linux: the same two forms under `$XDG_CONFIG_HOME`, when it is set to an
//!   absolute path other than `~/.config`.
//! - Windows: `%APPDATA%\finicky\finicky.js` and `.ts`, the AppData
//!   counterpart of `~/.config/finicky/`.
//!
//! The Go host also accepted a directory at a candidate path and then failed
//! to read it; a directory is skipped here.
//!
//! `rules.json` is PwrRoute's own, in the OS's per-user configuration
//! directory, as the Go host kept Finicky's: `~/Library/Application
//! Support/PwrRoute/rules.json` on macOS, `$XDG_CONFIG_HOME/PwrRoute/` (or
//! `~/.config/PwrRoute/`) on Linux, and `%APPDATA%\PwrRoute\` on Windows.
//!
//! Only these exact paths are ever checked. Nothing here lists or watches a
//! directory: on macOS that can touch protected folders and prompt the user.

use std::{
    env,
    path::{Path, PathBuf},
};

/// The OS facts discovery depends on, so tests can supply their own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Environment {
    pub home: Option<PathBuf>,
    /// `$XDG_CONFIG_HOME`, used on Linux.
    pub xdg_config_home: Option<PathBuf>,
    /// `%APPDATA%`, used on Windows.
    pub app_data: Option<PathBuf>,
}

impl Environment {
    /// This process's environment.
    pub fn current() -> Self {
        let absolute = |key: &str| {
            env::var_os(key)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        };
        Self {
            home: env::home_dir().filter(|p| !p.as_os_str().is_empty()),
            xdg_config_home: absolute("XDG_CONFIG_HOME"),
            app_data: absolute("APPDATA"),
        }
    }
}

/// Every path a config may be at, in the order they are tried.
pub fn config_candidates(env: &Environment) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let pair = |paths: &mut Vec<PathBuf>, dir: &Path, stem: &str| {
        for ext in ["js", "ts"] {
            paths.push(dir.join(format!("{stem}.{ext}")));
        }
    };
    if let Some(home) = &env.home {
        pair(&mut paths, home, ".finicky");
        let dot_config = home.join(".config");
        pair(&mut paths, &dot_config, "finicky");
        pair(&mut paths, &dot_config.join("finicky"), "finicky");
    }
    if cfg!(target_os = "linux")
        && let Some(xdg) = &env.xdg_config_home
        && env.home.as_ref().map(|h| h.join(".config")).as_ref() != Some(xdg)
    {
        pair(&mut paths, xdg, "finicky");
        pair(&mut paths, &xdg.join("finicky"), "finicky");
    }
    if cfg!(windows)
        && let Some(app_data) = &env.app_data
    {
        pair(&mut paths, &app_data.join("finicky"), "finicky");
    }
    paths
}

/// The first candidate that is a file, with symlinks resolved.
pub fn find_config(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates
        .iter()
        .find(|p| p.is_file())
        .map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()))
}

/// Where PwrRoute keeps `rules.json`.
pub fn rules_path(env: &Environment) -> Option<PathBuf> {
    config_dir(env).map(|d| d.join("PwrRoute").join("rules.json"))
}

/// The per-user configuration directory, as Go's `os.UserConfigDir` and
/// Rust's `dirs` define it.
fn config_dir(env: &Environment) -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        env.home
            .as_ref()
            .map(|h| h.join("Library").join("Application Support"))
    } else if cfg!(windows) {
        env.app_data.clone()
    } else {
        env.xdg_config_home
            .clone()
            .or_else(|| env.home.as_ref().map(|h| h.join(".config")))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("pwrroute-discovery-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn finicky_order_comes_first() {
        let env = Environment {
            home: Some(PathBuf::from("/home/u")),
            ..Environment::default()
        };
        let got = config_candidates(&env);
        let want: Vec<PathBuf> = [
            "/home/u/.finicky.js",
            "/home/u/.finicky.ts",
            "/home/u/.config/finicky.js",
            "/home/u/.config/finicky.ts",
            "/home/u/.config/finicky/finicky.js",
            "/home/u/.config/finicky/finicky.ts",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(got[..6], want[..]);
    }

    #[test]
    fn platform_locations_follow() {
        let env = Environment {
            home: Some(PathBuf::from("/home/u")),
            xdg_config_home: Some(PathBuf::from("/xdg")),
            app_data: Some(PathBuf::from("/appdata")),
        };
        let extra = &config_candidates(&env)[6..];
        if cfg!(target_os = "linux") {
            assert_eq!(extra[0], PathBuf::from("/xdg/finicky.js"));
            assert_eq!(extra[3], PathBuf::from("/xdg/finicky/finicky.ts"));
        } else if cfg!(windows) {
            assert_eq!(extra[0], PathBuf::from("/appdata/finicky/finicky.js"));
        } else {
            assert!(extra.is_empty(), "{extra:?}");
        }
    }

    #[test]
    fn xdg_at_the_default_location_is_not_listed_twice() {
        let env = Environment {
            home: Some(PathBuf::from("/home/u")),
            xdg_config_home: Some(PathBuf::from("/home/u/.config")),
            ..Environment::default()
        };
        assert_eq!(config_candidates(&env).len(), 6);
    }

    #[test]
    fn finds_the_first_file_and_skips_directories() {
        let home = scratch("find");
        let env = Environment {
            home: Some(home.clone()),
            ..Environment::default()
        };
        let candidates = config_candidates(&env);
        assert_eq!(find_config(&candidates), None);
        fs::create_dir_all(home.join(".finicky.js")).unwrap();
        fs::create_dir_all(home.join(".config/finicky")).unwrap();
        fs::write(home.join(".config/finicky/finicky.ts"), "export default {}").unwrap();
        assert_eq!(
            find_config(&candidates),
            Some(home.join(".config/finicky/finicky.ts"))
        );
        fs::write(home.join(".finicky.ts"), "export default {}").unwrap();
        assert_eq!(find_config(&candidates), Some(home.join(".finicky.ts")));
        fs::remove_dir_all(home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_config_resolves_to_its_target() {
        let home = scratch("symlink");
        fs::write(home.join("real.js"), "export default {}").unwrap();
        std::os::unix::fs::symlink(home.join("real.js"), home.join(".finicky.js")).unwrap();
        let env = Environment {
            home: Some(home.clone()),
            ..Environment::default()
        };
        assert_eq!(
            find_config(&config_candidates(&env)),
            Some(home.join("real.js"))
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn rules_live_in_the_user_config_dir() {
        let env = Environment {
            home: Some(PathBuf::from("/home/u")),
            xdg_config_home: None,
            app_data: Some(PathBuf::from("/appdata")),
        };
        let path = rules_path(&env).unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(
                path,
                PathBuf::from("/home/u/Library/Application Support/PwrRoute/rules.json")
            );
        } else if cfg!(windows) {
            assert_eq!(path, PathBuf::from("/appdata/PwrRoute/rules.json"));
        } else {
            assert_eq!(path, PathBuf::from("/home/u/.config/PwrRoute/rules.json"));
        }
    }
}
