//! Smoke tests through gpui-kit's window harness: open the real view, draw
//! frames, click, type and press keys. The window reads and writes a copy
//! of the fixture `rules.json` in a scratch directory, and Test a link runs
//! the real engine over it (and the fixture Finicky config).

use std::path::{Path, PathBuf};

use gpui_kit::{Bounds, Entity, Modifiers, Pixels, TestAppContext, VisualTestContext, px};
use pwrroute_engine::RulesFile;
use pwrroute_router::{
    DispatchId, DispatchRecord, DryRunLauncher, Link, Options, Outcome, Router, Source,
};

use crate::activity::{self, EMPTY_ID, Feed, Filter};
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
        view.read_with(cx, |app, _| app.activity.rows[index].id)
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
                .map(|row| row.id)
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
        view.read_with(cx, |app, _| app.activity.expanded),
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
