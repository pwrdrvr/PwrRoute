//! The AppKit calls GPUI does not make for us: the Dock icon (the
//! activation policy), the screen under the menu bar item, the screen
//! under the mouse, resizing the tray dropdown from its bottom edge, and
//! keeping the agent approval panel up while PwrRoute is in the background.
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

/// Where the menu bar item just clicked is.
///
/// tray-icon reports the item in physical pixels, measured from the top-left
/// of the primary screen, scaled by the item's own screen. GPUI opens windows
/// relative to a display's top-left, in points. `by_pointer` says the pointer
/// clicked the item (not `click_here`'s launch-time click), so where the
/// pointer is now says where the item is.
pub fn status_item_anchor(
    position: (f64, f64),
    item_size: (f64, f64),
    by_pointer: bool,
) -> Option<StatusItemAnchor> {
    let mtm = MainThreadMarker::new()?;
    let screens: Vec<ScreenRects> = NSScreen::screens(mtm)
        .iter()
        .filter_map(|screen| {
            Some(ScreenRects {
                display: screen_display(&screen)?,
                frame: screen.frame(),
                visible: screen.visibleFrame(),
                scale: screen.backingScaleFactor(),
            })
        })
        .collect();
    let pointer = by_pointer.then(|| {
        let mouse = NSEvent::mouseLocation();
        (mouse.x, mouse.y)
    });
    anchor_on(&screens, position, item_size, pointer)
}

/// A screen as AppKit reports it, in AppKit's global space: bottom-left
/// origin at the primary screen's corner, in points.
#[derive(Clone, Copy, Debug)]
struct ScreenRects {
    display: DisplayId,
    frame: NSRect,
    visible: NSRect,
    scale: f64,
}

/// [`status_item_anchor`] apart from AppKit. `screens` start with the
/// primary one (the menu bar's, at the origin); `position` and `item_size`
/// are tray-icon's; `pointer` is where the pointer is, in AppKit's space.
fn anchor_on(
    screens: &[ScreenRects],
    position: (f64, f64),
    item_size: (f64, f64),
    _pointer: Option<(f64, f64)>,
) -> Option<StatusItemAnchor> {
    let primary_height = screens.first()?.frame.size.height;
    for screen in screens {
        let scale = screen.scale;
        let frame = screen.frame;
        let left = frame.origin.x;
        let top = primary_height - (frame.origin.y + frame.size.height);
        let (x, y) = (position.0 / scale, position.1 / scale);
        let inside = (left..left + frame.size.width).contains(&x)
            && (top..top + frame.size.height).contains(&y);
        if !inside {
            continue;
        }
        return Some(StatusItemAnchor {
            display: screen.display,
            item: Bounds {
                origin: point(px((x - left) as f32), px((y - top) as f32)),
                size: size(
                    px((item_size.0 / scale) as f32),
                    px((item_size.1 / scale) as f32),
                ),
            },
            display_size: size(px(frame.size.width as f32), px(frame.size.height as f32)),
            visible_bottom: px(visible_bottom(frame, screen.visible) as f32),
        });
    }
    None
}

/// The display GPUI knows `screen` by.
fn screen_display(screen: &NSScreen) -> Option<DisplayId> {
    let number = screen
        .deviceDescription()
        .objectForKey(&NSString::from_str("NSScreenNumber"))?
        .downcast::<NSNumber>()
        .ok()?;
    Some(DisplayId::new(u64::from(number.unsignedIntValue())))
}

