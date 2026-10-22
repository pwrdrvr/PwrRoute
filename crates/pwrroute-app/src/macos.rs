//! The AppKit calls GPUI does not make for us: the Dock icon (the
//! activation policy), the screen under the menu bar item, the screen
//! under the mouse, and resizing the tray dropdown from its bottom edge.
//!
//! GPUI sets `NSApplicationActivationPolicyRegular` when the app finishes
//! launching and never changes it, so a resident router would keep a Dock
//! icon with no window behind it. objc2's AppKit bindings for these calls
//! are safe, so this module needs no `unsafe`; keep any AppKit the app shell
//! needs here, beside them.

use gpui_kit::{Bounds, DisplayId, Pixels, Point, Size, point, px, size};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSEvent, NSScreen};
use objc2_foundation::NSRect;
use objc2_foundation::{NSNumber, NSString};

/// Show or hide the Dock icon (and the app's menu bar while it is active).
///
/// `Regular` while a PwrRoute window is open, `Accessory` otherwise. Off the
/// main thread this does nothing: AppKit requires it there.
pub fn set_dock_icon(visible: bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let policy = if visible {
        NSApplicationActivationPolicy::Regular
    } else {
        NSApplicationActivationPolicy::Accessory
    };
    NSApplication::sharedApplication(mtm).setActivationPolicy(policy);
}

/// Where the menu bar item is, in the coordinates GPUI opens windows in.
#[derive(Clone, Copy, Debug)]
pub struct StatusItemAnchor {
    pub display: DisplayId,
    /// The item, relative to its display's top-left corner, in points.
    pub item: Bounds<Pixels>,
    /// The display's size in points.
    pub display_size: Size<Pixels>,
    /// The bottom of the display's visible area (above the Dock), from the
    /// display's top, in points.
    pub visible_bottom: Pixels,
}

/// The display a tray-icon rect is on, and the rect relative to it.
///
/// tray-icon reports the item in physical pixels, measured from the top-left
/// of the primary screen, scaled by the item's own screen. GPUI opens windows
/// relative to a display's top-left, in points.
pub fn status_item_anchor(position: (f64, f64), item_size: (f64, f64)) -> Option<StatusItemAnchor> {
    let mtm = MainThreadMarker::new()?;
    let screens = NSScreen::screens(mtm);
    // The first screen is the one with the menu bar, at the origin of
    // AppKit's bottom-left-origin global space.
    let primary_height = screens.firstObject()?.frame().size.height;
    for screen in screens.iter() {
        let scale = screen.backingScaleFactor();
        let frame = screen.frame();
        let left = frame.origin.x;
        let top = primary_height - (frame.origin.y + frame.size.height);
        let (x, y) = (position.0 / scale, position.1 / scale);
        let inside = (left..left + frame.size.width).contains(&x)
            && (top..top + frame.size.height).contains(&y);
        if !inside {
            continue;
        }
        let number = screen
            .deviceDescription()
            .objectForKey(&NSString::from_str("NSScreenNumber"))?
            .downcast::<NSNumber>()
            .ok()?;
        return Some(StatusItemAnchor {
            display: DisplayId::new(u64::from(number.unsignedIntValue())),
            item: Bounds {
                origin: point(px((x - left) as f32), px((y - top) as f32)),
                size: size(
                    px((item_size.0 / scale) as f32),
                    px((item_size.1 / scale) as f32),
                ),
            },
            display_size: size(px(frame.size.width as f32), px(frame.size.height as f32)),
            visible_bottom: px(visible_bottom(frame, screen.visibleFrame()) as f32),
        });
    }
    None
}

/// The display under the mouse pointer, as GPUI names displays. `None` off
/// the main thread, or when no screen holds the pointer.
pub fn display_under_mouse() -> Option<DisplayId> {
    let mtm = MainThreadMarker::new()?;
    // Both in AppKit's global space: bottom-left origin, in points.
    let mouse = NSEvent::mouseLocation();
    NSScreen::screens(mtm).iter().find_map(|screen| {
        let frame = screen.frame();
        let inside = (frame.origin.x..frame.origin.x + frame.size.width).contains(&mouse.x)
            && (frame.origin.y..frame.origin.y + frame.size.height).contains(&mouse.y);
        if !inside {
            return None;
        }
        let number = screen
            .deviceDescription()
            .objectForKey(&NSString::from_str("NSScreenNumber"))?
            .downcast::<NSNumber>()
            .ok()?;
        Some(DisplayId::new(u64::from(number.unsignedIntValue())))
    })
}

