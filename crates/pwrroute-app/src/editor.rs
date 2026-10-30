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

use crate::theme::px;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, Div, ElementId, HighlightStyle, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, StyledText, Window, deferred, div,
};
use pwrroute_engine::{Privacy, TestSource};

use crate::app::{PwrRoute, Selection};
use crate::e2e;
use crate::rules_model::{Handler, KEEP_LEVELS, KINDS, Rewrite, RowBadge, Rule, kind_of};
use crate::rules_view::{ChipTone, chip_in, example_mark, winner_label};
use crate::text_field::{Edit, TextField};
use crate::theme::{self, mono, text, tracked};
use crate::trace_view::{UrlMark, url_diff};
use crate::try_it::{TryResult, Verdict, sample_url};
use crate::widgets::{
    ButtonKind, button, caret, focus_ring, input_box, segment, segmented, switch,
};

/// The rail's width.
pub const WIDTH: f32 = 372.0;

/// One of the rail's text fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Pattern(usize),
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
    /// Which of Open in's pickers (or "+ app"'s menu) is open.
    pub menu: Option<RailMenu>,
    /// The one that was open when a picker's box was pressed, as
    /// `was_picking`.
    was_menu: Option<RailMenu>,
    /// "Other app…" was chosen: Open in is a text field until "Choose from
    /// the list". Kept across rebuilds of the same rule's rail.
    pub browser_typed: bool,
    /// The same for Profile ("Profile by name or folder…").
    pub profile_typed: bool,
    /// The rule this rail was built for, so a rebuild for the same rule
    /// keeps what the person chose to type.
    pub(crate) of: Option<Selection>,
    pub try_result: Option<TryResult>,
}

/// The rail's menus: Start from, Open in, Profile, and "+ app" under Sent
/// from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailMenu {
    StartFrom,
    Opener,
}

impl Rail {
    pub fn for_handler(handler: &Handler, cx: &mut App) -> Self {
        Self {
            patterns: fields(&handler.patterns, cx),
            browser: TextField::new(handler.browser.clone(), cx),
            profile: TextField::new(handler.profile.clone(), cx),
            values: Vec::new(),
            sample: TextField::new(sample_url(&handler.patterns), cx),
            example: TextField::new("", cx),
            becomes: TextField::new("", cx),
            sample_edited: false,
            picking_kind: None,
            was_picking: None,
            menu: None,
            was_menu: None,
            browser_typed: false,
            profile_typed: false,
            of: None,
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
            browser: TextField::new("", cx),
            profile: TextField::new("", cx),
            values: fields(&values, cx),
            sample: TextField::new(sample_url(&rewrite.patterns), cx),
            example: TextField::new("", cx),
            becomes: TextField::new("", cx),
            sample_edited: false,
            picking_kind: None,
            was_picking: None,
            menu: None,
            was_menu: None,
            browser_typed: false,
            profile_typed: false,
            of: None,
            try_result: None,
        }
    }

    /// Takes what the person chose on `old`, the rail this one replaces for
    /// the same rule: typed Open in and Profile, and a link typed in Try it.
    pub(crate) fn keep_from(&mut self, old: Rail) {
        self.browser_typed = old.browser_typed;
        self.profile_typed = old.profile_typed;
        if old.sample_edited {
            self.sample = old.sample;
            self.sample_edited = true;
        }
    }

