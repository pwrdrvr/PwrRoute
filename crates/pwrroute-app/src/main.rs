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
mod activity_actions;
mod app;
mod app_menu;
mod assets;
#[cfg(target_os = "windows")]
mod browser_registration;
mod browser_scan;
mod chrome;
mod config_view;
mod default_browser;
mod dock;
mod e2e;
mod editor;
mod first_run;
mod fixtures;
#[cfg(target_os = "macos")]
mod macos;
mod mcp;
mod onboarding;
mod router;
mod rules_model;
mod rules_store;
mod rules_view;
mod settings;
mod settings_live;
mod settings_store;
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
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
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

fn main() -> ExitCode {
    // `pwrroute mcp`: the stdio end an MCP client launches. A byte pipe to
    // the running app's socket, with no window, no router and no lock.
    if std::env::args_os().nth(1).is_some_and(|arg| arg == "mcp") {
        return pwrroute_mcp::proxy::main();
    }
    // First: Velopack's install, update and uninstall hooks run here and
    // exit, before this process could take the router lock.
    updater::run_startup_hooks();
    // `pwrroute rules test` runs without the router or a window.
    if let Some(code) = rules_command(std::env::args().skip(1).collect()) {
        return code;
    }
    let args = router::Args::from_env();
    let resident = match router::start(&args) {
        router::Start::Exit => return ExitCode::SUCCESS,
        router::Start::Resident(resident) => *resident,
    };
    let handle = resident.handle();
    let e2e_hook = e2e::start(handle.clone());
    // `--first-run` forces the window; otherwise it opens once, on the first
    // launch that is for the window rather than a link, until it is done or
    // closed. Not on its own under the E2E hook: the suite drives the main
    // window, from an empty home where onboarding is always pending.
    let first_run = args.first_run
        || (e2e_hook.is_none()
            && pwrroute_os::data_dir().is_some_and(|dir| onboarding::State::pending(&dir)));
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
        settings_store::install(resident.settings.clone(), Some(resident.handle()), cx);
        tray::install(cx);
        default_browser::install(default_browser::Os::real(), cx);
        tray::follow_default_browser(cx);
        // Local agents: MCP on <data dir>/mcp.sock, beside the router's.
        mcp::install(
            mcp::Wiring {
                handle: resident.handle(),
                config: args.config.clone(),
            },
            cx,
        );
        // The one browser scan, read when a picker or Settings first asks.
        browser_scan::install(pwrroute_browsers::Env::system(), cx);
        cx.set_global(settings::SettingsHost {
            model: Rc::new(settings_live::model),
            on_request: Rc::new(|request, _, cx| settings_live::handle_request(request, cx)),
        });
        updater::install(cx);
        if let Some(hook) = e2e_hook {
            e2e::attach(hook, cx);
        } else {
            updater::check_in_background(updater::Trigger::Startup);
        }

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
    ExitCode::SUCCESS
}

