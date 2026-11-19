//! Windows: apps' icons, the suspend-excluding clock and local time.

mod app_icon;
mod awake_time;
mod local_time;

pub use app_icon::{icon_png as app_icon_png, installed_app as installed_app_bundle};
pub use awake_time::now as awake_time;
pub use local_time::utc_offset as local_utc_offset;
