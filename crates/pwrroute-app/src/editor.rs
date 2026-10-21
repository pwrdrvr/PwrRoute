//! The right rail: the editor for the selected rewrite or handler.
//!
//! Every field writes to the store as it is typed (`PwrRoute::rail_key`),
//! and the store writes the file after a short debounce. The design draws
//! the handler editor (Rule Editing 1a) and the rewrite editor (2a); a draft
//! has a chip where a saved rule has its switch, and a rule with problems
//! shows them under the fields they belong to (5a, 5b). Open in carries
//! "Open in a private window" and Keep (Activity Privacy 3b), and every
//! rule has an Examples section: the URLs that must resolve to it, each with
//! its result from the last test run, and a field that adds one. An example
//! is the one way a link reaches `rules.json` from this rail, on Return, and
//! the help under the field says so.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, HighlightStyle, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, StyledText, Window, deferred, div, px,
};
use pwrroute_engine::{Privacy, TestSource};

use crate::app::{PwrRoute, Selection};
use crate::rules_model::{
    Destination, Handler, KEEP_LEVELS, KINDS, Rewrite, RowBadge, Rule, kind_of,
};
use crate::rules_view::{ChipTone, chip_in, example_mark, winner_label};
use crate::text_field::{Edit, TextField};
use crate::theme::{self, mono, text, tracked};
use crate::trace_view::{UrlMark, url_diff};
use crate::try_it::{TryResult, Verdict, sample_url};
use crate::widgets::{
    ButtonKind, badge, button, caret, focus_ring, input_box, segment, segmented, switch,
};

/// The rail's width.
pub const WIDTH: f32 = 372.0;

/// One of the rail's text fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Pattern(usize),
    /// An app the handler requires the link to come from.
    Opener(usize),
    Browser,
    Profile,
    /// The value of a rewrite's change.
    Value(usize),
    /// Try it's link.
    Sample,
    /// A URL to add as the rule's example.
    Example,
    /// For a rewrite's example: what the URL must become.
    Becomes,
}

/// The editor's fields for the selected rule. Built when the selection
/// changes; kept while typing, so the focus stays where it is.
pub struct Rail {
    pub patterns: Vec<TextField>,
    pub openers: Vec<TextField>,
    pub browser: TextField,
    pub profile: TextField,
    pub values: Vec<TextField>,
    pub sample: TextField,
    /// The example to add, and (for a rewrite) what it must become.
    pub example: TextField,
    pub becomes: TextField,
    /// The sample was typed: it no longer follows the first pattern.
    pub sample_edited: bool,
    /// Which change's kind menu is open.
    pub picking_kind: Option<usize>,
    /// The menu that was open when a kind box was pressed; see
    /// `test_link`'s opener picker.
    was_picking: Option<usize>,
    pub try_result: Option<TryResult>,
}

impl Rail {
    pub fn for_handler(handler: &Handler, cx: &mut App) -> Self {
        Self {
            patterns: fields(&handler.patterns, cx),
            openers: fields(&handler.openers, cx),
            browser: TextField::new(handler.browser.clone(), cx),
            profile: TextField::new(handler.profile.clone(), cx),
            values: Vec::new(),
            sample: TextField::new(sample_url(&handler.patterns), cx),
            example: TextField::new("", cx),
            becomes: TextField::new("", cx),
            sample_edited: false,
            picking_kind: None,
            was_picking: None,
            try_result: None,
        }
    }

    pub fn for_rewrite(rewrite: &Rewrite, cx: &mut App) -> Self {
        let values: Vec<String> = rewrite
            .actions
            .iter()
            .map(crate::rules_model::value_of)
            .collect();
        Self {
            patterns: fields(&rewrite.patterns, cx),
            openers: Vec::new(),
            browser: TextField::new("", cx),
            profile: TextField::new("", cx),
            values: fields(&values, cx),
            sample: TextField::new(sample_url(&rewrite.patterns), cx),
            example: TextField::new("", cx),
            becomes: TextField::new("", cx),
            sample_edited: false,
            picking_kind: None,
            was_picking: None,
            try_result: None,
        }
    }

    pub fn field_mut(&mut self, field: Field) -> Option<&mut TextField> {
        match field {
            Field::Pattern(i) => self.patterns.get_mut(i),
            Field::Opener(i) => self.openers.get_mut(i),
            Field::Browser => Some(&mut self.browser),
            Field::Profile => Some(&mut self.profile),
            Field::Value(i) => self.values.get_mut(i),
            Field::Sample => Some(&mut self.sample),
            Field::Example => Some(&mut self.example),
            Field::Becomes => Some(&mut self.becomes),
        }
    }

