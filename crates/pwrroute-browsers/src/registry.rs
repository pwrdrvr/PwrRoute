//! The Windows registry's browser registrations. Reading it is native-only;
//! parsing what it holds is not, so that part is tested on every OS.
//!
//! - `SOFTWARE\Clients\StartMenuInternet\<key>`: every registered browser,
//!   with its display name and `shell\open\command`.
//!   <https://learn.microsoft.com/windows/win32/shell/start-menu-reg>
//! - `SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\<exe>`: where an
//!   executable lives when it is not on `PATH`.
//!   <https://learn.microsoft.com/windows/win32/shell/app-registration>

use std::path::PathBuf;

use crate::env::Env;

/// A `StartMenuInternet` registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Registration {
    pub name: String,
    pub exe: PathBuf,
}

/// The executable at the start of a registry command line:
/// `"C:\Program Files\X\x.exe" --flag "%1"` or an unquoted path.
pub(crate) fn command_program(command: &str) -> Option<PathBuf> {
    let command = command.trim();
    let program = match command.strip_prefix('"') {
        Some(rest) => rest.split_once('"')?.0,
        // Unquoted: up to `.exe`, which may be followed by arguments.
        None => match command.to_ascii_lowercase().find(".exe") {
            Some(i) => &command[..i + 4],
            None => command.split(' ').next()?,
        },
    };
    (!program.is_empty()).then(|| PathBuf::from(program))
}

/// Every `StartMenuInternet` browser, per-user registrations first.
pub(crate) fn start_menu_internet(env: &Env) -> Vec<Registration> {
    if !env.registry() {
        return Vec::new();
    }
    native::start_menu_internet()
        .into_iter()
        .filter_map(|(name, command)| {
            Some(Registration {
                name,
                exe: command_program(&expand_vars(env, &command))?,
            })
        })
        .collect()
}

/// The `App Paths` entry for `exe` (`chrome.exe`), when that file exists.
pub(crate) fn app_path(env: &Env, exe: &str) -> Option<PathBuf> {
    if !env.registry() {
        return None;
    }
    let path = command_program(&expand_vars(env, &native::app_path(exe)?))?;
    path.is_file().then_some(path)
}

/// `REG_EXPAND_SZ` values: `%ProgramFiles%\x` with the variable expanded.
/// Unknown variables are left as written.
pub(crate) fn expand_vars(env: &Env, value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match env.var(name) {
                    Some(v) if !name.is_empty() => out.push_str(v),
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(windows)]
mod native {
    use windows_registry::{CURRENT_USER, Key, LOCAL_MACHINE};

    const ROOTS: [&str; 2] = [
        r"SOFTWARE\Clients\StartMenuInternet",
        r"SOFTWARE\WOW6432Node\Clients\StartMenuInternet",
    ];

    /// `(display name, shell\open\command)` for every registration.
    pub fn start_menu_internet() -> Vec<(String, String)> {
        let mut out = Vec::new();
        for hive in [CURRENT_USER, LOCAL_MACHINE] {
            for root in ROOTS {
                let Ok(clients) = hive.open(root) else {
                    continue;
                };
                let Ok(keys) = clients.keys() else { continue };
                out.extend(keys.filter_map(|key| registration(&clients, &key)));
            }
        }
        out
    }

    fn registration(clients: &Key, key: &str) -> Option<(String, String)> {
        let client = clients.open(key).ok()?;
        let name = client.get_string("").unwrap_or_else(|_| key.to_owned());
        let command = client
            .open(r"shell\open\command")
            .ok()?
            .get_string("")
            .ok()?;
        Some((name, command))
    }

    pub fn app_path(exe: &str) -> Option<String> {
        let key = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{exe}");
        [CURRENT_USER, LOCAL_MACHINE]
            .into_iter()
            .find_map(|hive| hive.open(&key).ok()?.get_string("").ok())
    }
}

#[cfg(not(windows))]
mod native {
    pub fn start_menu_internet() -> Vec<(String, String)> {
        Vec::new()
    }

    pub fn app_path(_exe: &str) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_command_lines() {
        let p = |s| command_program(s).map(|p| p.to_string_lossy().into_owned());
        assert_eq!(
            p(r#""C:\Program Files\Google\Chrome\Application\chrome.exe" --single-argument %1"#)
                .as_deref(),
            Some(r"C:\Program Files\Google\Chrome\Application\chrome.exe")
        );
        assert_eq!(
            p(r"C:\Program Files\Mozilla Firefox\firefox.exe -osint -url %1").as_deref(),
            Some(r"C:\Program Files\Mozilla Firefox\firefox.exe")
        );
        assert_eq!(p(r#""unterminated"#), None);
        assert_eq!(p(""), None);
    }

    #[test]
    fn expands_percent_variables() {
        let mut env = Env::fixture(crate::Platform::Windows, "/r");
        env.set_var("ProgramFiles", r"C:\Program Files".into());
        assert_eq!(
            expand_vars(&env, r"%PROGRAMFILES%\x.exe %1 %UNKNOWN% 100%"),
            r"C:\Program Files\x.exe %1 %UNKNOWN% 100%"
        );
    }
}
