//! The dropdown through gpui-kit's window harness: its card is as tall as
//! `card_height` says, the window is asked to be exactly that tall (no
//! empty band below Quit) and follows the card as it changes, a display too
//! short for the card scrolls the middle, and the pause row pauses the
//! router.

use std::cell::Cell;
use std::rc::Rc;
use std::time::SystemTime;

use gpui_kit::{
    AppContext as _, Entity, Modifiers, Pixels, TestAppContext, VisualTestContext, px, size,
};
use pwrroute_router::{
    Destination, DryRunLauncher, Options, PauseTarget, Router, RouterHandle, SendScope,
};

use super::dropdown::{
    BODY_ID, CARD_ID, DESTINATION_ID, DESTINATIONS_ID, Dropdown, FOR_HOUR_ID, MAKE_DEFAULT_ID,
    PAUSE_ID, QUIT_ID, SEND_ALL_ID, SEND_NEXT_ID, SEND_RULES_ID, STEP_UP_ID, WIDTH, card_height,
};
use super::model::{BrowserLabel, Recent, RecentRoute, Routing, TrayModel};
use super::{ConfigDefault, Tray};

/// A display with room for any card.
const TALL: f32 = 2_000.0;

struct Opened<'a> {
    model: Entity<TrayModel>,
    view: Entity<Dropdown>,
    /// The height the dropdown last asked the OS for.
    asked: Rc<Cell<Option<Pixels>>>,
    cx: &'a mut VisualTestContext,
}

fn open(model: TrayModel, cx: &mut TestAppContext) -> Opened<'_> {
    open_on(model, px(TALL), cx)
}

/// Open the dropdown on a display with `max_height` under the menu bar,
/// and let it size its window as `dropdown::open` does.
fn open_on(model: TrayModel, max_height: Pixels, cx: &mut TestAppContext) -> Opened<'_> {
    cx.update(|cx| crate::assets::load_fonts(cx));
    let model = cx.new(|_| model);
    let shared = model.clone();
    let asked = Rc::new(Cell::new(None));
    let record = asked.clone();
    let (view, cx) = cx.add_window_view(move |window, cx| {
        Dropdown::new(
            shared,
            max_height,
            Rc::new(move |height| record.set(Some(height))),
            window,
            cx,
        )
    });
    view.update_in(cx, |this, window, cx| this.fit(window, cx));
    let mut opened = Opened {
        model,
        view,
        asked,
        cx,
    };
    opened.settle();
    opened
}

impl Opened<'_> {
    /// Apply the resize the dropdown asked for, as AppKit would, and draw.
    fn settle(&mut self) {
        draw(self.cx);
        self.cx.run_until_parked();
        if let Some(height) = self.asked.take() {
            self.cx.simulate_resize(size(px(WIDTH), height));
            draw(self.cx);
        }
    }

    fn update_model(&mut self, change: impl FnOnce(&mut TrayModel)) {
        self.model.update(self.cx, |model, cx| {
            change(model);
            cx.notify();
        });
        self.settle();
    }

    fn window_height(&mut self) -> Pixels {
        self.cx.update(|window, _| window.viewport_size().height)
    }
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

/// The laid-out card and the computed one agree to a pixel, the window is
/// the card's height (nothing below Quit), and Quit ends at the card's
/// bottom.
fn assert_card_fits(opened: &mut Opened<'_>) {
    let model = opened.model.clone();
    let choosing = opened.view.read_with(opened.cx, |view, _| view.choosing);
    let card = opened.cx.debug_bounds(CARD_ID).expect("the card");
    let computed = opened.cx.update(|_, cx| {
        let text_system = cx.text_system().clone();
        card_height(model.read(cx), choosing, &text_system)
    });
    let laid_out = card.size.height;
    assert!(
        (laid_out - computed).abs() <= px(1.0),
        "laid out {laid_out:?}, computed {computed:?}"
    );
    let window = opened.window_height();
    let asked = opened.view.read_with(opened.cx, |view, _| view.height);
    assert_eq!(asked, window, "the window is what the dropdown asked for");
    assert!(
        (window - laid_out).abs() <= px(1.0),
        "window {window:?} is the card {laid_out:?}, with no band below it"
    );
    let quit = opened.cx.debug_bounds(QUIT_ID).expect("Quit");
    assert!(quit.bottom() <= card.bottom() && card.bottom() - quit.bottom() <= px(8.0));
}

#[gpui_kit::test]
fn the_routing_dropdown_is_as_tall_as_computed(cx: &mut TestAppContext) {
    let mut opened = open(TrayModel::fixture(SystemTime::now()), cx);
    assert_card_fits(&mut opened);
}