    fn field(&self, field: Field) -> Option<&TextField> {
        match field {
            Field::Pattern(i) => self.patterns.get(i),
            Field::Opener(i) => self.openers.get(i),
            Field::Browser => Some(&self.browser),
            Field::Profile => Some(&self.profile),
            Field::Value(i) => self.values.get(i),
            Field::Sample => Some(&self.sample),
            Field::Example => Some(&self.example),
            Field::Becomes => Some(&self.becomes),
        }
    }
}

fn fields(texts: &[String], cx: &mut App) -> Vec<TextField> {
    texts
        .iter()
        .map(|text| TextField::new(text.clone(), cx))
        .collect()
}

/// `.rs`: one section of the rail.
fn section() -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .py(px(14.0))
        .px(px(16.0))
        .border_b_1()
        .border_color(theme::border_faint())
}

/// `.rs h5`.
fn section_title(title: &str) -> Div {
    div()
        .mb(px(2.0))
        .child(tracked(title, 9.5, 600.0, 0.14, theme::text_muted()))
}

/// `.lab`.
fn label(content: &'static str) -> Div {
    text(content, 11.0, 600.0, theme::text_secondary()).line_height(px(14.3))
}

/// `.lrow`: a label on the left, its control on the right.
fn labeled_row(content: &'static str, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(10.0))
        .child(label(content))
        .child(control)
}

/// A wrapping paragraph of subtle help text.
fn paragraph(content: impl Into<SharedString>, size: f32, line_height: f32) -> Div {
    crate::widgets::paragraph(content, size, line_height, theme::text_subtle())
}

/// `.err-t`: a problem under the field it belongs to.
fn problem(message: &str) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(6.0))
        .child(
            div()
                .flex_none()
                .size(px(14.0))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::danger_soft())
                .child(mono("!", 9.0, 700.0, theme::danger_text())),
        )
        .child(crate::widgets::paragraph(
            message.to_owned(),
            10.5,
            15.2,
            theme::danger_text(),
        ))
}

/// Where a problem's message belongs in the rail, by its words.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Under {
    Patterns,
    Openers,
    Browser,
    Changes,
    /// The Keep control: a privacy word the engine does not know.
    Keep,
    Examples,
}

fn under(message: &str) -> Under {
    if message.contains("pattern") {
        Under::Patterns
    } else if message.contains("App names") {
        Under::Openers
    } else if message.starts_with("Privacy") {
        Under::Keep
    } else if message.starts_with("Examples") {
        Under::Examples
    } else if message.contains("browser") || message.contains("PwrRoute") {
        Under::Browser
    } else {
        Under::Changes
    }
}

fn problems_under(problems: &[String], place: Under) -> Vec<Div> {
    problems
        .iter()
        .filter(|m| under(m) == place)
        .map(|m| problem(m))
        .collect()
}

/// The rail's head: which rule of how many, its title, and its switch (or
/// its draft chip).
fn header(eyebrow: &str, title: SharedString, rule: &Rule, cx: &mut Context<'_, PwrRoute>) -> Div {
    let state: AnyElement = if rule.badge == RowBadge::Draft {
        chip_in("draft", ChipTone::Accent).into_any_element()
    } else {
        div()
            .id("handler-enabled")
            .debug_selector(|| "handler-enabled".into())
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| this.toggle_selected(cx)))
            .child(switch(rule.enabled))
            .into_any_element()
    };
    section()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(tracked(eyebrow, 9.5, 600.0, 0.14, theme::text_muted()))
                .child(
                    crate::widgets::one_line(
                        title,
                        theme::SANS,
                        700.0,
                        15.0,
                        18.0,
                        theme::text_primary(),
                    )
                    .mt(px(6.0)),
                ),
        )
        .child(state)
}

/// How a field draws its text.
#[derive(Clone, Copy)]
struct Look {
    mono: bool,
    placeholder: &'static str,
    /// `.inp.err`: the field has a problem.
    error: bool,
}

