//! Native input and AccessKit actions use the same app-owned field.
use super::*;
use gpui_kit::accesskit::{Action, ActionData, Node, Role, TextPosition, TextSelection};
use gpui_kit::{
    AppContext as _, Context, ElementInputHandler, EntityInputHandler, UTF16Selection, WeakEntity,
};

/// A window resolves a field by its stable focus handle and applies the
/// same model changes as keyboard edits. No second text buffer is kept here.
pub trait InputOwner: Sized + 'static {
    fn input_field(&mut self, focus: &FocusHandle) -> Option<&mut TextField>;
    fn input_changed(&mut self, focus: &FocusHandle, cx: &mut Context<'_, Self>);
}

struct NativeInput<V: InputOwner> {
    owner: WeakEntity<V>,
    focus: FocusHandle,
}

impl<V: InputOwner> NativeInput<V> {
    fn read<R>(&self, cx: &mut App, f: impl FnOnce(&mut TextField) -> R) -> Option<R> {
        self.owner
            .update(cx, |owner, _| owner.input_field(&self.focus).map(f))
            .ok()
            .flatten()
    }

    fn edit(&self, cx: &mut App, changed: bool, f: impl FnOnce(&mut TextField)) {
        let _ = self.owner.update(cx, |owner, cx| {
            if let Some(field) = owner.input_field(&self.focus) {
                f(field);
                if changed {
                    owner.input_changed(&self.focus, cx);
                }
                cx.notify();
            }
        });
    }
}

impl TextField {
    fn utf16(&self, at: usize) -> usize {
        self.text[..self.clamp(at)].encode_utf16().count()
    }

    fn byte(&self, at: usize) -> usize {
        byte_from_utf16(&self.text, at)
    }

    fn bytes(&self, range: Range<usize>) -> Range<usize> {
        let start = self.byte(range.start);
        start..self.byte(range.end).max(start)
    }

    pub(super) fn native_replace(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        mark: bool,
    ) {
        let range = range
            .map(|r| self.bytes(r))
            .or(self.marked.clone())
            .unwrap_or_else(|| self.selection());
        let text = single_line(text);
        self.replace(range.clone(), &text);
        if mark {
            self.marked = (!text.is_empty()).then_some(range.start..range.start + text.len());
            if let Some(selected) = selected {
                self.select(
                    range.start + byte_from_utf16(&text, selected.start),
                    range.start + byte_from_utf16(&text, selected.end),
                );
            }
        }
    }
}

fn byte_from_utf16(text: &str, at: usize) -> usize {
    let mut offset = 0;
    for (byte, c) in text.char_indices() {
        if offset + c.len_utf16() > at {
            return byte;
        }
        offset += c.len_utf16();
    }
    text.len()
}

fn single_line(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .collect()
}

