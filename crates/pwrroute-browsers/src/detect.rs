//! Which browsers are installed.
//!
//! - macOS: `.app` bundles in `/Applications`, `~/Applications` and
//!   `/System/Applications`, kept when their bundle id is in the catalog or
//!   their `Info.plist` claims http(s) at rank `Default` or `Alternate`
//!   (Finicky's test). Launch Services is not consulted: that needs FFI, and
//!   launching does not need detection on macOS (`open` asks Launch
//!   Services itself).
//! - Windows: the registry's `StartMenuInternet` registrations, then each
//!   catalog entry's install paths.
//! - Linux: desktop entries that handle `x-scheme-handler/https` or are in
//!   the catalog, then catalog executables on `$PATH`.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    catalog::{BrowserInfo, Catalog},
    desktop_entry::{self, DesktopEntry},
    env::{Env, Platform},
    plist, registry,
};

/// Bundle ids, executables and desktop ids of link routers, which must never
/// be offered as a destination: opening a link in one sends it back here.
const ROUTERS: &[&str] = &[
    "com.pwrdrvr.pwrroute",
    "com.pwrdrvr.pwrfinicky",
    "se.johnste.finicky",
    "net.kassett.finicky",
    "pwrroute.exe",
    "pwrroute",
    "com.pwrdrvr.pwrroute.desktop",
    "pwrroute.desktop",
];

/// A browser found on this machine.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct InstalledBrowser {
    /// The name to show, and to write in a config with `appType: "appName"`
    /// on macOS.
    pub name: String,
    /// The catalog entry, when the browser is one PwrRoute knows.
    pub browser: Option<&'static BrowserInfo>,
    /// The `.app` bundle, the `.exe`, or the `.desktop` file.
    pub location: PathBuf,
    /// The macOS bundle id, when known.
    pub bundle_id: Option<String>,
}

/// The browsers installed on `env`'s machine, one per bundle id, executable
/// or desktop id, in discovery order. Never includes a link router.
pub fn detect_installed(env: &Env) -> Vec<InstalledBrowser> {
    let catalog = Catalog::builtin();
    match env.platform() {
        Platform::MacOs => macos(env, catalog),
        Platform::Windows => windows(env, catalog),
        Platform::Linux => linux(env, catalog),
    }
}

fn is_router(id: &str) -> bool {
    ROUTERS.iter().any(|r| r.eq_ignore_ascii_case(id))
}

fn macos(env: &Env, catalog: &'static Catalog) -> Vec<InstalledBrowser> {
    let dirs = [
        env.system_path("/Applications"),
        env.home().join("Applications"),
        env.system_path("/System/Applications"),
    ];
    let mut found: Vec<InstalledBrowser> = Vec::new();
    for dir in dirs {
        for app in sorted_entries(&dir, "app") {
            let Some(b) = macos_app(catalog, &app) else {
                continue;
            };
            let duplicate = found.iter().any(|f| match (&f.bundle_id, &b.bundle_id) {
                (Some(a), Some(c)) => a.eq_ignore_ascii_case(c),
                _ => f.name == b.name,
            });
            if !duplicate {
                found.push(b);
            }
        }
    }
    found
}

pub(crate) fn macos_app(catalog: &'static Catalog, app: &Path) -> Option<InstalledBrowser> {
    let name = app.file_stem()?.to_str()?.to_owned();
    let info = fs::read_to_string(app.join("Contents/Info.plist"))
        .ok()
        .and_then(|xml| plist::parse(&xml));
    let (browser, bundle_id) = match &info {
        Some(info) => {
            let id = info.bundle_id.clone();
            if id.as_deref().is_some_and(is_router) {
                return None;
            }
            let browser = id.as_deref().and_then(|id| catalog.find_bundle_id(id));
            if browser.is_none() && !info.is_likely_browser() {
                return None;
            }
            (browser, id)
        }
        // A binary or missing plist: known only by its bundle name.
        None => {
            let browser = catalog.find_by_path(Platform::MacOs, app)?;
            let id = browser.macos.as_ref().and_then(|m| m.bundle_id.clone());
            (Some(browser), id)
        }
    };
    Some(InstalledBrowser {
        name,
        browser,
        location: app.to_owned(),
        bundle_id,
    })
}

