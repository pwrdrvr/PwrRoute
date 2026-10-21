//! Test a link (design option 2a): a URL, the app it comes from, and the
//! trace of how the rules resolve it. Nothing is opened.
//!
//! The screen starts empty: no link and no app, so nothing on it looks
//! like something that happened. The app picker offers the apps the rules
//! name (the only ones that change an answer) and, marked as such, the
//! design's examples, so a "sent from" rule can be tried without the app.
//!
//! The result card's Save as test turns the run into a top-level `tests[]`
//! entry: the link, the app it came from, and the browser, profile and URL
//! that came out. That is a URL the user chose to write to `rules.json`, by
//! that click alone, and the card says so.
//!
//! The window runs its own [`Engine`]: over the Finicky config when one is
//! in charge, exactly as the router loads it, and otherwise over the
//! `rules.json` the Rules screen edits, as the file stands once every
//! saveable edit is written (`RulesStore::written`). It is built on the
//! first test and lives on the UI thread; QuickJS runtimes are not `Send`.

use std::{
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant, SystemTime},
};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClipboardItem, Context, Div, HighlightStyle, Hsla, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, SharedString, StatefulInteractiveElement as _,
    StrikethroughStyle, Styled as _, StyledText, Window, deferred, div, px,
};
use pwrroute_engine::{
    AppType, DefaultHost, Engine, Expectation, Limits, Link, RouteTest, RulesFile, Sources, Trace,
};

use crate::app::PwrRoute;
use crate::fixtures;
use crate::rules_view::{card, pane_frame, pane_heading};
use crate::text_field::{self, Edit, Look, TextField};
use crate::theme::{self, mono, text};
use crate::trace_view::{Chip, StepBody, StepState, TraceStep, TraceView, UrlMark, UrlPart};
use crate::widgets::{
    ButtonKind, action_tag, app_tag, badge, badge_for, button, caret, input_box, paragraph, tag,
};

/// An app the picker offers as the link's sender.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenerChoice {
    pub badge: SharedString,
    pub name: SharedString,
    pub bundle_id: SharedString,
}

impl From<fixtures::Opener> for OpenerChoice {
    fn from(app: fixtures::Opener) -> Self {
        Self {
            badge: app.badge.into(),
            name: app.name.into(),
            bundle_id: app.bundle_id.into(),
        }
    }
}

impl OpenerChoice {
    /// An app as a rule names it: a name or a bundle id, matched by the
    /// engine against either, so it serves as both here.
    fn as_written(written: &str) -> Self {
        Self {
            badge: badge_for(written),
            name: written.to_owned().into(),
            bundle_id: written.to_owned().into(),
        }
    }

    /// Whether `written`, from a rule, names this app.
    fn is_named(&self, written: &str) -> bool {
        self.name.eq_ignore_ascii_case(written) || self.bundle_id.eq_ignore_ascii_case(written)
    }
}

/// The last run, kept so folding and unfolding redraw without resolving
/// again.
struct Run {
    trace: Trace,
    opener: Option<OpenerChoice>,
    took: Duration,
}

/// The screen's state.
pub struct TestLink {
    pub url: TextField,
    /// Index into [`TestLink::openers`]: 0 is no app. The first entries
    /// are [`fixtures::OPENERS`], in order, which Activity's re-test
    /// counts on.
    pub opener: usize,
    /// The opener menu is open.
    pub picking: bool,
    /// The menu was open when the picker was pressed. The menu closes on
    /// any press outside it, before the picker's click, so the click needs
    /// to know whether it was open.
    was_picking: bool,
    /// Show each missed handler rather than one folded step.
    pub unfold: bool,
    /// Copy trace was pressed since the last run.
    pub copied: bool,
    /// The run was saved as this test (its index in `tests`) since the
    /// last run.
    pub saved_as: Option<usize>,
    config: Option<PathBuf>,
    rules: RulesFile,
    /// Handed in by the window, or built on the first test. A load error is
    /// not kept, so the next test tries again (after the config is fixed,
    /// say).
    engine: Option<Engine>,
    /// The engine's files and when each last changed, as it was built: a
    /// test after any of them changes builds it again.
    loaded: Vec<(PathBuf, Option<SystemTime>)>,
    run: Option<Run>,
    /// The trace of the last run, or why it failed.
    pub outcome: Option<Result<TraceView, String>>,
}

