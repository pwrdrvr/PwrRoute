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
use pwrroute_router::{DryRunLauncher, Options, PauseTarget, Router};

use super::dropdown::{
    BODY_ID, CARD_ID, Dropdown, MAKE_DEFAULT_ID, PAUSE_ID, QUIT_ID, WIDTH, card_height,
};
use super::model::{BrowserLabel, Recent, RecentRoute, TrayModel};
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
    let card = opened.cx.debug_bounds(CARD_ID).expect("the card");
    let computed = opened.cx.update(|_, cx| {
        let text_system = cx.text_system().clone();
        card_height(model.read(cx), &text_system)
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
        card_height(opened.model.read(cx), &text_system)
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

/// An earlier build's registration is the default: the row stays, and the
/// card still fits it.
#[gpui_kit::test]
fn a_legacy_default_keeps_the_make_default_row(cx: &mut TestAppContext) {
    let mut legacy = TrayModel::fixture(SystemTime::now());
    legacy.default_is_legacy = true;
    let mut opened = open(legacy, cx);
    assert!(opened.cx.debug_bounds(MAKE_DEFAULT_ID).is_some(), "the row");
    assert_card_fits(&mut opened);
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
