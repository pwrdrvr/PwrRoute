//! Where a new window opens: on the display the user is working on, as
//! macOS places a new window, rather than on the primary display GPUI
//! falls back to when it is given none.
//!
//! The display, in order:
//! 1. the one PwrRoute's focused window is on: Settings opened from the
//!    main window, the importer from the Rules screen;
//! 2. the one under the pointer: the menu bar item just clicked (each
//!    display has its own), or the app a link or an agent's request came
//!    from;
//! 3. none, and GPUI's primary display.
//!
//! A window that is already open stays where the user left it; this only
//! places new ones. Every opener passes the display both to
//! `Bounds::centered` and as `WindowOptions::display_id`: GPUI reads the
//! bounds relative to that display, and opens on the primary one without
//! it.

use gpui_kit::{App, DisplayId};

/// The display a new PwrRoute window should open on, or `None` for the
/// primary display.
pub fn display_for_new_window(cx: &mut App) -> Option<DisplayId> {
    let focused = cx.active_window().and_then(|window| {
        // Err while that window is the one being updated (a click in it
        // opened this one): the pointer is on it then.
        window
            .update(cx, |_, window, cx| window.display(cx).map(|d| d.id()))
            .ok()
            .flatten()
    });
    let connected: Vec<DisplayId> = cx.displays().iter().map(|display| display.id()).collect();
    choose(focused, under_pointer(), &connected)
}

/// [`display_for_new_window`]'s rule, apart from the platform: the focused
/// window's display, else the pointer's, skipping any that is no longer
/// connected.
fn choose(
    focused: Option<DisplayId>,
    pointer: Option<DisplayId>,
    connected: &[DisplayId],
) -> Option<DisplayId> {
    [focused, pointer]
        .into_iter()
        .flatten()
        .find(|display| connected.contains(display))
}

#[cfg(target_os = "macos")]
fn under_pointer() -> Option<DisplayId> {
    crate::macos::display_under_mouse()
}

/// Windows and Linux: GPUI has no pointer position outside a window, so a
/// new window follows the focused one or opens on the primary display.
#[cfg(not(target_os = "macos"))]
fn under_pointer() -> Option<DisplayId> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> DisplayId {
        DisplayId::new(n)
    }

    #[test]
    fn the_focused_window_wins_then_the_pointer() {
        let both = [id(1), id(2)];
        assert_eq!(choose(Some(id(2)), Some(id(1)), &both), Some(id(2)));
        // No focused window (the tray, a link, an agent): the pointer's.
        assert_eq!(choose(None, Some(id(2)), &both), Some(id(2)));
        // Neither: GPUI's primary display.
        assert_eq!(choose(None, None, &both), None);
    }

    #[test]
    fn a_display_no_longer_connected_is_skipped() {
        let one = [id(1)];
        assert_eq!(choose(Some(id(2)), Some(id(1)), &one), Some(id(1)));
        assert_eq!(choose(Some(id(2)), Some(id(3)), &one), None);
    }
}