impl TestLink {
    /// `config` is the Finicky config in charge, if any, and `engine` the
    /// engine the window already loaded it in (config alone, as here).
    /// `rules` is the visual rules file as the router reads it.
    pub fn new(
        config: Option<PathBuf>,
        engine: Option<Engine>,
        rules: RulesFile,
        cx: &mut App,
    ) -> Self {
        // The router does not read `rules.json` yet; with a config in charge
        // it routes by the config alone, and so does this.
        let rules = if config.is_some() {
            RulesFile::default()
        } else {
            rules
        };
        Self {
            url: TextField::new("", cx),
            opener: 0,
            picking: false,
            was_picking: false,
            unfold: false,
            copied: false,
            saved_as: None,
            config,
            rules,
            loaded: engine.as_ref().map(stamps).unwrap_or_default(),
            engine,
            run: None,
            outcome: None,
        }
    }

    /// What the picker offers: no app, the design's examples, then every
    /// app the rules name that is not among them, as written.
    pub fn openers(&self) -> Vec<Option<OpenerChoice>> {
        let mut choices: Vec<Option<OpenerChoice>> = fixtures::OPENERS
            .iter()
            .map(|app| app.map(OpenerChoice::from))
            .collect();
        for written in self.named_openers() {
            let offered = choices
                .iter()
                .flatten()
                .any(|choice| choice.is_named(written));
            if !offered {
                choices.push(Some(OpenerChoice::as_written(written)));
            }
        }
        choices
    }

    /// The apps the rules require a link to come from, as written.
    fn named_openers(&self) -> impl Iterator<Item = &str> {
        self.rules
            .rules
            .iter()
            .flat_map(|rule| rule.opener.iter())
            .map(String::as_str)
            .map(str::trim)
            .filter(|written| !written.is_empty())
    }

    /// Whether a rule names this app: the picker groups those first.
    pub fn named_by_rules(&self, choice: &OpenerChoice) -> bool {
        self.named_openers().any(|written| choice.is_named(written))
    }

    pub fn opener(&self) -> Option<OpenerChoice> {
        self.openers().get(self.opener).cloned().flatten()
    }

    /// Resolves the field's URL and records the trace. An empty field is
    /// nothing to test, not an error.
    pub fn run(&mut self) {
        self.copied = false;
        self.saved_as = None;
        self.picking = false;
        // Each test starts folded, as the design draws it.
        self.unfold = false;
        if self.url.text.trim().is_empty() {
            self.run = None;
            self.outcome = None;
            return;
        }
        let url = match pwrroute_router::normalize_url(&self.url.text) {
            Ok(url) => url,
            Err(error) => {
                self.run = None;
                self.outcome = Some(Err(format!("Not a link the router accepts: {error}.")));
                return;
            }
        };
        let opener = self.opener();
        let engine_opener = opener.as_ref().map(|app| pwrroute_engine::Opener {
            name: app.name.to_string(),
            bundle_id: app.bundle_id.to_string(),
            ..pwrroute_engine::Opener::default()
        });
        let started = Instant::now();
        match self.trace(&url, engine_opener.as_ref()) {
            Ok(trace) => {
                self.run = Some(Run {
                    trace,
                    opener,
                    took: started.elapsed(),
                });
                self.redraw();
            }
            Err(error) => {
                self.run = None;
                self.outcome = Some(Err(error));
            }
        }
    }