/// `.inp` for one of the rail's fields: its text, a caret while focused and
/// the focus ring; typing goes to `PwrRoute::rail_key`. Trailing content (a
/// × to remove the field) goes in `trailing`. The same shape as
/// `text_field::field`, with an id per field.
fn editable(
    rail: &Rail,
    field: Field,
    selector: String,
    look: Look,
    trailing: Option<Div>,
    window: &Window,
    cx: &mut Context<'_, PwrRoute>,
) -> Stateful<Div> {
    let Some(text_field) = rail.field(field) else {
        return div().id(ElementId::Name(selector.into()));
    };
    let focused = text_field.focus.is_focused(window);
    let (family, weight, size) = if look.mono {
        (theme::mono_family(500.0), 500.0, 12.0)
    } else {
        (theme::SANS, 400.0, 12.5)
    };
    let content = if text_field.text.is_empty() {
        div()
            .text_color(theme::text_subtle())
            .child(look.placeholder)
    } else {
        div()
            .text_color(theme::text_primary())
            .when(text_field.selected, |text| text.bg(theme::accent_soft()))
            .child(text_field.shared())
    };
    let content = content
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
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
    let caret_bar = || div().flex_none().w(px(1.0)).h(px(14.0)).bg(theme::accent());
    let mut inner = input_box().w_full().gap(px(8.0)).when(look.error, |input| {
        input.border_color(theme::status_error())
    });
    if text_field.text.is_empty() && focused {
        inner = inner.child(caret_bar());
    }
    inner = inner.child(content);
    if !text_field.text.is_empty() && focused && !text_field.selected {
        inner = inner.child(caret_bar().ml(px(-8.0)));
    }
    inner = inner.when_some(trailing, |input, trailing| {
        input.pr(px(6.0)).child(div().ml_auto().child(trailing))
    });
    let boxed = if focused { focus_ring(inner) } else { inner };
    let focus = text_field.focus.clone();
    let id: SharedString = selector.into();
    div()
        .id(ElementId::Name(id.clone()))
        .debug_selector(move || id.to_string())
        .track_focus(&focus)
        .w_full()
        .min_w_0()
        .cursor_text()
        .child(boxed)
        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
            let taken = match field {
                Field::Example | Field::Becomes => this.example_key(field, event, cx),
                _ => this.rail_key(field, event, cx),
            };
            if taken {
                cx.stop_propagation();
                cx.notify();
            }
        }))
        .on_click(cx.listener(move |this, _, window, cx| {
            if let Some(focus) = this
                .rail
                .as_ref()
                .and_then(|rail| rail.field(field))
                .map(|f| f.focus.clone())
            {
                window.focus(&focus, cx);
                cx.notify();
            }
        }))
}

/// `.inp .x`: removes the field it sits in.
fn remove_button(
    selector: String,
    on_click: impl Fn(&mut PwrRoute, &mut Window, &mut Context<'_, PwrRoute>) + 'static,
    cx: &mut Context<'_, PwrRoute>,
) -> Div {
    let id: SharedString = selector.into();
    div().child(
        div()
            .id(ElementId::Name(id.clone()))
            .debug_selector(move || id.to_string())
            .cursor_pointer()
            .px(px(2.0))
            .child(mono("\u{00d7}", 11.0, 500.0, theme::text_subtle()))
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx))),
    )
}

