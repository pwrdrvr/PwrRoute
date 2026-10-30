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
use objc2::{MainThreadMarker, rc::autoreleasepool, runtime::NSObjectProtocol, sel};
use objc2_app_kit::{NSApplication, NSRunningApplication, NSWorkspace};
use objc2_foundation::{NSAppleEventManager, NSBundle, NSProcessInfo, NSString, NSURL};
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

/// Apple's LaunchServices tool. No public API takes a bundle that is still
/// on disk out of the database; `lsregister -u` does.
const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/\
                          LaunchServices.framework/Support/lsregister";

/// Asks for one scheme, so macOS shows one prompt: each call puts up its
/// own "change your default web browser?" dialog (macOS 27 stacks them),
/// and the answer to `http`'s makes PwrRoute the default web browser,
/// `https` and `public.html` with it. Only a request that left `https`
/// behind asks for it, on the next try.
///
/// LaunchServices keeps the default browser as a bundle id and opens the
/// highest-versioned copy with that id, so a newer dev build, or a copy on
/// a mounted DMG, would take this copy's links, and macOS asks nothing when
/// the id already holds the default. This copy is the one being asked for,
/// so the other copies are taken out of LaunchServices first. One comes
/// back when it is launched again.
pub fn set_default() -> Result<DefaultRequest> {
    let bundle = app_bundle()?;
    let other_copy_held = status()?.other_copy.is_some();
    forget_other_copies(&bundle);
    launch_services::register(&bundle)?;
    let http = is_current_handler("http", &bundle);
    let https = is_current_handler("https", &bundle);
    if http && https {
        return Ok(if other_copy_held {
            DefaultRequest::TookOverFromCopy
        } else {
            DefaultRequest::Set
        });
    }
    let scheme = scheme_to_request(http, https);
    let workspace = NSWorkspace::sharedWorkspace();
    if workspace.respondsToSelector(sel!(
        setDefaultApplicationAtURL:toOpenURLsWithScheme:completionHandler:
    )) {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&bundle.to_string_lossy()));
        workspace.setDefaultApplicationAtURL_toOpenURLsWithScheme_completionHandler(
            &url,
            &NSString::from_str(scheme),
            None,
        );
    } else {
        // macOS 11 predates NSWorkspace's path-specific API.
        launch_services::set_default_handler(scheme, BUNDLE_ID)?;
    }
    Ok(DefaultRequest::Prompted)
}

/// Every copy of PwrRoute LaunchServices knows other than `current`: an
/// install in `/Applications`, a mounted DMG, each checkout's dev build.
fn other_copies(current: &Path) -> Vec<PathBuf> {
    NSWorkspace::sharedWorkspace()
        .URLsForApplicationsWithBundleIdentifier(&NSString::from_str(BUNDLE_ID))
        .iter()
        .filter_map(|url| url.path().map(|path| PathBuf::from(path.to_string())))
        .filter(|path| !same_bundle(Some(path), current))
        .collect()
}

/// Takes the other copies out of LaunchServices, all at once: this runs on
/// the click, on the main thread. The files stay. A copy that will not go
/// is left, and the status read afterwards names it.
fn forget_other_copies(current: &Path) {
    use std::process::{Command, Stdio};
    let running: Vec<_> = other_copies(current)
        .into_iter()
        .filter_map(|copy| {
            Command::new(LSREGISTER)
                .arg("-u")
                .arg(&copy)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .ok()
        })
        .collect();
    for mut child in running {
        let _ = child.wait();
    }
}

/// Ask for `https` once `http` selects this bundle and `https` does not,
/// otherwise `http`. Copies with the same bundle id can have different paths.
fn scheme_to_request(http_is_current: bool, https_is_current: bool) -> &'static str {
    if http_is_current && !https_is_current {
        "https"
    } else {
        "http"
    }
}

/// System Settings › Desktop & Dock, which holds "Default web browser".
pub fn open_settings() -> Result<()> {
    open_settings_pane("x-apple.systempreferences:com.apple.Desktop-Settings.extension")
}

/// System Settings › Privacy & Security › Files & Folders, where each app's
/// access to other apps' data is switched on.
pub fn open_files_and_folders() -> Result<()> {
    open_settings_pane(
        "x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders",
    )
}