    /// Resolves a normalized URL as from `opener`, building the engine
    /// first when it is missing or its files changed. The editor's Try it
    /// uses this too; the screen's own run is left alone.
    pub fn trace(
        &mut self,
        url: &str,
        opener: Option<&pwrroute_engine::Opener>,
    ) -> Result<Trace, String> {
        if self.loaded.iter().any(|(path, at)| modified(path) != *at) {
            self.engine = None;
        }
        if self.engine.is_none() {
            let sources = Sources {
                config: self.config.clone(),
                rules: self.rules.clone(),
            };
            match Engine::from_sources(&sources, Rc::new(DefaultHost), Limits::default()) {
                Ok(engine) => {
                    self.loaded = stamps(&engine);
                    self.engine = Some(engine);
                }
                Err(error) => return Err(format!("The rules did not load: {error:#}")),
            }
        }
        let Some(engine) = &self.engine else {
            return Err("The rules did not load.".into());
        };
        engine
            .trace(&Link::new(url), opener)
            .map_err(|error| format!("The rules failed: {error:#}"))
    }

    /// Rebuilds the view of the last run, as after folding or unfolding.
    fn redraw(&mut self) {
        if let Some(run) = &self.run {
            self.outcome = Some(Ok(TraceView::from_engine(
                &run.trace,
                &self.rules,
                run.opener.as_ref(),
                run.took,
                self.unfold,
            )));
        }
    }

    /// The run as a saved test would pin it: the link as tested, the app it
    /// came from, and the browser, profile and URL that came out. `None`
    /// before a run, or when the config chose not to open the link.
    pub fn proposed_test(&self) -> Option<RouteTest> {
        let run = self.run.as_ref()?;
        let browser = &run.trace.resolution.browser;
        if browser.app_type == AppType::None || browser.name.trim().is_empty() {
            return None;
        }
        let opener = run.opener.as_ref().map(|app| {
            if app.name == app.bundle_id {
                app.name.to_string()
            } else {
                format!("{}:{}", app.name, app.bundle_id)
            }
        });
        Some(RouteTest {
            url: run.trace.url.clone(),
            opener,
            expect: Expectation {
                browser: browser.name.clone(),
                profile: (!browser.profile.is_empty()).then(|| browser.profile.clone()),
                url: Some(browser.url.clone()),
            },
        })
    }

    /// The visual rules changed: test against the change. With a config in
    /// charge the visual rules are not loaded, so nothing changes. A change
    /// to the saved tests alone routes nothing differently, so the run on
    /// screen stays.
    pub fn set_rules(&mut self, rules: RulesFile) {
        if self.config.is_some() || self.rules == rules {
            return;
        }
        let routing_unchanged = RulesFile {
            tests: Vec::new(),
            ..self.rules.clone()
        } == RulesFile {
            tests: Vec::new(),
            ..rules.clone()
        };
        if routing_unchanged {
            self.rules = rules;
            return;
        }
        self.rules = rules;
        // An app only the old rules named is gone from the picker.
        if self.opener >= self.openers().len() {
            self.opener = 0;
        }
        // Rebuild on the next test, and run it again when the screen opens.
        self.engine = None;
        self.run = None;
        self.outcome = None;
    }

    /// The rules the engine resolves with, as the router reads them.
    #[cfg(test)]
    pub fn rules(&self) -> &RulesFile {
        &self.rules
    }

    pub fn set_unfold(&mut self, unfold: bool) {
        self.unfold = unfold;
        self.redraw();
    }

    /// The trace was run at least once.
    pub fn has_run(&self) -> bool {
        self.outcome.is_some()
    }
}

/// The engine's files and when each last changed.
fn stamps(engine: &Engine) -> Vec<(PathBuf, Option<SystemTime>)> {
    engine
        .dependencies()
        .into_iter()
        .map(|path| {
            let at = modified(&path);
            (path, at)
        })
        .collect()
}

