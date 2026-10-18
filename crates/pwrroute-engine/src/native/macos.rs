//! macOS: CoreGraphics, IOKit, SystemConfiguration and AppKit.
//!
//! The Go host asked the same frameworks (its `info.m`). The C functions are
//! declared here rather than through binding crates because there are few of
//! them and each is used one way.
// FFI into Apple's C frameworks needs `unsafe`. Each block's safety comment
// says why the call is sound; nothing else in the crate may use `unsafe`.
#![allow(unsafe_code)]

use std::ffi::{CStr, c_char, c_void};

use objc2_app_kit::NSWorkspace;

use crate::{ModifierKeys, PowerInfo};

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFArrayRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFIndex = isize;

const UTF8: u32 = 0x0800_0100; // kCFStringEncodingUTF8
const CF_NUMBER_INT: CFIndex = 9; // kCFNumberIntType
const COMBINED_SESSION_STATE: i32 = 0; // kCGEventSourceStateCombinedSessionState

// CGEventFlags masks (CGEventTypes.h); NSEvent.modifierFlags uses the same.
const FLAG_CAPS_LOCK: u64 = 0x0001_0000;
const FLAG_SHIFT: u64 = 0x0002_0000;
const FLAG_CONTROL: u64 = 0x0004_0000;
const FLAG_OPTION: u64 = 0x0008_0000;
const FLAG_COMMAND: u64 = 0x0010_0000;
const FLAG_FN: u64 = 0x0080_0000;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFBooleanGetTypeID() -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFStringCreateWithCString(
        alloc: CFTypeRef,
        c_str: *const c_char,
        encoding: u32,
    ) -> CFStringRef;
    fn CFStringGetCString(
        string: CFStringRef,
        buffer: *mut c_char,
        size: CFIndex,
        encoding: u32,
    ) -> u8;
    fn CFStringGetLength(string: CFStringRef) -> CFIndex;
    fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: u32) -> CFIndex;
    fn CFArrayGetCount(array: CFArrayRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(array: CFArrayRef, index: CFIndex) -> CFTypeRef;
    fn CFDictionaryGetValue(dict: CFDictionaryRef, key: CFTypeRef) -> CFTypeRef;
    fn CFBooleanGetValue(boolean: CFTypeRef) -> u8;
    fn CFNumberGetValue(number: CFTypeRef, kind: CFIndex, out: *mut c_void) -> u8;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceFlagsState(state: i32) -> u64;
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
    fn IOPSCopyPowerSourcesList(blob: CFTypeRef) -> CFArrayRef;
    fn IOPSGetPowerSourceDescription(blob: CFTypeRef, source: CFTypeRef) -> CFDictionaryRef;
}

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCDynamicStoreCopyComputerName(store: CFTypeRef, encoding: *mut u32) -> CFStringRef;
    fn SCDynamicStoreCopyLocalHostName(store: CFTypeRef) -> CFStringRef;
}

/// A CF object this code owns (from a Create or Copy function), released on
/// drop.
struct Owned(CFTypeRef);

impl Owned {
    fn new(cf: CFTypeRef) -> Option<Self> {
        (!cf.is_null()).then_some(Self(cf))
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: `self.0` is non-null and was returned by a Create or Copy
        // function, so this code holds one reference to release.
        unsafe { CFRelease(self.0) }
    }
}

/// A CFString for a dictionary key.
fn cf_string(text: &CStr) -> Option<Owned> {
    // SAFETY: `text` is a valid NUL-terminated C string; a null allocator
    // means the default one.
    Owned::new(unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), UTF8) })
}

/// A CFString's contents, when `cf` is one.
fn to_string(cf: CFTypeRef) -> Option<String> {
    // SAFETY: `cf` is a valid CF object (checked non-null); the type check
    // makes the CFString calls valid, and the buffer is as large as the
    // encoded string can be, plus the NUL.
    unsafe {
        if cf.is_null() || CFGetTypeID(cf) != CFStringGetTypeID() {
            return None;
        }
        let size = CFStringGetMaximumSizeForEncoding(CFStringGetLength(cf), UTF8) + 1;
        let mut buffer = vec![0 as c_char; usize::try_from(size).ok()?];
        if CFStringGetCString(cf, buffer.as_mut_ptr(), size, UTF8) == 0 {
            return None;
        }
        Some(
            CStr::from_ptr(buffer.as_ptr())
                .to_string_lossy()
                .into_owned(),
        )
    }
}

/// `dict[key]`, borrowed from the dictionary.
fn get(dict: CFDictionaryRef, key: &CStr) -> CFTypeRef {
    let Some(key) = cf_string(key) else {
        return std::ptr::null();
    };
    // SAFETY: `dict` is a valid CFDictionary and `key` a valid CFString; the
    // result follows the Get rule (borrowed, may be null).
    unsafe { CFDictionaryGetValue(dict, key.0) }
}

