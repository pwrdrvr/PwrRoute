//! The right rail: the editor for the selected rewrite or handler.
//!
//! Display only for now. The fields are drawn as the design shows them; the
//! enabled switch is the one control wired to state. The design draws the
//! handler editor (1a); the rewrite editor follows its patterns, with the
//! action where a handler has its browser.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Div, HighlightStyle, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, StyledText, div, px,
};

use crate::app::{PwrRoute, Selection};
use crate::fixtures::{Destination, Handler, Rewrite, Rule};
use crate::theme::{self, mono, text, tracked};
use crate::trace_view::{UrlMark, url_diff};
use crate::widgets::{
    ButtonKind, badge, button, caret, focus_ring, input_box, switch, tag, tag_frame,
};

/// The rail's width.
pub const WIDTH: f32 = 372.0;

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
fn paragraph(content: &'static str, size: f32, line_height: f32) -> Div {
    crate::widgets::paragraph(content, size, line_height, theme::text_subtle())
}

/// The rail's head: which rule of how many, its title and its switch.
fn header(eyebrow: &str, title: &'static str, rule: &Rule, cx: &mut Context<'_, PwrRoute>) -> Div {
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
                    text(title, 15.0, 700.0, theme::text_primary())
                        .mt(px(6.0))
                        .line_height(px(18.0)),
                ),
        )
        .child(
            div()
                .id("handler-enabled")
                .debug_selector(|| "handler-enabled".into())
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.toggle_selected(cx)))
                .child(switch(rule.enabled)),
        )
}

