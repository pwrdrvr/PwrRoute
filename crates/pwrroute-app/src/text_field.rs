//! A one-line text field: enough editing for a URL or a search.
//!
//! The caret stays at the end: typing appends, backspace deletes the last
//! character, ⌘A selects everything (the next edit replaces it), ⌘C, ⌘X and
//! ⌘V use the clipboard and Return submits. gpui-kit's own input lives in its
//! `component` layer, which this app does not build.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClipboardItem, Div, FocusHandle, InteractiveElement as _, Keystroke,
    ParentElement as _, SharedString, Stateful, Styled as _, Window, div, px,
};

use crate::theme;
use crate::widgets::{focus_ring, input_box};

/// The field's text and focus.
pub struct TextField {
    pub text: String,
    pub focus: FocusHandle,
    /// ⌘A: the whole text is selected.
    pub selected: bool,
}

/// What a keystroke did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    /// The text changed.
    Changed,
    /// Return.
    Submit,
    /// Handled, nothing changed (a copy, a selection).
    Handled,
    /// Not the field's key; let it bubble.
    Ignored,
}

impl TextField {
    pub fn new(text: impl Into<String>, cx: &mut App) -> Self {
        Self {
            text: text.into(),
            focus: cx.focus_handle(),
            selected: false,
        }
    }

    pub fn shared(&self) -> SharedString {
        self.text.clone().into()
    }

    /// Replaces the text, leaving the caret at its end.
    pub fn set(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.selected = false;
    }

    fn insert(&mut self, text: &str) {
        if self.selected {
            self.text.clear();
            self.selected = false;
        }
        // One line: pasted text stops at its first line break.
        let line = text.lines().next().unwrap_or_default();
        self.text.extend(line.chars().filter(|c| !c.is_control()));
    }

    /// Applies one keystroke.
    pub fn key(&mut self, keystroke: &Keystroke, cx: &mut App) -> Edit {
        let modifiers = &keystroke.modifiers;
        let command = modifiers.platform || modifiers.control;
        match (command, keystroke.key.as_str()) {
            (_, "enter") => Edit::Submit,
            (false, "backspace") => {
                if self.selected {
                    self.text.clear();
                    self.selected = false;
                } else {
                    self.text.pop();
                }
                Edit::Changed
            }
            (true, "backspace") => {
                self.set("");
                Edit::Changed
            }
            (true, "a") => {
                self.selected = !self.text.is_empty();
                Edit::Handled
            }
            (true, "c" | "x") => {
                let copied = if self.selected { &self.text } else { "" };
                if !copied.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(copied.to_owned()));
                }
                if keystroke.key == "x" && self.selected {
                    self.set("");
                    return Edit::Changed;
                }
                Edit::Handled
            }
            (true, "v") => match cx.read_from_clipboard().and_then(|item| item.text()) {
                Some(text) => {
                    self.insert(&text);
                    Edit::Changed
                }
                None => Edit::Handled,
            },
            (true, _) => Edit::Ignored,
            (false, "escape") => {
                self.selected = false;
                Edit::Handled
            }
            (false, key) => {
                let typed = keystroke.key_char.clone().or_else(|| match key {
                    "space" => Some(" ".to_owned()),
                    _ if key.chars().count() == 1 => Some(key.to_owned()),
                    _ => None,
                });
                match typed {
                    Some(text) if !text.chars().all(char::is_control) => {
                        self.insert(&text);
                        Edit::Changed
                    }
                    _ => Edit::Ignored,
                }
            }
        }
    }
}

/// How a field draws its text.
#[derive(Clone, Copy)]
pub struct Look {
    /// `.inp.mono`
    pub mono: bool,
    pub placeholder: &'static str,
}

/// `.inp` for a field: its text, a caret while focused, and the focus ring.
/// Leading content (a search icon) goes in `leading`. The caller adds the
/// key handler (`on_key_down`) to the returned element.
pub fn field(
    id: &'static str,
    field: &TextField,
    look: Look,
    leading: Option<AnyElement>,
    window: &Window,
) -> Stateful<Div> {
    let focused = field.focus.is_focused(window);
    let (family, weight, size) = if look.mono {
        (theme::mono_family(500.0), 500.0, 12.0)
    } else {
        (theme::SANS, 400.0, 12.5)
    };
    let content = if field.text.is_empty() {
        div()
            .text_color(theme::text_subtle())
            .child(look.placeholder)
    } else {
        div()
            .text_color(theme::text_primary())
            .when(field.selected, |text| text.bg(theme::accent_soft()))
            .child(field.shared())
    };
    let content = content
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        // While typing, keep the end of a long URL in view.
        .map(|text| {
            if focused {
                text.text_ellipsis_start()
            } else {
                text.text_ellipsis()
            }
        })
        .font_family(family)
        .font_weight(theme::weight(weight))
        .text_size(px(size))
        .line_height(px(size));
    let mut inner = input_box()
        .w_full()
        .gap(px(8.0))
        .when_some(leading, |row, leading| row.child(leading));
    if field.text.is_empty() && focused {
        inner = inner.child(caret_bar());
    }
    inner = inner.child(content);
    if !field.text.is_empty() && focused && !field.selected {
        inner = inner.child(caret_bar().ml(px(-8.0)));
    }
    let boxed = if focused { focus_ring(inner) } else { inner };
    div()
        .id(id)
        .debug_selector(|| id.into())
        .track_focus(&field.focus)
        .flex_1()
        .min_w_0()
        .cursor_text()
        .child(boxed)
}

fn caret_bar() -> Div {
    div().flex_none().w(px(1.0)).h(px(14.0)).bg(theme::accent())
}
