//! A one-line text field: enough editing for a URL or a search.
//!
//! It has a caret and a selection, as a native field does. ← and → move by
//! character, ⌥ (or Ctrl) by word, ⌘ to the ends, as do Home and End; Shift
//! extends the selection. A click places the caret, Shift-click extends, a
//! drag selects, a double-click selects a word and a triple-click the text.
//! Typing and ⌘V replace the selection; ⌫ and Delete remove it or the
//! character beside the caret (⌥ a word, ⌘ to the line's end); ⌘A, ⌘C and
//! ⌘X work on it; Return submits. While focused, a long text scrolls to
//! keep the caret in view; otherwise it ends in an ellipsis. gpui-kit's own
//! input lives in its `component` layer, which this app does not build.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Bounds, ClipboardItem, ContentMask, DispatchPhase, Div, Element, ElementId,
    FocusHandle, Font, GlobalElementId, InspectorElementId, InteractiveElement as _, IntoElement,
    Keystroke, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement as _, Pixels, ShapedLine, SharedString, Stateful, Style, Styled as _, TextAlign,
    TextRun, TruncateFrom, Window, div, fill, font, point, px, relative,
};

use crate::theme;
use crate::widgets::{focus_ring, input_box};

/// The field's text, focus, caret and selection.
pub struct TextField {
    pub text: String,
    pub focus: FocusHandle,
    /// Shared with the drawn text, which places the caret on a click.
    caret: Rc<RefCell<Caret>>,
}

/// Where the caret is, and what the last paint drew.
#[derive(Default)]
struct Caret {
    /// Where the selection started; equal to `head` when nothing is
    /// selected. A byte offset into the text, like `head`.
    anchor: usize,
    /// The caret.
    head: usize,
    /// How far the text is scrolled left (focused only).
    scroll: Pixels,
    /// The last paint's whole line and the window x of its start, for
    /// placing the caret under the pointer. None for an empty field.
    painted: Option<(ShapedLine, Pixels)>,
    /// A drag from a press in the field.
    drag: Option<Drag>,
}

/// What a drag selects by.
#[derive(Clone)]
enum Drag {
    Char,
    /// From a double-click: whole words, from the word clicked.
    Word(Range<usize>),
}

impl Caret {
    /// The text offset under a window x, from the last paint.
    fn index_at(&self, x: Pixels) -> usize {
        match &self.painted {
            Some((line, start)) => line.closest_index_for_x(x - *start),
            None => 0,
        }
    }

    fn painted_text(&self) -> SharedString {
        self.painted
            .as_ref()
            .map(|(line, _)| line.text.clone())
            .unwrap_or_default()
    }
}

/// What a keystroke did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    /// The text changed.
    Changed,
    /// Return.
    Submit,
    /// Handled, nothing changed (a copy, a caret move, a selection).
    Handled,
    /// Not the field's key; let it bubble.
    Ignored,
}

impl TextField {
    pub fn new(text: impl Into<String>, cx: &mut App) -> Self {
        let text = text.into();
        let end = text.len();
        Self {
            text,
            // Every field is in its window's Tab order.
            focus: cx.focus_handle().tab_stop(true),
            caret: Rc::new(RefCell::new(Caret {
                anchor: end,
                head: end,
                ..Caret::default()
            })),
        }
    }

    pub fn shared(&self) -> SharedString {
        self.text.clone().into()
    }

    /// Replaces the text, leaving the caret at its end.
    pub fn set(&mut self, text: impl Into<String>) {
        self.text = text.into();
        let end = self.text.len();
        self.select(end, end);
    }

    /// Selects the whole text (the next edit replaces it).
    pub fn select_all(&mut self) {
        self.select(0, self.text.len());
    }

    /// The whole text, and something, is selected.
    #[cfg(test)]
    pub fn all_selected(&self) -> bool {
        !self.text.is_empty() && self.selection() == (0..self.text.len())
    }