/// When a file last changed, if it can be read.
fn modified(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The step circle's colors: background, border, text.
fn circle_colors(state: StepState) -> (Hsla, Hsla, Hsla) {
    match state {
        StepState::Plain | StepState::Miss => (
            theme::bg_elevated(),
            theme::border_default(),
            theme::text_muted(),
        ),
        StepState::Hit => (
            theme::accent_soft(),
            theme::accent_border(),
            theme::accent_bright(),
        ),
        StepState::End => (
            theme::success_soft(),
            theme::transparent(),
            theme::success_text(),
        ),
        StepState::Error => (
            theme::danger_soft(),
            theme::transparent(),
            theme::danger_text(),
        ),
    }
}

/// `.url`: a wrapping mono URL with this step's changes marked.
fn url(parts: &[UrlPart], color: Hsla) -> Div {
    let mut whole = String::new();
    let mut highlights = Vec::new();
    for part in parts {
        let range = whole.len()..whole.len() + part.text.len();
        whole.push_str(&part.text);
        match part.mark {
            UrlMark::Same => {}
            UrlMark::Changed => highlights.push((
                range,
                HighlightStyle {
                    color: Some(theme::accent_bright()),
                    background_color: Some(theme::accent_soft()),
                    ..HighlightStyle::default()
                },
            )),
            UrlMark::Deleted => highlights.push((
                range,
                HighlightStyle {
                    color: Some(theme::danger_text()),
                    strikethrough: Some(StrikethroughStyle {
                        thickness: px(1.0),
                        color: Some(theme::danger_text()),
                    }),
                    fade_out: Some(0.15),
                    ..HighlightStyle::default()
                },
            )),
        }
    }
    div()
        .min_w_0()
        .font_family(theme::mono_family(500.0))
        .font_weight(theme::weight(500.0))
        .text_size(px(11.5))
        .line_height(px(17.25))
        .text_color(color)
        .child(StyledText::new(whole).with_highlights(highlights))
}

fn chip(chip: &Chip) -> Div {
    match chip {
        Chip::Action(kind, value) => action_tag(kind.clone(), value.clone()),
        Chip::App(badge, name) => app_tag(badge.clone(), name.clone()),
        Chip::Plain(label) => tag(label.clone()),
    }
}

/// `.ts`: number, title and detail, and what the step did.
fn step_row(
    index: usize,
    count: usize,
    step: &TraceStep,
    cx: &mut Context<'_, PwrRoute>,
) -> impl IntoElement + use<> {
    let (fill, border, number_color) = circle_colors(step.state);
    let number = step
        .number
        .map_or_else(|| "\u{2713}".to_owned(), |n| n.to_string());
    let dim = |cell: Div| {
        if step.state == StepState::Miss {
            cell.opacity(0.55)
        } else {
            cell
        }
    };
    let first = index == 0;
    let last = index + 1 == count;
    let body = step.body.iter().fold(
        div().flex_1().min_w_0().flex().flex_col().gap(px(5.0)),
        |column, body| match body {
            StepBody::Chips(chips, note) => column.child(
                chips
                    .iter()
                    .fold(
                        div().flex().flex_wrap().items_center().gap(px(5.0)),
                        |row, item| row.child(chip(item)),
                    )
                    .when_some(note.clone(), |row, note| {
                        row.child(paragraph(note, 10.5, 15.75, theme::text_subtle()))
                    }),
            ),
            StepBody::Url(parts) => column.child(url(parts, theme::text_primary())),
            StepBody::Text(content) => column.child(paragraph(
                content.clone(),
                11.5,
                17.25,
                if step.state == StepState::Error {
                    theme::danger_text()
                } else {
                    theme::text_muted()
                },
            )),
            StepBody::Folded => column
                .child(paragraph("No match.", 11.5, 17.25, theme::text_muted()))
                .child(
                    div()
                        .id("trace-show-each")
                        .debug_selector(|| "trace-show-each".into())
                        .self_start()
                        .cursor_pointer()
                        .font_family(theme::SANS)
                        .font_weight(theme::weight(400.0))
                        .text_size(px(11.5))
                        .line_height(px(17.25))
                        .text_color(theme::accent_bright())
                        .underline()
                        .child("Show each")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.test.set_unfold(true);
                            cx.notify();
                        })),
                ),
        },
    );
    div()
        .relative()
        .flex()
        .gap(px(12.0))
        .py(px(10.0))
        .px(px(14.0))
        .when(!last, |row| {
            row.border_b_1().border_color(theme::border_faint())
        })
        // `.ts::before`: the line that joins the circles.
        .when(count > 1, |row| {
            row.child(
                div()
                    .absolute()
                    .left(px(24.0))
                    .w(px(1.0))
                    .bg(theme::border_subtle())
                    .map(|line| {
                        if first {
                            line.top(px(20.0)).bottom_0()
                        } else if last {
                            line.top_0().h(px(20.0))
                        } else {
                            line.top_0().bottom_0()
                        }
                    }),
            )
        })
        .child(
            div().w(px(22.0)).flex_none().child(
                div()
                    .size(px(20.0))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(fill)
                    .border_1()
                    .border_color(border)
                    // The circle hides the line behind it.
                    .child(mono(number, 9.5, 600.0, number_color)),
            ),
        )
        .child(dim(div()
            .w(px(150.0))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(3.0))
            .child(text(step.title.clone(), 11.5, 600.0, theme::text_secondary()).line_height(px(16.1)))
            .child(
                div()
                    .font_family(theme::mono_family(500.0))
                    .font_weight(theme::weight(500.0))
                    .text_size(px(10.0))
                    .line_height(px(13.0))
                    .text_color(theme::text_subtle())
                    .child(step.detail.clone()),
            )))
        .child(dim(body))
}

