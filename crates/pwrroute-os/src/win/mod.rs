//! Windows: apps' icons and the suspend-excluding clock.

mod app_icon;
mod awake_time;

pub use app_icon::{icon_png as app_icon_png, installed_app as installed_app_bundle};
pub use awake_time::now as awake_time;
