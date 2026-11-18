//! Local clock labels shared by the app's views.

use std::time::SystemTime;

use jiff::Zoned;

/// A local clock label such as `09:41`.
pub fn clock(at: SystemTime) -> String {
    format(at, "%H:%M")
}

/// A local clock label such as `09:41:08`.
pub fn clock_with_seconds(at: SystemTime) -> String {
    format(at, "%H:%M:%S")
}

fn format(at: SystemTime, format: &str) -> String {
    Zoned::try_from(at)
        .map(|at| at.strftime(format).to_string())
        .unwrap_or_default()
}