impl<V: InputOwner> EntityInputHandler for NativeInput<V> {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<String> {
        self.read(cx, |f| {
            let range = f.bytes(range);
            *adjusted = Some(f.utf16(range.start)..f.utf16(range.end));
            f.text[range].to_owned()
        })
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<UTF16Selection> {
        self.read(cx, |f| {
            let range = f.selection();
            let caret = f.caret.borrow();
            UTF16Selection {
                range: f.utf16(range.start)..f.utf16(range.end),
                reversed: caret.head < caret.anchor,
            }
        })
    }
    fn marked_text_range(
        &self,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<Range<usize>> {
        self.read(cx, |f| {
            f.marked.as_ref().map(|r| f.utf16(r.start)..f.utf16(r.end))
        })
        .flatten()
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) {
        self.edit(cx, false, |f| f.marked = None);
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.edit(cx, true, |f| f.native_replace(range, text, None, false));
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.edit(cx, true, |f| f.native_replace(range, text, selected, true));
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<Bounds<Pixels>> {
        self.read(cx, |f| {
            let range = f.bytes(range);
            let caret = f.caret.borrow();
            let x = |at| {
                caret
                    .painted
                    .as_ref()
                    .map_or(bounds.left(), |(line, start)| *start + line.x_for_index(at))
            };
            Bounds::from_corners(
                point(x(range.start), bounds.top()),
                point(x(range.end).max(x(range.start) + px(1.)), bounds.bottom()),
            )
        })
    }
    fn character_index_for_point(
        &mut self,
        point: gpui_kit::Point<Pixels>,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<usize> {
        self.read(cx, |f| f.utf16(f.caret.borrow().index_at(point.x)))
    }
    fn set_selected_text_range(
        &mut self,
        range: Range<usize>,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.edit(cx, false, |f| {
            let range = f.bytes(range);
            f.select(range.start, range.end);
        });
    }
    fn text_length_utf16(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> Option<usize> {
        self.read(cx, |f| f.text.encode_utf16().count())
    }
    fn text_input_editable_range(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Option<Range<usize>> {
        self.text_length_utf16(window, cx).map(|len| 0..len)
    }
}

impl FieldText {
    /// Register the platform input handler while this field is painted.
    pub fn input<V: InputOwner>(mut self, field: &TextField, cx: &mut Context<'_, V>) -> Self {
        if !self.focused {
            return self;
        }
        let owner = cx.entity().downgrade();
        let focus = field.focus.clone();
        let input = cx.new(|_| NativeInput {
            owner,
            focus: focus.clone(),
        });
        self.input = Some(Box::new(move |bounds, window, cx| {
            window.handle_input(&focus, ElementInputHandler::new(bounds, input), cx);
        }));
        self
    }
}

/// Native actions are bound to this particular field, even when it is not
/// focused when the assistive tool sends the request.
pub fn native_area<V: InputOwner>(
    element: Stateful<Div>,
    field: &TextField,
    cx: &mut Context<'_, V>,
) -> Stateful<Div> {
    let owner = cx.entity().downgrade();
    let focus = field.focus.clone();
    let value_owner = owner.clone();
    let value_focus = focus.clone();
    hit_area(element, field)
        .on_a11y_action(Action::SetValue, move |data, window, cx| {
            let Some(ActionData::Value(value)) = data else {
                return;
            };
            window.focus(&value_focus, cx);
            let _ = value_owner.update(cx, |owner, cx| {
                if let Some(field) = owner.input_field(&value_focus) {
                    field.set(single_line(value));
                    owner.input_changed(&value_focus, cx);
                    cx.notify();
                }
            });
        })
        .on_a11y_action(Action::SetTextSelection, move |data, window, cx| {
            let Some(ActionData::SetTextSelection(selection)) = data else {
                return;
            };
            window.focus(&focus, cx);
            let _ = owner.update(cx, |owner, cx| {
                if let Some(field) = owner.input_field(&focus) {
                    let byte = |position: &TextPosition| {
                        let caret = field.caret.borrow();
                        let (_, range) = caret.runs.iter().find(|(id, _)| *id == position.node)?;
                        let run = field.text.get(range.clone())?;
                        Some(
                            range.start
                                + run
                                    .char_indices()
                                    .nth(position.character_index)
                                    .map_or(run.len(), |(i, _)| i),
                        )
                    };
                    if let (Some(anchor), Some(head)) =
                        (byte(&selection.anchor), byte(&selection.focus))
                    {
                        field.select(anchor, head);
                    }
                    cx.notify();
                }
            });
        })
}

pub(super) fn text_nodes(element: Stateful<Div>, field: &TextField) -> Stateful<Div> {
    let text = field.text.clone();
    let caret = field.caret.clone();
    element.a11y_synthetic_children(move |builder| {
        let mut caret = caret.borrow_mut();
        caret.runs.clear();
        let boundaries: Vec<_> = text
            .char_indices()
            .map(|(i, _)| i)
            .chain([text.len()])
            .collect();
        // AccessKit's word-start indices are u8, so each run has at most
        // 255 selectable characters. Empty fields still have a caret run.
        let count = boundaries.len() - 1;
        for start in (0..count.max(1)).step_by(255) {
            let end = (start + 255).min(count);
            let bytes = boundaries[start]..boundaries[end];
            let value = &text[bytes.clone()];
            let id = builder.synthetic_node_id(("text", start));
            let mut run = Node::new(Role::TextRun);
            run.set_value(value.to_owned());
            run.set_character_lengths(
                value
                    .chars()
                    .map(|c| c.len_utf8() as u8)
                    .collect::<Vec<_>>(),
            );
            let mut previous = text[..bytes.start].chars().next_back().map(is_word);
            let starts: Vec<_> = value
                .chars()
                .enumerate()
                .filter_map(|(i, c)| {
                    let word = is_word(c);
                    let begins = previous.is_none_or(|prev| prev != word);
                    previous = Some(word);
                    begins.then_some(i as u8)
                })
                .collect();
            run.set_word_starts(starts);
            run.set_text_direction(gpui_kit::accesskit::TextDirection::LeftToRight);
            if let Some(bounds) = caret.bounds {
                let scale = f64::from(caret.scale);
                let x = |byte| {
                    caret
                        .painted
                        .as_ref()
                        .map_or(bounds.left(), |(line, origin)| {
                            *origin + line.x_for_index(byte)
                        })
                };
                let left = x(bytes.start);
                run.set_bounds(gpui_kit::accesskit::Rect {
                    x0: f64::from(left) * scale,
                    y0: f64::from(bounds.top()) * scale,
                    x1: f64::from(x(bytes.end).max(left + px(1.))) * scale,
                    y1: f64::from(bounds.bottom()) * scale,
                });
                run.set_character_positions(
                    value
                        .char_indices()
                        .map(|(i, _)| f32::from(x(bytes.start + i) - left) * caret.scale)
                        .collect::<Vec<_>>(),
                );
                run.set_character_widths(
                    value
                        .char_indices()
                        .map(|(i, c)| {
                            f32::from(x(bytes.start + i + c.len_utf8()) - x(bytes.start + i))
                                * caret.scale
                        })
                        .collect::<Vec<_>>(),
                );
            }
            builder.push_child(id, run);
            caret.runs.push((id, bytes));
        }
        let position = |byte: usize| {
            let byte = byte.min(text.len());
            let (id, range) = caret
                .runs
                .iter()
                .find(|(_, r)| byte < r.end)
                .unwrap_or_else(|| caret.runs.last().unwrap());
            TextPosition {
                node: *id,
                character_index: text[range.start..byte].chars().count(),
            }
        };
        builder.parent_node().set_text_selection(TextSelection {
            anchor: position(caret.anchor),
            focus: position(caret.head),
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{Render, TestAppContext};

    struct Editor {
        field: TextField,
        changes: usize,
    }
    impl InputOwner for Editor {
        fn input_field(&mut self, focus: &FocusHandle) -> Option<&mut TextField> {
            (self.field.focus == *focus).then_some(&mut self.field)
        }
        fn input_changed(&mut self, _: &FocusHandle, _: &mut Context<'_, Self>) {
            self.changes += 1;
        }
    }
    impl Render for Editor {
        fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            super::super::field(
                "native-field",
                &self.field,
                Look {
                    mono: false,
                    placeholder: "Link",
                },
                None,
                window,
                cx,
            )
            .on_key_down(cx.listener(|this, event: &gpui_kit::KeyDownEvent, _, cx| {
                if this.field.key(&event.keystroke, cx) != Edit::Ignored {
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
        }
    }

    #[gpui_kit::test]
    fn platform_input_inserts_once_and_keeps_selection_editing(cx: &mut TestAppContext) {
        cx.update(|cx| crate::assets::load_fonts(cx));
        let (view, cx) = cx.add_window_view(|window, cx| {
            let field = TextField::new("example.com", cx);
            window.focus(&field.focus, cx);
            Editor { field, changes: 0 }
        });
        cx.simulate_input("/path");
        view.read_with(cx, |view, _| {
            assert_eq!(view.field.text, "example.com/path");
            assert!(view.changes > 0);
        });
        cx.simulate_keystrokes("shift-home");
        cx.simulate_input("https://example.org");
        view.read_with(cx, |view, _| {
            assert_eq!(view.field.text, "https://example.org")
        });
    }

    #[gpui_kit::test]
    fn ime_replaces_marked_text_and_dictation_replaces_utf16_ranges(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| Editor {
            field: TextField::new("a😀z", cx),
            changes: 0,
        });
        cx.update(|window, cx| {
            let focus = view.read(cx).field.focus.clone();
            let input = cx.new(|_| NativeInput {
                owner: view.downgrade(),
                focus,
            });
            input.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(Some(1..3), "に", Some(0..1), window, cx);
                assert_eq!(input.marked_text_range(window, cx), Some(1..2));
                assert_eq!(
                    input.selected_text_range(false, window, cx).unwrap().range,
                    1..2
                );
                input.replace_and_mark_text_in_range(None, "日本", Some(2..2), window, cx);
                assert_eq!(input.marked_text_range(window, cx), Some(1..3));
                assert_eq!(
                    input.selected_text_range(false, window, cx).unwrap().range,
                    3..3
                );
                input.replace_text_in_range(None, "日本語", window, cx);
                assert_eq!(input.marked_text_range(window, cx), None);
                assert_eq!(input.text_length_utf16(window, cx), Some(5));
                // A dictation-style replacement is multi-character input,
                // delivered directly by the platform input handler.
                input.replace_text_in_range(Some(0..5), "spoken link", window, cx);
                input.set_selected_text_range(0..6, window, cx);
                let mut adjusted = None;
                assert_eq!(
                    input
                        .text_for_range(0..6, &mut adjusted, window, cx)
                        .as_deref(),
                    Some("spoken")
                );
                assert_eq!(adjusted, Some(0..6));
                input.unmark_text(window, cx);
            });
            assert_eq!(view.read(cx).field.text, "spoken link");
            assert_eq!(view.read(cx).changes, 4);
        });
    }

    #[gpui_kit::test]
    fn utf16_ranges_clamp_surrogate_pairs_and_keep_reversed_selection(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let mut field = TextField::new("a😀é", cx);
            assert_eq!(field.bytes(2..usize::MAX), 1..7);
            assert_eq!(field.utf16(5), 3);
            field.select(7, 1);
            assert_eq!(field.selection(), 1..7);
            field.native_replace(Some(2..3), "x\nignored", None, false);
            assert_eq!(field.text, "axé");
            assert_eq!(field.cursor(), 2);
        });
    }
}