fn windows(env: &Env, catalog: &'static Catalog) -> Vec<InstalledBrowser> {
    let mut found: Vec<InstalledBrowser> = Vec::new();
    let push = |found: &mut Vec<InstalledBrowser>, b: InstalledBrowser| {
        let file = b
            .location
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("");
        let same = |f: &InstalledBrowser| {
            f.location
                .to_string_lossy()
                .eq_ignore_ascii_case(&b.location.to_string_lossy())
        };
        if !is_router(file) && !found.iter().any(same) {
            found.push(b);
        }
    };
    for r in registry::start_menu_internet(env) {
        let browser = catalog.find_by_path(Platform::Windows, &r.exe);
        push(
            &mut found,
            InstalledBrowser {
                name: browser.map_or(r.name, |b| b.name.clone()),
                browser,
                location: r.exe,
                bundle_id: None,
            },
        );
    }
    for b in catalog.browsers() {
        if let Some(exe) = windows_exe(env, b) {
            push(
                &mut found,
                InstalledBrowser {
                    name: b.name.clone(),
                    browser: Some(b),
                    location: exe,
                    bundle_id: None,
                },
            );
        }
    }
    found
}

/// The first of a catalog entry's Windows install paths that exists.
pub(crate) fn windows_exe(env: &Env, b: &BrowserInfo) -> Option<PathBuf> {
    b.windows
        .as_ref()?
        .exe
        .iter()
        .filter_map(|t| env.expand(t))
        .find(|p| p.is_file())
}

fn linux(env: &Env, catalog: &'static Catalog) -> Vec<InstalledBrowser> {
    let mut found: Vec<InstalledBrowser> = Vec::new();
    let mut ids = std::collections::HashSet::new();
    // Catalog browsers whose desktop entry is hidden or unusable: the PATH
    // fallback below must not bring them back.
    let mut unusable = std::collections::HashSet::new();
    for dir in desktop_entry::application_dirs(env) {
        for path in sorted_entries(&dir, "desktop") {
            let Some(id) = desktop_entry::id_of(&path).map(str::to_owned) else {
                continue;
            };
            // The first directory with an id wins, even if it hides it.
            if !ids.insert(id.clone()) || is_router(&id) {
                continue;
            }
            let Some(entry) = fs::read_to_string(&path)
                .ok()
                .and_then(|t| DesktopEntry::parse(&t))
            else {
                continue;
            };
            let browser = catalog
                .find_by_path(Platform::Linux, &path)
                .or_else(|| exec_browser(catalog, &entry));
            if !entry.usable(env) {
                unusable.extend(browser.map(|b| b.id.as_str()));
                continue;
            }
            if browser.is_none() && !entry.handles_https() {
                continue;
            }
            found.push(InstalledBrowser {
                name: browser.map_or(entry.name, |b| b.name.clone()),
                browser,
                location: path,
                bundle_id: None,
            });
        }
    }
    for b in catalog.browsers() {
        if unusable.contains(b.id.as_str())
            || found
                .iter()
                .any(|f| f.browser.is_some_and(|f| f.id == b.id))
        {
            continue;
        }
        let Some(linux) = &b.linux else { continue };
        if let Some(exe) = linux.executables.iter().find_map(|e| env.which(e)) {
            found.push(InstalledBrowser {
                name: b.name.clone(),
                browser: Some(b),
                location: exe,
                bundle_id: None,
            });
        }
    }
    found
}

/// The catalog entry whose executable an `Exec` line runs.
fn exec_browser(catalog: &'static Catalog, entry: &DesktopEntry) -> Option<&'static BrowserInfo> {
    let (program, _) = desktop_entry::exec_command(&entry.exec, &[])?;
    catalog.find_by_path(Platform::Linux, &program)
}

/// Entries of `dir` with extension `ext`, sorted by name. Missing or
/// unreadable directories are empty.
fn sorted_entries(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = read
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| p.extension().is_some_and(|e| e == ext))
        .collect();
    paths.sort();
    paths
}
