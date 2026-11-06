//! The scheduler needs time awake, while Windows' Instant/QPC includes sleep.
#![allow(unsafe_code)]

use std::time::Duration;

/// Monotonic time since boot, excluding sleep and hibernation (Windows 10+).
pub fn now() -> Duration {
    // SAFETY: this Win32 query has no caller-supplied resources or thread
    // restrictions. The windows binding supplies the valid output pointer.
    let ticks =
        unsafe { windows::Win32::System::WindowsProgramming::QueryUnbiasedInterruptTimePrecise() };
    // Each tick is 100 ns. Multiply the Duration instead of u64 ticks so
    // the conversion cannot overflow for a long-running system.
    Duration::from_nanos(ticks) * 100
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_native_awake_clock_is_readable_and_monotonic() {
        let before = super::now();
        let after = super::now();
        assert!(after >= before);
        assert_eq!(
            crate::system_awake_time().map(|now| now >= after),
            Some(true)
        );
    }
}