/// The opener picker and its menu: no app, the apps the rules name, and
/// the design's examples under their own heading.
fn opener_picker(app: &PwrRoute, cx: &mut Context<'_, PwrRoute>) -> impl IntoElement + use<> {
    let label = |opener: Option<&OpenerChoice>| -> Div {
        match opener {
            Some(app) => div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(badge(app.badge.clone()))
                .child(text(app.name.clone(), 12.5, 400.0, theme::text_primary())),
            None => div().child(text("no app", 12.5, 400.0, theme::text_subtle())),
        }
    };
    let choices = app.test.openers();
    let row = |index: usize, choice: Option<&OpenerChoice>| {
        let selected = index == app.test.opener;
        div()
            .id(("test-opener-choice", index))
            .debug_selector(move || format!("test-opener-choice-{index}"))
            .px(px(8.0))
            .py(px(6.0))
            .rounded(px(5.0))
            .cursor_pointer()
            .when(selected, |row| row.bg(theme::bg_row_active()))
            .hover(|row| row.bg(theme::bg_hover()))
            .child(label(choice))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.test.opener = index;
                this.test.picking = false;
                this.test.run();
                cx.notify();
            }))
    };
    let heading = |title: &'static str| {
        div().px(px(8.0)).pt(px(8.0)).pb(px(3.0)).child(text(
            title,
            10.0,
            600.0,
            theme::text_subtle(),
        ))
    };
    let (named, examples): (Vec<_>, Vec<_>) = choices
        .iter()
        .enumerate()
        .filter_map(|(index, choice)| choice.as_ref().map(|choice| (index, choice)))
        .partition(|(_, choice)| app.test.named_by_rules(choice));
    let menu = div()
        .id("test-opener-menu")
        .occlude()
        .absolute()
        .top(px(34.0))
        .left_0()
        .w(px(170.0))
        .flex()
        .flex_col()
        .p(px(4.0))
        .bg(theme::bg_elevated())
        .border_1()
        .border_color(theme::border_default())
        .rounded(px(7.0))
        .shadow_lg()
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.test.picking = false;
            cx.notify();
        }))
        .child(row(0, None))
        .when(!named.is_empty(), |menu| {
            menu.child(heading("In your rules")).children(
                named
                    .iter()
                    .map(|(index, choice)| row(*index, Some(choice))),
            )
        })
        .when(!examples.is_empty(), |menu| {
            menu.child(heading("Examples")).children(
                examples
                    .iter()
                    .map(|(index, choice)| row(*index, Some(choice))),
            )
        });
    div()
        .relative()
        .w(px(170.0))
        .flex_none()
        .child(
            input_box()
                .id("test-opener")
                .debug_selector(|| "test-opener".into())
                .cursor_pointer()
                .child(text("from", 10.5, 400.0, theme::text_subtle()))
                .child(label(app.test.opener().as_ref()))
                .child(caret())
                // Capture runs in paint order: before the deferred menu's
                // press-outside handler closes it.
                .capture_any_mouse_down(cx.listener(|this, _, _, _| {
                    this.test.was_picking = this.test.picking;
                }))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.test.picking = !this.test.was_picking;
                    cx.notify();
                })),
        )
        .when(app.test.picking, |picker| picker.child(deferred(menu)))
}

