//! What the app provides to configs beyond Finicky's own utilities.
//!
//! The Go host added four native functions to the `finicky` global next to
//! the config API's utilities. The engine asks a [`Host`] for each, so the
//! app can answer from the OS and tests can answer with fixed values.

use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use serde::{Deserialize, Serialize};

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

/// Where a config's `console` output goes. `console.log` and
/// `console.info` are both [`LogLevel::Info`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// The `console` method a config calls for this level, as a label:
    /// `debug`, `log`, `warn` or `error`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "log",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// One line of a config's `console` output. The message is whatever the
/// config printed, which is often a URL: keep lines in memory and never
/// write them to disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsoleLine {
    pub level: LogLevel,
    pub message: String,
}

/// The last [`ConsoleLog::CAPACITY`] lines of config `console` output, in
/// memory and shared by clone: a host pushes into it as the config prints,
/// and whoever shows the output takes the lines. The engine keeps one for
/// each run's [`Trace`](crate::Trace); a caller that also wants a config's
/// load-time output gives the engine a [`NativeHost`](crate::NativeHost)
/// built with [`ConsoleLog::native_host`]. Nothing here writes to disk.
#[derive(Debug, Clone, Default)]
pub struct ConsoleLog {
    lines: Rc<RefCell<VecDeque<ConsoleLine>>>,
}

impl ConsoleLog {
    /// How many lines are kept: a config that prints in a loop keeps the
    /// newest and loses the oldest.
    pub const CAPACITY: usize = 100;

    pub fn new() -> Self {
        Self::default()
    }

    /// Records a line, dropping the oldest when the log is full.
    pub fn push(&self, level: LogLevel, message: &str) {
        let mut lines = self.lines.borrow_mut();
        if lines.len() == Self::CAPACITY {
            lines.pop_front();
        }
        lines.push_back(ConsoleLine {
            level,
            message: message.to_owned(),
        });
    }

    /// The lines so far, oldest first, leaving the log empty.
    pub fn take(&self) -> Vec<ConsoleLine> {
        self.lines.borrow_mut().drain(..).collect()
    }

    /// Forgets the lines so far.
    pub fn clear(&self) {
        self.lines.borrow_mut().clear();
    }

    pub fn is_empty(&self) -> bool {
        self.lines.borrow().is_empty()
    }

    /// A host that answers from the OS and sends the config's `console`
    /// output here.
    pub fn native_host(&self) -> crate::NativeHost {
        let log = self.clone();
        crate::NativeHost::with_log(move |level, message| log.push(level, message))
    }
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

/// A host with the portable defaults and no logging: for loads that only
/// count or describe a config's rules. A host that resolves links should
/// be a [`NativeHost`](crate::NativeHost), or configs see no modifier keys
/// and no running apps.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultHost;

impl Host for DefaultHost {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_keeps_the_newest_lines() {
        let log = ConsoleLog::new();
        assert!(log.is_empty());
        for i in 0..(ConsoleLog::CAPACITY + 5) {
            log.push(LogLevel::Info, &format!("line {i}"));
        }
        let lines = log.take();
        assert_eq!(lines.len(), ConsoleLog::CAPACITY);
        assert_eq!(lines[0].message, "line 5");
        assert_eq!(
            lines[ConsoleLog::CAPACITY - 1].message,
            format!("line {}", ConsoleLog::CAPACITY + 4)
        );
        assert!(log.is_empty(), "take drains the log");

        let other = log.clone();
        other.push(LogLevel::Warn, "shared");
        assert_eq!(
            log.take(),
            [ConsoleLine {
                level: LogLevel::Warn,
                message: "shared".into()
            }]
        );
    }

    #[test]
    fn levels_serialize_as_console_names() {
        assert_eq!(serde_json::to_string(&LogLevel::Warn).unwrap(), "\"warn\"");
        assert_eq!(LogLevel::Info.name(), "log");
    }
}