#[gpui_kit::test]
fn the_paused_dropdown_is_as_tall_as_computed(cx: &mut TestAppContext) {
    let mut opened = open(TrayModel::paused_fixture(SystemTime::now()), cx);
    assert_card_fits(&mut opened);
}

#[gpui_kit::test]
fn an_empty_dropdown_is_as_tall_as_computed(cx: &mut TestAppContext) {
    let mut empty = TrayModel::fixture(SystemTime::now());
    empty.recents.clear();
    empty.is_default_browser = false;
    let mut opened = open(empty, cx);
    assert_card_fits(&mut opened);
}

/// The registrar's re-read says another browser took the default while the
/// dropdown is open: the chip and the Make default row follow the model,
/// and the window grows and shrinks with the card.
#[gpui_kit::test]
fn losing_the_default_shows_the_make_default_row(cx: &mut TestAppContext) {
    let mut opened = open(TrayModel::fixture(SystemTime::now()), cx);
    assert!(opened.cx.debug_bounds(MAKE_DEFAULT_ID).is_none());
    assert_card_fits(&mut opened);
    let before = opened.window_height();

    opened.update_model(|model| model.is_default_browser = false);
    assert!(opened.cx.debug_bounds(MAKE_DEFAULT_ID).is_some(), "the row");
    assert_card_fits(&mut opened);
    assert!(
        opened.window_height() > before,
        "the window grew with the row"
    );

    opened.update_model(|model| model.is_default_browser = true);
    assert!(opened.cx.debug_bounds(MAKE_DEFAULT_ID).is_none());
    assert_card_fits(&mut opened);
    assert_eq!(opened.window_height(), before, "and shrank back");
}

/// Recent links arriving while the dropdown is open grow the window with
/// the card, up to the recents' limit.
#[gpui_kit::test]
fn new_recent_links_grow_the_window(cx: &mut TestAppContext) {
    let mut empty = TrayModel::fixture(SystemTime::now());
    let recents = std::mem::take(&mut empty.recents);
    let mut opened = open(empty, cx);
    assert_card_fits(&mut opened);
    let mut last = opened.window_height();
    for recent in recents.into_iter().rev() {
        opened.update_model(|model| model.recents.insert(0, recent));
        assert_card_fits(&mut opened);
        let now = opened.window_height();
        assert!(now >= last, "{now:?} after {last:?}");
        last = now;
    }
}

/// A display too short for the card: the window stops at its visible
/// area, the header and Quit stay inside it, and the sections between
/// them scroll.
#[gpui_kit::test]
fn a_short_display_scrolls_between_the_header_and_quit(cx: &mut TestAppContext) {
    let mut opened = open_on(TrayModel::fixture(SystemTime::now()), px(220.0), cx);
    assert_eq!(opened.window_height(), px(220.0));
    let card = opened.cx.debug_bounds(CARD_ID).expect("the card");
    assert!((card.size.height - px(220.0)).abs() <= px(1.0), "{card:?}");
    let quit = opened.cx.debug_bounds(QUIT_ID).expect("Quit");
    assert!(quit.bottom() <= card.bottom(), "Quit {quit:?} in {card:?}");
    let body = opened.cx.debug_bounds(BODY_ID).expect("the middle");
    assert!(body.bottom() <= quit.top(), "{body:?} above {quit:?}");
    let full = opened.cx.update(|_, cx| {
        let text_system = cx.text_system().clone();
        card_height(opened.model.read(cx), false, &text_system)
    });
    assert!(full > px(220.0), "the card is taller than the display");
}

/// A long profile name in a recent's detail line ends in an ellipsis
/// inside the card instead of running under the age column.
#[gpui_kit::test]
fn a_long_recent_detail_stays_inside_the_card(cx: &mut TestAppContext) {
    let mut model = TrayModel::fixture(SystemTime::now());
    model.recents[0] = Recent::from_urls(
        "https://docs.example.com/a",
        None,
        BrowserLabel::new(
            "Ed",
            "Microsoft Edge",
            Some("Research and Development Team Profile With A Long Name"),
        ),
        RecentRoute::Handler(12),
        SystemTime::now(),
    )
    .unwrap();
    let mut opened = open(model, cx);
    assert_card_fits(&mut opened);
    let card = opened.cx.debug_bounds(CARD_ID).expect("the card");
    let detail = opened
        .cx
        .debug_bounds("tray-recent-detail-0")
        .expect("the detail line");
    let age = opened
        .cx
        .debug_bounds("tray-recent-age-0")
        .expect("the age");
    assert!(
        detail.right() <= age.left(),
        "{detail:?} runs under {age:?}"
    );
    assert!(age.right() <= card.right(), "{age:?} in {card:?}");
}

