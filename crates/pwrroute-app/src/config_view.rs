//! The Rules screen while a Finicky JS or TS config is in charge (design
//! option 1b): a banner with the ways out, the config's handlers read-only,
//! and the visual rules folded below, since they run after it.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Context as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Div, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, px,
};
use pwrroute_engine::{
    AppType, BrowserSpec, Description, Engine, FunctionSpec, HandlerRule, MatcherSpec, RewriteRule,
    UrlSpec,
};

use crate::app::PwrRoute;
use crate::rules_model::{Change, Destination, Matcher, Rule};
use crate::rules_view::{
    RowState, card, card_header_with, card_title, pane_frame, rule_row, visual_cards,
};
use crate::theme::{self, icon, text};
use crate::widgets::{
    ButtonKind, NOT_OPENED, NOT_OPENED_BADGE, Tone, badge_for, button, fn_tag, paragraph,
    toned_chip,
};

/// How much of a function's source or a load error a row or the banner
/// keeps. Rows end in an ellipsis long before this; it bounds layout work.
const MAX_TEXT: usize = 240;

/// What the screen shows about the config: what the engine reads in it.
#[derive(Clone, Debug)]
pub struct ConfigInCharge {
    /// The config's path as the user knows it: `~/.finicky.ts`.
    pub path: String,
    /// The file itself, for the importer.
    pub file: PathBuf,
    /// When the file last changed, for the banner to say how long ago.
    pub modified: Option<SystemTime>,
    /// Why the engine could not read the config; then there are no rows.
    pub problem: Option<String>,
    pub rewrites: Vec<Rule>,
    pub handlers: Vec<Rule>,
    /// `None` when the config did not load.
    pub default_browser: Option<Destination>,
}

impl ConfigInCharge {
    /// Describes the config at `path` from the engine that loaded it, or
    /// says why it did not load. `home` shortens the path to `~/…`.
    pub fn new(path: &Path, home: Option<&Path>, engine: Result<&Engine, &anyhow::Error>) -> Self {
        let shown = display_path(path, home);
        let described = engine
            .map_err(|error| anyhow::anyhow!("{error:#}"))
            .and_then(|engine| {
                engine
                    .describe()?
                    .context("the engine has no JS config to describe")
            });
        match described {
            Ok(description) => Self::from_description(path, shown, modified(path), &description),
            Err(error) => Self {
                path: shown,
                file: path.to_path_buf(),
                modified: None,
                problem: Some(one_line(&format!("It did not load: {error:#}"))),
                rewrites: Vec::new(),
                handlers: Vec::new(),
                default_browser: None,
            },
        }
    }

    /// The rows for an engine's description of the config.
    pub fn from_description(
        file: &Path,
        path: String,
        modified: Option<SystemTime>,
        description: &Description,
    ) -> Self {
        Self {
            path,
            file: file.to_path_buf(),
            modified,
            problem: None,
            rewrites: description.rewrites.iter().map(rewrite_row).collect(),
            handlers: description.handlers.iter().map(handler_row).collect(),
            default_browser: Some(destination(&description.default_browser)),
        }
    }

    /// What the banner says after its fixed text: the load error, or how
    /// long ago the file changed, as of `now`.
    pub fn status(&self, now: SystemTime) -> String {
        if let Some(problem) = &self.problem {
            return problem.clone();
        }
        self.modified
            .and_then(|modified| now.duration_since(modified).ok())
            .map(edited)
            .unwrap_or_default()
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

/// A config rewrite's read-only row.
pub(crate) fn rewrite_row(rewrite: &RewriteRule) -> Rule {
    let destination = match &rewrite.url {
        UrlSpec::Url(url) => Destination::Rewrite(vec![Change {
            kind: "url",
            value: url.clone().into(),
        }]),
        UrlSpec::Function(function) => Destination::Function(source(function)),
    };
    row(&rewrite.matchers, destination)
}

/// A config handler's read-only row.
pub(crate) fn handler_row(handler: &HandlerRule) -> Rule {
    row(&handler.matchers, destination(&handler.browser))
}

fn row(matchers: &[MatcherSpec], destination: Destination) -> Rule {
    Rule::plain(matchers.iter().map(matcher).collect(), destination)
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

pub(crate) fn destination(browser: &BrowserSpec) -> Destination {
    match browser {
        BrowserSpec::Browser(fixed) if fixed.app_type == AppType::None => Destination::Browser {
            badge: NOT_OPENED_BADGE.into(),
            name: NOT_OPENED.into(),
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
        Some(source) => one_line(source).into(),
        None if function.name.is_empty() => "function".into(),
        None => format!("function {}", function.name).into(),
    }
}

/// `text` with each run of whitespace (newlines too) as one space, cut at
/// [`MAX_TEXT`] characters.
fn one_line(text: &str) -> String {
    let mut line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some((cut, _)) = line.char_indices().nth(MAX_TEXT) {
        line.truncate(cut);
        line.push('\u{2026}');
    }
    line
}

/// When the file at `path` last changed.
fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
}

/// How long ago a file changed, as the banner says it.
fn edited(age: std::time::Duration) -> String {
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
        Some(rest) => {
            let rest = rest.display().to_string();
            // Shown with "/" after the "~" on every platform, so a nested
            // path on Windows does not mix separators ("~/journal\\x").
            #[cfg(windows)]
            let rest = rest.replace('\\', "/");
            format!("~/{rest}")
        }
        None => path.display().to_string(),
    }
}

/// The banner's "Import to visual rules…".
pub const IMPORT_ID: &str = "config-import";

/// `.banner`
fn banner(config: &ConfigInCharge, cx: &mut Context<'_, PwrRoute>) -> Div {
    let file = config.file.clone();
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
                        config.status(SystemTime::now())
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
                .child(
                    button("Import to visual rules\u{2026}", ButtonKind::Default, true)
                        .id(IMPORT_ID)
                        .debug_selector(|| IMPORT_ID.into())
                        .on_click(cx.listener(move |_, _, _, cx| {
                            let file = file.clone();
                            cx.defer(move |cx| crate::importer_view::open(file, false, cx));
                        })),
                )
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
        card()
            .debug_selector(|| "config-handlers".into())
            .child(card_header_with(
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
            &Rule::plain(Vec::new(), browser.clone()),
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
    // A config that did not load has no rows to show: the banner says why.
    let handlers = config.problem.is_none().then(|| {
        handlers.children(default_row).child(
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
        )
    });

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
        .children(crate::importer_view::rules_banners(app, cx))
        .child(banner(config, cx))
        .children(rewrites)
        .children(handlers)
        .child(visual)
        .when(expanded, |pane| pane.child(visual_cards(app, cx)))
}
