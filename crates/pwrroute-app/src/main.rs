//! `pwrroute`: PwrRoute's resident router and its desktop window.
//!
//! The first launch becomes the router (see [`router`]) and stays running
//! with every window closed; later launches hand it their links and exit.
//! The window draws "Rules with editor" from the design review over contrived
//! fixture rules. Nothing here makes PwrRoute the default browser: that is an
//! explicit action (`pwrroute_os::set_default_browser`).

// A window, not a console program, when built for Windows.
#![windows_subsystem = "windows"]

mod app;
mod app_menu;
mod assets;
#[cfg(target_os = "windows")]
mod browser_registration;
mod chrome;
mod editor;
mod fixtures;
mod router;
mod rules_view;
#[cfg(test)]
mod tests;
mod theme;
mod updater;
mod widgets;

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use gpui_kit::{
    App, AppContext as _, Bounds, Global, QuitMode, TitlebarOptions, WindowBounds, WindowOptions,
    point, px, size,
};

use crate::{app::PwrRoute, router::Resident};

/// The window's default size: the design's 1240px width.
const WINDOW_SIZE: (f32, f32) = (1240.0, 820.0);

/// Where AppKit puts the close button, from the window's top-left. The family
/// spec: 16px in, centered in the 40px strip.
const TRAFFIC_LIGHTS: (f32, f32) = (16.0, 13.0);

/// How long a macOS launch waits for an Apple Event before it opens the
/// window: a cold launch for a link must not show it.
const LINK_GRACE: Duration = Duration::from_millis(250);

/// Keeps the router alive for the life of the app.
struct ResidentRouter(#[allow(dead_code, reason = "held for its Drop")] Resident);

impl Global for ResidentRouter {}

fn main() {
    // First: Velopack's install, update and uninstall hooks run here and
    // exit, before this process could take the router lock.
    updater::run_startup_hooks();
    let args = router::Args::from_env();
    let resident = match router::start(&args) {
        router::Start::Exit => return,
        router::Start::Resident(resident) => resident,
    };
    let handle = resident.handle();
    let got_link = Arc::new(AtomicBool::new(resident.launched_with_links));

    let app = gpui_kit::application()
        .with_assets(assets::Assets)
        // The router owns dispatch, so the process outlives its windows. A
        // second launch, the Dock icon or the tray brings the window back.
        .with_quit_mode(QuitMode::Explicit);
    let seen = Arc::clone(&got_link);
    app.on_open_urls(move |urls| {
        seen.store(true, Ordering::Release);
        router::dispatch_os_links(&handle, urls);
    });
    app.on_reopen(show_main_window);
    app.run(move |cx| {
        assets::load_fonts(cx);
        app_menu::install(cx);
        updater::check_in_background(updater::Trigger::Startup);

        let activate = resident.activate.clone();
        cx.spawn(async move |cx| {
            while activate.recv().await.is_ok() {
                cx.update(show_main_window);
            }
        })
        .detach();
        cx.set_global(ResidentRouter(resident));

        if got_link.load(Ordering::Acquire) {
            return;
        }
        if cfg!(target_os = "macos") {
            // A link that launched the app arrives as an Apple Event around
            // now; give it a moment before deciding this launch is for the
            // window.
            cx.spawn(async move |cx| {
                cx.background_executor().timer(LINK_GRACE).await;
                if !got_link.load(Ordering::Acquire) {
                    cx.update(show_main_window);
                }
            })
            .detach();
        } else {
            show_main_window(cx);
        }
    });
}

/// Brings the main window forward, opening it if it is closed.
fn show_main_window(cx: &mut App) {
    if let Some(window) = cx.windows().first() {
        let _ = window.update(cx, |_, window, _| window.activate_window());
        cx.activate(true);
        return;
    }
    let bounds = Bounds::centered(None, size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)), cx);
    let window = cx
        .open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("PwrRoute".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(TRAFFIC_LIGHTS.0), px(TRAFFIC_LIGHTS.1))),
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
}