#[gpui_kit::test]
fn the_pause_row_pauses_the_router_and_the_card_still_fits(cx: &mut TestAppContext) {
    // A real router with no config, as `install` would hold it; the
    // fixture model, so the target is the design's and not this machine's.
    let router = Router::spawn(Options::default(), None, DryRunLauncher::new()).unwrap();
    let handle = router.handle();
    let mut opened = open(TrayModel::fixture(SystemTime::now()), cx);
    let model = opened.model.clone();
    opened.cx.update(|_, cx| {
        cx.set_global(Tray {
            model: model.clone(),
            live: false,
            router: Some(handle.clone()),
            config_default: ConfigDefault::default(),
            native: None,
            dropdown: None,
            dropdown_closed_at: None,
        });
    });
    assert_eq!(handle.paused(), None);
    let pause = opened.cx.debug_bounds(PAUSE_ID).expect("the pause row");

    opened.cx.simulate_click(pause.center(), Modifiers::none());
    opened.settle();
    assert!(model.read_with(opened.cx, |model, _| model.is_paused()));
    // The router is paused, to the browser the row names.
    assert_eq!(
        handle.paused(),
        Some(PauseTarget {
            browser: "Google Chrome".into(),
            profile: "Personal".into(),
        })
    );
    // Paused drops the activity row: the window shrinks with the card.
    assert_card_fits(&mut opened);

    let pause = opened.cx.debug_bounds(PAUSE_ID).expect("the pause row");
    opened.cx.simulate_click(pause.center(), Modifiers::none());
    opened.settle();
    assert!(!model.read_with(opened.cx, |model, _| model.is_paused()));
    assert_eq!(handle.paused(), None);
    assert_card_fits(&mut opened);
}

/// The tray as `install` holds it, over `opened`'s model and a router with
/// no config.
fn with_router(opened: &mut Opened<'_>) -> (Router, RouterHandle) {
    let router = Router::spawn(Options::default(), None, DryRunLauncher::new()).unwrap();
    let handle = router.handle();
    let model = opened.model.clone();
    let tray_router = handle.clone();
    opened.cx.update(|_, cx| {
        cx.set_global(Tray {
            model,
            live: false,
            router: Some(tray_router),
            config_default: ConfigDefault::default(),
            native: None,
            dropdown: None,
            dropdown_closed_at: None,
        });
    });
    (router, handle)
}

fn click(opened: &mut Opened<'_>, id: &'static str) {
    let at = opened
        .cx
        .debug_bounds(id)
        .unwrap_or_else(|| panic!("{id} is drawn"));
    opened.cx.simulate_click(at.center(), Modifiers::none());
    opened.settle();
}

fn edge_work() -> Destination {
    Destination {
        browser: "Microsoft Edge".into(),
        profile: "Work".into(),
        private: false,
    }
}

/// Send links (design: Send Links 1c, 1d): Next link sends the next link
/// to the destination the row names, the stepper adds more, and Rules
/// ends it; the card fits at every step.
#[gpui_kit::test]
fn next_link_sends_to_the_destination_and_rules_ends_it(cx: &mut TestAppContext) {
    let mut opened = open(TrayModel::fixture(SystemTime::now()), cx);
    let (_router, handle) = with_router(&mut opened);
    assert_card_fits(&mut opened);

    click(&mut opened, SEND_NEXT_ID);
    let send = handle.send_to().expect("sending");
    assert_eq!(send.destination, edge_work());
    assert_eq!(send.scope, SendScope::Next { left: 1, of: 1 });
    let model = opened.model.clone();
    assert_eq!(
        model.read_with(opened.cx, |model, _| model.chip().label()),
        "next link"
    );
    assert!(opened.cx.debug_bounds(STEP_UP_ID).is_some(), "the stepper");
    assert_card_fits(&mut opened);

    click(&mut opened, STEP_UP_ID);
    click(&mut opened, STEP_UP_ID);
    assert_eq!(
        handle.send_to().map(|send| send.scope),
        Some(SendScope::Next { left: 3, of: 3 })
    );
    assert_card_fits(&mut opened);

    click(&mut opened, SEND_RULES_ID);
    assert_eq!(handle.send_to(), None);
    assert!(opened.cx.debug_bounds(STEP_UP_ID).is_none());
    assert_card_fits(&mut opened);
}

