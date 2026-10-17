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
mod chrome;
mod editor;
mod fixtures;
mod router;
mod rules_view;
#[cfg(test)]
mod tests;
mod theme;
mod widgets;

use std::{
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
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

/// How long a macOS launch, or a reopen, waits for an Apple Event before it
/// opens the window: a launch for a link must not show it.
const LINK_GRACE: Duration = Duration::from_millis(250);

/// LaunchServices can send a reopen beside a link's Apple Event, before or
/// after it. A reopen this close to a link is part of that link, not a
/// request for the window (as the PwrFinicky router coalesced them).
const REOPEN_AFTER_LINK: Duration = Duration::from_millis(750);

/// When the OS last handed the app a link.
#[derive(Clone, Default)]
struct LastLink(Arc<Mutex<Option<Instant>>>);

impl LastLink {
    fn mark(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
    }

    fn within(&self, window: Duration) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some_and(|at| at.elapsed() < window)
    }

    fn ever(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }
}

/// Keeps the router alive for the life of the app.
struct ResidentRouter(#[allow(dead_code, reason = "held for its Drop")] Resident);

impl Global for ResidentRouter {}

fn main() {
    let args = router::Args::from_env();
    let resident = match router::start(&args) {
        router::Start::Exit => return,
        router::Start::Resident(resident) => resident,
    };
    let handle = resident.handle();
    let last_link = LastLink::default();
    if resident.launched_with_links {
        last_link.mark();
    }

    let app = gpui_kit::application()
        .with_assets(assets::Assets)
        // The router owns dispatch, so the process outlives its windows. A
        // second launch, the Dock icon or the tray brings the window back.
        .with_quit_mode(QuitMode::Explicit);
    let seen = last_link.clone();
    app.on_open_urls(move |urls| {
        seen.mark();
        router::dispatch_os_links(&handle, urls);
    });
    let reopened = last_link.clone();
    app.on_reopen(move |cx| {
        let last_link = reopened.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(LINK_GRACE).await;
            if !last_link.within(REOPEN_AFTER_LINK) {
                cx.update(show_main_window);
            }
        })
        .detach();
    });
    app.run(move |cx| {
        assets::load_fonts(cx);
        app_menu::install(cx);

        let activate = resident.activate.clone();
        cx.spawn(async move |cx| {
            while activate.recv().await.is_ok() {
                cx.update(show_main_window);
            }
        })
        .detach();
        cx.set_global(ResidentRouter(resident));

        if last_link.ever() {
            return;
        }
        if cfg!(target_os = "macos") {
            // A link that launched the app arrives as an Apple Event around
            // now; give it a moment before deciding this launch is for the
            // window.
            cx.spawn(async move |cx| {
                cx.background_executor().timer(LINK_GRACE).await;
                if !last_link.ever() {
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
