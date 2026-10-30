//! What Help's own windows share: the window options, one window of each
//! kind (a second open brings it forward), and the title strip with its
//! "Help › …" crumb (the family's auxiliary title bar).

use crate::theme::px;
use gpui_kit::{
    App, AppContext as _, Bounds, Context, Div, Entity, ParentElement as _, Render, Styled as _,
    TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions, point,
};

use crate::menu_bar::MenuBar;
use crate::theme::{self, text};

/// Where AppKit puts the traffic lights: the family's (16, 13), as the main
/// and Settings windows.
const TRAFFIC_LIGHTS: (f32, f32) = (16.0, 13.0);

/// A window's title, size and least size.
pub struct Frame {
    pub title: &'static str,
    pub size: (f32, f32),
    pub min: (f32, f32),
}

/// Brings `existing` forward if it is still open; `true` when it was.
pub fn activate<V: 'static>(existing: Option<WindowHandle<V>>, cx: &mut App) -> bool {
    let Some(handle) = existing else {
        return false;
    };
    let shown = handle
        .update(cx, |_, window, cx| {
            crate::menu_bar::bring_forward(window, cx)
        })
        .is_ok();
    if shown {
        cx.activate(true);
    }
    shown
}

/// Opens a window on the display the user is on (`placement`), in front.
pub fn open<V: Render + 'static>(
    frame: &Frame,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut Context<'_, V>) -> V,
) -> Option<WindowHandle<V>> {
    let display = crate::placement::display_for_new_window(cx);
    let bounds = Bounds::centered(
        display,
        crate::zoom::window_size(display, frame.size, cx),
        cx,
    );
    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            display_id: display,
            titlebar: Some(TitlebarOptions {
                title: Some(frame.title.into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(TRAFFIC_LIGHTS.0), px(TRAFFIC_LIGHTS.1))),
            }),
            app_id: Some("pwrroute".to_owned()),
            window_min_size: Some(crate::zoom::min_size(display, frame.min, cx)),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| build(window, cx)),
    );
    match handle {
        Ok(handle) => {
            cx.activate(true);
            Some(handle)
        }
        Err(error) => {
            log::warn!("could not open the {} window: {error}", frame.title);
            None
        }
    }
}

/// The strip: brand › Help › `title`. Whatever follows (buttons) goes in
/// as children, after a spacer.
pub fn strip(title: &'static str, menu_bar: &Entity<MenuBar>, window: &Window, cx: &App) -> Div {
    crate::chrome::strip(menu_bar, window, cx)
        .child(text("\u{203a}", 14.0, 500.0, theme::text_subtle()))
        .child(text("Help", 13.0, 500.0, theme::text_muted()))
        .child(text("\u{203a}", 14.0, 500.0, theme::text_subtle()))
        .child(text(title, 13.0, 500.0, theme::text_secondary()))
        .child(gpui_kit::div().flex_1())
}