    pub fn field_mut(&mut self, field: Field) -> Option<&mut TextField> {
        match field {
            Field::Pattern(i) => self.patterns.get_mut(i),
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
fn label(content: impl Into<SharedString>) -> Div {
    text(content, 11.0, 600.0, theme::text_secondary()).line_height(px(14.3))
}

/// `.lrow`: a label on the left, its control on the right.
fn labeled_row(content: impl Into<SharedString>, control: impl IntoElement) -> Div {
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

/// `.warn-t`: what the rule asks for that this machine does not have. Not
/// a problem: the rule saves and runs, and the router falls back.
fn warning(selector: &'static str, message: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(selector)
        .debug_selector(move || selector.into())
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
                .bg(theme::warn_soft())
                .child(mono("!", 9.0, 700.0, theme::warn_text())),
        )
        .child(crate::widgets::paragraph(
            message,
            10.5,
            15.2,
            theme::warn_text(),
        ))
}

/// Where a problem's message belongs in the rail, by its words.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Under {
    Patterns,
    Openers,
    Browser,
    Changes,
    /// The Keep control: a privacy word the engine does not know. A
    /// rewrite has no Keep control, so its editor draws these rows under
    /// Changes (`changes_section`).
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
            .map(crate::accessibility::clickable)
            .role(gpui_kit::Role::Switch)
            .aria_label("Rule enabled")
            .aria_toggled(if rule.enabled {
                gpui_kit::Toggled::True
            } else {
                gpui_kit::Toggled::False
            })
            .on_click(cx.listener(|this, _, _, cx| this.toggle_selected(cx)))
            .child(switch(rule.enabled))
            .into_any_element()
    };
    section()
        .on_children_prepainted(e2e::regions(&["editor.title", "editor.enabled"]))
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
    let shown = crate::text_field::Look {
        mono: look.mono,
        placeholder: look.placeholder,
    };
    let mut inner = input_box()
        .w_full()
        .gap(px(8.0))
        .when(look.error, |input| {
            input.border_color(theme::status_error())
        })
        .child(crate::text_field::text(text_field, shown, focused));
    inner = inner.when_some(trailing, |input, trailing| {
        input.pr(px(6.0)).child(div().ml_auto().child(trailing))
    });
    let boxed = if focused { focus_ring(inner) } else { inner };
    let id: SharedString = selector.into();
    let area = div()
        .id(ElementId::Name(id.clone()))
        .debug_selector(move || id.to_string());
    crate::text_field::hit_area(area, text_field)
        .aria_label(match field {
            Field::Pattern(index) => format!("URL pattern {}", index + 1),
            Field::Browser => "Browser name".into(),
            Field::Profile => "Browser profile".into(),
            Field::Value(index) => format!("Rewrite value {}", index + 1),
            Field::Sample => "Try this link".into(),
            Field::Example => "Example link".into(),
            Field::Becomes => "Expected rewritten link".into(),
        })
        .aria_placeholder(look.placeholder)
        .w_full()
        .min_w_0()
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
        .map(crate::accessibility::keyboard_only)
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
            .aria_label("Remove field")
            .child(mono("\u{00d7}", 11.0, 500.0, theme::text_subtle()))
            .map(crate::accessibility::clickable)
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
        .map(crate::accessibility::clickable)
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
    app: &PwrRoute,
    handler: &Handler,
    rail: &Rail,
    window: &Window,
    cx: &mut Context<'_, PwrRoute>,
) -> Div {
    // Each app as a chip: its icon and the name it goes by here, never
    // typed (design: Destination and App Pickers 2a).
    let openers = handler.openers.iter().enumerate().fold(
        div().flex().flex_wrap().items_center().gap(px(6.0)),
        |list, (index, value)| list.child(app_chip(index, value, cx)),
    );
    let sent_from = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(10.0))
        .mt(px(6.0))
        .child(label("Sent from"))
        .when(handler.openers.is_empty(), |row| {
            row.child(
                div()
                    .id("opener-any")
                    .debug_selector(|| "opener-any".into())
                    .child(text("Any app", 11.0, 500.0, theme::text_muted())),
            )
        });
    let suggested = match app.selection {
        Some(Selection::Handler(index)) => crate::start_from::suggestions(
            app,
            index,
            rail.patterns
                .first()
                .map_or("", |field| field.text.as_str()),
            cx,
        ),
        _ => None,
    };
    // The suggestions sit under the fields, above "+ pattern".
    let mut patterns = pattern_list(rail, &handler.problems, window, cx);
    if let Some(suggested) = suggested {
        let at = patterns.len().saturating_sub(1);
        patterns.insert(at, suggested.into_any_element());
    }
    section()
        .child(section_title("Match"))
        .children(patterns)
        .child(sent_from)
        .child({
            // The menu hangs from the whole row, at its left, so it stays
            // inside the rail however far "+ app" sits along it.
            let (add, menu) = opener_add(app, handler, rail, cx);
            div()
                .relative()
                .child(openers.child(add))
                .when_some(menu, |row, menu| row.child(deferred(menu)))
        })
        .children(problems_under(&handler.problems, Under::Openers))
        .child(paragraph(
            "The app that handed the link to the OS. With none, the rule takes links from \
             any app.",
            10.5,
            15.75,
        ))
}

