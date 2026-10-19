//! The dropdown through gpui-kit's window harness: its card is as tall as
//! `card_height` says (the window is sized from that), it fits the window
//! in every state, and the pause row pauses.

use std::time::SystemTime;

use gpui_kit::{AppContext as _, Entity, Modifiers, TestAppContext, VisualTestContext, px};

use super::dropdown::{CARD_ID, Dropdown, PAUSE_ID, card_height, window_height};
use super::model::TrayModel;

fn open(model: TrayModel, cx: &mut TestAppContext) -> (Entity<TrayModel>, &mut VisualTestContext) {
    cx.update(|cx| crate::assets::load_fonts(cx));
    let model = cx.new(|_| model);
    let shared = model.clone();
    let (_view, cx) = cx.add_window_view(|window, cx| Dropdown::new(shared, window, cx));
    draw(cx);
    (model, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

/// The laid-out card and the computed one agree to a pixel, and the window
/// the dropdown opens in holds it.
fn assert_card_fits(model: &Entity<TrayModel>, cx: &mut VisualTestContext) {
    let laid_out = cx.debug_bounds(CARD_ID).expect("the card").size.height;
    let (computed, window) = cx.update(|_, cx| {
        let text_system = cx.text_system().clone();
        let model = model.read(cx);
        (
            card_height(model, &text_system),
            window_height(model, &text_system),
        )
    });
    assert!(
        (laid_out - computed).abs() <= px(1.0),
        "laid out {laid_out:?}, computed {computed:?}"
    );
    assert!(laid_out <= window, "card {laid_out:?} in window {window:?}");
}

#[gpui_kit::test]
fn the_routing_dropdown_is_as_tall_as_computed(cx: &mut TestAppContext) {
    let (model, cx) = open(TrayModel::fixture(SystemTime::now()), cx);
    assert_card_fits(&model, cx);
}

#[gpui_kit::test]
fn the_paused_dropdown_is_as_tall_as_computed(cx: &mut TestAppContext) {
    let (model, cx) = open(TrayModel::paused_fixture(SystemTime::now()), cx);
    assert_card_fits(&model, cx);
}

#[gpui_kit::test]
fn an_empty_dropdown_is_as_tall_as_computed(cx: &mut TestAppContext) {
    let mut empty = TrayModel::fixture(SystemTime::now());
    empty.recents.clear();
    empty.is_default_browser = false;
    let (model, cx) = open(empty, cx);
    assert_card_fits(&model, cx);
}

#[gpui_kit::test]
fn the_pause_row_pauses_and_the_card_still_fits(cx: &mut TestAppContext) {
    let (model, cx) = open(TrayModel::fixture(SystemTime::now()), cx);
    let pause = cx.debug_bounds(PAUSE_ID).expect("the pause row");

    cx.simulate_click(pause.center(), Modifiers::none());
    draw(cx);
    assert!(model.read_with(cx, |model, _| model.is_paused()));
    // Paused drops the activity row: the card shrinks inside the same window.
    assert_card_fits(&model, cx);

    let pause = cx.debug_bounds(PAUSE_ID).expect("the pause row");
    cx.simulate_click(pause.center(), Modifiers::none());
    draw(cx);
    assert!(!model.read_with(cx, |model, _| model.is_paused()));
}
