//! The Rules screen while a Finicky JS or TS config is in charge (design
//! option 1b, and Overview and Setup 5a-5c): a banner with the ways out and
//! the switch "Let PwrRoute's default take everything else", the config's
//! handlers read-only, and the visual rules folded. The cards read in
//! evaluation order: the visual rules after the config's handlers, or
//! before them once the switch puts `rules.json`'s `precedence` at
//! `"pwrroute"`, with PwrRoute's default as the last row.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::Context as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, Context, Div, ElementId, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    div, ease_out_quint, px,
};
use pwrroute_engine::{
    AppType, BrowserSpec, Description, Engine, FunctionSpec, HandlerRule, MatcherSpec, Precedence,
    RewriteRule, UrlSpec,
};

use crate::app::PwrRoute;
use crate::rules_model::{Change, Destination, Matcher, Rule};
use crate::rules_view::{
    RowState, card, card_header_with, card_title, everything_else_name, pane_frame, rule_row,
    visual_cards,
};
use crate::theme::{self, icon, mono, text};
use crate::widgets::{
    ButtonKind, NOT_OPENED, NOT_OPENED_BADGE, Tone, badge_for, button, fn_tag, paragraph, switch,
    tag_frame, toned_chip,
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
    /// What kind of `defaultBrowser` the config has, for the switch's
    /// warnings.
    pub default_kind: DefaultKind,
    /// The first handler whose match takes every link (`"*"`), by
    /// position: PwrRoute's default can never apply after it.
    pub catch_all: Option<usize>,
    /// How many times the switch flipped since this view of the config was
    /// made. Zero draws the switch and the banner still; each flip plays
    /// the thumb's slide and the sentence's fade once.
    pub flips: usize,
}

/// A config's `defaultBrowser`, as the switch's warnings need it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DefaultKind {
    /// A fixed browser, app or path.
    #[default]
    Browser,
    /// A function that picks the browser per link: it stops running under
    /// PwrRoute's default.
    Function,
    /// `null`: links nothing claims are not opened.
    NotOpened,
}

impl DefaultKind {
    fn of(browser: &BrowserSpec) -> Self {
        match browser {
            BrowserSpec::Browser(fixed) if fixed.app_type == AppType::None => Self::NotOpened,
            BrowserSpec::Browser(_) => Self::Browser,
            BrowserSpec::Function(_) => Self::Function,
        }
    }
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
                default_kind: DefaultKind::Browser,
                catch_all: None,
                flips: 0,
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
            default_kind: DefaultKind::of(&description.default_browser),
            catch_all: description
                .handlers
                .iter()
                .position(|handler| handler.matchers.iter().any(matches_every_link)),
            flips: 0,
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

    /// What changes hands when PwrRoute's default takes everything else
    /// (design 5c): each warning the config's description gives, in the
    /// banner's words. Empty for a config that did not load.
    pub fn override_warnings(&self) -> Vec<String> {
        if self.problem.is_some() {
            return Vec::new();
        }
        let mut warnings = Vec::new();
        match self.default_kind {
            DefaultKind::Browser => {}
            DefaultKind::Function => {
                warnings.push("Its defaultBrowser function will no longer run.".to_owned());
            }
            DefaultKind::NotOpened => {
                warnings.push("Unmatched links will now open instead of being dropped.".to_owned())
            }
        }
        if let Some(index) = self.catch_all {
            warnings.push(format!(
                "Handler {} matches every link, so PwrRoute's default never applies.",
                index + 1
            ));
        }
        warnings
    }

    /// The banner's sentence (design 5a/5b): who runs first and who takes
    /// everything else, then the load error or how long ago the file
    /// changed. `everything_else` names PwrRoute's default.
    pub fn banner_sentence(
        &self,
        rules_first: bool,
        everything_else: &str,
        now: SystemTime,
    ) -> String {
        let order = if rules_first {
            format!(
                "Your rules run first, then its handlers. PwrRoute's default, \
                 {everything_else}, takes everything else."
            )
        } else {
            "Its rewrites and handlers run first and its defaultBrowser takes everything \
             else. Your visual rules run after it."
                .to_owned()
        };
        match self.status(now) {
            status if status.is_empty() => order,
            status => format!("{order} {status}"),
        }
    }