/// `.appc`: one app a rule requires, as the reader knows it, with the
/// value `rules.json` keeps on hover and × to remove it.
fn app_chip(index: usize, value: &str, cx: &mut Context<'_, PwrRoute>) -> Stateful<Div> {
    let shown = crate::default_browser::browser_name(value.trim());
    let icon = crate::widgets::AppBadge::new(
        crate::widgets::badge_for(&shown).letters().clone(),
        value.trim().to_owned(),
    );
    let saved = (shown != value.trim()).then(|| format!("{shown} \u{00b7} {}", value.trim()));
    div()
        .id(ElementId::Name(format!("opener-{index}").into()))
        .debug_selector(move || format!("opener-{index}"))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(7.0))
        .h(px(28.0))
        .pl(px(7.0))
        .pr(px(4.0))
        .border_1()
        .border_color(theme::border_default())
        .rounded(px(8.0))
        .bg(theme::bg_elevated())
        .child(crate::widgets::badge(icon))
        .child(text(shown, 11.5, 600.0, theme::text_primary()))
        .child(remove_button(
            format!("opener-remove-{index}"),
            move |this, window, cx| this.remove_opener(index, window, cx),
            cx,
        ))
        .when_some(saved, |chip, saved| {
            chip.tooltip(crate::overview::tip(saved, None))
        })
}

/// "+ app" (design 2b): the apps that sent links recently, from Activity,
/// then the apps other rules name, then any app in Applications through
/// the file panel. Nothing here is typed. The button, and the menu while
/// it is open, for the caller to hang under the Sent from row.
fn opener_add(
    app: &PwrRoute,
    handler: &Handler,
    rail: &Rail,
    cx: &mut Context<'_, PwrRoute>,
) -> (Div, Option<crate::accessibility::MenuCard>) {
    let Some(Selection::Handler(h)) = app.selection else {
        return (div(), None);
    };
    let add = div().flex().child(
        add_button(
            "opener-add",
            "+ app",
            None,
            |this, _, cx| this.toggle_rail_menu(RailMenu::Opener, cx),
            cx,
        )
        .h(px(28.0))
        .capture_any_mouse_down(cx.listener(|this, _, _, _| this.note_rail_menu())),
    );
    // The menu reads the feed: only while it is open.
    if rail.menu != Some(RailMenu::Opener) {
        return (add, None);
    }
    let has = |value: &str| {
        handler
            .openers
            .iter()
            .any(|o| o.trim().eq_ignore_ascii_case(value))
    };
    let mut menu = floating_menu("opener-menu", cx).w(px(280.0));
    let recent = crate::start_from::recent_senders(cx);
    let mut row = 0;
    if !recent.is_empty() {
        menu = menu.child(crate::widgets::menu_heading("Sent links recently"));
        for sender in recent {
            // The rule may name the app by bundle id or by name.
            let mine = has(&sender.value)
                || has(&sender.name)
                || handler.openers.iter().any(|o| {
                    crate::default_browser::browser_name(o.trim())
                        .eq_ignore_ascii_case(&sender.name)
                });
            let chosen = sender.value.clone();
            let hint = if mine {
                "in this rule".to_owned()
            } else {
                format!(
                    "{} {}",
                    sender.links,
                    if sender.links == 1 { "link" } else { "links" }
                )
            };
            menu = menu.child(choice_row(
                format!("opener-choice-{row}"),
                Some(crate::widgets::AppBadge::new(
                    crate::widgets::badge_for(&sender.name).letters().clone(),
                    sender.value.clone(),
                )),
                sender.name,
                Some(hint.into()),
                false,
                mine,
                cx.listener(move |this, _, window, cx| {
                    if !mine {
                        this.choose_opener(&chosen, window, cx);
                    }
                }),
            ));
            row += 1;
        }
    }
    let named = crate::rule_pickers::named_openers(&app.handlers, h);
    if !named.is_empty() {
        menu = menu.child(crate::widgets::menu_heading("Named in your rules"));
        for name in named {
            let shown = crate::default_browser::browser_name(&name);
            let chosen = name.clone();
            menu = menu.child(choice_row(
                format!("opener-choice-{row}"),
                Some(crate::widgets::AppBadge::new(
                    crate::widgets::badge_for(&shown).letters().clone(),
                    name.clone(),
                )),
                shown,
                None,
                false,
                false,
                cx.listener(move |this, _, window, cx| this.choose_opener(&chosen, window, cx)),
            ));
            row += 1;
        }
    }
    let menu = menu.child(crate::widgets::menu_divider()).child(other_row(
        "opener-other",
        "Choose an app in Applications\u{2026}",
        cx.listener(|this, _, window, cx| this.pick_opener_app(window, cx)),
    ));
    (add, Some(menu.top(gpui_kit::relative(1.0)).mt(px(6.0))))
}

