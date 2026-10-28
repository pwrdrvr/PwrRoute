//! The browser catalog: `data/browsers.json`, embedded at build time.
//!
//! It starts from Finicky's `browsers.json` (MIT, Copyright (c) 2015-2025
//! John Sterling; every entry there has `"finicky": true` here) and adds the
//! Windows and Linux identifiers, install locations and profile directories
//! Finicky never needed, plus browsers Finicky lacks. Each entry lists the
//! sources its facts came from.

use std::{path::Path, sync::OnceLock};

use serde::Deserialize;

use crate::env::{Env, Platform};

/// Every browser PwrRoute knows how to find, list profiles for and launch.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    /// Where the data came from; not used by code.
    #[serde(rename = "_origin")]
    _origin: Vec<String>,
    browsers: Vec<BrowserInfo>,
}

/// One browser (one channel of a browser: Chrome Beta is its own entry).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct BrowserInfo {
    /// A stable kebab-case key, e.g. `chrome-beta`.
    pub id: String,
    /// The display name. On macOS it is also the app name Finicky configs use.
    pub name: String,
    pub engine: BrowserEngine,
    /// A verified singleton wire format. Chromium derivatives can change
    /// it while still replying ACK, so the engine alone is not enough.
    #[serde(default)]
    pub(crate) singleton_protocol: Option<SingletonProtocol>,
    /// Present in Finicky's `browsers.json`.
    #[serde(default)]
    pub finicky: bool,
    /// Other names a config may use for it.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Overrides the engine's profile flag, for browsers that ignore it.
    #[serde(default)]
    profile_flag: Option<ProfileFlag>,
    /// Overrides the engine's private-window flag: Edge spells it
    /// `--inprivate`; Arc and Dia take none.
    #[serde(default)]
    private_flag: Option<PrivateFlag>,
    /// What the browser's own menus call a private window, where it is not
    /// the flag's word (Brave, Vivaldi and Opera say "Private window").
    #[serde(default)]
    private_label: Option<String>,
    pub macos: Option<MacOs>,
    pub windows: Option<Windows>,
    pub linux: Option<Linux>,
    /// Where each fact above came from.
    pub sources: Vec<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BrowserEngine {
    Chromium,
    Firefox,
    WebKit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SingletonProtocol {
    /// Upstream's START, cwd, argv (process_singleton_posix.cc).
    Chromium,
    /// Microsoft Edge's: START, cwd, the wall clock and the uptime in
    /// microseconds, argv (captured from Edge 156's own sender; see
    /// `singleton.rs`).
    Edge,
}

impl SingletonProtocol {
    /// Its catalog name, for diagnostics.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Chromium => "chromium",
            Self::Edge => "edge",
        }
    }
}

/// How a browser is told which profile to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfileFlag {
    /// `--profile-directory=<dir>`, the directory key from `Local State`.
    Chromium,
    /// `-P <name>`, a profile name from `profiles.ini`.
    Firefox,
    /// No command-line profile selection.
    None,
}

/// How a browser is told to open a URL in a private window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum PrivateFlag {
    /// `--incognito`, Chromium's switch.
    #[serde(rename = "incognito")]
    Incognito,
    /// `--inprivate`, Microsoft Edge's spelling.
    #[serde(rename = "inprivate")]
    InPrivate,
    /// `--private-window`, Firefox's.
    #[serde(rename = "private-window")]
    PrivateWindow,
    /// No command-line way (Safari, Orion, Arc).
    #[serde(rename = "none")]
    None,
}

impl PrivateFlag {
    /// Every flag a config's `args` may already carry.
    const KNOWN: [PrivateFlag; 3] = [Self::Incognito, Self::InPrivate, Self::PrivateWindow];

