//! macOS: the sender of an Apple Event, and LaunchServices registration.

mod launch_services;

use std::path::PathBuf;

use anyhow::{Result, bail};
use objc2_app_kit::NSRunningApplication;
use objc2_foundation::{NSAppleEventManager, NSBundle};
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