/// A rail menu placed under its control at the left; the caller sets its
/// width.
fn floating_menu(
    id: &'static str,
    cx: &mut Context<'_, PwrRoute>,
) -> crate::accessibility::MenuCard {
    crate::widgets::menu_card(id)
        .debug_selector(move || id.into())
        .absolute()
        .top(px(34.0))
        .left_0()
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            if let Some(rail) = &mut this.rail {
                rail.menu = None;
            }
            cx.notify();
        }))
}

type OnClick = Box<dyn Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static>;

/// One choice in a rail picker's list.
fn choice_row(
    selector: String,
    badge_label: Option<crate::widgets::AppBadge>,
    label: impl Into<SharedString>,
    hint: Option<SharedString>,
    selected: bool,
    dim: bool,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let id: SharedString = selector.into();
    let on_click: OnClick = Box::new(on_click);
    theme::dimmed_if(dim, || {
        crate::widgets::menu_row(ElementId::Name(id.clone()), badge_label, label, hint)
    })
    .debug_selector(move || id.to_string())
    .aria_selected(selected)
    .when(selected, |row| row.bg(theme::bg_row_active()))
    .hover(|row| row.bg(theme::bg_hover()))
    .on_click(on_click)
}

/// The list's way out to a typed value ("Other app…").
fn other_row(
    id: &'static str,
    label: &'static str,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .px(px(8.0))
        .py(px(6.0))
        .rounded(px(5.0))
        .cursor_pointer()
        .hover(|row| row.bg(theme::bg_hover()))
        .child(text(label, 11.5, 500.0, theme::text_muted()))
        .map(crate::accessibility::clickable)
        .on_click(on_click)
}

/// "Choose from the list": back from a typed value to the picker.
fn list_link(
    selector: &'static str,
    on_click: impl Fn(&mut PwrRoute, &mut Window, &mut Context<'_, PwrRoute>) + 'static,
    cx: &mut Context<'_, PwrRoute>,
) -> Div {
    div().child(
        div()
            .id(selector)
            .debug_selector(move || selector.into())
            .cursor_pointer()
            .px(px(2.0))
            .child(text("\u{2190} list", 10.5, 500.0, theme::accent()))
            .map(crate::accessibility::clickable)
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx))),
    )
}

/// The scan's browsers are known: it ran (so finding none is an answer),
/// or the window was handed a scan, as a test or screenshot does. Before
/// then nothing is drawn as missing.
pub(crate) fn scanned(app: &PwrRoute, cx: &App) -> bool {
    !app.installed.is_empty() || crate::browser_scan::has_run(cx)
}