/// The display under the mouse pointer, as GPUI names displays. `None` off
/// the main thread, or when no screen holds the pointer.
pub fn display_under_mouse() -> Option<DisplayId> {
    let mtm = MainThreadMarker::new()?;
    // Both in AppKit's global space: bottom-left origin, in points.
    let mouse = NSEvent::mouseLocation();
    NSScreen::screens(mtm).iter().find_map(|screen| {
        let frame = screen.frame();
        if !in_frame(
            (
                frame.origin.x,
                frame.origin.y,
                frame.size.width,
                frame.size.height,
            ),
            (mouse.x, mouse.y),
        ) {
            return None;
        }
        screen_display(&screen)
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
    // The approval panel is at the same level; it has a title, the
    // dropdown none.
    let Some(panel) = windows.iter().find(|window| {
        window.level() == POPUP_LEVEL && window.isVisible() && window.title().length() == 0
    }) else {
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

/// Keeps the pop-up panels titled `title` (the agent approval prompts) on
/// screen while PwrRoute is in the background. They open over the app an
/// agent runs in, with PwrRoute inactive; a panel hides when its app
/// deactivates unless told otherwise, so a visit to PwrRoute's own window
/// would hide a prompt still waiting for its answer. `false` off the main
/// thread.
pub fn keep_shown_while_inactive(title: &str) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    for window in NSApplication::sharedApplication(mtm).windows().iter() {
        if window.level() == POPUP_LEVEL && window.title().to_string() == title {
            window.setHidesOnDeactivate(false);
        }
    }
    true
}

/// How far a dropdown keeps from the menu bar and the display's edges.
const DROPDOWN_MARGIN: f32 = 4.0;

/// Whether `point` is on the screen whose frame is `(x, y, width, height)`,
/// both in AppKit's bottom-left space. As `NSMouseInRect` reads an
/// unflipped rect: the top edge is in and the bottom edge is not, because a
/// pointer pushed against the top of a display (to click its menu bar) sits
/// at exactly `y + height`.
fn in_frame((x, y, width, height): (f64, f64, f64, f64), (px, py): (f64, f64)) -> bool {
    (x..x + width).contains(&px) && py > y && py <= y + height
}

/// Where a `width` × `height` dropdown opens under the item: centered on it,
/// 4pt below the menu bar, kept 4pt inside the display.
pub fn dropdown_origin(anchor: &StatusItemAnchor, width: Pixels) -> Point<Pixels> {
    let item = anchor.item;
    let centered = item.origin.x + item.size.width / 2.0 - width / 2.0;
    let max_x = anchor.display_size.width - width - px(DROPDOWN_MARGIN);
    point(
        centered.clamp(px(DROPDOWN_MARGIN), max_x.max(px(DROPDOWN_MARGIN))),
        item.origin.y + item.size.height + px(DROPDOWN_MARGIN),
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

    fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
        NSRect::new(
            objc2_foundation::NSPoint::new(x, y),
            objc2_foundation::NSSize::new(width, height),
        )
    }

    /// A 1496x967pt built-in display at 2x with a 28pt menu bar and the
    /// Dock hidden, as AppKit reports it.
    fn built_in() -> ScreenRects {
        ScreenRects {
            display: DisplayId::new(1),
            frame: rect(0.0, 0.0, 1496.0, 967.0),
            visible: rect(0.0, 0.0, 1496.0, 939.0),
            scale: 2.0,
        }
    }

    /// The item window AppKit gives a status item on the menu bar,
    /// (515, 938, 63x30) in AppKit's space, as tray-icon reports it.
    const SHOWN: ((f64, f64), (f64, f64)) = ((1030.0, -2.0), (126.0, 60.0));

    /// The item window macOS 27 leaves a status item it draws for the app
    /// (hidden behind the menu bar's "»", or shown from it): 0x0 at
    /// AppKit's origin, the bottom-left of the primary screen. tray-icon
    /// flips that to the primary screen's height.
    const HIDDEN: ((f64, f64), (f64, f64)) = ((0.0, 967.0 * 2.0), (0.0, 0.0));

    /// The pointer on the menu bar, pressing the item, in AppKit's space.
    const ON_THE_MENU_BAR: (f64, f64) = (1258.0, 955.0);

    fn origin_on(
        screens: &[ScreenRects],
        (position, item_size): ((f64, f64), (f64, f64)),
        pointer: Option<(f64, f64)>,
    ) -> Option<(DisplayId, Point<Pixels>)> {
        let anchor = anchor_on(screens, position, item_size, pointer)?;
        Some((anchor.display, dropdown_origin(&anchor, px(300.0))))
    }

    #[test]
    fn an_item_on_the_menu_bar_anchors_the_dropdown() {
        // The pointer on the item (515..578).
        assert_eq!(
            origin_on(&[built_in()], SHOWN, Some((540.0, 955.0))),
            Some((DisplayId::new(1), point(px(396.5), px(33.0))))
        );
    }

    #[test]
    fn an_item_macos_27_draws_itself_opens_under_the_pointer() {
        // Was the fallback: the top-left corner of the primary display.
        assert_eq!(
            origin_on(&[built_in()], HIDDEN, Some(ON_THE_MENU_BAR)),
            Some((DisplayId::new(1), point(px(1108.0), px(32.0))))
        );
    }

    #[test]
    fn an_empty_item_never_anchors_on_the_display_below() {
        // A 1920x1080 display at 1x under the built-in one: the hidden
        // item's flipped corner is on it.
        let below = ScreenRects {
            display: DisplayId::new(2),
            frame: rect(0.0, -1080.0, 1920.0, 1080.0),
            visible: rect(0.0, -1080.0, 1920.0, 1080.0),
            scale: 1.0,
        };
        assert_eq!(
            origin_on(&[built_in(), below], HIDDEN, Some(ON_THE_MENU_BAR)),
            Some((DisplayId::new(1), point(px(1108.0), px(32.0))))
        );
    }

    #[test]
    fn an_item_away_from_the_click_follows_the_pointer() {
        // AppKit's window is still where the item was before macOS moved it.
        assert_eq!(
            origin_on(&[built_in()], SHOWN, Some(ON_THE_MENU_BAR)),
            Some((DisplayId::new(1), point(px(1108.0), px(32.0))))
        );
    }

    #[test]
    fn a_pointer_off_the_menu_bar_keeps_the_item() {
        // A click from the keyboard or VoiceOver: the pointer is anywhere.
        assert_eq!(
            origin_on(&[built_in()], SHOWN, Some((700.0, 400.0))),
            Some((DisplayId::new(1), point(px(396.5), px(33.0))))
        );
    }

    #[test]
    fn with_no_item_and_no_pointer_the_dropdown_opens_top_right() {
        // `click_here` at launch: under the menu bar, at the right edge.
        assert_eq!(
            origin_on(&[built_in()], HIDDEN, None),
            Some((DisplayId::new(1), point(px(1192.0), px(32.0))))
        );
    }

    #[test]
    fn an_auto_hidden_menu_bar_still_opens_below_it() {
        // The visible frame reaches the top of the display.
        let screen = ScreenRects {
            visible: rect(0.0, 0.0, 1496.0, 967.0),
            ..built_in()
        };
        assert_eq!(
            origin_on(&[screen], HIDDEN, Some((1258.0, 960.0))),
            Some((DisplayId::new(1), point(px(1108.0), px(28.0))))
        );
    }

    #[test]
    fn the_pointer_at_the_top_of_a_display_is_on_it() {
        // The built-in display, with a taller one to its right.
        let built_in = (0.0, 0.0, 1496.0, 967.0);
        let external = (1496.0, 0.0, 2560.0, 1440.0);
        // Pushed against the external display's menu bar.
        assert!(in_frame(external, (2000.0, 1440.0)));
        assert!(!in_frame(built_in, (2000.0, 1440.0)));
        // On the edge the two share: the display to the right.
        assert!(in_frame(external, (1496.0, 500.0)));
        assert!(!in_frame(built_in, (1496.0, 500.0)));
        // The bottom edge belongs to the display below, if any.
        assert!(!in_frame(built_in, (100.0, 0.0)));
    }
}
