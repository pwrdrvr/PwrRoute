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
    // Every window lands on Overview with nothing selected; these tests
    // start on Rules with the first handler (else rewrite) selected, as a
    // click there leaves it.
    view.update_in(cx, |app, window, cx| {
        app.navigate(Screen::Rules, window, cx);
        if let Some(first) = (!app.handlers.is_empty())
            .then_some(Selection::Handler(0))
            .or((!app.rewrites.is_empty()).then_some(Selection::Rewrite(0)))
        {
            app.select(first, cx);
        }
    });
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
    let mut last = None;
    for _ in 0..20 {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("no element {selector} on screen"));
        let height = cx.update(|window, _| window.viewport_size().height);
        let below = bounds.bottom() + px(24.0) - height;
        // In view, or as far as its pane scrolls with the whole of it
        // inside the window (the last row of a pane that ends there).
        if below <= px(0.0) || (last == Some(bounds) && bounds.bottom() <= height) {
            return;
        }
        last = Some(bounds);
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

    // The design's example: two rewrites apply, rule 3 wins on Slack.
    type_sample(cx);
    let first = trace(&view, cx);
    assert_eq!(
        titles(&first),
        [
            "Received",
            "Rewrite 1",
            "Rewrite 2",
            "Rewrite 3",
            "Rules 1 and 2",
            "Rule 3",
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
    assert!(titles(&trace(&view, cx)).contains(&"Rule 2"));

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
    assert!(titles(&meet).contains(&"Rule 1"));
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
    assert!(titles(&trace(&view, cx)).contains(&"Rule 2"));
    click(cx, "test-run");
    assert!(titles(&trace(&view, cx)).contains(&"Rules 1 and 2"));
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
    // A link no visual rule takes: rules.json routes beside the config.
    click(cx, "test-url");
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("https://unmatched.example.com/");
    cx.simulate_keystrokes("enter");
    draw(cx);
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

    // A profile by name, typed: "Other app or profile by name…" opens the
    // fields Open in had before its lanes.
    click(cx, "destination-typed");
    click(cx, "profile");
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("Personal");
    // "+ app" offers the app another rule names, and never a text field.
    click(cx, "opener-add");
    assert!(cx.debug_bounds("opener-other").is_some());
    click(cx, "opener-choice-0");
    draw(cx);
    let file = saved(cx, &path);
    assert_eq!(file.rules[0].profile, "Personal");
    assert_eq!(file.rules[0].opener, ["com.tinyspeck.slackmacgap"]);
    // The app shows as a chip, by the name it goes by.
    assert!(cx.debug_bounds("opener-0").is_some());
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
    assert_eq!(handler(cx).title, "New rule");
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
    click(cx, "destination-typed");
    cx.simulate_input("Firefox");
    draw(cx);
    assert_eq!(handler(cx).rule.badge, RowBadge::None);
    let result = view.read_with(cx, |app, _| app.rail.as_ref().unwrap().try_result.clone());
    assert_eq!(
        result,
        Some(crate::try_it::TryResult::Text {
            verdict: crate::try_it::Verdict::Takes,
            message: "This rule opens that link in Firefox.".into(),
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
    scroll_to(cx, "rail-delete");
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

/// Open in's lanes stay inside the sheet: a browser with many profiles
/// wraps its pills, a long name ends in an ellipsis, and one pill is the
/// whole choice (design: Destination and App Pickers 1a, 1b).
#[gpui_kit::test]
fn the_add_rule_lanes_stay_inside_the_sheet(cx: &mut TestAppContext) {
    let dir = scratch("add-rule-lanes");
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
    // Row 4 opened in Google Chrome, which leads.
    click(cx, "activity-row-4");
    click(cx, "activity-add-rule-4");
    let sheet = cx.debug_bounds("add-rule-sheet").expect("the sheet");
    for lane in 0..3 {
        let bounds = cx
            .debug_bounds(format!("add-rule-dest-lane-{lane}").leak())
            .expect("a lane");
        assert!(bounds.right() <= sheet.right(), "lane {lane}: {bounds:?}");
        let mut pill = 0;
        while let Some(p) = cx.debug_bounds(format!("add-rule-dest-{lane}-{pill}").leak()) {
            assert!(
                p.right() <= bounds.right(),
                "lane {lane} pill {pill}: {p:?}"
            );
            pill += 1;
        }
        assert!(pill > 0, "lane {lane} has pills");
    }
    let sheet_state = |cx: &mut VisualTestContext| {
        view.read_with(cx, |app, _| {
            let sheet = app.actions.sheet.as_ref().unwrap();
            (
                sheet.browser.clone(),
                sheet.profile.clone(),
                sheet.private,
                sheet.keep,
            )
        })
    };
    // Edge's Spanish: Default, then the five profiles.
    click(cx, "add-rule-dest-1-5");
    assert_eq!(
        sheet_state(cx),
        (
            "Microsoft Edge".into(),
            "Spanish".into(),
            false,
            Privacy::Full
        )
    );
    // Its InPrivate pill: private, the browser's own profile, host only.
    click(cx, "add-rule-dest-1-6");
    assert_eq!(
        sheet_state(cx),
        ("Microsoft Edge".into(), String::new(), true, Privacy::Host)
    );
    // Back to a profile: Keep follows.
    click(cx, "add-rule-dest-0-2");
    assert_eq!(
        sheet_state(cx),
        ("Google Chrome".into(), "Work".into(), false, Privacy::Full)
    );
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

    // Private window and Keep write `private` and `privacy`. With no scan
    // the rule's own browser is the one lane: Default, Work, Incognito.
    click(cx, "destination-0-2");
    assert!(handler(cx).private);
    assert_eq!(handler(cx).profile, "");
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
    // Default leaves the private window; with no scan to list them, Work
    // went with the pick.
    click(cx, "destination-0-0");
    click(cx, "keep-full");
    let file = saved(cx, &path);
    assert!(!file.rules[0].private);
    assert_eq!(file.rules[0].profile, "");
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

/// "+ Rule" (design: Activity Quick Add Rule): on the row under the
/// pointer, at rest on every row under Everything else, never on a
/// host-only row or the expanded one; its click opens the sheet and leaves
/// the row as it was. ⌘N opens it for the expanded row, or the row whose
/// "⋯" menu is open.
#[gpui_kit::test]
fn activity_adds_a_rule_from_a_row_in_one_click(cx: &mut TestAppContext) {
    let dir = scratch("quick-rule");
    // Row 0 kept host only.
    let mut records = fixtures::activity();
    records[0].privacy = Privacy::Host;
    let mut newest = records[1].clone();
    newest.id = DispatchId(99);
    let feed = cx.update(|cx| activity::install_feed(Feed::from_records(records), cx));
    let (view, cx) = open_store(cx, None, fixture_store(&dir));
    click(cx, "nav-activity");
    let sheet_key = |cx: &mut VisualTestContext| {
        view.read_with(cx, |app, _| app.actions.sheet.as_ref().map(|s| s.key))
    };
    let key = |cx: &mut VisualTestContext, index: usize| {
        view.read_with(cx, |app, _| app.activity.rows[index].key)
    };
    let expanded = |cx: &mut VisualTestContext| view.read_with(cx, |app, _| app.activity.expanded);

    // At rest under All: no row offers it.
    assert!(cx.debug_bounds("activity-quick-rule-4").is_none());
    // Over row 4 (handler 2's pull request) it takes the Took cell's place
    // without moving the row.
    let rest = cx.debug_bounds("activity-row-4").expect("row 4");
    hover(cx, "activity-row-4");
    assert_eq!(
        view.read_with(cx, |app, _| app.activity.hovered),
        Some(key(cx, 4))
    );
    assert!(cx.debug_bounds("activity-quick-rule-4").is_some());
    assert_eq!(cx.debug_bounds("activity-row-4"), Some(rest));
    assert!(cx.debug_bounds("activity-quick-rule-3").is_none());
    // A host-only row has no URL to write a rule for: nothing.
    hover(cx, "activity-row-0");
    assert_eq!(
        view.read_with(cx, |app, _| app.activity.rows[0].privacy),
        Privacy::Host
    );
    assert!(cx.debug_bounds("activity-quick-rule-0").is_none());
    assert!(cx.debug_bounds("activity-quick-rule-4").is_none());

    // Its click opens the sheet for that row and does not expand it.
    hover(cx, "activity-row-4");
    click(cx, "activity-quick-rule-4");
    assert_eq!(sheet_key(cx), Some(key(cx, 4)));
    assert_eq!(expanded(cx), None);
    click(cx, "add-rule-cancel");
    assert_eq!(sheet_key(cx), None);

    // Under Everything else every row shows it at rest.
    click(cx, "activity-filter-Unrouted");
    let unrouted: Vec<usize> = view.read_with(cx, |app, _| {
        app.activity
            .visible()
            .into_iter()
            .map(|(index, _)| index)
            .collect()
    });
    assert!(!unrouted.is_empty());
    for index in &unrouted {
        assert!(
            cx.debug_bounds(format!("activity-quick-rule-{index}").leak())
                .is_some(),
            "row {index}"
        );
    }
    // The expanded row has the primary Add a rule… instead.
    let first = unrouted[0];
    click(cx, format!("activity-row-{first}").leak());
    assert_eq!(expanded(cx), Some(key(cx, first)));
    assert!(
        cx.debug_bounds(format!("activity-quick-rule-{first}").leak())
            .is_none()
    );
    assert!(
        cx.debug_bounds(format!("activity-add-rule-{first}").leak())
            .is_some()
    );

    // ⌘N (Ctrl+N off macOS: "secondary"): the expanded row.
    cx.simulate_keystrokes("secondary-n");
    draw(cx);
    assert_eq!(sheet_key(cx), Some(key(cx, first)));
    click(cx, "add-rule-cancel");

    // ⌘N with a row's "⋯" menu open: that row, not the expanded one.
    click(cx, "activity-filter-All");
    click(cx, "activity-more-4");
    assert!(cx.debug_bounds("activity-menu-4").is_some());
    cx.simulate_keystrokes("secondary-n");
    draw(cx);
    assert_eq!(sheet_key(cx), Some(key(cx, 4)));
    click(cx, "add-rule-cancel");

    // Nothing expanded and no menu: ⌘N does nothing.
    click(cx, format!("activity-row-{first}").leak());
    assert_eq!(expanded(cx), None);
    cx.simulate_keystrokes("secondary-n");
    draw(cx);
    assert_eq!(sheet_key(cx), None);

    // Nor for an expanded row the filter hides.
    click(cx, "activity-row-4");
    assert_eq!(expanded(cx), Some(key(cx, 4)));
    click(cx, "activity-filter-Failed");
    assert!(cx.debug_bounds("activity-row-4").is_none());
    cx.simulate_keystrokes("secondary-n");
    draw(cx);
    assert_eq!(sheet_key(cx), None);
    click(cx, "activity-filter-All");
    click(cx, "activity-row-4");

    // A new link moves every row under a pointer that did not move:
    // nothing stays hovered until it moves again.
    hover(cx, "activity-row-5");
    assert!(cx.debug_bounds("activity-quick-rule-5").is_some());
    feed.update(cx, |feed, cx| {
        feed.push(newest);
        cx.notify();
    });
    draw(cx);
    assert_eq!(view.read_with(cx, |app, _| app.activity.hovered), None);
    assert!(cx.debug_bounds("activity-quick-rule-5").is_none());
    assert!(cx.debug_bounds("activity-quick-rule-6").is_none());
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
        caption.starts_with(
            "Would also catch these 6 recent links \u{00b7} 5 routed by a config handler now"
        ),
        "{caption}"
    );
    assert!(cx.debug_bounds("add-rule-collision-5").is_some());

    // Browser and profile in one pill: Work in Google Chrome, where the
    // link opened (Default, Personal, Work); then Keep.
    click(cx, "add-rule-dest-0-2");
    click(cx, "add-rule-keep-none");
    click(cx, "add-rule-position-top");
    let rule = view.read_with(cx, |app, _| {
        app.actions.sheet.as_ref().unwrap().rule().unwrap()
    });
    assert_eq!(rule.browser, "Google Chrome");
    assert_eq!(rule.profile, "Work");
    assert!(rule.opener.is_empty());
    assert!(!rule.private);
    assert_eq!(rule.privacy, Some(Privacy::None.into()));
    assert_eq!(
        rule.examples,
        ["https://github.com/pwrdrvr/PwrGit/pull/361"]
    );
    // Only from the app that sent it: the link is saved as a test with
    // that app, since a rule's examples run from no app.
    click(cx, "add-rule-only-from");
    // The rule now sees only PwrAgnt's links, and this was its only one.
    let (pattern, _, collisions, caption) = sheet(cx);
    assert_eq!(pattern, "*");
    assert!(collisions.is_empty(), "{collisions:?}");
    assert_eq!(
        caption,
        "No other recent link from PwrAgnt matches this pattern."
    );
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
    assert!(!file.rules[0].private);
    assert_eq!(file.rules.len(), 8);
    assert_eq!(file.tests.len(), 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn activity_adds_a_rule_for_any_link_from_the_app(cx: &mut TestAppContext) {
    let dir = scratch("add-rule-any-from");
    let path = dir.join("rules.json");
    let (view, cx) = open_activity(cx, &dir, None);
    // Row 0 is the Jira link Slack sent.
    click(cx, "activity-row-0");
    click(cx, "activity-add-rule-0");
    let sheet = |cx: &mut VisualTestContext| {
        view.read_with(cx, |app, _| {
            let sheet = app.actions.sheet.as_ref().expect("the sheet is open");
            (
                sheet.pattern.text.clone(),
                sheet.any_from,
                sheet.only_from,
                sheet
                    .collisions
                    .iter()
                    .map(|c| c.url.clone())
                    .collect::<Vec<_>>(),
                sheet.collision_caption(),
            )
        })
    };
    // A click on the dimmed window around the sheet keeps it open.
    let scrim = cx.debug_bounds("add-rule-scrim").expect("the scrim");
    cx.simulate_click(scrim.origin + point(px(8.0), px(8.0)), Modifiers::none());
    assert!(cx.debug_bounds("add-rule-sheet").is_some());
    // Offered, never chosen for the person.
    assert!(cx.debug_bounds("add-rule-any-from").is_some());
    let (pattern, any_from, only_from, _, _) = sheet(cx);
    assert_eq!(pattern, "example.atlassian.net/browse/*");
    assert!(!any_from && !only_from);

    // Any link from Slack: every site, Sent from on, and only Slack's
    // other links caught.
    click(cx, "add-rule-any-from");
    let (pattern, any_from, only_from, collisions, caption) = sheet(cx);
    assert_eq!(pattern, "*");
    assert!(any_from && only_from);
    assert_eq!(collisions, [fixtures::TEST_URL], "{caption}");
    assert!(
        caption.starts_with("Would also catch these 1 recent link from Slack"),
        "{caption}"
    );

    // Sent from off goes back to the narrowest pattern, never every link
    // from every app.
    click(cx, "add-rule-only-from");
    let (pattern, any_from, only_from, _, _) = sheet(cx);
    assert_eq!(pattern, "example.atlassian.net/browse/*");
    assert!(!any_from && !only_from);
    // A pattern chosen after it puts Sent from back where it was.
    click(cx, "add-rule-any-from");
    click(cx, "add-rule-candidate-1");
    let (pattern, any_from, only_from, _, _) = sheet(cx);
    assert_eq!(pattern, "example.atlassian.net/*");
    assert!(!any_from && !only_from);

    // An earlier rule takes this link today, so the new rule goes on top.
    click(cx, "add-rule-any-from");
    choose_sheet_browser(cx);
    click(cx, "add-rule-position-top");
    click(cx, "add-rule-save");
    assert!(cx.debug_bounds("add-rule-sheet").is_none());
    let file = saved(cx, &path);
    let added = &file.rules[0];
    assert_eq!(added.patterns, ["*"]);
    assert_eq!(added.opener, ["Slack"]);
    assert!(added.examples.is_empty());
    assert_eq!(file.tests.len(), 1);
    assert_eq!(file.tests[0].opener.as_deref(), Some("Slack"));
    // The saved test resolves to the new rule.
    draw(cx);
    assert_eq!(
        view.read_with(cx, |app, _| app.tests.passed(TestSource::Test { test: 0 })),
        Some(true)
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn the_add_rule_sheet_folds_a_long_link(cx: &mut TestAppContext) {
    let dir = scratch("add-rule-long-link");
    let records = fixtures::activity();
    let long = format!(
        "https://auth.example.com/o/oauth2/v2/auth?client_id=demo&state={}&prompt=consent",
        "x".repeat(200)
    );
    let mut feed = records.clone();
    feed.push(record_like(
        &records[2],
        99,
        &long,
        Matched::Default { owner: None },
    ));
    cx.update(|cx| activity::install_feed(Feed::from_records(feed), cx));
    let (view, cx) = open_store(cx, None, fixture_store(&dir));
    click(cx, "nav-activity");
    view.update_in(cx, |app, window, cx| {
        app.add_rule(
            RowKey {
                id: DispatchId(99),
                journaled: false,
            },
            window,
            cx,
        );
    });
    draw(cx);
    let folded = cx
        .debug_bounds("add-rule-link")
        .expect("the link")
        .size
        .height;
    // Two lines of 16.5px.
    assert!(folded <= px(34.0), "{folded:?}");
    click(cx, "add-rule-link-toggle");
    let shown = cx
        .debug_bounds("add-rule-link")
        .expect("the link")
        .size
        .height;
    assert!(shown > folded, "{shown:?} after {folded:?}");
    assert!(view.read_with(cx, |app, _| {
        app.actions.sheet.as_ref().unwrap().url_expanded
    }));
    let _ = std::fs::remove_dir_all(dir);
}

/// A sign-in link whose path is one segment and whose query is long: the
/// patterns leave the query out (design: Activity Privacy and Route Tests
/// 2c), an OAuth redirect_uri is no reason for JS, and a pattern too long
/// for its row ends in an ellipsis inside the sheet instead of pushing its
/// explanation out and stretching the row.
#[gpui_kit::test]
fn the_add_rule_sheet_keeps_long_patterns_in_their_rows(cx: &mut TestAppContext) {
    let dir = scratch("add-rule-long-pattern");
    let records = fixtures::activity();
    let sign_in = format!(
        "https://auth.example.com/authorize?response_type=code&client_id=demo\
         &redirect_uri=https%3A%2F%2Fapp.example%2Fcallback&state={}",
        "x".repeat(300)
    );
    let long_path = format!("https://docs.example.com/{}?page=2", "a".repeat(240));
    let mut feed = records.clone();
    feed.push(record_like(
        &records[2],
        98,
        &sign_in,
        Matched::Default { owner: None },
    ));
    feed.push(record_like(
        &records[2],
        99,
        &long_path,
        Matched::Default { owner: None },
    ));
    cx.update(|cx| activity::install_feed(Feed::from_records(feed), cx));
    let (view, cx) = open_store(cx, None, fixture_store(&dir));
    click(cx, "nav-activity");
    let open = |id: u64, view: &Entity<PwrRoute>, cx: &mut VisualTestContext| {
        view.update_in(cx, |app, window, cx| {
            app.add_rule(
                RowKey {
                    id: DispatchId(id),
                    journaled: false,
                },
                window,
                cx,
            );
        });
        draw(cx);
    };

    open(98, &view, cx);
    let (candidates, needs_js) = view.read_with(cx, |app, _| {
        let sheet = app.actions.sheet.as_ref().unwrap();
        (
            sheet
                .candidates
                .iter()
                .map(|(p, _)| p.clone())
                .collect::<Vec<_>>(),
            sheet.needs_js.clone(),
        )
    });
    assert_eq!(
        candidates,
        ["auth.example.com/authorize*", "auth.example.com/*"]
    );
    assert!(needs_js.is_empty(), "{needs_js:?}");
    click(cx, "add-rule-cancel");

    open(99, &view, cx);
    let pattern = view.read_with(cx, |app, _| {
        app.actions.sheet.as_ref().unwrap().candidates[0].0.clone()
    });
    assert!(pattern.len() > 200 && !pattern.contains('?'), "{pattern}");
    let sheet = cx.debug_bounds("add-rule-sheet").expect("the sheet");
    for row in ["add-rule-candidate-0", "add-rule-candidate-1"] {
        let bounds = cx.debug_bounds(row).expect(row);
        let value = cx
            .debug_bounds(format!("{row}-value").leak())
            .expect("the pattern");
        let why = cx
            .debug_bounds(format!("{row}-why").leak())
            .expect("the explanation");
        assert!(
            bounds.size.height <= px(80.0),
            "{row} is {:?} tall",
            bounds.size.height
        );
        assert!(
            bounds.right() <= sheet.right(),
            "{row} ends at {:?}, past the sheet at {:?}",
            bounds.right(),
            sheet.right()
        );
        // The pattern stops before its explanation, which keeps its room.
        assert!(
            value.right() <= why.left(),
            "{row}'s pattern runs to {:?}, over its explanation at {:?}",
            value.right(),
            why.left()
        );
        assert!(why.right() <= bounds.right(), "{row}'s explanation is cut");
        assert!(why.size.width >= px(100.0), "{:?}", why.size.width);
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Activity with the fixture records, a window over the fixture rules in
/// `dir`, and `config` in charge when given.
fn open_activity<'a>(
    cx: &'a mut TestAppContext,
    dir: &Path,
    config: Option<PathBuf>,
) -> (Entity<PwrRoute>, &'a mut VisualTestContext) {
    cx.update(|cx| activity::install_feed(Feed::from_records(fixtures::activity()), cx));
    let (view, cx) = open_store(cx, config, fixture_store(dir));
    click(cx, "nav-activity");
    (view, cx)
}

/// Picks the sheet's first browser: the sheet chooses none for the person.
fn choose_sheet_browser(cx: &mut VisualTestContext) {
    // The first lane's Default pill: the browser the link opened in.
    click(cx, "add-rule-dest-0-0");
}

/// What the open sheet says stops its rule from taking the link.
fn sheet_problem(view: &Entity<PwrRoute>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |app, _| {
        app.actions
            .sheet
            .as_ref()
            .expect("the sheet is open")
            .problem
            .as_ref()
            .map(|problem| problem.text.clone())
    })
}

fn action_note(view: &Entity<PwrRoute>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |app, _| app.actions.note.clone())
        .expect("a note")
        .1
}

#[gpui_kit::test]
fn a_rule_added_beside_a_config_shows_under_its_visual_rules(cx: &mut TestAppContext) {
    let dir = scratch("add-rule-config");
    let path = dir.join("rules.json");
    let (view, cx) = open_activity(cx, &dir, Some(fixture_home().join(".finicky.ts")));
    // Row 4, the GitHub pull request from PwrAgnt: no config handler takes
    // it, so a visual rule at the top does.
    click(cx, "activity-row-4");
    click(cx, "activity-add-rule-4");
    choose_sheet_browser(cx);
    click(cx, "add-rule-position-top");
    click(cx, "add-rule-save");
    assert!(cx.debug_bounds("add-rule-sheet").is_none());
    assert_eq!(saved(cx, &path).rules.len(), 8);
    let note = action_note(&view, cx);
    assert!(note.starts_with("Added rule 1 "), "{note}");
    assert!(note.contains("after ~/.finicky.ts"), "{note}");
    // The visual rules open, so the Rules screen shows the new rule rather
    // than the config's handlers alone.
    assert!(view.read_with(cx, |app, _| app.visual_rules_expanded));
    click(cx, "activity-show-rule-4");
    assert_eq!(view.read_with(cx, |app, _| app.screen), Screen::Rules);
    assert_eq!(
        view.read_with(cx, |app, _| app.selection),
        Some(Selection::Handler(0))
    );
    assert!(view.read_with(cx, |app, _| app.editing()));
    assert!(cx.debug_bounds("editor-rail").is_some());
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn a_rule_an_earlier_rule_shadows_is_flagged_before_it_is_added(cx: &mut TestAppContext) {
    let dir = scratch("add-rule-shadowed");
    let path = dir.join("rules.json");
    let (view, cx) = open_activity(cx, &dir, None);
    click(cx, "activity-row-4");
    click(cx, "activity-add-rule-4");
    choose_sheet_browser(cx);
    // At the top, a pattern that misses the link: handler 2 still takes
    // it, but not first, so the sheet says the rule does not match.
    click(cx, "add-rule-position-top");
    click(cx, "add-rule-pattern");
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("example.com/*");
    draw(cx);
    click(cx, "add-rule-save");
    let problem = sheet_problem(&view, cx).expect("a problem");
    assert!(
        problem.starts_with("This rule does not match the link"),
        "{problem}"
    );
    click(cx, "add-rule-candidate-0");
    click(cx, "add-rule-position-bottom");
    assert_eq!(sheet_problem(&view, cx), None);
    // At the bottom, handler 2's github.com/pwrdrvr/* takes the link first.
    click(cx, "add-rule-save");
    assert!(
        cx.debug_bounds("add-rule-sheet").is_some(),
        "the sheet stays open"
    );
    assert!(cx.debug_bounds("add-rule-problem").is_some());
    let problem = sheet_problem(&view, cx).expect("a problem");
    assert!(
        problem.starts_with("Rule 2 (github.com/pwrdrvr/*) takes this link first"),
        "{problem}"
    );
    assert_eq!(saved(cx, &path).rules.len(), 7, "nothing was added");
    // At the top it takes the link: added.
    click(cx, "add-rule-position-top");
    assert_eq!(sheet_problem(&view, cx), None);
    click(cx, "add-rule-save");
    assert!(cx.debug_bounds("add-rule-sheet").is_none());
    let file = saved(cx, &path);
    assert_eq!(file.rules.len(), 8);
    assert_eq!(file.rules[0].patterns, ["github.com/pwrdrvr/*"]);
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn a_rule_the_config_takes_first_says_so_and_adds_only_on_a_second_click(cx: &mut TestAppContext) {
    let dir = scratch("add-rule-config-first");
    let path = dir.join("rules.json");
    let (view, cx) = open_activity(cx, &dir, Some(fixture_home().join(".finicky.ts")));
    // Row 0 came from Slack: the config's second handler takes every link
    // from Slack, before any visual rule.
    click(cx, "activity-row-0");
    click(cx, "activity-add-rule-0");
    choose_sheet_browser(cx);
    click(cx, "add-rule-position-top");
    click(cx, "add-rule-save");
    assert!(cx.debug_bounds("add-rule-sheet").is_some());
    let problem = sheet_problem(&view, cx).expect("a problem");
    assert!(
        problem.starts_with("Config handler 2 of ~/.finicky.ts takes this link first"),
        "{problem}"
    );
    assert_eq!(saved(cx, &path).rules.len(), 7);
    // Add anyway: the rule goes in, and the note keeps the warning.
    click(cx, "add-rule-save");
    assert!(cx.debug_bounds("add-rule-sheet").is_none());
    assert_eq!(saved(cx, &path).rules.len(), 8);
    let note = action_note(&view, cx);
    assert!(
        note.contains("Config handler 2 of ~/.finicky.ts takes this link first"),
        "{note}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn the_same_rule_is_never_added_twice(cx: &mut TestAppContext) {
    let dir = scratch("add-rule-twice");
    let path = dir.join("rules.json");
    let (view, cx) = open_activity(cx, &dir, None);
    click(cx, "activity-row-4");
    click(cx, "activity-add-rule-4");
    choose_sheet_browser(cx);
    click(cx, "add-rule-position-top");
    click(cx, "add-rule-save");
    assert_eq!(saved(cx, &path).rules.len(), 8);
    // The same rule again, from the row's menu.
    click(cx, "activity-add-rule-menu-4");
    choose_sheet_browser(cx);
    click(cx, "add-rule-position-top");
    click(cx, "add-rule-save");
    assert!(cx.debug_bounds("add-rule-sheet").is_some());
    let problem = sheet_problem(&view, cx).expect("a problem");
    assert!(
        problem.starts_with("Rule 1 is already this rule"),
        "{problem}"
    );
    // A second click does not add it either.
    click(cx, "add-rule-save");
    assert!(cx.debug_bounds("add-rule-sheet").is_some());
    assert_eq!(saved(cx, &path).rules.len(), 8);
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
            Matched::Default { owner: None },
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
    choose_sheet_browser(cx);
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

    // The add left row 1's menu open, over row 2's "⋯": a click elsewhere
    // closes it.
    assert!(cx.debug_bounds("activity-show-rule-1").is_some());
    click(cx, "nav-activity");
    assert!(cx.debug_bounds("activity-menu-1").is_none());

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
            added: None,
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
        note.starts_with("Saved to rules.json under rule 1."),
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

// Engine precedence (WP-A): with the fixture config in charge, Test a link
// and Try it say who decides, in the order rules.json's `precedence` sets.

/// The fixture rules under `precedence`, written to a scratch directory.
fn precedence_store(precedence: pwrroute_engine::Precedence) -> RulesStore {
    let path = scratch("precedence").join("rules.json");
    RulesFile {
        precedence,
        ..crate::fixtures::rules_file()
    }
    .save(&path)
    .expect("write the rules");
    RulesStore::load(Some(path))
}

/// Test a link on `url` from no app; the trace's last step before
/// "Opens in", and the result.
fn test_unmatched(
    view: &Entity<PwrRoute>,
    cx: &mut VisualTestContext,
    url: &str,
) -> (String, String, String) {
    click(cx, "nav-test");
    click(cx, "test-url");
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input(url);
    cx.simulate_keystrokes("enter");
    draw(cx);
    let view = trace(view, cx);
    let last = &view.steps[view.steps.len() - 2];
    (last.title.clone(), last.detail.clone(), view.result.title)
}

#[gpui_kit::test]
fn test_a_link_names_the_configs_default_in_the_go_order(cx: &mut TestAppContext) {
    let config = fixture_home().join(".finicky.ts");
    let store = precedence_store(pwrroute_engine::Precedence::Config);
    let (view, cx) = open_store(cx, Some(config), store);
    assert_eq!(
        test_unmatched(&view, cx, "https://unmatched.example.com/"),
        (
            "Everything else \u{00b7} ~/.finicky.ts's default".into(),
            "no config handler or rule matched".into(),
            "Google Chrome".into()
        )
    );
}

#[gpui_kit::test]
fn test_a_link_names_pwrroutes_default_when_it_takes_everything_else(cx: &mut TestAppContext) {
    let config = fixture_home().join(".finicky.ts");
    let store = precedence_store(pwrroute_engine::Precedence::PwrRoute);
    let (view, cx) = open_store(cx, Some(config), store);
    assert_eq!(
        test_unmatched(&view, cx, "https://unmatched.example.com/"),
        (
            "Everything else \u{00b7} PwrRoute's default".into(),
            "no rule or config handler matched".into(),
            "Google Chrome \u{00b7} Personal".into()
        )
    );
}

/// Try it on the first handler, whose sample the config's handler 4 also
/// matches.
fn try_first_handler(
    view: &Entity<PwrRoute>,
    cx: &mut VisualTestContext,
) -> crate::try_it::TryResult {
    click(cx, "visual-rules-toggle");
    view.update(cx, |app, cx| {
        app.select(Selection::Handler(1), cx);
        app.select(Selection::Handler(0), cx);
    });
    draw(cx);
    view.read_with(cx, |app, _| {
        app.rail.as_ref().and_then(|r| r.try_result.clone())
    })
    .expect("Try it ran")
}

#[gpui_kit::test]
fn try_it_never_says_to_move_a_rule_above_a_config_handler(cx: &mut TestAppContext) {
    let config = fixture_home().join(".finicky.ts");
    let store = precedence_store(pwrroute_engine::Precedence::Config);
    let (view, cx) = open_store(cx, Some(config), store);
    assert_eq!(
        try_first_handler(&view, cx),
        crate::try_it::TryResult::Text {
            verdict: crate::try_it::Verdict::Beaten,
            message: "Config handler 4 wins first: Google Chrome. ~/.finicky.ts's handlers \
                      run before your rules; let PwrRoute's default take everything else to \
                      run yours first."
                .into(),
            bold: Some("Google Chrome".into()),
        }
    );
}

#[gpui_kit::test]
fn try_it_under_pwrroute_runs_the_rule_before_the_config(cx: &mut TestAppContext) {
    let config = fixture_home().join(".finicky.ts");
    let store = precedence_store(pwrroute_engine::Precedence::PwrRoute);
    let (view, cx) = open_store(cx, Some(config), store);
    assert_eq!(
        try_first_handler(&view, cx),
        crate::try_it::TryResult::Text {
            verdict: crate::try_it::Verdict::Takes,
            message: "This rule opens that link in Google Chrome \u{00b7} Work.".into(),
            bold: None,
        }
    );
}

#[gpui_kit::test]
fn add_rule_under_pwrroute_blames_the_pattern_not_the_configs_order(cx: &mut TestAppContext) {
    cx.update(|cx| activity::install_feed(Feed::from_records(fixtures::activity()), cx));
    let config = fixture_home().join(".finicky.ts");
    let store = precedence_store(pwrroute_engine::Precedence::PwrRoute);
    let (view, cx) = open_store(cx, Some(config), store);
    click(cx, "nav-activity");
    // Row 0 came from Slack, which the config's handler 2 takes. Under
    // "pwrroute" the rules run first, so a rule that misses the link is
    // what the sheet says, not the config's order.
    click(cx, "activity-row-0");
    click(cx, "activity-add-rule-0");
    choose_sheet_browser(cx);
    click(cx, "add-rule-position-top");
    click(cx, "add-rule-pattern");
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("example.com/*");
    draw(cx);
    click(cx, "add-rule-save");
    let problem = sheet_problem(&view, cx).expect("a problem");
    assert!(
        problem.starts_with("This rule does not match the link"),
        "{problem}"
    );
    // The suggested pattern takes the link ahead of the config: added.
    click(cx, "add-rule-candidate-0");
    assert_eq!(sheet_problem(&view, cx), None);
    click(cx, "add-rule-save");
    assert!(cx.debug_bounds("add-rule-sheet").is_none());
}

// --- Attribution (WP-B) -----------------------------------------------------

/// `base` as if `rules.json`'s rule `index` routed `url`, from no app, to
/// `browser` · `profile`.
fn routed_by_rule(
    base: &DispatchRecord,
    id: u64,
    url: &str,
    index: usize,
    browser: &str,
    profile: &str,
) -> DispatchRecord {
    let mut record = record_like(base, id, url, Matched::Rule { index });
    record.link.opener = None;
    let set = |b: &mut pwrroute_router::Browser| {
        b.name = browser.into();
        b.profile = profile.into();
    };
    if let Some(b) = &mut record.browser {
        set(b);
    }
    for step in &mut record.steps {
        if let Step::Browser { browser: b } = step {
            set(b);
        }
    }
    record
}

/// Three `rules.json` rules over example.com hosts, the first two opening
/// the same browser, so a record whose number now points at the other one
/// would pass a browser check.
fn attribution_rules() -> RulesFile {
    serde_json::from_str(
        r#"{
            "defaultBrowser": "Safari",
            "rules": [
                { "match": "docs.example.com/*", "browser": "Google Chrome", "profile": "Work" },
                { "match": "tickets.example.com/*", "browser": "Google Chrome", "profile": "Work" },
                { "match": "news.example.com/*", "browser": "Firefox" }
            ]
        }"#,
    )
    .expect("a valid rules.json")
}

/// A record that names its `rules.json` rule shows it as "rule N", and
/// Save as example puts the link under that rule straight from the record
/// while the file is as it was when the link was routed. A record older
/// than the file's last write is probed: the rule at its number now may be
/// another one (here one that opens the same browser but misses the link).
/// So is a record whose rule now opens somewhere else, and any record once
/// the file has been saved since.
#[gpui_kit::test]
fn activity_names_the_rule_and_saves_its_example_from_the_record(cx: &mut TestAppContext) {
    use std::time::{Duration, SystemTime};

    let dir = scratch("wp-b-attribution");
    let path = dir.join("rules.json");
    let store = store_for(&dir, &attribution_rules());
    // Routed after the file was written, but for the stale one.
    let now = SystemTime::now();
    let base = fixtures::activity()[0].clone();
    let at = |mut record: DispatchRecord, at: SystemTime| {
        record.received_at = at;
        record
    };
    let records = vec![
        at(
            routed_by_rule(
                &base,
                41,
                "https://tickets.example.com/wp-b",
                1,
                "Google Chrome",
                "Work",
            ),
            now,
        ),
        // Rule 2 opens Google Chrome · Work, not Firefox.
        at(
            routed_by_rule(
                &base,
                42,
                "https://other.example.com/moved",
                1,
                "Firefox",
                "",
            ),
            now,
        ),
        // Routed as rule 2 before the file last changed: a rule has since
        // gone in above, and rule 2 now is tickets.example.com's.
        at(
            routed_by_rule(
                &base,
                43,
                "https://docs.example.com/stale",
                1,
                "Google Chrome",
                "Work",
            ),
            now - Duration::from_secs(3600),
        ),
    ];
    cx.update(|cx| activity::install_feed(Feed::from_records(records), cx));
    let (view, cx) = open_store(cx, None, store);
    click(cx, "nav-activity");
    let labels: Vec<String> = view.read_with(cx, |app, _| {
        app.activity
            .rows
            .iter()
            .map(|row| row.rule.to_string())
            .collect()
    });
    assert_eq!(labels, ["rule 2", "rule 2", "rule 2"]);
    let menu_save = |cx: &mut VisualTestContext, row: usize| {
        click(cx, "nav-activity");
        click(
            cx,
            ["activity-more-0", "activity-more-1", "activity-more-2"][row],
        );
        view.read_with(cx, |app, _| app.actions.menu.clone().and_then(|m| m.save))
    };

    // The stale record: rule 2 opens the same browser but does not take
    // the link, so the probe finds rule 1, which does.
    assert_eq!(
        menu_save(cx, 2),
        Some(Save::Example {
            handler: 0,
            pattern: Some("docs.example.com/*".into()),
        })
    );
    // Rule 2 opens somewhere else and no rule takes the link: a test.
    assert_eq!(
        menu_save(cx, 1),
        Some(Save::Test {
            expects: "Firefox".into()
        })
    );
    // From the record, no probe: rule 2 as it was.
    assert_eq!(
        menu_save(cx, 0),
        Some(Save::Example {
            handler: 1,
            pattern: Some("tickets.example.com/*".into()),
        })
    );
    click(cx, "activity-save-0");
    assert_eq!(
        view.read_with(cx, |app, _| app.handlers[1].examples.clone()),
        ["https://tickets.example.com/wp-b"]
    );
    let note = view.read_with(cx, |app, _| app.actions.note.clone().unwrap().1);
    assert!(
        note.starts_with("Saved to rules.json under rule 2."),
        "{note}"
    );
    assert_eq!(
        saved(cx, &path).rules[1].examples,
        ["https://tickets.example.com/wp-b"]
    );

    // Saved since the link was routed: the record is no longer trusted,
    // and the probe agrees that rule 2 takes it.
    assert!(view.read_with(cx, |app, _| matches!(
        app.store.status(),
        crate::rules_store::Status::Saved { .. }
    )));
    assert_eq!(
        menu_save(cx, 0),
        Some(Save::Example {
            handler: 1,
            pattern: Some("tickets.example.com/*".into()),
        })
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Beside a config, the sheet knows whose rules run first: the config's
/// handlers, until `rules.json` says `"precedence": "pwrroute"`; the note
/// after Add says which.
#[gpui_kit::test]
fn the_add_rule_sheet_follows_the_precedence(cx: &mut TestAppContext) {
    let dir = scratch("wp-b-precedence");
    let (view, cx) = open_activity(cx, &dir, Some(fixture_home().join(".finicky.ts")));
    let sheet = |cx: &mut VisualTestContext| {
        view.read_with(cx, |app, _| {
            let sheet = app.actions.sheet.as_ref().expect("the sheet is open");
            (sheet.rules_first, sheet.config.clone())
        })
    };
    click(cx, "activity-row-4");
    click(cx, "activity-add-rule-4");
    choose_sheet_browser(cx);
    assert_eq!(sheet(cx), (false, Some("~/.finicky.ts".to_owned())));
    click(cx, "add-rule-cancel");

    view.update(cx, |app, _| {
        app.store
            .set_precedence(pwrroute_engine::Precedence::PwrRoute);
    });
    click(cx, "activity-add-rule-4");
    choose_sheet_browser(cx);
    assert_eq!(sheet(cx), (true, Some("~/.finicky.ts".to_owned())));
    click(cx, "add-rule-position-top");
    click(cx, "add-rule-save");
    let note = action_note(&view, cx);
    assert!(note.starts_with("Added rule 1 "), "{note}");
    assert!(
        note.contains("your rules run before ~/.finicky.ts's handlers"),
        "{note}"
    );
    let _ = std::fs::remove_dir_all(dir);
}

// Overview (WP-D) ----------------------------------------------------------

/// A window over `rules` (and the config, when given) as it opens: on
/// Overview, with the Overview's browsers, `records` as today's links, the
/// routing state set by hand and motion off.
fn open_overview(
    cx: &mut TestAppContext,
    config: Option<PathBuf>,
    rules: RulesFile,
    records: Vec<DispatchRecord>,
) -> (Entity<PwrRoute>, &mut VisualTestContext, PathBuf) {
    let dir = scratch("overview");
    let path = dir.join("rules.json");
    rules.save(&path).expect("write the rules");
    cx.update(|cx| {
        crate::assets::load_fonts(cx);
        cx.set_reduce_motion(true);
        activity::install_feed(Feed::from_records(records.clone()), cx);
    });
    let store = RulesStore::load(Some(path.clone()));
    let (view, cx) =
        cx.add_window_view(|_, cx| PwrRoute::with_store(config, Some(fixture_home()), store, cx));
    let focus = view.read_with(cx, |app, _| app.focus.clone());
    cx.update(|window, cx| window.focus(&focus, cx));
    view.update(cx, |app, _| {
        app.installed = fixtures::overview_browsers();
        app.activity.load(records);
        app.overview.status = Some(crate::overview_model::Status::routing());
    });
    draw(cx);
    (view, cx, path)
}

#[gpui_kit::test]
fn every_window_lands_on_overview_and_rules_opens_as_a_list(cx: &mut TestAppContext) {
    let (view, cx, _) = open_overview(cx, None, fixtures::rules_file(), Vec::new());
    assert_eq!(view.read_with(cx, |app, _| app.screen), Screen::Overview);
    assert!(cx.debug_bounds(crate::overview::PANE_ID).is_some());
    assert!(cx.debug_bounds("editor-rail").is_none());

    click(cx, "nav-rules");
    let selection = |cx: &mut VisualTestContext| view.read_with(cx, |app, _| app.selection);
    assert_eq!(selection(cx), None, "Rules opens with nothing selected");
    assert!(cx.debug_bounds("editor-rail").is_none());
    cx.simulate_keystrokes("down");
    draw(cx);
    assert_eq!(selection(cx), Some(Selection::Rewrite(0)));
    assert!(cx.debug_bounds("editor-rail").is_some());

    click(cx, "nav-overview");
    assert_eq!(view.read_with(cx, |app, _| app.screen), Screen::Overview);
}

/// The routes the map painted last, and how many of them were lit.
fn painted(view: &Entity<PwrRoute>, cx: &mut VisualTestContext) -> Vec<crate::overview::Painted> {
    view.read_with(cx, |app, _| app.overview.bounds.borrow().painted.clone())
}

/// Where a focus the Overview keeps (by key) is on the map drawn now.
fn placed(
    view: &Entity<PwrRoute>,
    cx: &mut VisualTestContext,
    which: fn(&crate::overview::OverviewState) -> Option<crate::overview_model::FocusKey>,
) -> Option<crate::overview_model::Focus> {
    view.update(cx, |app, cx| {
        let key = which(&app.overview)?;
        let status = crate::overview::status(app, cx);
        crate::overview::build(app, &status, cx).map.resolve(&key)
    })
}

fn lit_count(view: &Entity<PwrRoute>, cx: &mut VisualTestContext) -> usize {
    painted(view, cx).iter().filter(|route| route.lit).count()
}

fn hover(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("no element {selector} on screen"));
    cx.simulate_mouse_move(bounds.center(), None, Modifiers::default());
    draw(cx);
    draw(cx);
}

#[gpui_kit::test]
fn every_route_is_drawn_and_a_hovered_row_lights_its_own(cx: &mut TestAppContext) {
    let (view, cx, _) = open_overview(
        cx,
        None,
        fixtures::overview_rules(),
        fixtures::overview_rules_day(),
    );
    // At rest: five enabled rules and Everything else, none lit.
    assert_eq!(painted(&view, cx).len(), 6);
    assert_eq!(lit_count(&view, cx), 0);
    assert!(
        painted(&view, cx).iter().all(|route| !route.dashed),
        "all running"
    );

    scroll_to(cx, "overview-row-1");
    hover(cx, "overview-row-1");
    assert_eq!(
        placed(&view, cx, |state| state.hovered.clone()),
        Some(crate::overview_model::Focus::Row(1))
    );
    let row = cx.debug_bounds("overview-row-1").expect("rule 2's row");
    let node = cx
        .debug_bounds("overview-node-0-0")
        .expect("Chrome's Work node");
    let lit: Vec<_> = painted(&view, cx)
        .into_iter()
        .filter(|route| route.lit)
        .collect();
    assert_eq!(lit.len(), 1, "one route: {lit:?}");
    let route = lit[0];
    assert!(
        (route.from.x - row.right()).abs() < px(12.0),
        "{:?} starts at the row",
        route.from
    );
    assert!(
        (route.to.x - node.left()).abs() < px(12.0),
        "{:?} ends at the node",
        route.to
    );
    assert!((route.to.y - node.center().y).abs() < px(15.0));
    // The hovered row offers Edit ›.
    assert!(cx.debug_bounds("overview-row-edit-1").is_some());
    assert!(cx.debug_bounds("overview-row-edit-0").is_none());
}

/// More than six rows into one profile: every route lights (the old map
/// drew none at all past six).
#[gpui_kit::test]
fn hovering_a_node_with_seven_rows_lights_every_route(cx: &mut TestAppContext) {
    let (view, cx, _) = open_overview(
        cx,
        None,
        fixtures::overview_fanin_rules(),
        fixtures::overview_fanin_day(),
    );
    scroll_to(cx, "overview-node-0-0");
    hover(cx, "overview-node-0-0");
    assert_eq!(
        placed(&view, cx, |state| state.hovered.clone()),
        Some(crate::overview_model::Focus::Node(0, 0))
    );
    // Seven rules and Everything else.
    assert_eq!(lit_count(&view, cx), 8);
    let node = cx.debug_bounds("overview-node-0-0").expect("the node");
    let ends: Vec<_> = painted(&view, cx)
        .into_iter()
        .filter(|route| route.lit)
        .map(|route| route.to.y)
        .collect();
    // They fan into the port top to bottom, within its 25px.
    assert!(ends.windows(2).all(|pair| pair[0] < pair[1]), "{ends:?}");
    assert!(ends.iter().all(|y| (*y - node.center().y).abs() <= px(8.5)));
}

#[gpui_kit::test]
fn a_pin_survives_the_pointer_leaving_and_escape_clears_it(cx: &mut TestAppContext) {
    use crate::overview_model::Focus;
    let (view, cx, _) = open_overview(
        cx,
        None,
        fixtures::overview_fanin_rules(),
        fixtures::overview_fanin_day(),
    );
    scroll_to(cx, "overview-node-0-0");
    click(cx, "overview-node-0-0");
    let pinned = |cx: &mut VisualTestContext| placed(&view, cx, |state| state.pinned.clone());
    assert_eq!(pinned(cx), Some(Focus::Node(0, 0)));
    assert!(
        cx.debug_bounds("overview-pinned").is_some(),
        "the legend says so"
    );

    // The pointer leaves for a row elsewhere: the pin still traces.
    hover(cx, "overview-row-9");
    assert_eq!(pinned(cx), Some(Focus::Node(0, 0)));
    draw(cx);
    assert_eq!(lit_count(&view, cx), 8);

    cx.simulate_keystrokes("escape");
    draw(cx);
    assert_eq!(pinned(cx), None);
    assert!(cx.debug_bounds("overview-pinned").is_none());

    // Clear lets go too, and so does a second click on the pin.
    click(cx, "overview-node-0-0");
    assert_eq!(pinned(cx), Some(Focus::Node(0, 0)));
    click(cx, crate::overview::PIN_CLEAR_ID);
    assert_eq!(pinned(cx), None);
    click(cx, "overview-row-8");
    assert_eq!(pinned(cx), Some(Focus::Row(8)));
    click(cx, "overview-row-8");
    assert_eq!(pinned(cx), None);
}

#[gpui_kit::test]
fn a_card_header_click_pins_it_until_escape(cx: &mut TestAppContext) {
    let (view, cx, _) = open_overview(
        cx,
        None,
        fixtures::overview_rules(),
        fixtures::overview_rules_day(),
    );
    scroll_to(cx, "overview-card-1");
    click(cx, "overview-card-1");
    let pinned = |cx: &mut VisualTestContext| placed(&view, cx, |state| state.pinned.clone());
    assert_eq!(pinned(cx), Some(crate::overview_model::Focus::Card(1)));
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert_eq!(pinned(cx), None);
}

/// Folded profiles open in place and stay open for the session.
#[gpui_kit::test]
fn more_profiles_open_in_place(cx: &mut TestAppContext) {
    let (view, cx, _) = open_overview(
        cx,
        None,
        fixtures::overview_rules(),
        fixtures::overview_rules_day(),
    );
    // Microsoft Edge, no rules yet, shows two of its four profiles.
    scroll_to(cx, "overview-more-3");
    assert!(cx.debug_bounds("overview-node-3-2").is_none());
    click(cx, "overview-more-3");
    assert!(cx.debug_bounds("overview-node-3-3").is_some());
    click(cx, "nav-rules");
    click(cx, "nav-overview");
    assert!(cx.debug_bounds("overview-node-3-3").is_some(), "still open");
    assert!(view.read_with(cx, |app, _| {
        app.overview.expanded.contains("microsoft edge")
    }));
}

/// A node with no rules yet offers "+ Rule for …": Rules' new rule, with
/// that browser and profile chosen.
#[gpui_kit::test]
fn an_idle_node_adds_a_rule_for_its_profile(cx: &mut TestAppContext) {
    let (view, cx, _) = open_overview(
        cx,
        None,
        fixtures::overview_rules(),
        fixtures::overview_rules_day(),
    );
    // Chrome's Design: no rule names it.
    scroll_to(cx, "overview-node-0-2");
    hover(cx, "overview-node-0-2");
    assert_eq!(
        placed(&view, cx, |state| state.idle_hover.clone()),
        Some(crate::overview_model::Focus::Node(0, 2))
    );
    click(cx, "overview-node-0-2");
    let (screen, selection) = view.read_with(cx, |app, _| (app.screen, app.selection));
    assert_eq!(screen, Screen::Rules);
    assert_eq!(selection, Some(Selection::Handler(6)));
    let rule = view.read_with(cx, |app, _| app.store.edited().rules[6].clone());
    assert_eq!(
        (rule.browser.as_str(), rule.profile.as_str()),
        ("Google Chrome", "Design")
    );
}

/// Send Links 2c and 2a: a node's Next link here sends the next link to
/// that profile, the node and the hero say so, and Back to rules ends it.
#[gpui_kit::test]
fn next_link_here_sends_the_next_link_and_back_to_rules_ends_it(cx: &mut TestAppContext) {
    use crate::tray::model::{Routing, TrayModel};
    use pwrroute_router::{Destination as SendTarget, DryRunLauncher, Options, Router, SendScope};

    let router = Router::spawn(Options::default(), None, DryRunLauncher::new()).unwrap();
    let handle = router.handle();
    let model = cx.update(|cx| {
        crate::tray::install_for_test(
            TrayModel::fixture(std::time::SystemTime::now()),
            Some(handle.clone()),
            cx,
        )
    });
    let (view, cx, _) = open_overview(
        cx,
        None,
        fixtures::overview_rules(),
        fixtures::overview_rules_day(),
    );
    // Read the routing state from the tray, as the app does.
    view.update(cx, |app, _| {
        app.overview.status = None;
    });
    draw(cx);
    assert!(
        cx.debug_bounds("overview-node-send-0-2").is_none(),
        "only on hover"
    );

    // Chrome's Design, a node with no rules yet.
    scroll_to(cx, "overview-node-0-2");
    hover(cx, "overview-node-0-2");
    click(cx, "overview-node-send-0-2");
    let send = handle.send_to().expect("the next link is sent");
    assert_eq!(
        send.destination,
        SendTarget {
            browser: "Google Chrome".into(),
            profile: "Design".into(),
            private: false,
        }
    );
    assert_eq!(send.scope, SendScope::Next { left: 1, of: 1 });
    // It sent the link; it did not start a rule.
    assert_eq!(view.read_with(cx, |app, _| app.screen), Screen::Overview);
    draw(cx);
    assert!(
        cx.debug_bounds("overview-node-sent-0-2").is_some(),
        "the pill"
    );
    let state = cx.update(|_, cx| {
        let app = view.read(cx);
        crate::overview::status(app, cx).state()
    });
    assert_eq!(state, crate::overview_model::State::Sending);

    click(cx, crate::overview::HERO_ACTION_ID);
    assert_eq!(handle.send_to(), None);
    assert_eq!(
        model.read_with(cx, |model, _| model.routing),
        Routing::Active
    );
    draw(cx);
    assert!(cx.debug_bounds("overview-node-sent-0-2").is_none());
}

/// Rule 5 opens a private Firefox window: hovering Firefox's private
/// node lights its route, which ends there and not at the profile.
#[gpui_kit::test]
fn hovering_a_private_window_lights_its_routes(cx: &mut TestAppContext) {
    use crate::overview_model::{Focus, FocusKey};
    let (view, cx, _) = open_overview(
        cx,
        None,
        fixtures::overview_rules(),
        fixtures::overview_rules_day(),
    );
    // At rest every enabled rule still has its route.
    assert_eq!(painted(&view, cx).len(), 6);
    scroll_to(cx, "overview-node-2-1");
    hover(cx, "overview-node-2-1");
    assert_eq!(
        view.read_with(cx, |app, _| app.overview.hovered.clone()),
        Some(FocusKey::Private("firefox".into()))
    );
    assert_eq!(
        placed(&view, cx, |state| state.hovered.clone()),
        Some(Focus::Node(2, 1))
    );
    let node = cx
        .debug_bounds("overview-node-2-1")
        .expect("Firefox's private window");
    let lit: Vec<_> = painted(&view, cx)
        .into_iter()
        .filter(|route| route.lit)
        .collect();
    assert_eq!(lit.len(), 1, "rule 5 alone: {lit:?}");
    assert!((lit[0].to.x - node.left()).abs() < px(12.0));
    assert!((lit[0].to.y - node.center().y).abs() < px(15.0));
    // The private node sits under the profile, apart from it.
    let profile = cx
        .debug_bounds("overview-node-2-0")
        .expect("Firefox's default profile");
    assert!(node.top() >= profile.bottom() + px(4.0));
}

/// An idle private-window node offers "+ Rule for …": Rules' new rule
/// with that browser, no profile, and a private window on.
#[gpui_kit::test]
fn an_idle_private_window_adds_a_private_rule(cx: &mut TestAppContext) {
    let (view, cx, _) = open_overview(
        cx,
        None,
        fixtures::overview_rules(),
        fixtures::overview_rules_day(),
    );
    // Chrome's Incognito, after its three profiles.
    scroll_to(cx, "overview-node-0-3");
    hover(cx, "overview-node-0-3");
    assert_eq!(
        placed(&view, cx, |state| state.idle_hover.clone()),
        Some(crate::overview_model::Focus::Node(0, 3))
    );
    assert_eq!(lit_count(&view, cx), 0, "an idle node traces nothing");
    click(cx, "overview-node-0-3");
    let (screen, selection) = view.read_with(cx, |app, _| (app.screen, app.selection));
    assert_eq!(screen, Screen::Rules);
    assert_eq!(selection, Some(Selection::Handler(6)));
    let rule = view.read_with(cx, |app, _| app.store.edited().rules[6].clone());
    assert_eq!(
        (rule.browser.as_str(), rule.profile.as_str(), rule.private),
        ("Google Chrome", "", true)
    );
}

/// The keyboard reaches a card's private window after its profiles, and
/// Return on an idle one adds a private rule as its click does.
#[gpui_kit::test]
fn return_on_an_idle_private_window_adds_a_private_rule(cx: &mut TestAppContext) {
    use crate::overview_model::FocusKey;
    let (view, cx, _) = open_overview(
        cx,
        None,
        fixtures::overview_rules(),
        fixtures::overview_rules_day(),
    );
    view.update(cx, |app, _| {
        app.overview
            .key(Some(FocusKey::Private("microsoft edge".into())));
    });
    draw(cx);
    cx.simulate_keystrokes("enter");
    draw(cx);
    let rule = view.read_with(cx, |app, _| app.store.edited().rules[6].clone());
    assert_eq!(
        (rule.browser.as_str(), rule.profile.as_str(), rule.private),
        ("Microsoft Edge", "", true)
    );
}

#[gpui_kit::test]
fn the_keyboard_traces_and_return_opens_the_rule(cx: &mut TestAppContext) {
    use crate::overview_model::Focus;
    let (view, cx, _) = open_overview(
        cx,
        None,
        fixtures::overview_rules(),
        fixtures::overview_rules_day(),
    );
    cx.simulate_keystrokes("down down");
    draw(cx);
    assert_eq!(
        placed(&view, cx, |state| state.keyed.clone()),
        Some(Focus::Row(1))
    );
    assert_eq!(
        lit_count(&view, cx),
        1,
        "the keyboard traces like the pointer"
    );
    cx.simulate_keystrokes("enter");
    draw(cx);
    let (screen, selection) = view.read_with(cx, |app, _| (app.screen, app.selection));
    assert_eq!(
        (screen, selection),
        (Screen::Rules, Some(Selection::Handler(1)))
    );
}

/// Return on a node with no rules yet does what its click does: adds a
/// rule for that profile, rather than pinning a node that traces nothing.
#[gpui_kit::test]
fn return_on_an_idle_node_adds_a_rule(cx: &mut TestAppContext) {
    use crate::overview_model::FocusKey;
    let (view, cx, _) = open_overview(
        cx,
        None,
        fixtures::overview_rules(),
        fixtures::overview_rules_day(),
    );
    view.update(cx, |app, cx| {
        app.overview.key(Some(FocusKey::Node(
            "google chrome".into(),
            Some("Design".into()),
        )));
        cx.notify();
    });
    draw(cx);
    assert_eq!(lit_count(&view, cx), 0, "it traces nothing");
    cx.simulate_keystrokes("enter");
    draw(cx);
    let (screen, pinned) = view.read_with(cx, |app, _| (app.screen, app.overview.pinned.clone()));
    assert_eq!((screen, pinned), (Screen::Rules, None));
    let rule = view.read_with(cx, |app, _| app.store.edited().rules[6].clone());
    assert_eq!(
        (rule.browser.as_str(), rule.profile.as_str()),
        ("Google Chrome", "Design")
    );
}

/// A scroll moves the map under a pointer that did not move: the hover
/// lets go rather than tracing a row that is no longer under it (GPUI may
/// then report the row that now is).
#[gpui_kit::test]
fn a_scroll_lets_go_of_the_hover(cx: &mut TestAppContext) {
    use crate::overview_model::{FocusKey, RowSource};
    let (view, cx, _) = open_overview(
        cx,
        None,
        fixtures::overview_rules(),
        fixtures::overview_rules_day(),
    );
    scroll_to(cx, "overview-row-1");
    hover(cx, "overview-row-1");
    assert!(view.read_with(cx, |app, _| app.overview.hovered.is_some()));
    let row = cx.debug_bounds("overview-row-1").expect("rule 2's row");
    let event = ScrollWheelEvent {
        position: row.center(),
        delta: ScrollDelta::Pixels(point(px(0.0), px(-40.0))),
        ..Default::default()
    };
    cx.update(|window, cx| window.dispatch_event(event.to_platform_input(), cx));
    draw(cx);
    let moved = cx.debug_bounds("overview-row-1").expect("rule 2's row");
    assert!(moved.top() < row.top(), "the pane scrolled");
    let hovered = view.read_with(cx, |app, _| app.overview.hovered.clone());
    assert_ne!(hovered, Some(FocusKey::Row(RowSource::Rule(1))));
    assert!(
        painted(&view, cx)
            .iter()
            .filter(|route| route.lit)
            .all(|route| (route.from.y - moved.center().y).abs() > px(1.0)),
        "rule 2's route is not traced"
    );
}

#[gpui_kit::test]
fn a_tile_opens_activity_filtered_and_edit_opens_a_rule(cx: &mut TestAppContext) {
    let (view, cx, _) = open_overview(
        cx,
        None,
        fixtures::overview_rules(),
        fixtures::overview_rules_day(),
    );
    click(cx, "overview-tile-1");
    let (screen, filter) = view.read_with(cx, |app, _| (app.screen, app.activity.filter));
    assert_eq!((screen, filter), (Screen::Activity, Filter::Unrouted));

    click(cx, "nav-overview");
    scroll_to(cx, "overview-row-3");
    // A click pins the row; it no longer opens Rules.
    click(cx, "overview-row-3");
    assert_eq!(view.read_with(cx, |app, _| app.screen), Screen::Overview);
    assert_eq!(
        placed(&view, cx, |state| state.pinned.clone()),
        Some(crate::overview_model::Focus::Row(3))
    );
    click(cx, "overview-row-edit-3");
    let (screen, selection) = view.read_with(cx, |app, _| (app.screen, app.selection));
    assert_eq!(screen, Screen::Rules);
    assert_eq!(selection, Some(Selection::Handler(3)));
    assert!(cx.debug_bounds("editor-rail").is_some());
}

#[gpui_kit::test]
fn the_callout_lets_pwrroutes_default_take_everything_else_and_undo(cx: &mut TestAppContext) {
    use pwrroute_engine::Precedence;
    let (view, cx, path) = open_overview(
        cx,
        Some(fixtures::overview_config()),
        fixtures::overview_config_rules(),
        fixtures::overview_config_day(),
    );
    assert!(view.read_with(cx, |app, _| app.described.is_some()));
    scroll_to(cx, crate::overview::CALLOUT_BUTTON_ID);
    click(cx, crate::overview::CALLOUT_BUTTON_ID);
    assert_eq!(saved(cx, &path).precedence, Precedence::PwrRoute);
    draw(cx);

    scroll_to(cx, crate::overview::CALLOUT_UNDO_ID);
    click(cx, crate::overview::CALLOUT_UNDO_ID);
    assert_eq!(saved(cx, &path).precedence, Precedence::Config);
    draw(cx);
    assert!(
        cx.debug_bounds(crate::overview::CALLOUT_BUTTON_ID)
            .is_some()
    );
}

#[gpui_kit::test]
fn the_setup_card_chooses_where_everything_else_goes(cx: &mut TestAppContext) {
    let rules = RulesFile {
        default_browser: String::new(),
        default_profile: String::new(),
        ..fixtures::overview_rules()
    };
    let (_view, cx, path) = open_overview(cx, None, rules, Vec::new());
    assert!(cx.debug_bounds("overview-setup").is_some());
    click(cx, "overview-setup-choose");
    // The list is as wide as its field.
    let field = cx.debug_bounds("overview-setup-choose").expect("the field");
    let menu = cx.debug_bounds("overview-setup-menu").expect("the list");
    assert_eq!(
        (menu.left(), menu.size.width),
        (field.left(), field.size.width)
    );
    // Chrome has profiles: its row opens them in the list's place.
    click(cx, "overview-setup-choice-0");
    assert!(cx.debug_bounds("overview-setup-choice-0").is_none());
    assert!(cx.debug_bounds("overview-setup-profile-2").is_some());
    assert_eq!(saved(cx, &path).default_browser, "", "nothing chosen yet");
    click(cx, "overview-setup-profile-1");
    let file = saved(cx, &path);
    assert_eq!(
        (file.default_browser.as_str(), file.default_profile.as_str()),
        ("Google Chrome", "Personal")
    );
    draw(cx);
    assert!(
        cx.debug_bounds("overview-setup").is_none(),
        "nothing left to set up"
    );
}

#[gpui_kit::test]
fn the_setup_card_saves_a_browser_without_profiles_at_once(cx: &mut TestAppContext) {
    let rules = RulesFile {
        default_browser: String::new(),
        default_profile: String::new(),
        ..fixtures::overview_rules()
    };
    let (_view, cx, path) = open_overview(cx, None, rules, Vec::new());
    click(cx, "overview-setup-choose");
    // Firefox, second in the scan, has no profiles.
    click(cx, "overview-setup-choice-1");
    let file = saved(cx, &path);
    assert_eq!(
        (file.default_browser.as_str(), file.default_profile.as_str()),
        ("Firefox", "")
    );
}

#[gpui_kit::test]
fn leaving_overview_forgets_its_fades_and_the_hand_off(cx: &mut TestAppContext) {
    let (view, cx, _) = open_overview(
        cx,
        Some(fixtures::overview_config()),
        fixtures::overview_config_rules(),
        fixtures::overview_config_day(),
    );
    scroll_to(cx, crate::overview::CALLOUT_BUTTON_ID);
    click(cx, crate::overview::CALLOUT_BUTTON_ID);
    let handoff =
        |cx: &mut VisualTestContext| view.read_with(cx, |app, _| app.overview.handing_off());
    let first = handoff(cx).expect("the hand-off plays");
    // Undo plays a new one, and it is gone once played.
    scroll_to(cx, crate::overview::CALLOUT_UNDO_ID);
    click(cx, crate::overview::CALLOUT_UNDO_ID);
    let undo = handoff(cx).expect("Undo plays it back");
    assert!(undo > first);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(1));
    draw(cx);
    assert_eq!(handoff(cx), None);

    // Leaving drops what plays or fades, so coming back replays nothing.
    scroll_to(cx, crate::overview::CALLOUT_BUTTON_ID);
    click(cx, crate::overview::CALLOUT_BUTTON_ID);
    assert!(handoff(cx).is_some());
    view.update(cx, |app, _| {
        app.overview
            .hover(Some(crate::overview_model::FocusKey::Card(
                "firefox".into(),
            )));
    });
    click(cx, "nav-rules");
    assert_eq!(handoff(cx), None);
    assert!(view.read_with(cx, |app, _| app.overview.fading().is_none()));
    click(cx, "nav-overview");
    assert_eq!(handoff(cx), None, "coming back does not replay it");
}

/// The callout over a store that will not write (the file changed on disk
/// under an unsaved edit): nothing is written, the callout still offers
/// the switch with no Undo, and it says why.
#[gpui_kit::test]
fn the_callout_says_why_when_the_save_is_refused(cx: &mut TestAppContext) {
    use pwrroute_engine::Precedence;
    let (view, cx, path) = open_overview(
        cx,
        Some(fixtures::overview_config()),
        fixtures::overview_config_rules(),
        fixtures::overview_config_day(),
    );
    view.update(cx, |app, cx| {
        app.store.set_enabled(Selection::Handler(0), false);
        app.refresh(cx);
    });
    written_by_someone_else(&path, 2);
    scroll_to(cx, crate::overview::CALLOUT_BUTTON_ID);
    click(cx, crate::overview::CALLOUT_BUTTON_ID);
    draw(cx);
    assert_eq!(
        RulesFile::load(&path).unwrap().precedence,
        Precedence::Config
    );
    assert_eq!(
        view.read_with(cx, |app, _| app.store.precedence()),
        Precedence::Config
    );
    assert!(!view.read_with(cx, |app, _| app.overview.just_overridden));
    assert_eq!(
        view.read_with(cx, |app, _| app.overview.handing_off()),
        None
    );
    assert!(
        cx.debug_bounds(crate::overview::CALLOUT_BUTTON_ID)
            .is_some()
    );
    assert!(cx.debug_bounds(crate::overview::CALLOUT_UNDO_ID).is_none());
    assert!(
        cx.debug_bounds(crate::overview::CALLOUT_REFUSED_ID)
            .is_some()
    );
}

/// The setup card over a rules.json that does not load: the choice writes
/// nothing, and the card stays with the reason.
#[gpui_kit::test]
fn the_setup_card_stays_when_rules_json_does_not_load(cx: &mut TestAppContext) {
    let rules = RulesFile {
        default_browser: String::new(),
        default_profile: String::new(),
        ..fixtures::overview_rules()
    };
    let (view, cx, path) = open_overview(cx, None, rules, Vec::new());
    std::fs::write(&path, "{ not json").unwrap();
    click(cx, "overview-setup-choose");
    // Firefox has no profiles: it is saved at once, or would be.
    click(cx, "overview-setup-choice-1");
    draw(cx);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    assert!(cx.debug_bounds("overview-setup").is_some());
    assert!(cx.debug_bounds(crate::overview::SETUP_REFUSED_ID).is_some());
    assert!(matches!(
        view.read_with(cx, |app, _| app.overview.refused.clone()),
        Some(crate::overview::Refused::Setup(reason)) if reason.contains("did not load")
    ));
}

/// The Rules list's hits column counts today's links per rule from the
/// feed, as the Overview's rows do.
#[gpui_kit::test]
fn rules_rows_show_todays_hits(cx: &mut TestAppContext) {
    let (view, cx, _) = open_overview(
        cx,
        None,
        fixtures::overview_rules(),
        fixtures::overview_rules_day(),
    );
    click(cx, "nav-rules");
    let hits = cx.update(|_, cx| {
        let app = view.read(cx);
        crate::rules_view::todays_hits(app, cx)
    });
    let hits: Vec<&str> = hits.iter().map(|hits| hits.as_ref()).collect();
    assert_eq!(hits[..5], ["214", "88", "57", "402", "9"], "{hits:?}");
    assert!(hits[5..].iter().all(|hits| *hits == "\u{2014}"), "{hits:?}");
}

#[test]
fn the_overview_reads_the_routers_config_trouble() {
    use crate::overview_model::ConfigTrouble;
    use crate::router::RoutingSource;
    let trouble = crate::overview::config_trouble;
    // Never loaded, and nothing else either: the router's fallback, with
    // its load error.
    let source = RoutingSource {
        config_not_loaded: true,
        fallback: Some("no config has loaded".into()),
        error: Some("SyntaxError: Unexpected token\nat line 42".into()),
        ..RoutingSource::default()
    };
    assert_eq!(
        trouble(source, None),
        Some(ConfigTrouble::NothingLoaded(
            "SyntaxError: Unexpected token".into()
        ))
    );
    // Never loaded, with rules.json routing without it.
    let source = RoutingSource {
        config_not_loaded: true,
        rules: Some("rules.json".into()),
        error: Some("SyntaxError: Unexpected token\nat line 42".into()),
        ..RoutingSource::default()
    };
    assert_eq!(
        trouble(source, None),
        Some(ConfigTrouble::NotLoaded(
            "SyntaxError: Unexpected token".into()
        ))
    );
    // A reload failed: the last load still routes.
    let source = RoutingSource {
        error: Some("SyntaxError: Unexpected token".into()),
        ..RoutingSource::default()
    };
    assert_eq!(
        trouble(source, None),
        Some(ConfigTrouble::ReloadFailed(
            "SyntaxError: Unexpected token".into()
        ))
    );
    // The router routes; only this window could not read the file.
    assert_eq!(
        trouble(RoutingSource::default(), Some("It did not load: x")),
        Some(ConfigTrouble::ReloadFailed("It did not load: x".into()))
    );
    assert_eq!(trouble(RoutingSource::default(), None), None);
}

// Rules and config views (WP-E): the Rules banner's switch "Let PwrRoute's
// default take everything else", its warnings, Stop using, the overridden
// defaultBrowser row, and the blank state's ways out.
mod rules_and_config_views {
    use std::time::SystemTime;

    use gpui_kit::{TestAppContext, VisualTestContext};
    use pwrroute_engine::{DefaultHost, Engine, Limits, Precedence, RulesFile};

    use super::{click, draw, fixture_home, fixture_store, open_store, scratch, store_for};
    use crate::activity_actions::BrowserChoice;
    use crate::app::Selection;
    use crate::config_view::{self, ConfigInCharge, DefaultKind, Order};
    use crate::rules_store::RulesStore;
    use crate::rules_view::{self, everything_else_name};

    /// A Finicky config written to `dir` from `body`, an object literal.
    fn write_config(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
        let path = dir.join(".finicky.ts");
        std::fs::write(&path, format!("export default {body};\n")).expect("write the config");
        path
    }

    /// What the Rules screen reads in the config at `path`.
    fn described(path: &std::path::Path) -> ConfigInCharge {
        let engine = Engine::load(path, std::rc::Rc::new(DefaultHost), Limits::default());
        ConfigInCharge::new(path, path.parent(), engine.as_ref())
    }

    fn top(cx: &mut VisualTestContext, selector: &'static str) -> gpui_kit::Pixels {
        cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("no element {selector} on screen"))
            .top()
    }

    #[gpui_kit::test]
    fn the_switch_writes_precedence_and_reorders_the_cards(cx: &mut TestAppContext) {
        let dir = scratch("precedence");
        let path = dir.join("rules.json");
        let config = fixture_home().join(".finicky.ts");
        let (view, cx) = open_store(cx, Some(config), fixture_store(&dir));

        // Off, what every upgrade sees (5a): the config's handlers, its
        // defaultBrowser tagged Everything else, then the visual rules.
        assert_eq!(
            view.read_with(cx, |app, _| app.store.precedence()),
            Precedence::Config
        );
        assert!(top(cx, "config-handlers") < top(cx, "config-visual"));
        assert!(cx.debug_bounds("config-default").is_some());
        assert!(cx.debug_bounds("config-default-struck").is_none());
        assert!(cx.debug_bounds("config-everything-else").is_none());
        let (shown, everything_else) = view.read_with(cx, |app, _| {
            (
                app.config.clone().expect("the config is in charge"),
                everything_else_name(&app.store),
            )
        });
        let still = ConfigInCharge {
            modified: None,
            ..shown.clone()
        };
        let off = view.read_with(cx, |app, _| Order::of(&app.store, &shown));
        assert_eq!(off, Order::with(false, true, true));
        assert_eq!(everything_else, "Google Chrome \u{00b7} Personal");
        assert_eq!(
            still.banner_sentence(off, &everything_else, SystemTime::now()),
            "Its rewrites and handlers run first and its defaultBrowser takes everything \
             else. Your visual rules run after it."
        );
        // The visual card (5a): eyebrow and owner sentence, and the tag on
        // the config's defaultBrowser row.
        assert_eq!(
            still.visual_copy(off, &everything_else),
            (
                "Run after the config",
                "~/.finicky.ts's defaultBrowser, Google Chrome, takes everything else.".to_owned()
            )
        );
        assert_eq!(config_view::EVERYTHING_ELSE_TAG, "Everything else");
        assert!(shown.override_warnings(off).is_empty());

        // On (5b): written at once, without the edit debounce, and the
        // page reads in the new order.
        click(cx, config_view::SWITCH_ID);
        assert_eq!(
            RulesFile::load(&path).unwrap().precedence,
            Precedence::PwrRoute
        );
        assert!(top(cx, "config-visual") < top(cx, "config-handlers"));
        assert!(top(cx, "config-handlers") < top(cx, "config-everything-else"));
        assert!(cx.debug_bounds("config-default").is_none());
        assert!(cx.debug_bounds("config-default-struck").is_some());
        let flips = view.read_with(cx, |app, _| app.config.as_ref().map(|c| c.flips));
        assert_eq!(flips, Some(1));
        let on = view.read_with(cx, |app, _| Order::of(&app.store, &shown));
        assert_eq!(on, Order::with(true, true, true));
        assert_eq!(
            still.banner_sentence(on, &everything_else, SystemTime::now()),
            "Your rules run first, then its handlers. PwrRoute's default, Google Chrome \
             \u{00b7} Personal, takes everything else."
        );
        // The visual card (5b), the struck row's note and the Everything
        // else row's tag.
        assert_eq!(
            still.visual_copy(on, &everything_else),
            (
                "Run before the config's handlers",
                "They run before the config's handlers.".to_owned()
            )
        );
        assert_eq!(
            config_view::STRUCK_NOTE,
            "not used: PwrRoute's default takes everything else"
        );
        assert_eq!(config_view::PWRROUTE_DEFAULT_TAG, "PwrRoute's default");

        // Off again: the field leaves the file, as the Go host wrote it.
        click(cx, config_view::SWITCH_ID);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("precedence"), "{text}");
        assert!(top(cx, "config-handlers") < top(cx, "config-visual"));
        assert!(cx.debug_bounds("config-default").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[gpui_kit::test]
    fn the_switch_warns_about_a_function_a_null_and_a_catch_all(cx: &mut TestAppContext) {
        let dir = scratch("override-warnings");
        let function = write_config(
            &dir,
            r#"{
                defaultBrowser: (url) => url.host.endsWith("example.org") ? "Firefox" : "Safari",
                handlers: [{ match: "chat.example.com/*", browser: "Firefox" }],
            }"#,
        );
        let on = Order::with(true, true, true);
        let off = Order::with(false, true, true);
        let config = described(&function);
        assert_eq!(config.default_kind, DefaultKind::Function);
        assert_eq!(
            config.override_warnings(on),
            ["Its defaultBrowser function will no longer run."]
        );
        // Off, the function still runs: nothing to warn about.
        assert!(config.override_warnings(off).is_empty());
        assert_eq!(
            config.owner_sentence(off, "Safari"),
            format!(
                "{}'s defaultBrowser function picks where everything else goes.",
                config.path
            )
        );

        let null = write_config(
            &dir,
            r#"{
                defaultBrowser: null,
                handlers: [{ match: "chat.example.com/*", browser: "Firefox" }],
            }"#,
        );
        let config = described(&null);
        assert_eq!(config.default_kind, DefaultKind::NotOpened);
        assert_eq!(
            config.override_warnings(on),
            ["Unmatched links will now open instead of being dropped."]
        );
        assert!(config.override_warnings(off).is_empty());

        let catch_all = write_config(
            &dir,
            r#"{
                defaultBrowser: "Safari",
                handlers: [
                    { match: "chat.example.com/*", browser: "Firefox" },
                    { match: ["docs.example.com/*", "*"], browser: "Firefox" },
                    { match: "*", browser: "Safari" },
                ],
            }"#,
        );
        let config = described(&catch_all);
        assert_eq!(config.catch_all, Some(1));
        // A catch-all is so either way of the switch.
        for order in [on, off] {
            assert_eq!(
                config.override_warnings(order),
                ["Config handler 2 matches every link, so PwrRoute's default never applies."]
            );
        }

        // Off, only the catch-all shows; on, the function's line stacks
        // above it.
        let both = write_config(
            &dir,
            r#"{
                defaultBrowser: () => "Safari",
                handlers: [{ match: /.*/, browser: "Firefox" }],
            }"#,
        );
        let (_view, cx) = open_store(cx, Some(both), fixture_store(&dir));
        assert!(cx.debug_bounds("config-warning-0").is_some());
        assert!(cx.debug_bounds("config-warning-1").is_none());
        click(cx, config_view::SWITCH_ID);
        assert!(cx.debug_bounds("config-warning-1").is_some());
        assert!(cx.debug_bounds("config-warning-2").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[gpui_kit::test]
    fn a_config_that_does_not_load_has_no_warnings_and_pwrroute_takes_the_rest(
        cx: &mut TestAppContext,
    ) {
        let dir = scratch("override-broken");
        let config = fixture_home().join("missing.finicky.ts");
        let (view, cx) = open_store(cx, Some(config), fixture_store(&dir));
        let shown = view
            .read_with(cx, |app, _| app.config.clone())
            .expect("the config is in charge");
        let order = view.read_with(cx, |app, _| Order::of(&app.store, &shown));
        assert!(order.pwrroute_default);
        assert!(shown.override_warnings(order).is_empty());
        // The banner agrees with the router, whichever way the switch is:
        // a config that never loaded is left out, so the rules run without
        // it and PwrRoute's default (`rules.json`'s) takes the rest.
        for rules_first in [false, true] {
            let order = Order::with(rules_first, true, false);
            let sentence = shown.banner_sentence(order, "Google Chrome", SystemTime::now());
            assert!(
                sentence.starts_with(
                    "Until it loads, your rules run without it and PwrRoute's default, \
                     Google Chrome, takes everything else. It did not load:"
                ),
                "{sentence}"
            );
            assert_eq!(
                shown.visual_copy(order, "Google Chrome \u{00b7} Personal"),
                (
                    "Run without the config",
                    "They run without it until ~/missing.finicky.ts loads, and PwrRoute's \
                     default, Google Chrome \u{00b7} Personal, takes everything else."
                        .to_owned()
                )
            );
            // Nothing loaded at all, rules.json included: every link goes
            // to the fallback and the rules wait.
            let nothing = order.nothing_runs(true);
            let sentence = shown.banner_sentence(nothing, "Google Chrome", SystemTime::now());
            assert!(
                sentence.starts_with(
                    "Until it loads, every link opens in PwrRoute's default, Google Chrome. \
                     It did not load:"
                ),
                "{sentence}"
            );
            assert_eq!(
                shown
                    .visual_copy(nothing, "Google Chrome \u{00b7} Personal")
                    .1,
                "They wait for it: every link opens in PwrRoute's default, Google Chrome \
                 \u{00b7} Personal, until ~/missing.finicky.ts loads."
            );
            // A failed reload: the last load still routes.
            let stale = order.stale(true);
            let sentence = shown.banner_sentence(stale, "Google Chrome", SystemTime::now());
            assert!(
                sentence.starts_with(
                    "It didn't reload, so links still route with the version that last \
                     loaded. It did not load:"
                ),
                "{sentence}"
            );
            assert_eq!(
                shown.owner_sentence(stale, "Google Chrome"),
                "They route with the version of ~/missing.finicky.ts that last loaded."
            );
        }
        // No handlers to strike; the row PwrRoute's default takes is shown.
        assert!(cx.debug_bounds("config-everything-else").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[gpui_kit::test]
    fn stop_using_turns_the_config_off_and_shows_the_visual_rules(cx: &mut TestAppContext) {
        let dir = scratch("stop-using");
        let settings = crate::settings_store::SettingsStore::load(Some(dir.clone()));
        cx.update(|cx| crate::settings_store::install(settings, None, cx));
        let config = fixture_home().join(".finicky.ts");
        let (view, cx) = open_store(cx, Some(config.clone()), fixture_store(&dir));
        assert!(cx.debug_bounds("config-handlers").is_some());

        click(cx, config_view::STOP_ID);
        let saved = crate::settings_store::Settings::load(&dir).unwrap();
        assert!(saved.source.visual_only);
        // Its "Found …" notice does not come back.
        assert_eq!(saved.source.dismissed, [config]);
        assert!(view.read_with(cx, |app, _| app.config.is_none()));
        draw(cx);
        assert!(cx.debug_bounds("config-handlers").is_none());
        assert!(cx.debug_bounds("handler-0").is_some());
        assert!(cx.debug_bounds("import-found").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// First run's Don't use it while the main window is open: the real
    /// `set_source_and_show` turns the config off and shows the window the
    /// visual rules.
    #[gpui_kit::test]
    fn dont_use_it_with_the_main_window_open_shows_the_visual_rules(cx: &mut TestAppContext) {
        let dir = scratch("dont-use-it-open");
        let settings = crate::settings_store::SettingsStore::load(Some(dir.clone()));
        cx.update(|cx| crate::settings_store::install(settings, None, cx));
        let config = fixture_home().join(".finicky.ts");
        let (view, cx) = open_store(cx, Some(config.clone()), fixture_store(&dir));
        assert!(cx.debug_bounds("config-handlers").is_some());

        let hooks = crate::onboarding::hooks_with(
            Some(dir.clone()),
            Some(dir.join("rules.json")),
            Some(config),
            std::rc::Rc::new(crate::importer_view::set_source_and_show),
            std::rc::Rc::new(|_, _| {}),
        );
        // From outside the main window, as first run's own window calls it.
        cx.cx.update(|cx| {
            (hooks.on_import)(
                crate::first_run::ImportChoice::DontUse,
                std::rc::Rc::new(|_, _| {}),
                cx,
            );
        });
        cx.run_until_parked();
        draw(cx);
        assert!(view.read_with(cx, |app, _| app.config.is_none()));
        assert!(cx.debug_bounds("config-handlers").is_none());
        assert!(cx.debug_bounds("handler-0").is_some());
        let saved = crate::settings_store::Settings::load(&dir).unwrap();
        assert!(saved.source.visual_only);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[gpui_kit::test]
    fn the_blank_state_chooses_a_browser_from_a_list(cx: &mut TestAppContext) {
        let dir = scratch("blank-browser");
        let path = dir.join("rules.json");
        let (view, cx) = open_store(cx, None, RulesStore::load(Some(path.clone())));
        view.update(cx, |app, _| {
            app.browsers = vec![
                BrowserChoice {
                    name: "Firefox".into(),
                    profiles: Vec::new(),
                },
                BrowserChoice {
                    name: "Google Chrome".into(),
                    profiles: vec!["Personal".into(), "Work".into()],
                },
            ];
        });
        draw(cx);
        assert!(cx.debug_bounds("blank-browser-0").is_none());
        click(cx, rules_view::BLANK_BROWSER_ID);
        assert!(cx.debug_bounds("blank-browser-0").is_some());
        // The button closes what it opened.
        click(cx, rules_view::BLANK_BROWSER_ID);
        assert!(cx.debug_bounds("blank-browser-0").is_none());

        click(cx, rules_view::BLANK_BROWSER_ID);
        click(cx, "blank-browser-1");
        // Written at once: the blank gives way to the empty cards.
        let file = RulesFile::load(&path).unwrap();
        assert_eq!(file.default_browser, "Google Chrome");
        assert_eq!(file.default_profile, "");
        assert!(cx.debug_bounds("rules-blank").is_none());
        assert!(cx.debug_bounds("empty-add-handler").is_some());
        assert_eq!(
            view.read_with(cx, |app, _| everything_else_name(&app.store)),
            "Google Chrome"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[gpui_kit::test]
    fn the_blank_state_opens_settings_on_configuration(cx: &mut TestAppContext) {
        cx.update(crate::settings::install);
        let dir = scratch("blank-settings");
        let (_view, cx) = open_store(cx, None, RulesStore::load(Some(dir.join("rules.json"))));
        click(cx, rules_view::BLANK_SETTINGS_ID);
        cx.run_until_parked();
        let panes: Vec<crate::settings::Pane> = cx.update(|_, cx| {
            cx.windows()
                .into_iter()
                .filter_map(|window| window.downcast::<crate::settings::SettingsWindow>())
                .filter_map(|window| window.read(cx).ok().map(|view| view.pane))
                .collect()
        });
        assert_eq!(panes, [crate::settings::Pane::Configuration]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[gpui_kit::test]
    fn without_a_rules_json_default_the_switch_leaves_the_configs_default(cx: &mut TestAppContext) {
        let dir = scratch("override-no-default");
        let path = dir.join("rules.json");
        let file = RulesFile {
            default_browser: String::new(),
            default_profile: String::new(),
            ..crate::fixtures::rules_file()
        };
        let config = fixture_home().join(".finicky.ts");
        let (view, cx) = open_store(cx, Some(config), store_for(&dir, &file));

        click(cx, config_view::SWITCH_ID);
        assert_eq!(
            RulesFile::load(&path).unwrap().precedence,
            Precedence::PwrRoute
        );
        let (shown, order) = view.read_with(cx, |app, _| {
            let shown = app.config.clone().expect("the config is in charge");
            let order = Order::of(&app.store, &shown);
            (shown, order)
        });
        assert_eq!(order, Order::with(true, false, true));
        assert!(order.needs_default && !order.pwrroute_default);
        // The visual rules still run first, but the config's default stays:
        // its row is not struck, and there is no Everything else row.
        assert!(top(cx, "config-visual") < top(cx, "config-handlers"));
        assert!(cx.debug_bounds("config-default").is_some());
        assert!(cx.debug_bounds("config-default-struck").is_none());
        assert!(cx.debug_bounds("config-everything-else").is_none());
        assert_eq!(
            shown.override_warnings(order),
            ["Pick a default browser for PwrRoute to take everything else."]
        );
        assert!(cx.debug_bounds("config-warning-0").is_some());
        let still = ConfigInCharge {
            modified: None,
            ..shown.clone()
        };
        assert_eq!(
            still.banner_sentence(order, "Safari", SystemTime::now()),
            "Your rules run first, then its handlers. Its defaultBrowser still takes \
             everything else."
        );

        // A default naming PwrRoute is no default either: the router would
        // refuse it as a loop.
        for itself in ["PwrRoute", "com.pwrdrvr.pwrroute"] {
            let file = RulesFile {
                default_browser: itself.into(),
                precedence: Precedence::PwrRoute,
                ..crate::fixtures::rules_file()
            };
            let store = store_for(&dir, &file);
            assert_eq!(
                Order::of(&store, &shown),
                Order::with(true, false, true),
                "{itself}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[gpui_kit::test]
    fn a_refused_save_leaves_the_switch_where_the_router_has_it(cx: &mut TestAppContext) {
        // No configuration directory: nothing can be written.
        let config = fixture_home().join(".finicky.ts");
        let (view, cx) = open_store(cx, Some(config.clone()), RulesStore::load(None));
        click(cx, config_view::SWITCH_ID);
        let (precedence, flips) = view.read_with(cx, |app, _| {
            (
                app.store.precedence(),
                app.config.as_ref().map(|config| config.flips),
            )
        });
        assert_eq!(precedence, Precedence::Config);
        assert_eq!(flips, Some(0));
        assert!(top(cx, "config-handlers") < top(cx, "config-visual"));
        assert!(cx.debug_bounds("config-default-struck").is_none());
        // The banner says why, under the switch.
        assert!(cx.debug_bounds("config-warning-0").is_some());

        // The file changed on disk under an unsaved edit.
        let dir = scratch("override-refused");
        let path = dir.join("rules.json");
        let (view, cx) = open_store(cx, Some(config), fixture_store(&dir));
        view.update(cx, |app, cx| {
            app.store.set_enabled(Selection::Handler(0), false);
            app.refresh(cx);
        });
        super::written_by_someone_else(&path, 2);
        click(cx, config_view::SWITCH_ID);
        assert_eq!(
            view.read_with(cx, |app, _| app.store.precedence()),
            Precedence::Config
        );
        assert_eq!(
            RulesFile::load(&path).unwrap().precedence,
            Precedence::Config
        );
        assert!(cx.debug_bounds("config-default-struck").is_none());
        assert!(cx.debug_bounds("config-warning-0").is_some());
        let _ = std::fs::remove_dir_all(&dir);

        // The write itself fails (the folder is read-only): the switch goes
        // back to the file's order, and a later save does not write it.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = scratch("override-write-fails");
            let path = dir.join("rules.json");
            let (view, cx) = open_store(cx, Some(fixture_home().join(".finicky.ts")), {
                fixture_store(&dir)
            });
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
            click(cx, config_view::SWITCH_ID);
            let (precedence, flips, refused) = view.read_with(cx, |app, _| {
                let config = app.config.as_ref().unwrap();
                (app.store.precedence(), config.flips, config.refused.clone())
            });
            assert_eq!(precedence, Precedence::Config);
            assert_eq!(flips, 0);
            assert!(refused.is_some());
            assert!(cx.debug_bounds("config-warning-0").is_some());
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
            view.update(cx, |app, cx| app.save_now(cx));
            assert_eq!(
                RulesFile::load(&path).unwrap().precedence,
                Precedence::Config
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[gpui_kit::test]
    fn the_switch_keeps_the_selection(cx: &mut TestAppContext) {
        let dir = scratch("override-selection");
        let config = fixture_home().join(".finicky.ts");
        let (view, cx) = open_store(cx, Some(config), fixture_store(&dir));
        view.update(cx, |app, cx| {
            app.visual_rules_expanded = true;
            app.selection = None;
            cx.notify();
        });
        draw(cx);
        // Nothing selected stays so: the editor does not open on a row.
        click(cx, config_view::SWITCH_ID);
        assert_eq!(view.read_with(cx, |app, _| app.selection), None);
        view.update(cx, |app, cx| {
            app.selection = Some(Selection::Handler(0));
            cx.notify();
        });
        draw(cx);
        click(cx, config_view::SWITCH_ID);
        assert_eq!(
            view.read_with(cx, |app, _| app.selection),
            Some(Selection::Handler(0))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn everything_else_names_a_bundle_id_by_its_app() {
        let dir = scratch("override-bundle-id");
        let file = RulesFile {
            default_browser: "com.google.Chrome".into(),
            default_profile: "Personal".into(),
            ..crate::fixtures::rules_file()
        };
        let store = store_for(&dir, &file);
        assert_eq!(
            everything_else_name(&store),
            "Google Chrome \u{00b7} Personal"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// --- the rule editor's pickers and the ways to start a rule ------------

/// A window over the fixture rules with the Overview's contrived scan:
/// Chrome, Firefox, Arc, Safari and Edge with made-up profiles.
fn open_with_scan<'a>(
    cx: &'a mut TestAppContext,
    dir: &Path,
) -> (Entity<PwrRoute>, &'a mut VisualTestContext) {
    let (view, cx) = open_store(cx, None, fixture_store(dir));
    view.update(cx, |app, _| {
        app.installed = fixtures::overview_browsers();
        app.browsers = app
            .installed
            .iter()
            .map(|app| crate::activity_actions::BrowserChoice {
                name: app.name.clone(),
                profiles: app.profiles.clone(),
            })
            .collect();
    });
    draw(cx);
    (view, cx)
}

/// Open in is a lane per browser in the scan, in the scan's order, and a
/// pill is the whole choice; the way out types a name (design:
/// Destination and App Pickers 1c, 1d).
#[gpui_kit::test]
fn open_in_and_profile_pick_from_the_scan(cx: &mut TestAppContext) {
    let dir = scratch("pickers");
    let path = dir.join("rules.json");
    let (view, cx) = open_with_scan(cx, &dir);
    let rule = |cx: &mut VisualTestContext| {
        view.read_with(cx, |app, _| {
            let h = &app.handlers[0];
            (h.browser.clone(), h.profile.clone(), h.private)
        })
    };
    assert_eq!(rule(cx), ("Google Chrome".into(), "Work".into(), false));

    // This Mac's five browsers, and no lane for a missing one.
    for i in 0..5 {
        assert!(
            cx.debug_bounds(format!("destination-lane-{i}").leak())
                .is_some()
        );
    }
    assert!(cx.debug_bounds("destination-lane-5").is_none());
    // Edge: Default, its four profiles, then InPrivate.
    click(cx, "destination-4-4");
    assert_eq!(rule(cx), ("Microsoft Edge".into(), "Travel".into(), false));
    let file = saved(cx, &path);
    assert_eq!(
        (
            file.rules[0].browser.as_str(),
            file.rules[0].profile.as_str()
        ),
        ("Microsoft Edge", "Travel")
    );
    // The lanes stay in the scan's order after a pick.
    click(cx, "destination-4-0");
    assert_eq!(rule(cx), ("Microsoft Edge".into(), String::new(), false));
    click(cx, "destination-4-5");
    assert_eq!(rule(cx), ("Microsoft Edge".into(), String::new(), true));
    // Chrome's Design, by pill; a pick from a lane turns private off.
    click(cx, "destination-0-3");
    assert_eq!(rule(cx), ("Google Chrome".into(), "Design".into(), false));

    // A profile by folder: Chrome's "Profile 2" is Design, so it is no
    // warning; a folder Chrome lacks is, and keeps a pill of its own.
    click(cx, "destination-typed");
    click(cx, "profile");
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("Profile 2");
    draw(cx);
    assert_eq!(rule(cx).1, "Profile 2");
    click(cx, "browser-list");
    assert!(cx.debug_bounds("profile-warning").is_none());
    assert!(
        cx.debug_bounds("destination-0-5").is_none(),
        "no stray pill"
    );
    click(cx, "destination-typed");
    click(cx, "profile");
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("Profile 9");
    draw(cx);
    click(cx, "browser-list");
    assert!(cx.debug_bounds("profile-warning").is_some());
    assert!(cx.debug_bounds("destination-0-4").is_some(), "Profile 9");

    // Any other app by name: typed, and drawn as not installed here, in a
    // lane of its own ahead of the scan's.
    click(cx, "destination-typed");
    click(cx, "browser");
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("Example Viewer");
    draw(cx);
    assert_eq!(rule(cx).0, "Example Viewer");
    assert!(cx.debug_bounds("browser-warning").is_some());
    click(cx, "browser-list");
    assert!(
        cx.debug_bounds("destination-lane-5").is_some(),
        "Example Viewer, then the five"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A new rule starts with no pill on, and a browser that can't open a
/// private window from a link has no private pill.
#[gpui_kit::test]
fn a_new_rules_profile_waits_for_its_browser(cx: &mut TestAppContext) {
    let dir = scratch("pickers-new");
    let (view, cx) = open_with_scan(cx, &dir);
    click(cx, "add-handler");
    let h = 7;
    assert_eq!(
        view.read_with(cx, |app, _| app.selection),
        Some(Selection::Handler(h))
    );
    // The editor's Examples start empty, and say what one is for.
    assert!(cx.debug_bounds("examples-empty").is_some());
    // Safari (fourth on this Mac) opens no private window from a link:
    // its lane is Default alone.
    click(cx, "destination-3-0");
    assert_eq!(
        view.read_with(cx, |app, _| app.handlers[h].browser.clone()),
        "Safari"
    );
    assert!(cx.debug_bounds("destination-3-1").is_none());
    // Chrome's fifth pill is Incognito.
    click(cx, "destination-0-4");
    assert!(view.read_with(cx, |app, _| app.handlers[h].private));
    let _ = std::fs::remove_dir_all(dir);
}

/// "+ app" offers the apps other rules name before any app by name.
#[gpui_kit::test]
fn plus_app_offers_the_apps_the_rules_name(cx: &mut TestAppContext) {
    let dir = scratch("pickers-opener");
    let (view, cx) = open_with_scan(cx, &dir);
    assert!(cx.debug_bounds("opener-any").is_some());
    click(cx, "opener-add");
    click(cx, "opener-choice-0");
    assert_eq!(
        view.read_with(cx, |app, _| app.handlers[0].openers.clone()),
        ["com.tinyspeck.slackmacgap"]
    );
    assert!(cx.debug_bounds("opener-any").is_none());
    // An app the rule already names (as the file panel may pick it) is
    // not added twice.
    view.update_in(cx, |app, window, cx| {
        app.choose_opener("COM.tinyspeck.slackmacgap", window, cx);
    });
    assert_eq!(
        view.read_with(cx, |app, _| app.handlers[0].openers.clone()),
        ["com.tinyspeck.slackmacgap"]
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Under a config, Rules still has Add rule: the visual rules open with
/// the draft selected, and the rail says where it runs (design 1a, 1b).
#[gpui_kit::test]
fn rules_under_a_config_have_add_rule(cx: &mut TestAppContext) {
    let (view, cx) = open_with(cx, Some(fixture_home().join(".finicky.ts")));
    assert!(!view.read_with(cx, |app, _| app.visual_rules_expanded));
    assert!(cx.debug_bounds("config-handlers").is_some());
    click(cx, "add-handler");
    assert!(view.read_with(cx, |app, _| app.visual_rules_expanded));
    let (selection, editing) = view.read_with(cx, |app, _| (app.selection, app.editing()));
    assert_eq!(selection, Some(Selection::Handler(7)));
    assert!(editing);
    assert!(cx.debug_bounds("editor-rail").is_some());
    assert!(cx.debug_bounds("draft-note").is_some());
}

/// A card's "+ Rule" is the browser's, with no profile; a profile's own
/// is on its node (design 2a to 2e).
#[gpui_kit::test]
fn an_overview_card_adds_a_rule_for_its_browser(cx: &mut TestAppContext) {
    let (view, cx, _) = open_overview(
        cx,
        None,
        fixtures::overview_rules(),
        fixtures::overview_rules_day(),
    );
    // Microsoft Edge, no rules yet, shows it at rest.
    scroll_to(cx, "overview-card-add-3");
    click(cx, "overview-card-add-3");
    let (screen, selection) = view.read_with(cx, |app, _| (app.screen, app.selection));
    assert_eq!(screen, Screen::Rules);
    assert_eq!(selection, Some(Selection::Handler(6)));
    let rule = view.read_with(cx, |app, _| app.store.edited().rules[6].clone());
    assert_eq!(
        (rule.browser.as_str(), rule.profile.as_str()),
        ("Microsoft Edge", "")
    );
    // No pattern from a link: with the browser set it would save at once.
    assert_eq!(rule.patterns, [""]);
    assert!(cx.debug_bounds("draft-note").is_some());
    // Not the card's pin: the click was the button's.
    assert_eq!(
        view.read_with(cx, |app, _| app.overview.pinned.clone()),
        None
    );
    // A card that routes links shows it under the pointer, so its counts
    // keep their room at rest.
    click(cx, "nav-overview");
    assert!(cx.debug_bounds("overview-card-add-0").is_none());
    hover(cx, "overview-card-0");
    assert!(cx.debug_bounds("overview-card-add-0").is_some());
}

/// Test a link's "Add a rule…": the suggested pattern, selected, and Try
/// it holding the tested link; the browser is left to choose (design 5a,
/// 5b).
#[gpui_kit::test]
fn test_a_link_adds_a_rule_for_the_link(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    click(cx, "nav-test");
    type_sample(cx);
    click(cx, "test-add-rule");
    // The fixture rewrites x.com to xcancel.com before any rule runs: the
    // pattern is for the URL the rules see, or the rule would never match.
    let expected = pwrroute_engine::suggest_rule("https://xcancel.com/pwrdrvr/status/1842?s=20")
        .candidates[0]
        .clone();
    assert!(expected.starts_with("xcancel.com/"), "{expected}");
    let (screen, selection, handler, sample, selected) = view.read_with(cx, |app, _| {
        let rail = app.rail.as_ref().expect("the editor");
        (
            app.screen,
            app.selection,
            app.handlers[7].clone(),
            (rail.sample.text.clone(), rail.sample_edited),
            rail.patterns[0].selected,
        )
    });
    assert_eq!(screen, Screen::Rules);
    assert_eq!(selection, Some(Selection::Handler(7)));
    assert_eq!(handler.patterns, [expected]);
    assert_eq!(handler.browser, "");
    assert_eq!(sample, (fixtures::TEST_URL.to_owned(), true));
    assert!(selected, "typing replaces the suggestion");
    assert!(cx.debug_bounds("draft-note").is_some());
    // Given a browser, the draft takes the tested link.
    click(cx, "destination-typed");
    cx.simulate_input("Firefox");
    draw(cx);
    let result = view.read_with(cx, |app, _| app.rail.as_ref().unwrap().try_result.clone());
    assert!(
        matches!(
            result,
            Some(crate::try_it::TryResult::Text {
                verdict: crate::try_it::Verdict::Takes,
                ..
            })
        ),
        "{result:?}"
    );
}

/// A Try it link typed on one rule never follows the selection to the
/// rule that takes its place when that rule is deleted.
#[gpui_kit::test]
fn a_deleted_rules_try_it_link_stays_with_it(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    click(cx, "try-sample");
    cx.simulate_keystrokes("cmd-a backspace");
    cx.simulate_input("https://meet.google.com/typed-here");
    draw(cx);
    click(cx, "rail-delete");
    let (selection, sample, edited) = view.read_with(cx, |app, _| {
        let rail = app.rail.as_ref().expect("the editor");
        (app.selection, rail.sample.text.clone(), rail.sample_edited)
    });
    assert_eq!(selection, Some(Selection::Handler(0)));
    assert!(!edited);
    assert_ne!(sample, "https://meet.google.com/typed-here");
}

/// A scanned browser whose profiles the scan could not read still lets
/// a profile be named.
#[gpui_kit::test]
fn a_browser_without_readable_profiles_still_takes_a_named_one(cx: &mut TestAppContext) {
    let dir = scratch("pickers-no-profiles");
    let (view, cx) = open_with_scan(cx, &dir);
    click(cx, "add-handler");
    // Firefox, second on this Mac, with no profiles in the scan: Default
    // and its private window.
    click(cx, "destination-1-0");
    assert!(cx.debug_bounds("destination-1-2").is_none());
    click(cx, "destination-typed");
    click(cx, "profile");
    cx.simulate_input("default-release");
    draw(cx);
    assert_eq!(
        view.read_with(cx, |app, _| app.handlers[7].profile.clone()),
        "default-release"
    );
    // Back in the lanes it is a pill of its own, on, and no warning.
    click(cx, "browser-list");
    assert!(cx.debug_bounds("destination-1-1").is_some());
    assert!(cx.debug_bounds("profile-warning").is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// Edit › Paste does what ⌘V does in the focused field, and ⌘V itself still
/// reaches the field with the Edit menu's shortcuts installed.
#[gpui_kit::test]
fn edit_menu_rows_reach_the_focused_text_field(cx: &mut TestAppContext) {
    cx.update(crate::app_menu::install);
    let (view, cx) = open(cx);
    view.update_in(cx, |app, window, cx| {
        app.navigate(Screen::TestLink, window, cx);
    });
    draw(cx);
    click(cx, "test-url");
    cx.simulate_keystrokes("cmd-a backspace");
    let url = |view: &Entity<PwrRoute>, cx: &mut VisualTestContext| {
        view.read_with(cx, |app, _| app.test.url.text.clone())
    };
    assert_eq!(url(&view, cx), "");

    cx.update(|_, cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
            "https://example.com/a".to_owned(),
        ));
    });
    cx.simulate_keystrokes("cmd-v");
    assert_eq!(url(&view, cx), "https://example.com/a");

    // The menu's rows go to the window in front.
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    assert!(cx.update(|_, cx| cx.active_window().is_some()));
    cx.dispatch_action(crate::app_menu::SelectAll);
    cx.update(|_, cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
            "https://example.com/b".to_owned(),
        ));
    });
    cx.dispatch_action(crate::app_menu::Paste);
    cx.run_until_parked();
    assert_eq!(url(&view, cx), "https://example.com/b");
}

/// File › Close Window closes the window in front.
#[gpui_kit::test]
fn close_window_closes_the_window_in_front(cx: &mut TestAppContext) {
    cx.update(crate::app_menu::install);
    let (_, vcx) = open(cx);
    vcx.update(|window, _| window.activate_window());
    vcx.run_until_parked();
    vcx.dispatch_action(crate::app_menu::CloseWindow);
    vcx.run_until_parked();
    assert!(cx.update(|cx| cx.windows().is_empty()));
}

// --- the Windows and Linux strip's menu bar ----------------------------------

/// The window, with the menus and shortcuts as Windows has them: the strip
/// draws its menu bar on this Mac too.
fn open_with_strip(cx: &mut TestAppContext) -> (Entity<PwrRoute>, &mut VisualTestContext) {
    cx.update(|cx| crate::app_menu::install_for(crate::chrome::Chrome::Windows, cx));
    let (view, vcx) = open(cx);
    vcx.update(|window, _| window.activate_window());
    vcx.run_until_parked();
    // Past the Alt that comes with becoming active.
    vcx.executor()
        .advance_clock(crate::menu_bar::FAKE_ALT_WINDOW);
    draw(vcx);
    (view, vcx)
}

fn bar_state(view: &Entity<PwrRoute>, cx: &mut VisualTestContext) -> crate::menu_bar::State {
    let bar = view.read_with(cx, |app, _| app.menu_bar.clone());
    bar.read_with(cx, |bar, _| bar.state)
}

fn screen(view: &Entity<PwrRoute>, cx: &mut VisualTestContext) -> Screen {
    view.read_with(cx, |app, _| app.screen)
}

/// Keys, then the frame they leave.
fn keys(cx: &mut VisualTestContext, keys: &str) {
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
    draw(cx);
}

/// macOS has the system's menu bar: its strip draws none.
#[gpui_kit::test]
fn the_mac_strip_has_no_menu_bar(cx: &mut TestAppContext) {
    cx.update(|cx| crate::app_menu::install_for(crate::chrome::Chrome::Mac, cx));
    let (_, cx) = open(cx);
    assert!(cx.debug_bounds("menu-bar").is_none());
    assert!(cx.debug_bounds("caption-close").is_none());
}

/// Windows draws the template's titles after the brand, and its own
/// caption buttons on the right.
#[gpui_kit::test]
fn the_windows_strip_draws_the_template_and_the_caption_buttons(cx: &mut TestAppContext) {
    let (_, cx) = open_with_strip(cx);
    let titles = ["file", "edit", "view", "window", "help"].map(|name| {
        cx.debug_bounds(Box::leak(format!("menu-title-{name}").into_boxed_str()))
            .unwrap_or_else(|| panic!("no {name} title"))
    });
    for pair in titles.windows(2) {
        assert!(pair[0].right() <= pair[1].left(), "{titles:?}");
    }
    // Inside the strip, after the brand at x=16.
    assert!(titles[0].left() > px(crate::chrome::OTHER_BRAND_X + 40.0));
    assert!(titles.iter().all(|title| title.bottom() <= px(41.0)));
    let width = cx.update(|window, _| window.viewport_size().width);
    let close = cx.debug_bounds("caption-close").expect("the close button");
    assert_eq!(close.right(), width);
    assert_eq!(close.size.width, px(crate::chrome::CAPTION_WIDTH));
    let minimize = cx.debug_bounds("caption-minimize").expect("minimize");
    assert!(minimize.right() <= cx.debug_bounds("caption-maximize").unwrap().left());
}

/// A click opens a title's menu; a row runs its action and closes it.
#[gpui_kit::test]
fn a_menu_row_runs_its_action(cx: &mut TestAppContext) {
    let (view, cx) = open_with_strip(cx);
    assert!(cx.debug_bounds("menu-card-view").is_none());
    click(cx, "menu-title-view");
    let card = cx.debug_bounds("menu-card-view").expect("View's menu");
    // Under the strip, below its title.
    assert!(card.top() >= px(41.0));
    click(cx, "menu-row-view-activity");
    cx.run_until_parked();
    draw(cx);
    assert_eq!(screen(&view, cx), Screen::Activity);
    assert!(cx.debug_bounds("menu-card-view").is_none());
    assert_eq!(bar_state(&view, cx), crate::menu_bar::State::default());
}

/// With a menu open the pointer switches menus by hovering, a second press
/// on the open title closes it, and a press elsewhere lets go.
#[gpui_kit::test]
fn hovering_across_titles_switches_menus(cx: &mut TestAppContext) {
    let (view, cx) = open_with_strip(cx);
    // At rest, hovering opens nothing.
    hover(cx, "menu-title-edit");
    draw(cx);
    assert!(cx.debug_bounds("menu-card-edit").is_none());
    click(cx, "menu-title-file");
    assert!(cx.debug_bounds("menu-card-file").is_some());
    hover(cx, "menu-title-help");
    draw(cx);
    assert!(cx.debug_bounds("menu-card-file").is_none());
    assert!(cx.debug_bounds("menu-card-help").is_some());
    click(cx, "menu-title-help");
    assert!(cx.debug_bounds("menu-card-help").is_none());
    // A press in the pane closes an open menu.
    click(cx, "menu-title-window");
    assert!(cx.debug_bounds("menu-card-window").is_some());
    let pane = cx.debug_bounds(ROOT_ID).expect("the window");
    cx.simulate_click(
        point(pane.center().x, pane.bottom() - px(40.0)),
        Modifiers::default(),
    );
    draw(cx);
    assert!(cx.debug_bounds("menu-card-window").is_none());
    assert_eq!(bar_state(&view, cx), crate::menu_bar::State::default());
}

/// Alt puts the keyboard on File; the arrows walk to a row and Enter runs
/// it. F10 does what Alt does, and Escape steps back out.
#[gpui_kit::test]
fn alt_and_the_arrows_reach_a_row(cx: &mut TestAppContext) {
    let (view, cx) = open_with_strip(cx);
    view.update_in(cx, |app, window, cx| {
        app.navigate(Screen::Overview, window, cx);
    });
    keys(cx, "alt");
    let state = bar_state(&view, cx);
    assert_eq!(
        (state.title, state.open, state.keyboard),
        (Some(0), false, true)
    );
    assert!(cx.debug_bounds("menu-card-file").is_none());
    // File, Edit, View; Down opens it on Overview, then Rules.
    keys(cx, "right right down down");
    assert!(cx.debug_bounds("menu-card-view").is_some());
    assert_eq!(bar_state(&view, cx).row, Some(1));
    keys(cx, "enter");
    assert_eq!(screen(&view, cx), Screen::Rules);
    assert_eq!(bar_state(&view, cx), crate::menu_bar::State::default());

    keys(cx, "f10");
    assert_eq!(bar_state(&view, cx).title, Some(0));
    keys(cx, "down");
    assert!(cx.debug_bounds("menu-card-file").is_some());
    keys(cx, "escape");
    assert!(cx.debug_bounds("menu-card-file").is_none());
    assert_eq!(bar_state(&view, cx).title, Some(0));
    keys(cx, "escape");
    assert_eq!(bar_state(&view, cx), crate::menu_bar::State::default());
    // At rest the keys are the window's again: Alt+letter for no title
    // and plain arrows leave the bar alone.
    keys(cx, "down alt-z");
    assert_eq!(bar_state(&view, cx), crate::menu_bar::State::default());
}

/// Edit's rows, from the pointer or the keyboard, still replay their key
/// into the field that has focus: the bar never takes it.
#[gpui_kit::test]
fn edit_rows_on_the_strip_reach_the_focused_field(cx: &mut TestAppContext) {
    let (view, cx) = open_with_strip(cx);
    view.update_in(cx, |app, window, cx| {
        app.navigate(Screen::TestLink, window, cx);
    });
    draw(cx);
    click(cx, "test-url");
    cx.simulate_keystrokes("cmd-a backspace");
    let url = |view: &Entity<PwrRoute>, cx: &mut VisualTestContext| {
        view.read_with(cx, |app, _| app.test.url.text.clone())
    };
    cx.update(|_, cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
            "https://example.com/a".to_owned(),
        ));
    });
    click(cx, "menu-title-edit");
    click(cx, "menu-row-edit-paste");
    cx.run_until_parked();
    assert_eq!(url(&view, cx), "https://example.com/a");

    // Alt+E opens Edit on Cut; Select All is two rows past Paste's
    // separator.
    cx.update(|_, cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
            "https://example.com/b".to_owned(),
        ));
    });
    keys(cx, "alt-e down down down enter");
    cx.run_until_parked();
    keys(cx, "alt-e down down enter");
    cx.run_until_parked();
    assert_eq!(url(&view, cx), "https://example.com/b");
}

/// Each row shows the keymap's shortcut, in Windows' words.
#[gpui_kit::test]
fn strip_rows_show_the_keymaps_shortcuts(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::app_menu::install_for(crate::chrome::Chrome::Windows, cx);
        let menus = crate::menu_bar::strip_menus(None, cx);
        let shortcut = |menu: &str, row: &str| {
            menus
                .iter()
                .find(|m| m.name == menu)
                .and_then(|m| {
                    m.rows.iter().find_map(|r| match r {
                        crate::menu_bar::Row::Action {
                            label, shortcut, ..
                        } if label == row => Some(shortcut.clone()),
                        _ => None,
                    })
                })
                .unwrap_or_else(|| panic!("no {menu} › {row}"))
                .map(|shortcut| shortcut.to_string())
        };
        assert_eq!(
            shortcut("File", "Settings\u{2026}").as_deref(),
            Some("Ctrl+,")
        );
        assert_eq!(shortcut("File", "Close Window").as_deref(), Some("Ctrl+W"));
        assert_eq!(shortcut("File", "Quit").as_deref(), Some("Ctrl+Q"));
        assert_eq!(shortcut("Edit", "Paste").as_deref(), Some("Ctrl+V"));
        assert_eq!(shortcut("Window", "Minimize").as_deref(), Some("Ctrl+M"));
        assert_eq!(shortcut("Help", "About PwrRoute"), None);
    });
}

/// Window lists the windows with a strip after Minimize, this one checked;
/// a row brings its window forward.
#[gpui_kit::test]
fn the_window_menu_lists_the_open_windows(cx: &mut TestAppContext) {
    let (_, main) = open_with_strip(cx);
    let main_handle = main.update(|window, _| window.window_handle());
    let (_, other) = cx.add_window_view(|_, cx| crate::logs_window::LogsWindow::new(None, cx));
    draw(other);
    other.update(|window, _| window.activate_window());
    other.run_until_parked();
    draw(other);
    click(other, "menu-title-window");
    assert!(other.debug_bounds("menu-row-window-minimize").is_some());
    assert!(other.debug_bounds("menu-row-window-pwrroute").is_some());
    assert!(other.debug_bounds("menu-row-window-logs").is_some());
    click(other, "menu-row-window-pwrroute");
    other.run_until_parked();
    assert_eq!(cx.update(|cx| cx.active_window()), Some(main_handle));
}

/// View's rows open their screen in the window in front, from the menu
/// bar (macOS) as from the strip: the action arrives inside that window's
/// own dispatch.
#[gpui_kit::test]
fn view_rows_open_their_screen_in_the_window_in_front(cx: &mut TestAppContext) {
    cx.update(crate::app_menu::install);
    let (view, cx) = open(cx);
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    cx.dispatch_action(crate::app_menu::ShowActivity);
    cx.run_until_parked();
    assert_eq!(screen(&view, cx), Screen::Activity);
    cx.dispatch_action(crate::app_menu::ShowOverview);
    cx.run_until_parked();
    assert_eq!(screen(&view, cx), Screen::Overview);
}

/// Help › Changelog from the Changelog window's own strip brings that
/// window forward; it never opens a second.
#[gpui_kit::test]
fn a_help_window_chosen_from_itself_stays_one_window(cx: &mut TestAppContext) {
    cx.update(|cx| crate::app_menu::install_for(crate::chrome::Chrome::Linux, cx));
    cx.update(|cx| crate::assets::load_fonts(cx));
    let (_, doc) = cx.add_window_view(|_, cx| {
        crate::doc_window::DocWindow::new(crate::doc_window::Doc::Changelog, cx)
    });
    doc.update(|window, _| window.activate_window());
    doc.run_until_parked();
    draw(doc);
    // Linux: the menu bar, and the desktop's own caption buttons.
    assert!(doc.debug_bounds("caption-close").is_none());
    click(doc, "menu-title-help");
    click(doc, "menu-row-help-changelog");
    doc.run_until_parked();
    let docs = cx.update(|cx| {
        cx.windows()
            .iter()
            .filter(|window| window.downcast::<crate::doc_window::DocWindow>().is_some())
            .count()
    });
    assert_eq!(docs, 1);
}

/// On Windows GPUI taps Alt itself to bring a window forward: that Alt is
/// not the user's and leaves the bar at rest. A real one later still works.
#[gpui_kit::test]
fn the_alt_that_brings_a_window_forward_is_not_the_users(cx: &mut TestAppContext) {
    let (view, cx) = open_with_strip(cx);
    cx.update(crate::menu_bar::bring_forward);
    keys(cx, "alt");
    assert_eq!(bar_state(&view, cx), crate::menu_bar::State::default());
    cx.executor()
        .advance_clock(crate::menu_bar::FAKE_ALT_WINDOW);
    keys(cx, "alt");
    assert_eq!(bar_state(&view, cx).title, Some(0));
}

/// Alt held for a click is not a lone Alt: releasing it leaves the bar at
/// rest. A tap of Alt after a key still works.
#[gpui_kit::test]
fn an_alt_click_is_not_a_lone_alt(cx: &mut TestAppContext) {
    let (view, cx) = open_with_strip(cx);
    let pane = cx.debug_bounds(ROOT_ID).expect("the window");
    cx.simulate_click(
        point(pane.center().x, pane.bottom() - px(40.0)),
        Modifiers::alt(),
    );
    keys(cx, "alt");
    assert_eq!(bar_state(&view, cx), crate::menu_bar::State::default());
    keys(cx, "alt");
    assert_eq!(bar_state(&view, cx).title, Some(0));
}

/// Links the router opened, newest first, each kept with its full URL and
/// opened in Microsoft Edge.
fn opened_links(urls: &[&str]) -> Vec<DispatchRecord> {
    let template = fixtures::activity()
        .into_iter()
        .find(|record| record.browser.is_some())
        .expect("a fixture record that opened");
    urls.iter()
        .enumerate()
        .map(|(at, url)| {
            let mut record = template.clone();
            record.id = DispatchId(100 - at as u64);
            record.privacy = Privacy::Full;
            record.link = Link::new(*url, Source::AppleEvent);
            record.steps = vec![Step::Received {
                url: (*url).to_owned(),
            }];
            record.outcome = Outcome::Opened;
            record.final_url = Some((*url).to_owned());
            if let Some(browser) = &mut record.browser {
                "Microsoft Edge".clone_into(&mut browser.name);
                browser.profile.clear();
            }
            record
        })
        .collect()
}

#[gpui_kit::test]
fn a_new_rule_starts_from_the_last_link(cx: &mut TestAppContext) {
    let dir = scratch("start-from");
    let path = dir.join("rules.json");
    cx.update(|cx| {
        activity::install_feed(
            Feed::from_records(opened_links(&[
                "https://git.example.com/acme/widgets/pull/128",
                "https://docs.example.com/guides/setup",
            ])),
            cx,
        )
    });
    let (view, cx) = open_store(cx, None, fixture_store(&dir));
    click(cx, "add-handler");
    let handler = |cx: &mut VisualTestContext| view.read_with(cx, |app, _| app.handlers[7].clone());
    let sample = |cx: &mut VisualTestContext| {
        view.read_with(cx, |app, _| app.rail.as_ref().unwrap().sample.text.clone())
    };
    // The draft starts from the last link: its narrowest pattern, selected,
    // and the link in Try it.
    assert_eq!(handler(cx).rule.badge, RowBadge::Draft);
    assert_eq!(handler(cx).patterns, ["git.example.com/acme/*"]);
    assert!(view.read_with(cx, |app, _| app.rail.as_ref().unwrap().patterns[0].selected));
    assert_eq!(sample(cx), "https://git.example.com/acme/widgets/pull/128");
    assert!(cx.debug_bounds("start-from").is_some());
    assert!(cx.debug_bounds("start-suggestions").is_some());

    // The wider suggestion.
    click(cx, "start-suggestion-1");
    assert_eq!(handler(cx).patterns, ["git.example.com/*"]);

    // Another recent link replaces the pattern and Try it.
    click(cx, "start-box");
    assert!(cx.debug_bounds("start-menu").is_some());
    click(cx, "start-link-1");
    assert!(cx.debug_bounds("start-menu").is_none());
    assert_eq!(handler(cx).patterns, ["docs.example.com/guides/*"]);
    assert_eq!(sample(cx), "https://docs.example.com/guides/setup");

    // Typing replaces the selected suggestion.
    cx.simulate_input("notion.so/*");
    draw(cx);
    assert_eq!(handler(cx).patterns, ["notion.so/*"]);

    // Clear goes back to a blank rule; the picker stays.
    click(cx, "start-clear");
    assert_eq!(handler(cx).patterns, [""]);
    assert_eq!(handler(cx).title, "New rule");
    assert_ne!(sample(cx), "https://docs.example.com/guides/setup");
    assert!(cx.debug_bounds("start-suggestions").is_none());
    assert!(cx.debug_bounds("start-box").is_some());
    assert_eq!(
        view.read_with(cx, |app, _| app.draft_link.clone()),
        Some((7, None))
    );
    // No link reached the file: the draft has no browser.
    let file = saved(cx, &path);
    assert_eq!(file.rules.len(), 7);
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn with_no_links_a_new_rule_starts_blank(cx: &mut TestAppContext) {
    let dir = scratch("start-blank");
    cx.update(|cx| activity::install_feed(Feed::from_records(Vec::new()), cx));
    let (view, cx) = open_store(cx, None, fixture_store(&dir));
    click(cx, "add-handler");
    let handler = view.read_with(cx, |app, _| app.handlers[7].clone());
    assert_eq!(handler.patterns, [""]);
    assert!(cx.debug_bounds("start-from").is_some());
    assert!(cx.debug_bounds("start-suggestions").is_none());
    assert!(cx.debug_bounds("start-clear").is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn the_first_rule_prompt_names_the_last_link(cx: &mut TestAppContext) {
    let dir = scratch("first-rule-link");
    let path = dir.join("rules.json");
    cx.update(|cx| {
        activity::install_feed(
            Feed::from_records(opened_links(&[
                "https://git.example.com/acme/widgets/pull/128",
            ])),
            cx,
        )
    });
    let (view, cx) = open_store(cx, None, RulesStore::load(Some(path)));
    view.update_in(cx, |app, window, cx| {
        app.navigate(Screen::Overview, window, cx)
    });
    draw(cx);
    assert!(cx.debug_bounds("overview-last-link").is_some());
    click(cx, "overview-add-rule");
    assert_eq!(view.read_with(cx, |app, _| app.screen), Screen::Rules);
    let patterns = view.read_with(cx, |app, _| app.handlers[0].patterns.clone());
    assert_eq!(patterns, ["git.example.com/acme/*"]);

    // Blank rule keeps the old path.
    view.update_in(cx, |app, window, cx| {
        app.delete_selected(window, cx);
        app.navigate(Screen::Overview, window, cx);
    });
    draw(cx);
    click(cx, "overview-blank-rule");
    let patterns = view.read_with(cx, |app, _| app.handlers[0].patterns.clone());
    assert_eq!(patterns, [""]);
    let _ = std::fs::remove_dir_all(dir);
}

#[gpui_kit::test]
fn a_rewritten_link_starts_a_rule_for_where_it_went(cx: &mut TestAppContext) {
    let dir = scratch("start-rewritten");
    let mut records = opened_links(&["https://x.com/pwrdrvr/status/1842"]);
    records[0].final_url = Some("https://xcancel.com/pwrdrvr/status/1842".into());
    cx.update(|cx| activity::install_feed(Feed::from_records(records), cx));
    let (view, cx) = open_store(cx, None, fixture_store(&dir));
    click(cx, "add-handler");
    // The fixture rewrites x.com to xcancel.com: the handlers see the
    // latter, so the pattern is for it, while Try it runs the link as
    // opened, through the rewrite.
    let (patterns, sample) = view.read_with(cx, |app, _| {
        (
            app.handlers[7].patterns.clone(),
            app.rail.as_ref().unwrap().sample.text.clone(),
        )
    });
    assert_eq!(patterns, ["xcancel.com/pwrdrvr/*"]);
    assert_eq!(sample, "https://x.com/pwrdrvr/status/1842");

    // A handler above the draft goes: the draft's link moves up with it,
    // and no other rule shows its suggestions.
    view.update_in(cx, |app, window, cx| {
        app.select(Selection::Handler(0), cx);
        app.delete_selected(window, cx);
    });
    draw(cx);
    assert_eq!(
        view.read_with(cx, |app, _| app.draft_link.as_ref().map(|(at, _)| *at)),
        Some(6)
    );
    view.update_in(cx, |app, _, cx| app.select(Selection::Handler(5), cx));
    draw(cx);
    assert!(cx.debug_bounds("start-suggestions").is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// The rail's build block (design: Build Identity): each row copies its
/// full value, the header's button the whole report, and a copy's check
/// goes after `COPIED_FOR`.
#[gpui_kit::test]
fn the_rail_build_block_copies_each_part_and_the_whole(cx: &mut TestAppContext) {
    use crate::build_info::{BuildInfo, Part};
    use crate::chrome::{BUILD_ALL_ID, BUILD_BRANCH_ID, BUILD_CHECKOUT_ID, BUILD_COMMIT_ID};
    let (view, cx) = open(cx);
    let build = BuildInfo::fixture();
    view.update(cx, |app, cx| {
        app.build = build.clone();
        cx.notify();
    });
    draw(cx);
    let copied = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
    };
    click(cx, BUILD_CHECKOUT_ID);
    assert_eq!(copied(cx), build.checkout.clone());
    click(cx, BUILD_BRANCH_ID);
    assert_eq!(copied(cx).as_deref(), Some("feat/build-identity"));
    click(cx, BUILD_COMMIT_ID);
    assert_eq!(
        copied(cx).as_deref(),
        Some("5e1c07a2f19d4b6c08e3a7d5b2c91f40e6a8d317")
    );
    assert_eq!(
        view.read_with(cx, |app, _| app.build_copied.map(|(part, _)| part)),
        Some(Part::Commit)
    );
    click(cx, BUILD_ALL_ID);
    assert_eq!(copied(cx), Some(build.report()));
    cx.executor().advance_clock(crate::app::COPIED_FOR);
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |app, _| app.build_copied), None);
}

/// A detached HEAD's branch row says so and copies nothing; a release is
/// one line that copies the report.
#[gpui_kit::test]
fn a_detached_build_and_a_release_in_the_rail(cx: &mut TestAppContext) {
    use crate::build_info::BuildInfo;
    use crate::chrome::{BUILD_ALL_ID, BUILD_BRANCH_ID, BUILD_CHECKOUT_ID, BUILD_COMMIT_ID};
    let (view, cx) = open(cx);
    view.update(cx, |app, cx| {
        app.build = BuildInfo {
            branch: None,
            dirty: true,
            built_at: Some(1_793_197_920),
            ..BuildInfo::fixture()
        };
        cx.notify();
    });
    draw(cx);
    cx.update(|_, cx| cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("before".into())));
    click(cx, BUILD_BRANCH_ID);
    assert_eq!(
        cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
            .as_deref(),
        Some("before")
    );
    assert_eq!(view.read_with(cx, |app, _| app.build_copied), None);

    let release = BuildInfo {
        profile: "release",
        tag: Some(format!("v{}", BuildInfo::fixture().version)),
        checkout: None,
        branch: None,
        ..BuildInfo::fixture()
    };
    view.update(cx, |app, cx| {
        app.build = release.clone();
        cx.notify();
    });
    draw(cx);
    for row in [BUILD_CHECKOUT_ID, BUILD_BRANCH_ID, BUILD_COMMIT_ID] {
        assert!(cx.debug_bounds(row).is_none(), "{row} shows for a release");
    }
    click(cx, BUILD_ALL_ID);
    assert_eq!(
        cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text())),
        Some(release.report())
    );
}