fn open_in_section(
    app: &PwrRoute,
    handler: &Handler,
    rail: &Rail,
    window: &Window,
    cx: &mut Context<'_, PwrRoute>,
) -> Div {
    let value = handler.browser.trim();
    let shown = crate::default_browser::browser_name(value);
    // Don't open names no app: never missing, and no profile to check.
    let dont_open = pwrroute_engine::is_dont_open(value);
    let missing = !dont_open
        && crate::rule_pickers::is_missing(value, &app.browsers, &app.installed, scanned(app, cx));
    let stray_profile = !dont_open
        && crate::destinations::unknown_profile(
            &app.browsers,
            &app.installed,
            &crate::destinations::Choice::of(handler),
        );
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
    let destination = if rail.browser_typed {
        typed_destination(app, handler, rail, window, cx)
    } else {
        lanes_destination(app, handler, cx)
    };
    section()
        .child(section_title("Open in"))
        .child(destination)
        .children(problems_under(&handler.problems, Under::Browser))
        .when(missing, |section| {
            section.child(warning(
                "browser-warning",
                format!(
                    "{shown} isn't installed here. Links this rule takes go to the fallback \
                     browser until it is, or until you pick another."
                ),
            ))
        })
        .when(!missing && stray_profile, |section| {
            section.child(warning(
                "profile-warning",
                format!(
                    "{shown} has no profile named {} on this Mac. Pick one, or keep it for \
                     another machine.",
                    handler.profile.trim()
                ),
            ))
        })
        .when(dont_open, |section| {
            section.child(paragraph(
                "Links this rule takes are not opened. Activity lists them under Not opened, \
                 where you can open one or add a rule.",
                10.5,
                15.75,
            ))
        })
        .child(labeled_row("Activity keeps", keep_control).mt(px(4.0)))
        .children(problems_under(&handler.problems, Under::Keep))
        .child(paragraph(
            "What Activity and the journal remember about links this rule opens. A private \
             window keeps host only unless set here.",
            10.5,
            15.75,
        ))
}

/// Open in as lanes (design: Destination and App Pickers 1c, 1d): the
/// rule's browser first, then the scan's, and under them the hidden count
/// and the way out to typed values.
fn lanes_destination(app: &PwrRoute, handler: &Handler, cx: &mut Context<'_, PwrRoute>) -> Div {
    let current = crate::destinations::Choice::of(handler);
    // Scan order, so a lane stays put when one of its pills is picked.
    let lanes = crate::destinations::lanes(
        &app.browsers,
        &app.installed,
        &current,
        None,
        scanned(app, cx),
        true,
    );
    let hidden = app.installed.iter().filter(|a| a.hidden).count();
    let foot = div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(10.0))
        .px(px(2.0))
        .pt(px(2.0))
        .child(if hidden > 0 {
            text(
                format!("Hidden in Settings \u{203a} Browsers: {hidden}"),
                10.5,
                500.0,
                theme::text_subtle(),
            )
        } else {
            div()
        })
        .child(
            div()
                .id("destination-typed")
                .debug_selector(|| "destination-typed".into())
                .cursor_pointer()
                .child(text(
                    "Other app or profile by name\u{2026}",
                    10.5,
                    500.0,
                    theme::accent_bright(),
                ))
                .map(crate::accessibility::clickable)
                .on_click(cx.listener(|this, _, window, cx| this.type_browser(window, cx))),
        );
    div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(crate::destinations::draw(
            "destination",
            &lanes,
            crate::destinations::Density::Regular,
            |this: &mut PwrRoute, choice, _, cx| this.choose_destination(choice, cx),
            cx,
        ))
        .when(app.browsers.is_empty(), |list| {
            list.child(paragraph(
                if scanned(app, cx) {
                    "No browsers found. Rescan in Settings \u{203a} Browsers, or name one."
                } else {
                    "No browsers to list yet. Name one below."
                },
                10.5,
                15.2,
            ))
        })
        .child(foot)
}

