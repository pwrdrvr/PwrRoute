//! The app as a user meets it: it launches, draws its window, takes input,
//! hides and comes back, and quits.

use std::{
    thread,
    time::{Duration, Instant},
};

use pwrroute_e2e::{App, harness, os, quit_keystroke, rect};
use pwrroute_router::instance::{self, Claim, Endpoint};
use serde_json::Value;

/// The family spec: the brand starts 96px in, after the traffic lights, in a
/// strip 40px tall plus its 1px border (`chrome.rs`).
const BRAND_X: f64 = 96.0;
const STRIP_HEIGHT: f64 = 41.0;
/// The rail's width (`chrome.rs`).
const RAIL_WIDTH: f64 = 196.0;

fn region(state: &Value, name: &str) -> (f64, f64, f64, f64) {
    rect(&state["main"]["regions"][name])
        .unwrap_or_else(|| panic!("region {name} was not painted: {state:#}"))
}

/// Every launch lands on Overview: shows Rules and selects its first
/// handler with the keyboard (Down selects the rewrite first, then steps),
/// and returns the state once the editor is drawn.
fn open_rules(app: &App) -> Value {
    app.click("rail.rules").unwrap();
    app.wait_for("the Rules list", pwrroute_e2e::WAIT, |s| {
        (s["main"]["section"] == "rules").then_some(())
    })
    .unwrap();
    app.keys("down down").unwrap();
    app.wait_for("the first handler selected", pwrroute_e2e::WAIT, |s| {
        (s["main"]["rules"]["selection"] == serde_json::json!({ "kind": "handler", "index": 0 })
            && s["main"]["regions"]["editor"].is_object())
        .then(|| s.clone())
    })
    .unwrap()
}

fn main_window(state: &Value) -> &Value {
    state["windows"]
        .as_array()
        .and_then(|w| w.iter().find(|w| w["main"] == true))
        .unwrap_or_else(|| panic!("no main window: {state:#}"))
}

#[test]
fn launches_and_draws_the_title_strip() {
    let harness = harness!("launches_and_draws_the_title_strip");
    let app = harness.launch().unwrap();
    let state = app.wait_for_main_window().unwrap();
    app.screenshot("launched");

    let window = main_window(&state);
    assert!(
        window["title"].is_null() || window["title"] == "PwrRoute",
        "{window}"
    );
    let viewport_width = window["viewport"]["width"].as_f64().unwrap();
    assert!(
        viewport_width >= 1000.0,
        "below the minimum width: {window}"
    );

    // The strip spans the window's top edge at its fixed height.
    let (x, y, width, height) = region(&state, "titleStrip");
    assert_eq!((x, y), (0.0, 0.0));
    assert_eq!(width, viewport_width);
    assert_eq!(height, STRIP_HEIGHT);
    // The brand sits where the family spec puts it, inside the strip.
    let (brand_x, brand_y, brand_w, brand_h) = region(&state, "titleStrip.brand");
    assert_eq!(brand_x, BRAND_X);
    assert!(brand_w > 0.0 && brand_y >= 0.0 && brand_y + brand_h <= STRIP_HEIGHT);
    // The breadcrumb and the Overview's two status chips are laid out
    // after it, in order.
    let (crumb_x, ..) = region(&state, "titleStrip.breadcrumb");
    let (chip1_x, _, chip1_w, _) = region(&state, "titleStrip.chip1");
    let (chip2_x, _, chip2_w, _) = region(&state, "titleStrip.chip2");
    assert!(brand_x + brand_w < crumb_x && crumb_x < chip1_x);
    assert!(chip1_x + chip1_w <= chip2_x);
    assert!(chip2_x + chip2_w <= viewport_width);

    // Rail and the Overview fill the body under the strip: every launch
    // lands on Overview, with no editor.
    let (rail_x, rail_y, rail_w, _) = region(&state, "rail");
    assert_eq!((rail_x, rail_y, rail_w), (0.0, STRIP_HEIGHT, RAIL_WIDTH));
    let (pane_x, _, pane_w, _) = region(&state, "pane");
    assert_eq!(pane_x, RAIL_WIDTH);
    assert_eq!(pane_x + pane_w, viewport_width);
    assert!(state["main"]["regions"]["editor"].is_null(), "{state:#}");

    // The router came up in dry run with no Finicky config, so the window
    // shows the rules.json the harness seeded in the test's empty home
    // (`fixtures/rules.json`), editable, never the developer's.
    assert_eq!(state["router"]["dryRun"], true);
    assert_eq!(state["router"]["configError"], Value::Null);
    let rules = &state["main"]["rules"];
    assert_eq!(
        (rules["rewrites"].as_u64(), rules["handlers"].as_u64()),
        (Some(1), Some(5)),
        "{rules}"
    );
    assert_eq!(state["main"]["section"], "overview");
    assert_eq!(state["main"]["configInCharge"], false);
    assert_eq!(state["main"]["editing"], false);
    assert_eq!(rules["selection"], Value::Null, "nothing selected yet");

    // And the OS agrees: it lists a window titled PwrRoute of that size for
    // this process.
    assert_os_window(&app, window);
}

