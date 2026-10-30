//! Shared accessibility semantics and keyboard focus for custom GPUI widgets.

use gpui_kit::{
    AnyElement, App, Div, ElementId, FocusHandle, InteractiveElement, Interactivity, IntoElement,
    ParentElement, RenderOnce, Role, SharedString, Stateful, StatefulInteractiveElement,
    StyleRefinement, Styled, Window,
};
use std::hash::{Hash, Hasher};

// Div helpers are assembled before their caller assigns an id. GPUI stores
// semantics in Interactivity; this adapter lets the helper fill them in while
// preserving its return type. The caller still must supply a stable id.
struct Semantic<E>(E);
impl<E: InteractiveElement> InteractiveElement for Semantic<E> {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.0.interactivity()
    }
}
impl<E: InteractiveElement> StatefulInteractiveElement for Semantic<E> {}

pub fn named<E: InteractiveElement>(element: E, role: Role, label: impl Into<SharedString>) -> E {
    Semantic(element).role(role).aria_label(label).0
}

pub fn selected<E: InteractiveElement>(
    element: E,
    label: impl Into<SharedString>,
    selected: bool,
) -> E {
    Semantic(element)
        .role(Role::RadioButton)
        .aria_label(label)
        .aria_selected(selected)
        .0
}

#[track_caller]
pub fn exposed(mut element: Div, role: Role, label: &str) -> Div {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    std::panic::Location::caller().hash(&mut hash);
    label.hash(&mut hash);
    element.interactivity().element_id = Some(ElementId::Integer(hash.finish()));
    Semantic(element).role(role).aria_label(label.to_owned()).0
}

#[track_caller]
pub fn heading(element: Div, label: &str) -> Div {
    Semantic(exposed(element, Role::Heading, label))
        .aria_level(2)
        .0
}

/// All custom click targets use GPUI's Return/Space synthetic click, so the
/// pointer and keyboard always run exactly the same callback. Text descendants
/// supply the default accessible name; icon-only controls supply an aria_label.
pub fn clickable<E: StatefulInteractiveElement>(element: E) -> E {
    crate::widgets::keyboard_control(element.role(Role::Button), false)
        .focus_visible(|style| style.border_1().border_color(crate::theme::focus_ring()))
}

/// A real AccessKit Label, including for text assembled by the app's helpers.
/// Parent element ids disambiguate repeated rows; content and call site keep
/// separate labels within one row separate without a global counter.
#[track_caller]
pub fn label(content: impl Into<SharedString>) -> gpui_kit::Text {
    let content = content.into();
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    std::panic::Location::caller().hash(&mut hash);
    content.hash(&mut hash);
    gpui_kit::Text::new(ElementId::Integer(hash.finish()), content)
}

/// A menu with a scoped Tab order and wrapping arrow navigation. State exists
/// only while the card is rendered, so reopening always records the new trigger.
#[derive(IntoElement)]
pub struct MenuCard {
    pub card: Stateful<Div>,
    id: ElementId,
    navigation: bool,
    dialog: bool,
    initial: Option<FocusHandle>,
}

impl MenuCard {
    pub fn existing_navigation(mut self) -> Self {
        self.navigation = false;
        self
    }
    pub fn dialog(id: impl Into<ElementId>, card: Stateful<Div>, initial: FocusHandle) -> Self {
        Self {
            card,
            id: id.into(),
            navigation: true,
            dialog: true,
            initial: Some(initial),
        }
    }
    pub fn new(id: ElementId, card: Stateful<Div>) -> Self {
        Self {
            card,
            id,
            navigation: true,
            dialog: false,
            initial: None,
        }
    }
}
impl Styled for MenuCard {
    fn style(&mut self) -> &mut StyleRefinement {
        self.card.style()
    }
}
impl InteractiveElement for MenuCard {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.card.interactivity()
    }
}
impl StatefulInteractiveElement for MenuCard {}
impl ParentElement for MenuCard {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.card.extend(elements);
    }
}

struct MenuFocus {
    scope: FocusHandle,
    trigger: Option<FocusHandle>,
}

/// Focus the first or last descendant, or wrap a single step. The temporary
/// move outside is synchronous: no frame or accessibility event observes it.
pub(crate) fn move_focus(scope: &FocusHandle, key: &str, window: &mut Window, cx: &mut App) {
    match key {
        "home" => {
            window.focus(scope, cx);
            window.focus_next(cx);
            if !scope.contains_focused(window, cx) {
                window.focus(scope, cx);
            }
        }
        "end" => {
            window.focus(scope, cx);
            let mut last = scope.clone();
            let mut first = None;
            loop {
                window.focus_next(cx);
                if !scope.contains_focused(window, cx) || scope.is_focused(window) {
                    break;
                }
                let Some(next) = window.focused(cx) else {
                    break;
                };
                if first.as_ref() == Some(&next) || next == last {
                    break;
                }
                if first.is_none() {
                    first = Some(next.clone());
                }
                last = next;
            }
            window.focus(&last, cx);
        }
        "up" | "shift-tab" => {
            window.focus_prev(cx);
            if !scope.contains_focused(window, cx) || scope.is_focused(window) {
                move_focus(scope, "end", window, cx);
            }
        }
        _ => {
            window.focus_next(cx);
            if !scope.contains_focused(window, cx) || scope.is_focused(window) {
                move_focus(scope, "home", window, cx);
            }
        }
    }
}

impl RenderOnce for MenuCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if !self.navigation {
            return self.card.role(Role::Menu).aria_label("Choices");
        }
        let dialog = self.dialog;
        let initial = self.initial;
        let state = window.use_keyed_state(self.id.clone(), cx, |window, cx| {
            let scope = cx.focus_handle();
            let trigger = window.focused(cx);
            let first = scope.clone();
            window.defer(cx, move |window, cx| {
                if let Some(initial) = initial {
                    window.focus(&initial, cx);
                } else {
                    move_focus(&first, "home", window, cx);
                }
            });
            let handle = window.window_handle();
            cx.on_release(move |state: &mut MenuFocus, cx| {
                let scope = state.scope.clone();
                let trigger = state.trigger.clone();
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        if (scope.contains_focused(window, cx) || window.focused(cx).is_none())
                            && let Some(trigger) = trigger
                        {
                            window.focus(&trigger, cx);
                        }
                    });
                });
            })
            .detach();
            MenuFocus { scope, trigger }
        });
        let scope = state.read(cx).scope.clone();
        let trigger = state.read(cx).trigger.clone();
        self.card
            .role(if dialog { Role::Dialog } else { Role::Menu })
            .aria_label(if dialog { "Add a rule" } else { "Choices" })
            .key_context("PwrRoutePopover")
            .tab_group()
            .track_focus(&scope)
            .on_action({
                let scope = scope.clone();
                move |_: &crate::app_menu::FocusNext, window, cx| {
                    move_focus(&scope, "down", window, cx);
                    cx.stop_propagation();
                }
            })
            .on_action({
                let scope = scope.clone();
                move |_: &crate::app_menu::FocusPrevious, window, cx| {
                    move_focus(&scope, "shift-tab", window, cx);
                    cx.stop_propagation();
                }
            })
            .on_key_down(move |event, window, cx| {
                let key = event.keystroke.key.as_str();
                if !dialog && matches!(key, "up" | "down" | "home" | "end") {
                    move_focus(&scope, key, window, cx);
                    cx.stop_propagation();
                } else if key == "escape"
                    && let Some(trigger) = &trigger
                {
                    window.focus(trigger, cx);
                    // The owner dismisses its model in the bubbling handler.
                }
            })
    }
}
