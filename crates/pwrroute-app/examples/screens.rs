//! Renders each screen of the window to a PNG with GPUI's headless Metal
//! renderer, at the window's default size, over the fixture data: for
//! comparing with the design and for PR screenshots.
//!
//! ```text
//! cargo run -p pwrroute-app --example screens -- OUT_DIR
//! ```
//!
//! macOS only: the headless renderer is Metal, and GPUI's Mac platform must
//! be created on the main thread, which a libtest test never runs on. The
//! app is a binary, so this example compiles the window's modules itself.

#![allow(
    dead_code,
    reason = "the app's modules, compiled here for their views only"
)]

#[cfg(target_os = "macos")]
#[path = "../src/activity.rs"]
mod activity;
#[cfg(target_os = "macos")]
#[path = "../src/app.rs"]
mod app;
#[cfg(target_os = "macos")]
#[path = "../src/assets.rs"]
mod assets;
#[cfg(target_os = "macos")]
#[path = "../src/chrome.rs"]
mod chrome;
#[cfg(target_os = "macos")]
#[path = "../src/config_view.rs"]
mod config_view;
#[cfg(target_os = "macos")]
#[path = "../src/default_browser.rs"]
mod default_browser;
#[cfg(target_os = "macos")]
#[path = "../src/editor.rs"]
mod editor;
#[cfg(target_os = "macos")]
#[path = "../src/fixtures.rs"]
mod fixtures;
#[cfg(target_os = "macos")]
#[path = "../src/router.rs"]
mod router;
#[cfg(target_os = "macos")]
#[path = "../src/rules_model.rs"]
mod rules_model;
#[cfg(target_os = "macos")]
#[path = "../src/rules_store.rs"]
mod rules_store;
#[cfg(target_os = "macos")]
#[path = "../src/rules_view.rs"]
mod rules_view;
#[cfg(target_os = "macos")]
#[path = "../src/settings/mod.rs"]
mod settings;
#[cfg(target_os = "macos")]
#[path = "../src/test_link.rs"]
mod test_link;
#[cfg(target_os = "macos")]
#[path = "../src/text_field.rs"]
mod text_field;
#[cfg(target_os = "macos")]
#[path = "../src/theme.rs"]
mod theme;
#[cfg(target_os = "macos")]
#[path = "../src/trace_view.rs"]
mod trace_view;
#[cfg(target_os = "macos")]
#[path = "../src/try_it.rs"]
mod try_it;
#[cfg(target_os = "macos")]
#[path = "../src/widgets.rs"]
mod widgets;

#[cfg(target_os = "macos")]
fn main() {
    use std::{path::PathBuf, sync::Arc, time::Duration};

    use gpui_kit::{AppContext as _, Context, HeadlessAppContext, Window, px, size};

    use crate::app::{PwrRoute, Screen, Selection};

    /// The window's default size, as `main` opens it.
    const WINDOW_SIZE: (f32, f32) = (1240.0, 820.0);

    let Some(dir) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: screens OUT_DIR");
        std::process::exit(2);
    };
    std::fs::create_dir_all(&dir).expect("create the output directory");
    let home = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/home");
    let config = home.join(".finicky.ts");

    type Prepare = dyn Fn(&mut PwrRoute, &mut Window, &mut Context<'_, PwrRoute>);
    let shoot = |name: &str, config: Option<PathBuf>, prepare: &Prepare| {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(assets::Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| assets::load_fonts(cx));
        let home = home.clone();
        // The design's rules, as if read from the real file.
        let store = rules_store::RulesStore::from_file(
            Some(home.join("Library/Application Support/PwrRoute/rules.json")),
            fixtures::rules_file(),
            true,
            Some(std::time::SystemTime::now()),
        );
        let handle = cx
            .open_window(size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)), |_, cx| {
                cx.new(|cx| PwrRoute::with_store(config, Some(home), store, cx))
            })
            .expect("open a headless window");
        cx.update_window(handle.into(), |view, window, cx| {
            let view = view.downcast::<PwrRoute>().expect("the PwrRoute view");
            view.update(cx, |app, cx| prepare(app, window, cx));
        })
        .expect("prepare the window");
        // Icons load as assets: draw until they have arrived.
        cx.allow_parking();
        for _ in 0..3 {
            cx.run_until_parked();
            cx.update_window(handle.into(), |_, window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
            })
            .expect("draw");
            std::thread::sleep(Duration::from_millis(50));
        }
        let image = cx
            .capture_screenshot(handle.into())
            .expect("render with Metal");
        let path = dir.join(format!("{name}.png"));
        image.save(&path).expect("write the PNG");
        println!("{}", path.display());
    };

    shoot("app-1a", None, &|_, _, _| {});
    shoot("app-1a-rewrite", None, &|app, _, cx| {
        app.select(Selection::Rewrite(0), cx);
    });
    shoot("app-1b", Some(config), &|_, _, _| {});
    shoot("app-2a", None, &|app, window, cx| {
        // The design's example; the window itself starts empty.
        app.test.url.set(fixtures::TEST_URL);
        app.test.opener = fixtures::TEST_OPENER;
        app.navigate(Screen::TestLink, window, cx);
    });
    shoot("app-2a-empty", None, &|app, window, cx| {
        app.navigate(Screen::TestLink, window, cx);
    });
    shoot("app-2a-picker", None, &|app, window, cx| {
        app.navigate(Screen::TestLink, window, cx);
        app.test.picking = true;
    });
    shoot("app-2b", None, &|app, window, cx| {
        app.activity.load(fixtures::activity());
        app.activity.expanded = Some(app.activity.rows[1].id);
        app.navigate(Screen::Activity, window, cx);
    });
    shoot("app-2b-empty", None, &|app, window, cx| {
        app.navigate(Screen::Activity, window, cx);
    });

    // Settings, at each option's `.dv-card` size in headless Chrome less
    // its 1px border (`design/PwrRoute Settings.dc.html`).
    for (name, pane, height) in [
        ("settings-1a", settings::Pane::LocalAgents, 1603.0),
        ("settings-1b", settings::Pane::Configuration, 1315.0),
        ("settings-1c", settings::Pane::Browsers, 880.0),
    ] {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(assets::Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            assets::load_fonts(cx);
            settings::install(cx);
        });
        let handle = cx
            .open_window(size(px(1058.0), px(height)), |_, cx| {
                cx.new(|cx| {
                    let mut view = settings::SettingsWindow::new(
                        settings::fixtures::model(),
                        std::rc::Rc::new(|_, _, _| {}),
                        cx,
                    );
                    view.pane = pane;
                    view
                })
            })
            .expect("open a headless window");
        cx.allow_parking();
        for _ in 0..3 {
            cx.run_until_parked();
            cx.update_window(handle.into(), |_, window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
            })
            .expect("draw");
            std::thread::sleep(Duration::from_millis(50));
        }
        let image = cx
            .capture_screenshot(handle.into())
            .expect("render with Metal");
        let path = dir.join(format!("{name}.png"));
        image.save(&path).expect("write the PNG");
        println!("{}", path.display());
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("screens: macOS only; GPUI's headless renderer is Metal");
}
