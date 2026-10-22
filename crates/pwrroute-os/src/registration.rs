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
//!   shows its consent prompt (none when another copy of PwrRoute already
//!   holds the default: this copy takes it over), Windows opens Settings › Default apps (apps
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
/// The macOS link helper's bundle identifier, from
/// `packaging/macos/LinkHelper.Info.plist.in`: the windowless app that
/// [`set_default_handler`]`(`[`Handler::LinkHelper`]`)` makes the `http`
/// and `https` handler, so a link never activates the app with the windows.
/// Opt-in: [`set_default_browser`] asks for the app.
pub const LINK_HELPER_ID: &str = "com.pwrdrvr.pwrroute.links";
/// Where the link helper sits inside `PwrRoute.app`, as `cargo xtask bundle`
/// nests it.
pub const LINK_HELPER_PATH: &str = "Contents/Helpers/PwrRoute.app";
/// The argument the app starts the link helper with ([`start_link_helper`]):
/// a launch that is neither for a link nor a request for the window.
pub const LINK_HELPER_STARTED_BY_APP: &str = "--started-by-app";
/// The Windows ProgID that `http` and `https` point at.
pub const PROG_ID: &str = "PwrRouteURL";
/// The Windows `StartMenuInternet` and `RegisteredApplications` name.
pub const REGISTERED_NAME: &str = "PwrRoute";
/// The Linux desktop file id, as in `packaging/linux/pwrroute.desktop`.
pub const DESKTOP_ID: &str = "pwrroute.desktop";

/// Which app handles web links now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DefaultStatus {
    /// PwrRoute handles both `http` and `https`. A bundled macOS app also
    /// checks that both select its path, rather than another installed copy.
    pub is_default: bool,
    /// macOS: PwrRoute is the default through its link helper
    /// ([`LINK_HELPER_ID`]) on both schemes, so a link never activates the
    /// app. `false` while either scheme names the app itself, and always on
    /// Windows and Linux.
    pub link_helper: bool,
    /// The `http` handler as the OS names it: a bundle id, ProgID or desktop
    /// file id.
    pub http: Option<String>,
    pub https: Option<String>,
    /// macOS: another copy of PwrRoute (same bundle id, another path, such
    /// as an older dev build) that LaunchServices hands links to instead of
    /// the running one. Its bundle's path.
    pub other_copy: Option<String>,
}

/// What [`set_default_browser`] set in motion. Poll
/// [`default_browser_status`] to see the result: the OS has the final say,
/// except on a macOS takeover from another PwrRoute copy, which needs no
/// consent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultRequest {
    /// macOS: the consent prompt is showing.
    Prompted,
    /// Windows: Settings › Default apps is open on PwrRoute.
    OpenedSettings,
    /// PwrRoute is the default with no prompt to answer: on Linux
    /// `xdg-settings` set it; on macOS this copy already handled both
    /// schemes.
    Set,
    /// macOS: another copy of PwrRoute held the default. The other copies
    /// were taken out of LaunchServices, so links reach this one with no
    /// prompt to answer.
    TookOverFromCopy,
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

/// Which of PwrRoute's bundles a default-browser request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handler {
    /// The app itself: what "Make default browser" asks for everywhere.
    App,
    /// macOS only: the windowless link helper nested in the app
    /// ([`LINK_HELPER_PATH`]), which forwards each link to the running app
    /// without activating it. Settings › Experimental asks for it.
    LinkHelper,
}

/// Asks the OS to make PwrRoute the default browser. Call it only from an
/// explicit user action. Registers first, so the OS can find PwrRoute.
pub fn set_default_browser() -> Result<DefaultRequest> {
    set_default_handler(Handler::App)
}

