//! Registering PwrRoute as a browser, and asking to become the default.
//!
//! Two different acts, kept apart on purpose:
//!
//! - [`register_browser`] / [`unregister_browser`] tell the OS PwrRoute *can*
//!   open web links, so it is listed among the browsers. Installers and the
//!   updater call them from install and uninstall hooks. They never change
//!   which browser is the default.
//! - [`set_default_browser`] asks for PwrRoute to *be* the default. Only an
//!   explicit user action calls it, and the OS has the last word: macOS
//!   shows its consent prompt, Windows opens Settings › Default apps (apps
//!   cannot set themselves default since Windows 8), and Linux asks
//!   `xdg-settings`.
//!
//! The Windows registry values and the Linux desktop entry are built by
//! plain functions ([`windows_registry_values`], [`desktop_entry`]) that the
//! platform code applies, so they are tested on every OS without touching
//! the machine.

use std::path::Path;

use anyhow::Result;

/// The macOS bundle identifier, from `packaging/macos/Info.plist.in`.
pub const BUNDLE_ID: &str = "com.pwrdrvr.pwrroute";
/// The Windows ProgID that `http` and `https` point at.
pub const PROG_ID: &str = "PwrRouteURL";
/// The Windows `StartMenuInternet` and `RegisteredApplications` name.
pub const REGISTERED_NAME: &str = "PwrRoute";
/// The Linux desktop file id, as in `packaging/linux/pwrroute.desktop`.
pub const DESKTOP_ID: &str = "pwrroute.desktop";

/// Which app handles web links now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DefaultStatus {
    /// PwrRoute handles both `http` and `https`.
    pub is_default: bool,
    /// The `http` handler as the OS names it: a bundle id, ProgID or desktop
    /// file id.
    pub http: Option<String>,
    pub https: Option<String>,
}

/// What [`set_default_browser`] set in motion. In every case the OS has the
/// final say; poll [`default_browser_status`] to see the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultRequest {
    /// macOS: the consent prompt is showing.
    Prompted,
    /// Windows: Settings › Default apps is open on PwrRoute.
    OpenedSettings,
    /// Linux: `xdg-settings` made PwrRoute the default.
    Set,
}

/// Makes the OS list PwrRoute as a browser. Idempotent; never changes the
/// default browser.
pub fn register_browser() -> Result<()> {
    imp::register()
}

/// Takes PwrRoute off the OS's list of browsers, for uninstall. If PwrRoute
/// was the default, the OS picks another (or asks) on the next link.
pub fn unregister_browser() -> Result<()> {
    imp::unregister()
}

/// Asks the OS to make PwrRoute the default browser. Call it only from an
/// explicit user action. Registers first, so the OS can find PwrRoute.
pub fn set_default_browser() -> Result<DefaultRequest> {
    imp::register()?;
    imp::set_default()
}

/// Which app handles `http` and `https` now. Reads only.
pub fn default_browser_status() -> Result<DefaultStatus> {
    imp::status()
}

/// One registry value under `HKEY_CURRENT_USER`. `name` is empty for the
/// key's default value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryValue {
    pub key: String,
    pub name: &'static str,
    pub data: String,
}

const CLIENT_KEY: &str = r"Software\Clients\StartMenuInternet\PwrRoute";
const PROG_ID_KEY: &str = r"Software\Classes\PwrRouteURL";
const REGISTERED_APPS_KEY: &str = r"Software\RegisteredApplications";

/// The per-user registry values that make Windows list PwrRoute as a
/// browser, for the executable at `exe`: a `StartMenuInternet` client with
/// its capabilities, a ProgID whose command line passes the link as `%1`,
/// and a `RegisteredApplications` entry pointing at the capabilities. See
/// Microsoft's "Registering Programs with Client Types".
pub fn windows_registry_values(exe: &str) -> Vec<RegistryValue> {
    let value = |key: &str, name: &'static str, data: &str| RegistryValue {
        key: key.to_owned(),
        name,
        data: data.to_owned(),
    };
    let capabilities = format!(r"{CLIENT_KEY}\Capabilities");
    let icon = format!("\"{exe}\",0");
    let open = format!("\"{exe}\"");
    let open_link = format!("\"{exe}\" \"%1\"");
    let description = "Every link to the right browser";
    vec![
        // The ProgID http and https point at.
        value(PROG_ID_KEY, "", "PwrRoute URL"),
        value(PROG_ID_KEY, "FriendlyTypeName", "PwrRoute URL"),
        value(PROG_ID_KEY, "URL Protocol", ""),
        value(&format!(r"{PROG_ID_KEY}\DefaultIcon"), "", &icon),
        value(
            &format!(r"{PROG_ID_KEY}\Application"),
            "ApplicationName",
            "PwrRoute",
        ),
        value(
            &format!(r"{PROG_ID_KEY}\Application"),
            "ApplicationDescription",
            description,
        ),
        value(
            &format!(r"{PROG_ID_KEY}\shell\open\command"),
            "",
            &open_link,
        ),
        // The browser client.
        value(CLIENT_KEY, "", "PwrRoute"),
        value(&format!(r"{CLIENT_KEY}\DefaultIcon"), "", &icon),
        value(&format!(r"{CLIENT_KEY}\shell\open\command"), "", &open),
        value(&capabilities, "ApplicationName", "PwrRoute"),
        value(&capabilities, "ApplicationDescription", description),
        value(&capabilities, "ApplicationIcon", &icon),
        value(
            &format!(r"{capabilities}\StartMenu"),
            "StartMenuInternet",
            REGISTERED_NAME,
        ),
        value(&format!(r"{capabilities}\URLAssociations"), "http", PROG_ID),
        value(
            &format!(r"{capabilities}\URLAssociations"),
            "https",
            PROG_ID,
        ),
        value(REGISTERED_APPS_KEY, "PwrRoute", &capabilities),
    ]
}