/// `.addp`: the dashed way to add a pattern, an app or a change.
fn add_button(
    id: &'static str,
    label: &'static str,
    hint: Option<&'static str>,
    on_click: impl Fn(&mut PwrRoute, &mut Window, &mut Context<'_, PwrRoute>) + 'static,
    cx: &mut Context<'_, PwrRoute>,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(|| id.into())
        .flex()
        .flex_none()
        .self_start()
        .items_center()
        .gap(px(6.0))
        .py(px(5.0))
        .px(px(8.0))
        .border_1()
        .border_dashed()
        .border_color(theme::border_default())
        .rounded(px(7.0))
        .cursor_pointer()
        .hover(|style| style.bg(theme::bg_hover()))
        .child(text(label, 11.0, 500.0, theme::text_muted()))
        .when_some(hint, |add, hint| {
            add.child(
                text(hint, 10.5, 400.0, theme::text_subtle())
                    .line_height(px(15.75))
                    .ml(px(4.0)),
            )
        })
        .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
}

/// `.plist` and `.addp`: the URL patterns and the way to add one.
fn pattern_list(
    rail: &Rail,
    problems: &[String],
    window: &Window,
    cx: &mut Context<'_, PwrRoute>,
) -> Vec<AnyElement> {
    let list =
        (0..rail.patterns.len()).fold(div().flex().flex_col().gap(px(5.0)), |list, index| {
            let remove = remove_button(
                format!("pattern-remove-{index}"),
                move |this, window, cx| this.remove_pattern(index, window, cx),
                cx,
            );
            list.child(editable(
                rail,
                Field::Pattern(index),
                format!("pattern-{index}"),
                Look {
                    mono: true,
                    placeholder: "example.com/*",
                    error: rail.patterns[index].text.trim().is_empty(),
                },
                Some(remove),
                window,
                cx,
            ))
        });
    let mut rows = vec![
        label("URL patterns").into_any_element(),
        list.into_any_element(),
    ];
    rows.extend(
        problems_under(problems, Under::Patterns)
            .into_iter()
            .map(IntoElement::into_any_element),
    );
    rows.push(
        add_button(
            "pattern-add",
            "+ pattern",
            Some("wildcard, or /regex/"),
            |this, window, cx| this.add_pattern(window, cx),
            cx,
        )
        .into_any_element(),
    );
    rows
}

fn match_section(
    handler: &Handler,
    rail: &Rail,
    window: &Window,
    cx: &mut Context<'_, PwrRoute>,
) -> Div {
    let openers =
        (0..rail.openers.len()).fold(div().flex().flex_col().gap(px(5.0)), |list, index| {
            let remove = remove_button(
                format!("opener-remove-{index}"),
                move |this, window, cx| this.remove_opener(index, window, cx),
                cx,
            );
            list.child(editable(
                rail,
                Field::Opener(index),
                format!("opener-{index}"),
                Look {
                    mono: true,
                    placeholder: "App name or bundle id",
                    error: rail.openers[index].text.trim().is_empty(),
                },
                Some(remove),
                window,
                cx,
            ))
        });
    section()
        .child(section_title("Match"))
        .children(pattern_list(rail, &handler.problems, window, cx))
        .child(label("Sent from").mt(px(6.0)))
        .when(!rail.openers.is_empty(), |section| section.child(openers))
        .children(problems_under(&handler.problems, Under::Openers))
        .child(add_button(
            "opener-add",
            "+ app",
            None,
            |this, window, cx| this.add_opener(window, cx),
            cx,
        ))
        .child(paragraph(
            "Optional. Leave empty to match a link from any app. The app that handed the link \
             to the OS, by name or bundle id.",
            10.5,
            15.75,
        ))
}

fn open_in_section(
    handler: &Handler,
    rail: &Rail,
    window: &Window,
    cx: &mut Context<'_, PwrRoute>,
) -> Div {
    let browser_badge = match &handler.rule.destination {
        Destination::Browser { badge, .. } => Some(badge.clone()),
        _ => None,
    };
    let browser = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .when_some(browser_badge, |row, label| row.child(badge(label)))
        .child(editable(
            rail,
            Field::Browser,
            "browser".into(),
            Look {
                mono: false,
                placeholder: "Choose a browser or app",
                error: handler.problems.iter().any(|m| under(m) == Under::Browser),
            },
            None,
            window,
            cx,
        ));
    let profile = div().w(px(190.0)).flex_none().child(editable(
        rail,
        Field::Profile,
        "profile".into(),
        Look {
            mono: false,
            placeholder: "Default",
            error: false,
        },
        None,
        window,
        cx,
    ));
    let private = div()
        .id("handler-private")
        .debug_selector(|| "handler-private".into())
        .cursor_pointer()
        .child(switch(handler.private))
        .on_click(cx.listener(|this, _, _, cx| this.toggle_private(cx)));
    let keep = handler.keep();
    let keep_control = KEEP_LEVELS.iter().fold(segmented(), |seg, (level, label)| {
        let level = *level;
        seg.child(
            segment(*label, keep == Some(level))
                .id(ElementId::Name(format!("keep-{}", level_key(level)).into()))
                .debug_selector(move || format!("keep-{}", level_key(level)))
                .on_click(cx.listener(move |this, _, _, cx| this.set_keep(level, cx))),
        )
    });
    section()
        .child(section_title("Open in"))
        .child(browser)
        .children(problems_under(&handler.problems, Under::Browser))
        .child(paragraph(
            "A browser or app by name, bundle id or path.",
            10.5,
            15.75,
        ))
        .child(labeled_row("Profile", profile))
        .child(labeled_row("Open in a private window", private).mt(px(4.0)))
        .child(labeled_row("Keep", keep_control))
        .children(problems_under(&handler.problems, Under::Keep))
        .child(paragraph(
            "What Activity and the journal remember about links this handler takes. A \
             private window keeps host only unless set here.",
            10.5,
            15.75,
        ))
        // rules.json has no field for these yet; drawn as the design shows
        // them, not wired.
        .child(labeled_row("Open in background", switch(false)).opacity(0.5))
        .child(
            labeled_row(
                "Extra arguments",
                input_box()
                    .w(px(190.0))
                    .flex_none()
                    .child(mono("--new-window", 12.0, 500.0, theme::text_subtle())),
            )
            .opacity(0.5),
        )
}

/// A Keep level as an element id and selector suffix.
fn level_key(level: Privacy) -> &'static str {
    match level {
        Privacy::Full => "full",
        Privacy::Host => "host",
        Privacy::None => "none",
    }
}

