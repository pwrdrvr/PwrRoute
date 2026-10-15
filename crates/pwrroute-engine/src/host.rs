//! What the app provides to configs beyond Finicky's own utilities.
//!
//! The Go host added four native functions to the `finicky` global next to
//! the config API's utilities. The engine asks a [`Host`] for each, so the
//! app can answer from the OS and tests can answer with fixed values.

use serde::Serialize;

/// Modifier keys held when the link was opened, as `finicky.getModifierKeys()`
/// returns them. `fn` and `function` are the same key under both names
/// Finicky has used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModifierKeys {
    pub shift: bool,
    pub option: bool,
    pub command: bool,
    pub control: bool,
    pub caps_lock: bool,
    #[serde(rename = "fn")]
    pub fn_key: bool,
    #[serde(rename = "function")]
    pub function_key: bool,
}

/// `finicky.getSystemInfo()`. `platform` is a PwrRoute addition, so configs
/// can branch on macOS, Windows and Linux.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    pub name: String,
    pub localized_name: String,
    pub platform: String,
}

/// `finicky.getPowerInfo()`. Every field is `null` where the OS can't say.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerInfo {
    pub is_charging: Option<bool>,
    pub is_connected: Option<bool>,
    pub percentage: Option<f64>,
}

/// Where a config's `console` output goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// The app side of the `finicky` global. Every method has a portable default,
/// so a host only overrides what its platform can answer.
pub trait Host {
    fn modifier_keys(&self) -> ModifierKeys {
        ModifierKeys::default()
    }

    fn system_info(&self) -> SystemInfo {
        SystemInfo {
            name: String::new(),
            localized_name: String::new(),
            platform: std::env::consts::OS.to_owned(),
        }
    }

    fn power_info(&self) -> PowerInfo {
        PowerInfo::default()
    }

    /// `finicky.isAppRunning(identifier)`, where the identifier is a bundle
    /// id or an app name.
    fn is_app_running(&self, _identifier: &str) -> bool {
        false
    }

    /// A `console.*` call from the config. Messages may contain URLs, which
    /// are private: a host that writes them to disk must say so.
    fn log(&self, level: LogLevel, message: &str) {
        let _ = (level, message);
    }
}

/// A host with the portable defaults and no logging.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultHost;

impl Host for DefaultHost {}