/// All links, then for an hour; Pause ends the send and Rules resumes.
#[gpui_kit::test]
fn all_links_and_pause_share_the_slot(cx: &mut TestAppContext) {
    let mut opened = open(TrayModel::fixture(SystemTime::now()), cx);
    let (_router, handle) = with_router(&mut opened);

    click(&mut opened, SEND_ALL_ID);
    assert_eq!(
        handle.send_to().map(|send| send.scope),
        Some(SendScope::All { until: None })
    );
    assert_card_fits(&mut opened);
    click(&mut opened, FOR_HOUR_ID);
    assert!(matches!(
        handle.send_to().map(|send| send.scope),
        Some(SendScope::All { until: Some(_) })
    ));
    assert_card_fits(&mut opened);

    click(&mut opened, PAUSE_ID);
    assert_eq!(handle.send_to(), None, "pausing ends the send");
    assert!(handle.paused().is_some());
    let model = opened.model.clone();
    assert_eq!(
        model.read_with(opened.cx, |model, _| model.send_mode()),
        None
    );
    assert_card_fits(&mut opened);

    click(&mut opened, SEND_RULES_ID);
    assert_eq!(handle.paused(), None, "Rules resumes");
    assert_eq!(
        model.read_with(opened.cx, |model, _| model.routing),
        Routing::Active
    );
}

/// The destination row opens the list in place; a row picks it, and a
/// send in force goes there from the next link (1b).
#[gpui_kit::test]
fn choosing_a_destination_retargets_the_send(cx: &mut TestAppContext) {
    let mut opened = open(TrayModel::fixture(SystemTime::now()), cx);
    let (_router, handle) = with_router(&mut opened);
    click(&mut opened, SEND_NEXT_ID);

    assert!(opened.cx.debug_bounds(DESTINATIONS_ID).is_none());
    click(&mut opened, DESTINATION_ID);
    assert!(
        opened.cx.debug_bounds(DESTINATIONS_ID).is_some(),
        "the list"
    );
    assert_card_fits(&mut opened);

    // Edge's InPrivate window: the fourth destination.
    click(&mut opened, "tray-destination-3");
    assert!(opened.cx.debug_bounds(DESTINATIONS_ID).is_none(), "closed");
    assert_eq!(
        handle.send_to().map(|send| send.destination),
        Some(Destination {
            browser: "Microsoft Edge".into(),
            profile: String::new(),
            private: true,
        })
    );
    assert_eq!(
        handle.send_to().map(|send| send.scope),
        Some(SendScope::Next { left: 1, of: 1 }),
        "the count stands"
    );
    assert_card_fits(&mut opened);
}

/// A long destination list stops at 8½ rows and the card still fits.
#[gpui_kit::test]
fn a_long_destination_list_scrolls(cx: &mut TestAppContext) {
    let mut model = TrayModel::fixture(SystemTime::now());
    let group = model.destinations[0].clone();
    for _ in 0..4 {
        model.destinations.push(group.clone());
    }
    let mut opened = open(model, cx);
    click(&mut opened, DESTINATION_ID);
    let list = opened.cx.debug_bounds(DESTINATIONS_ID).expect("the list");
    assert!(list.size.height <= px(26.0 * 8.5 + 9.0 + 0.5), "{list:?}");
    assert_card_fits(&mut opened);
}

/// The router counts Next link down; after each dispatch the tray reads
/// it back (`sync_send`), so the stepper and the chip follow and the
/// switch returns to Rules when the count runs out.
#[gpui_kit::test]
fn the_tray_follows_the_routers_countdown(cx: &mut TestAppContext) {
    use pwrroute_router::{Link, Source};

    let mut opened = open(TrayModel::fixture(SystemTime::now()), cx);
    let (_router, handle) = with_router(&mut opened);
    opened
        .cx
        .update(|_, cx| cx.global_mut::<Tray>().live = true);
    click(&mut opened, SEND_NEXT_ID);
    click(&mut opened, STEP_UP_ID);
    let model = opened.model.clone();
    let dispatch_and_sync = |opened: &mut Opened<'_>| {
        handle
            .dispatch_and_wait(Link::new("https://docs.example.com/", Source::CommandLine))
            .unwrap();
        let model = opened.model.clone();
        opened.cx.update(|_, cx| super::sync_send(&model, cx));
        opened.settle();
    };
    dispatch_and_sync(&mut opened);
    assert_eq!(
        model.read_with(opened.cx, |model, _| model.routing),
        Routing::Sending(SendScope::Next { left: 1, of: 2 })
    );
    assert_card_fits(&mut opened);
    dispatch_and_sync(&mut opened);
    assert_eq!(
        model.read_with(opened.cx, |model, _| model.routing),
        Routing::Active
    );
    assert_eq!(
        model.read_with(opened.cx, |model, _| model.chip().label()),
        "default"
    );
    assert_card_fits(&mut opened);
}