/// `.saved`: what Save as test wrote, and where.
fn saved_banner(
    test: &RouteTest,
    index: usize,
    cx: &mut Context<'_, PwrRoute>,
) -> impl IntoElement + use<> {
    let from = match &test.opener {
        Some(opener) => format!(
            " from {}",
            opener
                .split_once(':')
                .map_or(opener.as_str(), |(name, _)| name)
        ),
        None => String::new(),
    };
    let mut expects = test.expect.browser.clone();
    if let Some(profile) = &test.expect.profile {
        expects.push_str(&format!(" \u{00b7} {profile}"));
    }
    if let Some(url) = &test.expect.url {
        expects.push_str(&format!(" and {url}"));
    }
    div()
        .id("test-saved")
        .debug_selector(|| "test-saved".into())
        .flex()
        .items_center()
        .gap(px(10.0))
        .py(px(10.0))
        .px(px(16.0))
        .border_t_1()
        .border_color(theme::border_faint())
        .bg(theme::bg_row_active())
        .child(
            paragraph(
                format!(
                    "Saved to rules.json \u{203a} tests as test {}. This link{from} expects \
                     {expects}. Run tests checks it on every save.",
                    index + 1
                ),
                11.5,
                17.25,
                theme::text_secondary(),
            )
            .flex_1()
            .min_w_0(),
        )
        .child(
            button("Show in Rules", ButtonKind::Quiet, true)
                .id("test-show-rules")
                .debug_selector(|| "test-show-rules".into())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.navigate(crate::app::Screen::Rules, window, cx);
                })),
        )
}