/// Marks a desktop file [`register_browser`] wrote, so [`unregister_browser`]
/// removes only its own.
const DESKTOP_MARKER: &str = "X-PwrRoute-Registered=true";

/// The per-user desktop entry that makes Linux list PwrRoute as a browser,
/// for the executable at `exe`.
pub fn desktop_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=PwrRoute\n\
         Comment=Every link to the right browser\n\
         Exec={} %u\n\
         Terminal=false\n\
         Categories=Network;WebBrowser;\n\
         MimeType=x-scheme-handler/http;x-scheme-handler/https;\n\
         StartupWMClass=PwrRoute\n\
         {DESKTOP_MARKER}\n",
        exec_arg(&exe.to_string_lossy())
    )
}

/// Quotes a path for `Exec=`, per the Desktop Entry Specification: inside
/// double quotes, `"`, `` ` ``, `$` and `\` take a backslash; then, because
/// `Exec` is a string value, every backslash is doubled again; and `%` is
/// written `%%` so it is not a field code.
fn exec_arg(path: &str) -> String {
    let mut quoted = String::from("\"");
    for c in path.chars() {
        match c {
            '"' | '`' | '$' => {
                quoted.push_str(r"\\");
                quoted.push(c);
            }
            '\\' => quoted.push_str(r"\\\\"),
            '%' => quoted.push_str("%%"),
            _ => quoted.push(c),
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(target_os = "macos")]
mod imp {
    pub use crate::macos::{register, set_default, status, unregister};
}

#[cfg(windows)]
mod imp {
    use std::process::Command;

    use anyhow::{Context as _, Result};
    use windows_registry::CURRENT_USER;

    use super::{
        CLIENT_KEY, DefaultRequest, DefaultStatus, PROG_ID, PROG_ID_KEY, REGISTERED_APPS_KEY,
        REGISTERED_NAME, windows_registry_values,
    };

    pub fn register() -> Result<()> {
        let exe = std::env::current_exe().context("finding pwrroute.exe")?;
        for value in windows_registry_values(&exe.to_string_lossy()) {
            CURRENT_USER
                .create(&value.key)
                .and_then(|key| key.set_string(value.name, &value.data))
                .with_context(|| format!(r"writing HKCU\{}", value.key))?;
        }
        Ok(())
    }

    pub fn unregister() -> Result<()> {
        for key in [CLIENT_KEY, PROG_ID_KEY] {
            // Already gone is fine.
            let _ = CURRENT_USER.remove_tree(key);
        }
        if let Ok(apps) = CURRENT_USER.create(REGISTERED_APPS_KEY) {
            let _ = apps.remove_value(REGISTERED_NAME);
        }
        Ok(())
    }

    pub fn set_default() -> Result<DefaultRequest> {
        // Since Windows 8 only the user can pick the default, in Settings.
        // `registeredAppUser` opens the page on PwrRoute (Windows 11).
        Command::new("rundll32.exe")
            .args([
                "url.dll,FileProtocolHandler",
                &format!("ms-settings:defaultapps?registeredAppUser={REGISTERED_NAME}"),
            ])
            .spawn()
            .context("opening Settings › Default apps")?;
        Ok(DefaultRequest::OpenedSettings)
    }

    pub fn status() -> Result<DefaultStatus> {
        let choice = |scheme: &str| {
            CURRENT_USER
                .open(format!(
                    r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\{scheme}\UserChoice"
                ))
                .and_then(|key| key.get_string("ProgId"))
                .ok()
        };
        let (http, https) = (choice("http"), choice("https"));
        Ok(DefaultStatus {
            is_default: http.as_deref() == Some(PROG_ID) && https.as_deref() == Some(PROG_ID),
            http,
            https,
        })
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use std::{fs, path::PathBuf, process::Command};

    use anyhow::{Context as _, Result, bail};

    use super::{DESKTOP_ID, DESKTOP_MARKER, DefaultRequest, DefaultStatus, desktop_entry};

    fn applications_dir() -> Result<PathBuf> {
        Ok(dirs::data_dir()
            .context("finding $XDG_DATA_HOME")?
            .join("applications"))
    }

    /// Runs a desktop helper and returns its trimmed stdout.
    fn run(program: &str, args: &[&str]) -> Result<String> {
        let out = Command::new(program)
            .args(args)
            .output()
            .with_context(|| format!("running {program}"))?;
        if !out.status.success() {
            bail!(
                "{program} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    pub fn register() -> Result<()> {
        let exe = std::env::current_exe().context("finding the pwrroute binary")?;
        let dir = applications_dir()?;
        fs::create_dir_all(&dir)?;
        let path = dir.join(DESKTOP_ID);
        fs::write(&path, desktop_entry(&exe))
            .with_context(|| format!("writing {}", path.display()))?;
        // Refreshes the MIME cache where the helper exists; optional.
        let _ = run("update-desktop-database", &["-q", &dir.to_string_lossy()]);
        Ok(())
    }

    pub fn unregister() -> Result<()> {
        let dir = applications_dir()?;
        let path = dir.join(DESKTOP_ID);
        // Only the file register() wrote; a packaged or hand-made one stays.
        if fs::read_to_string(&path).is_ok_and(|s| s.lines().any(|l| l == DESKTOP_MARKER)) {
            fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
            let _ = run("update-desktop-database", &["-q", &dir.to_string_lossy()]);
        }
        Ok(())
    }

    pub fn set_default() -> Result<DefaultRequest> {
        if run("xdg-settings", &["set", "default-web-browser", DESKTOP_ID]).is_err() {
            run(
                "xdg-mime",
                &[
                    "default",
                    DESKTOP_ID,
                    "x-scheme-handler/http",
                    "x-scheme-handler/https",
                ],
            )?;
        }
        Ok(DefaultRequest::Set)
    }

    pub fn status() -> Result<DefaultStatus> {
        let query = |scheme: &str| {
            run(
                "xdg-mime",
                &["query", "default", &format!("x-scheme-handler/{scheme}")],
            )
            .ok()
            .filter(|s| !s.is_empty())
        };
        let (http, https) = (query("http"), query("https"));
        Ok(DefaultStatus {
            is_default: http.as_deref() == Some(DESKTOP_ID) && https.as_deref() == Some(DESKTOP_ID),
            http,
            https,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_points_http_and_https_at_the_prog_id() {
        let values = windows_registry_values(r"C:\Users\me\AppData\Local\PwrRoute\pwrroute.exe");
        let find = |key: &str, name: &str| {
            values
                .iter()
                .find(|v| v.key == key && v.name == name)
                .map(|v| v.data.as_str())
                .unwrap_or_else(|| panic!(r"no HKCU\{key} {name}"))
        };
        let caps = r"Software\Clients\StartMenuInternet\PwrRoute\Capabilities";
        assert_eq!(find(&format!(r"{caps}\URLAssociations"), "http"), PROG_ID);
        assert_eq!(find(&format!(r"{caps}\URLAssociations"), "https"), PROG_ID);
        assert_eq!(find(r"Software\RegisteredApplications", "PwrRoute"), caps);
        assert_eq!(
            find(r"Software\Classes\PwrRouteURL\shell\open\command", ""),
            r#""C:\Users\me\AppData\Local\PwrRoute\pwrroute.exe" "%1""#
        );
        // Everything stays under the user's hive and PwrRoute's own keys.
        for v in &values {
            assert!(
                v.key
                    .starts_with(r"Software\Clients\StartMenuInternet\PwrRoute")
                    || v.key.starts_with(r"Software\Classes\PwrRouteURL")
                    || v.key == r"Software\RegisteredApplications",
                "unexpected key {}",
                v.key
            );
        }
    }

    #[test]
    fn the_desktop_entry_handles_web_links() {
        let entry = desktop_entry(Path::new("/opt/PwrRoute/pwrroute"));
        assert!(entry.starts_with("[Desktop Entry]\n"));
        assert!(entry.contains("\nExec=\"/opt/PwrRoute/pwrroute\" %u\n"));
        assert!(entry.contains("\nMimeType=x-scheme-handler/http;x-scheme-handler/https;\n"));
        assert!(entry.contains("\nCategories=Network;WebBrowser;\n"));
        assert!(entry.contains(&format!("\n{DESKTOP_MARKER}\n")));
    }

    #[test]
    fn exec_paths_are_escaped() {
        assert_eq!(exec_arg("/a b/pwrroute"), r#""/a b/pwrroute""#);
        assert_eq!(exec_arg("/100%/x"), r#""/100%%/x""#);
        assert_eq!(exec_arg("/$HOME/x"), r#""/\\$HOME/x""#);
        assert_eq!(exec_arg(r#"/q"t/x"#), r#""/q\\"t/x""#);
        assert_eq!(exec_arg(r"/b\s/x"), r#""/b\\\\s/x""#);
    }

    #[test]
    fn the_names_match_the_packaging() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let plist = std::fs::read_to_string(root.join("packaging/macos/Info.plist.in")).unwrap();
        assert!(plist.contains(&format!("<string>{BUNDLE_ID}</string>")));
        assert!(root.join("packaging/linux").join(DESKTOP_ID).exists());
    }
}