    /// Who takes everything else while the visual rules run after the
    /// config: the visual card's second sentence.
    pub fn owner_sentence(&self, everything_else: &str) -> String {
        let path = &self.path;
        if self.problem.is_some() {
            return format!(
                "PwrRoute's default, {everything_else}, takes everything else until {path} \
                 loads."
            );
        }
        match (self.default_kind, &self.default_browser) {
            (DefaultKind::Function, _) => {
                format!("{path}'s defaultBrowser function picks where everything else goes.")
            }
            (DefaultKind::NotOpened, _) => {
                format!("{path}'s defaultBrowser is null, so links nothing claims are dropped.")
            }
            (DefaultKind::Browser, Some(Destination::Browser { name, profile, .. })) => {
                let browser = match profile {
                    Some(profile) => format!("{name} \u{00b7} {profile}"),
                    None => name.to_string(),
                };
                format!("{path}'s defaultBrowser, {browser}, takes everything else.")
            }
            (DefaultKind::Browser, _) => {
                format!("{path}'s defaultBrowser takes everything else.")
            }
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

/// A match that takes every link: Finicky's `"*"` (or `"*://*"`-like
/// patterns of nothing but wildcards and separators), or a regex that
/// matches anything.
fn matches_every_link(spec: &MatcherSpec) -> bool {
    match spec {
        MatcherSpec::Wildcard(pattern) => {
            let pattern = pattern.trim();
            pattern.contains('*') && pattern.chars().all(|c| matches!(c, '*' | ':' | '/'))
        }
        MatcherSpec::Regex(pattern) => {
            let body = pattern
                .strip_prefix('/')
                .and_then(|rest| rest.rsplit_once('/'))
                .map_or(pattern.as_str(), |(body, _flags)| body);
            matches!(body, ".*" | "^.*" | ".*$" | "^.*$" | ".+" | "^.+$")
        }
        MatcherSpec::Function(_) | MatcherSpec::Other(_) => false,
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
/// The banner's "Open in editor".
pub const OPEN_ID: &str = "config-open";
/// The banner's "Stop using".
pub const STOP_ID: &str = "config-stop-using";
/// The banner's switch row, "Let PwrRoute's default take everything else".
pub const SWITCH_ID: &str = "config-precedence";

/// The switch thumb's slide (design: Motion, Rules).
const SWITCH_SLIDE: Duration = Duration::from_millis(120);
/// The banner sentence's fade when the switch flips.
const SENTENCE_FADE: Duration = Duration::from_millis(150);

/// The switch's one name, on the banner and in Settings.
const SWITCH_LABEL: &str = "Let PwrRoute's default take everything else";

impl PwrRoute {
    /// The banner's switch: `rules.json`'s `precedence` flips between the
    /// config's order (absent) and `"pwrroute"`, and is written at once,
    /// so the router takes it on the next link.
    pub fn toggle_precedence(&mut self, cx: &mut Context<'_, Self>) {
        // Another writer's change is read first, so the flip lands on the
        // file as it now is.
        self.sync_with_disk(cx);
        let next = if self.store.precedence().is_config() {
            Precedence::PwrRoute
        } else {
            Precedence::Config
        };
        self.store.set_precedence(next);
        if let Some(config) = &mut self.config {
            config.flips += 1;
        }
        self.refresh(cx);
        self.save_now(cx);
    }

    /// "Stop using": the router reads no config from now on (`settings.json`
    /// `source.visualOnly`, as Settings › Configuration's Visual rules), and
    /// the "Found …" notice does not come back for this file. The window
    /// turns to the visual rules. Settings › Configuration turns it back on.
    pub fn stop_using_config(&mut self, cx: &mut Context<'_, Self>) {
        let Some(file) = self.config.as_ref().map(|config| config.file.clone()) else {
            return;
        };
        let Some(store) = crate::settings_store::store(cx) else {
            return;
        };
        let mut source = store.read(cx).settings().source.clone();
        source.visual_only = true;
        if !source.dismissed.contains(&file) {
            source.dismissed.push(file);
        }
        if let Some(config) = crate::importer_view::set_source(source, cx) {
            self.set_config(config, cx);
        }
    }
}

/// `.tag.fn` with words: "Everything else", "PwrRoute's default".
fn accent_tag(label: &'static str) -> Div {
    tag_frame()
        .bg(theme::accent_soft())
        .border_color(theme::transparent())
        .child(mono(label, 10.5, 500.0, theme::accent_bright()))
}

/// `.sw`, whose thumb slides once each time the switch flips. A switch
/// that has not flipped is drawn still, as `widgets::switch` draws it.
fn precedence_switch(on: bool, flips: usize) -> AnyElement {
    if flips == 0 {
        return switch(on).into_any_element();
    }
    let (track, border, thumb) = if on {
        (theme::accent(), theme::transparent(), theme::accent_on())
    } else {
        (
            theme::bg_hover_strong(),
            theme::border_default(),
            theme::text_muted(),
        )
    };
    let (from, to) = if on { (2.0, 14.0) } else { (14.0, 2.0) };
    div()
        .relative()
        .flex_none()
        .w(px(30.0))
        .h(px(18.0))
        .rounded_full()
        .border_1()
        .bg(track)
        .border_color(border)
        .child(
            div()
                .absolute()
                .top(px(2.0))
                .left(px(to))
                .size(px(12.0))
                .rounded_full()
                .bg(thumb)
                .with_animation(
                    ElementId::named_usize("config-precedence-thumb", flips),
                    Animation::new(SWITCH_SLIDE).with_easing(ease_out_quint()),
                    move |thumb, delta| thumb.left(px(from + (to - from) * delta)),
                ),
        )
        .into_any_element()
}

/// `.warnl`: one line of what the switch changes, under it.
fn warning_line(index: usize, warning: String) -> Div {
    div()
        .debug_selector(move || format!("config-warning-{index}"))
        .flex()
        .items_center()
        .gap(px(6.0))
        .mt(px(2.0))
        .child(
            // `.wi`
            div()
                .flex_none()
                .size(px(14.0))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::warn_soft())
                .child(text("!", 9.0, 700.0, theme::warn_text())),
        )
        .child(
            paragraph(warning, 10.5, 15.75, theme::warn_text()).font_weight(theme::weight(500.0)),
        )
}

/// `.banner`: who is in charge and in what order, the switch and its
/// warnings, and the ways out.
fn banner(app: &PwrRoute, config: &ConfigInCharge, cx: &mut Context<'_, PwrRoute>) -> Div {
    let rules_first = !app.store.precedence().is_config();
    let sentence = paragraph(
        config.banner_sentence(
            rules_first,
            &everything_else_name(&app.store),
            SystemTime::now(),
        ),
        11.0,
        16.5,
        theme::text_muted(),
    );
    // The new sentence fades in when the switch flips; a window that just
    // opened shows it still.
    let sentence = if config.flips == 0 {
        sentence.into_any_element()
    } else {
        sentence
            .with_animation(
                ElementId::named_usize("config-banner-sentence", config.flips),
                Animation::new(SENTENCE_FADE).with_easing(ease_out_quint()),
                |sentence, delta| sentence.opacity(delta),
            )
            .into_any_element()
    };
    let toggle = div()
        .id(SWITCH_ID)
        .debug_selector(|| SWITCH_ID.into())
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .h(px(18.0))
        .mt(px(6.0))
        .cursor_pointer()
        .child(precedence_switch(rules_first, config.flips))
        .child(text(SWITCH_LABEL, 11.5, 600.0, theme::text_primary()).line_height(px(14.95)))
        .on_click(cx.listener(|this, _, _, cx| this.toggle_precedence(cx)));
    let warnings = config
        .override_warnings()
        .into_iter()
        .enumerate()
        .map(|(index, warning)| warning_line(index, warning));
    let import = config.file.clone();
    let open = config.file.clone();
    div()
        .flex()
        .flex_none()
        .items_start()
        .gap(px(14.0))
        .py(px(12.0))
        .px(px(14.0))
        .border_1()
        .border_color(theme::accent_border())
        .rounded(px(10.0))
        .bg(theme::bg_row_active())
        .child(
            div()
                .mt(px(1.0))
                .child(icon("pwrroute/code.svg", 18.0, theme::accent())),
        )
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
                .child(sentence)
                .child(toggle)
                .children(warnings),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.0))
                .child(
                    button("Open in editor", ButtonKind::Default, true)
                        .id(OPEN_ID)
                        .debug_selector(|| OPEN_ID.into())
                        .on_click(move |_, _, cx| cx.open_with_system(&open)),
                )
                .child(
                    button("Import to visual rules\u{2026}", ButtonKind::Default, true)
                        .id(IMPORT_ID)
                        .debug_selector(|| IMPORT_ID.into())
                        .on_click(cx.listener(move |_, _, _, cx| {
                            let file = import.clone();
                            cx.defer(move |cx| crate::importer_view::open(file, false, cx));
                        })),
                )
                .child(
                    button("Stop using", ButtonKind::Quiet, true)
                        .id(STOP_ID)
                        .debug_selector(|| STOP_ID.into())
                        .on_click(cx.listener(|this, _, _, cx| this.stop_using_config(cx))),
                ),
        )
}

/// "1 rewrite", "3 rules".
fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

pub fn pane(
    app: &PwrRoute,
    config: &ConfigInCharge,
    cx: &mut Context<'_, PwrRoute>,
) -> impl IntoElement + use<> {
    let rules_first = !app.store.precedence().is_config();
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
    // first, as they run first in either order.
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
    let handlers_from = if rules_first {
        format!("{from} \u{00b7} runs after your rules")
    } else {
        from.clone()
    };
    let handlers = rows(
        card()
            .debug_selector(|| "config-handlers".into())
            .child(card_header_with(
                &handlers_from,
                "Handlers",
                toned_chip(format!("{need_js} need JS"), Tone::Warn),
            )),
        "config-handler",
        &config.handlers,
    );
    let default_row = config.default_browser.as_ref().map(|browser| {
        let rule = Rule::plain(Vec::new(), browser.clone());
        if rules_first {
            // `.rr.ovr`: the config's default, not used, and why.
            rule_row(
                "config-default",
                "",
                &rule,
                RowState {
                    struck: true,
                    ..read_only(true)
                },
                Some(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(7.0))
                        .child(
                            text("defaultBrowser", 11.0, 500.0, theme::text_subtle())
                                .line_through(),
                        )
                        .child(
                            crate::widgets::one_line(
                                "not used: PwrRoute's default takes everything else",
                                theme::SANS,
                                500.0,
                                10.5,
                                13.65,
                                theme::text_muted(),
                            )
                            .flex_1(),
                        ),
                ),
            )
            .debug_selector(|| "config-default-struck".into())
            .bg(theme::bg_elevated())
        } else {
            rule_row(
                "config-default",
                "",
                &rule,
                read_only(true),
                Some(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .child(text("defaultBrowser", 11.0, 600.0, theme::text_primary()))
                        .child(accent_tag("Everything else")),
                ),
            )
            .debug_selector(|| "config-default".into())
        }
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
    // `.rr.ee`: PwrRoute's default, last, while it takes everything else:
    // under the switch, or while the config does not load.
    let everything_else = (rules_first || config.problem.is_some()).then(|| {
        card()
            .debug_selector(|| "config-everything-else".into())
            .child(rule_row(
                "config-everything-else-row",
                "\u{221e}",
                &app.store.everything_else(),
                RowState {
                    everything_else: true,
                    ..read_only(true)
                },
                Some(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .child(text("Everything else", 11.0, 600.0, theme::text_primary()))
                        .child(accent_tag("PwrRoute's default")),
                ),
            ))
    });

    let expanded = app.visual_rules_expanded;
    let (eyebrow, order) = if rules_first {
        (
            "Run before the config's handlers",
            "They run before the config's handlers.".to_owned(),
        )
    } else {
        (
            "Run after the config",
            config.owner_sentence(&everything_else_name(&app.store)),
        )
    };
    let visual = card().debug_selector(|| "config-visual".into()).child(
        div()
            .flex()
            .items_start()
            .gap(px(12.0))
            .py(px(13.0))
            .px(px(16.0))
            .child(
                card_title(eyebrow, "Visual rules").child(
                    paragraph(
                        format!(
                            "{}, {}. {order}",
                            plural(app.rewrites.len(), "rewrite"),
                            plural(app.handlers.len(), "rule"),
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
    let visual = div()
        .flex()
        .flex_col()
        .gap(px(18.0))
        .child(visual)
        .when(expanded, |visual| visual.child(visual_cards(app, cx)));

    // In evaluation order (design 5b): the cards swap at once when the
    // switch flips.
    let (first, second) = if rules_first {
        (Some(visual), None)
    } else {
        (None, Some(visual))
    };
    pane_frame("config-pane")
        .children(crate::importer_view::rules_banners(app, cx))
        .child(banner(app, config, cx))
        .children(rewrites)
        .children(first)
        .children(handlers)
        .children(second)
        .children(everything_else)
}