    /// The selection, start first; empty at the caret when nothing is.
    pub fn selection(&self) -> Range<usize> {
        let caret = self.caret.borrow();
        let (a, b) = (self.clamp(caret.anchor), self.clamp(caret.head));
        a.min(b)..a.max(b)
    }

    /// The caret's byte offset.
    pub fn cursor(&self) -> usize {
        self.clamp(self.caret.borrow().head)
    }

    /// An offset made safe for the text: at most its length, on a char
    /// boundary.
    fn clamp(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    fn select(&mut self, anchor: usize, head: usize) {
        let mut caret = self.caret.borrow_mut();
        caret.anchor = anchor;
        caret.head = head;
    }

    /// Moves the caret to `to`, extending the selection when `extend`.
    fn move_to(&mut self, to: usize, extend: bool) {
        let anchor = if extend {
            self.clamp(self.caret.borrow().anchor)
        } else {
            to
        };
        self.select(anchor, to);
    }

    /// Replaces `range` with `with`, the caret after it.
    fn replace(&mut self, range: Range<usize>, with: &str) {
        self.text.replace_range(range.clone(), with);
        let at = range.start + with.len();
        self.select(at, at);
    }

    /// Types or pastes `text` over the selection.
    fn insert(&mut self, text: &str) {
        // One line: pasted text stops at its first line break.
        let line: String = text
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_control())
            .collect();
        let selection = self.selection();
        self.replace(selection, &line);
    }

