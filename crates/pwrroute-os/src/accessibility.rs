//! Read the system's contrast preference. Linux is read asynchronously by
//! the app through the desktop portal, which also supplies change events.

#[cfg(target_os = "macos")]
pub fn increase_contrast() -> Option<bool> {
    Some(objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldIncreaseContrast())
}

#[cfg(target_os = "windows")]
#[allow(
    unsafe_code,
    reason = "SPI_GETHIGHCONTRAST requires a typed Win32 output pointer"
)]
pub fn increase_contrast() -> Option<bool> {
    use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SPI_GETHIGHCONTRAST, SystemParametersInfoW};
    let mut state = HIGHCONTRASTW {
        cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
        dwFlags: 0,
        lpszDefaultScheme: std::ptr::null_mut(),
    };
    // SAFETY: SPI_GETHIGHCONTRAST writes a HIGHCONTRASTW to this valid,
    // correctly sized pointer. No scheme string is dereferenced or retained.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            state.cbSize,
            (&mut state as *mut HIGHCONTRASTW).cast(),
            0,
        )
    };
    (ok != 0).then_some(state.dwFlags & HCF_HIGHCONTRASTON != 0)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn increase_contrast() -> Option<bool> {
    None
}
