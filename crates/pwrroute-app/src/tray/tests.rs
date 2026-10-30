//! The dropdown through gpui-kit's window harness: its card is as tall as
//! `card_height` says, the window is asked to be exactly that tall (no
//! empty band below Quit) and follows the card as it changes, a display too
//! short for the card scrolls the middle, and the Incoming links switch
//! drives the router's slot: send, hold and pause, by pointer and by
//! keyboard, and held links open from the card.

use std::cell::Cell;
use std::rc::Rc;
use std::time::SystemTime;

use gpui_kit::{
    AppContext as _, Entity, Modifiers, Pixels, TestAppContext, VisualTestContext, px, size,
};
use pwrroute_router::{
    Destination, DryRunLauncher, Link, Options, Override, PauseTarget, Retention, Router,
    RouterHandle, SendScope, Source,
};

use super::dropdown::{
    BODY_ID, CARD_ID, DESTINATION_ID, DESTINATIONS_ID, Dropdown, FOR_IDS, HELD_LANES_ID,
    LANES_WIDTH, LIST_MAX, LIVE_BUTTON_ID, MAKE_DEFAULT_ID, MODE_IDS, OPEN_ALL_ID, QUIT_ID,
    STEP_UP_ID, Unfolded, WIDTH, card_height, held_id, lanes_height,
};
use super::model::{
    BrowserLabel, Dispatched, ForKind, Mode, Recent, RecentRoute, Routing, TrayModel,
};
use super::{ConfigDefault, Tray};
use crate::activity::Feed;

