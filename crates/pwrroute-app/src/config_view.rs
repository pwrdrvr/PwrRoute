//! The Rules screen while a Finicky JS or TS config is in charge (design
//! option 1b): a banner with the ways out, the config's handlers read-only,
//! and the visual rules folded below, since they run after it.

use std::path::Path;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Div, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};

use crate::app::PwrRoute;
use crate::fixtures::{Destination, Matcher, Rule};
use crate::rules_view::{
    RowState, card, card_header_with, card_title, pane_frame, rule_row, visual_cards,
};
use crate::theme::{self, icon, text};
use crate::widgets::{ButtonKind, Tone, button, fn_tag, paragraph, toned_chip};

/// What the screen shows about the config.
#[derive(Clone, Debug)]
pub struct ConfigInCharge {
    /// The config's path as the user knows it: `~/.finicky.ts`.
    pub path: String,
    pub edited: &'static str,
    pub handlers: Vec<Rule>,
    pub default_browser: Destination,
}

impl ConfigInCharge {
    /// How many handlers only JS can express.
    pub fn need_js(&self) -> usize {
        self.handlers
            .iter()
            .filter(|rule| {
                rule.matchers
                    .iter()
                    .any(|m| matches!(m, Matcher::Function(_)))
            })
            .count()
    }
}

/// `path` with the home directory written `~`, as the design names configs.
pub fn display_path(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// `.banner`
fn banner(config: &ConfigInCharge) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(14.0))
        .py(px(12.0))
        .px(px(14.0))
        .border_1()
        .border_color(theme::accent_border())
        .rounded(px(10.0))
        .bg(theme::bg_row_active())
        .child(icon("pwrroute/code.svg", 18.0, theme::accent()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    text(
                        format!("{} is in charge", config.path),
                        12.5,
                        600.0,
                        theme::text_primary(),
                    )
                    .line_height(px(16.25)),
                )
                .child(paragraph(
                    format!(
                        "Its rewrites and handlers run first and it owns the default browser. \
                         Visual rules run after it. {}",
                        config.edited
                    ),
                    11.0,
                    16.5,
                    theme::text_muted(),
                )),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.0))
                .child(button("Open in editor", ButtonKind::Default, true))
                .child(button(
                    "Import to visual rules\u{2026}",
                    ButtonKind::Default,
                    true,
                ))
                .child(button("Stop using", ButtonKind::Quiet, true)),
        )
}

pub fn pane(
    app: &PwrRoute,
    config: &ConfigInCharge,
    cx: &mut Context<'_, PwrRoute>,
) -> impl IntoElement + use<> {
    let read_only = |last| RowState {
        last,
        read_only: true,
        ..RowState::default()
    };
    let need_js = config.need_js();
    let handlers = config.handlers.iter().enumerate().fold(
        card().child(card_header_with(
            &format!("From {} \u{00b7} read-only", config.path),
            "Handlers",
            toned_chip(format!("{need_js} need JS"), Tone::Warn),
        )),
        |card, (index, rule)| {
            card.child(rule_row(
                ElementId::named_usize("config-handler", index),
                &format!("{:02}", index + 1),
                rule,
                read_only(false),
                None,
            ))
        },
    );
    let default_row = rule_row(
        "config-default",
        "",
        &Rule {
            matchers: Vec::new(),
            destination: config.default_browser.clone(),
            hits: "",
            enabled: true,
        },
        read_only(true),
        Some(div().flex_1().min_w_0().child(text(
            "defaultBrowser",
            11.0,
            600.0,
            theme::text_primary(),
        ))),
    )
    .py(px(9.0));
    let handlers = handlers.child(default_row).child(
        // `.card-foot`
        div()
            .flex()
            .items_center()
            .gap(px(10.0))
            .py(px(9.0))
            .px(px(12.0))
            .border_t_1()
            .border_color(theme::border_faint())
            .child(fn_tag())
            .child(paragraph(
                "Function matchers and transformers run as written. The importer offers the \
                 visual form only where it can prove the same result.",
                10.5,
                14.7,
                theme::text_subtle(),
            )),
    );

    let expanded = app.visual_rules_expanded;
    let visual = card().child(
        div()
            .flex()
            .items_start()
            .gap(px(12.0))
            .py(px(13.0))
            .px(px(16.0))
            .child(
                card_title("Run after the config", "Visual rules").child(
                    paragraph(
                        format!(
                            "{} rewrites, {} handlers. The default browser is set by the \
                             config while it is selected.",
                            app.rewrites.len(),
                            app.handlers.len()
                        ),
                        11.5,
                        18.4,
                        theme::text_muted(),
                    )
                    .mt(px(6.0))
                    .max_w(px(430.0)),
                ),
            )
            .child(
                button(
                    if expanded { "Collapse" } else { "Expand" },
                    ButtonKind::Quiet,
                    true,
                )
                .id("visual-rules-toggle")
                .debug_selector(|| "visual-rules-toggle".into())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.visual_rules_expanded = !this.visual_rules_expanded;
                    cx.notify();
                })),
            ),
    );

    pane_frame("config-pane")
        .child(banner(config))
        .child(handlers)
        .child(visual)
        .when(expanded, |pane| pane.child(visual_cards(app, cx)))
}