/// One saved example (`.exl .inp`): its result from the last run, the URL,
/// where it went instead when it failed, and the × that removes it.
fn example_row(
    index: usize,
    url: &str,
    passed: Option<bool>,
    went_to: Option<String>,
    cx: &mut Context<'_, PwrRoute>,
) -> Div {
    let failed = passed == Some(false);
    let remove = remove_button(
        format!("example-remove-{index}"),
        move |this, window, cx| this.remove_example(index, window, cx),
        cx,
    );
    input_box()
        .w_full()
        .gap(px(7.0))
        .pr(px(6.0))
        .when(failed, |row| row.border_color(theme::status_error()))
        .child(example_mark(passed))
        .child(
            crate::widgets::one_line(
                url.to_owned(),
                theme::mono_family(500.0),
                500.0,
                12.0,
                12.0,
                if failed {
                    theme::danger_text()
                } else {
                    theme::text_primary()
                },
            )
            .flex_1(),
        )
        .when_some(went_to, |row, went_to| {
            row.child(
                text(
                    format!("\u{2192} {went_to}"),
                    10.0,
                    500.0,
                    theme::danger_text(),
                )
                .whitespace_nowrap(),
            )
        })
        .child(div().ml_auto().child(remove))
}

/// `Examples`: the URLs that must resolve to the selected rule, with the
/// field that adds one. For a rewrite, each example also says what the URL
/// must become.
fn examples_section(
    app: &PwrRoute,
    selection: Selection,
    rail: &Rail,
    problems: &[String],
    window: &Window,
    cx: &mut Context<'_, PwrRoute>,
) -> Div {
    let (title, rows, help): (&str, Vec<Div>, &str) = match selection {
        Selection::Handler(h) => {
            let examples = app
                .handlers
                .get(h)
                .map(|handler| handler.examples.as_slice())
                .unwrap_or_default();
            let rows = examples
                .iter()
                .enumerate()
                .map(|(i, url)| {
                    let source = TestSource::RuleExample {
                        rule: h,
                        example: i,
                    };
                    let went_to = app.tests.failure(source).map(|f| {
                        f.winner
                            .as_ref()
                            .map_or_else(|| f.actual.clone(), winner_label)
                    });
                    example_row(i, url, app.tests.passed(source), went_to, cx)
                })
                .collect();
            (
                "Examples \u{00b7} must resolve here",
                rows,
                "Saved to rules.json as this handler's example. Run tests checks it still \
                 resolves here; a rule that takes it later fails the run.",
            )
        }
        Selection::Rewrite(r) => {
            let examples = app
                .rewrites
                .get(r)
                .map(|rewrite| rewrite.examples.as_slice())
                .unwrap_or_default();
            let rows = examples
                .iter()
                .enumerate()
                .map(|(i, example)| {
                    let source = TestSource::RewriteExample {
                        rewrite: r,
                        example: i,
                    };
                    let went_to = app.tests.failure(source).map(|f| f.actual.clone());
                    let shown = format!("{} \u{2192} {}", example.url, example.becomes);
                    example_row(i, &shown, app.tests.passed(source), went_to, cx)
                })
                .collect();
            (
                "Examples \u{00b7} what each becomes",
                rows,
                "Saved to rules.json as this rewrite's example: the URL and what it must \
                 become. Run tests checks it on every save.",
            )
        }
    };
    let add = editable(
        rail,
        Field::Example,
        "example-add".into(),
        Look {
            mono: true,
            placeholder: "https://",
            error: false,
        },
        None,
        window,
        cx,
    );
    let becomes = matches!(selection, Selection::Rewrite(_)).then(|| {
        editable(
            rail,
            Field::Becomes,
            "example-becomes".into(),
            Look {
                mono: true,
                placeholder: "becomes https://",
                error: false,
            },
            None,
            window,
            cx,
        )
    });
    section()
        .child(section_title(title))
        .when(!rows.is_empty(), |section| {
            section.child(div().flex().flex_col().gap(px(5.0)).children(rows))
        })
        .children(problems_under(problems, Under::Examples))
        .child(add)
        .children(becomes)
        .child(paragraph(
            "Return adds the link. It is written to rules.json with the rule.",
            10.5,
            15.75,
        ))
        .child(paragraph(help, 10.5, 15.75))
}