    /// Applies one keystroke.
    pub fn key(&mut self, keystroke: &Keystroke, cx: &mut App) -> Edit {
        let modifiers = &keystroke.modifiers;
        let command = modifiers.platform || modifiers.control;
        // ⌘ goes to the line's end; ⌥ (Ctrl off macOS) by word.
        let line = modifiers.platform;
        let word = !line && (modifiers.alt || modifiers.control);
        let extend = modifiers.shift;
        let len = self.text.len();
        let head = self.cursor();
        let selection = self.selection();
        match keystroke.key.as_str() {
            "enter" => Edit::Submit,
            key @ ("left" | "right") => {
                let forward = key == "right";
                let to = if line {
                    if forward { len } else { 0 }
                } else if word {
                    if forward {
                        next_word(&self.text, head)
                    } else {
                        prev_word(&self.text, head)
                    }
                } else if !extend && !selection.is_empty() {
                    if forward {
                        selection.end
                    } else {
                        selection.start
                    }
                } else if forward {
                    next_char(&self.text, head)
                } else {
                    prev_char(&self.text, head)
                };
                self.move_to(to, extend);
                Edit::Handled
            }
            key @ ("home" | "end") => {
                self.move_to(if key == "end" { len } else { 0 }, extend);
                Edit::Handled
            }
            key @ ("backspace" | "delete") => {
                let forward = key == "delete";
                let range = if !selection.is_empty() {
                    selection
                } else if forward {
                    let to = if line {
                        len
                    } else if word {
                        next_word(&self.text, head)
                    } else {
                        next_char(&self.text, head)
                    };
                    head..to
                } else {
                    let from = if line {
                        0
                    } else if word {
                        prev_word(&self.text, head)
                    } else {
                        prev_char(&self.text, head)
                    };
                    from..head
                };
                if range.is_empty() {
                    return Edit::Handled;
                }
                self.replace(range, "");
                Edit::Changed
            }
            "a" if command => {
                self.select_all();
                Edit::Handled
            }
            key @ ("c" | "x") if command => {
                if selection.is_empty() {
                    return Edit::Handled;
                }
                let copied = self.text[selection.clone()].to_owned();
                cx.write_to_clipboard(ClipboardItem::new_string(copied));
                if key == "x" {
                    self.replace(selection, "");
                    return Edit::Changed;
                }
                Edit::Handled
            }
            "v" if command => match cx.read_from_clipboard().and_then(|item| item.text()) {
                Some(text) => {
                    self.insert(&text);
                    Edit::Changed
                }
                None => Edit::Handled,
            },
            _ if command => Edit::Ignored,
            "escape" => {
                self.move_to(head, false);
                Edit::Handled
            }
            key => {
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

    /// The box's mouse-down handler: a press places the caret where it
    /// lands (Shift extends the selection), a double-click selects a word,
    /// a triple-click the text, and a drag from the press selects.
    fn on_press(&self) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
        let caret = self.caret.clone();
        move |event, window, _| {
            let mut caret = caret.borrow_mut();
            let at = caret.index_at(event.position.x);
            let text = caret.painted_text();
            match event.click_count {
                0 | 1 => {
                    caret.head = at;
                    if !event.modifiers.shift {
                        caret.anchor = at;
                    }
                    caret.drag = Some(Drag::Char);
                }
                2 => {
                    let word = word_at(&text, at);
                    caret.anchor = word.start;
                    caret.head = word.end;
                    caret.drag = Some(Drag::Word(word));
                }
                _ => {
                    caret.anchor = 0;
                    caret.head = text.len();
                    caret.drag = None;
                }
            }
            window.refresh();
        }
    }
}

/// Letters, digits and `_` make words; anything else (`.`, `/`, `-`)
/// separates them.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn prev_char(text: &str, at: usize) -> usize {
    text[..at].char_indices().next_back().map_or(0, |(i, _)| i)
}

fn next_char(text: &str, at: usize) -> usize {
    text[at..].chars().next().map_or(at, |c| at + c.len_utf8())
}

/// The start of the word before `at`, past any separators.
fn prev_word(text: &str, at: usize) -> usize {
    let mut chars = text[..at].char_indices().rev().peekable();
    let mut to = at;
    while let Some((i, _)) = chars.next_if(|(_, c)| !is_word(*c)) {
        to = i;
    }
    while let Some((i, _)) = chars.next_if(|(_, c)| is_word(*c)) {
        to = i;
    }
    to
}

/// The end of the word after `at`, past any separators.
fn next_word(text: &str, at: usize) -> usize {
    let mut chars = text[at..].char_indices().peekable();
    let mut to = at;
    while let Some((i, c)) = chars.next_if(|(_, c)| !is_word(*c)) {
        to = at + i + c.len_utf8();
    }
    while let Some((i, c)) = chars.next_if(|(_, c)| is_word(*c)) {
        to = at + i + c.len_utf8();
    }
    to
}

/// The word at `at` (the one it ends, at the text's end), or the single
/// separator there.
fn word_at(text: &str, at: usize) -> Range<usize> {
    let at = at.min(text.len());
    let start = if at == text.len() {
        prev_char(text, at)
    } else {
        at
    };
    match text[start..].chars().next() {
        None => at..at,
        Some(c) if !is_word(c) => start..start + c.len_utf8(),
        Some(_) => {
            let from = text[..start]
                .char_indices()
                .rev()
                .take_while(|(_, c)| is_word(*c))
                .last()
                .map_or(start, |(i, _)| i);
            from..next_word(text, start)
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
    field_with(id, field, look, leading, None, window)
}

/// [`field`] with content after the text, at the box's right edge (a
/// search's match count and its steps).
pub fn field_with(
    id: &'static str,
    field: &TextField,
    look: Look,
    leading: Option<AnyElement>,
    trailing: Option<AnyElement>,
    window: &Window,
) -> Stateful<Div> {
    let focused = field.focus.is_focused(window);
    let inner = input_box()
        .w_full()
        .gap(px(8.0))
        .when_some(leading, |row, leading| row.child(leading))
        .child(text(field, look, focused))
        .when_some(trailing, |row, trailing| {
            row.child(
                div()
                    .ml_auto()
                    .flex()
                    .flex_none()
                    .items_center()
                    .child(trailing),
            )
        });
    let boxed = if focused { focus_ring(inner) } else { inner };
    hit_area(div().id(id).debug_selector(|| id.into()), field)
        .flex_1()
        .min_w_0()
        .child(boxed)
}

/// Makes `element`, the box around a field, its hit area: a press there
/// focuses the field and places the caret (see [`TextField::on_press`]),
/// under the text cursor.
pub fn hit_area(element: Stateful<Div>, field: &TextField) -> Stateful<Div> {
    element
        .track_focus(&field.focus)
        .cursor_text()
        .on_mouse_down(MouseButton::Left, field.on_press())
}

/// The field's text with its caret and selection, for an `.inp` box made a
/// [`hit_area`].
pub fn text(field: &TextField, look: Look, focused: bool) -> FieldText {
    let (family, weight, size) = if look.mono {
        (theme::mono_family(500.0), 500.0, 12.0)
    } else {
        (theme::SANS, 400.0, 12.5)
    };
    FieldText {
        text: field.shared(),
        placeholder: look.placeholder.into(),
        caret: field.caret.clone(),
        focused,
        font: Font {
            weight: theme::weight(weight),
            ..font(family)
        },
        size: px(size),
    }
}

/// The caret's width, and its height (centered on the line).
const CARET: (f32, f32) = (1.0, 14.0);

/// A field's text, as a native field draws it: shaped once, scrolled to
/// keep the caret in view, the selection behind it.
pub struct FieldText {
    text: SharedString,
    placeholder: SharedString,
    caret: Rc<RefCell<Caret>>,
    focused: bool,
    font: Font,
    size: Pixels,
}

/// What prepaint laid out for paint.
pub struct Laid {
    line: ShapedLine,
    origin_x: Pixels,
    selection: Option<Bounds<Pixels>>,
    caret: Option<Bounds<Pixels>>,
}

impl FieldText {
    fn run(&self, len: usize, color: gpui_kit::Hsla) -> TextRun {
        TextRun {
            len,
            font: self.font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }
    }

    /// The whole line, and the line drawn in `width`: the same, or cut
    /// with an ellipsis.
    fn fitted(
        &self,
        text: SharedString,
        run: TextRun,
        width: Pixels,
        window: &Window,
    ) -> (ShapedLine, ShapedLine) {
        let system = window.text_system();
        let runs = [run];
        let whole = system.shape_line(text.clone(), self.size, &runs, None);
        if whole.width <= width {
            return (whole.clone(), whole);
        }
        let (cut, cut_runs) = system
            .line_wrapper(self.font.clone(), self.size)
            .truncate_line(text, width, "\u{2026}", &runs, TruncateFrom::End);
        let shown = system.shape_line(cut, self.size, &cut_runs, None);
        (whole, shown)
    }
}

impl IntoElement for FieldText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for FieldText {
    type RequestLayoutState = ();
    type PrepaintState = Laid;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.min_size.width = px(0.).into();
        style.flex_shrink = 1.;
        style.size.height = self.size.into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) -> Laid {
        let width = bounds.size.width;
        let caret_width = px(CARET.0);
        let mut caret = self.caret.borrow_mut();
        let len = self.text.len();
        let (anchor, head) = (caret.anchor.min(len), caret.head.min(len));
        let (line, scroll) = if self.text.is_empty() {
            caret.painted = None;
            let run = self.run(self.placeholder.len(), theme::text_subtle());
            let (_, shown) = self.fitted(self.placeholder.clone(), run, width, window);
            (shown, px(0.))
        } else if self.focused {
            let run = self.run(len, theme::text_primary());
            let line = window
                .text_system()
                .shape_line(self.text.clone(), self.size, &[run], None);
            // Keep the caret in view, and never scroll past the end.
            let room = (width - caret_width).max(px(0.));
            let at = line.x_for_index(head);
            let mut scroll = caret.scroll;
            if at - scroll > room {
                scroll = at - room;
            }
            if at < scroll {
                scroll = at;
            }
            let most = (line.width + caret_width - width).max(px(0.));
            scroll = scroll.min(most).max(px(0.));
            caret.painted = Some((line.clone(), bounds.left() - scroll));
            (line, scroll)
        } else {
            let run = self.run(len, theme::text_primary());
            let (whole, shown) = self.fitted(self.text.clone(), run, width, window);
            caret.painted = Some((whole, bounds.left()));
            (shown, px(0.))
        };
        caret.scroll = scroll;
        let origin_x = bounds.left() - scroll;
        let top = bounds.top() + (self.size - px(CARET.1)) / 2.;
        let span = |from: Pixels, to: Pixels| {
            Bounds::from_corners(
                point(origin_x + from, top),
                point(origin_x + to, top + px(CARET.1)),
            )
        };
        let (selection, caret_box) = if !self.focused || self.text.is_empty() {
            let caret_box = (self.focused).then(|| span(px(0.), caret_width));
            (None, caret_box)
        } else if anchor != head {
            let (from, to) = (anchor.min(head), anchor.max(head));
            (
                Some(span(line.x_for_index(from), line.x_for_index(to))),
                None,
            )
        } else {
            let at = line.x_for_index(head);
            (None, Some(span(at, at + caret_width)))
        };
        Laid {
            line,
            origin_x,
            selection,
            caret: caret_box,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        laid: &mut Laid,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Glyphs reach past a line box as tall as the font: clip sideways
        // only.
        let slack = self.size * 0.3 + px(1.);
        let mut mask = bounds;
        mask.origin.y -= slack;
        mask.size.height += slack * 2.;
        window.with_content_mask(Some(ContentMask { bounds: mask }), |window| {
            if let Some(selection) = laid.selection {
                window.paint_quad(fill(selection, theme::accent_soft()));
            }
            let painted = laid.line.paint(
                point(laid.origin_x, bounds.top()),
                self.size,
                TextAlign::Left,
                None,
                window,
                cx,
            );
            if painted.is_err() {
                // Never the error: it can quote the field's text.
                log::warn!("a text field's line failed to paint");
            }
            if let Some(caret) = laid.caret {
                window.paint_quad(fill(caret, theme::accent()));
            }
        });

        // A drag that began with a press in the field follows the pointer
        // anywhere in the window, until the button comes up.
        if self.caret.borrow().drag.is_none() {
            return;
        }
        let caret = self.caret.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let mut caret = caret.borrow_mut();
            if event.pressed_button != Some(MouseButton::Left) {
                caret.drag = None;
                return;
            }
            let at = caret.index_at(event.position.x);
            match caret.drag.clone() {
                Some(Drag::Char) => caret.head = at,
                Some(Drag::Word(from)) => {
                    let word = word_at(&caret.painted_text(), at);
                    if word.start < from.start {
                        caret.anchor = from.end;
                        caret.head = word.start;
                    } else {
                        caret.anchor = from.start;
                        caret.head = word.end.max(from.end);
                    }
                }
                None => return,
            }
            window.refresh();
        });
        let caret = self.caret.clone();
        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, _| {
            if phase == DispatchPhase::Bubble {
                caret.borrow_mut().drag = None;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    /// Presses each of `keys` ("alt-left", "a") in `field`.
    fn press(field: &mut TextField, keys: &str, cx: &mut App) -> Edit {
        let mut last = Edit::Ignored;
        for key in keys.split_whitespace() {
            let mut keystroke = Keystroke::parse(key).expect("a keystroke");
            if keystroke.key.chars().count() == 1 && !keystroke.modifiers.platform {
                keystroke.key_char = Some(keystroke.key.clone());
            }
            last = field.key(&keystroke, cx);
        }
        last
    }

    #[gpui_kit::test]
    fn edits_happen_at_the_caret(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let mut field = TextField::new("mcp.linear.app/*", cx);
            assert_eq!(field.cursor(), 16);
            // To the start, then Delete trims the first label.
            assert_eq!(press(&mut field, "home", cx), Edit::Handled);
            assert_eq!(field.cursor(), 0);
            for _ in 0..4 {
                assert_eq!(press(&mut field, "delete", cx), Edit::Changed);
            }
            assert_eq!(field.text, "linear.app/*");
            // Typing inserts at the caret.
            press(&mut field, "right right x", cx);
            assert_eq!(field.text, "lixnear.app/*");
            assert_eq!(field.cursor(), 3);
            press(&mut field, "backspace", cx);
            assert_eq!(field.text, "linear.app/*");
            // ← with a selection collapses it to its start.
            press(&mut field, "shift-right shift-right left", cx);
            assert_eq!((field.cursor(), field.selection()), (2, 2..2));
            // Backspace and Delete at the ends change nothing.
            press(&mut field, "cmd-left", cx);
            assert_eq!(press(&mut field, "backspace", cx), Edit::Handled);
            press(&mut field, "end", cx);
            assert_eq!(press(&mut field, "delete", cx), Edit::Handled);
            assert_eq!(field.text, "linear.app/*");
        });
    }

    #[gpui_kit::test]
    fn words_and_lines(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let mut field = TextField::new("mcp.linear.app/*", cx);
            press(&mut field, "alt-left", cx);
            assert_eq!(field.cursor(), 11);
            press(&mut field, "alt-shift-left", cx);
            assert_eq!(field.selection(), 4..11);
            // As on macOS, the word and the dot after it.
            press(&mut field, "w w w .", cx);
            assert_eq!(field.text, "mcp.www.app/*");
            press(&mut field, "alt-backspace", cx);
            assert_eq!(field.text, "mcp.app/*");
            press(&mut field, "alt-delete", cx);
            assert_eq!(field.text, "mcp./*");
            press(&mut field, "cmd-backspace", cx);
            assert_eq!(field.text, "/*");
            press(&mut field, "cmd-delete", cx);
            assert_eq!(field.text, "");
        });
    }

