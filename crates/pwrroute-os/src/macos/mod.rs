//! macOS: the sender of an Apple Event, LaunchServices registration and
//! apps' icons.

mod app_icon;
mod launch_services;

pub use app_icon::{bundle_path as installed_app_bundle, icon_png as app_icon_png};

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use objc2_app_kit::{NSRunningApplication, NSWorkspace, NSWorkspaceOpenConfiguration};
use objc2_foundation::{NSAppleEventManager, NSArray, NSBundle, NSString, NSURL};
use pwrroute_router::OpenerInfo;

use crate::registration::{
    BUNDLE_ID, DefaultRequest, DefaultStatus, LINK_HELPER_ID, LINK_HELPER_PATH,
    LINK_HELPER_STARTED_BY_APP, macos_status,
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

/// The link helper nested in `app` (`Contents/Helpers/PwrRoute.app`), when
/// it is there and is the helper: `None` for a bundle built before it, or
/// one put together by hand.
fn link_helper(app: &Path) -> Option<PathBuf> {
    let path = app.join(LINK_HELPER_PATH);
    let bundle = NSBundle::bundleWithPath(&NSString::from_str(&path.to_string_lossy()))?;
    let id = bundle.bundleIdentifier()?.to_string();
    (id == LINK_HELPER_ID).then_some(path)
}

/// Lists the app and its link helper with LaunchServices. The helper is
/// registered by its own URL: LaunchServices need not have scanned a bundle
/// nested in another before it can be made a handler.
pub fn register() -> Result<()> {
    let app = app_bundle()?;
    launch_services::register(&app)?;
    if let Some(helper) = link_helper(&app) {
        launch_services::register(&helper)?;
    }
    Ok(())
}

/// LaunchServices forgets a bundle once it is deleted, and there is no
/// public call to forget one that is still on disk, so there is nothing to
/// undo here.
pub fn unregister() -> Result<()> {
    Ok(())
}

/// Asks for the link helper to handle `http` and `https`. The helper, not
/// the app, so that a link never activates the process with the windows:
/// LaunchServices activates whichever app it hands a link to.
pub fn set_default() -> Result<DefaultRequest> {
    let app = app_bundle()?;
    let Some(helper) = link_helper(&app) else {
        bail!(
            "this PwrRoute.app has no link helper at {LINK_HELPER_PATH}; build it with `cargo \
             xtask bundle`"
        );
    };
    // LaunchServices hands a scheme only to a bundle id it knows. A DMG
    // install never runs the updater's register hook, and LaunchServices
    // need not have scanned a bundle nested in another: register the helper
    // by its URL first, or the request names an id macOS has never seen.
    launch_services::register(&helper)?;
    for scheme in ["http", "https"] {
        launch_services::set_default_handler(scheme, LINK_HELPER_ID)?;
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
    Ok(macos_status(
        launch_services::default_handler("http"),
        launch_services::default_handler("https"),
    ))
}

/// [`crate::start_link_helper`] on macOS.
pub fn start_link_helper() {
    if app_is_running(LINK_HELPER_ID) {
        return;
    }
    let bundle = NSBundle::mainBundle().bundlePath().to_string();
    let helper = Path::new(&bundle).join(LINK_HELPER_PATH);
    if !helper.join("Contents/Info.plist").is_file() {
        return;
    }
    let url =
        NSURL::fileURLWithPath_isDirectory(&NSString::from_str(&helper.to_string_lossy()), true);
    let configuration = NSWorkspaceOpenConfiguration::configuration();
    configuration.setActivates(false);
    configuration.setAddsToRecentItems(false);
    configuration.setArguments(&NSArray::from_retained_slice(&[NSString::from_str(
        LINK_HELPER_STARTED_BY_APP,
    )]));
    // Fire and forget: a helper that does not start is started by the
    // first link instead, as before.
    NSWorkspace::sharedWorkspace().openApplicationAtURL_configuration_completionHandler(
        &url,
        &configuration,
        None,
    );
}
