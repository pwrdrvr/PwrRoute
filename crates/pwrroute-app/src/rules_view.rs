//! The middle pane: the heading, the rewrites and handlers cards, the
//! catch-all row and the file footer.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Div, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, Stateful,
    StatefulInteractiveElement as _, Styled as _, div, px, svg,
};

use crate::app::PwrRoute;
use crate::fixtures::{self, Destination, Matcher, Rule};
use crate::theme::{self, mono, text, tracked};
use crate::widgets::{action_tag, add_button, app_tag, badge, chip, regex_tag, tag};

/// Widths of a row's fixed columns. The design lays rows out as the grid
/// `12px 22px minmax(0,1fr) 14px minmax(0,230px) 64px 20px`; at this pane
/// width the second minmax track resolves to its 230px maximum, so fixed
/// flex columns give the same tracks.
mod column {
    pub const GRIP: f32 = 12.0;
    pub const INDEX: f32 = 22.0;
    pub const ARROW: f32 = 14.0;
    pub const DESTINATION: f32 = 230.0;
    pub const HITS: f32 = 64.0;
    pub const MORE: f32 = 20.0;
}

fn matcher(matcher: &Matcher) -> Div {
    match matcher {
        Matcher::Pattern(pattern) => tag(pattern),
        Matcher::Regex(pattern) => regex_tag(pattern),
        Matcher::Label(label) => text(*label, 11.0, 500.0, theme::text_muted()),
        Matcher::App { badge, name } => app_tag(badge, name),
        Matcher::Off => chip("off"),
    }
}

fn destination(destination: &Destination) -> Div {
    match destination {
        Destination::Rewrite { kind, value } => div()
            .flex()
            .items_center()
            .min_w_0()
            .child(action_tag(kind, value)),
        Destination::Browser {
            badge: badge_label,
            name,
            profile,
        } => div()
            .flex()
            .items_center()
            .gap(px(7.0))
            .min_w_0()
            .child(badge(badge_label))
            .child(text(*name, 12.0, 500.0, theme::text_primary()))
            .when_some(*profile, |row, profile| {
                row.child(mono(profile, 10.5, 500.0, theme::text_muted()))
            }),
    }
}

/// How a row reads besides its content.
#[derive(Clone, Copy, Default)]
struct RowState {
    selected: bool,
    last: bool,
}

/// `.rr`: grip, index, matchers, arrow, destination, hits, menu.
///
/// `matchers` replaces the rule's own match column when given, for the
/// catch-all row whose match is a label rather than patterns.
fn rule_row(
    id: impl Into<ElementId>,
    index: &str,
    rule: &Rule,
    state: RowState,
    matchers: Option<Div>,
) -> Stateful<Div> {
    // `.rr.off > * { opacity: .5 }`
    let dim = |cell: Div| {
        if rule.enabled {
            cell
        } else {
            cell.opacity(0.5)
        }
    };
    let grip = !rule.matchers.is_empty();
    let matchers = matchers.unwrap_or_else(|| {
        rule.matchers.iter().fold(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(5.0)),
            |cell, item| cell.child(matcher(item)),
        )
    });
    div()
        .id(id)
        .relative()
        .flex()
        .items_center()
        .gap(px(10.0))
        .py(px(9.0))
        .pl(px(10.0))
        .pr(px(12.0))
        .when(!state.last, |row| {
            row.border_b_1().border_color(theme::border_faint())
        })
        .map(|row| {
            if state.selected {
                // `.rr.sel` and its `::before` bar.
                row.bg(theme::bg_row_active()).child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(6.0))
                        .bottom(px(6.0))
                        .w(px(3.0))
                        .rounded_r(px(2.0))
                        .bg(theme::accent()),
                )
            } else {
                row.hover(|style| style.bg(theme::bg_hover()))
            }
        })
        .child(dim(div()
            .w(px(column::GRIP))
            .flex_none()
            .flex()
            .justify_center()
            .when(grip, |cell| {
                cell.child(
                    svg()
                        .path("pwrroute/grip.svg")
                        .w(px(7.0))
                        .h(px(12.0))
                        .text_color(theme::text_subtle()),
                )
            })))
        .child(dim(div().w(px(column::INDEX)).flex_none().child(mono(
            index.to_owned(),
            10.5,
            600.0,
            theme::text_subtle(),
        ))))
        .child(dim(matchers))
        .child(dim(div().w(px(column::ARROW)).flex_none().child(mono(
            "\u{2192}",
            12.0,
            500.0,
            theme::text_subtle(),
        ))))
        .child(dim(div()
            .w(px(column::DESTINATION))
            .flex_none()
            .min_w_0()
            .child(destination(&rule.destination))))
        .child(dim(div()
            .w(px(column::HITS))
            .flex_none()
            .flex()
            .justify_end()
            .child(mono(rule.hits, 10.5, 500.0, theme::text_subtle()))))
        .child(dim(div()
            .w(px(column::MORE))
            .flex_none()
            .flex()
            .justify_center()
            .child(mono("\u{22ef}", 12.0, 600.0, theme::text_subtle()))))
}

