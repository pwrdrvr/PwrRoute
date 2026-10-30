//! View › Zoom (PWR-14; design: Contrast and Scale 3).
//!
//! The model is an in-app zoom, as a browser's: Actual Size, Zoom In and
//! Zoom Out (⌘0, ⌘+ and ⌘−; Ctrl off macOS) step through
//! `theme::ZOOM_STEPS`, 100% to 200%, and every window follows at once.
//! macOS gives AppKit apps no text size to follow, so the zoom is the one
//! control on every OS. It is kept in `settings.json` (`view.zoom`, in
//! percent) and read before the first window opens.
//!
//! Every view sizes through `theme::px`, so the zoom scales text, boxes,
//! gaps and rules together, the way a page zooms. A window already open
//! keeps its size; one opened later starts at its design size zoomed, as
//! large as the display allows ([`window_size`]).

use gpui_kit::{App, DisplayId, Pixels, Size, size};

use crate::theme;

/// The zoom in force: one of `theme::ZOOM_STEPS`.
pub fn current() -> f32 {
    theme::zoom()
}

/// Steps the zoom by `by` (1 in, -1 out), or back to 100% with `None`, then
/// saves it and redraws every window.
pub fn step(by: Option<i32>, cx: &mut App) {
    let to = match by {
        Some(by) => theme::zoom_step(current(), by),
        None => 1.0,
    };
    if (to - current()).abs() < f32::EPSILON {
        return;
    }
    set(to, cx);
}

/// Sets the zoom, saves it to `settings.json` and redraws every window.
pub fn set(zoom: f32, cx: &mut App) {
    theme::set_zoom(zoom);
    let percent = percent(theme::zoom());
    if let Some(store) = crate::settings_store::store(cx) {
        store.update(cx, |store, _| {
            if let Err(error) = store.update(|settings| settings.view.zoom = percent) {
                log::warn!("View › Zoom is not saved: {error:#}");
            }
        });
    }
    cx.refresh_windows();
}

/// `zoom` as `settings.json` keeps it.
pub fn percent(zoom: f32) -> u16 {
    // ZOOM_STEPS are 100 to 200: always in range.
    (theme::snap_zoom(zoom) * 100.0).round() as u16
}

/// A window's design size, zoomed, but never larger than `display`'s
/// visible area (the primary display's when `None`): at 200% a window
/// drawn for 1240 points would not fit a laptop.
pub fn window_size(display: Option<DisplayId>, css: (f32, f32), cx: &App) -> Size<Pixels> {
    let wanted = size(theme::px(css.0), theme::px(css.1));
    let area = display
        .and_then(|id| cx.find_display(id))
        .or_else(|| cx.primary_display())
        .map(|display| display.visible_bounds().size);
    match area {
        Some(area) => fit(wanted, area),
        None => wanted,
    }
}

/// `wanted`, no larger than `area` on either side.
fn fit(wanted: Size<Pixels>, area: Size<Pixels>) -> Size<Pixels> {
    size(wanted.width.min(area.width), wanted.height.min(area.height))
}

#[cfg(test)]
mod tests {
    use gpui_kit::px;

    use super::*;

    #[test]
    fn a_zoomed_window_fits_the_display() {
        let area = size(px(1512.0), px(944.0));
        assert_eq!(
            fit(size(px(2480.0), px(1640.0)), area),
            size(px(1512.0), px(944.0))
        );
        assert_eq!(
            fit(size(px(1240.0), px(820.0)), area),
            size(px(1240.0), px(820.0))
        );
    }

    #[test]
    fn percent_is_a_step() {
        assert_eq!(percent(1.0), 100);
        assert_eq!(percent(1.25), 125);
        assert_eq!(percent(1.3), 125);
        assert_eq!(percent(9.0), 200);
    }
}
