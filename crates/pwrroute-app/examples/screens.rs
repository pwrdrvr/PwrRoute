//! Renders each screen of the window to a PNG with GPUI's headless Metal
//! renderer, at the window's default size, over the fixture data: for
//! comparing with the design and for PR screenshots.
//!
//! ```text
//! cargo run -p pwrroute-app --example screens -- OUT_DIR [--app-icons]
//! ```
//!
//! The badges are letters, so every machine draws the same PNGs.
//! `--app-icons` draws the apps' own icons instead, from what this Mac has
//! installed (`app_icons.rs`): an app the fixtures name that is not
//! installed keeps its letters.
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
#[path = "../src/activity_actions.rs"]
mod activity_actions;
#[cfg(target_os = "macos")]
#[path = "../src/app.rs"]
mod app;
#[cfg(target_os = "macos")]
#[path = "../src/app_icons.rs"]
mod app_icons;
#[cfg(target_os = "macos")]
#[path = "../src/app_menu.rs"]
mod app_menu;
#[cfg(target_os = "macos")]
#[path = "../src/assets.rs"]
mod assets;
#[cfg(target_os = "macos")]
#[path = "../src/browser_scan.rs"]
mod browser_scan;
#[cfg(target_os = "macos")]
#[path = "../src/chrome.rs"]
mod chrome;
#[cfg(target_os = "macos")]
#[path = "../src/config_view.rs"]
mod config_view;
#[cfg(target_os = "macos")]
#[path = "../src/default_browser.rs"]
mod default_browser;

#[path = "../src/destinations.rs"]
mod destinations;
#[cfg(target_os = "macos")]
#[path = "../src/editor.rs"]
mod editor;
/// The test hook's probes, inert: the real hook needs the app's menu and
/// `main`, which this example does not compile.
#[cfg(target_os = "macos")]
mod e2e {
    use gpui_kit::{App, Bounds, Pixels, Window};

    pub const SOCKET_ENV: &str = "PWRROUTE_E2E_SOCKET";
    pub const ROUTER_DIR_ENV: &str = "PWRROUTE_E2E_ROUTER_DIR";
    pub const DRY_RUN_ENV: &str = "PWRROUTE_E2E_DRY_RUN";
    pub const CONFIG_ENV: &str = "PWRROUTE_E2E_CONFIG";

    pub fn regions(
        _: &'static [&'static str],
    ) -> impl Fn(Vec<Bounds<Pixels>>, &mut Window, &mut App) + 'static {
        |_, _, _| {}
    }

    pub fn frame(_: &Window) {}

    pub fn requested() -> bool {
        false
    }
}
/// New windows on the primary display: the example's windows render
/// offscreen, so it skips the real rule, which follows the pointer.
#[cfg(target_os = "macos")]
mod placement {
    use gpui_kit::{App, DisplayId};

    pub fn display_for_new_window(_: &mut App) -> Option<DisplayId> {
        None
    }
}
#[cfg(target_os = "macos")]
#[path = "../src/first_run.rs"]
mod first_run;
#[cfg(target_os = "macos")]
#[path = "../src/fixtures.rs"]
mod fixtures;
#[cfg(target_os = "macos")]
#[path = "../src/importer.rs"]
mod importer;
#[cfg(target_os = "macos")]
#[path = "../src/importer_view.rs"]
mod importer_view;
#[cfg(target_os = "macos")]
#[path = "../src/macos.rs"]
mod macos;
#[cfg(target_os = "macos")]
#[path = "../src/menu_bar.rs"]
mod menu_bar;
#[cfg(target_os = "macos")]
#[path = "../src/overview.rs"]
mod overview;
#[cfg(target_os = "macos")]
#[path = "../src/overview_model.rs"]
mod overview_model;
#[cfg(target_os = "macos")]
#[path = "../src/router.rs"]
mod router;
#[cfg(target_os = "macos")]
#[path = "../src/rule_pickers.rs"]
mod rule_pickers;
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
#[path = "../src/settings_store.rs"]
mod settings_store;
#[cfg(target_os = "macos")]
#[path = "../src/start_from.rs"]
mod start_from;
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
// The first-run window's browser labels and the tray dropdown, where they
// look for them. The dropdown reaches its siblings through `super`, here the
// crate root: its commands do nothing, since it is only drawn.
#[cfg(target_os = "macos")]
#[path = "../src/tray/dropdown.rs"]
mod tray_dropdown;
#[cfg(target_os = "macos")]
#[path = "../src/tray/model.rs"]
mod tray_model;
#[cfg(target_os = "macos")]
mod tray {
    pub(crate) use crate::tray_dropdown as dropdown;
    pub(crate) use crate::tray_model as model;
    // Overview's Resume routing, and its pause state: no tray here.
    pub(crate) use crate::{TrayCommand, run};

    pub fn model(_: &gpui_kit::App) -> Option<gpui_kit::Entity<crate::tray_model::TrayModel>> {
        None
    }

    /// View's rows: nothing to show here.
    pub fn show_screen(_: crate::app::Screen, _: &mut gpui_kit::App) {}
}
#[cfg(target_os = "macos")]
use tray_model as model;

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrayCommand {
    TestLink,
    Rules,
    Activity,
    Settings,
    TogglePause,
    MakeDefault,
    Quit,
}

#[cfg(target_os = "macos")]
fn toggle_pause(_: &gpui_kit::Entity<tray_model::TrayModel>, _: &mut gpui_kit::App) {}

#[cfg(target_os = "macos")]
fn dropdown_closed(_: &mut gpui_kit::App) {}

#[cfg(target_os = "macos")]
fn run(_: TrayCommand, _: &mut gpui_kit::App) {}

#[cfg(target_os = "macos")]
#[path = "../src/aux_window.rs"]
mod aux_window;
#[cfg(target_os = "macos")]
#[path = "../src/diag_log.rs"]
mod diag_log;
#[cfg(target_os = "macos")]
#[path = "../src/doc_window.rs"]
mod doc_window;
#[cfg(target_os = "macos")]
#[path = "../src/log_view.rs"]
mod log_view;
#[cfg(target_os = "macos")]
#[path = "../src/logs_window.rs"]
mod logs_window;
/// The Logs window's Copy header, contrived as everything drawn here, and
/// Help's actions for the strip's menus, which do nothing here.
#[cfg(target_os = "macos")]
mod help {
    #![allow(
        clippy::derive_partial_eq_without_eq,
        reason = "GPUI's actions! macro writes the derives, not us"
    )]

    gpui_kit::actions!(
        pwrroute,
        [
            OpenDocumentation,
            OpenChangelog,
            ReplayOnboarding,
            ReportIssue,
            ReportVulnerability,
            CopyDiagnostics,
            OpenLogs,
            ViewSource,
            ViewLicense,
            ThirdPartyNotices,
        ]
    );

    pub fn install(_: &mut gpui_kit::App) {}

    pub fn diagnostics_report(_: &mut gpui_kit::App) -> String {
        "PwrRoute 0.2.0 \u{00b7} release \u{00b7} arm64\n".to_owned()
    }
}
#[cfg(target_os = "macos")]
#[path = "../src/github_releases.rs"]
mod github_releases;
#[cfg(target_os = "macos")]
#[path = "../src/handoff.rs"]
mod handoff;
#[cfg(target_os = "macos")]
#[path = "../src/update_channel.rs"]
mod update_channel;
#[cfg(target_os = "macos")]
#[path = "../src/updater.rs"]
mod updater;

/// As `main.rs` places the traffic lights, for the first-run window.
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHTS: (f32, f32) = (16.0, 13.0);
#[cfg(target_os = "macos")]
#[path = "../src/widgets.rs"]
mod widgets;

/// `--app-icons`: draw the apps' own icons, not their letters.
#[cfg(target_os = "macos")]
static APP_ICONS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "macos")]
thread_local! {
    /// The strip to draw instead of macOS's: Windows' or Linux's, with
    /// their menu bar and Ctrl shortcuts.
    static CHROME: std::cell::Cell<Option<chrome::Chrome>> = const { std::cell::Cell::new(None) };
}

/// Each window's fonts, the apps' icons when asked for, and the menus and
/// shortcuts of the strip being drawn.
#[cfg(target_os = "macos")]
fn setup(cx: &mut gpui_kit::App) {
    assets::load_fonts(cx);
    if APP_ICONS.load(std::sync::atomic::Ordering::Relaxed) {
        app_icons::install(cx);
    }
    if let Some(chrome) = CHROME.get() {
        app_menu::install_for(chrome, cx);
    }
}