    #[gpui_kit::test]
    fn select_all_copy_cut_and_paste(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let mut field = TextField::new("example.com", cx);
            assert_eq!(press(&mut field, "cmd-a", cx), Edit::Handled);
            assert!(field.all_selected());
            assert_eq!(press(&mut field, "x", cx), Edit::Changed);
            assert_eq!(field.text, "x");

            field.set("https://example.com/path");
            press(&mut field, "shift-home cmd-x", cx);
            assert_eq!(field.text, "");
            press(&mut field, "cmd-v cmd-v", cx);
            assert_eq!(
                field.text,
                "https://example.com/pathhttps://example.com/path"
            );
            // ⌘C with nothing selected leaves the clipboard alone.
            press(&mut field, "cmd-c", cx);
            assert_eq!(
                cx.read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref(),
                Some("https://example.com/path")
            );
            // Esc drops the selection, leaving the caret where it was.
            press(&mut field, "shift-left escape", cx);
            assert_eq!(field.selection(), 47..47);
        });
    }

    #[test]
    fn words_stop_at_url_punctuation() {
        let url = "mcp.linear.app/*";
        assert_eq!(prev_word(url, url.len()), 11);
        assert_eq!(prev_word(url, 11), 4);
        assert_eq!(prev_word(url, 4), 0);
        assert_eq!(next_word(url, 0), 3);
        assert_eq!(next_word(url, 3), 10);
        assert_eq!(next_word(url, 14), url.len());
        assert_eq!(word_at(url, 5), 4..10);
        assert_eq!(word_at(url, 3), 3..4);
        assert_eq!(word_at(url, url.len()), 15..16);
        assert_eq!(word_at("", 0), 0..0);
    }

    #[test]
    fn chars_step_over_multibyte_text() {
        let text = "a\u{e9}b";
        assert_eq!(next_char(text, 1), 3);
        assert_eq!(prev_char(text, 3), 1);
        assert_eq!(prev_char(text, 0), 0);
        assert_eq!(next_char(text, text.len()), text.len());
    }
}