/// `.card`.
fn card() -> Div {
    div()
        .flex_none()
        .flex()
        .flex_col()
        .border_1()
        .border_color(theme::border_subtle())
        .rounded(px(10.0))
        .bg(theme::bg_panel())
        .overflow_hidden()
}

/// `.card-hd`: eyebrow, title and a count chip.
fn card_header(eyebrow: &str, title: &'static str, count: &'static str) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(12.0))
        .py(px(13.0))
        .px(px(16.0))
        .border_b_1()
        .border_color(theme::border_faint())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(tracked(eyebrow, 9.5, 600.0, 0.14, theme::text_muted()))
                .child(
                    text(title, 13.5, 600.0, theme::text_primary())
                        .mt(px(6.0))
                        .line_height(px(16.2)),
                ),
        )
        .child(chip(count))
}

/// `.pane-head`: what this screen is, and the add buttons.
fn heading() -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(16.0))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(tracked("Routing", 9.5, 600.0, 0.14, theme::accent()))
                .child(
                    text("Rules", 19.0, 700.0, theme::text_primary())
                        .mt(px(6.0))
                        .line_height(px(22.8)),
                )
                .child(
                    div()
                        .mt(px(6.0))
                        // 56ch: Geist's "0" is about 0.6em, 6.9px at 11.5px.
                        .max_w(px(386.0))
                        .font_family(theme::SANS)
                        .font_weight(theme::weight(400.0))
                        .text_size(px(11.5))
                        .line_height(px(18.4))
                        .text_color(theme::text_muted())
                        .child(
                            "Rewrites run in order and every match applies. Then the first \
                             matching handler opens the link. Anything left goes to the default.",
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(add_button("Add rewrite", false))
                .child(add_button("Add handler", true)),
        )
}

/// `.pane`. Scrolls: at the default window height the handlers card runs
/// past the bottom, which the design clips.
pub fn pane(app: &PwrRoute, cx: &mut Context<'_, PwrRoute>) -> impl IntoElement + use<> {
    let rewrites = app.rewrites.iter().enumerate().fold(
        card().child(card_header(
            "Step 1 \u{00b7} run in order \u{00b7} every match applies",
            "Rewrites",
            "3 rewrites",
        )),
        |card, (index, rule)| {
            let state = RowState {
                selected: false,
                last: index + 1 == app.rewrites.len(),
            };
            card.child(rule_row(
                ElementId::named_usize("rewrite", index),
                &format!("{:02}", index + 1),
                rule,
                state,
                None,
            ))
        },
    );

    let handlers = app.handlers.iter().enumerate().fold(
        card().child(card_header(
            "Step 2 \u{00b7} first match wins",
            "Handlers",
            "7 handlers \u{00b7} 1 off",
        )),
        |card, (index, handler)| {
            let state = RowState {
                selected: index == app.selected,
                last: index + 1 == app.handlers.len(),
            };
            card.child(
                rule_row(
                    ElementId::named_usize("handler", index),
                    &format!("{:02}", index + 1),
                    &handler.rule,
                    state,
                    None,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    window.focus(&this.focus, cx);
                    this.select(index, cx);
                })),
            )
        },
    );

    let everything_else = card().child(rule_row(
        "everything-else",
        "\u{221e}",
        &fixtures::everything_else(),
        RowState {
            selected: false,
            last: true,
        },
        Some(div().flex_1().min_w_0().child(text(
            "Everything else",
            11.0,
            600.0,
            theme::text_primary(),
        ))),
    ));

    div()
        .id("rules-pane")
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(18.0))
        .py(px(22.0))
        .px(px(24.0))
        .overflow_y_scroll()
        .child(heading())
        .child(rewrites)
        .child(handlers)
        .child(everything_else)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(theme::dot(theme::status_ok()))
                .child(mono(
                    "rules.json \u{00b7} ~/Library/Application Support/PwrRoute \u{00b7} saved on change",
                    10.5,
                    500.0,
                    theme::text_subtle(),
                )),
        )
}