impl PwrRoute {
    /// Switches the selected handler's private window on or off.
    pub fn toggle_private(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(Selection::Handler(h)) = self.selection
            && let Some(handler) = self.handlers.get(h)
        {
            let private = !handler.private;
            self.store.set_private(h, private);
            self.refresh(cx);
        }
    }

    /// Sets what the selected handler keeps of the links it takes.
    pub fn set_keep(&mut self, level: Privacy, cx: &mut Context<'_, Self>) {
        if let Some(Selection::Handler(h)) = self.selection {
            self.store.set_privacy(h, level);
            self.refresh(cx);
        }
    }

    /// A keystroke in the example fields: Return adds the example to the
    /// selected rule. Returns whether the field took the key.
    pub fn example_key(
        &mut self,
        field: Field,
        event: &KeyDownEvent,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(text_field) = self.rail.as_mut().and_then(|rail| rail.field_mut(field)) else {
            return false;
        };
        match text_field.key(&event.keystroke, cx) {
            Edit::Ignored => false,
            Edit::Handled | Edit::Changed => true,
            Edit::Submit => {
                self.add_example_from_rail(cx);
                true
            }
        }
    }

    /// Adds the example typed in the rail to the selected rule, and clears
    /// the fields once it is taken.
    pub fn add_example_from_rail(&mut self, cx: &mut Context<'_, Self>) {
        let (Some(selection), Some(rail)) = (self.selection, self.rail.as_mut()) else {
            return;
        };
        let url = rail.example.text.clone();
        let becomes = rail.becomes.text.clone();
        let added = match selection {
            Selection::Handler(h) => self.store.add_example(h, &url),
            Selection::Rewrite(r) => self.store.add_rewrite_example(r, &url, &becomes),
        };
        if added.is_some() {
            rail.example.set("");
            rail.becomes.set("");
            self.refresh(cx);
        }
    }

    pub fn remove_example(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match self.selection {
            Some(Selection::Handler(h)) => self.store.remove_example(h, index),
            Some(Selection::Rewrite(r)) => self.store.remove_rewrite_example(r, index),
            None => return,
        }
        self.refresh(cx);
        window.focus(&self.focus, cx);
    }
}

/// `.try`: the dot and what the rule makes of the sample.
fn try_result(status: gpui_kit::Hsla, result: StyledText) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .py(px(8.0))
        .px(px(10.0))
        .rounded(px(7.0))
        .bg(theme::bg_app())
        .border_1()
        .border_color(theme::border_faint())
        .child(theme::dot(status))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_family(theme::SANS)
                .font_weight(theme::weight(500.0))
                .text_size(px(11.0))
                .line_height(px(15.4))
                .text_color(theme::text_secondary())
                .child(result),
        )
}

fn bold() -> HighlightStyle {
    HighlightStyle {
        color: Some(theme::text_primary()),
        font_weight: Some(theme::weight(600.0)),
        ..Default::default()
    }
}

/// The result as styled text and its dot's color.
fn try_text(result: &TryResult) -> (gpui_kit::Hsla, StyledText) {
    match result {
        TryResult::Text {
            verdict,
            message,
            bold: run,
        } => {
            let color = match verdict {
                Verdict::Takes => theme::status_ok(),
                Verdict::Beaten => theme::status_warning(),
                Verdict::Passes => theme::text_muted(),
                Verdict::Broken => theme::status_error(),
            };
            let highlights = run
                .as_deref()
                .and_then(|run| {
                    message
                        .find(run)
                        .map(|start| (start..start + run.len(), bold()))
                })
                .into_iter()
                .collect::<Vec<_>>();
            (
                color,
                StyledText::new(message.clone()).with_highlights(highlights),
            )
        }
        TryResult::Rewrote {
            before,
            after,
            then,
        } => {
            let mut message = "Rewrites to ".to_owned();
            let mut highlights = Vec::new();
            for part in url_diff(before, after) {
                if part.mark == UrlMark::Deleted {
                    continue;
                }
                let start = message.len();
                message.push_str(&part.text);
                if part.mark == UrlMark::Changed {
                    highlights.push((start..message.len(), bold()));
                }
            }
            message.push_str(". ");
            message.push_str(then);
            (
                theme::status_ok(),
                StyledText::new(message).with_highlights(highlights),
            )
        }
    }
}