/// How many frames to draw before the capture: icons load off the UI
/// thread, a few at a time.
#[cfg(target_os = "macos")]
fn draws() -> usize {
    if APP_ICONS.load(std::sync::atomic::Ordering::Relaxed) {
        10
    } else {
        3
    }
}

#[cfg(target_os = "macos")]
fn main() {
    use std::{path::PathBuf, sync::Arc, time::Duration};

    use gpui_kit::{AppContext as _, Context, HeadlessAppContext, Window, px, size};

    use crate::app::{PwrRoute, Screen, Selection};

    /// The window's default size, as `main` opens it.
    const WINDOW_SIZE: (f32, f32) = (1240.0, 820.0);

    let (flags, dirs): (Vec<_>, Vec<_>) = std::env::args_os()
        .skip(1)
        .partition(|arg| arg.to_string_lossy().starts_with("--"));
    APP_ICONS.store(
        flags.iter().any(|flag| flag == "--app-icons"),
        std::sync::atomic::Ordering::Relaxed,
    );
    let Some(dir) = dirs.into_iter().next().map(PathBuf::from) else {
        eprintln!("usage: screens OUT_DIR [--app-icons]");
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
        cx.update(setup);
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
        for _ in 0..draws() {
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

    // The window lands on Overview: these open Rules, as a click does.
    shoot("app-1a", None, &|app, window, cx| {
        app.navigate(Screen::Rules, window, cx);
        app.select(Selection::Handler(0), cx);
    });
    // The Windows and Linux strips (design: Windows and Linux Chrome 1a,
    // 1b): the brand at x=16 and the menu bar after it, Windows' caption
    // buttons, and menus open as Alt and a click leave them.
    for (chrome, name) in [
        (chrome::Chrome::Windows, "chrome-1a-windows"),
        (chrome::Chrome::Linux, "chrome-1b-linux"),
    ] {
        CHROME.set(Some(chrome));
        shoot(name, None, &|app, window, cx| {
            app.navigate(Screen::Rules, window, cx);
            app.select(Selection::Handler(0), cx);
        });
    }
    CHROME.set(Some(chrome::Chrome::Windows));
    let menu = |title: usize, row: Option<usize>, keyboard: bool| {
        move |app: &mut PwrRoute, window: &mut Window, cx: &mut Context<'_, PwrRoute>| {
            app.navigate(Screen::Rules, window, cx);
            app.select(Selection::Handler(0), cx);
            app.menu_bar.update(cx, |bar, cx| {
                bar.state = menu_bar::State {
                    title: Some(title),
                    open: true,
                    row,
                    keyboard,
                };
                cx.notify();
            });
        }
    };
    shoot("chrome-1a-file-menu", None, &menu(0, Some(2), true));
    shoot("chrome-1a-edit-menu", None, &menu(1, None, false));
    shoot("chrome-1a-window-menu", None, &menu(3, Some(0), false));
    shoot("chrome-1a-help-menu", None, &menu(4, Some(7), false));
    CHROME.set(None);
    shoot("app-1a-rewrite", None, &|app, window, cx| {
        app.navigate(Screen::Rules, window, cx);
        app.select(Selection::Rewrite(0), cx);
    });
    shoot("app-1b", Some(config.clone()), &|app, window, cx| {
        app.navigate(Screen::Rules, window, cx);
    });
    // A config that never loaded, on Rules: the banner and the visual card
    // say what the router does until it loads.
    shoot(
        "app-1b-config-error",
        Some(home.join("missing.finicky.ts")),
        &|app, window, cx| {
            if let Some(config) = &mut app.config {
                // The fixture's error, not this machine's path.
                config.path = "~/.finicky.ts".into();
                config.problem = Some(
                    "It did not load: SyntaxError: Unexpected token '}' \u{00b7} line 42".into(),
                );
            }
            app.overview.status = Some(overview_model::Status {
                config_error: Some(overview_model::ConfigTrouble::NotLoaded(
                    "SyntaxError: Unexpected token '}' \u{00b7} line 42".into(),
                )),
                ..overview_model::Status::routing()
            });
            app.navigate(Screen::Rules, window, cx);
        },
    );
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
        app.activity.expanded = Some(app.activity.rows[1].key);
        app.navigate(Screen::Activity, window, cx);
    });
    shoot("app-2b-empty", None, &|app, window, cx| {
        app.navigate(Screen::Activity, window, cx);
    });

    // Examples, saved tests and the Run tests strip, over a copy of the
    // design's rules with contrived examples on them (one wrong on purpose).
    // The store is given a scratch path so a debounced save never reaches
    // the fixture file.
    fn with_examples(app: &mut PwrRoute, cx: &mut Context<'_, PwrRoute>) {
        let mut file = fixtures::rules_file();
        file.rewrites[0]
            .examples
            .push(pwrroute_engine::RewriteExample {
                url: "https://x.com/pwrdrvr/status/1842".into(),
                becomes: "https://xcancel.com/pwrdrvr/status/1842".into(),
            });
        file.rules[0]
            .examples
            .push("https://meet.google.com/abc-defg-hij".into());
        file.rules[1]
            .examples
            .push("https://github.com/pwrdrvr/PwrGit/pull/361".into());
        file.rules[3].examples.extend([
            "https://news.ycombinator.com/item?id=41".to_owned(),
            "https://example.atlassian.net/browse/PG-12".to_owned(),
        ]);
        file.rules[3].private = true;
        file.tests.push(pwrroute_engine::RouteTest {
            url: "https://docs.google.com/document/d/1".into(),
            opener: Some("Slack:com.tinyspeck.slackmacgap".into()),
            expect: pwrroute_engine::Expectation {
                browser: "Firefox".into(),
                profile: Some("default-release".into()),
                url: None,
            },
        });
        app.store = rules_store::RulesStore::from_file(
            Some(std::env::temp_dir().join("pwrroute-screens-rules.json")),
            file,
            true,
            Some(std::time::SystemTime::now()),
        );
        app.refresh(cx);
        app.run_tests(cx);
    }
    shoot("app-1a-examples", None, &|app, window, cx| {
        with_examples(app, cx);
        app.navigate(Screen::Rules, window, cx);
        app.select(Selection::Handler(3), cx);
    });

    // The rule editor's pickers (Rule Editor Pickers 3a-5b) over the
    // Overview's contrived scan. Shots that add a rule get a scratch store,
    // so a debounced save never reaches the fixture file.
    fn with_scan(app: &mut PwrRoute) {
        app.installed = fixtures::overview_browsers();
        app.browsers = app
            .installed
            .iter()
            .map(|app| activity_actions::BrowserChoice {
                name: app.name.clone(),
                profiles: app.profiles.clone(),
            })
            .collect();
    }
    fn scratch_store(app: &mut PwrRoute, cx: &mut Context<'_, PwrRoute>) {
        app.store = rules_store::RulesStore::from_file(
            Some(std::env::temp_dir().join("pwrroute-screens-rules.json")),
            fixtures::rules_file(),
            true,
            Some(std::time::SystemTime::now()),
        );
        app.refresh(cx);
    }
    // Destination and App Pickers 1c, 2a, 2b: the editor's Open in lanes
    // and Sent from's chips, with "+ app" open over recent senders.
    shoot("app-1a-open-in", None, &|app, window, cx| {
        with_scan(app);
        app.navigate(Screen::Rules, window, cx);
        app.select(Selection::Handler(2), cx);
    });
    shoot("app-1a-sent-from-menu", None, &|app, window, cx| {
        with_scan(app);
        activity::install_feed(activity::Feed::from_records(fixtures::activity()), cx);
        app.navigate(Screen::Rules, window, cx);
        app.select(Selection::Handler(2), cx);
        if let Some(rail) = &mut app.rail {
            rail.menu = Some(editor::RailMenu::Opener);
        }
    });
    shoot("app-1a-new-rule", None, &|app, window, cx| {
        with_scan(app);
        scratch_store(app, cx);
        app.navigate(Screen::Rules, window, cx);
        app.add_handler(window, cx);
    });
    shoot(
        "app-1b-new-rule",
        Some(config.clone()),
        &|app, window, cx| {
            with_scan(app);
            scratch_store(app, cx);
            app.navigate(Screen::Rules, window, cx);
            app.add_handler(window, cx);
        },
    );
    // Rule From a Link 1a to 1c: a new rule starts from the last link the
    // design's Activity fixtures opened.
    shoot("app-1a-new-rule-from-link", None, &|app, window, cx| {
        with_scan(app);
        activity::install_feed(activity::Feed::from_records(fixtures::activity()), cx);
        scratch_store(app, cx);
        app.navigate(Screen::Rules, window, cx);
        app.add_handler(window, cx);
    });
    shoot("app-1a-new-rule-start-menu", None, &|app, window, cx| {
        with_scan(app);
        activity::install_feed(activity::Feed::from_records(fixtures::activity()), cx);
        scratch_store(app, cx);
        app.navigate(Screen::Rules, window, cx);
        app.add_handler(window, cx);
        if let Some(rail) = &mut app.rail {
            rail.menu = Some(editor::RailMenu::StartFrom);
        }
    });
    shoot("app-1a-new-rule-cleared", None, &|app, window, cx| {
        with_scan(app);
        activity::install_feed(activity::Feed::from_records(fixtures::activity()), cx);
        scratch_store(app, cx);
        app.navigate(Screen::Rules, window, cx);
        app.add_handler(window, cx);
        app.clear_start(window, cx);
    });
    shoot("app-0-overview-card-add", None, &|app, window, cx| {
        with_scan(app);
        scratch_store(app, cx);
        app.add_rule_for("Microsoft Edge", "", false, window, cx);
    });
    shoot("app-2a-new-rule", None, &|app, window, cx| {
        with_scan(app);
        scratch_store(app, cx);
        app.test.url.set(fixtures::TEST_URL);
        app.navigate(Screen::TestLink, window, cx);
        app.add_rule_from_test(window, cx);
    });

    // Activity's Add a rule sheet for the design's apple.com record, which
    // the default browser took, with contrived browsers to pick from.
    shoot("app-2b-add-rule", None, &|app, window, cx| {
        let records = fixtures::activity();
        activity::install_feed(activity::Feed::from_records(records.clone()), cx);
        app.activity.load(records);
        app.browsers = vec![
            activity_actions::BrowserChoice {
                name: "Google Chrome".into(),
                profiles: vec!["Personal".into(), "Work".into()],
            },
            activity_actions::BrowserChoice {
                name: "Firefox".into(),
                profiles: vec!["default-release".into()],
            },
            activity_actions::BrowserChoice {
                name: "Safari".into(),
                profiles: Vec::new(),
            },
        ];
        app.navigate(Screen::Activity, window, cx);
        app.add_rule(
            activity::RowKey {
                id: pwrroute_router::DispatchId(2),
                journaled: false,
            },
            window,
            cx,
        );
    });

    // Add Rule From an App 1b and 1c: a long contrived sign-in link Slack
    // sent, folded to two lines, then with "Any link from Slack" chosen.
    fn long_sign_in_link(app: &mut PwrRoute, cx: &mut Context<'_, PwrRoute>) {
        let mut records = fixtures::activity();
        let url = "https://auth.example.com/o/oauth2/v2/auth?client_id=4417-demo-client.apps.\
                   example.com&redirect_uri=https%3A%2F%2Ftracker.example%2Fauth%2Fdesktop-\
                   redirect&scope=openid%20email%20profile&access_type=offline&response_type=\
                   code&state=%7B%22key%22%3A%22demo-state-0001%22%2C%22flow%22%3A%22desktop\
                   %22%7D&prompt=consent";
        let mut record = records[0].clone();
        record.id = pwrroute_router::DispatchId(99);
        record.link.url = url.into();
        record.final_url = Some(url.into());
        if let Some(browser) = &mut record.browser {
            browser.url = url.into();
        }
        for step in &mut record.steps {
            match step {
                pwrroute_router::Step::Received { url: received } => *received = url.into(),
                pwrroute_router::Step::Browser { browser } => browser.url = url.into(),
                _ => {}
            }
        }
        records.insert(0, record);
        activity::install_feed(activity::Feed::from_records(records.clone()), cx);
        app.activity.load(records);
        with_scan(app);
    }
    fn open_long(app: &mut PwrRoute, window: &mut Window, cx: &mut Context<'_, PwrRoute>) {
        app.navigate(Screen::Activity, window, cx);
        app.add_rule(
            activity::RowKey {
                id: pwrroute_router::DispatchId(99),
                journaled: false,
            },
            window,
            cx,
        );
    }
    shoot("app-2b-add-rule-long-link", None, &|app, window, cx| {
        long_sign_in_link(app, cx);
        open_long(app, window, cx);
    });
    shoot("app-2b-add-rule-any-from", None, &|app, window, cx| {
        long_sign_in_link(app, cx);
        open_long(app, window, cx);
        app.pick_any_from(cx);
    });

    // The same sheet with the browser list open, over a browser with many
    // profiles and one with a long name (Popovers and Overflow 1a).
    shoot("app-2b-add-rule-browsers", None, &|app, window, cx| {
        let records = fixtures::activity();
        activity::install_feed(activity::Feed::from_records(records.clone()), cx);
        app.activity.load(records);
        app.browsers = vec![
            activity_actions::BrowserChoice {
                name: "Google Chrome".into(),
                profiles: vec!["Personal".into(), "Work".into(), "Design".into()],
            },
            activity_actions::BrowserChoice {
                name: "Microsoft Edge".into(),
                profiles: [
                    "Research and Development Team",
                    "Personal",
                    "Work",
                    "Design",
                    "Spanish",
                ]
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            },
            activity_actions::BrowserChoice {
                name: "Example Browser With A Very Long Product Name".into(),
                profiles: vec!["Default".into(), "Work".into(), "Travel".into()],
            },
            activity_actions::BrowserChoice {
                name: "Safari".into(),
                profiles: Vec::new(),
            },
        ];
        app.navigate(Screen::Activity, window, cx);
        app.add_rule(
            activity::RowKey {
                id: pwrroute_router::DispatchId(2),
                journaled: false,
            },
            window,
            cx,
        );
    });

    // Activity's row menu with a Save hint that names a long profile
    // (Popovers and Overflow 1d).
    shoot("app-2b-row-menu", None, &|app, window, cx| {
        let records = fixtures::activity();
        activity::install_feed(activity::Feed::from_records(records.clone()), cx);
        app.activity.load(records);
        app.navigate(Screen::Activity, window, cx);
        app.actions.menu = Some(activity_actions::RowMenu {
            key: app.activity.rows[2].key,
            save: Some(activity_actions::Save::Test {
                expects: "Microsoft Edge \u{00b7} Research and Development Team".into(),
            }),
            added: None,
        });
    });

    // Activity Quick Add Rule 1a, 1c and 1d: "+ Rule" on the row under the
    // pointer, at rest under Everything else, and the expanded row with a
    // long contrived link, Add a rule… first.
    fn long_link(
        mut records: Vec<pwrroute_router::DispatchRecord>,
    ) -> Vec<pwrroute_router::DispatchRecord> {
        const LONG: &str = "https://auth.example.com/o/oauth2/v2/auth?client_id=4417-demo-client.apps.example.com&redirect_uri=https%3A%2F%2Ftracker.example.com%2Fauth%2Fdesktop-redirect&scope=openid%20email%20profile&access_type=offline&response_type=code&state=%7B%22key%22%3A%22demo-state-0001%22%2C%22flow%22%3A%22desktop%22%7D&prompt=consent";
        // The record PwrRoute's default took (www.apple.com in the fixture).
        let record = &mut records[6];
        record.link.url = LONG.into();
        record.final_url = Some(LONG.into());
        if let Some(browser) = &mut record.browser {
            browser.url = LONG.into();
        }
        for step in &mut record.steps {
            match step {
                pwrroute_router::Step::Received { url } => *url = LONG.into(),
                pwrroute_router::Step::Browser { browser } => browser.url = LONG.into(),
                _ => {}
            }
        }
        records
    }
    shoot("app-2b-quick-rule-hover", None, &|app, window, cx| {
        let records = fixtures::activity();
        activity::install_feed(activity::Feed::from_records(records.clone()), cx);
        app.activity.load(records);
        app.navigate(Screen::Activity, window, cx);
        app.activity.hovered = Some(app.activity.rows[4].key);
    });
    shoot(
        "app-2b-quick-rule-everything-else",
        None,
        &|app, window, cx| {
            let records = fixtures::activity();
            activity::install_feed(activity::Feed::from_records(records.clone()), cx);
            app.activity.load(records);
            app.navigate(Screen::Activity, window, cx);
            app.activity.filter = activity::Filter::Unrouted;
            app.activity.hovered = Some(app.activity.rows[6].key);
        },
    );
    shoot("app-2b-expanded-long-link", None, &|app, window, cx| {
        let records = long_link(fixtures::activity());
        activity::install_feed(activity::Feed::from_records(records.clone()), cx);
        app.activity.load(records);
        app.navigate(Screen::Activity, window, cx);
        app.activity.expanded = Some(app.activity.rows[6].key);
    });

    // The tray dropdown (Tray and First Run 1a; Popovers and Overflow 2a)
    // with the window sized as `dropdown::open` sizes it, over the design's
    // fixture with a rules.json default saved as a bundle id and a recent
    // link to a profile with a long name.
    {
        use tray::model::{BrowserLabel, Recent, RecentRoute, TrayModel};

        let mut model = TrayModel::fixture(std::time::SystemTime::now());
        model.pause_target =
            BrowserLabel::new("Ed", "Microsoft Edge", None).for_app("com.microsoft.edgemac");
        model.recents[0] = Recent::from_urls(
            "https://docs.example.com/a",
            None,
            BrowserLabel::new(
                "Ed",
                "Microsoft Edge",
                Some("Research and Development Team"),
            ),
            RecentRoute::Handler(2),
            std::time::SystemTime::now() - Duration::from_secs(180),
        )
        .expect("a host");
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(assets::Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(setup);
        let height = cx.update(|cx| {
            tray::dropdown::fitted_height(&model, &cx.text_system().clone(), px(f32::MAX))
        });
        let handle = cx
            .open_window(size(px(tray::dropdown::WIDTH), height), |window, cx| {
                let model = cx.new(|_| model);
                cx.new(|cx| {
                    tray::dropdown::Dropdown::new(
                        model,
                        px(f32::MAX),
                        std::rc::Rc::new(|_| {}),
                        window,
                        cx,
                    )
                })
            })
            .expect("open a headless window");
        cx.allow_parking();
        for _ in 0..draws() {
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
        let path = dir.join("tray-1a.png");
        image.save(&path).expect("write the PNG");
        println!("{}", path.display());
    }
    // The same sheet with the Finicky config in charge, for the Slack
    // record: Add found the config's Slack handler takes the link first,
    // and the sheet says so before anything is added.
    shoot(
        "app-2b-add-rule-config-first",
        Some(config.clone()),
        &|app, window, cx| {
            let records = fixtures::activity();
            activity::install_feed(activity::Feed::from_records(records.clone()), cx);
            app.activity.load(records);
            app.navigate(Screen::Activity, window, cx);
            app.add_rule(
                activity::RowKey {
                    id: pwrroute_router::DispatchId(8),
                    journaled: false,
                },
                window,
                cx,
            );
            app.set_add_rule(|sheet| sheet.position = activity_actions::Position::Top, cx);
            app.add_rule_from_sheet(cx);
        },
    );

    // A rule added beside the config, shown in Rules: the visual rules
    // open with the new handler selected.
    shoot(
        "app-2b-add-rule-shown",
        Some(config.clone()),
        &|app, window, cx| {
            let records = fixtures::activity();
            activity::install_feed(activity::Feed::from_records(records.clone()), cx);
            app.activity.load(records);
            app.navigate(Screen::Activity, window, cx);
            app.add_rule(
                activity::RowKey {
                    id: pwrroute_router::DispatchId(4),
                    journaled: false,
                },
                window,
                cx,
            );
            app.set_add_rule(|sheet| sheet.position = activity_actions::Position::Top, cx);
            app.add_rule_from_sheet(cx);
            app.show_added_rule(0, window, cx);
        },
    );

    // First run (Overview and Setup 4a-4g) over contrived browsers: the
    // Everything else step with a profiled browser chosen and its profile
    // menu open, then the default-browser step before and after the OS
    // agrees. The default it names is an app the catalog does not list, by
    // the name its bundle gives.
    {
        use first_run::{BrowserChoice, FinickyConfig, FirstRun, FirstRunHooks, FirstRunModel};
        use tray::model::BrowserLabel;

        let model = |fallback: Option<BrowserLabel>| FirstRunModel {
            fallback,
            browsers: vec![
                BrowserChoice {
                    value: "com.google.Chrome".into(),
                    label: BrowserLabel::new("Ch", "Google Chrome", None),
                    profiles: vec!["Personal".into(), "Work".into()],
                    profiles_blocked: false,
                },
                BrowserChoice {
                    value: "org.mozilla.firefox".into(),
                    label: BrowserLabel::new("Ff", "Firefox", None),
                    profiles: vec!["default-release".into()],
                    profiles_blocked: false,
                },
                BrowserChoice {
                    value: "com.apple.Safari".into(),
                    label: BrowserLabel::new("Sf", "Safari", None),
                    profiles: Vec::new(),
                    profiles_blocked: false,
                },
            ],
            finicky: Some(FinickyConfig {
                path: "~/.finicky.ts".into(),
                rewrites: 2,
                handlers: 5,
                importable: 0,
                default_browser: Some("com.apple.Safari".into()),
                default_name: Some("Safari".into()),
                ..FinickyConfig::default()
            }),
            import: Some(first_run::ImportChoice::KeepFile),
            precedence: pwrroute_engine::Precedence::PwrRoute,
            current_default: Some("PwrFinicky".into()),
            ..FirstRunModel::default()
        };
        let chrome_work = || Some(BrowserLabel::new("Ch", "Google Chrome", Some("Work")));
        let hooks = || FirstRunHooks {
            on_fallback: std::rc::Rc::new(|_, _| Ok(())),
            on_precedence: std::rc::Rc::new(|_, _| Ok(())),
            on_import: std::rc::Rc::new(|_, _, _| {}),
            on_rescan: std::rc::Rc::new(|_| Vec::new()),
            on_open_access_settings: std::rc::Rc::new(|_| Ok(())),
            on_reread: std::rc::Rc::new(|_| Vec::new()),
            on_set_default_browser: std::rc::Rc::new(|_| Ok(())),
            on_finish: std::rc::Rc::new(|_, _| {}),
        };
        let step_3 = model(chrome_work());
        let mut done = step_3.clone();
        done.is_default = true;
        for (name, model, picking) in [
            (
                "first-run-1-profile",
                model(chrome_work()),
                first_run::Picking::Profile,
            ),
            (
                "first-run-1",
                FirstRunModel {
                    import: None,
                    ..model(chrome_work())
                },
                first_run::Picking::None,
            ),
            ("first-run-3", step_3, first_run::Picking::None),
            ("first-run-3-done", done, first_run::Picking::None),
        ] {
            let mut cx = HeadlessAppContext::with_platform(
                gpui_kit::platform::current_platform(true).text_system(),
                Arc::new(assets::Assets),
                gpui_kit::platform::current_headless_renderer,
            );
            // Motion at its end state: the shot is of where it settles.
            cx.update(|cx| {
                setup(cx);
                cx.set_reduce_motion(true);
            });
            let handle = cx
                .open_window(size(px(680.0), px(600.0)), |_, cx| {
                    cx.new(|cx| {
                        let mut view = FirstRun::new(model, hooks(), cx);
                        view.picking = picking;
                        view
                    })
                })
                .expect("open a headless window");
            cx.allow_parking();
            for _ in 0..draws() {
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

    // The Finicky importer (Finicky Importer 2a–3a) over its fixture
    // config, at the design's window size, with the design's two visual
    // handlers for Place.
    let importer_config =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/importer/finicky.ts");
    for (name, step, selected) in [
        ("importer-review", importer::Step::Review, 3),
        ("importer-review-exact", importer::Step::Review, 4),
        ("importer-place", importer::Step::Place, 0),
        ("importer-source", importer::Step::Source, 0),
    ] {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(assets::Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(setup);
        let existing = pwrroute_engine::RulesFile {
            default_browser: "Google Chrome".into(),
            rules: vec![
                pwrroute_engine::Rule {
                    patterns: vec!["localhost:*".into()],
                    browser: "Arc".into(),
                    ..pwrroute_engine::Rule::default()
                },
                pwrroute_engine::Rule {
                    patterns: vec!["bsky.app/*".into()],
                    browser: "Firefox".into(),
                    ..pwrroute_engine::Rule::default()
                },
            ],
            ..pwrroute_engine::RulesFile::default()
        };
        // Read as the config in charge, as the design draws it.
        let mut model = importer::ImportModel::read(
            &importer_config,
            None,
            Some(&importer_config),
            &[],
            existing,
        );
        // Shown as the design names it, not by the fixture's path.
        model.shown = "~/.finicky.ts".into();
        model.selected = selected;
        model.step = step;
        let handle = cx
            .open_window(size(px(1180.0), px(640.0)), |_, cx| {
                cx.new(|cx| importer_view::Importer::new(model, cx))
            })
            .expect("open a headless window");
        cx.allow_parking();
        for _ in 0..draws() {
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

    // Settings, at each option's `.dv-card` size in headless Chrome less
    // its 1px border (`design/PwrRoute Settings.dc.html`).
    for (name, pane, height) in [
        ("settings-1a", settings::Pane::LocalAgents, 1603.0),
        ("settings-1b", settings::Pane::Configuration, 1315.0),
        ("settings-1c", settings::Pane::Browsers, 880.0),
        ("settings-privacy", settings::Pane::Privacy, 1560.0),
        // The family's panes, at a height that holds their cards.
        ("settings-updates", settings::Pane::Updates, 760.0),
        ("settings-diagnostics", settings::Pane::Diagnostics, 900.0),
        ("settings-about", settings::Pane::About, 700.0),
    ] {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(assets::Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            setup(cx);
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
        for _ in 0..draws() {
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

    // Help's windows: the documents as compiled in, and the Logs window
    // over a contrived log (no real link, host or app list), plain, with a
    // selection, and searched.
    let scratch = std::env::temp_dir().join("pwrroute-screens-logs");
    std::fs::create_dir_all(&scratch).expect("a scratch folder");
    let log = scratch.join("main.log");
    std::fs::write(&log, FIXTURE_LOG).expect("write the fixture log");
    let _ = std::fs::remove_file(scratch.join("main.old.log"));
    enum Help {
        Doc(doc_window::Doc),
        /// The Logs window over the fixture log, then this.
        Logs(fn(&mut logs_window::LogsWindow)),
    }
    for (name, which, (width, height)) in [
        (
            "help-changelog",
            Help::Doc(doc_window::Doc::Changelog),
            (920.0, 760.0),
        ),
        (
            "help-license",
            Help::Doc(doc_window::Doc::License),
            (920.0, 760.0),
        ),
        (
            "help-notices",
            Help::Doc(doc_window::Doc::ThirdPartyNotices),
            (920.0, 760.0),
        ),
        // Wrap is on by default: the long link lines read in full.
        ("help-logs", Help::Logs(|_| {}), (1040.0, 760.0)),
        // Off, they run past the edge and scroll sideways.
        (
            "help-logs-nowrap",
            Help::Logs(|logs| logs.wrap = false),
            (1040.0, 760.0),
        ),
        (
            "help-logs-selected",
            Help::Logs(|logs| logs.selection = Some((9, 12))),
            (1040.0, 760.0),
        ),
        // Filter: the matches, gap rows for what is hidden, "1 of 4 lines"
        // and the status bar's keys.
        (
            "help-logs-search",
            Help::Logs(|logs| logs.set_query_for_screens("outcome")),
            (1040.0, 760.0),
        ),
        // Context ±1 around the one error, stepped to.
        (
            "help-logs-context",
            Help::Logs(|logs| {
                logs.view.set_context(1);
                logs.set_query_for_screens("could not");
            }),
            (1040.0, 760.0),
        ),
        // Highlight keeps every line; the current match is solid.
        (
            "help-logs-highlight",
            Help::Logs(|logs| {
                logs.view.set_mode(log_view::SearchMode::Highlight);
                logs.set_query_for_screens("hand-off");
                logs.view.step(true);
            }),
            (1040.0, 760.0),
        ),
        // A mark, the lines logged after it, and Select since mark.
        (
            "help-logs-mark",
            Help::Logs(|logs| {
                logs.set_mark_for_screens("09:20:04");
                logs.view.append(FIXTURE_LOG_AFTER_MARK);
                logs.selection = logs.view.since_mark();
            }),
            (1040.0, 760.0),
        ),
        // Narrow: Wrap and Mark keep their icons and tooltips, here at the
        // window's least width with Context showing.
        (
            "help-logs-narrow",
            Help::Logs(|logs| logs.set_query_for_screens("outcome")),
            (880.0, 640.0),
        ),
    ] {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(assets::Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(setup);
        let window_size = size(px(width), px(height));
        let handle: gpui_kit::AnyWindowHandle = match which {
            Help::Doc(doc) => cx
                .open_window(window_size, move |_, cx| {
                    cx.new(|cx| doc_window::DocWindow::new(doc, cx))
                })
                .expect("open a headless window")
                .into(),
            Help::Logs(prepare) => {
                let log = log.clone();
                cx.open_window(window_size, move |_, cx| {
                    cx.new(|cx| {
                        let mut view = logs_window::LogsWindow::new(Some(log), cx);
                        prepare(&mut view);
                        view
                    })
                })
                .expect("open a headless window")
                .into()
            }
        };
        cx.allow_parking();
        for _ in 0..draws() {
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
            })
            .expect("draw");
            std::thread::sleep(Duration::from_millis(50));
        }
        let image = cx.capture_screenshot(handle).expect("render with Metal");
        let path = dir.join(format!("{name}.png"));
        image.save(&path).expect("write the PNG");
        println!("{}", path.display());
    }

    // Engine precedence (WP-A): with the fixture config in charge, Test a
    // link names who takes everything else under each order.
    fn with_precedence(
        app: &mut PwrRoute,
        precedence: pwrroute_engine::Precedence,
        cx: &mut Context<'_, PwrRoute>,
    ) {
        let file = pwrroute_engine::RulesFile {
            precedence,
            ..fixtures::rules_file()
        };
        app.store = rules_store::RulesStore::from_file(
            Some(std::env::temp_dir().join("pwrroute-screens-precedence.json")),
            file,
            true,
            Some(std::time::SystemTime::now()),
        );
        app.refresh(cx);
    }
    for (name, precedence) in [
        ("app-2a-config-default", pwrroute_engine::Precedence::Config),
        (
            "app-2a-pwrroute-default",
            pwrroute_engine::Precedence::PwrRoute,
        ),
    ] {
        shoot(name, Some(config.clone()), &move |app, window, cx| {
            with_precedence(app, precedence, cx);
            app.test.url.set("https://unmatched.example.com/");
            app.navigate(Screen::TestLink, window, cx);
        });
    }

    // Attribution (WP-B): Activity names who routed each link, "rule 3"
    // for rules.json's rules, "cfg h 2" for the config's handlers, and
    // whose default took the rest; the expanded rows say it in full.
    shoot("app-2b-attribution", None, &|app, window, cx| {
        let records = fixtures::attribution_activity();
        activity::install_feed(activity::Feed::from_records(records.clone()), cx);
        app.activity.load(records);
        app.activity.expanded = Some(app.activity.rows[0].key);
        app.navigate(Screen::Activity, window, cx);
    });
    shoot("app-2b-attribution-unrouted", None, &|app, window, cx| {
        let records = fixtures::attribution_activity();
        activity::install_feed(activity::Feed::from_records(records.clone()), cx);
        app.activity.load(records);
        app.activity.filter = activity::Filter::Unrouted;
        app.activity.expanded = Some(app.activity.rows[4].key);
        app.navigate(Screen::Activity, window, cx);
    });
    shoot("app-2b-attribution-menu", None, &|app, window, cx| {
        let records = fixtures::attribution_activity();
        activity::install_feed(activity::Feed::from_records(records.clone()), cx);
        app.activity.load(records);
        app.navigate(Screen::Activity, window, cx);
        app.actions.menu = Some(activity_actions::RowMenu {
            key: app.activity.rows[0].key,
            save: Some(activity_actions::Save::Example {
                handler: 2,
                pattern: Some("docs.example.com/*".into()),
            }),
            added: None,
        });
    });

    // First run (WP-C): the design's states over its one fixture config
    // (2 rewrites and 5 handlers, 3 of them importable, Safari as its
    // default) and the same three browsers.
    {
        use first_run::{
            BrowserChoice, FinickyConfig, FirstRun, FirstRunHooks, FirstRunModel, ImportChoice,
            Imported, Picked, Picking,
        };
        use pwrroute_engine::Precedence;
        use tray::model::BrowserLabel;

        let browsers = vec![
            BrowserChoice {
                value: "com.google.Chrome".into(),
                label: BrowserLabel::new("Ch", "Google Chrome", None),
                profiles: vec!["Personal".into(), "Work".into()],
                profiles_blocked: false,
            },
            BrowserChoice {
                value: "org.mozilla.firefox".into(),
                label: BrowserLabel::new("Ff", "Firefox", None),
                profiles: vec!["default-release".into()],
                profiles_blocked: false,
            },
            BrowserChoice {
                value: "com.apple.Safari".into(),
                label: BrowserLabel::new("Sf", "Safari", None),
                profiles: Vec::new(),
                profiles_blocked: false,
            },
        ];
        let config = FinickyConfig {
            path: "~/.finicky.ts".into(),
            rewrites: 2,
            handlers: 5,
            importable: 3,
            default_browser: Some("com.apple.Safari".into()),
            default_name: Some("Safari".into()),
            ..FinickyConfig::default()
        };
        let safari = || Some(BrowserLabel::new("Sf", "Safari", None));
        let chrome_personal = || Some(BrowserLabel::new("Ch", "Google Chrome", Some("Personal")));
        // 4a: the config step is current; Everything else is already
        // picked from the config's default, and waits.
        let found = FirstRunModel {
            fallback: safari(),
            picked: Some(Picked::Config),
            browsers: browsers.clone(),
            finicky: Some(config.clone()),
            current_default: Some("Safari".into()),
            ..FirstRunModel::default()
        };
        // 4b: the importer is open.
        let importing = FirstRunModel {
            importing: true,
            ..found.clone()
        };
        // 4c: the config does not load.
        let broken = FirstRunModel {
            finicky: Some(FinickyConfig {
                rewrites: 0,
                handlers: 0,
                importable: 0,
                default_browser: None,
                default_name: None,
                error: Some(
                    "SyntaxError: Unexpected token '}' \u{00b7} ~/.finicky.ts line 42".into(),
                ),
                ..config.clone()
            }),
            fallback: chrome_personal(),
            picked: Some(Picked::OsDefault),
            current_default: Some("Google Chrome".into()),
            ..found.clone()
        };
        // 4d: no config, two steps, picked from the OS default.
        let plain = FirstRunModel {
            finicky: None,
            fallback: chrome_personal(),
            picked: Some(Picked::OsDefault),
            current_default: Some("Google Chrome".into()),
            ..found.clone()
        };
        // 4e: the config kept; its default owns everything else.
        let kept = FirstRunModel {
            import: Some(ImportChoice::KeepFile),
            ..found.clone()
        };
        // 4f: PwrRoute's default owns it instead.
        let pwrroute_default = FirstRunModel {
            precedence: Precedence::PwrRoute,
            fallback: chrome_personal(),
            picked: None,
            ..kept.clone()
        };
        // 4g: routing.
        let done = FirstRunModel {
            is_default: true,
            ..kept.clone()
        };
        // 4g after an import that kept the config, with PwrRoute's default.
        let done_imported = FirstRunModel {
            import: Some(ImportChoice::Import),
            imported: Some(Imported {
                count: 3,
                keeps_config: true,
                rules: 3,
                ..Imported::default()
            }),
            rules: 3,
            ..FirstRunModel {
                is_default: true,
                ..pwrroute_default.clone()
            }
        };
        // 4g with the broken config kept: the rules run without it.
        let done_broken = FirstRunModel {
            is_default: true,
            rules: 3,
            finicky: broken.finicky.clone(),
            fallback: chrome_personal(),
            ..kept.clone()
        };
        // 4h: no browsers found.
        let no_browsers = FirstRunModel {
            browsers: Vec::new(),
            fallback: None,
            picked: None,
            current_default: None,
            ..plain.clone()
        };
        // 4i: the OS refused the request.
        let refused = FirstRunModel {
            default_error: Some("Launch Services returned error -54".into()),
            ..plain.clone()
        };
        // 6a: macOS kept Chrome's and Firefox's profile lists from
        // PwrRoute, so the access step comes first.
        let blocked_browsers: Vec<BrowserChoice> = browsers
            .iter()
            .cloned()
            .map(|mut choice| {
                if !choice.profiles.is_empty() {
                    choice.profiles.clear();
                    choice.profiles_blocked = true;
                }
                choice
            })
            .collect();
        let access_blocked = FirstRunModel {
            browsers: blocked_browsers.clone(),
            fallback: Some(BrowserLabel::new("Ch", "Google Chrome", None)),
            access: Some(first_run::ProfileAccess {
                browsers: browsers[..2].iter().map(|c| c.label.clone()).collect(),
                ..first_run::ProfileAccess::default()
            }),
            ..plain.clone()
        };
        // 6b: Chrome turned on, Firefox still off.
        let access_partial = FirstRunModel {
            browsers: {
                let mut some = blocked_browsers;
                some[0] = browsers[0].clone();
                some
            },
            ..access_blocked.clone()
        };
        // 6c: back from System Settings with both on.
        let access_done = FirstRunModel {
            browsers: browsers.clone(),
            ..access_blocked.clone()
        };
        let hooks = || FirstRunHooks {
            on_fallback: std::rc::Rc::new(|_, _| Ok(())),
            on_precedence: std::rc::Rc::new(|_, _| Ok(())),
            on_import: std::rc::Rc::new(|_, _, _| {}),
            on_rescan: std::rc::Rc::new(|_| Vec::new()),
            on_open_access_settings: std::rc::Rc::new(|_| Ok(())),
            on_reread: std::rc::Rc::new(|_| Vec::new()),
            on_set_default_browser: std::rc::Rc::new(|_| Ok(())),
            on_finish: std::rc::Rc::new(|_, _| {}),
        };
        for (name, model, picking) in [
            ("first-run-config", found, Picking::None),
            ("first-run-importing", importing, Picking::None),
            ("first-run-broken", broken, Picking::None),
            ("first-run-plain", plain, Picking::None),
            ("first-run-everything-else", kept, Picking::None),
            (
                "first-run-browser-menu",
                pwrroute_default.clone(),
                Picking::Browser,
            ),
            (
                "first-run-pwrroute-default",
                pwrroute_default,
                Picking::None,
            ),
            ("first-run-done-summary", done, Picking::None),
            ("first-run-done-imported", done_imported, Picking::None),
            ("first-run-done-broken", done_broken, Picking::None),
            ("first-run-no-browsers", no_browsers, Picking::None),
            ("first-run-default-failed", refused, Picking::None),
            ("first-run-access-blocked", access_blocked, Picking::None),
            ("first-run-access-partial", access_partial, Picking::None),
            ("first-run-access-done", access_done, Picking::None),
        ] {
            // The access step is tall; its shots show the whole window.
            let height = if name.starts_with("first-run-access") {
                860.0
            } else {
                600.0
            };
            let mut cx = HeadlessAppContext::with_platform(
                gpui_kit::platform::current_platform(true).text_system(),
                Arc::new(assets::Assets),
                gpui_kit::platform::current_headless_renderer,
            );
            // Motion at its end state: the shot is of where it settles.
            cx.update(|cx| {
                setup(cx);
                cx.set_reduce_motion(true);
            });
            let handle = cx
                .open_window(size(px(680.0), px(height)), |_, cx| {
                    cx.new(|cx| {
                        let mut view = FirstRun::new(model, hooks(), cx);
                        view.picking = picking;
                        view
                    })
                })
                .expect("open a headless window");
            cx.allow_parking();
            for _ in 0..draws() {
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

    // Overview (WP-D): the landing screen (PwrRoute Overview and Setup
    // 1a-3f) over its example.com fixtures, with the routing state set by
    // hand and motion off so each shot is the settled frame.
    {
        use overview_model::{ConfigTrouble, FocusKey, Pause, RowSource, Status};
        use pwrroute_engine::{Precedence, RulesFile};

        type Day = fn() -> Vec<pwrroute_router::DispatchRecord>;
        struct Shot {
            name: &'static str,
            width: f32,
            config: Option<fn() -> std::path::PathBuf>,
            rules: fn() -> RulesFile,
            exists: bool,
            day: Option<Day>,
            status: Status,
            prepare: fn(&mut PwrRoute),
        }
        let routing = Status::routing;
        let shots = [
            Shot {
                name: "app-0-overview-routing",
                width: 1240.0,
                config: None,
                rules: fixtures::overview_rules,
                exists: true,
                day: Some(fixtures::overview_rules_day),
                status: routing(),
                prepare: |_| {},
            },
            Shot {
                name: "app-0-overview-hover-rule",
                width: 1240.0,
                config: None,
                rules: fixtures::overview_rules,
                exists: true,
                day: Some(fixtures::overview_rules_day),
                status: routing(),
                prepare: |app| {
                    app.overview.hover(Some(FocusKey::Row(RowSource::Rule(1))));
                },
            },
            Shot {
                name: "app-0-overview-hover-card",
                width: 1240.0,
                config: None,
                rules: fixtures::overview_rules,
                exists: true,
                day: Some(fixtures::overview_rules_day),
                status: routing(),
                prepare: |app| {
                    app.overview
                        .hover(Some(FocusKey::Card("google chrome".into())));
                },
            },
            // 1e: seven rules and Everything else into one profile, every
            // route traced, in a wide window.
            Shot {
                name: "app-0-overview-hover-profile",
                width: 1880.0,
                config: None,
                rules: fixtures::overview_fanin_rules,
                exists: true,
                day: Some(fixtures::overview_fanin_day),
                status: routing(),
                prepare: |app| {
                    app.overview.hover(Some(FocusKey::Node(
                        "microsoft edge".into(),
                        Some("Research and Development Team".into()),
                    )));
                },
            },
            // 1f: Chrome · Work pinned, and Safari's idle node under the
            // pointer offering "+ Rule for …".
            Shot {
                name: "app-0-overview-pinned",
                width: 1240.0,
                config: None,
                rules: fixtures::overview_rules,
                exists: true,
                day: Some(fixtures::overview_rules_day),
                status: routing(),
                prepare: |app| {
                    app.overview.pin(Some(FocusKey::Node(
                        "google chrome".into(),
                        Some("Work".into()),
                    )));
                    app.overview.idle_hover = Some(FocusKey::Node("safari".into(), None));
                },
            },
            // Private windows: rule 5's private Firefox window pinned, and
            // Chrome's idle Incognito under the pointer offering "+ Add a
            // rule…"; Edge's InPrivate idle, Arc and Safari without one.
            Shot {
                name: "app-0-overview-private",
                width: 1240.0,
                config: None,
                rules: fixtures::overview_rules,
                exists: true,
                day: Some(fixtures::overview_rules_day),
                status: routing(),
                prepare: |app| {
                    app.overview.pin(Some(FocusKey::Private("firefox".into())));
                    app.overview.idle_hover = Some(FocusKey::Private("google chrome".into()));
                },
            },
            // The two private row targets in the narrowest rule column:
            // "Incognito · Work" beside a long pattern pair, and Safari's
            // warn-coloured "not private".
            Shot {
                name: "app-0-overview-private-targets",
                width: 1000.0,
                config: None,
                rules: fixtures::overview_private_rules,
                exists: true,
                day: Some(fixtures::overview_rules_day),
                status: routing(),
                prepare: |_| {},
            },
            // 1e at 2560: both columns at their caps, the rest to the
            // routes.
            Shot {
                name: "app-0-overview-wide",
                width: 2560.0,
                config: None,
                rules: fixtures::overview_fanin_rules,
                exists: true,
                day: Some(fixtures::overview_fanin_day),
                status: routing(),
                prepare: |app| {
                    app.overview.hover(Some(FocusKey::Node(
                        "microsoft edge".into(),
                        Some("Research and Development Team".into()),
                    )));
                },
            },
            // 1e with the user's kind of config: inline handlers Finicky
            // names "match", each read by its source, and a matchHostnames
            // list read as its hostnames, all into one Edge profile, traced.
            Shot {
                name: "app-0-overview-hover-handlers",
                width: 1880.0,
                config: Some(fixtures::overview_handlers_config),
                rules: RulesFile::default,
                exists: false,
                day: None,
                status: routing(),
                prepare: |app| {
                    app.overview.hover(Some(FocusKey::Node(
                        "microsoft edge".into(),
                        Some("Research and Development Team".into()),
                    )));
                },
            },
            Shot {
                name: "app-0-overview-config",
                width: 1240.0,
                config: Some(fixtures::overview_config),
                rules: fixtures::overview_config_rules,
                exists: true,
                day: Some(fixtures::overview_config_day),
                status: routing(),
                prepare: |_| {},
            },
            Shot {
                name: "app-0-overview-override",
                width: 1240.0,
                config: Some(fixtures::overview_config),
                rules: || RulesFile {
                    precedence: Precedence::PwrRoute,
                    ..fixtures::overview_config_rules()
                },
                exists: true,
                day: Some(fixtures::overview_override_day),
                status: routing(),
                prepare: |app| app.overview.just_overridden = true,
            },
            Shot {
                name: "app-0-overview-paused",
                width: 1000.0,
                config: None,
                rules: fixtures::overview_paused_rules,
                exists: true,
                day: Some(fixtures::overview_paused_day),
                status: Status {
                    paused: Some(Pause {
                        name: "Google Chrome".into(),
                        display: "Google Chrome \u{00b7} Personal".into(),
                    }),
                    ..routing()
                },
                prepare: |_| {},
            },
            Shot {
                name: "app-0-overview-not-default",
                width: 1000.0,
                config: None,
                rules: fixtures::overview_paused_rules,
                exists: true,
                day: None,
                status: Status {
                    default: default_browser::DefaultState::Other("Safari".into()),
                    ..routing()
                },
                prepare: |_| {},
            },
            Shot {
                name: "app-0-overview-config-error",
                width: 1000.0,
                config: Some(fixtures::overview_config),
                rules: fixtures::overview_config_rules,
                exists: true,
                day: Some(fixtures::overview_config_error_day),
                status: Status {
                    config_error: Some(ConfigTrouble::NotLoaded(
                        "SyntaxError: Unexpected token '}' \u{00b7} line 42".into(),
                    )),
                    ..routing()
                },
                // 3c's machine has no Microsoft Edge.
                prepare: |app| app.installed.retain(|app| app.name != "Microsoft Edge"),
            },
            // A reload failed: the last version that loaded still routes.
            Shot {
                name: "app-0-overview-config-stale",
                width: 1000.0,
                config: Some(fixtures::overview_config),
                rules: fixtures::overview_config_rules,
                exists: true,
                day: Some(fixtures::overview_config_day),
                status: Status {
                    config_error: Some(ConfigTrouble::ReloadFailed(
                        "SyntaxError: Unexpected token '}' \u{00b7} line 42".into(),
                    )),
                    ..routing()
                },
                prepare: |_| {},
            },
            Shot {
                name: "app-0-overview-nothing",
                width: 1000.0,
                config: None,
                rules: RulesFile::default,
                exists: false,
                day: Some(fixtures::overview_nothing_day),
                status: routing(),
                prepare: |_| {},
            },
            Shot {
                name: "app-0-overview-default-only",
                width: 1000.0,
                config: None,
                rules: || RulesFile {
                    default_browser: "Google Chrome".into(),
                    default_profile: "Personal".into(),
                    ..RulesFile::default()
                },
                exists: true,
                day: Some(fixtures::overview_default_only_day),
                status: routing(),
                prepare: |app| {
                    app.found_config = Some(importer_view::FoundConfig {
                        file: fixtures::overview_config(),
                        shown: "~/.finicky.ts".into(),
                        holds: "2 rewrites, 5 handlers and a default browser".into(),
                    });
                },
            },
        ];
        for shot in shots {
            let mut cx = HeadlessAppContext::with_platform(
                gpui_kit::platform::current_platform(true).text_system(),
                Arc::new(assets::Assets),
                gpui_kit::platform::current_headless_renderer,
            );
            cx.update(|cx| {
                setup(cx);
                cx.set_reduce_motion(true);
                if let Some(day) = shot.day {
                    activity::install_feed(activity::Feed::from_records(day()), cx);
                }
            });
            let store = rules_store::RulesStore::from_file(
                Some(std::env::temp_dir().join("pwrroute-screens-overview-rules.json")),
                (shot.rules)(),
                shot.exists,
                shot.exists.then(std::time::SystemTime::now),
            );
            let config = shot.config.map(|config| config());
            // Tall enough for the whole map; the config's runs longer, with
            // the callout under it.
            let height = if config.is_some() { 1300.0 } else { 1100.0 };
            let home = home.clone();
            let handle = cx
                .open_window(size(px(shot.width), px(height)), |_, cx| {
                    cx.new(|cx| PwrRoute::with_store(config, Some(home), store, cx))
                })
                .expect("open a headless window");
            cx.update_window(handle.into(), |view, _, cx| {
                let view = view.downcast::<PwrRoute>().expect("the PwrRoute view");
                view.update(cx, |app, cx| {
                    if let Some(config) = &mut app.config {
                        config.path = "~/.finicky.ts".into();
                    }
                    app.installed = fixtures::overview_browsers();
                    if let Some(day) = shot.day {
                        app.activity.load(day());
                    }
                    app.overview.status = Some(shot.status.clone());
                    app.overview.ticked = true;
                    (shot.prepare)(app);
                    cx.notify();
                });
            })
            .expect("prepare the window");
            cx.allow_parking();
            for _ in 0..draws() {
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
            let path = dir.join(format!("{}.png", shot.name));
            image.save(&path).expect("write the PNG");
            println!("{}", path.display());
        }
    }

    // Rules and config views (WP-E): the Rules banner's switch, on (design
    // Overview and Setup 5b), and with the warnings it stacks (5c), over the
    // canvas's config: five handlers, two needing JS, on example.com hosts.
    // The files go to a scratch directory, so a save never reaches a
    // fixture.
    {
        let scratch = std::env::temp_dir().join("pwrroute-screens-override");
        std::fs::create_dir_all(&scratch).expect("a scratch directory");
        let handlers = r#"[
    {
      match: function isWorkplace(url) { return url.host.endsWith("workplace.example.com") },
      browser: "Firefox",
    },
    { match: "chat.example.com/*", browser: { name: "Google Chrome", profile: "Work" } },
    {
      match: ["*.docs.example.com/*", "*.wiki.example.com/*"],
      browser: { name: "Google Chrome", profile: "Work" },
    },
    {
      match: "meet.example.com/*",
      browser: (url) => (url.pathname.startsWith("/j/") ? "Firefox" : "Safari"),
    },
    { match: LAST, browser: "Orion" },
  ]"#;
        let write = |name: &str, default: &str, last: &str| {
            let path = scratch.join(name);
            std::fs::write(
                &path,
                format!(
                    "export default {{\n  defaultBrowser: {default},\n  handlers: {},\n}};\n",
                    handlers.replace("LAST", last)
                ),
            )
            .expect("write the config");
            path
        };
        let plain = write(
            "override.finicky.ts",
            "\"Safari\"",
            "\"beta.example.net/*\"",
        );
        let warned = write("warnings.finicky.ts", "(url) => \"Safari\"", "\"*\"");
        let rules = scratch.join("rules.json");
        std::fs::write(
            &rules,
            r#"{
  "defaultBrowser": "Google Chrome",
  "defaultProfile": "Personal",
  "precedence": "pwrroute",
  "rewrites": [{ "match": "http://*.example.com/*", "actions": [{ "setScheme": "https" }] }],
  "rules": [
    { "match": ["localhost:*", "127.0.0.1:*", "*.test/*"], "browser": "Arc" },
    { "match": "billing.example.com/*", "browser": "Google Chrome", "profile": "Personal" },
    { "match": "status.example.com/*", "browser": "Safari" }
  ]
}
"#,
        )
        .expect("write the rules");
        let canvas = move |app: &mut PwrRoute, cx: &mut Context<'_, PwrRoute>| {
            let file = pwrroute_engine::RulesFile::load(&rules).expect("read the rules");
            app.store = rules_store::RulesStore::from_file(
                Some(rules.clone()),
                file,
                true,
                Some(std::time::SystemTime::now()),
            );
            if let Some(config) = &mut app.config {
                // Named and dated as the design shows it.
                config.path = "~/.finicky.ts".into();
                config.modified =
                    std::time::SystemTime::now().checked_sub(Duration::from_secs(4 * 60));
            }
            app.refresh(cx);
        };
        let canvas = std::rc::Rc::new(canvas);
        let on = canvas.clone();
        // The window opens on Overview: these are the Rules banner's.
        shoot("app-1b-override", Some(plain), &move |app, window, cx| {
            on(app, cx);
            app.navigate(Screen::Rules, window, cx);
        });
        shoot(
            "app-1b-override-warnings",
            Some(warned),
            &move |app, window, cx| {
                canvas(app, cx);
                app.navigate(Screen::Rules, window, cx);
            },
        );
    }

    // Settings Everything else (WP-F): Configuration while ~/.finicky.ts is
    // in charge (Overview and Setup 5d, its 1000px card less the border),
    // the same row with design 5c's warnings, and after a change that was
    // not saved.
    for (name, configuration) in [
        (
            "settings-configuration-everything-else",
            settings::fixtures::configuration_everything_else(),
        ),
        (
            "settings-configuration-everything-else-warnings",
            settings::fixtures::configuration_everything_else_warnings(),
        ),
        (
            "settings-configuration-everything-else-not-saved",
            settings::fixtures::configuration_everything_else_not_saved(),
        ),
    ] {
        let mut cx = HeadlessAppContext::with_platform(
            gpui_kit::platform::current_platform(true).text_system(),
            Arc::new(assets::Assets),
            gpui_kit::platform::current_headless_renderer,
        );
        cx.update(|cx| {
            setup(cx);
            settings::install(cx);
        });
        let handle = cx
            .open_window(size(px(998.0), px(620.0)), |_, cx| {
                cx.new(|cx| {
                    let mut model = settings::fixtures::model();
                    model.configuration = configuration;
                    let mut view =
                        settings::SettingsWindow::new(model, std::rc::Rc::new(|_, _, _| {}), cx);
                    view.pane = settings::Pane::Configuration;
                    view
                })
            })
            .expect("open a headless window");
        cx.allow_parking();
        for _ in 0..draws() {
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

/// A contrived `main.log` for the Logs window: example apps and made-up
/// rule numbers, never a link.
#[cfg(target_os = "macos")]
const FIXTURE_LOG: &str = "\
2026-10-24T09:12:01.004-04:00 INFO  pwrroute: PwrRoute 0.2.0 starting (release, arm64)
2026-10-24T09:12:01.031-04:00 INFO  pwrroute::router: router listening on ~/Library/Application Support/PwrRoute/router.sock
2026-10-24T09:12:01.044-04:00 INFO  pwrroute::router: routing with ~/.finicky.ts then rules.json (7 handlers, 3 rewrites)
2026-10-24T09:12:01.102-04:00 INFO  pwrroute::mcp: local agents listening on ~/Library/Application Support/PwrRoute/mcp.sock
2026-10-24T09:12:01.240-04:00 INFO  velopack::manager: checking for updates (Stable \u{00b7} Latest)
2026-10-24T09:12:02.511-04:00 INFO  velopack::manager: no update: 0.2.0 is the newest
2026-10-24T09:14:37.880-04:00 INFO  pwrroute::link: link 1 source=apple_event outcome=opened from=com.example.chat browser=\"Example Browser\" profile=\"Work\" rule=\"rule 2\" resolve=3ms launch=41ms
2026-10-24T09:14:38.090-04:00 INFO  pwrroute::link: link 1 hand-off: com.example.chat at receipt, PwrRoute +18ms, com.example.browser +206ms
2026-10-24T09:15:02.417-04:00 WARN  pwrroute::default_browser: reading the default browser took 68ms on the main thread
2026-10-24T09:16:10.002-04:00 INFO  pwrroute::link: link 2 source=apple_event outcome=opened from=com.example.mail browser=\"Example Browser\" profile=\"Personal\" rule=\"config handler 4\" resolve=2ms launch=38ms
2026-10-24T09:16:10.230-04:00 INFO  pwrroute::link: link 2 hand-off: com.example.mail at receipt, com.example.browser +221ms
2026-10-24T09:17:44.519-04:00 INFO  pwrroute::link: link 3 source=apple_event outcome=fallback from=com.example.editor browser=\"Other Browser\" rule=\"default\" resolve=2ms launch=55ms
2026-10-24T09:17:44.520-04:00 ERROR pwrroute::link: link 3 could not open \"Missing Browser\": no such app; opened the fallback browser
2026-10-24T09:18:03.771-04:00 INFO  pwrroute::router: rules.json changed; reloaded in 4ms
2026-10-24T09:19:20.118-04:00 INFO  pwrroute::link: link 4 source=api outcome=opened from=agent browser=\"Example Browser\" profile=\"Work\" rule=\"rule 5\" resolve=6ms launch=40ms
2026-10-24T09:20:00.000-04:00 INFO  pwrroute::default_browser: PwrRoute is the default browser
";

/// What the fixture logs after the mark in `help-logs-mark`: contrived.
#[cfg(target_os = "macos")]
const FIXTURE_LOG_AFTER_MARK: &str = "\
2026-10-24T09:20:11.204-04:00 INFO  pwrroute::router: rules.json changed; reloaded in 3ms
2026-10-24T09:20:15.880-04:00 INFO  pwrroute::link: link 5 source=apple_event outcome=opened from=com.example.chat browser=\"Example Browser\" profile=\"Work\" rule=\"rule 6\" resolve=2ms launch=39ms
2026-10-24T09:20:16.002-04:00 WARN  pwrroute::handoff: link 5 hand-off: com.example.browser took 1,240ms to come forward
";

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("screens: macOS only; GPUI's headless renderer is Metal");
}