/// `.res`: where the link opens, and the actions: Copy trace, Save as test
/// and Open now.
fn result_card(app: &PwrRoute, view: &TraceView, cx: &mut Context<'_, PwrRoute>) -> Div {
    let copied = app.test.copied;
    let proposed = app.test.proposed_test();
    let saved = app.test.saved_as.and_then(|index| {
        app.store
            .tests()
            .get(index)
            .map(|test| (test.clone(), index))
    });
    let save: Option<gpui_kit::AnyElement> = match (&saved, proposed.is_some()) {
        (Some((_, index)), _) => Some(
            button(
                format!("Saved as test {}", index + 1),
                ButtonKind::Default,
                false,
            )
            .into_any_element(),
        ),
        (None, true) => Some(
            button("Save as test", ButtonKind::Default, false)
                .id("test-save")
                .debug_selector(|| "test-save".into())
                .on_click(cx.listener(|this, _, _, cx| this.save_as_test(cx)))
                .into_any_element(),
        ),
        (None, false) => None,
    };
    card()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(14.0))
                .py(px(14.0))
                .px(px(16.0))
                .child(
                    div()
                        .size(px(44.0))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(10.0))
                        .border_1()
                        .border_color(theme::border_strong())
                        .bg(theme::bg_elevated())
                        .child(mono(
                            view.result.badge.clone(),
                            13.0,
                            700.0,
                            theme::text_secondary(),
                        )),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .child(
                            text(
                                view.result.title.clone(),
                                15.0,
                                600.0,
                                theme::text_primary(),
                            )
                            .line_height(px(18.0)),
                        )
                        .child(url(
                            &[UrlPart {
                                text: view.result.url.clone(),
                                mark: UrlMark::Same,
                            }],
                            theme::text_secondary(),
                        ))
                        .when_some(view.result.error.clone(), |column, error| {
                            column.child(paragraph(error, 11.0, 16.5, theme::danger_text()))
                        }),
                )
                .child(
                    div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        button(
                            if copied { "Copied" } else { "Copy trace" },
                            ButtonKind::Default,
                            false,
                        )
                        .id("test-copy")
                        .debug_selector(|| "test-copy".into())
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(Ok(view)) = &this.test.outcome {
                                cx.write_to_clipboard(ClipboardItem::new_string(view.to_text()));
                                this.test.copied = true;
                                cx.notify();
                            }
                        })),
                    )
                    .children(save)
                    // Dispatching from here waits for the browser launcher.
                    .child(button("Open now", ButtonKind::Primary, false)),
                ),
        )
        .children(saved.map(|(test, index)| saved_banner(&test, index, cx)))
}

impl PwrRoute {
    /// Save as test: the last run becomes a top-level test in `rules.json`,
    /// written on the next save.
    pub fn save_as_test(&mut self, cx: &mut Context<'_, Self>) {
        let Some(test) = self.test.proposed_test() else {
            return;
        };
        let index = self.store.add_test(test);
        self.test.saved_as = Some(index);
        self.refresh(cx);
    }
}

pub fn pane(
    app: &PwrRoute,
    window: &mut Window,
    cx: &mut Context<'_, PwrRoute>,
) -> impl IntoElement + use<> {
    let field = text_field::field(
        "test-url",
        &app.test.url,
        Look {
            mono: true,
            placeholder: "Paste a link",
        },
        None,
        window,
    )
    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
        match this.test.url.key(&event.keystroke, cx) {
            Edit::Submit => this.test.run(),
            Edit::Changed | Edit::Handled => {}
            Edit::Ignored => return,
        }
        cx.stop_propagation();
        cx.notify();
    }))
    .on_click(cx.listener(|this, _, window, cx| {
        window.focus(&this.test.url.focus, cx);
        cx.notify();
    }));

    let test_row = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .child(field)
        .child(opener_picker(app, cx))
        .child(
            button("Test", ButtonKind::Primary, false)
                .id("test-run")
                .debug_selector(|| "test-run".into())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.sync_with_disk(cx);
                    this.test.run();
                    cx.notify();
                })),
        );

    let outcome = match &app.test.outcome {
        None => None,
        Some(Err(error)) => Some(
            card()
                .py(px(14.0))
                .px(px(16.0))
                .child(paragraph(error.clone(), 11.5, 17.25, theme::danger_text()))
                .into_any_element(),
        ),
        Some(Ok(view)) => {
            let count = view.steps.len();
            let trace = view.steps.iter().enumerate().fold(
                card()
                    .id("test-trace")
                    .debug_selector(|| "test-trace".into()),
                |trace, (index, step)| trace.child(step_row(index, count, step, cx)),
            );
            Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(18.0))
                    .child(trace)
                    .child(result_card(app, view, cx))
                    .into_any_element(),
            )
        }
    };

    pane_frame("test-pane")
        .child(div().flex().items_start().child(pane_heading(
            "Dry run",
            "Test a link",
            Some(
                "Resolves against the active rules in the real router. Nothing opens until \
                 you say so.",
            ),
        )))
        .child(test_row)
        .children(outcome)
}
