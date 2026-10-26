//! macOS: the sender of an Apple Event, LaunchServices registration, apps'
//! icons and the window order.

mod app_icon;
mod launch_services;
mod window_order;

pub use app_icon::{bundle_path as installed_app_bundle, icon_png as app_icon_png};
pub use window_order::top_window_owner;

use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};

use anyhow::{Result, bail};
use dispatch2::DispatchQueue;
use objc2::{MainThreadMarker, rc::autoreleasepool};
use objc2_app_kit::{NSApplication, NSRunningApplication, NSWorkspace};
use objc2_foundation::{NSAppleEventManager, NSBundle, NSString};
use pwrroute_router::OpenerInfo;

use crate::{
    Yield,
    registration::{BUNDLE_ID, DefaultRequest, DefaultStatus},
};

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

/// Asks for one scheme, so macOS shows one prompt: each call puts up its
/// own "change your default web browser?" dialog (macOS 27 stacks them),
/// and the answer to `http`'s makes PwrRoute the default web browser,
/// `https` and `public.html` with it. Only a request that left `https`
/// behind asks for it, on the next try.
pub fn set_default() -> Result<DefaultRequest> {
    app_bundle()?;
    let status = status()?;
    let scheme = scheme_to_request(&status);
    launch_services::set_default_handler(scheme, BUNDLE_ID)?;
    Ok(DefaultRequest::Prompted)
}

/// The scheme a request asks for: `https` once `http` is PwrRoute's and
/// `https` is not, else `http`.
fn scheme_to_request(status: &DefaultStatus) -> &'static str {
    if is_ours(status.http.as_deref()) && !is_ours(status.https.as_deref()) {
        "https"
    } else {
        "http"
    }
}

/// System Settings › Desktop & Dock, which holds "Default web browser".
pub fn open_settings() -> Result<()> {
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.Desktop-Settings.extension")
        .spawn()
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("opening System Settings: {e}"))
}

/// System Settings › Privacy & Security › Files & Folders, where each app's
/// access to other apps' data is switched on.
pub fn open_files_and_folders() -> Result<()> {
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders")
        .spawn()
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("opening System Settings: {e}"))
}

/// Lets the app with `bundle_id` bring itself forward, when this app is the
/// active one: macOS 14's cooperative activation lets an app activate
/// itself only once the active app yields to it. A browser that takes a
/// link from the singleton socket, or from a `-n` instance, activates the
/// window it opened the tab in, which is the one to bring forward; this
/// app activating the browser instead would raise whichever window was
/// last in front.
///
/// `yieldActivation` is an `NSApplication` call, so it runs on the main
/// queue. From another thread this waits up to `wait` for it, so that the
/// yield comes before the hand-off when the main thread is free; a busy one
/// gets the yield queued and the caller goes on.
pub fn yield_front_to(bundle_id: &str, wait: Duration) -> Yield {
    if !objc2::available!(macos = 14.0) {
        return Yield::Unsupported;
    }
    let active = autoreleasepool(|_| NSRunningApplication::currentApplication().isActive());
    if !active {
        return Yield::NotActive;
    }
    let id = bundle_id.to_owned();
    let give_way = move |mtm: MainThreadMarker| {
        autoreleasepool(|_| {
            NSApplication::sharedApplication(mtm)
                .yieldActivationToApplicationWithBundleIdentifier(&NSString::from_str(&id));
        });
    };
    if let Some(mtm) = MainThreadMarker::new() {
        give_way(mtm);
        return Yield::Yielded;
    }
    let (done, yielded) = mpsc::sync_channel(1);
    DispatchQueue::main().exec_async(move || {
        if let Some(mtm) = MainThreadMarker::new() {
            give_way(mtm);
        }
        let _ = done.send(());
    });
    match yielded.recv_timeout(wait) {
        Ok(()) => Yield::Yielded,
        Err(_) => Yield::Queued,
    }
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

/// Whether a scheme's handler, as LaunchServices names it, is PwrRoute.
fn is_ours(handler: Option<&str>) -> bool {
    handler.is_some_and(|handler| handler.eq_ignore_ascii_case(BUNDLE_ID))
}

pub fn status() -> Result<DefaultStatus> {
    let http = launch_services::default_handler("http");
    let https = launch_services::default_handler("https");
    Ok(DefaultStatus {
        is_default: is_ours(http.as_deref()) && is_ours(https.as_deref()),
        http,
        https,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(http: Option<&str>, https: Option<&str>) -> DefaultStatus {
        DefaultStatus {
            is_default: false,
            http: http.map(str::to_owned),
            https: https.map(str::to_owned),
        }
    }

    #[test]
    fn one_scheme_is_asked_for_and_https_only_when_left_behind() {
        let edge = Some("com.microsoft.edgemac");
        let upper = BUNDLE_ID.to_ascii_uppercase();
        let ours = Some(upper.as_str());
        assert_eq!(scheme_to_request(&status(edge, edge)), "http");
        assert_eq!(scheme_to_request(&status(None, None)), "http");
        assert_eq!(scheme_to_request(&status(edge, ours)), "http");
        assert_eq!(scheme_to_request(&status(ours, edge)), "https");
        assert_eq!(scheme_to_request(&status(ours, None)), "https");
    }
}