/// Open in by hand: the app and the profile as typed, and the private
/// switch, for a browser on another machine or a profile by its folder.
fn typed_destination(
    app: &PwrRoute,
    handler: &Handler,
    rail: &Rail,
    window: &Window,
    cx: &mut Context<'_, PwrRoute>,
) -> Div {
    use crate::rule_pickers::PrivateMode;
    let error = handler.problems.iter().any(|m| under(m) == Under::Browser);
    let back = list_link(
        "browser-list",
        |this, window, cx| this.list_browser(window, cx),
        cx,
    );
    let value = handler.browser.trim();
    let detected = !value.is_empty()
        && (app
            .browsers
            .iter()
            .any(|b| b.name.eq_ignore_ascii_case(value))
            || crate::rule_pickers::installed_named(value, &app.installed).is_some());
    let (private_label, private_enabled) = match crate::rule_pickers::private_mode(value, detected)
    {
        PrivateMode::Window(phrase) => (format!("Open in {phrase}"), true),
        PrivateMode::Unknown => ("Open in a private window".to_owned(), true),
        // Already on: it can still be switched off.
        PrivateMode::Unsupported => ("Open in a private window".to_owned(), handler.private),
    };
    let private = div()
        .id("handler-private")
        .debug_selector(|| "handler-private".into())
        .when(private_enabled, |switch| {
            switch
                .cursor_pointer()
                .map(crate::accessibility::clickable)
                .on_click(cx.listener(|this, _, _, cx| this.toggle_private(cx)))
        })
        .when(!private_enabled, |switch| switch.opacity(theme::DISABLED))
        .role(gpui_kit::Role::Switch)
        .aria_label(private_label.clone())
        .aria_toggled(if handler.private {
            gpui_kit::Toggled::True
        } else {
            gpui_kit::Toggled::False
        })
        .child(switch(handler.private));
    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(editable(
            rail,
            Field::Browser,
            "browser".into(),
            Look {
                mono: false,
                placeholder: "App name, bundle id or path",
                error,
            },
            Some(back),
            window,
            cx,
        ))
        .child(label("Profile").mt(px(4.0)))
        .child(editable(
            rail,
            Field::Profile,
            "profile".into(),
            Look {
                mono: false,
                placeholder: "Default profile, or a name or folder",
                error: false,
            },
            None,
            window,
            cx,
        ))
        .child(labeled_row(private_label, private).mt(px(4.0)))
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
                "Return adds it to rules.json with the rule. Run tests checks it still \
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
                "Return adds it to rules.json with the rewrite: the URL and what it must \
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
    let empty = match selection {
        Selection::Handler(_) => {
            "No examples yet. Add a link this rule must open; every save checks it still \
             lands here."
        }
        Selection::Rewrite(_) => {
            "No examples yet. Add a link and what it must become; every save checks it."
        }
    };
    section()
        .child(section_title(title))
        .map(|section| {
            if rows.is_empty() {
                section.child(
                    div()
                        .id("examples-empty")
                        .debug_selector(|| "examples-empty".into())
                        .child(paragraph(empty, 10.5, 15.75)),
                )
            } else {
                section.child(div().flex().flex_col().gap(px(5.0)).children(rows))
            }
        })
        .children(problems_under(problems, Under::Examples))
        .child(add)
        .children(becomes)
        .child(paragraph(help, 10.5, 15.75))
}

impl PwrRoute {
    /// A press on a picker's box: which menu was open, before the menu's
    /// press-outside handler closes it.
    pub(crate) fn note_rail_menu(&mut self) {
        if let Some(rail) = &mut self.rail {
            rail.was_menu = rail.menu;
        }
    }

    /// Opens `menu`, or closes it when it was the one open.
    pub fn toggle_rail_menu(&mut self, menu: RailMenu, cx: &mut Context<'_, Self>) {
        if let Some(rail) = &mut self.rail {
            rail.menu = if rail.was_menu == Some(menu) {
                None
            } else {
                Some(menu)
            };
            rail.was_menu = None;
        }
        cx.notify();
    }

    /// "Other app…": Open in becomes a text field, focused.
    pub fn type_browser(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(rail) = &mut self.rail {
            rail.menu = None;
            rail.browser_typed = true;
            window.focus(&rail.browser.focus, cx);
        }
        cx.notify();
    }