fn try_section(
    rail: &Rail,
    problems: &[String],
    window: &Window,
    cx: &mut Context<'_, PwrRoute>,
) -> Div {
    let (status, message) = if !problems.is_empty() {
        (
            theme::status_error(),
            StyledText::new("Can't test until the problems above are fixed."),
        )
    } else {
        match &rail.try_result {
            Some(result) => try_text(result),
            None => (
                theme::text_muted(),
                StyledText::new("Type a link and press Return."),
            ),
        }
    };
    section()
        .child(section_title("Try it"))
        .child(editable(
            rail,
            Field::Sample,
            "try-sample".into(),
            Look {
                mono: true,
                placeholder: "https://example.com/",
                error: false,
            },
            None,
            window,
            cx,
        ))
        .child(try_result(status, message))
}

/// The rewrite's changes, applied in order: each a kind and a value.
fn changes_section(
    rewrite: &Rewrite,
    rail: &Rail,
    window: &Window,
    cx: &mut Context<'_, PwrRoute>,
) -> Div {
    let rows = rewrite.actions.iter().enumerate().fold(
        div().flex().flex_col().gap(px(6.0)),
        |rows, (index, action)| {
            let kind = kind_of(action);
            let value_empty = rail
                .values
                .get(index)
                .is_some_and(|v| v.text.trim().is_empty());
            let remove = remove_button(
                format!("change-remove-{index}"),
                move |this, window, cx| this.remove_action(index, window, cx),
                cx,
            );
            rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .w(px(18.0))
                            .flex_none()
                            .flex()
                            .justify_center()
                            .child(mono(
                                (index + 1).to_string(),
                                10.0,
                                600.0,
                                theme::text_subtle(),
                            )),
                    )
                    .child(kind_picker(index, kind.label, rail, cx))
                    .child(div().flex_1().min_w_0().child(editable(
                        rail,
                        Field::Value(index),
                        format!("change-{index}"),
                        Look {
                            mono: true,
                            placeholder: kind.placeholder,
                            error: value_empty,
                        },
                        Some(remove),
                        window,
                        cx,
                    ))),
            )
        },
    );
    let help = rewrite
        .actions
        .last()
        .map(|action| paragraph(kind_of(action).help, 10.5, 15.75));
    section()
        .child(section_title("Changes \u{00b7} applied in order"))
        .child(rows)
        .children(help)
        .children(problems_under(&rewrite.problems, Under::Changes))
        .child(add_button(
            "change-add",
            "+ change",
            None,
            |this, window, cx| this.add_action(window, cx),
            cx,
        ))
}

/// The kind box of a change and, while open, its menu of the five kinds.
fn kind_picker(
    index: usize,
    label: &'static str,
    rail: &Rail,
    cx: &mut Context<'_, PwrRoute>,
) -> Div {
    let menu = KINDS.iter().enumerate().fold(
        div()
            .id(("change-kind-menu", index))
            .occlude()
            .absolute()
            .top(px(32.0))
            .left_0()
            .w(px(300.0))
            .flex()
            .flex_col()
            .p(px(4.0))
            .bg(theme::bg_elevated())
            .border_1()
            .border_color(theme::border_default())
            .rounded(px(7.0))
            .shadow_lg()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if let Some(rail) = &mut this.rail {
                    rail.picking_kind = None;
                }
                cx.notify();
            })),
        |menu, (kind, item)| {
            let selected = item.label == label;
            menu.child(
                div()
                    .id(("change-kind-choice", kind))
                    .debug_selector(move || format!("change-{index}-kind-{kind}"))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(8.0))
                    .py(px(6.0))
                    .rounded(px(5.0))
                    .cursor_pointer()
                    .when(selected, |row| row.bg(theme::bg_row_active()))
                    .hover(|row| row.bg(theme::bg_hover()))
                    .child(text(item.label, 12.0, 500.0, theme::text_primary()))
                    .child(div().ml_auto().pl(px(16.0)).child(mono(
                        item.example,
                        10.5,
                        500.0,
                        theme::text_muted(),
                    )))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_action_kind(index, kind, cx);
                    })),
            )
        },
    );
    let footer = div()
        .px(px(8.0))
        .pt(px(6.0))
        .pb(px(4.0))
        .border_t_1()
        .border_color(theme::border_subtle())
        .mt(px(4.0))
        .child(paragraph(
            "Anything else is a rewrite function in a Finicky config.",
            10.5,
            15.2,
        ));
    let menu = menu.child(footer);
    div()
        .relative()
        .w(px(112.0))
        .flex_none()
        .child(
            input_box()
                .id(("change-kind", index))
                .debug_selector(move || format!("change-{index}-kind"))
                .min_h(px(28.0))
                .px(px(8.0))
                .cursor_pointer()
                .child(crate::widgets::one_line(
                    label,
                    theme::SANS,
                    400.0,
                    11.5,
                    11.5,
                    theme::text_primary(),
                ))
                .child(caret())
                // Capture runs in paint order: before the deferred menu's
                // press-outside handler closes it.
                .capture_any_mouse_down(cx.listener(|this, _, _, _| {
                    if let Some(rail) = &mut this.rail {
                        rail.was_picking = rail.picking_kind;
                    }
                }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(rail) = &mut this.rail {
                        rail.picking_kind = if rail.was_picking == Some(index) {
                            None
                        } else {
                            Some(index)
                        };
                    }
                    cx.notify();
                })),
        )
        .when(rail.picking_kind == Some(index), |picker| {
            picker.child(deferred(menu))
        })
}