/// Opens a System Settings pane through NSWorkspace, which says whether it
/// did, rather than a spawned `open`, which does not.
fn open_settings_pane(pane: &str) -> Result<()> {
    let Some(url) = NSURL::URLWithString(&NSString::from_str(pane)) else {
        bail!("not a System Settings URL: {pane}");
    };
    if !NSWorkspace::sharedWorkspace().openURL(&url) {
        bail!("macOS did not open System Settings");
    }
    Ok(())
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

/// `NSProcessInfo.systemUptime`: time awake since boot, from the same clock
/// as `CLOCK_UPTIME_RAW` (`mach_absolute_time`). Thread-safe.
pub fn system_uptime() -> Option<Duration> {
    let seconds = NSProcessInfo::processInfo().systemUptime();
    Duration::try_from_secs_f64(seconds).ok()
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

/// The bundle id of the app at `path` (an `.app` the person picked), from
/// its Info.plist. `None` when it is not a bundle or has no id.
pub fn app_bundle_id(path: &Path) -> Option<String> {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let id = NSBundle::bundleWithURL(&url)?
        .bundleIdentifier()?
        .to_string();
    (!id.trim().is_empty()).then_some(id)
}

/// Whether a scheme's handler, as LaunchServices names it, is PwrRoute.
fn is_ours(handler: Option<&str>) -> bool {
    handler.is_some_and(|handler| handler.eq_ignore_ascii_case(BUNDLE_ID))
}

pub fn status() -> Result<DefaultStatus> {
    let http = launch_services::default_handler("http");
    let https = launch_services::default_handler("https");
    let mut is_default = is_ours(http.as_deref()) && is_ours(https.as_deref());
    let mut other_copy = None;
    if let Ok(bundle) = app_bundle() {
        for (scheme, handler) in [("http", &http), ("https", &https)] {
            if !is_ours(handler.as_deref()) {
                continue;
            }
            let path = handler_path(scheme);
            if !same_bundle(path.as_deref(), &bundle) {
                is_default = false;
                if other_copy.is_none() {
                    other_copy = path.map(|path| path.to_string_lossy().into_owned());
                }
            }
        }
    }
    Ok(DefaultStatus {
        is_default,
        http,
        https,
        other_copy,
    })
}

/// The bundle LaunchServices opens a `scheme` link with.
fn handler_path(scheme: &str) -> Option<PathBuf> {
    let url = NSURL::URLWithString(&NSString::from_str(&format!("{scheme}://example.com")));
    let handler =
        url.and_then(|url| NSWorkspace::sharedWorkspace().URLForApplicationToOpenURL(&url));
    handler
        .and_then(|url| url.path())
        .map(|s| PathBuf::from(s.to_string()))
}

fn is_current_handler(scheme: &str, bundle: &Path) -> bool {
    same_bundle(handler_path(scheme).as_deref(), bundle)
}

fn same_bundle(handler: Option<&Path>, current: &Path) -> bool {
    handler.is_some_and(
        |handler| match (handler.canonicalize(), current.canonicalize()) {
            (Ok(handler), Ok(current)) => handler == current,
            _ => handler == current,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_scheme_is_asked_for_and_https_only_when_left_behind() {
        assert_eq!(scheme_to_request(false, false), "http");
        assert_eq!(scheme_to_request(false, true), "http");
        assert_eq!(scheme_to_request(true, false), "https");
        assert_eq!(scheme_to_request(true, true), "http");
    }

    #[test]
    fn an_installed_copy_does_not_make_the_worktree_copy_default() {
        let worktree = Path::new("/worktrees/feature/target/dev/PwrRoute.app");
        assert!(same_bundle(Some(worktree), worktree));
        assert!(!same_bundle(
            Some(Path::new("/Applications/PwrRoute.app")),
            worktree
        ));
        assert!(!same_bundle(None, worktree));
        // Even with the same bundle id, a split between copies must ask
        // for https after http has moved to this checkout.
        assert_eq!(
            scheme_to_request(
                same_bundle(Some(worktree), worktree),
                same_bundle(Some(Path::new("/Applications/PwrRoute.app")), worktree),
            ),
            "https"
        );
    }
}
