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
#[path = "../src/activity_actions.rs"]
mod activity_actions;
#[cfg(target_os = "macos")]
#[path = "../src/app.rs"]
mod app;
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
#[path = "../src/overview.rs"]
mod overview;
#[cfg(target_os = "macos")]
#[path = "../src/overview_model.rs"]
mod overview_model;
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
#[path = "../src/settings_store.rs"]
mod settings_store;
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
#[path = "../src/updater.rs"]
mod updater;

/// As `main.rs` places the traffic lights, for the first-run window.
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHTS: (f32, f32) = (16.0, 13.0);
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

    // The window lands on Overview: these open Rules, as a click does.
    shoot("app-1a", None, &|app, window, cx| {
        app.navigate(Screen::Rules, window, cx);
        app.select(Selection::Handler(0), cx);
    });
    shoot("app-1a-rewrite", None, &|app, window, cx| {
        app.navigate(Screen::Rules, window, cx);
        app.select(Selection::Rewrite(0), cx);
    });
    shoot("app-1b", Some(config.clone()), &|app, window, cx| {
        app.navigate(Screen::Rules, window, cx);
    });
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
        if let Some(sheet) = app.actions.sheet.as_mut() {
            sheet.picking = activity_actions::Picking::Browser;
        }
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
        cx.update(|cx| assets::load_fonts(cx));
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

    // First run (design 2a) over contrived browsers: step 1 with a profiled
    // browser chosen and its profile menu open, then step 3 before and
    // after the OS agrees. The default it names is an app the catalog does
    // not list, by the name its bundle gives.
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
                },
                BrowserChoice {
                    value: "org.mozilla.firefox".into(),
                    label: BrowserLabel::new("Ff", "Firefox", None),
                    profiles: vec!["default-release".into()],
                },
                BrowserChoice {
                    value: "com.apple.Safari".into(),
                    label: BrowserLabel::new("Sa", "Safari", None),
                    profiles: Vec::new(),
                },
            ],
            finicky: Some(FinickyConfig {
                path: "~/.finicky.ts".into(),
                rewrites: 2,
                handlers: 6,
                importable: 0,
            }),
            import: None,
            current_default: "PwrFinicky".into(),
            default_requested: false,
            waiting: false,
            is_default: false,
        };
        let chrome_work = || Some(BrowserLabel::new("Ch", "Google Chrome", Some("Work")));
        let hooks = || FirstRunHooks {
            on_fallback: std::rc::Rc::new(|_, _| {}),
            on_import: std::rc::Rc::new(|_, _| {}),
            on_set_default_browser: std::rc::Rc::new(|(), _| {}),
            on_finish: std::rc::Rc::new(|_, _| {}),
        };
        let mut step_3 = model(chrome_work());
        step_3.import = Some(first_run::ImportChoice::KeepFile);
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
                model(chrome_work()),
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
            cx.update(|cx| assets::load_fonts(cx));
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
        cx.update(|cx| assets::load_fonts(cx));
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

    // Overview (WP-D): the landing screen (PwrRoute Overview and Setup
    // 1a-3f) over its example.com fixtures, with the routing state set by
    // hand and motion off so each shot is the settled frame.
    {
        use overview_model::{Focus, Pause, Status};
        use pwrroute_engine::{Precedence, RulesFile};

        type Day = fn() -> Vec<pwrroute_router::DispatchRecord>;
        struct Shot {
            name: &'static str,
            width: f32,
            config: bool,
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
                config: false,
                rules: fixtures::overview_rules,
                exists: true,
                day: Some(fixtures::overview_rules_day),
                status: routing(),
                prepare: |_| {},
            },
            Shot {
                name: "app-0-overview-hover-rule",
                width: 1240.0,
                config: false,
                rules: fixtures::overview_rules,
                exists: true,
                day: Some(fixtures::overview_rules_day),
                status: routing(),
                prepare: |app| {
                    app.overview.hover(Some(Focus::Row(1)));
                },
            },
            Shot {
                name: "app-0-overview-hover-card",
                width: 1240.0,
                config: false,
                rules: fixtures::overview_rules,
                exists: true,
                day: Some(fixtures::overview_rules_day),
                status: routing(),
                prepare: |app| {
                    app.overview.hover(Some(Focus::Card(1)));
                },
            },
            Shot {
                name: "app-0-overview-config",
                width: 1240.0,
                config: true,
                rules: fixtures::overview_config_rules,
                exists: true,
                day: Some(fixtures::overview_config_day),
                status: routing(),
                prepare: |_| {},
            },
            Shot {
                name: "app-0-overview-override",
                width: 1240.0,
                config: true,
                rules: || RulesFile {
                    precedence: Precedence::PwrRoute,
                    ..fixtures::overview_config_rules()
                },
                exists: true,
                day: Some(fixtures::overview_config_day),
                status: routing(),
                prepare: |app| app.overview.just_overridden = true,
            },
            Shot {
                name: "app-0-overview-paused",
                width: 1000.0,
                config: false,
                rules: fixtures::overview_rules,
                exists: true,
                day: Some(fixtures::overview_rules_day),
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
                config: false,
                rules: fixtures::overview_rules,
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
                config: true,
                rules: fixtures::overview_config_rules,
                exists: true,
                day: Some(fixtures::overview_config_error_day),
                status: Status {
                    config_error: Some("SyntaxError: Unexpected token '}' \u{00b7} line 42".into()),
                    ..routing()
                },
                prepare: |_| {},
            },
            Shot {
                name: "app-0-overview-nothing",
                width: 1000.0,
                config: false,
                rules: RulesFile::default,
                exists: false,
                day: Some(fixtures::overview_nothing_day),
                status: routing(),
                prepare: |_| {},
            },
            Shot {
                name: "app-0-overview-default-only",
                width: 1000.0,
                config: false,
                rules: || RulesFile {
                    default_browser: "Google Chrome".into(),
                    default_profile: "Personal".into(),
                    ..RulesFile::default()
                },
                exists: true,
                day: Some(fixtures::overview_nothing_day),
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
                assets::load_fonts(cx);
                cx.set_reduce_motion(true);
                if let Some(day) = shot.day {
                    activity::install_feed(activity::Feed::from_records(day()), cx);
                }
            });
            let store = rules_store::RulesStore::from_file(
                Some(std::env::temp_dir().join("pwrroute-screens-overview-rules.json")),
                (shot.rules)(),
                shot.exists,
                Some(std::time::SystemTime::now()),
            );
            let config = shot.config.then(fixtures::overview_config);
            // The config's map runs longer: room for the callout under it.
            let height = if shot.config { 1100.0 } else { 900.0 };
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
            let path = dir.join(format!("{}.png", shot.name));
            image.save(&path).expect("write the PNG");
            println!("{}", path.display());
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("screens: macOS only; GPUI's headless renderer is Metal");
}
