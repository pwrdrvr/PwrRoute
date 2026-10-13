//! The right rail: the editor for the selected handler.
//!
//! Display only for now. The fields are drawn as the design shows them; the
//! enabled switch is the one control wired to state.

use gpui_kit::{
    Context, Div, HighlightStyle, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, StyledText, div, px,
};

use crate::app::PwrRoute;
use crate::fixtures::{Destination, Handler};
use crate::theme::{self, mono, text, tracked};
use crate::widgets::{badge, caret, focus_ring, input_box, switch, tag_frame};

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

/// A wrapping paragraph of small text.
fn paragraph(content: &'static str, size: f32, line_height: f32) -> Div {
    div()
        .font_family(theme::SANS)
        .font_weight(theme::weight(400.0))
        .text_size(px(size))
        .line_height(px(line_height))
        .text_color(theme::text_subtle())
        .child(content)
}

fn header(app: &PwrRoute, handler: &Handler, cx: &mut Context<'_, PwrRoute>) -> Div {
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
                .child(tracked(
                    &format!(
                        "Handler \u{00b7} {} of {}",
                        app.selected + 1,
                        app.handlers.len()
                    ),
                    9.5,
                    600.0,
                    0.14,
                    theme::text_muted(),
                ))
                .child(
                    text(handler.title, 15.0, 700.0, theme::text_primary())
                        .mt(px(6.0))
                        .line_height(px(18.0)),
                ),
        )
        .child(
            div()
                .id("handler-enabled")
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.toggle_selected(cx)))
                .child(switch(handler.rule.enabled)),
        )
}

fn match_section(handler: &Handler) -> Div {
    let patterns =
        handler
            .patterns
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
        .child(
            // `.addp`, with its `.note`.
            div()
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
                ),
        )
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

fn try_it_section(app: &PwrRoute, handler: &Handler) -> Div {
    // The sample link is a contrived Atlassian URL: handler 2 matches it, so
    // the rail says who wins from the selected handler's point of view.
    const SAMPLE_WINNER: usize = 1;
    let browser = match &handler.rule.destination {
        Destination::Browser { name, .. } => *name,
        Destination::Rewrite { .. } => "",
    };
    let (result, status) = match app.selected.cmp(&SAMPLE_WINNER) {
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
        .child(focus_ring(input_box().child(mono(
            "https://example.atlassian.net/browse/PG-12",
            12.0,
            500.0,
            theme::text_primary(),
        ))))
        .child(
            // `.try`
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
                ),
        )
}

/// `.rrail`.
pub fn rail(app: &PwrRoute, cx: &mut Context<'_, PwrRoute>) -> impl IntoElement + use<> {
    let Some(handler) = app.handlers.get(app.selected) else {
        return div().w(px(WIDTH)).flex_none();
    };
    div()
        .w(px(WIDTH))
        .flex_none()
        .flex()
        .flex_col()
        .border_l_1()
        .border_color(theme::border_subtle())
        .bg(theme::bg_panel())
        .overflow_hidden()
        .child(header(app, handler, cx))
        .child(match_section(handler))
        .child(open_in_section(handler))
        .child(try_it_section(app, handler))
        .child(
            // `.rail-end`
            div()
                .mt_auto()
                .flex()
                .items_center()
                .justify_between()
                .py(px(12.0))
                .px(px(16.0))
                .border_t_1()
                .border_color(theme::border_faint())
                .child(div().py(px(5.0)).px(px(9.0)).child(text(
                    "Delete handler",
                    11.0,
                    550.0,
                    theme::danger_text(),
                )))
                .child(text(
                    "edited 09:41 \u{00b7} saved",
                    10.5,
                    400.0,
                    theme::text_subtle(),
                )),
        )
}