const RULES: &str = MODE_IDS[0];
const SEND: &str = MODE_IDS[1];
const HOLD: &str = MODE_IDS[2];
const PAUSE: &str = MODE_IDS[3];

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
    let unfolded = opened.view.read_with(opened.cx, |view, _| view.unfolded);
    let card = opened.cx.debug_bounds(CARD_ID).expect("the card");
    let computed = opened.cx.update(|_, cx| {
        let text_system = cx.text_system().clone();
        card_height(model.read(cx), unfolded, &text_system)
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

/// Design 2c and 2d: held links lead the card, and a row's "···" opens
/// its actions and Open in…'s lanes in place; the card fits each time.
#[gpui_kit::test]
fn the_holding_dropdown_is_as_tall_as_computed(cx: &mut TestAppContext) {
    let mut opened = open(TrayModel::holding_fixture(SystemTime::now()), cx);
    assert!(opened.cx.debug_bounds(OPEN_ALL_ID).is_some(), "Open all");
    assert!(
        opened
            .cx
            .debug_bounds(Box::leak(format!("{QUIT_ID}-note").into_boxed_str()))
            .is_some(),
        "Quit says the held links will be lost"
    );
    assert_card_fits(&mut opened);
    let before = opened.window_height();
    click_named(&mut opened, &held_id("more", 0));
    assert!(
        opened
            .cx
            .debug_bounds(&*held_id("copy", 0).leak())
            .is_some()
    );
    assert_card_fits(&mut opened);
    assert!(opened.window_height() > before, "the row's actions grow it");
    click_named(&mut opened, &held_id("open-in", 0));
    assert!(opened.cx.debug_bounds(HELD_LANES_ID).is_some(), "the lanes");
    assert_card_fits(&mut opened);
    click_named(&mut opened, &held_id("more", 0));
    assert!(opened.cx.debug_bounds(HELD_LANES_ID).is_none(), "folded");
    assert_card_fits(&mut opened);
    assert_eq!(opened.window_height(), before);
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
        card_height(opened.model.read(cx), Unfolded::default(), &text_system)
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
            clock: None,
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

fn click_named(opened: &mut Opened<'_>, id: &str) {
    click(opened, Box::leak(id.to_owned().into_boxed_str()));
}

fn edge_work() -> Destination {
    Destination {
        browser: "Microsoft Edge".into(),
        profile: "Work".into(),
        private: false,
    }
}

/// Pause from the switch pauses the router to the browser the line names;
/// the live line's Resume ends it; the card fits each time.
#[gpui_kit::test]
fn pause_pauses_the_router_and_resume_ends_it(cx: &mut TestAppContext) {
    let mut opened = open(TrayModel::fixture(SystemTime::now()), cx);
    let (_router, handle) = with_router(&mut opened);
    let model = opened.model.clone();
    assert_eq!(handle.paused(), None);

    click(&mut opened, PAUSE);
    assert!(model.read_with(opened.cx, |model, _| model.is_paused()));
    assert_eq!(
        handle.paused(),
        Some(PauseTarget {
            browser: "Google Chrome".into(),
            profile: "Personal".into(),
        })
    );
    // Paused drops the activity row: the window shrinks with the card.
    assert_card_fits(&mut opened);

    click(&mut opened, LIVE_BUTTON_ID);
    assert!(!model.read_with(opened.cx, |model, _| model.is_paused()));
    assert_eq!(handle.paused(), None);
    assert_card_fits(&mut opened);
}

/// The native menu's Send next link and the Overview's Next link here send
/// one link and leave the stepper's count as the user set it.
#[gpui_kit::test]
fn a_one_link_send_keeps_the_steppers_count(cx: &mut TestAppContext) {
    let mut fixture = TrayModel::fixture(SystemTime::now());
    fixture.next_count = 5;
    let mut opened = open(fixture, cx);
    let (_router, handle) = with_router(&mut opened);
    opened
        .cx
        .update(|_, cx| super::run(super::TrayCommand::SendNext, cx));
    assert_eq!(
        handle.send_to().map(|send| send.scope),
        Some(SendScope::Next { left: 1, of: 1 })
    );
    let model = opened.model.clone();
    assert_eq!(model.read_with(opened.cx, |model, _| model.next_count), 5);
    opened.cx.update(|_, cx| {
        super::send_next_to(super::destination("Google Chrome", "Work", false), cx);
    });
    assert_eq!(
        handle.send_to().map(|send| send.scope),
        Some(SendScope::Next { left: 1, of: 1 })
    );
    assert_eq!(model.read_with(opened.cx, |model, _| model.next_count), 5);
    // The switch's Send takes the stepper's count.
    opened.cx.update(|_, cx| super::set_mode(Mode::Send, cx));
    assert_eq!(
        handle.send_to().map(|send| send.scope),
        Some(SendScope::Next { left: 5, of: 5 })
    );
}

/// Send (design: Incoming Links 2b): the next link to the destination the
/// To row names, the stepper adds more, and Rules ends it; the card fits
/// at every step.
#[gpui_kit::test]
fn send_goes_to_the_destination_and_rules_ends_it(cx: &mut TestAppContext) {
    let mut opened = open(TrayModel::fixture(SystemTime::now()), cx);
    let (_router, handle) = with_router(&mut opened);
    assert_card_fits(&mut opened);

    click(&mut opened, SEND);
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

    click(&mut opened, RULES);
    assert_eq!(handle.send_to(), None);
    assert!(opened.cx.debug_bounds(STEP_UP_ID).is_none());
    assert_card_fits(&mut opened);
}

/// For › Until off, then Time; Pause ends the send and Rules resumes: one
/// slot.
#[gpui_kit::test]
fn the_for_row_and_pause_share_the_slot(cx: &mut TestAppContext) {
    let mut opened = open(TrayModel::fixture(SystemTime::now()), cx);
    let (_router, handle) = with_router(&mut opened);

    click(&mut opened, SEND);
    click(&mut opened, FOR_IDS[2]);
    assert_eq!(
        handle.send_to().map(|send| send.scope),
        Some(SendScope::All { until: None })
    );
    assert!(opened.cx.debug_bounds(STEP_UP_ID).is_none(), "no stepper");
    assert_card_fits(&mut opened);
    click(&mut opened, FOR_IDS[1]);
    assert!(matches!(
        handle.send_to().map(|send| send.scope),
        Some(SendScope::All { until: Some(_) })
    ));
    assert_card_fits(&mut opened);
    // A longer time starts again from now.
    let before = handle.send_to().map(|send| send.scope);
    click(&mut opened, STEP_UP_ID);
    let after = handle.send_to().map(|send| send.scope);
    match (before, after) {
        (Some(SendScope::All { until: Some(a) }), Some(SendScope::All { until: Some(b) })) => {
            assert!(b > a, "{b:?} after {a:?}");
        }
        other => panic!("{other:?}"),
    }

    click(&mut opened, PAUSE);
    assert_eq!(handle.send_to(), None, "pausing ends the send");
    assert!(handle.paused().is_some());
    let model = opened.model.clone();
    assert_eq!(
        model.read_with(opened.cx, |model, _| model.mode()),
        Mode::Pause
    );
    assert_card_fits(&mut opened);

    click(&mut opened, RULES);
    assert_eq!(handle.paused(), None, "Rules resumes");
    assert_eq!(
        model.read_with(opened.cx, |model, _| model.routing),
        Routing::Active
    );
}

/// The To row opens the list in place; a pill picks it, and a send in
/// force goes there from the next link (Send Links 1b).
#[gpui_kit::test]
fn choosing_a_destination_retargets_the_send(cx: &mut TestAppContext) {
    let mut opened = open(TrayModel::fixture(SystemTime::now()), cx);
    let (_router, handle) = with_router(&mut opened);
    click(&mut opened, SEND);

    assert!(opened.cx.debug_bounds(DESTINATIONS_ID).is_none());
    click(&mut opened, DESTINATION_ID);
    assert!(
        opened.cx.debug_bounds(DESTINATIONS_ID).is_some(),
        "the list"
    );
    assert_card_fits(&mut opened);

    assert_lanes_measured(&mut opened);
    // Edge's lane: Default, Personal, Work (lit), Travel, InPrivate.
    let lit = opened.model.read_with(opened.cx, |model, _| {
        model.lanes()[0]
            .pills
            .iter()
            .map(|pill| pill.on)
            .collect::<Vec<_>>()
    });
    assert_eq!(lit, [false, false, true, false, false]);
    click(&mut opened, "tray-lanes-0-4");
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

/// The lanes are as tall as `lanes_height` measured them: the list is
/// sized before it is drawn, and pills wrap by their text.
fn assert_lanes_measured(opened: &mut Opened<'_>) {
    let model = opened.model.clone();
    let (lanes, measured) = opened.cx.update(|_, cx| {
        let lanes = model.read(cx).lanes();
        let measured = lanes_height(&lanes, &cx.text_system().clone());
        (lanes.len(), measured)
    });
    let first = opened
        .cx
        .debug_bounds("tray-lanes-lane-0")
        .expect("the first lane");
    let last = opened
        .cx
        .debug_bounds(Box::leak(
            format!("tray-lanes-lane-{}", lanes - 1).into_boxed_str(),
        ))
        .expect("the last lane");
    assert_eq!(first.size.width, px(LANES_WIDTH), "the lanes' width");
    let laid_out = last.bottom() - first.top();
    assert!(
        (laid_out - px(measured)).abs() <= px(0.5),
        "laid out {laid_out:?}, measured {measured}"
    );
}

/// A long destination list stops at [`LIST_MAX`] and scrolls, and the
/// card still fits.
#[gpui_kit::test]
fn a_long_destination_list_scrolls(cx: &mut TestAppContext) {
    let mut model = TrayModel::fixture(SystemTime::now());
    model.routing = Routing::Sending(SendScope::Next { left: 1, of: 1 });
    let browsers = model.choices.browsers.clone();
    for _ in 0..3 {
        model.choices.browsers.extend(browsers.clone());
    }
    let mut opened = open(model, cx);
    click(&mut opened, DESTINATION_ID);
    let list = opened.cx.debug_bounds(DESTINATIONS_ID).expect("the list");
    assert!(list.size.height <= px(LIST_MAX + 0.5), "{list:?}");
    assert_card_fits(&mut opened);
}

/// Dispatches a link and reads the router back, as `follow_feed` does
/// after every dispatch: the slot, and the held links from its buffer.
fn dispatch_and_sync(opened: &mut Opened<'_>, handle: &RouterHandle, url: &str) {
    handle
        .dispatch_and_wait(Link::new(url, Source::CommandLine))
        .unwrap();
    sync_from(opened, handle);
}

fn sync_from(opened: &mut Opened<'_>, handle: &RouterHandle) {
    let model = opened.model.clone();
    let records = handle.recent();
    opened.cx.update(|_, cx| {
        model.update(cx, |model, cx| {
            model.sync(Dispatched::of_records(&records, records.len() as u64));
            cx.notify();
        });
        super::sync_override(&model, cx);
    });
    opened.settle();
}

/// The router counts a send down; after each dispatch the tray reads it
/// back (`sync_override`), so the stepper and the chip follow and the
/// switch returns to Rules when the count runs out.
#[gpui_kit::test]
fn the_tray_follows_the_routers_countdown(cx: &mut TestAppContext) {
    let mut opened = open(TrayModel::fixture(SystemTime::now()), cx);
    let (_router, handle) = with_router(&mut opened);
    opened
        .cx
        .update(|_, cx| cx.global_mut::<Tray>().live = true);
    click(&mut opened, SEND);
    click(&mut opened, STEP_UP_ID);
    let model = opened.model.clone();
    dispatch_and_sync(&mut opened, &handle, "https://docs.example.com/");
    assert_eq!(
        model.read_with(opened.cx, |model, _| model.routing),
        Routing::Sending(SendScope::Next { left: 1, of: 2 })
    );
    assert_card_fits(&mut opened);
    dispatch_and_sync(&mut opened, &handle, "https://docs.example.com/");
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

/// Hold (design: Incoming Links 2c): the next two links are recorded and
/// not opened, they lead the card, Open opens one where the rules say,
/// Open all the rest, and the hold ends by itself after two.
#[gpui_kit::test]
fn held_links_wait_in_the_card_until_opened(cx: &mut TestAppContext) {
    let mut opened = open(TrayModel::fixture(SystemTime::now()), cx);
    let (_router, handle) = with_router(&mut opened);
    opened
        .cx
        .update(|_, cx| cx.global_mut::<Tray>().live = true);
    click(&mut opened, HOLD);
    click(&mut opened, STEP_UP_ID);
    assert_eq!(
        handle.override_in_force(),
        Some(Override::Holding(SendScope::Next { left: 2, of: 2 }))
    );
    let model = opened.model.clone();
    assert_eq!(
        model.read_with(opened.cx, |model, _| model.chip().label()),
        "holding"
    );
    assert_card_fits(&mut opened);

    dispatch_and_sync(&mut opened, &handle, "https://chat.example.com/r/team-7");
    dispatch_and_sync(&mut opened, &handle, "https://l.example.net/t/9f2c1a");
    assert_eq!(
        handle.override_in_force(),
        None,
        "two held: the rules again"
    );
    assert_eq!(handle.pending().len(), 2);
    let held = model.read_with(opened.cx, |model, _| model.held.clone());
    assert_eq!(
        held.iter()
            .map(|held| held.host.as_str())
            .collect::<Vec<_>>(),
        ["l.example.net", "chat.example.com"],
        "newest first"
    );
    assert_eq!(held[0].rest, "/t/9f2c1a");
    assert!(opened.cx.debug_bounds(OPEN_ALL_ID).is_some());
    assert_card_fits(&mut opened);

    click_named(&mut opened, &held_id("open", 0));
    opened.cx.run_until_parked();
    sync_from(&mut opened, &handle);
    assert_eq!(handle.pending(), [held[1].id], "the newest opened");
    assert_eq!(model.read_with(opened.cx, |model, _| model.held.len()), 1);
    assert_card_fits(&mut opened);

    click(&mut opened, OPEN_ALL_ID);
    opened.cx.run_until_parked();
    sync_from(&mut opened, &handle);
    assert!(handle.pending().is_empty());
    assert!(model.read_with(opened.cx, |model, _| model.held.is_empty()));
    assert!(
        opened.cx.debug_bounds(OPEN_ALL_ID).is_none(),
        "no Held section"
    );
    assert_card_fits(&mut opened);
}

/// With Activity keeping nothing, a held link would have nowhere to wait:
/// Hold does not turn on, whatever the tray last read, and a hold already
/// on ends as soon as the tray reads the retention again.
#[gpui_kit::test]
fn hold_does_not_turn_on_while_nothing_is_kept(cx: &mut TestAppContext) {
    let mut opened = open(TrayModel::fixture(SystemTime::now()), cx);
    let (_router, handle) = with_router(&mut opened);
    let model = opened.model.clone();
    let nothing = Retention { count: 0, hours: 0 };
    handle.set_retention(nothing).unwrap();
    assert!(
        model.read_with(opened.cx, |model, _| model.can_hold),
        "not read yet"
    );
    click(&mut opened, HOLD);
    assert_eq!(handle.override_in_force(), None);
    assert!(!model.read_with(opened.cx, |model, _| model.can_hold));
    opened.cx.update(|_, cx| super::set_mode(Mode::Hold, cx));
    assert_eq!(handle.override_in_force(), None);

    // Kept again: Hold reads it before it starts (the segment stays drawn
    // off until the tray reads it).
    handle.set_retention(Retention::default()).unwrap();
    opened.cx.update(|_, cx| super::set_mode(Mode::Hold, cx));
    assert!(matches!(
        handle.override_in_force(),
        Some(Override::Holding(_))
    ));
    handle.set_retention(nothing).unwrap();
    opened
        .cx
        .update(|_, cx| super::refresh_can_hold(&model, cx));
    opened.settle();
    assert_eq!(handle.override_in_force(), None);
    assert_eq!(
        model.read_with(opened.cx, |model, _| model.mode()),
        Mode::Rules
    );
}

/// A held link the router no longer keeps (gone by age, or swept) is not
/// listed, though the feed still has its record.
#[gpui_kit::test]
fn the_tray_lists_only_what_the_router_still_holds(cx: &mut TestAppContext) {
    let mut opened = open(TrayModel::fixture(SystemTime::now()), cx);
    let (_router, handle) = with_router(&mut opened);
    handle.set_hold(Some(SendScope::Next { left: 1, of: 1 }));
    handle
        .dispatch_and_wait(Link::new(
            "https://chat.example.com/r/team-7",
            Source::CommandLine,
        ))
        .unwrap();
    let mut feed = Feed::new(Some(handle.clone()));
    for record in handle.recent().into_iter().rev() {
        feed.push(record);
    }
    assert_eq!(Dispatched::of(&feed, SystemTime::now()).held.len(), 1);

    // The router drops it; the feed is not reloaded.
    handle
        .set_retention(Retention { count: 0, hours: 0 })
        .unwrap();
    handle.set_retention(Retention::default()).unwrap();
    assert_eq!(feed.records().len(), 1);
    assert!(Dispatched::of(&feed, SystemTime::now()).held.is_empty());
}

/// The keyboard (design: Incoming Links 2f): Tab reaches the switch,
/// Right turns Send on, Tab walks To, For and the stepper, Right and Up
/// change them, and Return on the live line's button goes back to the
/// rules.
#[gpui_kit::test]
fn the_switch_works_from_the_keyboard(cx: &mut TestAppContext) {
    let mut opened = open(TrayModel::fixture(SystemTime::now()), cx);
    let (_router, handle) = with_router(&mut opened);
    let view = opened.view.clone();
    let model = opened.model.clone();
    let focused = |opened: &mut Opened<'_>| {
        opened.cx.update(|window, cx| {
            let stops = &view.read(cx).stops;
            [
                ("switch", &stops.switch),
                ("to", &stops.to),
                ("for", &stops.for_row),
                ("stepper", &stops.stepper),
                ("live", &stops.live),
            ]
            .into_iter()
            .find(|(_, handle)| handle.is_focused(window))
            .map(|(name, _)| name)
        })
    };
    let key = |opened: &mut Opened<'_>, keys: &str| {
        opened.cx.simulate_keystrokes(keys);
        opened.settle();
    };
    // As `dropdown::open` does: the keys bound, the card focused.
    opened.cx.update(|window, cx| {
        super::dropdown::bind_keys(cx);
        let focus = view.read(cx).focus.clone();
        window.focus(&focus, cx);
    });
    opened.settle();

    key(&mut opened, "tab");
    assert_eq!(focused(&mut opened), Some("switch"));
    key(&mut opened, "right");
    assert_eq!(
        model.read_with(opened.cx, |model, _| model.mode()),
        Mode::Send
    );
    assert!(handle.send_to().is_some());
    assert_card_fits(&mut opened);
    key(&mut opened, "right");
    assert_eq!(
        model.read_with(opened.cx, |model, _| model.mode()),
        Mode::Hold
    );
    key(&mut opened, "left");
    assert_eq!(
        model.read_with(opened.cx, |model, _| model.mode()),
        Mode::Send
    );

    key(&mut opened, "tab");
    assert_eq!(focused(&mut opened), Some("to"));
    key(&mut opened, "tab");
    assert_eq!(focused(&mut opened), Some("for"));
    key(&mut opened, "right");
    assert_eq!(
        model.read_with(opened.cx, |model, _| model.for_shown()),
        ForKind::Time
    );
    key(&mut opened, "tab");
    assert_eq!(focused(&mut opened), Some("stepper"));
    let step = model.read_with(opened.cx, |model, _| model.time_step);
    key(&mut opened, "up");
    assert_eq!(
        model.read_with(opened.cx, |model, _| model.time_step),
        step + 1
    );
    assert_card_fits(&mut opened);
    key(&mut opened, "tab");
    assert_eq!(focused(&mut opened), Some("live"));
    key(&mut opened, "enter");
    assert_eq!(handle.send_to(), None, "back to rules");
    assert_eq!(focused(&mut opened), Some("switch"));
    assert_card_fits(&mut opened);
}
