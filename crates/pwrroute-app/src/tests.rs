//! Smoke tests through gpui-kit's window harness: open the real view, draw
//! frames, click, type and press keys. The window reads and writes a copy
//! of the fixture `rules.json` in a scratch directory, and Test a link runs
//! the real engine over it (and the fixture Finicky config).

use std::path::{Path, PathBuf};

use gpui_kit::{
    Bounds, Entity, InputEvent, Modifiers, Pixels, ScrollDelta, ScrollWheelEvent, TestAppContext,
    VisualTestContext, point, px,
};
use pwrroute_engine::{Privacy, PrivacySetting, RouteTest, RulesFile, TestSource};
use pwrroute_router::{
    DispatchId, DispatchRecord, DryRunLauncher, Link, Matched, Options, Outcome, Router, Source,
    Step,
};

use crate::activity::{self, EMPTY_ID, Feed, Filter, RowKey};
use crate::activity_actions::{BrowserChoice, RowMenu, Save};
use crate::app::{PwrRoute, ROOT_ID, Screen, Selection};
use crate::fixtures;
use crate::rules_model::{Destination, Matcher, RowBadge};
use crate::rules_store::RulesStore;
use crate::trace_view::TraceView;

/// The contrived home directory that holds `.finicky.ts` for option 1b.
fn fixture_home() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/home")
}

/// A scratch directory of its own for each test's `rules.json`. Tests run
/// in parallel, so the name alone is not enough.
fn scratch(name: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pwrroute-app-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch dir");
    dir
}

/// The fixture rules written to `dir`, and the store over them.
fn fixture_store(dir: &Path) -> RulesStore {
    let path = dir.join("rules.json");
    crate::fixtures::rules_file()
        .save(&path)
        .expect("write the fixture rules");
    RulesStore::load(Some(path))
}

fn open_store(
    cx: &mut TestAppContext,
    config: Option<PathBuf>,
    store: RulesStore,
) -> (Entity<PwrRoute>, &mut VisualTestContext) {
    cx.update(|cx| crate::assets::load_fonts(cx));
    let (view, cx) =
        cx.add_window_view(|_, cx| PwrRoute::with_store(config, Some(fixture_home()), store, cx));
    let focus = view.read_with(cx, |app, _| app.focus.clone());
    cx.update(|window, cx| window.focus(&focus, cx));
    draw(cx);
    (view, cx)
}

fn open_with(
    cx: &mut TestAppContext,
    config: Option<PathBuf>,
) -> (Entity<PwrRoute>, &mut VisualTestContext) {
    let dir = scratch("window");
    open_store(cx, config, fixture_store(&dir))
}

fn open(cx: &mut TestAppContext) -> (Entity<PwrRoute>, &mut VisualTestContext) {
    open_with(cx, None)
}

/// The file as it stands, after any debounced save has run.
fn saved(cx: &mut VisualTestContext, path: &Path) -> RulesFile {
    cx.executor().advance_clock(crate::app::SAVE_DEBOUNCE);
    cx.run_until_parked();
    RulesFile::load(path).expect("read the rules back")
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

/// Clicks the middle of the element with this debug selector.
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("no element {selector} on screen"));
    cx.simulate_click(bounds.center(), Modifiers::default());
    draw(cx);
}

/// Scrolls the pane under `selector` until the element is inside the window,
/// the way a wheel does; a click below the fold reaches nothing.
fn scroll_to(cx: &mut VisualTestContext, selector: &'static str) {
    for _ in 0..20 {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("no element {selector} on screen"));
        let height = cx.update(|window, _| window.viewport_size().height);
        let below = bounds.bottom() + px(24.0) - height;
        if below <= px(0.0) {
            return;
        }
        let event = ScrollWheelEvent {
            position: point(bounds.center().x, height / 2.0),
            delta: ScrollDelta::Pixels(point(px(0.0), -below)),
            ..Default::default()
        };
        cx.update(|window, cx| window.dispatch_event(event.to_platform_input(), cx));
        draw(cx);
    }
    panic!("{selector} never came into view");
}

/// Types the design's test link, from Slack, into the open Test a link
/// screen, which starts empty, and runs it.
fn type_sample(cx: &mut VisualTestContext) {
    click(cx, "test-url");
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input(fixtures::TEST_URL);
    click(cx, "test-opener");
    click(cx, "test-opener-choice-1");
}

fn trace(view: &Entity<PwrRoute>, cx: &mut VisualTestContext) -> TraceView {
    view.read_with(cx, |app, _| app.test.outcome.clone())
        .expect("Test a link ran")
        .expect("the trace resolved")
}

fn matcher(matcher: &Matcher) -> String {
    match matcher {
        Matcher::Pattern(s) | Matcher::Regex(s) => s.to_string(),
        Matcher::Function(source) => format!("fn {source}"),
        other => format!("{other:?}"),
    }
}

fn browser(destination: &Destination) -> String {
    match destination {
        Destination::Browser { name, .. } => name.to_string(),
        other => format!("{other:?}"),
    }
}

fn titles(trace: &TraceView) -> Vec<&str> {
    trace.steps.iter().map(|s| s.title.as_str()).collect()
}

#[gpui_kit::test]
fn the_window_draws_a_frame(cx: &mut TestAppContext) {
    let (_view, cx) = open(cx);
    let bounds: Bounds<Pixels> = cx.debug_bounds(ROOT_ID).expect("root element");
    assert!(bounds.size.width > px(0.0) && bounds.size.height > px(0.0));
    assert!(cx.debug_bounds("editor-rail").is_some());
}

#[gpui_kit::test]
fn arrow_keys_move_through_rewrites_and_handlers(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    let selection = |cx: &mut VisualTestContext| view.read_with(cx, |app, _| app.selection);
    assert_eq!(selection(cx), Some(Selection::Handler(0)));

    cx.simulate_keystrokes("down down");
    draw(cx);
    assert_eq!(selection(cx), Some(Selection::Handler(2)));

    // Up past the first handler into the rewrites, clamped at the top.
    cx.simulate_keystrokes("up up up up up up up up up up");
    draw(cx);
    assert_eq!(selection(cx), Some(Selection::Rewrite(0)));
    assert!(cx.debug_bounds("editor-rail").is_some());

    // The rewrite editor's switch turns the rewrite off.
    click(cx, "handler-enabled");
    assert!(view.read_with(cx, |app, _| !app.rewrites[0].rule.enabled));

    // Typing in a field does not move the selection.
    click(cx, "pattern-0");
    cx.simulate_keystrokes("down");
    draw(cx);
    assert_eq!(selection(cx), Some(Selection::Rewrite(0)));
}

#[gpui_kit::test]
fn the_rail_switches_screens(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    let screen = |cx: &mut VisualTestContext| view.read_with(cx, |app, _| app.screen);

    click(cx, "nav-test");
    assert_eq!(screen(cx), Screen::TestLink);
    assert!(cx.debug_bounds("test-run").is_some());
    assert!(cx.debug_bounds("test-trace").is_none());
    assert!(cx.debug_bounds("editor-rail").is_none());

    click(cx, "nav-activity");
    assert_eq!(screen(cx), Screen::Activity);
    // No router, so nothing has been routed: the empty state.
    assert!(cx.debug_bounds(EMPTY_ID).is_some());
    assert!(cx.debug_bounds("activity-row-0").is_none());
    click(cx, "activity-empty-test");
    assert_eq!(screen(cx), Screen::TestLink);

    click(cx, "nav-rules");
    assert_eq!(screen(cx), Screen::Rules);
    assert!(cx.debug_bounds("editor-rail").is_some());
}

#[gpui_kit::test]
fn test_a_link_traces_the_fixture_rules(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    click(cx, "nav-test");

    // The screen starts empty: no link, no app, nothing traced, and Test
    // with nothing typed is not an error.
    assert_eq!(view.read_with(cx, |app, _| app.test.url.text.clone()), "");
    assert_eq!(view.read_with(cx, |app, _| app.test.opener()), None);
    assert!(view.read_with(cx, |app, _| app.test.outcome.is_none()));
    click(cx, "test-run");
    assert!(view.read_with(cx, |app, _| app.test.outcome.is_none()));
    assert!(cx.debug_bounds("test-trace").is_none());

    // The design's example: two rewrites apply, handler 3 wins on Slack.
    type_sample(cx);
    let first = trace(&view, cx);
    assert_eq!(
        titles(&first),
        [
            "Received",
            "Rewrite 1",
            "Rewrite 2",
            "Rewrite 3",
            "Handlers 1 and 2",
            "Handler 3",
            "Opens in",
        ]
    );
    assert_eq!(first.result.title, "Firefox \u{00b7} default-release");
    assert_eq!(
        first.result.url,
        "https://xcancel.com/pwrdrvr/status/1842?s=20"
    );

    // Unfold the missed handlers.
    click(cx, "trace-show-each");
    assert!(titles(&trace(&view, cx)).contains(&"Handler 2"));

    // Type a new link and press Return.
    click(cx, "test-url");
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("https://meet.google.com/abc");
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(
        view.read_with(cx, |app, _| app.test.url.text.clone()),
        "https://meet.google.com/abc"
    );
    let meet = trace(&view, cx);
    assert_eq!(meet.result.title, "Google Chrome \u{00b7} Work");
    assert!(titles(&meet).contains(&"Handler 1"));
}

#[gpui_kit::test]
fn the_opener_picker_reruns_the_test(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    click(cx, "nav-test");
    click(cx, "test-url");
    cx.simulate_input(fixtures::TEST_URL);
    cx.simulate_keystrokes("enter");
    draw(cx);
    // From no app: handler 3 does not match and the default opens it.
    let none = trace(&view, cx);
    assert_eq!(none.result.title, "Google Chrome \u{00b7} Personal");
    assert!(titles(&none).contains(&"Everything else"));

    let picking = |cx: &mut VisualTestContext| view.read_with(cx, |app, _| app.test.picking);
    // The picker opens its menu and, clicked again, closes it.
    click(cx, "test-opener");
    assert!(picking(cx));
    click(cx, "test-opener");
    assert!(!picking(cx));
    click(cx, "test-opener");
    assert!(picking(cx));
    // The fixture rules name Slack by bundle id: the example is offered
    // as an app in the rules, and from it handler 3 wins.
    let slack = view.read_with(cx, |app, _| {
        let choices = app.test.openers();
        let slack = choices[1].clone().expect("the first example");
        assert_eq!(slack.bundle_id.as_ref(), "com.tinyspeck.slackmacgap");
        app.test.named_by_rules(&slack)
    });
    assert!(slack);
    click(cx, "test-opener-choice-1");
    let from_slack = trace(&view, cx);
    assert_eq!(from_slack.result.title, "Firefox \u{00b7} default-release");
    assert!(!picking(cx));
}

