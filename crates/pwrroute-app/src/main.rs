//! `pwrroute`: PwrRoute's resident router and its desktop window.
//!
//! The first launch becomes the router (see [`router`]) and stays running
//! with every window closed; later launches hand it their links and exit.
//! The window draws "Rules with editor" from the design review over contrived
//! fixture rules. Nothing here makes PwrRoute the default browser: that is an
//! explicit action, a click in first run or Settings that
//! [`default_browser`] turns into `pwrroute_os::set_default_browser`.

// A window, not a console program, when built for Windows.
#![windows_subsystem = "windows"]

mod activity;
mod app;
mod app_menu;
mod assets;
#[cfg(target_os = "windows")]
mod browser_registration;
mod chrome;
mod config_view;
mod default_browser;
mod dock;
mod editor;
mod first_run;
mod fixtures;
#[cfg(target_os = "macos")]
mod macos;
mod onboarding;
mod router;
mod rules_model;
mod rules_store;
mod rules_view;
mod settings;
mod test_link;
#[cfg(test)]
mod tests;
mod text_field;
mod theme;
mod trace_view;
mod tray;
mod try_it;
mod updater;
mod widgets;

use std::{
    rc::Rc,
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
    // First: Velopack's install, update and uninstall hooks run here and
    // exit, before this process could take the router lock.
    updater::run_startup_hooks();
    let args = router::Args::from_env();
    let resident = match router::start(&args) {
        router::Start::Exit => return,
        router::Start::Resident(resident) => resident,
    };
    let handle = resident.handle();
    // `--first-run` forces the window; otherwise it opens once, on the first
    // launch that is for the window rather than a link, until it is done or
    // closed.
    let first_run = args.first_run
        || pwrroute_os::data_dir().is_some_and(|dir| onboarding::State::pending(&dir));
    let forced_first_run = args.first_run;
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
        dock::install(cx);
        // Before the tray, which follows the feed from the start.
        activity::install(resident.handle(), resident.dispatched.clone(), cx);
        tray::install(cx);
        default_browser::install(default_browser::Os::real(), cx);
        cx.set_global(settings::SettingsHost {
            model: Rc::new(|window, cx| {
                default_browser::watch(window, cx);
                let mut model = settings::fixtures::model();
                model.browsers.default = default_browser::card(cx);
                model
            }),
            on_request: Rc::new(|request, _, cx| {
                if !default_browser::handle_settings_request(request, cx) {
                    eprintln!("pwrroute: settings: {request:?} is not wired up yet");
                }
            }),
        });
        updater::check_in_background(updater::Trigger::Startup);

        let activate = resident.activate.clone();
        cx.spawn(async move |cx| {
            while activate.recv().await.is_ok() {
                cx.update(show_main_window);
            }
        })
        .detach();
        cx.set_global(ResidentRouter(resident));

        if forced_first_run {
            onboarding::open(cx);
            return;
        }
        if last_link.ever() {
            return;
        }
        let show = move |cx: &mut App| {
            if first_run {
                onboarding::open(cx);
            } else {
                show_main_window(cx);
            }
        };
        if cfg!(target_os = "macos") {
            // A link that launched the app arrives as an Apple Event around
            // now; give it a moment before deciding this launch is for the
            // window.
            cx.spawn(async move |cx| {
                cx.background_executor().timer(LINK_GRACE).await;
                if !last_link.ever() {
                    cx.update(show);
                }
            })
            .detach();
        } else {
            show(cx);
        }
    });
}

/// Brings the main window forward, opening it if it is closed.
fn show_main_window(cx: &mut App) {
    // Not `first()`: the Settings window, the tray dropdown or the first-run
    // window may be the only one open.
    let main = cx
        .windows()
        .into_iter()
        .find(|window| window.downcast::<PwrRoute>().is_some());
    if let Some(window) = main {
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
            |window, cx| {
                // The strip's default-browser chip follows the OS's answer.
                default_browser::watch(window, cx);
                cx.new(PwrRoute::new)
            },
        )
        .expect("open the PwrRoute window");
    // The rules list owns the keyboard from the first frame.
    let _ = window.update(cx, |this, window, cx| {
        let focus = this.focus.clone();
        window.focus(&focus, cx);
    });
    cx.activate(true);
}
