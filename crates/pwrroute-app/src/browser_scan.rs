//! One scan of this machine's browsers, shared by everything that lists
//! them: Activity's Add a rule sheet (`PwrRoute::browsers`) and Settings ›
//! Browsers. The scan runs once, the first time something asks, and again
//! only on Settings' Rescan; a launch never scans twice.
//!
//! What the scan reads is the app's name, bundle id, location and profile
//! names, from `pwrroute_browsers`, and whether the OS kept a profile list
//! from it (`DetectedApp::profiles_blocked`). Nothing here holds a URL.

use std::path::PathBuf;

use gpui_kit::{App, BorrowAppContext as _, Global};
use pwrroute_browsers::{Env, detect_installed, read_profiles};

use crate::activity_actions::BrowserChoice;
use crate::app::PwrRoute;
use crate::settings_store;

/// One app the scan found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedApp {
    /// The name to show, and to write in a rule on macOS.
    pub name: String,
    /// The macOS bundle id, when known.
    pub bundle_id: Option<String>,
    /// The `.app` bundle, the `.exe`, or the `.desktop` file.
    pub location: PathBuf,
    pub profiles: Vec<String>,
    /// Each profile's directory, by `profiles`' index: Chromium's
    /// `Profile 1`, Firefox's profile path. A rule may name a profile by it.
    pub directories: Vec<String>,
    /// The browser's profile list is there, but the OS would not let
    /// PwrRoute read it (macOS 27's Files & Folders), so `profiles` is
    /// empty for want of access, not of profiles.
    pub profiles_blocked: bool,
}

impl DetectedApp {
    /// What Settings and `settings.json` call the app: the bundle id, or
    /// the name where the platform has none.
    pub fn id(&self) -> &str {
        self.bundle_id.as_deref().unwrap_or(&self.name)
    }

    /// Whether a rule's `browser` value names this app: its bundle id, its
    /// name or its location, ignoring case, as the launcher matches them.
    pub fn is_named(&self, value: &str) -> bool {
        let value = value.trim();
        if value.is_empty() {
            return false;
        }
        value.eq_ignore_ascii_case(&self.name)
            || self
                .bundle_id
                .as_deref()
                .is_some_and(|id| id.eq_ignore_ascii_case(value))
            || self
                .location
                .to_str()
                .is_some_and(|path| path.eq_ignore_ascii_case(value))
    }
}

/// Reads `env`'s machine: the installed browsers and their profiles.
pub fn scan(env: &Env) -> Vec<DetectedApp> {
    detect_installed(env)
        .into_iter()
        .map(|installed| {
            let read = installed
                .browser
                .map_or(Ok(Vec::new()), |info| read_profiles(info, env));
            let profiles_blocked = read.is_err();
            let profiles = read.unwrap_or_default();
            DetectedApp {
                profiles_blocked,
                directories: profiles.iter().map(|p| p.directory.clone()).collect(),
                profiles: profiles.into_iter().map(|p| p.name).collect(),
                name: installed.name,
                bundle_id: installed.bundle_id,
                location: installed.location,
            }
        })
        .collect()
}

/// The scan, once it has run, and the machine it reads.
struct ScanGlobal {
    env: Env,
    apps: Option<Vec<DetectedApp>>,
}

impl Global for ScanGlobal {}

/// Makes `env` the machine the scan reads. Nothing is read until
/// [`apps`] is first asked.
pub fn install(env: Env, cx: &mut App) {
    cx.set_global(ScanGlobal { env, apps: None });
}

/// A scan a test or screenshot supplies, in place of reading the machine.
#[cfg(test)]
pub fn install_fixture(apps: Vec<DetectedApp>, cx: &mut App) {
    cx.set_global(ScanGlobal {
        env: Env::fixture(pwrroute_browsers::Platform::current(), std::env::temp_dir()),
        apps: Some(apps),
    });
}

/// The browsers on this machine, scanned the first time this is called.
/// Before [`install`] (the window harness) the real machine is read each
/// time, as the pickers always did.
pub fn apps(cx: &mut App) -> Vec<DetectedApp> {
    if !cx.has_global::<ScanGlobal>() {
        return scan(&Env::system());
    }
    cx.update_global(|global: &mut ScanGlobal, _| {
        global.apps.get_or_insert_with(|| scan(&global.env)).clone()
    })
}

/// Whether the scan has run, so [`location_of`] can answer.
pub fn has_run(cx: &App) -> bool {
    cx.try_global::<ScanGlobal>()
        .is_some_and(|global| global.apps.is_some())
}

/// Where the scan found the app `value` names, when it has already run;
/// never starts one, so it never reads the disk.
pub fn location_of(value: &str, cx: &App) -> Option<PathBuf> {
    cx.try_global::<ScanGlobal>()?
        .apps
        .as_ref()?
        .iter()
        .find(|app| app.is_named(value))
        .map(|app| app.location.clone())
}

/// Reads the machine again: Settings' Rescan and first run's Look again.
/// The apps' icons are read again too.
pub fn rescan(cx: &mut App) -> Vec<DetectedApp> {
    crate::app_icons::forget(cx);
    reread(cx)
}

