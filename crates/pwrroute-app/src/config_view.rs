//! The Rules screen while a Finicky JS or TS config is in charge (design
//! option 1b): a banner with the ways out, the config's handlers read-only,
//! and the visual rules folded below, since they run after it.

use std::path::Path;
use std::rc::Rc;
use std::time::SystemTime;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Div, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, px,
};
use pwrroute_engine::{
    AppType, BrowserSpec, DefaultHost, Description, Engine, FunctionSpec, Limits, MatcherSpec,
    UrlSpec,
};

use crate::app::PwrRoute;
use crate::fixtures::{Destination, Matcher, Rule};
use crate::rules_view::{
    RowState, card, card_header_with, card_title, pane_frame, rule_row, visual_cards,
};
use crate::theme::{self, icon, text};
use crate::widgets::{ButtonKind, Tone, badge_for, button, fn_tag, paragraph, toned_chip};

/// What the screen shows about the config: what the engine reads in it.
#[derive(Clone, Debug)]
pub struct ConfigInCharge {
    /// The config's path as the user knows it: `~/.finicky.ts`.
    pub path: String,
    /// When the file last changed, or why the engine could not read it.
    pub status: String,
    pub rewrites: Vec<Rule>,
    pub handlers: Vec<Rule>,
    /// `None` when the config did not load.
    pub default_browser: Option<Destination>,
}

impl ConfigInCharge {
    /// Loads the config at `path` in its own engine and describes it. `home`
    /// shortens the path to `~/…`. A config that does not load is shown
    /// with the error and no rows.
    pub fn load(path: &Path, home: Option<&Path>) -> Self {
        let shown = display_path(path, home);
        let described = Engine::load(path, Rc::new(DefaultHost), Limits::default())
            .and_then(|engine| engine.describe());
        match described {
            Ok(Some(description)) => Self::from_description(shown, edited(path), &description),
            Ok(None) => Self::failed(shown, "It has no rules to show.".into()),
            Err(error) => Self::failed(shown, format!("It did not load: {error:#}")),
        }
    }

    /// The rows for an engine's description of the config.
    pub fn from_description(path: String, status: String, description: &Description) -> Self {
        Self {
            path,
            status,
            rewrites: description
                .rewrites
                .iter()
                .map(|rewrite| {
                    let destination = match &rewrite.url {
                        UrlSpec::Url(url) => Destination::Rewrite {
                            kind: "url",
                            value: url.clone().into(),
                        },
                        UrlSpec::Function(function) => Destination::Function(source(function)),
                    };
                    row(&rewrite.matchers, destination)
                })
                .collect(),
            handlers: description
                .handlers
                .iter()
                .map(|handler| row(&handler.matchers, destination(&handler.browser)))
                .collect(),
            default_browser: Some(destination(&description.default_browser)),
        }
    }

    fn failed(path: String, status: String) -> Self {
        Self {
            path,
            status,
            rewrites: Vec::new(),
            handlers: Vec::new(),
            default_browser: None,
        }
    }

    /// How many handlers only JS can express: a function matches, or picks
    /// the browser.
    pub fn need_js(&self) -> usize {
        self.handlers
            .iter()
            .filter(|rule| {
                matches!(rule.destination, Destination::Function(_))
                    || rule
                        .matchers
                        .iter()
                        .any(|m| matches!(m, Matcher::Function(_)))
            })
            .count()
    }
}

fn row(matchers: &[MatcherSpec], destination: Destination) -> Rule {
    Rule {
        matchers: matchers.iter().map(matcher).collect(),
        destination,
        hits: "",
        enabled: true,
    }
}

fn matcher(spec: &MatcherSpec) -> Matcher {
    match spec {
        MatcherSpec::Wildcard(pattern) => Matcher::Pattern(pattern.clone().into()),
        MatcherSpec::Regex(pattern) => Matcher::Regex(pattern.clone().into()),
        MatcherSpec::Function(function) => Matcher::Function(source(function)),
        // Not something Finicky matches with; the config's validation
        // normally rejects it. Shown by its JS type.
        MatcherSpec::Other(kind) => Matcher::Pattern(format!("({kind})").into()),
    }
}

fn destination(browser: &BrowserSpec) -> Destination {
    match browser {
        BrowserSpec::Browser(fixed) if fixed.app_type == AppType::None => Destination::Browser {
            badge: "\u{2014}".into(),
            name: "Not opened".into(),
            profile: None,
        },
        BrowserSpec::Browser(fixed) => Destination::Browser {
            badge: badge_for(&fixed.name),
            name: fixed.name.clone().into(),
            profile: (!fixed.profile.is_empty()).then(|| fixed.profile.clone().into()),
        },
        BrowserSpec::Function(function) => Destination::Function(source(function)),
    }
}

/// A function's source on one line, or its name when QuickJS has no source.
fn source(function: &FunctionSpec) -> SharedString {
    match &function.source {
        Some(source) => source
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .into(),
        None if function.name.is_empty() => "function".into(),
        None => format!("function {}", function.name).into(),
    }
}

/// When the file at `path` last changed, as the banner says it.
fn edited(path: &Path) -> String {
    let age = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok());
    let Some(age) = age else {
        return String::new();
    };
    let minutes = age.as_secs() / 60;
    match minutes {
        0 => "Edited just now.".into(),
        1..60 => format!("Edited {minutes} min ago."),
        60..1440 => format!("Edited {} hr ago.", minutes / 60),
        _ => match minutes / 1440 {
            1 => "Edited yesterday.".into(),
            days => format!("Edited {days} days ago."),
        },
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
                        config.status
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
    let from = format!("From {} \u{00b7} read-only", config.path);
    let rows = |card: Div, id: &'static str, rules: &[Rule]| {
        rules.iter().enumerate().fold(card, |card, (index, rule)| {
            card.child(rule_row(
                ElementId::named_usize(id, index),
                &format!("{:02}", index + 1),
                rule,
                read_only(false),
                None,
            ))
        })
    };
    // The design's config has no rewrites; one that has them shows them
    // first, as they run first.
    let rewrites = (!config.rewrites.is_empty()).then(|| {
        rows(
            card().child(card_header_with(
                &from,
                "Rewrites",
                toned_chip(config.rewrites.len().to_string(), Tone::Plain),
            )),
            "config-rewrite",
            &config.rewrites,
        )
    });
    let need_js = config.need_js();
    let handlers = rows(
        card().child(card_header_with(
            &from,
            "Handlers",
            toned_chip(format!("{need_js} need JS"), Tone::Warn),
        )),
        "config-handler",
        &config.handlers,
    );
    let default_row = config.default_browser.as_ref().map(|browser| {
        rule_row(
            "config-default",
            "",
            &Rule {
                matchers: Vec::new(),
                destination: browser.clone(),
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
        .py(px(9.0))
    });
    let handlers = handlers.children(default_row).child(
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
        .children(rewrites)
        .child(handlers)
        .child(visual)
        .when(expanded, |pane| pane.child(visual_cards(app, cx)))
}
