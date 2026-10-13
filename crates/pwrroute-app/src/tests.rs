//! Smoke tests through gpui-kit's window harness: open the real view, draw
//! frames, press keys.

use gpui_kit::{Bounds, Entity, Pixels, TestAppContext, VisualTestContext, px};

use crate::app::{PwrRoute, ROOT_ID};

fn open(cx: &mut TestAppContext) -> (Entity<PwrRoute>, &mut VisualTestContext) {
    cx.update(|cx| crate::assets::load_fonts(cx));
    let (view, cx) = cx.add_window_view(|_, cx| PwrRoute::new(cx));
    let focus = view.read_with(cx, |app, _| app.focus.clone());
    cx.update(|window, cx| window.focus(&focus, cx));
    (view, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

#[gpui_kit::test]
fn the_window_draws_a_frame(cx: &mut TestAppContext) {
    let (_view, cx) = open(cx);
    draw(cx);
    let bounds: Bounds<Pixels> = cx.debug_bounds(ROOT_ID).expect("root element");
    assert!(bounds.size.width > px(0.0) && bounds.size.height > px(0.0));
}

#[gpui_kit::test]
fn arrow_keys_move_the_selected_handler(cx: &mut TestAppContext) {
    let (view, cx) = open(cx);
    draw(cx);
    assert_eq!(view.read_with(cx, |app, _| app.selected), 2);

    cx.simulate_keystrokes("down down");
    draw(cx);
    assert_eq!(view.read_with(cx, |app, _| app.selected), 4);

    // Clamped at both ends.
    cx.simulate_keystrokes("up up up up up up up");
    draw(cx);
    assert_eq!(view.read_with(cx, |app, _| app.selected), 0);
}