/// The visible frame's bottom edge, measured down from the screen's top.
/// Both rects in AppKit's bottom-left-origin space.
fn visible_bottom(frame: NSRect, visible: NSRect) -> f64 {
    frame.origin.y + frame.size.height - visible.origin.y
}

/// The tallest a dropdown opening at `origin` may be: down to the visible
/// area's bottom (the Dock's top), less the same 4pt margin.
pub fn dropdown_max_height(anchor: &StatusItemAnchor, origin: Point<Pixels>) -> Pixels {
    (anchor.visible_bottom - origin.y - px(DROPDOWN_MARGIN)).max(px(0.0))
}

/// The dropdown's `NSPanel` level, `NSPopUpMenuWindowLevel`. No other
/// PwrRoute window opens at it.
const POPUP_LEVEL: isize = 101;

/// Make the open dropdown's content `height` points tall and keep its top
/// edge where it is. GPUI's `Window::resize` keeps the bottom edge, which
/// would push the card up under the menu bar. The frame changes by the
/// difference from the content view's height now, so a repeated call is a
/// no-op whatever the frame adds around the content. `false` when no
/// dropdown is open.
pub fn set_dropdown_height(height: f32) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let windows = NSApplication::sharedApplication(mtm).windows();
    let Some(panel) = windows
        .iter()
        .find(|window| window.level() == POPUP_LEVEL && window.isVisible())
    else {
        return false;
    };
    let Some(content) = panel.contentView() else {
        return false;
    };
    let delta = f64::from(height) - content.frame().size.height;
    if delta.abs() < 0.5 {
        return true;
    }
    let mut frame = panel.frame();
    // Bottom-left origin: the top stays at origin.y + height.
    frame.origin.y -= delta;
    frame.size.height += delta;
    panel.setFrame_display(frame, true);
    panel.invalidateShadow();
    true
}

/// How far a dropdown keeps from the menu bar and the display's edges.
const DROPDOWN_MARGIN: f32 = 4.0;

/// Where a `width` × `height` dropdown opens under the item: centered on it,
/// 4pt below the menu bar, kept 4pt inside the display.
pub fn dropdown_origin(anchor: &StatusItemAnchor, width: Pixels) -> Point<Pixels> {
    const MARGIN: f32 = DROPDOWN_MARGIN;
    let item = anchor.item;
    let centered = item.origin.x + item.size.width / 2.0 - width / 2.0;
    let max_x = anchor.display_size.width - width - px(MARGIN);
    point(
        centered.clamp(px(MARGIN), max_x.max(px(MARGIN))),
        item.origin.y + item.size.height + px(MARGIN),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor(x: f32, display_width: f32) -> StatusItemAnchor {
        StatusItemAnchor {
            display: DisplayId::new(1),
            item: Bounds {
                origin: point(px(x), px(0.0)),
                size: size(px(22.0), px(24.0)),
            },
            display_size: size(px(display_width), px(900.0)),
            visible_bottom: px(830.0),
        }
    }

    #[test]
    fn the_dropdown_stops_above_the_dock() {
        let anchor = anchor(1000.0, 1440.0);
        let origin = dropdown_origin(&anchor, px(300.0));
        // 28pt down, 830pt visible, 4pt margin.
        assert_eq!(dropdown_max_height(&anchor, origin), px(798.0));
    }

    #[test]
    fn the_visible_bottom_is_measured_from_the_top() {
        // A 900pt display at the origin with a 70pt Dock.
        let frame = NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            objc2_foundation::NSSize::new(1440.0, 900.0),
        );
        let visible = NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 70.0),
            objc2_foundation::NSSize::new(1440.0, 806.0),
        );
        assert_eq!(visible_bottom(frame, visible), 830.0);
    }

    #[test]
    fn the_dropdown_centers_under_the_item() {
        let origin = dropdown_origin(&anchor(1000.0, 1440.0), px(300.0));
        assert_eq!(origin, point(px(861.0), px(28.0)));
    }

    #[test]
    fn the_dropdown_stays_on_the_display() {
        let right = dropdown_origin(&anchor(1420.0, 1440.0), px(300.0));
        assert_eq!(right.x, px(1136.0));
        let left = dropdown_origin(&anchor(0.0, 1440.0), px(300.0));
        assert_eq!(left.x, px(4.0));
    }
}