    /// Back from the typed Open in to the picker; what was typed stays.
    pub fn list_browser(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(rail) = &mut self.rail {
            rail.browser_typed = false;
        }
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// A pill in Open in: browser, profile and private window at once.
    pub fn choose_destination(
        &mut self,
        choice: crate::destinations::Choice,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(Selection::Handler(h)) = self.selection else {
            return;
        };
        self.store.set_browser(h, &choice.browser);
        self.store.set_profile(h, &choice.profile);
        self.store.set_private(h, choice.private);
        if let Some(rail) = &mut self.rail {
            rail.browser.set(choice.browser);
            rail.profile.set(choice.profile);
            rail.menu = None;
            rail.browser_typed = false;
            rail.profile_typed = false;
        }
        self.refresh(cx);
    }

    /// "+ app"'s choice: the selected handler also requires links from
    /// `name`, an app another rule names.
    pub fn choose_opener(&mut self, name: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(Selection::Handler(h)) = self.selection else {
            return;
        };
        let name = name.trim();
        let has = self.handlers.get(h).is_some_and(|handler| {
            handler
                .openers
                .iter()
                .any(|o| o.trim().eq_ignore_ascii_case(name))
        });
        if !has && let Some(i) = self.store.add_opener(h) {
            self.store.set_opener(h, i, name);
        }
        if let Some(rail) = &mut self.rail {
            rail.menu = None;
        }
        self.refresh(cx);
        self.rebuild_rail_kept(cx);
        window.focus(&self.focus, cx);
    }

    /// "Choose an app in Applications…": the system file panel, and the
    /// app picked joins the rule by its bundle id (by its name when it has
    /// none).
    pub fn pick_opener_app(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        // The panel's answer is for this rule only: if another is selected
        // (or the rules moved) by then, it is dropped.
        let asked = (self.selection, self.handlers.len());
        if let Some(rail) = &mut self.rail {
            rail.menu = None;
        }
        cx.notify();
        let paths = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let value = pwrroute_os::app_bundle_id(&path).or_else(|| {
                path.file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
            });
            let Some(value) = value else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                if (this.selection, this.handlers.len()) == asked {
                    this.choose_opener(&value, window, cx);
                }
            });
        })
        .detach();
    }

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
        // `rewrites[].privacy` has no control here yet; its problem is
        // still the rewrite's, so it is drawn where the rewrite's other
        // fields are, not lost behind the NotSaved badge.
        .children(keep_problems(&rewrite.problems))
        .child(add_button(
            "change-add",
            "+ change",
            None,
            |this, window, cx| this.add_action(window, cx),
            cx,
        ))
}

/// The rewrite's Keep problems, under Changes, as one block the tests can
/// find; nothing when there are none.
fn keep_problems(problems: &[String]) -> Option<Stateful<Div>> {
    let rows = problems_under(problems, Under::Keep);
    (!rows.is_empty()).then(|| {
        div()
            .id("rewrite-keep-problems")
            .debug_selector(|| "rewrite-keep-problems".into())
            .flex()
            .flex_col()
            .gap(px(6.0))
            .children(rows)
    })
}

