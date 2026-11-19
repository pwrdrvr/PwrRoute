//! Windows is the authority for its currently selected time zone and DST rules.
#![allow(unsafe_code)]

use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows_sys::Win32::System::Time::{
    DYNAMIC_TIME_ZONE_INFORMATION, FileTimeToSystemTime, GetDynamicTimeZoneInformation,
    SystemTimeToFileTime, SystemTimeToTzSpecificLocalTimeEx, TIME_ZONE_ID_INVALID,
};

/// The local offset east of UTC, in seconds, at the given Unix second.
pub fn utc_offset(unix_seconds: i64) -> Option<i32> {
    const UNIX_EPOCH_FILE_SECONDS: i64 = 11_644_473_600;
    const TICKS_PER_SECOND: i64 = 10_000_000;

    let seconds = unix_seconds.checked_add(UNIX_EPOCH_FILE_SECONDS)?;
    let ticks = u64::try_from(seconds.checked_mul(TICKS_PER_SECOND)?).ok()?;
    let utc_file = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut zone = DYNAMIC_TIME_ZONE_INFORMATION::default();
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    let mut local_file = FILETIME::default();

    // Treat the local calendar fields as UTC only to measure the difference.
    // SAFETY: every input and output points to an initialized Win32 struct.
    // Each succeeding call uses output only after checking for success.
    let converted = unsafe {
        GetDynamicTimeZoneInformation(&mut zone) != TIME_ZONE_ID_INVALID
            && FileTimeToSystemTime(&utc_file, &mut utc) != 0
            && SystemTimeToTzSpecificLocalTimeEx(&zone, &utc, &mut local) != 0
            && SystemTimeToFileTime(&local, &mut local_file) != 0
    };
    if !converted {
        return None;
    }
    let local_ticks =
        (u64::from(local_file.dwHighDateTime) << 32) | u64::from(local_file.dwLowDateTime);
    let local_seconds = i64::try_from(local_ticks).ok()? / TICKS_PER_SECOND;
    i32::try_from(local_seconds - seconds).ok()
}