/// [`set_default_browser`] for one of PwrRoute's bundles: the app, or on
/// macOS its link helper (registered with LaunchServices first; refused
/// when the bundle has none, [`link_helper_bundled`]). Asking for the app
/// while the helper is the default hands links back to the app. Call it
/// only from an explicit user action.
pub fn set_default_handler(handler: Handler) -> Result<DefaultRequest> {
    imp::register()?;
    #[cfg(target_os = "macos")]
    {
        imp::set_default(handler)
    }
    #[cfg(not(target_os = "macos"))]
    {
        if handler == Handler::LinkHelper {
            anyhow::bail!("the link helper is macOS only");
        }
        imp::set_default()
    }
}

/// Whether the running copy carries the link helper: a bundled macOS app
/// built with it. Always `false` for a bare dev binary and off macOS.
pub fn link_helper_bundled() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::macos::link_helper_bundled()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Which app handles `http` and `https` now. Reads only.
pub fn default_browser_status() -> Result<DefaultStatus> {
    imp::status()
}

/// The name an installed app goes by, from the id the OS reports for a
/// handler: on macOS the bundle's display name (`CFBundleDisplayName`, else
/// `CFBundleName`, else the bundle's file name) for a bundle id. `None`
/// when no app has that id, and on Windows and Linux, where the caller
/// keeps the id. Reads only.
pub fn app_display_name(id: &str) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        crate::macos::app_display_name(id)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = id;
        None
    }
}

/// Whether a handler id the OS reports is PwrRoute: the link helper's or
/// the app's bundle id on macOS, the ProgID on Windows, the desktop file id
/// on Linux.
pub fn is_pwrroute_handler(id: &str) -> bool {
    [LINK_HELPER_ID, BUNDLE_ID, PROG_ID, DESKTOP_ID]
        .iter()
        .any(|ours| id.eq_ignore_ascii_case(ours))
}

/// macOS's answer for the `http` and `https` handlers. Either bundle id is
/// PwrRoute: the app's own, which "Make default browser" asks for, and the
/// link helper's, which Settings › Experimental asks for. Each must select
/// this copy's bundle, or it is `other_copy`'s default. Both on the helper's
/// id is `link_helper`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn macos_status(http: SchemeHandler, https: SchemeHandler) -> DefaultStatus {
    let is =
        |h: &SchemeHandler, id: &str| h.id.as_deref().is_some_and(|h| h.eq_ignore_ascii_case(id));
    let ours = |h: &SchemeHandler| is(h, BUNDLE_ID) || is(h, LINK_HELPER_ID);
    // One of PwrRoute's ids at another copy's path is that copy's default,
    // not this one's.
    let elsewhere = |h: &SchemeHandler| ours(h) && !h.this_copy;
    let other_copy = [&http, &https]
        .into_iter()
        .filter(|h| elsewhere(h))
        .find_map(|h| h.path.clone());
    let is_default = ours(&http) && ours(&https) && !elsewhere(&http) && !elsewhere(&https);
    DefaultStatus {
        is_default,
        link_helper: is_default && is(&http, LINK_HELPER_ID) && is(&https, LINK_HELPER_ID),
        http: http.id,
        https: https.id,
        other_copy,
    }
}

/// One scheme's handler as macOS reports it, for [`macos_status`].
#[derive(Debug, Clone, Default)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) struct SchemeHandler {
    /// Its bundle id.
    pub id: Option<String>,
    /// The bundle LaunchServices opens is this copy's: the running app, or
    /// its nested helper for the helper's id. Always `true` for a bare dev
    /// binary, which has no bundle to compare.
    pub this_copy: bool,
    /// That bundle's path.
    pub path: Option<String>,
}

