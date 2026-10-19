//! Smoke tests through gpui-kit's window harness: open the real view, draw
//! frames, click, type and press keys. Test a link runs the real engine over
//! the fixture rules (and the fixture Finicky config).

use std::path::PathBuf;

use gpui_kit::{Bounds, Entity, Modifiers, Pixels, TestAppContext, VisualTestContext, px};

use crate::activity::Filter;
use crate::app::{PwrRoute, ROOT_ID, Screen, Selection};
use crate::fixtures::{Destination, Matcher};
use crate::trace_view::TraceView;

/// The contrived home directory that holds `.finicky.ts` for option 1b.
fn fixture_home() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/home")
}

fn open_with(
    cx: &mut TestAppContext,
    config: Option<PathBuf>,
) -> (Entity<PwrRoute>, &mut VisualTestContext) {
    cx.update(|cx| crate::assets::load_fonts(cx));
    let (view, cx) =
        cx.add_window_view(|_, cx| PwrRoute::with_config(config, Some(fixture_home()), cx));
    let focus = view.read_with(cx, |app, _| app.focus.clone());
    cx.update(|window, cx| window.focus(&focus, cx));
    draw(cx);
    (view, cx)
}

fn open(cx: &mut TestAppContext) -> (Entity<PwrRoute>, &mut VisualTestContext) {
    open_with(cx, None)
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
    assert_eq!(selection(cx), Selection::Handler(2));

    cx.simulate_keystrokes("down down");
    draw(cx);
    assert_eq!(selection(cx), Selection::Handler(4));

    // Up past the first handler into the rewrites, clamped at the top.
    cx.simulate_keystrokes("up up up up up up up up up up");
    draw(cx);
    assert_eq!(selection(cx), Selection::Rewrite(0));
    assert!(cx.debug_bounds("editor-rail").is_some());

    // The rewrite editor's switch turns the rewrite off.
    click(cx, "handler-enabled");
    assert!(view.read_with(cx, |app, _| !app.rewrites[0].rule.enabled));
}

#[gpui_kit::test]
fn the_rail_switches_screens(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    let screen = |cx: &mut VisualTestContext| view.read_with(cx, |app, _| app.screen);

    click(cx, "nav-test");
    assert_eq!(screen(cx), Screen::TestLink);
    assert!(cx.debug_bounds("test-trace").is_some());
    assert!(cx.debug_bounds("editor-rail").is_none());

    click(cx, "nav-activity");
    assert_eq!(screen(cx), Screen::Activity);
    assert!(cx.debug_bounds("activity-row-0").is_some());

    click(cx, "nav-rules");
    assert_eq!(screen(cx), Screen::Rules);
    assert!(cx.debug_bounds("editor-rail").is_some());
}

#[gpui_kit::test]
fn test_a_link_traces_the_fixture_rules(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    click(cx, "nav-test");

    // The design's example: two rewrites apply, handler 3 wins on Slack.
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
    let picking = |cx: &mut VisualTestContext| view.read_with(cx, |app, _| app.test.picking);
    // The picker opens its menu and, clicked again, closes it.
    click(cx, "test-opener");
    assert!(picking(cx));
    click(cx, "test-opener");
    assert!(!picking(cx));
    click(cx, "test-opener");
    assert!(picking(cx));
    // Not from Slack: handler 3 no longer matches and the default opens it.
    click(cx, "test-opener-choice-0");
    let none = trace(&view, cx);
    assert_eq!(none.result.title, "Google Chrome \u{00b7} Personal");
    assert!(titles(&none).contains(&"Everything else"));
    assert!(!view.read_with(cx, |app, _| app.test.picking));
}

#[gpui_kit::test]
fn activity_expands_filters_and_retests(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    click(cx, "nav-activity");
    assert_eq!(view.read_with(cx, |app, _| app.activity.expanded), Some(1));

    click(cx, "activity-row-0");
    assert_eq!(view.read_with(cx, |app, _| app.activity.expanded), Some(0));

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
    // No editor until the visual rules are expanded.
    assert!(cx.debug_bounds("editor-rail").is_none());
    click(cx, "visual-rules-toggle");
    assert!(cx.debug_bounds("editor-rail").is_some());

    // Test a link runs the config first: its Slack function handler wins.
    click(cx, "nav-test");
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
    assert!(
        config.status.starts_with("It did not load:"),
        "{}",
        config.status
    );
    assert!(config.handlers.is_empty() && config.default_browser.is_none());
}