/// `.rail-end`: delete (or discard), and where the rule stands with the
/// file.
fn rail_end(what: &'static str, draft: bool, note: String, cx: &mut Context<'_, PwrRoute>) -> Div {
    let kind = if draft {
        ButtonKind::Quiet
    } else {
        ButtonKind::Danger
    };
    div()
        .mt_auto()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(10.0))
        .py(px(12.0))
        .px(px(16.0))
        .border_t_1()
        .border_color(theme::border_faint())
        .child(
            button(what, kind, true)
                .id("rail-delete")
                .debug_selector(|| "rail-delete".into())
                .on_click(cx.listener(|this, _, window, cx| this.delete_selected(window, cx))),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .justify_end()
                .child(crate::widgets::one_line(
                    note,
                    theme::SANS,
                    400.0,
                    10.5,
                    15.75,
                    theme::text_subtle(),
                )),
        )
}

/// `.rrail`.
pub fn rail(
    app: &PwrRoute,
    window: &Window,
    cx: &mut Context<'_, PwrRoute>,
) -> impl IntoElement + use<> {
    let frame = div()
        .id("editor-rail")
        .debug_selector(|| "editor-rail".into())
        .w(px(WIDTH))
        .flex_none()
        .flex()
        .flex_col()
        .border_l_1()
        .border_color(theme::border_subtle())
        .bg(theme::bg_panel())
        // The rail outgrows a short window now that it has Keep and
        // Examples: scroll rather than clip the foot.
        .overflow_y_scroll();
    let (Some(selection), Some(rail)) = (app.selection, &app.rail) else {
        return frame;
    };
    match selection {
        Selection::Handler(index) => match app.handlers.get(index) {
            Some(handler) => {
                let draft = handler.rule.badge == RowBadge::Draft;
                let what = if draft { "New handler" } else { "Handler" };
                frame
                    .child(header(
                        &format!("{what} \u{00b7} {} of {}", index + 1, app.handlers.len()),
                        handler.title.clone(),
                        &handler.rule,
                        cx,
                    ))
                    .child(match_section(handler, rail, window, cx))
                    .child(open_in_section(handler, rail, window, cx))
                    .child(examples_section(
                        app,
                        selection,
                        rail,
                        &handler.problems,
                        window,
                        cx,
                    ))
                    .child(try_section(rail, &handler.problems, window, cx))
                    .child(rail_end(
                        if draft {
                            "Discard draft"
                        } else {
                            "Delete handler"
                        },
                        draft,
                        app.store.rail_label(selection),
                        cx,
                    ))
            }
            None => frame,
        },
        Selection::Rewrite(index) => match app.rewrites.get(index) {
            Some(rewrite) => {
                let draft = rewrite.rule.badge == RowBadge::Draft;
                let what = if draft { "New rewrite" } else { "Rewrite" };
                frame
                    .child(header(
                        &format!("{what} \u{00b7} {} of {}", index + 1, app.rewrites.len()),
                        rewrite.title.clone(),
                        &rewrite.rule,
                        cx,
                    ))
                    .child(
                        section()
                            .child(section_title("Match"))
                            .children(pattern_list(rail, &rewrite.problems, window, cx)),
                    )
                    .child(changes_section(rewrite, rail, window, cx))
                    .child(examples_section(
                        app,
                        selection,
                        rail,
                        &rewrite.problems,
                        window,
                        cx,
                    ))
                    .child(try_section(rail, &rewrite.problems, window, cx))
                    .child(rail_end(
                        if draft {
                            "Discard draft"
                        } else {
                            "Delete rewrite"
                        },
                        draft,
                        app.store.rail_label(selection),
                        cx,
                    ))
            }
            None => frame,
        },
    }
}