/// `pwrroute rules test [--rules PATH] [--config PATH]`: runs the examples
/// and tests `rules.json` carries through the engine, with the Finicky
/// config in charge (discovered as the router does, or `--config`). The
/// exit code is 0 when every one passes, 1 on a failure, 2 when they could
/// not run; `main` returns it rather than calling `process::exit`, so the
/// process ends the normal way, destructors included. `None` when the
/// command line is not this command.
fn rules_command(args: Vec<String>) -> Option<ExitCode> {
    let mut rest = args.iter().map(String::as_str);
    if rest.next() != Some("rules") {
        return None;
    }
    let mut rules = None;
    let mut config = None;
    let mut verb = None;
    while let Some(arg) = rest.next() {
        match arg {
            "--rules" => rules = rest.next().map(PathBuf::from),
            "--config" => config = rest.next().map(PathBuf::from),
            other if other.starts_with("--rules=") => {
                rules = Some(PathBuf::from(&other["--rules=".len()..]));
            }
            other if other.starts_with("--config=") => {
                config = Some(PathBuf::from(&other["--config=".len()..]));
            }
            other if verb.is_none() => verb = Some(other.to_owned()),
            other => {
                eprintln!("pwrroute rules: unexpected argument {other}");
                return Some(ExitCode::from(2));
            }
        }
    }
    if verb.as_deref() != Some("test") {
        eprintln!("usage: pwrroute rules test [--rules PATH] [--config PATH]");
        return Some(ExitCode::from(2));
    }
    // No flags: the rules.json and config the router routes by. An explicit
    // `--rules` runs that file on its own unless `--config` names one too, so
    // a file under test is not read against a config it was not written for.
    let environment = pwrroute_engine::discovery::Environment::current();
    let (rules, config) = match rules {
        Some(rules) => (Some(rules), config),
        None => (
            pwrroute_engine::discovery::rules_path(&environment),
            config.or_else(|| {
                pwrroute_engine::discovery::find_config(
                    &pwrroute_engine::discovery::config_candidates(&environment),
                )
            }),
        ),
    };
    if let Some(path) = rules.as_deref().filter(|p| !p.is_file()) {
        eprintln!("pwrroute rules test: no rules file at {}", path.display());
        return Some(ExitCode::from(2));
    }
    let mut out = std::io::stdout().lock();
    Some(ExitCode::from(run_rules_tests(
        rules.as_deref(),
        config.as_deref(),
        &mut out,
    )))
}

/// Runs the file's examples and tests and prints the result: one line per
/// failure with the input, what was expected, what happened and which rule
/// won. The inputs are the user's own saved examples, so printing them is
/// what the command is for. The result is the command's exit code.
fn run_rules_tests(rules: Option<&Path>, config: Option<&Path>, out: &mut dyn Write) -> u8 {
    use pwrroute_engine::{DefaultHost, Engine, Limits, RulesFile, Sources, TestSource};

    let rules_file = match rules {
        Some(path) => match RulesFile::load(path) {
            Ok(file) => file,
            Err(error) => {
                let _ = writeln!(out, "rules.json did not load: {error:#}");
                return 2;
            }
        },
        None => RulesFile::default(),
    };
    let sources = Sources {
        config: config.map(Path::to_path_buf),
        rules: rules_file,
    };
    let engine = match Engine::from_sources(&sources, Rc::new(DefaultHost), Limits::default()) {
        Ok(engine) => engine,
        Err(error) => {
            let _ = writeln!(out, "the rules did not load: {error:#}");
            return 2;
        }
    };
    let report = match engine.test_rules(None) {
        Ok(report) => report,
        Err(error) => {
            let _ = writeln!(out, "the tests did not run: {error:#}");
            return 2;
        }
    };
    for failure in &report.failures {
        let source = match failure.source {
            TestSource::RuleExample { rule, example } => {
                format!("rules[{rule}].examples[{example}]")
            }
            TestSource::RewriteExample { rewrite, example } => {
                format!("rewrites[{rewrite}].examples[{example}]")
            }
            TestSource::Test { test } => format!("tests[{test}]"),
        };
        let from = match &failure.opener {
            Some(opener) => format!(" from {opener}"),
            None => String::new(),
        };
        let winner = match &failure.winner {
            Some(step) => {
                let list = match step.source {
                    pwrroute_engine::Source::Rules => "",
                    pwrroute_engine::Source::Config => "config ",
                };
                match step.stage {
                    pwrroute_engine::Stage::Default => "the default".to_owned(),
                    pwrroute_engine::Stage::Handler => {
                        format!("{list}handler {}", step.index + 1)
                    }
                    pwrroute_engine::Stage::Rewrite => {
                        format!("{list}rewrite {}", step.index + 1)
                    }
                }
            }
            None => "no handler".to_owned(),
        };
        let _ = writeln!(
            out,
            "FAIL {source}\n  input:    {}{from}\n  expected: {}\n  actual:   {}\n  winner:   {winner}",
            failure.input, failure.expected, failure.actual
        );
    }
    let _ = writeln!(
        out,
        "{} passed, {} failed, {} total",
        report.passed,
        report.failures.len(),
        report.total
    );
    u8::from(!report.is_ok())
}