/// The picker offers an app a rule names as written, after the examples,
/// so a "sent from" rule can be tried; it goes when the rule does.
#[gpui_kit::test]
fn the_opener_picker_offers_the_apps_the_rules_name(cx: &mut TestAppContext) {
    let dir = scratch("openers");
    let path = dir.join("rules.json");
    let mut rules = crate::fixtures::rules_file();
    rules.rules[1].opener = vec!["com.microsoft.teams2".to_owned()];
    rules.save(&path).expect("write the rules");
    let (view, cx) = open_store(cx, None, RulesStore::load(Some(path)));
    click(cx, "nav-test");
    let choices = view.read_with(cx, |app, _| app.test.openers());
    let teams = choices.len() - 1;
    assert_eq!(teams, fixtures::OPENERS.len());
    let last = choices[teams].clone().expect("the app from the rules");
    assert_eq!(last.name.as_ref(), "com.microsoft.teams2");
    assert_eq!(last.bundle_id.as_ref(), "com.microsoft.teams2");

    click(cx, "test-url");
    cx.simulate_input("https://github.com/pwrdrvr/PwrGit");
    click(cx, "test-opener");
    let id: &'static str = Box::leak(format!("test-opener-choice-{teams}").into_boxed_str());
    click(cx, id);
    assert_eq!(view.read_with(cx, |app, _| app.test.opener), teams);
    assert_eq!(trace(&view, cx).result.title, "Google Chrome \u{00b7} Work");

    // The rule stops naming it: the picker falls back to no app.
    view.update(cx, |app, _| {
        let mut rules = crate::fixtures::rules_file();
        rules.rules[1].opener = Vec::new();
        app.test.set_rules(rules);
    });
    assert_eq!(view.read_with(cx, |app, _| app.test.opener), 0);
    assert_eq!(
        view.read_with(cx, |app, _| app.test.openers().len()),
        fixtures::OPENERS.len()
    );
}

#[gpui_kit::test]
fn activity_expands_filters_and_retests(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    view.update(cx, |app, _| app.activity.load(fixtures::activity()));
    click(cx, "nav-activity");
    let expanded = |cx: &mut VisualTestContext| view.read_with(cx, |app, _| app.activity.expanded);
    let id = |cx: &mut VisualTestContext, index: usize| {
        view.read_with(cx, |app, _| app.activity.rows[index].key)
    };
    assert_eq!(expanded(cx), None);

    click(cx, "activity-row-1");
    assert_eq!(expanded(cx), Some(id(cx, 1)));
    click(cx, "activity-row-0");
    assert_eq!(expanded(cx), Some(id(cx, 0)));
    click(cx, "activity-row-0");
    assert_eq!(expanded(cx), None);

    click(cx, "activity-filter-Failed");
    assert_eq!(
        view.read_with(cx, |app, _| app.activity.filter),
        Filter::Failed
    );
    assert!(cx.debug_bounds("activity-row-3").is_some());
    assert!(cx.debug_bounds("activity-row-0").is_none());

    click(cx, "activity-filter-All");
    click(cx, "activity-search");
    cx.simulate_input("ghostty");
    draw(cx);
    assert!(cx.debug_bounds("activity-row-2").is_some());
    assert!(cx.debug_bounds("activity-row-1").is_none());

    // Re-test the expanded row: Test a link with its link and app.
    cx.simulate_keystrokes("cmd-a backspace");
    click(cx, "activity-row-1");
    click(cx, "activity-retest-1");
    assert_eq!(view.read_with(cx, |app, _| app.screen), Screen::TestLink);
    assert_eq!(
        trace(&view, cx).result.title,
        "Firefox \u{00b7} default-release"
    );

    // A row from an app the picker does not offer re-tests as no app, not
    // as Slack (picked by the previous re-test).
    click(cx, "nav-activity");
    click(cx, "activity-row-4");
    click(cx, "activity-retest-4");
    assert_eq!(view.read_with(cx, |app, _| app.test.opener), 0);
    assert_eq!(trace(&view, cx).result.title, "Google Chrome \u{00b7} Work");

    // Clear empties the list.
    click(cx, "nav-activity");
    click(cx, "activity-clear");
    assert!(view.read_with(cx, |app, _| app.activity.rows.is_empty()));
    assert!(cx.debug_bounds(EMPTY_ID).is_some());
}

#[gpui_kit::test]
fn activity_follows_the_dispatch_feed(cx: &mut TestAppContext) {
    let records = fixtures::activity();
    let newest = records[0].clone();
    let feed =
        cx.update(|cx| activity::install_feed(Feed::from_records(records[1..].to_vec()), cx));
    let (view, cx) = open(cx);
    click(cx, "nav-activity");
    let rows = |cx: &mut VisualTestContext| {
        view.read_with(cx, |app, _| {
            app.activity
                .rows
                .iter()
                .map(|row| row.key.id)
                .collect::<Vec<_>>()
        })
    };
    let ids: Vec<DispatchId> = records.iter().map(|r| r.id).collect();
    assert_eq!(rows(cx), ids[1..]);
    assert_eq!(
        view.read_with(cx, |app, _| app.activity.today.routed),
        ids.len() as u64 - 1
    );

    // Expand the first row, then a dispatch lands: it goes on top, the
    // expanded row stays expanded, and the counts move.
    click(cx, "activity-row-0");
    feed.update(cx, |feed, cx| {
        feed.push(newest);
        cx.notify();
    });
    draw(cx);
    assert_eq!(rows(cx), ids);
    assert_eq!(
        view.read_with(cx, |app, _| app.activity.expanded.map(|key| key.id)),
        Some(ids[1])
    );
    let today = view.read_with(cx, |app, _| app.activity.today);
    assert_eq!((today.routed, today.failed), (ids.len() as u64, 1));
    assert!(cx.debug_bounds("activity-row-0").is_some());

    // Clear goes through the feed and empties the screen.
    click(cx, "activity-clear");
    assert!(feed.read_with(cx, |feed, _| feed.records().is_empty()));
    assert!(rows(cx).is_empty());
    assert!(cx.debug_bounds(EMPTY_ID).is_some());
}

/// Links routed before `activity::install` runs (the command line's, or a
/// second launch's forwarded over IPC while GPUI starts) are in the router's
/// buffer and on the `dispatched` channel both. The feed takes each once.
#[gpui_kit::test]
fn links_routed_before_install_land_in_the_feed_once(cx: &mut TestAppContext) {
    let (dispatched_tx, dispatched) = async_channel::unbounded();
    let sender = dispatched_tx.clone();
    let options = Options {
        on_dispatch: Some(std::sync::Arc::new(move |record: &DispatchRecord| {
            let _ = dispatched_tx.try_send(record.clone());
        })),
        ..Options::default()
    };
    let router = Router::spawn(options, None, DryRunLauncher::new()).expect("spawn the router");
    let handle = router.handle();

    // Two links finish before the window comes up: the router has recorded
    // both, and both are waiting on the channel.
    let early: Vec<DispatchRecord> = ["first", "second"]
        .into_iter()
        .map(|path| {
            let link = Link::new(format!("https://a.example/{path}"), Source::CommandLine);
            handle.dispatch_and_wait(link).expect("routed")
        })
        .collect();
    assert_eq!(early[1].outcome, Outcome::DryRun);
    assert_eq!(handle.recent().len(), 2);

    let feed = cx.update(|cx| {
        activity::install(handle.clone(), dispatched, cx);
        activity::feed(cx).expect("installed")
    });
    let ids = |cx: &mut TestAppContext| {
        feed.read_with(cx, |feed, _| {
            feed.records().iter().map(|r| r.id).collect::<Vec<_>>()
        })
    };
    let (first, second) = (early[0].id, early[1].id);
    assert_eq!(ids(cx), [second, first], "seeded once each, newest first");
    cx.run_until_parked();
    assert_eq!(ids(cx), [second, first], "the pump does not add them again");

    // A dispatch after install still arrives through the pump. It is sent
    // from this thread: the test scheduler forbids the dispatch thread
    // waking a task, so the router's own callback cannot be used here.
    let mut later = early[1].clone();
    later.id = DispatchId(second.0 + 1);
    sender.try_send(later.clone()).expect("open");
    cx.run_until_parked();
    assert_eq!(ids(cx), [later.id, second, first]);
    let today = feed.read_with(cx, |feed, _| feed.today(std::time::SystemTime::now()));
    assert_eq!((today.routed, today.failed), (3, 0));
}

#[gpui_kit::test]
fn switching_a_rule_off_changes_test_a_link(cx: &mut TestAppContext) {
    let dir = scratch("switch-off");
    let (view, cx) = open_store(cx, None, fixture_store(&dir));
    // Rewrite 1 (X to xcancel) off, from its editor.
    cx.simulate_keystrokes("up up up");
    draw(cx);
    assert_eq!(
        view.read_with(cx, |app, _| app.selection),
        Some(Selection::Rewrite(0))
    );
    click(cx, "handler-enabled");
    // The switch is saved, after the debounce.
    assert!(view.read_with(cx, |app, _| app.store.needs_save()));
    assert!(!saved(cx, &dir.join("rules.json")).rewrites[0].enabled);
    assert!(view.read_with(cx, |app, _| !app.store.needs_save()));
    click(cx, "nav-test");
    type_sample(cx);
    let off = trace(&view, cx);
    assert_eq!(off.result.url, "https://x.com/pwrdrvr/status/1842?s=20");
    assert!(titles(&off).contains(&"Rewrite 2"));

    // Show each is for one trace: the next test folds again.
    click(cx, "trace-show-each");
    assert!(titles(&trace(&view, cx)).contains(&"Handler 2"));
    click(cx, "test-run");
    assert!(titles(&trace(&view, cx)).contains(&"Handlers 1 and 2"));
}

