//! [`NativeHost`]: the `finicky.*` native functions answered from the OS.
//!
//! Each OS's calls live in one module (`macos.rs`, `windows.rs`,
//! `linux.rs`); the FFI, and the only `unsafe` in this crate, is confined to
//! the macOS and Windows modules. What each OS can answer:
//!
//! | | macOS | Windows | Linux |
//! | --- | --- | --- | --- |
//! | `getModifierKeys` | all keys (CoreGraphics) | shift, option (Alt), command (Windows key), control, caps lock; never fn | none: there is no display-independent API, and Wayland gives none |
//! | `getSystemInfo` | host and computer name | DNS host name | host name, and `PRETTY_HOSTNAME` from `/etc/machine-info` |
//! | `getPowerInfo` | internal battery (IOKit), as the Go host read it | battery and AC line | `/sys/class/power_supply` |
//! | `isAppRunning` | bundle id or app name (`NSWorkspace`) | process image name, with or without `.exe` | process name or executable name |
//!
//! Names are compared ignoring ASCII case on Windows and Linux, and exactly on
//! macOS, as the Go host compared them.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
use linux as os;
#[cfg(target_os = "macos")]
use macos as os;
#[cfg(windows)]
use windows as os;

use crate::{Host, LogLevel, ModifierKeys, PowerInfo, SystemInfo};

/// Where config `console` output goes.
type LogSink = Box<dyn Fn(LogLevel, &str)>;

/// The OS's answers to the `finicky.*` native functions. Config `console`
/// output goes to the optional log callback, which receives URLs: it must not
/// write them to disk unless the user asked for that.
#[derive(Default)]
pub struct NativeHost {
    log: Option<LogSink>,
}

impl std::fmt::Debug for NativeHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeHost")
            .field("log", &self.log.is_some())
            .finish()
    }
}

impl NativeHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sends config `console` output to `log`.
    pub fn with_log(log: impl Fn(LogLevel, &str) + 'static) -> Self {
        Self {
            log: Some(Box::new(log)),
        }
    }
}

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
impl Host for NativeHost {
    fn modifier_keys(&self) -> ModifierKeys {
        os::modifier_keys()
    }

    fn system_info(&self) -> SystemInfo {
        let (name, localized_name) = os::computer_names();
        SystemInfo {
            name,
            localized_name,
            platform: std::env::consts::OS.to_owned(),
        }
    }

    fn power_info(&self) -> PowerInfo {
        os::power_info()
    }

    fn is_app_running(&self, identifier: &str) -> bool {
        !identifier.is_empty() && os::is_app_running(identifier)
    }

    fn log(&self, level: LogLevel, message: &str) {
        if let Some(log) = &self.log {
            log(level, message);
        }
    }
}

/// Other platforms get the portable defaults.
#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
impl Host for NativeHost {
    fn log(&self, level: LogLevel, message: &str) {
        if let Some(log) = &self.log {
            log(level, message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real OS answers, checked for shape only: values depend on the
    /// machine running the tests.
    #[test]
    fn answers_from_the_os() {
        let host = NativeHost::new();
        let info = host.system_info();
        assert_eq!(info.platform, std::env::consts::OS);
        assert!(!info.name.is_empty(), "{info:?}");
        let _ = host.modifier_keys();
        let power = host.power_info();
        if let Some(p) = power.percentage {
            assert!((0.0..=100.0).contains(&p), "{power:?}");
        }
        assert!(!host.is_app_running(""));
        assert!(!host.is_app_running("com.pwrdrvr.no-such-app-running-here"));
    }

    #[test]
    fn finds_this_test_process() {
        let exe = std::env::current_exe().unwrap();
        let name = exe.file_stem().unwrap().to_string_lossy().into_owned();
        let host = NativeHost::new();
        // macOS matches app names and bundle ids, which a test binary lacks.
        if !cfg!(target_os = "macos") {
            assert!(host.is_app_running(&name), "{name}");
        }
    }
}