/// The kind box of a change and, while open, its menu of the five kinds.
fn kind_picker(
    index: usize,
    label: &'static str,
    rail: &Rail,
    cx: &mut Context<'_, PwrRoute>,
) -> Div {
    let menu = KINDS.iter().enumerate().fold(
        crate::widgets::menu_card(("change-kind-menu", index))
            .absolute()
            .top(px(32.0))
            .left_0()
            .w(px(300.0))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if let Some(rail) = &mut this.rail {
                    rail.picking_kind = None;
                }
                cx.notify();
            })),
        |menu, (kind, item)| {
            let selected = item.label == label;
            menu.child(
                crate::widgets::menu_row(
                    ("change-kind-choice", kind),
                    None,
                    item.label,
                    Some(item.example.into()),
                )
                .debug_selector(move || format!("change-{index}-kind-{kind}"))
                .aria_selected(selected)
                .when(selected, |row| row.bg(theme::bg_row_active()))
                .hover(|row| row.bg(theme::bg_hover()))
                .map(crate::accessibility::clickable)
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
                .id(("change-kind", index)).aria_expanded(rail.picking_kind == Some(index))
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
                .map(crate::accessibility::clickable)
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

/// Where a new rule came from and where it runs, above its fields while
/// it is a draft (design: Rule Editor Pickers 1b, 2e, 5b).
fn draft_note(app: &PwrRoute, index: usize, draft: bool) -> Option<Stateful<Div>> {
    if !draft {
        return None;
    }
    let mut lines: Vec<String> = Vec::new();
    match app.draft_from {
        Some((at, DraftFrom::Overview)) if at == index => lines.push(
            "From the Overview: the browser and profile are set. Add the links this rule \
             should take."
                .to_owned(),
        ),
        Some((at, DraftFrom::TestLink)) if at == index => lines.push(
            "From Test a link: the pattern is suggested from the link, and Try it holds it."
                .to_owned(),
        ),
        _ => {}
    }
    if let Some(config) = &app.config {
        lines.push(crate::activity_actions::config_hint(
            &config.path,
            app.rules_first(),
        ));
    }
    if lines.is_empty() {
        return None;
    }
    Some(
        div()
            .id("draft-note")
            .debug_selector(|| "draft-note".into())
            .mx(px(16.0))
            .mt(px(12.0))
            .py(px(8.0))
            .px(px(10.0))
            .rounded(px(7.0))
            .bg(theme::bg_row_active())
            .border_1()
            .border_color(theme::border_faint())
            .child(crate::widgets::paragraph(
                lines.join(" "),
                10.5,
                15.75,
                theme::text_secondary(),
            )),
    )
}

/// Where a draft came from, for its note.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftFrom {
    /// A card's "+ Rule" or a profile's "+ Rule for …".
    Overview,
    /// Test a link's "Add a rule…".
    TestLink,
}

/// The stacked layout's way back to the list (`app::Layout`).
pub const BACK_ID: &str = "editor-back";

/// `.rrail`. `stacked`: the window is too narrow for the list beside it,
/// so the rail fills the pane and leads with "All rules".
pub fn rail(
    app: &PwrRoute,
    stacked: bool,
    window: &Window,
    cx: &mut Context<'_, PwrRoute>,
) -> impl IntoElement + use<> {
    let frame = div()
        .id("editor-rail")
        .debug_selector(|| "editor-rail".into())
        .map(|frame| {
            if stacked {
                frame.flex_1().min_w_0()
            } else {
                frame.w(px(WIDTH)).flex_none()
            }
        })
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
    let frame = frame.when(stacked, |frame| frame.child(back(cx)));
    match selection {
        Selection::Handler(index) => match app.handlers.get(index) {
            Some(handler) => {
                let draft = handler.rule.badge == RowBadge::Draft;
                let what = if draft { "New rule" } else { "Rule" };
                frame
                    .child(header(
                        &format!("{what} \u{00b7} {} of {}", index + 1, app.handlers.len()),
                        handler.title.clone(),
                        &handler.rule,
                        cx,
                    ))
                    .children(draft_note(app, index, draft))
                    .when(draft, |frame| {
                        frame.child(crate::start_from::section(app, index, cx))
                    })
                    .child(match_section(app, handler, rail, window, cx))
                    .child(open_in_section(app, handler, rail, window, cx))
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
                            "Delete rule"
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

/// "‹ All rules": back to the list from the stacked rail, as Esc is.
fn back(cx: &mut Context<'_, PwrRoute>) -> impl IntoElement + use<> {
    div().px(px(16.0)).pt(px(12.0)).child(
        crate::widgets::keyboard_control(div(), false)
            .id(BACK_ID)
            .debug_selector(|| BACK_ID.into())
            .flex()
            .items_center()
            .gap(px(6.0))
            .py(px(4.0))
            .px(px(6.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(theme::transparent())
            .cursor_pointer()
            .hover(|back| back.bg(theme::bg_hover()))
            .child(theme::text("\u{2039}", 14.0, 500.0, theme::accent_bright()))
            .child(theme::text(
                "All rules",
                12.0,
                500.0,
                theme::accent_bright(),
            ))
            .on_click(cx.listener(|this, _, _, cx| this.deselect(cx))),
    )
}