/// Reads the machine again for first run's profile-access step, which
/// re-reads each time its window comes to the front: the same scan as
/// [`rescan`], but the icons stay, as the apps did not change.
pub fn reread(cx: &mut App) -> Vec<DetectedApp> {
    if !cx.has_global::<ScanGlobal>() {
        return scan(&Env::system());
    }
    cx.update_global(|global: &mut ScanGlobal, _| {
        let apps = scan(&global.env);
        global.apps = Some(apps.clone());
        apps
    })
}

/// The pickers' choices from `apps`, less the ones `hidden` names.
pub fn choices_from(
    apps: &[DetectedApp],
    hidden: &settings_store::BrowserSettings,
) -> Vec<BrowserChoice> {
    apps.iter()
        .filter(|app| !hidden.is_hidden(app.id()))
        .map(|app| BrowserChoice {
            name: app.name.clone(),
            profiles: app.profiles.clone(),
        })
        .collect()
}

/// What the pickers offer: the scan less the apps hidden in Settings.
pub fn choices(cx: &mut App) -> Vec<BrowserChoice> {
    let apps = apps(cx);
    let hidden = settings_store::store(cx)
        .map(|store| store.read(cx).settings().browsers.clone())
        .unwrap_or_default();
    choices_from(&apps, &hidden)
}

/// Hands every open main window the choices as they now stand, after a
/// rescan or a hidden app changed.
pub fn refresh_windows(cx: &mut App) {
    let choices = choices(cx);
    let windows: Vec<_> = cx
        .windows()
        .into_iter()
        .filter_map(|window| window.downcast::<PwrRoute>())
        .collect();
    for window in windows {
        let choices = choices.clone();
        let _ = window.update(cx, |app, _, cx| {
            app.browsers = choices;
            cx.notify();
        });
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use pwrroute_browsers::Platform;

    use super::*;

    /// A fixture machine with one `.app` of `bundle_id` under
    /// `/Applications`. Safari is in the catalog, so a bundle id is all the
    /// plist needs.
    pub(crate) fn machine_with(apps: &[(&str, &str)]) -> (tempfile::TempDir, Env) {
        let dir = tempfile::tempdir().unwrap();
        for (name, bundle_id) in apps {
            let contents = dir
                .path()
                .join("Applications")
                .join(format!("{name}.app"))
                .join("Contents");
            fs::create_dir_all(&contents).unwrap();
            fs::write(
                contents.join("Info.plist"),
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                     <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
                     <plist version=\"1.0\"><dict>\n\
                     <key>CFBundleIdentifier</key><string>{bundle_id}</string>\n\
                     <key>CFBundleName</key><string>{name}</string>\n\
                     </dict></plist>\n"
                ),
            )
            .unwrap();
        }
        let env = Env::fixture(Platform::MacOs, dir.path());
        (dir, env)
    }

    #[test]
    fn a_machine_with_only_safari_scans_to_one_app() {
        let (_dir, env) = machine_with(&[("Safari", "com.apple.Safari")]);
        let apps = scan(&env);
        assert_eq!(apps.len(), 1, "{apps:?}");
        let safari = &apps[0];
        assert_eq!(safari.name, "Safari");
        assert_eq!(safari.id(), "com.apple.Safari");
        assert!(safari.location.ends_with("Applications/Safari.app"));
        assert!(
            safari.profiles.is_empty(),
            "no profile files in the fixture"
        );
        assert!(safari.is_named("com.apple.safari"));
        assert!(safari.is_named("safari"));
        assert!(safari.is_named(safari.location.to_str().unwrap()));
        assert!(!safari.is_named("Google Chrome"));
        assert!(!safari.is_named(""));
    }

    #[test]
    fn an_empty_machine_scans_to_nothing() {
        let (_dir, env) = machine_with(&[]);
        assert!(scan(&env).is_empty());
    }

    #[test]
    fn hidden_apps_leave_the_choices() {
        let apps = vec![
            DetectedApp {
                name: "Safari".into(),
                bundle_id: Some("com.apple.Safari".into()),
                location: "/Applications/Safari.app".into(),
                profiles: vec![],
                directories: Vec::new(),
                profiles_blocked: false,
            },
            DetectedApp {
                name: "Firefox".into(),
                bundle_id: Some("org.mozilla.firefox".into()),
                location: "/Applications/Firefox.app".into(),
                profiles: vec!["default-release".into()],
                directories: Vec::new(),
                profiles_blocked: false,
            },
            DetectedApp {
                name: "Nameless".into(),
                bundle_id: None,
                location: "/Applications/Nameless.app".into(),
                profiles: vec![],
                directories: Vec::new(),
                profiles_blocked: false,
            },
        ];
        let mut hidden = settings_store::BrowserSettings::default();
        let all = choices_from(&apps, &hidden);
        assert_eq!(
            all.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["Safari", "Firefox", "Nameless"]
        );
        assert_eq!(all[1].profiles, ["default-release"]);

        hidden.set_hidden("ORG.mozilla.Firefox", true);
        hidden.set_hidden("Nameless", true);
        let shown = choices_from(&apps, &hidden);
        assert_eq!(
            shown.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["Safari"]
        );
    }
}
