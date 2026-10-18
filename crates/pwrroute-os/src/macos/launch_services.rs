//! The LaunchServices calls. They are C functions, which Rust can only call
//! in `unsafe`; this module is the one place PwrRoute allows it.
#![allow(unsafe_code)]

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use objc2_core_foundation::{CFString, CFURL};
#[allow(deprecated)]
use objc2_core_services::{
    LSCopyDefaultHandlerForURLScheme, LSRegisterURL, LSSetDefaultHandlerForURLScheme,
};

/// Adds (or refreshes) the bundle in the LaunchServices database, which is
/// what lists it as able to open `http` and `https`.
pub fn register(bundle: &Path) -> Result<()> {
    let url = CFURL::from_directory_path(bundle).context("the app's path is not a valid URL")?;
    // SAFETY: `url` is a live CFURL for the call's duration; LSRegisterURL
    // only reads it.
    let status = unsafe { LSRegisterURL(&url, true) };
    if status != 0 {
        bail!("LaunchServices could not register PwrRoute (OSStatus {status})");
    }
    Ok(())
}

/// Asks for `bundle_id` to handle `scheme`. For `http` and `https` macOS
/// answers with its "change your default web browser?" prompt; this returns
/// before the user decides.
pub fn set_default_handler(scheme: &str, bundle_id: &str) -> Result<()> {
    let scheme_cf = CFString::from_str(scheme);
    let id = CFString::from_str(bundle_id);
    // SAFETY: both CFStrings are live for the call; the function reads them
    // and keeps no reference. Deprecated in favour of an NSWorkspace
    // completion-handler API, but it is the documented way to trigger the
    // consent prompt, and still supported.
    #[allow(deprecated)]
    let status = unsafe { LSSetDefaultHandlerForURLScheme(&scheme_cf, &id) };
    if status != 0 {
        bail!("macOS refused the {scheme} handler (OSStatus {status})");
    }
    Ok(())
}

/// The bundle id that handles `scheme` now.
pub fn default_handler(scheme: &str) -> Option<String> {
    let scheme = CFString::from_str(scheme);
    // SAFETY: `scheme` is live for the call; the result follows the Create
    // rule and the binding wraps it in an owned CFRetained.
    #[allow(deprecated)]
    let handler = unsafe { LSCopyDefaultHandlerForURLScheme(&scheme) };
    handler.map(|s| s.to_string())
}