/// Opens the OS's own default-apps page, where the user can pick or change
/// the default browser by hand. Changes nothing itself.
pub fn open_default_apps_settings() -> Result<()> {
    imp::open_settings()
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

/// Writes a path for `Exec=`, per the Desktop Entry Specification. `%` is
/// written `%%` so it is not a field code. A path with a reserved character
/// is quoted: inside double quotes, `"`, `` ` ``, `$` and `\` take a
/// backslash; then, because `Exec` is a string value, every backslash is
/// doubled again.
///
/// Quoted only when the spec requires it: xdg-utils 1.1.3's generic
/// `xdg-open` (Ubuntu's, used without a desktop environment) looks the first
/// word of `Exec` up with `which`, quotes and all, and runs nothing for a
/// quoted path.
fn exec_arg(path: &str) -> String {
    const RESERVED: &[char] = &[
        ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(',
        ')', '`',
    ];
    if !path.contains(RESERVED) {
        return path.replace('%', "%%");
    }
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
    pub use crate::macos::{open_settings, register, set_default, status, unregister};
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
        // Open, never create: uninstall must not leave a key behind.
        if let Ok(apps) = CURRENT_USER.options().write().open(REGISTERED_APPS_KEY) {
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

    pub fn open_settings() -> Result<()> {
        Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", "ms-settings:defaultapps"])
            .spawn()
            .context("opening Settings › Default apps")?;
        Ok(())
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
            link_helper: false,
            http,
            https,
            other_copy: None,
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

    /// The desktop's default-applications page, by the settings apps that
    /// have one: GNOME, KDE, Xfce.
    pub fn open_settings() -> Result<()> {
        let apps: [(&str, &[&str]); 3] = [
            ("gnome-control-center", &["default-apps"]),
            ("systemsettings", &["kcm_componentchooser"]),
            ("xfce4-mime-settings", &[]),
        ];
        for (program, args) in apps {
            if Command::new(program).args(args).spawn().is_ok() {
                return Ok(());
            }
        }
        bail!("no default-applications settings app found (GNOME, KDE or Xfce)")
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
            link_helper: false,
            http,
            https,
            other_copy: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Finder is on every Mac; no app has the made-up id. Reads only.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_installed_app_is_named_by_its_bundle() {
        assert_eq!(
            app_display_name("com.apple.finder").as_deref(),
            Some("Finder")
        );
        assert_eq!(app_display_name("com.example.no-such-app"), None);
    }

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
        assert!(entry.contains("\nExec=/opt/PwrRoute/pwrroute %u\n"));
        assert!(entry.contains("\nMimeType=x-scheme-handler/http;x-scheme-handler/https;\n"));
        assert!(entry.contains("\nCategories=Network;WebBrowser;\n"));
        assert!(entry.contains(&format!("\n{DESKTOP_MARKER}\n")));
    }

    #[test]
    fn exec_paths_are_escaped() {
        assert_eq!(exec_arg("/a b/pwrroute"), r#""/a b/pwrroute""#);
        assert_eq!(exec_arg("/opt/PwrRoute/pwrroute"), "/opt/PwrRoute/pwrroute");
        assert_eq!(exec_arg("/100%/x"), "/100%%/x");
        assert_eq!(exec_arg("/100% off/x"), r#""/100%% off/x""#);
        assert_eq!(exec_arg("/$HOME/x"), r#""/\\$HOME/x""#);
        assert_eq!(exec_arg(r#"/q"t/x"#), r#""/q\\"t/x""#);
        assert_eq!(exec_arg(r"/b\s/x"), r#""/b\\\\s/x""#);
    }

    #[test]
    fn the_names_match_the_packaging() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let plist = std::fs::read_to_string(root.join("packaging/macos/Info.plist.in")).unwrap();
        assert!(plist.contains(&format!("<string>{BUNDLE_ID}</string>")));
        // macOS lists only apps that claim HTML documents as web browsers,
        // in System Settings and in its consent prompt.
        for claim in ["http", "https", "public.html", "public.xhtml"] {
            assert!(
                plist.contains(&format!("<string>{claim}</string>")),
                "Info.plist.in must claim {claim}"
            );
        }
        let helper =
            std::fs::read_to_string(root.join("packaging/macos/LinkHelper.Info.plist.in")).unwrap();
        assert!(helper.contains(&format!("<string>{LINK_HELPER_ID}</string>")));
        // xtask nests the helper where the app looks for it.
        let xtask = std::fs::read_to_string(root.join("xtask/src/macos.rs")).unwrap();
        assert!(xtask.contains(&format!("\"{LINK_HELPER_PATH}\"")));
        assert!(root.join("packaging/linux").join(DESKTOP_ID).exists());
    }

    /// A handler at this copy's path.
    fn here(id: Option<&str>) -> SchemeHandler {
        SchemeHandler {
            id: id.map(str::to_owned),
            this_copy: true,
            path: None,
        }
    }

    fn mac(http: Option<&str>, https: Option<&str>) -> DefaultStatus {
        macos_status(here(http), here(https))
    }

    #[test]
    fn the_app_is_the_default_without_the_helper() {
        let status = mac(Some(BUNDLE_ID), Some(BUNDLE_ID));
        assert!(status.is_default && !status.link_helper);
    }

    #[test]
    fn the_link_helper_on_both_schemes_is_the_default_through_it() {
        let status = mac(Some(LINK_HELPER_ID), Some(LINK_HELPER_ID));
        assert!(status.is_default && status.link_helper);
        assert_eq!(status.http.as_deref(), Some(LINK_HELPER_ID));
        // LaunchServices may report a different case.
        let upper = LINK_HELPER_ID.to_uppercase();
        assert!(mac(Some(&upper), Some(&upper)).link_helper);
        // Half moved: still PwrRoute, but links on one scheme still reach
        // the app, so the helper is not on.
        let mixed = mac(Some(LINK_HELPER_ID), Some(BUNDLE_ID));
        assert!(mixed.is_default && !mixed.link_helper);
    }

    #[test]
    fn another_browser_on_either_scheme_is_not_the_default() {
        for status in [
            mac(Some("com.google.Chrome"), Some("com.google.Chrome")),
            mac(Some(LINK_HELPER_ID), Some("com.apple.Safari")),
            mac(Some("com.apple.Safari"), Some(BUNDLE_ID)),
            mac(None, Some(LINK_HELPER_ID)),
            mac(None, None),
            mac(Some("com.pwrdrvr.pwrroute.other"), Some(LINK_HELPER_ID)),
        ] {
            assert!(!status.is_default && !status.link_helper, "{status:?}");
            assert_eq!(status.other_copy, None);
        }
    }

    #[test]
    fn either_id_at_another_copys_path_is_that_copys_default() {
        let installed = |id: &str| SchemeHandler {
            id: Some(id.to_owned()),
            this_copy: false,
            path: Some("/Applications/PwrRoute.app".to_owned()),
        };
        for (http, https) in [
            (installed(BUNDLE_ID), installed(BUNDLE_ID)),
            (here(Some(BUNDLE_ID)), installed(BUNDLE_ID)),
            (installed(LINK_HELPER_ID), here(Some(LINK_HELPER_ID))),
        ] {
            let status = macos_status(http, https);
            assert!(!status.is_default && !status.link_helper, "{status:?}");
            assert_eq!(
                status.other_copy.as_deref(),
                Some("/Applications/PwrRoute.app")
            );
        }
        // Another app's path is not another copy of PwrRoute.
        let chrome = SchemeHandler {
            id: Some("com.google.Chrome".to_owned()),
            this_copy: false,
            path: Some("/Applications/Google Chrome.app".to_owned()),
        };
        assert_eq!(macos_status(chrome.clone(), chrome).other_copy, None);
    }

    #[test]
    fn every_id_of_ours_is_a_pwrroute_handler() {
        for id in [
            LINK_HELPER_ID,
            BUNDLE_ID,
            PROG_ID,
            DESKTOP_ID,
            "COM.PWRDRVR.PWRROUTE",
        ] {
            assert!(is_pwrroute_handler(id), "{id}");
        }
        for id in ["com.google.Chrome", "com.pwrdrvr.pwrfinicky", ""] {
            assert!(!is_pwrroute_handler(id), "{id}");
        }
    }
}