/// Polls the OS's own window list until it shows a window titled PwrRoute
/// for `app`'s process, the size the hook reports. Polled, because macOS
/// reports the frame mid-way through the window's opening animation. The
/// OS's rectangle may add a frame border (Windows), never lose size. macOS
/// withholds titles without Screen Recording; Linux and Windows always give
/// them.
fn assert_os_window(app: &App, window: &Value) {
    let width = window["bounds"]["width"].as_f64().unwrap();
    let height = window["bounds"]["height"].as_f64().unwrap();
    let fits = |os: f64, ours: f64| os >= ours - 1.0 && os <= ours + 32.0;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let windows = os::windows(app.pid()).unwrap();
        let found = windows.iter().any(|w| {
            fits(w.width, width)
                && fits(w.height, height)
                && match w.title.as_deref() {
                    Some(title) => title == "PwrRoute",
                    None => cfg!(target_os = "macos"),
                }
        });
        if found {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the OS lists no {width}x{height} window titled PwrRoute for pid {}: {windows:?}",
            app.pid()
        );
        thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn rail_items_are_laid_out_in_order() {
    let harness = harness!("rail_items_are_laid_out_in_order");
    let app = harness.launch().unwrap();
    let state = app.wait_for_main_window().unwrap();
    let items = [
        "rail.overview",
        "rail.rules",
        "rail.test",
        "rail.activity",
        "rail.openSettings",
    ];
    let mut last_bottom = STRIP_HEIGHT;
    for item in items {
        let (x, y, width, height) = region(&state, item);
        assert!(
            x >= 0.0 && x + width <= RAIL_WIDTH,
            "{item} leaves the rail"
        );
        assert!(y >= last_bottom && height > 0.0, "{item} is out of order");
        last_bottom = y + height;
    }
}

#[test]
fn clicking_the_enabled_switch_toggles_the_handler() {
    let harness = harness!("clicking_the_enabled_switch_toggles_the_handler");
    let app = harness.launch().unwrap();
    app.wait_for_main_window().unwrap();
    let state = open_rules(&app);
    let enabled = state["main"]["rules"]["selectionEnabled"]
        .as_bool()
        .expect("a selected handler");

    // A real mouse down and up at the switch: hit testing finds the
    // editor's `handler-enabled` element and its click handler runs.
    app.click("editor.enabled").unwrap();
    app.wait_for("the switch to flip", pwrroute_e2e::WAIT, |s| {
        (s["main"]["rules"]["selectionEnabled"].as_bool() == Some(!enabled)).then_some(())
    })
    .unwrap();
    app.click("editor.enabled").unwrap();
    app.wait_for("the switch to flip back", pwrroute_e2e::WAIT, |s| {
        (s["main"]["rules"]["selectionEnabled"].as_bool() == Some(enabled)).then_some(())
    })
    .unwrap();
}

/// A click on a rail item shows its section (CONTRACT.md, "Rail
/// navigation"). The window lands on Overview; Rules opens as a list, with
/// no editor until a rule is selected.
#[test]
fn rail_navigation_switches_sections() {
    let harness = harness!("rail_navigation_switches_sections");
    let app = harness.launch().unwrap();
    let state = app.wait_for_main_window().unwrap();
    assert_eq!(state["main"]["section"], "overview");
    for (item, section) in [
        ("rail.activity", "activity"),
        ("rail.test", "test"),
        ("rail.rules", "rules"),
        ("rail.overview", "overview"),
    ] {
        app.click(item).unwrap();
        // The section changes at once; the regions on the next frame.
        app.wait_for(
            &format!("section {section} drawn"),
            pwrroute_e2e::WAIT,
            |s| {
                (s["main"]["section"] == section
                    && s["main"]["regions"]["pane"].is_object()
                    && s["main"]["regions"]["editor"].is_null())
                .then_some(())
            },
        )
        .unwrap();
        app.screenshot(section);
    }
}

#[test]
fn arrow_keys_move_the_rule_selection() {
    let harness = harness!("arrow_keys_move_the_rule_selection");
    let app = harness.launch().unwrap();
    app.wait_for_main_window().unwrap();
    let state = open_rules(&app);
    let rules = &state["main"]["rules"];
    assert_eq!(rules["selection"]["kind"], "handler", "{rules}");
    let start = rules["selection"]["index"].as_u64().unwrap();
    assert!(
        rules["handlers"].as_u64().unwrap() > start + 2,
        "the fixture has room to move down: {rules}"
    );
    assert_eq!(
        state["main"]["focused"], true,
        "the rules list owns the keyboard"
    );
    let at = |index: u64| {
        move |s: &Value| {
            (s["main"]["rules"]["selection"]
                == serde_json::json!({ "kind": "handler", "index": index }))
            .then_some(())
        }
    };

    app.keys("down down").unwrap();
    app.wait_for(
        "the selection to move down",
        pwrroute_e2e::WAIT,
        at(start + 2),
    )
    .unwrap();
    app.keys("up").unwrap();
    app.wait_for(
        "the selection to move up",
        pwrroute_e2e::WAIT,
        at(start + 1),
    )
    .unwrap();
}

#[test]
fn closing_the_window_keeps_the_router_and_a_second_launch_reopens_it() {
    let harness = harness!("closing_the_window_keeps_the_router_and_a_second_launch_reopens_it");
    let app = harness.launch().unwrap();
    app.wait_for_main_window().unwrap();

    app.action("closeWindow").unwrap();
    app.wait_for("the window to close", pwrroute_e2e::WAIT, |s| {
        s["windows"]
            .as_array()
            .is_some_and(Vec::is_empty)
            .then_some(())
    })
    .unwrap();
    // The router outlives its windows and still answers.
    assert_eq!(app.state().unwrap()["router"]["dryRun"], true);

    // A launch with no links asks the running router for the window, and
    // exits without becoming a second router.
    let status = app.second_launch(&[]).unwrap();
    assert!(status.success(), "the second launch failed: {status}");
    let state = app
        .wait_for("the window to come back", pwrroute_e2e::WAIT, |s| {
            s["windows"]
                .as_array()
                .is_some_and(|w| w.iter().any(|w| w["main"] == true))
                .then(|| s.clone())
        })
        .unwrap();
    assert_os_window(&app, main_window(&state));
}

#[test]
fn quits_cleanly_from_the_keyboard() {
    let harness = harness!("quits_cleanly_from_the_keyboard");
    let mut app = harness.launch().unwrap();
    app.wait_for_main_window().unwrap();
    let router_dir = app.router_dir().unwrap().to_owned();

    app.keys(quit_keystroke()).unwrap();
    let status = app.wait_exit(Duration::from_secs(10)).unwrap();
    assert!(status.success(), "quit exited with {status}");
    assert_quit_released_the_router(&app, &router_dir);
}

#[test]
fn quits_cleanly_from_the_quit_action() {
    let harness = harness!("quits_cleanly_from_the_quit_action");
    let mut app = harness.launch().unwrap();
    app.wait_for_main_window().unwrap();
    let router_dir = app.router_dir().unwrap().to_owned();

    // What the menu bar's Quit and the tray dispatch.
    app.action("quit").unwrap();
    let status = app.wait_exit(Duration::from_secs(10)).unwrap();
    assert!(status.success(), "quit exited with {status}");
    assert_quit_released_the_router(&app, &router_dir);
}

/// After a quit the router lock is free: the next launch becomes the router
/// instead of forwarding to a ghost.
fn assert_quit_released_the_router(app: &App, router_dir: &std::path::Path) {
    match instance::claim(&Endpoint::new(router_dir)).unwrap() {
        Claim::Primary(_) => {}
        Claim::Running(_) => panic!("pid {} exited but its router lock is still held", app.pid()),
    }
}
