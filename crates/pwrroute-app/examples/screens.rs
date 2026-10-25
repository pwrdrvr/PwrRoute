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
#[path = "../src/diag_log.rs"]
mod diag_log;
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

/// Each window's fonts, and the apps' icons when asked for.
#[cfg(target_os = "macos")]
fn setup(cx: &mut gpui_kit::App) {
    assets::load_fonts(cx);
    if APP_ICONS.load(std::sync::atomic::Ordering::Relaxed) {
        app_icons::install(cx);
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
    shoot("app-1a-open-in", None, &|app, window, cx| {
        with_scan(app);
        app.navigate(Screen::Rules, window, cx);
        app.select(Selection::Handler(0), cx);
        if let Some(rail) = &mut app.rail {
            rail.menu = Some(editor::RailMenu::Browser);
        }
    });
    shoot("app-1a-profile", None, &|app, window, cx| {
        with_scan(app);
        app.navigate(Screen::Rules, window, cx);
        app.select(Selection::Handler(0), cx);
        if let Some(rail) = &mut app.rail {
            rail.menu = Some(editor::RailMenu::Profile);
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
                },
                BrowserChoice {
                    value: "org.mozilla.firefox".into(),
                    label: BrowserLabel::new("Ff", "Firefox", None),
                    profiles: vec!["default-release".into()],
                },
                BrowserChoice {
                    value: "com.apple.Safari".into(),
                    label: BrowserLabel::new("Sf", "Safari", None),
                    profiles: Vec::new(),
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
            },
            BrowserChoice {
                value: "org.mozilla.firefox".into(),
                label: BrowserLabel::new("Ff", "Firefox", None),
                profiles: vec!["default-release".into()],
            },
            BrowserChoice {
                value: "com.apple.Safari".into(),
                label: BrowserLabel::new("Sf", "Safari", None),
                profiles: Vec::new(),
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
        let hooks = || FirstRunHooks {
            on_fallback: std::rc::Rc::new(|_, _| Ok(())),
            on_precedence: std::rc::Rc::new(|_, _| Ok(())),
            on_import: std::rc::Rc::new(|_, _, _| {}),
            on_rescan: std::rc::Rc::new(|_| Vec::new()),
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

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("screens: macOS only; GPUI's headless renderer is Metal");
}