    /// The argument that asks for a private window, if there is one.
    pub fn arg(self) -> Option<&'static str> {
        match self {
            Self::Incognito => Some("--incognito"),
            Self::InPrivate => Some("--inprivate"),
            Self::PrivateWindow => Some("--private-window"),
            Self::None => Option::None,
        }
    }

    /// What browsers with this flag call a private window: "Incognito",
    /// "InPrivate", "Private window".
    pub fn label(self) -> Option<&'static str> {
        match self {
            Self::Incognito => Some("Incognito"),
            Self::InPrivate => Some("InPrivate"),
            Self::PrivateWindow => Some("Private window"),
            Self::None => Option::None,
        }
    }

    /// The flag an argument is, if it is one of the known private-window
    /// flags (so a config whose `args` carry one is not given it twice).
    pub fn from_arg(arg: &str) -> Option<PrivateFlag> {
        Self::KNOWN.into_iter().find(|f| f.arg() == Some(arg))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct MacOs {
    /// `None` only where no source confirms it; the app name still works.
    #[serde(default)]
    pub bundle_id: Option<String>,
    /// The bundle's name without `.app`.
    pub app_name: String,
    /// Path templates of the user-data directory (`Local State` or
    /// `profiles.ini` lives in it). Empty when profiles are not supported.
    #[serde(default)]
    pub profiles: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Windows {
    /// Path templates of the executable, per-machine and per-user installs.
    pub exe: Vec<String>,
    /// The URL-handler ProgID it registers. A trailing `-` means the
    /// installer appends a per-install hash (Firefox's `FirefoxURL-<hash>`).
    /// Chromium per-user installs may append `.<suffix>`; that matches too.
    #[serde(default)]
    pub prog_id: Option<String>,
    #[serde(default)]
    pub profiles: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Linux {
    /// Desktop entry ids, native packages first, then Flatpak and Snap.
    #[serde(default)]
    pub desktop_ids: Vec<String>,
    /// Executable names looked up on `$PATH`.
    #[serde(default)]
    pub executables: Vec<String>,
    /// User-data directory templates, in the order the browser itself would
    /// pick them (native, then Snap, then Flatpak). The first that exists is
    /// used.
    #[serde(default)]
    pub profiles: Vec<String>,
}

static BUILTIN: OnceLock<Catalog> = OnceLock::new();

impl Catalog {
    /// The embedded catalog. The data is validated by this crate's tests, so
    /// a parse failure here is a build defect.
    pub fn builtin() -> &'static Catalog {
        BUILTIN.get_or_init(|| {
            serde_json::from_str(include_str!("../data/browsers.json"))
                .expect("data/browsers.json is valid; see catalog tests")
        })
    }

    pub fn browsers(&self) -> &[BrowserInfo] {
        &self.browsers
    }

    /// The entry a config's browser name refers to: its id, display name,
    /// an alias, a macOS bundle id or app name, a Linux desktop id (with or
    /// without `.desktop`), or a Windows ProgID. Case-insensitive, like
    /// Launch Services bundle ids. Finicky matched the bundle id and app name
    /// exactly; these are a superset.
    pub fn find(&self, name: &str) -> Option<&BrowserInfo> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let eq = |s: &str| s.eq_ignore_ascii_case(name);
        self.browsers.iter().find(|b| {
            eq(&b.id)
                || eq(&b.name)
                || b.aliases.iter().any(|a| eq(a))
                || b.macos
                    .as_ref()
                    .is_some_and(|m| m.bundle_id.as_deref().is_some_and(eq) || eq(&m.app_name))
                || b.linux.as_ref().is_some_and(|l| {
                    l.desktop_ids
                        .iter()
                        .any(|d| eq(d) || eq(d.trim_end_matches(".desktop")))
                })
                || b.windows
                    .as_ref()
                    .and_then(|w| w.prog_id.as_deref())
                    .is_some_and(|p| prog_id_matches(p, name))
        })
    }

    /// The entry with this macOS bundle id (or an alias, which is how older
    /// bundle ids are listed). Unlike [`Catalog::find`], a display name or
    /// another platform's id does not match.
    pub fn find_bundle_id(&self, bundle_id: &str) -> Option<&BrowserInfo> {
        let eq = |s: &str| s.eq_ignore_ascii_case(bundle_id);
        self.browsers
            .iter()
            .find(|b| {
                b.macos
                    .as_ref()
                    .is_some_and(|m| m.bundle_id.as_deref().is_some_and(eq))
            })
            .or_else(|| {
                self.browsers
                    .iter()
                    .find(|b| b.macos.is_some() && b.aliases.iter().any(|a| eq(a)))
            })
    }

    /// The entry an app path belongs to on `platform`: a `.app` bundle by
    /// its name, a Windows executable by its install path, or a Linux
    /// executable or desktop file by its file name.
    pub fn find_by_path(&self, platform: Platform, path: &Path) -> Option<&BrowserInfo> {
        let file = path.file_name()?.to_str()?;
        match platform {
            Platform::MacOs => {
                let stem = file.strip_suffix(".app")?;
                self.browsers.iter().find(|b| {
                    b.macos
                        .as_ref()
                        .is_some_and(|m| m.app_name.eq_ignore_ascii_case(stem))
                })
            }
            Platform::Windows => {
                let path = normalize_windows(&path.to_string_lossy());
                self.browsers.iter().find(|b| {
                    b.windows.as_ref().is_some_and(|w| {
                        w.exe.iter().any(|t| {
                            crate::env::split_token(t)
                                .is_some_and(|(_, rel)| ends_with_component(&path, rel))
                        })
                    })
                })
            }
            Platform::Linux => self.browsers.iter().find(|b| {
                b.linux.as_ref().is_some_and(|l| {
                    l.executables.iter().any(|e| e == file)
                        || l.desktop_ids.iter().any(|d| d == file)
                })
            }),
        }
    }
}

impl BrowserInfo {
    /// How this browser selects a profile on the command line.
    pub fn profile_flag(&self) -> ProfileFlag {
        self.profile_flag.unwrap_or(match self.engine {
            BrowserEngine::Chromium => ProfileFlag::Chromium,
            BrowserEngine::Firefox => ProfileFlag::Firefox,
            BrowserEngine::WebKit => ProfileFlag::None,
        })
    }

    /// How this browser opens a private window from the command line.
    pub fn private_flag(&self) -> PrivateFlag {
        self.private_flag.unwrap_or(match self.engine {
            BrowserEngine::Chromium => PrivateFlag::Incognito,
            BrowserEngine::Firefox => PrivateFlag::PrivateWindow,
            BrowserEngine::WebKit => PrivateFlag::None,
        })
    }

    /// What this browser calls a private window ("Incognito", "InPrivate",
    /// "Private window"), or `None` when it cannot open one from the
    /// command line.
    pub fn private_label(&self) -> Option<&str> {
        let flag = self.private_flag();
        flag.label()?;
        self.private_label.as_deref().or(flag.label())
    }

    /// Whether the catalog has anything for `platform`.
    pub fn supports(&self, platform: Platform) -> bool {
        match platform {
            Platform::MacOs => self.macos.is_some(),
            Platform::Windows => self.windows.is_some(),
            Platform::Linux => self.linux.is_some(),
        }
    }

    /// The user-data directory templates for `env`'s platform.
    pub(crate) fn profile_templates(&self, platform: Platform) -> &[String] {
        match platform {
            Platform::MacOs => self.macos.as_ref().map(|m| &m.profiles[..]),
            Platform::Windows => self.windows.as_ref().map(|w| &w.profiles[..]),
            Platform::Linux => self.linux.as_ref().map(|l| &l.profiles[..]),
        }
        .unwrap_or_default()
    }

    /// The user-data directory on this machine: the first template whose
    /// directory holds the engine's profile index (`Local State` or
    /// `profiles.ini`), else the first template that expands.
    pub fn profile_dir(&self, env: &Env) -> Option<std::path::PathBuf> {
        let marker = match self.profile_flag() {
            ProfileFlag::Chromium => "Local State",
            ProfileFlag::Firefox => "profiles.ini",
            ProfileFlag::None => return None,
        };
        let dirs: Vec<_> = self
            .profile_templates(env.platform())
            .iter()
            .filter_map(|t| env.expand(t))
            .collect();
        dirs.iter()
            .find(|d| d.join(marker).is_file())
            .or(dirs.first())
            .cloned()
    }
}

/// `path` ends with the components `rel` (both Windows paths).
fn ends_with_component(path: &str, rel: &str) -> bool {
    let rel = normalize_windows(rel);
    path.strip_suffix(&rel)
        .is_some_and(|head| head.is_empty() || head.ends_with('/'))
}

/// Lowercase, forward slashes: Windows paths compare that way.
fn normalize_windows(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

/// `FirefoxURL-` matches `FirefoxURL-308046B0AF4A39CB`; `ChromeHTML`
/// matches itself and a per-user `ChromeHTML.<suffix>`.
pub(crate) fn prog_id_matches(pattern: &str, prog_id: &str) -> bool {
    let rest = prog_id
        .get(..pattern.len())
        .filter(|p| p.eq_ignore_ascii_case(pattern))
        .and_then(|_| prog_id.get(pattern.len()..));
    match rest {
        None => false,
        Some(rest) if pattern.ends_with('-') => !rest.is_empty(),
        Some(rest) => rest.is_empty() || (rest.len() > 1 && rest.starts_with('.')),
    }
}

#[cfg(test)]
mod tests;
