//! `pwrroute`: PwrRoute's desktop window.
//!
//! Draws the main window, "Rules with editor" from the design review, over
//! contrived fixture rules. It does not route links yet and never registers
//! itself as the default browser.

// A window, not a console program, when built for Windows.
#![windows_subsystem = "windows"]

mod app;
mod app_menu;
mod assets;
mod chrome;
mod editor;
mod fixtures;
mod rules_view;
#[cfg(test)]
mod tests;
mod theme;
mod widgets;

use gpui_kit::{
    AppContext as _, Bounds, QuitMode, TitlebarOptions, WindowBounds, WindowOptions, point, px,
    size,
};

use crate::app::PwrRoute;

/// The window's default size: the design's 1240px width.
const WINDOW_SIZE: (f32, f32) = (1240.0, 820.0);

/// Where AppKit puts the close button, from the window's top-left. The family
/// spec: 16px in, centered in the 40px strip.
const TRAFFIC_LIGHTS: (f32, f32) = (16.0, 13.0);

fn main() {
    gpui_kit::application()
        .with_assets(assets::Assets)
        // Until the menu-bar item exists there is no way back to a closed
        // window, so closing it quits. The router will need `Explicit`
        // (the macOS default), which keeps the process alive windowless.
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(|cx| {
            assets::load_fonts(cx);
            app_menu::install(cx);
            let bounds = Bounds::centered(None, size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)), cx);
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        titlebar: Some(TitlebarOptions {
                            title: Some("PwrRoute".into()),
                            appears_transparent: true,
                            traffic_light_position: Some(point(
                                px(TRAFFIC_LIGHTS.0),
                                px(TRAFFIC_LIGHTS.1),
                            )),
                        }),
                        app_id: Some("pwrroute".to_owned()),
                        window_min_size: Some(size(px(1000.0), px(600.0))),
                        ..Default::default()
                    },
                    |_, cx| cx.new(PwrRoute::new),
                )
                .expect("open the PwrRoute window");
            // The rules list owns the keyboard from the first frame.
            let _ = window.update(cx, |this, window, cx| {
                let focus = this.focus.clone();
                window.focus(&focus, cx);
            });
            cx.activate(true);
        });
}
