//! The main window's view: its state, keyboard handling and layout.

use gpui_kit::{
    Context, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _,
    Render, Styled as _, Window, div,
};

use crate::fixtures::{self, Handler, Rule};
use crate::{chrome, editor, rules_view, theme};

/// The element id of the window's root, for tests and the inspector.
pub const ROOT_ID: &str = "pwrroute-root";

pub struct PwrRoute {
    pub focus: FocusHandle,
    pub rewrites: Vec<Rule>,
    pub handlers: Vec<Handler>,
    /// Index into `handlers` of the row the editor rail shows.
    pub selected: usize,
}

impl PwrRoute {
    pub fn new(cx: &mut Context<'_, Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            rewrites: fixtures::rewrites(),
            handlers: fixtures::handlers(),
            selected: 2,
        }
    }

    /// Select a handler row, clamped to the list.
    pub fn select(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        let index = index.min(self.handlers.len().saturating_sub(1));
        if index != self.selected {
            self.selected = index;
            cx.notify();
        }
    }

    /// Switch the selected handler on or off.
    pub fn toggle_selected(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(handler) = self.handlers.get_mut(self.selected) {
            handler.rule.enabled = !handler.rule.enabled;
            cx.notify();
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<'_, Self>) {
        match event.keystroke.key.as_str() {
            "up" => self.select(self.selected.saturating_sub(1), cx),
            "down" => self.select(self.selected + 1, cx),
            _ => {}
        }
    }
}

impl Render for PwrRoute {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        div()
            .id(ROOT_ID)
            .debug_selector(|| ROOT_ID.into())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                this.on_key_down(event, cx);
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::bg_app())
            .text_color(theme::text_primary())
            .font_family(theme::SANS)
            .child(chrome::title_strip())
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(chrome::nav_rail())
                    .child(rules_view::pane(self, cx))
                    .child(editor::rail(self, cx)),
            )
    }
}