/// `.plist` and `.addp`: the URL patterns and the way to add one.
fn pattern_list(patterns: &[&'static str]) -> [Div; 2] {
    let list = patterns
        .iter()
        .fold(div().flex().flex_col().gap(px(5.0)), |list, pattern| {
            list.child(
                input_box()
                    .pr(px(6.0))
                    .child(mono(*pattern, 12.0, 500.0, theme::text_primary()))
                    .child(div().ml_auto().child(mono(
                        "\u{00d7}",
                        11.0,
                        500.0,
                        theme::text_subtle(),
                    ))),
            )
        });
    let add = div()
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
        .child(text("+ pattern", 11.0, 500.0, theme::text_muted()))
        .child(
            text("wildcard, or /regex/", 10.5, 400.0, theme::text_subtle())
                .line_height(px(15.75))
                .ml(px(4.0)),
        );
    [list, add]
}

fn match_section(handler: &Handler) -> Div {
    let [patterns, add] = pattern_list(&handler.patterns);
    let sent_from = match &handler.sent_from {
        Some(app) => tag_frame()
            .child(badge(app.badge))
            .child(mono(app.name, 10.5, 500.0, theme::text_primary()))
            .child(text(app.bundle_id, 10.5, 400.0, theme::text_subtle()))
            .child(mono("\u{00d7}", 10.5, 500.0, theme::text_subtle())),
        None => text("any app", 11.0, 500.0, theme::text_subtle()),
    };
    section()
        .child(section_title("Match"))
        .child(label("URL patterns"))
        .child(patterns)
        .child(add)
        .child(labeled_row("Sent from", sent_from).mt(px(6.0)))
        .child(paragraph(
            "The app that handed the link to the OS. Window title is not available on this host.",
            10.5,
            15.75,
        ))
}

fn open_in_section(handler: &Handler) -> Div {
    let (browser_badge, browser, profile) = match &handler.rule.destination {
        Destination::Browser {
            badge,
            name,
            profile,
        } => (*badge, *name, *profile),
        Destination::Rewrite { .. } => ("", "", None),
    };
    let field = |width: f32| input_box().w(px(width)).flex_none();
    section()
        .child(section_title("Open in"))
        .child(
            input_box()
                .child(badge(browser_badge))
                .child(text(browser, 12.5, 400.0, theme::text_primary()))
                .child(caret()),
        )
        .child(labeled_row(
            "Profile",
            field(190.0)
                .child(text(
                    profile.unwrap_or("Default"),
                    12.5,
                    400.0,
                    theme::text_primary(),
                ))
                .child(caret()),
        ))
        .child(labeled_row("Open in background", switch(false)))
        .child(labeled_row(
            "Extra arguments",
            field(190.0).child(mono("--new-window", 12.0, 500.0, theme::text_subtle())),
        ))
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

/// The sample link in its focused field.
fn try_input(sample: &'static str) -> Div {
    focus_ring(input_box().child(mono(sample, 12.0, 500.0, theme::text_primary())))
}

fn try_it_section(selected: usize, handler: &Handler) -> Div {
    // The sample link is a contrived Atlassian URL: handler 2 matches it, so
    // the rail says who wins from the selected handler's point of view.
    const SAMPLE_WINNER: usize = 1;
    let browser = match &handler.rule.destination {
        Destination::Browser { name, .. } => *name,
        Destination::Rewrite { .. } => "",
    };
    let (result, status) = match selected.cmp(&SAMPLE_WINNER) {
        std::cmp::Ordering::Greater => {
            let message = "Handler 2 wins first: Google Chrome \u{00b7} Work. Move this rule \
                           above it to change that.";
            let bold = "Google Chrome \u{00b7} Work";
            let start = message.find(bold).unwrap_or(0);
            (
                StyledText::new(message).with_highlights([(
                    start..start + bold.len(),
                    HighlightStyle {
                        color: Some(theme::text_primary()),
                        font_weight: Some(theme::weight(600.0)),
                        ..Default::default()
                    },
                )]),
                theme::status_warning(),
            )
        }
        std::cmp::Ordering::Equal => (
            StyledText::new(format!("This handler opens that link in {browser}.")),
            theme::status_ok(),
        ),
        std::cmp::Ordering::Less => (
            StyledText::new("No match. Handler 2 opens that link."),
            theme::text_muted(),
        ),
    };
    section()
        .child(section_title("Try it"))
        .child(try_input("https://example.atlassian.net/browse/PG-12"))
        .child(try_result(status, result))
}

/// The rewrite's condition: its patterns, and the query keys a link must
/// carry for a rewrite that drops them.
fn rewrite_match_section(rewrite: &Rewrite) -> Div {
    let [patterns, add] = pattern_list(&rewrite.patterns);
    let query = if rewrite.query.is_empty() {
        text("any query", 11.0, 500.0, theme::text_subtle())
    } else {
        rewrite
            .query
            .iter()
            .fold(div().flex().flex_wrap().gap(px(5.0)), |keys, key| {
                keys.child(tag(*key))
            })
    };
    section()
        .child(section_title("Match"))
        .child(label("URL patterns"))
        .child(patterns)
        .child(add)
        .child(labeled_row("With query", query).mt(px(6.0)))
}

/// What the rewrite changes: the action and its value.
fn rewrite_section(rewrite: &Rewrite) -> Div {
    let (kind, value) = match &rewrite.rule.destination {
        Destination::Rewrite { kind, value } => (*kind, *value),
        Destination::Browser { .. } => ("", ""),
    };
    let (action, field, help) = match kind {
        "host" => (
            "Set host",
            "Host",
            "Replaces the host and keeps the path and query.",
        ),
        "drop" => (
            "Drop query keys",
            "Keys",
            "Removes matching query parameters. * matches any run of characters.",
        ),
        "scheme" => ("Set scheme", "Scheme", "Replaces the scheme only."),
        _ => ("Replace URL", "URL", "Replaces the whole URL."),
    };
    let value_field = input_box().w(px(190.0)).flex_none().map(|input| {
        if kind == "drop" {
            value
                .split_whitespace()
                .fold(input.gap(px(5.0)), |input, key| input.child(tag(key)))
        } else {
            input.child(mono(value, 12.0, 500.0, theme::text_primary()))
        }
    });
    section()
        .child(section_title("Rewrite"))
        .child(
            input_box()
                .child(text(action, 12.5, 400.0, theme::text_primary()))
                .child(caret()),
        )
        .child(labeled_row(field, value_field))
        .child(paragraph(help, 10.5, 15.75))
}

fn rewrite_try_section(rewrite: &Rewrite) -> Div {
    let (sample, result) = rewrite.sample;
    let prefix = "Rewrites to ";
    let mut message = prefix.to_owned();
    let mut highlights = Vec::new();
    for part in url_diff(sample, result) {
        if part.mark == UrlMark::Deleted {
            continue;
        }
        let start = message.len();
        message.push_str(&part.text);
        if part.mark == UrlMark::Changed {
            highlights.push((
                start..message.len(),
                HighlightStyle {
                    color: Some(theme::text_primary()),
                    font_weight: Some(theme::weight(600.0)),
                    ..Default::default()
                },
            ));
        }
    }
    let message = if rewrite.rule.enabled {
        StyledText::new(message).with_highlights(highlights)
    } else {
        StyledText::new("Switched off: the link passes through unchanged.")
    };
    let status = if rewrite.rule.enabled {
        theme::status_ok()
    } else {
        theme::text_muted()
    };
    section()
        .child(section_title("Try it"))
        .child(try_input(sample))
        .child(try_result(status, message))
}

/// `.rail-end`: delete, and when the rule was saved.
fn rail_end(what: &'static str) -> Div {
    div()
        .mt_auto()
        .flex()
        .items_center()
        .justify_between()
        .py(px(12.0))
        .px(px(16.0))
        .border_t_1()
        .border_color(theme::border_faint())
        .child(button(what, ButtonKind::Danger, true))
        .child(text(
            "edited 09:41 \u{00b7} saved",
            10.5,
            400.0,
            theme::text_subtle(),
        ))
}

/// `.rrail`.
pub fn rail(app: &PwrRoute, cx: &mut Context<'_, PwrRoute>) -> impl IntoElement + use<> {
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
        .overflow_hidden();
    match app.selection {
        Selection::Handler(index) => match app.handlers.get(index) {
            Some(handler) => frame
                .child(header(
                    &format!("Handler \u{00b7} {} of {}", index + 1, app.handlers.len()),
                    handler.title,
                    &handler.rule,
                    cx,
                ))
                .child(match_section(handler))
                .child(open_in_section(handler))
                .child(try_it_section(index, handler))
                .child(rail_end("Delete handler")),
            None => frame,
        },
        Selection::Rewrite(index) => match app.rewrites.get(index) {
            Some(rewrite) => frame
                .child(header(
                    &format!("Rewrite \u{00b7} {} of {}", index + 1, app.rewrites.len()),
                    rewrite.title,
                    &rewrite.rule,
                    cx,
                ))
                .child(rewrite_match_section(rewrite))
                .child(rewrite_section(rewrite))
                .child(rewrite_try_section(rewrite))
                .child(rail_end("Delete rewrite")),
            None => frame,
        },
    }
}