#[gpui_kit::test]
fn test_a_link_reloads_a_changed_config(cx: &mut TestAppContext) {
    let dir = std::env::temp_dir().join(format!("pwrroute-app-reload-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temp dir");
    let config = dir.join("finicky.js");
    let write = |browser: &str| {
        std::fs::write(
            &config,
            format!("module.exports = {{ defaultBrowser: \"{browser}\", handlers: [] }};\n"),
        )
        .expect("write the config");
    };
    write("Firefox");
    let (view, cx) = open_with(cx, Some(config.clone()));
    click(cx, "nav-test");
    type_sample(cx);
    assert_eq!(trace(&view, cx).result.title, "Firefox");

    // A later modification time, as an edit gives it.
    write("Safari");
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&config)
        .and_then(|file| file.set_modified(later))
        .expect("touch the config");
    click(cx, "test-run");
    assert_eq!(trace(&view, cx).result.title, "Safari");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_finicky_config_in_charge_is_read_only(cx: &mut TestAppContext) {
    let config = fixture_home().join(".finicky.ts");
    let (view, cx) = open_with(cx, Some(config));
    let config = view
        .read_with(cx, |app, _| app.config.clone())
        .expect("the config is in charge");
    assert_eq!(config.path, "~/.finicky.ts");
    assert_eq!(config.need_js(), 2);
    // The rows are the engine's reading of the file, not fixtures: the
    // functions' source, the patterns and browsers in the file's order.
    let rows: Vec<(Vec<String>, String)> = config
        .handlers
        .iter()
        .map(|rule| {
            (
                rule.matchers.iter().map(matcher).collect(),
                browser(&rule.destination),
            )
        })
        .collect();
    let row = |matchers: &[&str], browser: &str| {
        (
            matchers.iter().map(|m| (*m).to_owned()).collect::<Vec<_>>(),
            browser.to_owned(),
        )
    };
    assert_eq!(
        rows,
        [
            row(
                &["fn (url, { opener }) => url.host.includes(\"workplace\")"],
                "Firefox"
            ),
            row(
                &["fn (url, { opener }) => opener?.name.includes(\"Slack\") || false"],
                "Firefox"
            ),
            row(&["bsky.app/*"], "Firefox"),
            row(&["google.com/*", "*.google.com*"], "Google Chrome"),
        ]
    );
    assert!(config.rewrites.is_empty());
    assert_eq!(
        config.default_browser.as_ref().map(browser).as_deref(),
        Some("Google Chrome")
    );
    assert!(cx.debug_bounds("config-handlers").is_some());
    // No editor until the visual rules are expanded.
    assert!(cx.debug_bounds("editor-rail").is_none());
    click(cx, "visual-rules-toggle");
    assert!(cx.debug_bounds("editor-rail").is_some());

    // Test a link runs the config first: its Slack function handler wins.
    click(cx, "nav-test");
    type_sample(cx);
    let slack = trace(&view, cx);
    assert_eq!(slack.result.title, "Firefox");
    assert!(titles(&slack).contains(&"Config handler 2"));
}

#[gpui_kit::test]
fn a_config_that_does_not_load_says_why(cx: &mut TestAppContext) {
    let config = fixture_home().join("missing.finicky.ts");
    let (view, cx) = open_with(cx, Some(config));
    let config = view
        .read_with(cx, |app, _| app.config.clone())
        .expect("the config is in charge");
    let problem = config.problem.as_deref().unwrap_or_default();
    assert!(problem.starts_with("It did not load:"), "{problem}");
    assert!(!problem.contains('\n'), "{problem}");
    assert!(config.handlers.is_empty() && config.default_browser.is_none());
    // No Handlers card claiming the config has none.
    assert!(cx.debug_bounds("config-handlers").is_none());
}

#[gpui_kit::test]
fn editing_a_handler_writes_the_file(cx: &mut TestAppContext) {
    let dir = scratch("edit");
    let path = dir.join("rules.json");
    let (view, cx) = open_store(cx, None, fixture_store(&dir));
    // Handler 1's first pattern, retyped.
    click(cx, "pattern-0");
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("calendar.google.com/*");
    draw(cx);
    let handler = |cx: &mut VisualTestContext| view.read_with(cx, |app, _| app.handlers[0].clone());
    assert_eq!(
        handler(cx).patterns,
        ["calendar.google.com/*", "*.google.com/*"]
    );
    assert_eq!(handler(cx).title, "calendar.google.com/*");
    assert_eq!(handler(cx).rule.badge, RowBadge::None);
    // The rail still has the field focused: the rows redrew around it.
    assert!(cx.debug_bounds("pattern-0").is_some());
    assert_eq!(
        saved(cx, &path).rules[0].patterns,
        ["calendar.google.com/*", "*.google.com/*"]
    );
    let label = view.read_with(cx, |app, _| app.store.status_label());
    assert!(label.starts_with("saved "), "{label}");

    // The profile, and the app the link must come from.
    click(cx, "profile");
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("Personal");
    click(cx, "opener-add");
    cx.simulate_input("com.apple.mail");
    draw(cx);
    let file = saved(cx, &path);
    assert_eq!(file.rules[0].profile, "Personal");
    assert_eq!(file.rules[0].opener, ["com.apple.mail"]);
    // The row shows the app before the patterns.
    assert!(matches!(handler(cx).rule.matchers[1], Matcher::App { .. }));

    // Clearing the browser is a problem: the row says so, the file keeps
    // the saved browser, and Test a link routes by the saved version.
    click(cx, "browser");
    cx.simulate_keystrokes("cmd-a backspace");
    draw(cx);
    assert_eq!(handler(cx).rule.badge, RowBadge::NotSaved);
    assert_eq!(handler(cx).problems, ["Select a browser."]);
    assert_eq!(
        view.read_with(cx, |app, _| app.store.rail_label(Selection::Handler(0))),
        "1 problem \u{00b7} not saved"
    );
    assert_eq!(saved(cx, &path).rules[0].browser, "Google Chrome");
    assert_eq!(
        view.read_with(cx, |app, _| app.test.rules().rules[0].browser.clone()),
        "Google Chrome"
    );
    cx.simulate_input("Firefox");
    draw(cx);
    assert_eq!(handler(cx).rule.badge, RowBadge::None);
    assert_eq!(saved(cx, &path).rules[0].browser, "Firefox");
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn a_new_handler_is_a_draft_until_it_is_complete(cx: &mut TestAppContext) {
    let dir = scratch("add");
    let path = dir.join("rules.json");
    let (view, cx) = open_store(cx, None, fixture_store(&dir));
    click(cx, "add-handler");
    assert_eq!(
        view.read_with(cx, |app, _| app.selection),
        Some(Selection::Handler(7))
    );
    assert!(cx.debug_bounds("handler-7").is_some());
    let handler = |cx: &mut VisualTestContext| view.read_with(cx, |app, _| app.handlers[7].clone());
    assert_eq!(handler(cx).rule.badge, RowBadge::Draft);
    assert_eq!(handler(cx).rule.hits, "draft");
    assert_eq!(handler(cx).title, "New handler");
    // The new pattern field has the keyboard.
    cx.simulate_input("notion.so/*");
    draw(cx);
    assert_eq!(handler(cx).patterns, ["notion.so/*"]);
    assert_eq!(handler(cx).title, "notion.so/*");
    // Not in the file yet: it has no browser.
    assert_eq!(saved(cx, &path).rules.len(), 7);
    assert_eq!(
        view.read_with(cx, |app, _| app.store.rail_label(Selection::Handler(7))),
        "saves once it has a pattern and a browser"
    );
    // Try it says the draft would take its sample once it is complete.
    click(cx, "browser");
    cx.simulate_input("Firefox");
    draw(cx);
    assert_eq!(handler(cx).rule.badge, RowBadge::None);
    let result = view.read_with(cx, |app, _| app.rail.as_ref().unwrap().try_result.clone());
    assert_eq!(
        result,
        Some(crate::try_it::TryResult::Text {
            verdict: crate::try_it::Verdict::Takes,
            message: "This handler opens that link in Firefox.".into(),
            bold: None,
        })
    );
    let file = saved(cx, &path);
    assert_eq!(file.rules.len(), 8);
    assert_eq!(file.rules[7].patterns, ["notion.so/*"]);
    assert_eq!(file.rules[7].browser, "Firefox");

    // A rewrite with a change of another kind.
    click(cx, "add-rewrite");
    assert_eq!(
        view.read_with(cx, |app, _| app.selection),
        Some(Selection::Rewrite(3))
    );
    cx.simulate_input("http://*.corp/*");
    click(cx, "change-add");
    click(cx, "change-0-kind");
    click(cx, "change-0-kind-1");
    click(cx, "change-0");
    cx.simulate_input("https");
    draw(cx);
    let file = saved(cx, &path);
    assert_eq!(file.rewrites.len(), 4);
    assert_eq!(
        file.rewrites[3].actions,
        [pwrroute_engine::RewriteAction::SetScheme("https".into())]
    );

    // Delete takes the rewrite out of the file; the row that takes its
    // place is selected.
    click(cx, "rail-delete");
    assert_eq!(
        view.read_with(cx, |app, _| app.selection),
        Some(Selection::Rewrite(2))
    );
    assert_eq!(saved(cx, &path).rewrites.len(), 3);
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn with_nothing_to_route_by_the_pane_offers_a_first_handler(cx: &mut TestAppContext) {
    let dir = scratch("blank");
    let path = dir.join("rules.json");
    let (view, cx) = open_store(cx, None, RulesStore::load(Some(path.clone())));
    assert!(view.read_with(cx, |app, _| app.nothing_to_route_by()));
    assert!(cx.debug_bounds("rules-blank").is_some());
    assert!(cx.debug_bounds("editor-rail").is_none());
    assert!(cx.debug_bounds("handler-0").is_none());

    click(cx, "blank-add-handler");
    assert!(cx.debug_bounds("rules-blank").is_none());
    assert!(cx.debug_bounds("editor-rail").is_some());
    assert!(cx.debug_bounds("handler-0").is_some());
    // The rewrites card is empty, with its own Add.
    assert!(cx.debug_bounds("empty-add-rewrite").is_some());
    // A draft is nothing to save; the file is written, empty, once the
    // debounce runs.
    assert!(saved(cx, &path).rules.is_empty());
    assert!(path.is_file());

    // Discarding the draft leaves two empty cards, not the blank: the file
    // exists now.
    click(cx, "rail-delete");
    assert!(cx.debug_bounds("rules-blank").is_none());
    assert!(cx.debug_bounds("empty-add-handler").is_some());
    assert_eq!(view.read_with(cx, |app, _| app.selection), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn a_file_with_only_a_default_shows_empty_cards(cx: &mut TestAppContext) {
    let dir = scratch("default-only");
    let path = dir.join("rules.json");
    RulesFile {
        default_browser: "Firefox".into(),
        ..RulesFile::default()
    }
    .save(&path)
    .unwrap();
    let (view, cx) = open_store(cx, None, RulesStore::load(Some(path.clone())));
    assert!(!view.read_with(cx, |app, _| app.nothing_to_route_by()));
    assert!(cx.debug_bounds("empty-add-rewrite").is_some());
    assert!(cx.debug_bounds("empty-add-handler").is_some());
    assert_eq!(
        view.read_with(cx, |app, _| app.store.default_browser().1),
        "Firefox"
    );
    click(cx, "empty-add-rewrite");
    assert_eq!(
        view.read_with(cx, |app, _| app.selection),
        Some(Selection::Rewrite(0))
    );
    // Test a link resolves this file: everything goes to Firefox.
    click(cx, "nav-test");
    type_sample(cx);
    assert_eq!(trace(&view, cx).result.title, "Firefox");
    let _ = std::fs::remove_dir_all(dir);
}

/// The fixture records plus one kept at host only and one kept at nothing,
/// reduced the way the router reduces them before the feed sees them.
fn leveled_activity() -> Vec<DispatchRecord> {
    use pwrroute_router::Privacy;
    let mut records = fixtures::activity();
    let mut host = records[0].clone();
    host.id = DispatchId(901);
    host.privacy = Privacy::Host;
    host.reduce(Privacy::Host);
    let mut none = records[1].clone();
    none.id = DispatchId(902);
    none.privacy = Privacy::None;
    none.reduce(Privacy::None);
    records.insert(0, host);
    records.insert(0, none);
    records
}

#[gpui_kit::test]
fn activity_filters_by_privacy_level_and_says_what_is_kept(cx: &mut TestAppContext) {
    let records = leveled_activity();
    let total = records.len();
    cx.update(|cx| activity::install_feed(Feed::from_records(records), cx));
    let (view, cx) = open(cx);
    click(cx, "nav-activity");

    // The record kept at nothing is a count, not a row; the host-only one
    // is a row with its host and nothing more.
    let rows = view.read_with(cx, |app, _| app.activity.rows.clone());
    assert_eq!(rows.len(), total - 1);
    assert_eq!(view.read_with(cx, |app, _| app.activity.kept_nothing), 1);
    assert_eq!(rows[0].privacy, pwrroute_router::Privacy::Host);
    assert_eq!(
        rows[0].url,
        pwrroute_router::host_only(&fixtures::activity()[0].link.url)
    );
    let counts = view.read_with(cx, |app, _| app.activity.counts);
    assert_eq!(counts.routed, total as u64);
    assert_eq!(counts.private, 2);
    assert_eq!(view.read_with(cx, |app, _| app.activity.today.private), 2);
    // The retention sentence is the router's (here the defaults), never a
    // literal of the screen's.
    assert_eq!(
        view.read_with(cx, |app, _| app.activity.retention.description.clone()),
        Options::default().retention_description()
    );
    assert!(cx.debug_bounds("activity-retention").is_some());

    // Private: the count cards and only the host-only rows.
    click(cx, "activity-filter-Private");
    assert_eq!(
        view.read_with(cx, |app, _| app.activity.filter),
        Filter::Private
    );
    let cards = view.read_with(cx, |app, _| app.activity.private_counts());
    assert_eq!(cards.len(), 2);
    assert_eq!((cards[0].count, cards[1].count), (1, 1));
    assert_eq!(cards[1].rule, "kept nothing");
    assert!(cx.debug_bounds("activity-row-0").is_some());
    assert!(cx.debug_bounds("activity-row-1").is_none());

    // Unrouted: the rows the default took, and no others.
    click(cx, "activity-filter-Unrouted");
    let unrouted: Vec<usize> = view.read_with(cx, |app, _| {
        app.activity
            .visible()
            .into_iter()
            .map(|(index, _)| index)
            .collect()
    });
    assert!(!unrouted.is_empty());
    assert_eq!(unrouted.len() as u64, counts.unrouted);
    for index in &unrouted {
        assert!(
            cx.debug_bounds(format!("activity-row-{index}").leak())
                .is_some(),
            "row {index}"
        );
    }
    assert!(cx.debug_bounds("activity-row-0").is_none());

    // A host-only row expands to its trace, with nothing to re-test.
    click(cx, "activity-filter-All");
    click(cx, "activity-row-0");
    assert_eq!(
        view.read_with(cx, |app, _| app.activity.expanded.map(|key| key.id)),
        Some(DispatchId(901))
    );
    assert!(cx.debug_bounds("activity-retest-0").is_none());
    assert!(cx.debug_bounds("activity-add-rule-0").is_none());
    // A full row offers Add a rule: the sheet, over this screen.
    click(cx, "activity-row-1");
    assert!(cx.debug_bounds("activity-add-rule-1").is_some());
    click(cx, "activity-add-rule-1");
    assert_eq!(view.read_with(cx, |app, _| app.screen), Screen::Activity);
    assert!(cx.debug_bounds("add-rule-sheet").is_some());
    click(cx, "add-rule-cancel");
    assert!(cx.debug_bounds("add-rule-sheet").is_none());
    // The row's "⋯" opens its menu and leaves the row as it was; the next
    // click closes it.
    click(cx, "activity-more-1");
    assert!(cx.debug_bounds("activity-menu-1").is_some());
    assert_eq!(
        view.read_with(cx, |app, _| app.actions.menu.as_ref().map(|menu| menu.key)),
        view.read_with(cx, |app, _| app.activity.expanded)
    );
    click(cx, "activity-more-1");
    assert!(cx.debug_bounds("activity-menu-1").is_none());
    assert!(view.read_with(cx, |app, _| app.activity.expanded.is_some()));
}

/// The Add a rule sheet's browser list is as wide as its field, and a
/// browser with many profiles or a long name stays inside it: the hint is
/// a count and ends before the list's edge. The profile list names every
/// profile, each inside the list.
#[gpui_kit::test]
fn the_add_rule_browser_list_stays_inside_its_field(cx: &mut TestAppContext) {
    let dir = scratch("add-rule-lists");
    cx.update(|cx| activity::install_feed(Feed::from_records(fixtures::activity()), cx));
    let (view, cx) = open_store(cx, None, fixture_store(&dir));
    let many: Vec<String> = [
        "Research and Development Team",
        "Personal",
        "Work",
        "Design",
        "Spanish",
    ]
    .iter()
    .map(|name| (*name).to_owned())
    .collect();
    view.update(cx, |app, _| {
        app.browsers = vec![
            BrowserChoice {
                name: "Google Chrome".into(),
                profiles: vec!["Personal".into(), "Work".into()],
            },
            BrowserChoice {
                name: "Microsoft Edge".into(),
                profiles: many.clone(),
            },
            BrowserChoice {
                name: "Example Browser With A Very Long Product Name For Testing".into(),
                profiles: vec!["A profile with a long name".into(); 3],
            },
        ];
    });
    click(cx, "nav-activity");
    click(cx, "activity-row-4");
    click(cx, "activity-add-rule-4");
    click(cx, "add-rule-browser");
    let field = cx.debug_bounds("add-rule-browser").expect("the field");
    let menu = cx.debug_bounds("add-rule-browser-menu").expect("the list");
    assert_eq!(menu.left(), field.left(), "{menu:?} under {field:?}");
    assert_eq!(menu.right(), field.right(), "{menu:?} as wide as {field:?}");
    for index in 0..3 {
        let row = cx
            .debug_bounds(format!("add-rule-browser-choice-{index}").leak())
            .expect("a row");
        assert!(row.right() <= menu.right(), "row {index}: {row:?}");
        let hint = cx
            .debug_bounds(format!("add-rule-browser-choice-{index}-hint").leak())
            .expect("a hint");
        assert!(
            hint.right() <= row.right(),
            "hint {index}: {hint:?} in {row:?}"
        );
    }

    // Edge's five profiles, each in its own row of the profile list.
    click(cx, "add-rule-browser-choice-1");
    click(cx, "add-rule-profile");
    let field = cx.debug_bounds("add-rule-profile").expect("the field");
    let menu = cx.debug_bounds("add-rule-profile-menu").expect("the list");
    assert_eq!((menu.left(), menu.right()), (field.left(), field.right()));
    for index in 0..=many.len() {
        let row = cx
            .debug_bounds(format!("add-rule-profile-choice-{index}").leak())
            .expect("a profile row");
        assert!(row.right() <= menu.right(), "profile {index}: {row:?}");
    }
}

/// A list longer than `MENU_MAX_HEIGHT` stops there and scrolls, and its
/// rows keep their height instead of being squeezed to fit.
#[gpui_kit::test]
fn a_long_browser_list_scrolls_with_whole_rows(cx: &mut TestAppContext) {
    let dir = scratch("add-rule-long-list");
    cx.update(|cx| activity::install_feed(Feed::from_records(fixtures::activity()), cx));
    let (view, cx) = open_store(cx, None, fixture_store(&dir));
    view.update(cx, |app, _| {
        app.browsers = (0..14)
            .map(|n| BrowserChoice {
                name: format!("Example Browser {n}"),
                profiles: vec!["Personal".into(), "Work".into()],
            })
            .collect();
    });
    click(cx, "nav-activity");
    click(cx, "activity-row-4");
    click(cx, "activity-add-rule-4");
    click(cx, "add-rule-browser");
    let menu = cx.debug_bounds("add-rule-browser-menu").expect("the list");
    assert!(
        menu.size.height <= px(crate::widgets::MENU_MAX_HEIGHT),
        "{menu:?}"
    );
    let first = cx
        .debug_bounds("add-rule-browser-choice-0")
        .expect("a row")
        .size
        .height;
    assert!(first > px(20.0), "{first:?}");
    for index in 1..14 {
        let row = cx
            .debug_bounds(format!("add-rule-browser-choice-{index}").leak())
            .expect("a row");
        assert_eq!(row.size.height, first, "row {index} keeps its height");
    }
    // The last rows lie past the card: the list scrolls to them.
    let last = cx
        .debug_bounds("add-rule-browser-choice-13")
        .expect("the last row");
    assert!(last.bottom() > menu.bottom(), "{last:?} past {menu:?}");
}

/// The row menu's Save hint names the browser and a long profile, and ends
/// at the menu's edge instead of running past it.
#[gpui_kit::test]
fn the_row_menus_save_hint_ends_inside_the_menu(cx: &mut TestAppContext) {
    let dir = scratch("row-menu-hint");
    cx.update(|cx| activity::install_feed(Feed::from_records(fixtures::activity()), cx));
    let (view, cx) = open_store(cx, None, fixture_store(&dir));
    click(cx, "nav-activity");
    click(cx, "activity-more-1");
    view.update(cx, |app, cx| {
        let menu = app.actions.menu.as_mut().expect("the row menu");
        menu.save = Some(Save::Test {
            expects: "Microsoft Edge \u{00b7} Research and Development Team Profile".into(),
        });
        cx.notify();
    });
    draw(cx);
    let menu = cx.debug_bounds("activity-menu-1").expect("the menu");
    let row = cx.debug_bounds("activity-save-1").expect("Save as test");
    let hint = cx.debug_bounds("activity-save-1-hint").expect("its hint");
    assert!(row.right() <= menu.right(), "{row:?} in {menu:?}");
    assert!(hint.right() <= row.right(), "{hint:?} in {row:?}");
}

/// With the journal on, records that fell out of memory are listed below
/// the in-memory ones, read from the journal at the level it kept.
#[gpui_kit::test]
fn activity_lists_journaled_records_below_the_in_memory_ones(cx: &mut TestAppContext) {
    use pwrroute_router::{JournalConfig, JournalMode, Retention};
    let dir = scratch("journal");
    let (dispatched_tx, dispatched) = async_channel::unbounded();
    // Held here: the router's own copy goes with its thread, and a channel
    // closed from there would wake the feed's pump off the test thread.
    let _sender = dispatched_tx.clone();
    let options = Options {
        journal: Some(JournalConfig::new(dir.join("journal"), JournalMode::Hosts)),
        on_dispatch: Some(std::sync::Arc::new(move |record: &DispatchRecord| {
            let _ = dispatched_tx.try_send(record.clone());
        })),
        ..Options::default()
    };
    let router = Router::spawn(options, None, DryRunLauncher::new()).expect("spawn the router");
    let handle = router.handle();
    let ids: Vec<DispatchId> = ["one", "two", "three"]
        .into_iter()
        .map(|path| {
            let link = Link::new(format!("https://a.example/{path}?q=1"), Source::CommandLine);
            handle.dispatch_and_wait(link).expect("routed").id
        })
        .collect();
    // Memory keeps the newest only from now on; the journal has all three.
    handle
        .set_retention(Retention {
            count: 1,
            hours: 24,
        })
        .unwrap();
    assert_eq!(handle.recent().len(), 1);
    assert_eq!(handle.journal(10).unwrap().len(), 3);

    cx.update(|cx| activity::install(handle.clone(), dispatched, cx));
    let (view, cx) = open(cx);
    click(cx, "nav-activity");
    let keys = |cx: &mut VisualTestContext| {
        view.read_with(cx, |app, _| {
            (
                app.activity.rows.iter().map(|r| r.key).collect::<Vec<_>>(),
                app.activity
                    .journal_rows
                    .iter()
                    .map(|r| r.key)
                    .collect::<Vec<_>>(),
            )
        })
    };
    let (memory, journaled) = keys(cx);
    assert_eq!(memory.len(), 1);
    assert_eq!(memory[0].id, ids[2]);
    assert!(!memory[0].journaled);
    assert_eq!(
        journaled.iter().map(|key| key.id).collect::<Vec<_>>(),
        vec![ids[1], ids[0]]
    );
    assert!(journaled.iter().all(|key| key.journaled));
    // Journaled at hosts only: the row shows the host, whatever memory
    // kept.
    let rows = view.read_with(cx, |app, _| app.activity.journal_rows.clone());
    assert_eq!(rows[0].url, "https://a.example");
    assert_eq!(rows[0].privacy, pwrroute_router::Privacy::Host);
    let in_memory = view.read_with(cx, |app, _| app.activity.rows[0].url.clone());
    assert_eq!(in_memory, "https://a.example/three?q=1");
    // The toolbar and the footer say so, in the router's words.
    let retention = view.read_with(cx, |app, _| app.activity.retention.clone());
    assert!(retention.journal.is_some());
    assert!(
        retention
            .description
            .contains("journaled as scheme and host only")
    );
    assert_eq!(
        activity::memory_note(&retention),
        "last 1 \u{00b7} 24 h \u{00b7} in memory \u{00b7} journaled"
    );
    // Journaled rows are indexed after the in-memory ones and expand.
    assert!(cx.debug_bounds("activity-row-0").is_some());
    assert!(cx.debug_bounds("activity-row-2").is_some());
    click(cx, "activity-row-2");
    assert_eq!(
        view.read_with(cx, |app, _| app.activity.expanded),
        Some(journaled[1])
    );
    // Clear empties memory; the journal's rows stay until Settings clears
    // the journal.
    click(cx, "activity-clear");
    let (memory, journaled) = keys(cx);
    assert!(memory.is_empty());
    assert_eq!(journaled.len(), 3);
    drop(router);
}

// --- examples, tests and Activity's row actions ----------------------------

/// The fixture rules with examples on handler 2 and a saved test, written
/// to `dir`: every one passes.
fn rules_with_examples() -> RulesFile {
    let mut rules = fixtures::rules_file();
    rules.rules[1].examples = vec![
        "https://pwrdrvr.atlassian.net/browse/PG-12".into(),
        "https://github.com/pwrdrvr/PwrGit/pull/361".into(),
    ];
    rules.rewrites[0].examples = vec![pwrroute_engine::RewriteExample {
        url: "https://x.com/pwrdrvr/status/1842".into(),
        becomes: "https://xcancel.com/pwrdrvr/status/1842".into(),
    }];
    rules.tests = vec![RouteTest {
        url: "https://www.apple.com/macos/".into(),
        opener: None,
        expect: pwrroute_engine::Expectation {
            browser: "Google Chrome".into(),
            profile: Some("Personal".into()),
            url: None,
        },
    }];
    rules
}

fn store_for(dir: &Path, rules: &RulesFile) -> RulesStore {
    let path = dir.join("rules.json");
    rules.save(&path).expect("write the rules");
    RulesStore::load(Some(path))
}

#[gpui_kit::test]
fn example_chips_and_the_tests_strip_follow_the_file(cx: &mut TestAppContext) {
    let dir = scratch("examples");
    let (view, cx) = open_store(cx, None, store_for(&dir, &rules_with_examples()));
    // Run on open: every example and test passed.
    let report = |cx: &mut VisualTestContext| {
        view.read_with(cx, |app, _| {
            app.tests
                .report()
                .cloned()
                .expect("ran")
                .expect("the tests ran")
        })
    };
    assert_eq!(view.read_with(cx, |app, _| app.tests.counts()), (3, 1));
    assert_eq!(
        view.read_with(cx, |app, _| (
            app.tests.ran_at().is_some(),
            app.store.needs_save()
        )),
        (true, false)
    );
    assert_eq!((report(cx).total, report(cx).passed), (4, 4));
    assert!(cx.debug_bounds("tests-strip").is_some());
    assert!(cx.debug_bounds("handler-1-examples").is_some());
    assert!(cx.debug_bounds("rewrite-0-examples").is_some());
    assert!(cx.debug_bounds("handler-0-examples").is_none());
    assert!(cx.debug_bounds("saved-test-0").is_some());
    assert!(cx.debug_bounds("failure-edit-0").is_none());

    // A wider handler added above takes handler 2's GitHub example.
    view.update(cx, |app, cx| {
        app.store.insert_rule(
            0,
            pwrroute_engine::Rule {
                patterns: vec!["github.com/*".into()],
                browser: "Firefox".into(),
                ..pwrroute_engine::Rule::default()
            },
        );
        app.refresh(cx);
    });
    draw(cx);
    // Stale until the save runs the tests again.
    assert!(view.read_with(cx, |app, _| app.tests.report().is_none()));
    // The debounce timer starts when the save task first runs.
    cx.run_until_parked();
    let on_disk = saved(cx, &dir.join("rules.json"));
    assert_eq!(on_disk.rules.len(), 8, "the save ran");
    assert_eq!(
        view.read_with(cx, |app, _| (
            app.store.needs_save(),
            app.tests.counts(),
            app.tests.report().is_some()
        )),
        (false, (3, 1), true)
    );
    draw(cx);
    let after = report(cx);
    assert_eq!((after.total, after.passed), (4, 3));
    assert_eq!(
        after.failures[0].input,
        "https://github.com/pwrdrvr/PwrGit/pull/361"
    );
    assert_eq!(
        after.failures[0].source,
        TestSource::RuleExample {
            rule: 2,
            example: 1
        }
    );
    assert_eq!(
        view.read_with(cx, |app, _| app.tests.passed(TestSource::RuleExample {
            rule: 2,
            example: 1
        })),
        Some(false)
    );
    // The failure row's Edit handler 1 selects the winner.
    click(cx, "failure-edit-0");
    assert_eq!(
        view.read_with(cx, |app, _| app.selection),
        Some(Selection::Handler(0))
    );
    // Run tests runs them again, on demand.
    // The report goes stale when the rules change under it.
    view.update(cx, |app, _| {
        let mut changed = app.store.edited();
        changed.default_browser = "Firefox".into();
        app.tests.set_rules(changed);
    });
    assert!(view.read_with(cx, |app, _| app.tests.report().is_none()));
    draw(cx);
    click(cx, "run-tests");
    assert_eq!(report(cx).total, 4);
    // Removing the saved test takes its card away.
    scroll_to(cx, "test-remove-0");
    click(cx, "test-remove-0");
    assert!(view.read_with(cx, |app, _| app.store.tests().is_empty()));
    assert!(cx.debug_bounds("saved-test-0").is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn the_editor_adds_examples_and_sets_private_and_keep(cx: &mut TestAppContext) {
    let dir = scratch("editor-examples");
    let path = dir.join("rules.json");
    let (view, cx) = open_store(cx, None, fixture_store(&dir));
    // Handler 1: an example, on Return. It is written with the rule.
    click(cx, "example-add");
    cx.simulate_input("https://meet.google.com/abc-defg-hij");
    cx.simulate_keystrokes("enter");
    draw(cx);
    let handler = |cx: &mut VisualTestContext| view.read_with(cx, |app, _| app.handlers[0].clone());
    assert_eq!(
        handler(cx).examples,
        ["https://meet.google.com/abc-defg-hij"]
    );
    assert_eq!(
        view.read_with(cx, |app, _| app.rail.as_ref().unwrap().example.text.clone()),
        "",
        "the field clears once the example is taken"
    );
    assert!(cx.debug_bounds("example-remove-0").is_some());
    assert!(cx.debug_bounds("handler-0-examples").is_some());
    assert_eq!(
        saved(cx, &path).rules[0].examples,
        ["https://meet.google.com/abc-defg-hij"]
    );
    draw(cx);
    // Saved, so the tests ran: the example resolves here.
    assert_eq!(
        view.read_with(cx, |app, _| app.tests.passed(TestSource::RuleExample {
            rule: 0,
            example: 0
        })),
        Some(true)
    );
    assert!(cx.debug_bounds("tests-strip").is_some());

    // Private window and Keep write `private` and `privacy`.
    click(cx, "handler-private");
    assert!(handler(cx).private);
    assert_eq!(handler(cx).keep(), Some(Privacy::Host));
    assert!(
        handler(cx)
            .rule
            .matchers
            .contains(&Matcher::Privacy("private \u{00b7} host only".into()))
    );
    click(cx, "keep-none");
    assert_eq!(handler(cx).keep(), Some(Privacy::None));
    let file = saved(cx, &path);
    assert!(file.rules[0].private);
    assert_eq!(file.rules[0].privacy, Some(Privacy::None.into()));
    click(cx, "handler-private");
    click(cx, "keep-full");
    let file = saved(cx, &path);
    assert!(!file.rules[0].private);
    assert_eq!(file.rules[0].privacy, Some(Privacy::Full.into()));

    // A bad example is a problem under the field; removing it clears it.
    click(cx, "example-add");
    cx.simulate_input("meet.google.com/x");
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(handler(cx).rule.badge, RowBadge::NotSaved);
    assert!(handler(cx).problems[0].starts_with("Examples must be full URLs"));
    click(cx, "example-remove-1");
    assert_eq!(handler(cx).rule.badge, RowBadge::None);

    // A rewrite's example says what the URL becomes.
    cx.simulate_keystrokes("up up up");
    draw(cx);
    assert_eq!(
        view.read_with(cx, |app, _| app.selection),
        Some(Selection::Rewrite(0))
    );
    click(cx, "example-add");
    cx.simulate_input("https://x.com/pwrdrvr/status/1842");
    cx.simulate_keystrokes("enter");
    draw(cx);
    // Not taken without what it becomes.
    assert!(view.read_with(cx, |app, _| app.rewrites[0].examples.is_empty()));
    click(cx, "example-becomes");
    cx.simulate_input("https://xcancel.com/pwrdrvr/status/1842");
    cx.simulate_keystrokes("enter");
    draw(cx);
    let file = saved(cx, &path);
    assert_eq!(
        file.rewrites[0].examples,
        [pwrroute_engine::RewriteExample {
            url: "https://x.com/pwrdrvr/status/1842".into(),
            becomes: "https://xcancel.com/pwrdrvr/status/1842".into(),
        }]
    );
    draw(cx);
    assert_eq!(
        view.read_with(cx, |app, _| app.tests.passed(TestSource::RewriteExample {
            rewrite: 0,
            example: 0
        })),
        Some(true)
    );
    assert!(cx.debug_bounds("rewrite-0-examples").is_some());
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn a_rewrite_with_a_bad_privacy_word_shows_the_problem(cx: &mut TestAppContext) {
    let dir = scratch("rewrite-privacy-problem");
    let mut rules = fixtures::rules_file();
    rules.rewrites[0].privacy = Some(PrivacySetting::Unknown("hidden".into()));
    let (view, cx) = open_store(cx, None, store_for(&dir, &rules));
    click(cx, "rewrite-0");
    draw(cx);
    let rewrite = view.read_with(cx, |app, _| app.rewrites[0].clone());
    assert_eq!(rewrite.problems, ["Privacy must be full, host or none."]);
    assert_eq!(rewrite.rule.badge, RowBadge::NotSaved);
    // The rewrite editor has no Keep control, so the row is drawn under
    // Changes rather than nowhere.
    assert!(cx.debug_bounds("rewrite-keep-problems").is_some());
    assert_eq!(
        view.read_with(cx, |app, _| app.store.rail_label(Selection::Rewrite(0))),
        "1 problem \u{00b7} not saved"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn test_a_link_saves_the_run_as_a_test(cx: &mut TestAppContext) {
    let dir = scratch("save-as-test");
    let path = dir.join("rules.json");
    let (view, cx) = open_store(cx, None, fixture_store(&dir));
    click(cx, "nav-test");
    assert!(cx.debug_bounds("test-save").is_none());
    type_sample(cx);
    assert!(cx.debug_bounds("test-save").is_some());
    click(cx, "test-save");
    let tests = view.read_with(cx, |app, _| app.store.tests().to_vec());
    assert_eq!(
        tests,
        [RouteTest {
            url: fixtures::TEST_URL.into(),
            opener: Some("Slack:com.tinyspeck.slackmacgap".into()),
            expect: pwrroute_engine::Expectation {
                browser: "Firefox".into(),
                profile: Some("default-release".into()),
                url: Some("https://xcancel.com/pwrdrvr/status/1842?s=20".into()),
            },
        }]
    );
    // The run stays on screen, with the banner and no second Save.
    assert!(cx.debug_bounds("test-trace").is_some());
    assert!(cx.debug_bounds("test-saved").is_some());
    assert!(cx.debug_bounds("test-save").is_none());
    assert_eq!(view.read_with(cx, |app, _| app.test.saved_as), Some(0));
    assert_eq!(saved(cx, &path).tests, tests);
    draw(cx);
    assert_eq!(
        view.read_with(cx, |app, _| app.tests.passed(TestSource::Test { test: 0 })),
        Some(true)
    );
    click(cx, "test-show-rules");
    assert_eq!(view.read_with(cx, |app, _| app.screen), Screen::Rules);
    assert!(cx.debug_bounds("saved-test-0").is_some());
    // The next run offers Save as test again.
    click(cx, "nav-test");
    click(cx, "test-run");
    assert!(cx.debug_bounds("test-save").is_some());
    assert!(cx.debug_bounds("test-saved").is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// A contrived record like the fixtures', with its own id, link and match.
fn record_like(base: &DispatchRecord, id: u64, url: &str, by: Matched) -> DispatchRecord {
    let mut record = base.clone();
    record.id = DispatchId(id);
    record.link.url = url.into();
    record.final_url = Some(url.into());
    if let Some(browser) = &mut record.browser {
        browser.url = url.into();
    }
    for step in &mut record.steps {
        match step {
            Step::Received { url: received } => *received = url.into(),
            Step::Matched { by: matched } => *matched = by,
            Step::Browser { browser } => browser.url = url.into(),
            _ => {}
        }
    }
    record
}

#[gpui_kit::test]
fn activity_adds_a_rule_from_a_record(cx: &mut TestAppContext) {
    let dir = scratch("add-rule");
    let path = dir.join("rules.json");
    let records = fixtures::activity();
    cx.update(|cx| activity::install_feed(Feed::from_records(records.clone()), cx));
    let (view, cx) = open_store(cx, None, fixture_store(&dir));
    view.update(cx, |app, _| {
        app.browsers = vec![
            BrowserChoice {
                name: "Google Chrome".into(),
                profiles: vec!["Personal".into(), "Work".into()],
            },
            BrowserChoice {
                name: "Firefox".into(),
                profiles: Vec::new(),
            },
        ];
    });
    click(cx, "nav-activity");
    assert!(cx.debug_bounds("add-rule-sheet").is_none());
    // Row 4 is the GitHub pull request, routed by handler 2.
    click(cx, "activity-row-4");
    assert!(cx.debug_bounds("activity-add-rule-4").is_some());
    click(cx, "activity-add-rule-4");
    assert!(cx.debug_bounds("add-rule-sheet").is_some());
    let sheet = |cx: &mut VisualTestContext| {
        view.read_with(cx, |app, _| {
            let sheet = app.actions.sheet.as_ref().expect("the sheet is open");
            (
                sheet.pattern.text.clone(),
                sheet.selected,
                sheet
                    .collisions
                    .iter()
                    .map(|c| c.url.clone())
                    .collect::<Vec<_>>(),
                sheet.collision_caption(),
            )
        })
    };
    let (pattern, selected, collisions, caption) = sheet(cx);
    assert_eq!(pattern, "github.com/pwrdrvr/*");
    assert_eq!(selected, Some(0));
    assert!(collisions.is_empty());
    assert_eq!(caption, "No other recent link matches this pattern.");
    assert!(cx.debug_bounds("add-rule-candidate-1").is_some());
    assert!(cx.debug_bounds("add-rule-only-from").is_some());

    // The host alone, then a pattern typed by hand: everything.
    click(cx, "add-rule-candidate-1");
    assert_eq!(sheet(cx).0, "github.com/*");
    click(cx, "add-rule-pattern");
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("*");
    draw(cx);
    let (pattern, selected, collisions, caption) = sheet(cx);
    assert_eq!(pattern, "*");
    assert_eq!(selected, None);
    // Every other full record that was routed; the failed launch is left out.
    assert_eq!(collisions.len(), 6, "{collisions:?}");
    assert!(collisions.contains(&"https://www.apple.com/macos/".to_owned()));
    assert!(!collisions.iter().any(|url| url.contains("zoom.us")));
    assert!(
        caption
            .starts_with("Would also catch these 6 recent links \u{00b7} 5 routed by a rule now"),
        "{caption}"
    );
    assert!(cx.debug_bounds("add-rule-collision-5").is_some());

    // Browser and profile from the detected browsers; the rule's privacy.
    click(cx, "add-rule-browser");
    click(cx, "add-rule-browser-choice-0");
    click(cx, "add-rule-profile");
    click(cx, "add-rule-profile-choice-2");
    click(cx, "add-rule-private");
    click(cx, "add-rule-keep-none");
    click(cx, "add-rule-position-top");
    let rule = view.read_with(cx, |app, _| {
        app.actions.sheet.as_ref().unwrap().rule().unwrap()
    });
    assert_eq!(rule.browser, "Google Chrome");
    assert_eq!(rule.profile, "Work");
    assert!(rule.opener.is_empty());
    assert!(rule.private);
    assert_eq!(rule.privacy, Some(Privacy::None.into()));
    assert_eq!(
        rule.examples,
        ["https://github.com/pwrdrvr/PwrGit/pull/361"]
    );
    // Only from the app that sent it: the link is saved as a test with
    // that app, since a rule's examples run from no app.
    click(cx, "add-rule-only-from");
    let (rule, test) = view.read_with(cx, |app, _| {
        let sheet = app.actions.sheet.as_ref().unwrap();
        (sheet.rule().unwrap(), sheet.test())
    });
    assert_eq!(
        rule.opener,
        ["PwrAgnt"],
        "the record's app, by name when it has no bundle id"
    );
    assert!(rule.examples.is_empty());
    let test = test.expect("a test with the opener");
    assert_eq!(test.url, "https://github.com/pwrdrvr/PwrGit/pull/361");
    assert_eq!(test.opener.as_deref(), Some("PwrAgnt"));
    assert_eq!(test.expect.browser, "Google Chrome");
    assert_eq!(test.expect.profile.as_deref(), Some("Work"));

    click(cx, "add-rule-save");
    assert!(cx.debug_bounds("add-rule-sheet").is_none());
    assert!(cx.debug_bounds("activity-action-note").is_some());
    let first = view.read_with(cx, |app, _| app.handlers[0].clone());
    assert_eq!(first.patterns, ["*"]);
    assert_eq!(first.openers, ["PwrAgnt"]);
    assert_eq!(view.read_with(cx, |app, _| app.handlers.len()), 8);
    assert_eq!(
        view.read_with(cx, |app, _| app.store.tests().to_vec()),
        [test]
    );
    // The tests ran: the saved test resolves to the new rule.
    assert_eq!(
        view.read_with(cx, |app, _| app.tests.passed(TestSource::Test { test: 0 })),
        Some(true)
    );
    let file = saved(cx, &path);
    assert_eq!(file.rules[0].patterns, ["*"]);
    assert_eq!(file.rules[0].opener, ["PwrAgnt"]);
    assert!(file.rules[0].private);
    assert_eq!(file.rules.len(), 8);
    assert_eq!(file.tests.len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn activity_saves_examples_and_tests_and_refuses_a_host_only_record(cx: &mut TestAppContext) {
    let dir = scratch("row-actions");
    let path = dir.join("rules.json");
    let mut records = fixtures::activity();
    let base = records[0].clone();
    // A rules.json handler took this one (the router does not number
    // those); an id in this one's path needs JS; this one kept host only.
    records.insert(
        0,
        record_like(
            &base,
            30,
            "https://meet.google.com/xyz",
            Matched::Unreported,
        ),
    );
    records.insert(
        0,
        record_like(
            &base,
            31,
            "https://issues.example/12345/comments",
            Matched::Default,
        ),
    );
    let mut host = record_like(&base, 32, "https://bank.example/login", Matched::Unreported);
    host.reduce(Privacy::Host);
    records.insert(0, host);
    cx.update(|cx| activity::install_feed(Feed::from_records(records), cx));
    let (view, cx) = open_store(cx, None, fixture_store(&dir));
    click(cx, "nav-activity");

    // Row 0, host only: the expanded row has no Add a rule, the menu
    // offers no Save, and its Add a rule opens the sheet's disabled state
    // with the reason.
    click(cx, "activity-row-0");
    assert!(cx.debug_bounds("activity-add-rule-0").is_none());
    click(cx, "activity-more-0");
    assert!(cx.debug_bounds("activity-menu-0").is_some());
    assert!(cx.debug_bounds("activity-save-0").is_none());
    click(cx, "activity-add-rule-menu-0");
    assert!(cx.debug_bounds("activity-menu-0").is_none());
    let disabled = view.read_with(cx, |app, _| {
        app.actions.sheet.as_ref().unwrap().disabled.clone()
    });
    assert!(
        disabled
            .as_deref()
            .is_some_and(|why| why.contains("keeps host only")),
        "{disabled:?}"
    );
    assert!(cx.debug_bounds("add-rule-save").is_none());
    click(cx, "add-rule-cancel");
    assert!(view.read_with(cx, |app, _| app.actions.sheet.is_none()));

    // Row 1, an id in the path: the sheet says it needs a JS handler, and
    // Add anyway still adds it.
    click(cx, "activity-row-1");
    click(cx, "activity-add-rule-1");
    let needs_js = view.read_with(cx, |app, _| {
        app.actions.sheet.as_ref().unwrap().needs_js.clone()
    });
    assert_eq!(needs_js.len(), 1);
    assert!(needs_js[0].contains("looks like an id"), "{needs_js:?}");
    click(cx, "add-rule-save");
    assert_eq!(
        view.read_with(cx, |app, _| app.handlers.last().unwrap().patterns.clone()),
        ["issues.example/12345/*"]
    );

    // Row 2, taken by a rules.json handler: the menu's Save names the
    // handler that takes it now (handler 1, meet.google.com/*) and the
    // example goes there; the menu stays, with the note in its footer.
    click(cx, "activity-more-2");
    assert_eq!(
        view.read_with(cx, |app, _| app.actions.menu.clone()),
        Some(RowMenu {
            key: RowKey {
                id: DispatchId(30),
                journaled: false
            },
            save: Some(Save::Example {
                handler: 0,
                pattern: Some("meet.google.com/*".into()),
            }),
        })
    );
    click(cx, "activity-save-2");
    assert_eq!(
        view.read_with(cx, |app, _| app.handlers[0].examples.clone()),
        ["https://meet.google.com/xyz"]
    );
    assert!(cx.debug_bounds("activity-action-note").is_some());
    let note = view.read_with(cx, |app, _| app.actions.note.clone().unwrap().1);
    assert!(
        note.starts_with("Saved to rules.json under handler 1."),
        "{note}"
    );

    // Row 9, apple.com, went to the default: Save as test pins that. The
    // "⋯" there closes row 2's menu and opens its own.
    click(cx, "activity-more-9");
    assert!(cx.debug_bounds("activity-menu-2").is_none());
    assert_eq!(
        view.read_with(cx, |app, _| app.actions.menu.clone().and_then(|m| m.save)),
        Some(Save::Test {
            expects: "Google Chrome \u{00b7} Personal".into()
        })
    );
    click(cx, "activity-save-9");
    let tests = view.read_with(cx, |app, _| app.store.tests().to_vec());
    assert_eq!(tests.len(), 1);
    assert_eq!(tests[0].url, "https://www.apple.com/macos/");
    assert_eq!(tests[0].opener.as_deref(), Some("Safari"));
    assert_eq!(tests[0].expect.browser, "Google Chrome");
    assert_eq!(tests[0].expect.profile.as_deref(), Some("Personal"));
    let file = saved(cx, &path);
    assert_eq!(file.tests, tests);
    assert_eq!(file.rules[0].examples, ["https://meet.google.com/xyz"]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Another writer's `rules.json`: `n` complete handlers, saved the way
/// `pwrroute_rules_apply` and an editor save it.
fn written_by_someone_else(path: &Path, n: usize) {
    RulesFile {
        default_browser: "Safari".into(),
        rules: (0..n)
            .map(|i| pwrroute_engine::Rule {
                patterns: vec![format!("h{i}.example/*")],
                browser: "Firefox".into(),
                ..pwrroute_engine::Rule::default()
            })
            .collect(),
        ..RulesFile::default()
    }
    .save(path)
    .expect("write rules.json");
}

#[gpui_kit::test]
fn a_changed_rules_json_reloads_or_offers_reload_and_overwrite(cx: &mut TestAppContext) {
    let dir = scratch("changed-on-disk");
    let path = dir.join("rules.json");
    let records = fixtures::activity();
    cx.update(|cx| activity::install_feed(Feed::from_records(records), cx));
    let (view, cx) = open_store(cx, None, fixture_store(&dir));
    let counts = |cx: &mut VisualTestContext| {
        view.read_with(cx, |app, _| {
            (
                app.handlers.len(),
                app.store.handler_count(),
                app.store.status_label(),
                app.store.needs_save(),
            )
        })
    };
    assert_eq!(counts(cx).0, 7);

    // Nothing unsaved: the window comes to the front and reads the file.
    written_by_someone_else(&path, 2);
    view.update(cx, |app, cx| {
        assert_eq!(app.sync_with_disk(cx), crate::rules_store::Disk::Reloaded);
    });
    draw(cx);
    let (rows, stored, label, dirty) = counts(cx);
    assert_eq!((rows, stored, dirty), (2, 2, false));
    assert!(label.starts_with("reloaded "), "{label}");
    assert!(cx.debug_bounds(crate::chrome::RELOAD_ID).is_none());
    // Handler 1 is still selected; the rail shows the file's pattern.
    assert_eq!(
        view.read_with(cx, |app, _| (
            app.selection,
            app.rail.as_ref().map(|rail| rail.patterns[0].text.clone())
        )),
        (Some(Selection::Handler(0)), Some("h0.example/*".to_owned()))
    );

    // An edit over a file that changed meanwhile is kept on screen, not
    // written, and the rail foot offers the two ways out.
    written_by_someone_else(&path, 3);
    view.update(cx, |app, cx| {
        app.store.set_profile(0, "Work");
        app.refresh(cx);
    });
    draw(cx);
    let (rows, _, label, dirty) = counts(cx);
    assert_eq!((rows, dirty), (2, true), "the edit, not the file");
    assert!(label.contains("changed on disk"), "{label}");
    assert!(cx.debug_bounds(crate::chrome::RELOAD_ID).is_some());
    assert!(cx.debug_bounds(crate::chrome::OVERWRITE_ID).is_some());
    // The debounced save runs and is refused.
    let on_disk = saved(cx, &path);
    assert_eq!(on_disk.rules.len(), 3);
    assert_eq!(on_disk.rules[0].profile, "");
    assert!(counts(cx).3, "still unsaved");

    // Overwrite writes the edit over the file.
    click(cx, crate::chrome::OVERWRITE_ID);
    let on_disk = saved(cx, &path);
    assert_eq!(on_disk.rules.len(), 2);
    assert_eq!(on_disk.rules[0].profile, "Work");
    let (_, _, label, dirty) = counts(cx);
    assert!(label.starts_with("saved "), "{label}");
    assert!(!dirty);
    assert!(cx.debug_bounds(crate::chrome::OVERWRITE_ID).is_none());

    // Reload drops the edit and reads the file; the selection past the
    // end is cleared.
    view.update(cx, |app, cx| {
        app.select(Selection::Handler(1), cx);
        app.store.set_profile(1, "Home");
        app.refresh(cx);
    });
    written_by_someone_else(&path, 1);
    view.update(cx, |app, cx| {
        app.store.set_browser(1, "Arc");
        app.refresh(cx);
    });
    draw(cx);
    assert!(cx.debug_bounds(crate::chrome::RELOAD_ID).is_some());
    click(cx, crate::chrome::RELOAD_ID);
    let (rows, stored, label, dirty) = counts(cx);
    assert_eq!((rows, stored, dirty), (1, 1, false));
    assert!(label.starts_with("reloaded "), "{label}");
    assert_eq!(
        view.read_with(cx, |app, _| (app.selection, app.rail.is_some())),
        (None, false)
    );
    assert_eq!(saved(cx, &path).rules[0].profile, "", "nothing written");

    // Opening the Add a rule sheet reads the file first: "Becomes handler
    // N" counts the agent's handlers.
    written_by_someone_else(&path, 4);
    click(cx, "nav-activity");
    click(cx, "activity-row-4");
    click(cx, "activity-add-rule-4");
    assert!(cx.debug_bounds("add-rule-sheet").is_some());
    assert_eq!(counts(cx).1, 4);
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn an_in_process_rules_apply_reloads_the_rules_screen(cx: &mut TestAppContext) {
    use pwrroute_mcp::{AuditRecord, AuditSink as _, Outcome, RulesStore as _, live::LiveRules};

    let dir = scratch("mcp-apply");
    let path = dir.join("rules.json");
    let (view, cx) = open_store(cx, None, RulesStore::load(Some(path.clone())));
    assert!(view.read_with(cx, |app, _| app.store.is_empty()));
    let (changed_tx, _changed_rx) = async_channel::unbounded::<()>();
    let (rules_tx, rules_rx) = async_channel::unbounded::<()>();
    cx.update(|_, cx| crate::mcp::follow_rules_writes(rules_rx, cx));
    let audit = crate::mcp::AppAudit::new(changed_tx, rules_tx);
    let record = |tool: &str, outcome: Outcome| AuditRecord {
        tool: tool.to_owned(),
        agent: "pwragnt".to_owned(),
        connection: 1,
        started: std::time::SystemTime::now(),
        duration: std::time::Duration::from_millis(3),
        outcome,
        note: None,
    };

    // The tool writes through the MCP crate's live store, as the host
    // serves it, then the audit hears of it.
    let live = LiveRules::new(path.clone(), || None);
    let revision = live.read().expect("read").revision;
    let rules = serde_json::json!({
        "defaultBrowser": "Safari",
        "rules": [
            { "match": "a.example/*", "browser": "Firefox" },
            { "match": "b.example/*", "browser": "Arc" }
        ]
    });
    live.write(&revision, rules, false).expect("write");
    audit.record(record("pwrroute_rules_apply", Outcome::Ok));
    cx.run_until_parked();
    draw(cx);
    let (rows, stored, label) = view.read_with(cx, |app, _| {
        (
            app.handlers.len(),
            app.store.handler_count(),
            app.store.status_label(),
        )
    });
    assert_eq!((rows, stored), (2, 2));
    assert!(label.starts_with("reloaded "), "{label}");
    assert_eq!(
        view.read_with(cx, |app, _| app.handlers[1].browser.clone()),
        "Arc"
    );
    // The strip's rule count is the rows'; Test a link resolves by the file.
    click(cx, "nav-test");
    click(cx, "test-url");
    cx.simulate_input("https://b.example/x");
    click(cx, "test-run");
    assert_eq!(trace(&view, cx).result.title, "Arc");

    // A refused or dry call changes nothing on disk, so nothing reloads.
    let before = view.read_with(cx, |app, _| app.store.status_label());
    audit.record(record("pwrroute_rules_apply", Outcome::Failed));
    audit.record(record("pwrroute_rules_list", Outcome::Ok));
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |app, _| app.store.status_label()),
        before
    );
    let _ = std::fs::remove_dir_all(dir);
}

// --- the Finicky importer ----------------------------------------------------

/// The importer's fixture config: the design's 2a, one rule of each verdict.
fn importer_config() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/importer/finicky.ts")
}

/// The open importer window, after whatever opened it has run.
fn importer_window(
    cx: &mut VisualTestContext,
) -> Option<gpui_kit::WindowHandle<crate::importer_view::Importer>> {
    cx.run_until_parked();
    cx.update(|_, cx| {
        cx.windows()
            .into_iter()
            .find_map(|window| window.downcast::<crate::importer_view::Importer>())
    })
}

#[gpui_kit::test]
fn the_banners_import_button_opens_the_importer(cx: &mut TestAppContext) {
    let config = fixture_home().join(".finicky.ts");
    let (_view, cx) = open_with(cx, Some(config));
    assert!(importer_window(cx).is_none());
    click(cx, crate::config_view::IMPORT_ID);
    let importer = importer_window(cx).expect("Import to visual rules… opens the importer");
    let model = cx
        .update(|_, cx| importer.read(cx).map(|importer| importer.model.clone()))
        .expect("the importer's model");
    assert_eq!(model.shown, "~/.finicky.ts");
    assert_eq!(model.step, crate::importer::Step::Review);
    // Design 1b's config: two function matchers stay JS, the patterns and
    // the default import.
    assert_eq!(model.counts(), (3, 2, 0));
    assert_eq!(
        model.review_footer(),
        "3 of 5 selected. Nothing is written yet."
    );
}

#[gpui_kit::test]
fn an_import_writes_rules_json_once_and_undo_takes_it_back(cx: &mut TestAppContext) {
    let dir = scratch("import");
    let path = dir.join("rules.json");
    let (view, cx) = open_store(cx, None, fixture_store(&dir));
    let before = RulesFile::load(&path).unwrap();

    // Outside any window's update, as the app's deferred open runs; the
    // file under review is the config in charge.
    cx.cx.update(|cx| {
        crate::importer_view::open_with(importer_config(), false, Some(importer_config()), cx);
    });
    let window = importer_window(cx).expect("the importer opened");
    let mut importer = VisualTestContext::from_window(window.into(), &cx.cx);
    draw(&mut importer);
    // Take the suggestion for the meet.google.com helper, then step
    // through: the file has handlers, so Place asks where they go.
    click(&mut importer, "importer-item-3");
    click(&mut importer, crate::importer_view::SUGGESTION_ID);
    click(&mut importer, crate::importer_view::NEXT_ID);
    assert_eq!(
        window
            .read_with(&importer, |importer, _| importer.model.step)
            .unwrap(),
        crate::importer::Step::Place
    );
    click(&mut importer, "importer-place-1");
    click(&mut importer, crate::importer_view::NEXT_ID);
    click(&mut importer, "importer-source-1");
    // Nothing is written before Import.
    assert_eq!(RulesFile::load(&path).unwrap(), before);
    // Import closes the window, so there is no frame to draw after it.
    let import = importer
        .debug_bounds(crate::importer_view::NEXT_ID)
        .expect("Import");
    importer.simulate_click(import.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(importer_window(cx).is_none(), "Import closes the importer");

    // One save: the imported handlers after the file's own (After yours),
    // the string rewrite after its rewrites, and the default.
    let after = RulesFile::load(&path).unwrap();
    assert_eq!(after.rules.len(), before.rules.len() + 4);
    assert_eq!(after.rules[..before.rules.len()], before.rules[..]);
    let imported: Vec<&str> = after.rules[before.rules.len()..]
        .iter()
        .map(|rule| rule.patterns[0].as_str())
        .collect();
    assert_eq!(
        imported,
        [
            "meet.google.com/*",
            "bsky.app/*",
            "google.com/*",
            "*.atlassian.net/*"
        ]
    );
    assert_eq!(after.rules[before.rules.len() + 3].profile, "Work");
    assert_eq!(after.rewrites.len(), before.rewrites.len() + 1);
    assert_eq!(after.default_browser, "Google Chrome");
    // The Rules screen shows the file as written, with the banner.
    let (handlers, done) =
        view.read_with(cx, |app, _| (app.handlers.len(), app.last_import.clone()));
    assert_eq!(handlers, after.rules.len());
    let done = done.expect("the import is remembered");
    assert_eq!((done.count, done.suggestions), (6, 1));
    draw(cx);
    assert!(cx.debug_bounds("import-done").is_some());

    click(cx, crate::importer_view::UNDO_ID);
    // The same rules as before the import (the store writes them again,
    // so a hand-written file comes back in its own format).
    assert_eq!(RulesFile::load(&path).unwrap(), before);
    assert!(view.read_with(cx, |app, _| app.last_import.is_none()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// File › Import Finicky Config… on a file that is not the config in
/// charge: Choose source names the one in charge, and Keep leaves
/// `settings.json`'s source as it was.
#[gpui_kit::test]
fn importing_a_file_not_in_charge_keeps_the_config_in_charge(cx: &mut TestAppContext) {
    let dir = scratch("import-other");
    let path = dir.join("rules.json");
    let in_charge = fixture_home().join(".finicky.ts");
    let mut settings = crate::settings_store::SettingsStore::load(Some(dir.clone()));
    settings
        .update(|settings| settings.source.config = Some(in_charge.clone()))
        .unwrap();
    let source = settings.settings().source.clone();
    cx.update(|cx| crate::settings_store::install(settings, None, cx));
    let (view, cx) = open_store(cx, Some(in_charge.clone()), fixture_store(&dir));
    let before = RulesFile::load(&path).unwrap();

    cx.cx.update(|cx| {
        crate::importer_view::open_with(importer_config(), false, Some(in_charge.clone()), cx);
    });
    let window = importer_window(cx).expect("the importer opened");
    let mut importer = VisualTestContext::from_window(window.into(), &cx.cx);
    draw(&mut importer);
    let model = window
        .read_with(&importer, |importer, _| importer.model.clone())
        .unwrap();
    assert_eq!(
        model.charge,
        crate::importer::Charge::Other {
            path: in_charge.clone(),
            shown: "~/.finicky.ts".to_owned()
        }
    );
    // Place, then Choose source: asked, with Keep selected.
    click(&mut importer, crate::importer_view::NEXT_ID);
    click(&mut importer, crate::importer_view::NEXT_ID);
    let model = window
        .read_with(&importer, |importer, _| importer.model.clone())
        .unwrap();
    assert_eq!(model.step, crate::importer::Step::Source);
    assert_eq!(model.source, crate::importer::SourceChoice::KeepConfig);
    let import = importer
        .debug_bounds(crate::importer_view::NEXT_ID)
        .expect("Import");
    importer.simulate_click(import.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(importer_window(cx).is_none(), "Import closes the importer");

    // The rules went in; the source did not move.
    let after = RulesFile::load(&path).unwrap();
    assert!(after.rules.len() > before.rules.len());
    let saved = crate::settings_store::Settings::load(&dir).unwrap();
    assert_eq!(saved.source, source);
    let outcome = view.read_with(cx, |app, _| {
        app.last_import.as_ref().map(|done| done.outcome.clone())
    });
    assert_eq!(
        outcome.as_deref(),
        Some("~/.finicky.ts stays in charge and runs first.")
    );

    // Another writer changes rules.json before Undo: Undo ends rather than
    // overwrite that change, and the source is not touched.
    let other = RulesFile {
        default_browser: "Firefox".into(),
        ..RulesFile::default()
    };
    other.save(&path).unwrap();
    let restored = view.update(cx, |app, cx| app.undo_import(cx));
    assert_eq!(restored, None);
    assert_eq!(RulesFile::load(&path).unwrap(), other);
    let undoable = view.read_with(cx, |app, _| {
        app.last_import.as_ref().map(|done| done.undoable)
    });
    assert_eq!(undoable, Some(false));
    let _ = std::fs::remove_dir_all(&dir);
}

/// An import the store would refuse (`rules.json` changed under unsaved
/// edits) writes nothing and opens the importer again with the reason.
#[gpui_kit::test]
fn a_refused_import_opens_the_importer_again_with_the_reason(cx: &mut TestAppContext) {
    let dir = scratch("import-refused");
    let path = dir.join("rules.json");
    let (view, cx) = open_store(cx, None, fixture_store(&dir));
    // An unsaved edit, then another writer.
    view.update(cx, |app, _| app.store.replace_all(RulesFile::default()));
    let other = RulesFile {
        default_browser: "Firefox".into(),
        ..RulesFile::default()
    };
    other.save(&path).unwrap();

    cx.cx.update(|cx| {
        crate::importer_view::open_with(importer_config(), false, Some(importer_config()), cx);
    });
    let window = importer_window(cx).expect("the importer opened");
    let mut importer = VisualTestContext::from_window(window.into(), &cx.cx);
    draw(&mut importer);
    // No visual handlers to place among: Review, then Choose source.
    click(&mut importer, crate::importer_view::NEXT_ID);
    let import = importer
        .debug_bounds(crate::importer_view::NEXT_ID)
        .expect("Import");
    importer.simulate_click(import.center(), Modifiers::default());
    cx.run_until_parked();

    let window = importer_window(cx).expect("the importer opened again");
    let model = cx
        .update(|_, cx| window.read(cx).map(|importer| importer.model.clone()))
        .unwrap();
    assert_eq!(
        model.refused.as_deref(),
        Some(crate::rules_store::CHANGED_ON_DISK)
    );
    assert_eq!(model.step, crate::importer::Step::Source);
    let mut importer = VisualTestContext::from_window(window.into(), &cx.cx);
    draw(&mut importer);
    assert!(importer.debug_bounds("importer-refused").is_some());
    assert_eq!(
        RulesFile::load(&path).unwrap(),
        other,
        "nothing was written"
    );
    assert!(view.read_with(cx, |app, _| app.last_import.is_none()));
    let _ = std::fs::remove_dir_all(&dir);
}