fn get_bool(dict: CFDictionaryRef, key: &CStr) -> Option<bool> {
    let value = get(dict, key);
    // SAFETY: `value` is checked to be a CFBoolean before it is read.
    unsafe {
        (!value.is_null() && CFGetTypeID(value) == CFBooleanGetTypeID())
            .then(|| CFBooleanGetValue(value) != 0)
    }
}

fn get_int(dict: CFDictionaryRef, key: &CStr) -> Option<i32> {
    let value = get(dict, key);
    let mut out: i32 = 0;
    // SAFETY: `value` is checked to be a CFNumber, and `out` is an i32 as
    // kCFNumberIntType requires.
    unsafe {
        (!value.is_null()
            && CFGetTypeID(value) == CFNumberGetTypeID()
            && CFNumberGetValue(value, CF_NUMBER_INT, (&raw mut out).cast()) != 0)
            .then_some(out)
    }
}

pub(super) fn modifier_keys() -> ModifierKeys {
    // SAFETY: a plain query with no pointers; safe from any thread.
    let flags = unsafe { CGEventSourceFlagsState(COMBINED_SESSION_STATE) };
    let fn_key = flags & FLAG_FN != 0;
    ModifierKeys {
        shift: flags & FLAG_SHIFT != 0,
        option: flags & FLAG_OPTION != 0,
        command: flags & FLAG_COMMAND != 0,
        control: flags & FLAG_CONTROL != 0,
        caps_lock: flags & FLAG_CAPS_LOCK != 0,
        fn_key,
        function_key: fn_key,
    }
}

/// `(name, localizedName)`: the Bonjour host name, as `NSHost.name` gives it,
/// and the computer name from Sharing settings. Neither does a DNS lookup,
/// which `NSHost` can.
pub(super) fn computer_names() -> (String, String) {
    // SAFETY: a null store means a temporary session; a null encoding
    // pointer is allowed. Each result is a Copy and is released by `Owned`.
    let (local, computer) = unsafe {
        (
            Owned::new(SCDynamicStoreCopyLocalHostName(std::ptr::null())),
            Owned::new(SCDynamicStoreCopyComputerName(
                std::ptr::null(),
                std::ptr::null_mut(),
            )),
        )
    };
    let computer = computer.and_then(|c| to_string(c.0)).unwrap_or_default();
    let name = local
        .and_then(|l| to_string(l.0))
        .map(|l| format!("{l}.local"))
        .unwrap_or_else(|| computer.clone());
    (name, computer)
}

/// The internal battery, read as the Go host read it: with no battery, both
/// flags are false and the percentage is unknown.
pub(super) fn power_info() -> PowerInfo {
    let mut info = PowerInfo {
        is_charging: Some(false),
        is_connected: Some(false),
        percentage: None,
    };
    // SAFETY: both are Copy functions; `Owned` releases their results. The
    // list's elements are borrowed handles valid while `blob` is held, and
    // `IOPSGetPowerSourceDescription` returns a borrowed dictionary.
    unsafe {
        let Some(blob) = Owned::new(IOPSCopyPowerSourcesInfo()) else {
            return info;
        };
        let Some(list) = Owned::new(IOPSCopyPowerSourcesList(blob.0)) else {
            return info;
        };
        if CFArrayGetCount(list.0) == 0 {
            return info;
        }
        let source = CFArrayGetValueAtIndex(list.0, 0);
        let dict = IOPSGetPowerSourceDescription(blob.0, source);
        if dict.is_null() || to_string(get(dict, c"Type")).as_deref() != Some("InternalBattery") {
            return info;
        }
        if let Some(charging) = get_bool(dict, c"Is Charging") {
            info.is_charging = Some(charging);
        }
        info.is_connected =
            Some(to_string(get(dict, c"Power Source State")).as_deref() == Some("AC Power"));
        if let Some(current) = get_int(dict, c"Current Capacity") {
            let max = get_int(dict, c"Max Capacity")
                .filter(|m| *m > 0)
                .unwrap_or(100);
            info.percentage = Some((f64::from(current) * 100.0 / f64::from(max)).round());
        }
    }
    info
}

/// Whether a running app has this bundle id or localized name.
pub(super) fn is_app_running(identifier: &str) -> bool {
    objc2::rc::autoreleasepool(|_| {
        let apps = NSWorkspace::sharedWorkspace().runningApplications();
        apps.iter().any(|app| {
            app.bundleIdentifier()
                .is_some_and(|id| id.to_string() == identifier)
                || app
                    .localizedName()
                    .is_some_and(|name| name.to_string() == identifier)
        })
    })
}
