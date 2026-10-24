//! macOS: the sender of an Apple Event, LaunchServices registration, apps'
//! icons and the window order.

mod app_icon;
mod launch_services;
mod window_order;

pub use app_icon::{bundle_path as installed_app_bundle, icon_png as app_icon_png};
pub use window_order::top_window_owner;

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use objc2::rc::autoreleasepool;
use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};
use objc2_foundation::{NSAppleEventManager, NSBundle, NSString};
use pwrroute_router::OpenerInfo;

use crate::registration::{BUNDLE_ID, DefaultRequest, DefaultStatus};

/// `keySenderPIDAttr` ('spid'): the pid of the process that sent the event.
const KEY_SENDER_PID: u32 = u32::from_be_bytes(*b"spid");

/// The app behind the `kAEGetURL` event AppKit is handling, as Finicky reads
/// it: name, bundle id, and the bundle's path. Cheap: no Accessibility query,
/// so no window title.
pub fn current_apple_event_opener() -> Option<OpenerInfo> {
    let event = NSAppleEventManager::sharedAppleEventManager().currentAppleEvent()?;
    let pid = event
        .attributeDescriptorForKeyword(KEY_SENDER_PID)?
        .int32Value();
    if pid <= 0 {
        return None;
    }
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
    let path = app
        .bundleURL()
        .or_else(|| app.executableURL())
        .and_then(|url| url.path())
        .map(|p| p.to_string())
        .unwrap_or_default();
    Some(OpenerInfo {
        name: app
            .localizedName()
            .map(|s| s.to_string())
            .unwrap_or_default(),
        bundle_id: app
            .bundleIdentifier()
            .map(|s| s.to_string())
            .unwrap_or_default(),
        path,
        window_title: None,
    })
}

/// The running `PwrRoute.app`, or an error saying to launch it: a bare dev
/// binary has no bundle for LaunchServices to register.
fn app_bundle() -> Result<PathBuf> {
    let bundle = NSBundle::mainBundle();
    let id = bundle.bundleIdentifier().map(|s| s.to_string());
    let path = PathBuf::from(bundle.bundlePath().to_string());
    if id.as_deref() != Some(BUNDLE_ID) || path.extension().is_none_or(|e| e != "app") {
        bail!("launch PwrRoute.app (from `cargo xtask bundle`) to register it as a browser");
    }
    Ok(path)
}

pub fn register() -> Result<()> {
    launch_services::register(&app_bundle()?)
}

/// LaunchServices forgets a bundle once it is deleted, and there is no
/// public call to forget one that is still on disk, so there is nothing to
/// undo here.
pub fn unregister() -> Result<()> {
    Ok(())
}

pub fn set_default() -> Result<DefaultRequest> {
    app_bundle()?;
    for scheme in ["http", "https"] {
        launch_services::set_default_handler(scheme, BUNDLE_ID)?;
    }
    Ok(DefaultRequest::Prompted)
}

/// System Settings › Desktop & Dock, which holds "Default web browser".
pub fn open_settings() -> Result<()> {
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.Desktop-Settings.extension")
        .spawn()
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("opening System Settings: {e}"))
}

/// Hands the front to the running `app` (a bundle id, an app name or a
/// path), when this app is the active one: macOS 14's cooperative
/// activation lets an app bring itself forward only when the active app
/// gives way, and a browser that takes a link from a second instance
/// (`open -g -n`, a Chromium profile hand-off) brings itself forward. Called
/// from any thread: `NSRunningApplication` is thread-safe, unlike
/// `NSApp.yieldActivation`, and the main thread may be the one busy.
/// `false` when nothing was asked: an older macOS, this app not active, or
/// `app` not running.
pub fn pass_front_to(app: &str) -> bool {
    if !objc2::available!(macos = 14.0) {
        return false;
    }
    autoreleasepool(|_| {
        let me = NSRunningApplication::currentApplication();
        if !me.isActive() {
            return false;
        }
        let Some(id) = bundle_id_for(app) else {
            return false;
        };
        // A hand-off's `-n` starts a second instance that only passes the
        // link on and quits: the front goes to the one that was there first.
        let Some(target) =
            NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(&id))
                .iter()
                .filter(|running| !running.isTerminated())
                .min_by(|a, b| launched_at(a).total_cmp(&launched_at(b)))
        else {
            return false;
        };
        target.activateFromApplication_options(&me, NSApplicationActivationOptions::empty())
    })
}

/// When `app` was launched, in seconds since 1970; the earliest possible for
/// an app LaunchServices didn't launch, which has no launch date.
fn launched_at(app: &NSRunningApplication) -> f64 {
    app.launchDate()
        .map_or(f64::NEG_INFINITY, |date| date.timeIntervalSince1970())
}

/// The bundle id of `app`: itself when an app with that id is running,
/// else the installed bundle's that `app` names.
fn bundle_id_for(app: &str) -> Option<String> {
    if app_is_running(app) {
        return Some(app.to_owned());
    }
    let path = installed_app_bundle(app)?;
    NSBundle::bundleWithPath(&NSString::from_str(path.to_str()?))?
        .bundleIdentifier()
        .map(|id| id.to_string())
}

/// Whether an app with `bundle_id` is running, as AppKit's running
/// applications list has it.
pub fn app_is_running(bundle_id: &str) -> bool {
    NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(bundle_id))
        .count()
        > 0
}

/// The installed app with `bundle_id`, by the name Finder shows: the
/// bundle's (localized) `CFBundleDisplayName`, else `CFBundleName`, else
/// the bundle's file name without `.app`. `None` when LaunchServices knows
/// no app with that id.
pub fn app_display_name(bundle_id: &str) -> Option<String> {
    let url = NSWorkspace::sharedWorkspace()
        .URLForApplicationWithBundleIdentifier(&NSString::from_str(bundle_id))?;
    let from_plist = NSBundle::bundleWithURL(&url).and_then(|bundle| {
        ["CFBundleDisplayName", "CFBundleName"]
            .into_iter()
            .find_map(|key| {
                bundle
                    .objectForInfoDictionaryKey(&NSString::from_str(key))?
                    .downcast::<NSString>()
                    .ok()
                    .map(|name| name.to_string())
                    .filter(|name| !name.trim().is_empty())
            })
    });
    from_plist.or_else(|| {
        let path = url.path()?.to_string();
        Path::new(&path)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
    })
}

pub fn status() -> Result<DefaultStatus> {
    let http = launch_services::default_handler("http");
    let https = launch_services::default_handler("https");
    let ours = |h: &Option<String>| {
        h.as_deref()
            .is_some_and(|h| h.eq_ignore_ascii_case(BUNDLE_ID))
    };
    Ok(DefaultStatus {
        is_default: ours(&http) && ours(&https),
        http,
        https,
    })
}