/// Brings the main window forward, opening it if it is closed.
pub(crate) fn show_main_window(cx: &mut App) {
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
                let view = cx.new(PwrRoute::new);
                // And the Rules screen follows rules.json.
                view.update(cx, |this, cx| this.watch_activation(window, cx));
                view
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

#[cfg(test)]
mod cli_tests {
    use super::*;

    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn rules_test_exits_by_the_report() {
        let dir = tempfile::tempdir().unwrap();
        let passing = write(
            dir.path(),
            "passing.json",
            r#"{ "defaultBrowser": "Firefox",
                 "rules": [{ "match": "github.com/*", "browser": "Arc", "examples": ["https://github.com/x"] }],
                 "tests": [{ "url": "https://example.com/", "expect": { "browser": "Firefox" } }] }"#,
        );
        let mut out = Vec::new();
        assert_eq!(run_rules_tests(Some(&passing), None, &mut out), 0);
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text, "2 passed, 0 failed, 2 total\n");

        // A wider rule above takes the example: 1, with the failure row.
        let failing = write(
            dir.path(),
            "failing.json",
            r#"{ "defaultBrowser": "Firefox",
                 "rules": [{ "match": "*", "browser": "Safari" },
                           { "match": "github.com/*", "browser": "Arc", "examples": ["https://github.com/x"] }] }"#,
        );
        let mut out = Vec::new();
        assert_eq!(run_rules_tests(Some(&failing), None, &mut out), 1);
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.starts_with("FAIL rules[1].examples[0]\n  input:    https://github.com/x\n"),
            "{text}"
        );
        assert!(text.contains("  winner:   handler 1\n"), "{text}");
        assert!(text.ends_with("0 passed, 1 failed, 1 total\n"), "{text}");

        // A file that does not load: 2.
        let broken = write(dir.path(), "broken.json", "{ not json");
        let mut out = Vec::new();
        assert_eq!(run_rules_tests(Some(&broken), None, &mut out), 2);
        // No file at all is an empty one: nothing to run, and that passes.
        let mut out = Vec::new();
        assert_eq!(
            run_rules_tests(Some(&dir.path().join("missing.json")), None, &mut out),
            0
        );
        // The JS config runs first: its handler takes the example.
        let config = write(
            dir.path(),
            "finicky.js",
            "module.exports = { defaultBrowser: \"Firefox\", handlers: [{ match: \"github.com/*\", browser: \"Safari\" }] };\n",
        );
        let mut out = Vec::new();
        assert_eq!(run_rules_tests(Some(&passing), Some(&config), &mut out), 1);
        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("winner:   config handler 1")
        );
    }

    #[test]
    fn only_the_rules_command_is_taken() {
        assert_eq!(rules_command(vec![]), None);
        assert_eq!(rules_command(vec!["https://example.com/".into()]), None);
        assert_eq!(rules_command(vec!["rules".into()]), Some(ExitCode::from(2)));
        assert_eq!(
            rules_command(vec!["rules".into(), "test".into(), "--bogus".into()]),
            Some(ExitCode::from(2))
        );
        let dir = tempfile::tempdir().unwrap();
        let rules = write(
            dir.path(),
            "rules.json",
            r#"{ "defaultBrowser": "Firefox", "rules": [] }"#,
        );
        assert_eq!(
            rules_command(vec![
                "rules".into(),
                "test".into(),
                format!("--rules={}", rules.display()),
                "--config".into(),
                dir.path().join("none.js").display().to_string(),
            ]),
            Some(ExitCode::from(2)),
            "a config that does not exist cannot load"
        );
        assert_eq!(
            rules_command(vec![
                "rules".into(),
                "test".into(),
                "--rules".into(),
                rules.display().to_string(),
            ]),
            Some(ExitCode::SUCCESS)
        );
    }
}
