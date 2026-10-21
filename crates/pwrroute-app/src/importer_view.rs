//! The Finicky importer's window (design `PwrRoute Finicky Importer.dc.html`
//! 2a–2c, 3a, 3c) and the Rules screen's banners around it (1a, 3b).
//!
//! [`open`] reads the config into an [`ImportModel`] and shows it, one
//! importer at a time. Import closes the window and [`finish`] writes
//! `rules.json` through the main window's store, so the Rules screen reloads
//! at once and keeps the previous file for Undo import, then switches the
//! router's source when Choose source asked for it (`settings.json`'s
//! `source`, `router::choose_source`). A write the store refuses opens the
//! importer again with the reason. The config file is never written, and
//! a file that is not the config in charge never takes its place.
//! Recent links reach the engine's probes from the Activity feed, in
//! memory only.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Bounds, ClickEvent, Context, Div, ElementId, FocusHandle, Global, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, TitlebarOptions, Window, WindowBounds,
    WindowHandle, WindowOptions, div, point, px, size,
};
use pwrroute_engine::Privacy;
use pwrroute_engine::{ConfigChoice, RulesFile, SourceLines, Verdict, Visual, discovery};

use crate::app::{PwrRoute, Screen};
use crate::config_view::display_path;
use crate::importer::{
    Charge, ImportModel, Item, Kind, Label, Placement, SourceChoice, Step, same_file,
};
use crate::rules_store::{RulesStore, clock};
use crate::rules_view::{RowState, card, rule_row};
use crate::settings_store::{self, SourceSettings};
use crate::theme::{self, mono, text, tracked};
use crate::widgets::{action_tag, badge, fn_tag, one_line, paragraph, regex_tag, tag};

/// The element id of the window's root.
pub const ROOT_ID: &str = "importer-root";

/// `.dv-card.win { width: 1180px; height: 640px }`.
const WINDOW_SIZE: (f32, f32) = (1180.0, 640.0);

/// The most recent links the suggestions are probed on.
const RECENT_LINKS: usize = 100;

/// Ids the tests click.
pub const NEXT_ID: &str = "importer-next";
pub const BACK_ID: &str = "importer-back";
pub const CANCEL_ID: &str = "importer-cancel";
pub const ALL_ID: &str = "importer-all";
pub const SUGGESTION_ID: &str = "importer-suggestion";
pub const UNDO_ID: &str = "importer-undo";
pub const NOTICE_REVIEW_ID: &str = "importer-notice-review";
pub const NOTICE_DISMISS_ID: &str = "importer-notice-dismiss";

/// What the app does around the window: show the main window on Rules
/// when an import lands with none open. `main` installs it; the harness
/// and the `screens` example run without.
#[derive(Clone)]
pub struct ImporterHost {
    pub show_rules: Rc<dyn Fn(&mut App)>,
}

impl Global for ImporterHost {}

/// What the Rules screen remembers of the last import, for its banner and
/// Undo import.
#[derive(Clone, Debug)]
pub struct ImportDone {
    pub shown: String,
    pub file: PathBuf,
    pub count: usize,
    pub suggestions: usize,
    /// Rules that stayed in the config.
    pub stays: usize,
    /// What the router reads now: `ImportModel::outcome`.
    pub outcome: String,
    pub at: SystemTime,
    /// `rules.json` before the import, and the source before it.
    pub previous: RulesFile,
    pub previous_source: Option<SourceSettings>,
    /// Undo is good until the next edit to `rules.json`.
    pub undoable: bool,
}

/// A config discovery found while visual rules are in charge (1a).
#[derive(Clone, Debug)]
pub struct FoundConfig {
    pub file: PathBuf,
    pub shown: String,
    /// "2 rewrites, 6 handlers and a default browser"
    pub holds: String,
}

/// The main window, when one is open.
fn main_window(cx: &App) -> Option<WindowHandle<PwrRoute>> {
    cx.windows()
        .into_iter()
        .find_map(|window| window.downcast::<PwrRoute>())
}

fn importer_window(cx: &App) -> Option<WindowHandle<Importer>> {
    cx.windows()
        .into_iter()
        .find_map(|window| window.downcast::<Importer>())
}

/// The router's recent links, newest last, as the rules saw them: only
/// records kept in full, from memory.
fn recent_links(cx: &App) -> Vec<String> {
    let Some(feed) = crate::activity::feed(cx) else {
        return Vec::new();
    };
    let records = feed.read(cx).records();
    let mut links: Vec<String> = records
        .iter()
        .rev()
        .filter(|record| record.privacy == Privacy::Full)
        .map(|record| crate::activity::matched_url(record).to_owned())
        .take(RECENT_LINKS)
        .collect();
    links.dedup();
    links
}

/// Opens the importer on the config at `path`: on Review, or for first
/// run's "Import N rules" (`first_run`) on Choose source with every proven
/// rule checked. A second call replaces what the open importer shows.
pub fn open(path: PathBuf, first_run: bool, cx: &mut App) {
    open_with(path, first_run, crate::router::config_in_charge(), cx);
}

/// [`open`], with the config the router reads now given: tests name it,
/// since the router's source is one for the whole process.
pub(crate) fn open_with(path: PathBuf, first_run: bool, in_charge: Option<PathBuf>, cx: &mut App) {
    let (existing, home) = main_window(cx)
        .and_then(|main| {
            main.read(cx)
                .ok()
                .map(|app| (app.store.written(), app.home.clone()))
        })
        .unwrap_or_else(|| {
            let environment = discovery::Environment::current();
            (
                RulesStore::load(discovery::rules_path(&environment)).written(),
                std::env::home_dir(),
            )
        });
    let recent = recent_links(cx);
    let refs: Vec<&str> = recent.iter().map(String::as_str).collect();
    let mut model = ImportModel::read(
        &path,
        home.as_deref(),
        in_charge.as_deref(),
        &refs,
        existing,
    );
    if first_run {
        model.start_at_source();
    }
    show(model, cx);
}

/// Shows `model` in the importer, opening it when none is open.
fn show(model: ImportModel, cx: &mut App) {
    if let Some(window) = importer_window(cx) {
        let _ = window.update(cx, |this, window, cx| {
            this.model = model;
            window.activate_window();
            cx.notify();
        });
        cx.activate(true);
        return;
    }
    let bounds = Bounds::centered(None, size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)), cx);
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("PwrRoute".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(16.0), px(13.0))),
            }),
            app_id: Some("pwrroute".to_owned()),
            window_min_size: Some(size(px(980.0), px(560.0))),
            ..Default::default()
        },
        |_, cx| cx.new(|cx| Importer::new(model, cx)),
    );
    match opened {
        Ok(window) => {
            let _ = window.update(cx, |this, window, cx| window.focus(&this.focus, cx));
            cx.activate(true);
        }
        Err(error) => eprintln!("pwrroute: could not open the importer: {error}"),
    }
}

/// The importer's window.
pub struct Importer {
    pub model: ImportModel,
    pub focus: FocusHandle,
}

impl Importer {
    pub fn new(model: ImportModel, cx: &mut Context<'_, Self>) -> Self {
        Self {
            model,
            focus: cx.focus_handle(),
        }
    }

    fn select(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        self.model.selected = index;
        cx.notify();
    }

    fn toggle(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        self.model.toggle(index);
        cx.notify();
    }

    fn toggle_all(&mut self, cx: &mut Context<'_, Self>) {
        let all = self.model.all_checked();
        self.model.set_all(!all);
        cx.notify();
    }

    fn next(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        match self.model.next_step() {
            Some(step) => {
                self.model.step = step;
                cx.notify();
            }
            None => self.import(window, cx),
        }
    }

    fn back(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(step) = self.model.previous_step() {
            self.model.step = step;
            cx.notify();
        }
    }

    /// Import: the window closes and the main window writes the file. A
    /// write the store refuses opens the importer again, as it was, with
    /// the reason in the footer.
    fn import(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let mut model = self.model.clone();
        model.refused = None;
        window.remove_window();
        cx.defer(move |cx| {
            if let Err(reason) = finish(&model, cx) {
                model.refused = Some(reason);
                show(model, cx);
            }
        });
    }

    fn choose_another(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        window.remove_window();
        cx.defer(prompt_for_config);
    }
}

/// File › Import Finicky config…: any `.js` or `.ts`, not only the one
/// discovery found.
pub fn prompt_for_config(cx: &mut App) {
    let paths = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some("Import".into()),
    });
    cx.spawn(async move |cx| {
        if let Ok(Ok(Some(paths))) = paths.await
            && let Some(path) = paths.into_iter().next()
        {
            cx.update(|cx| open(path, false, cx));
        }
    })
    .detach();
}

/// Writes the import into `rules.json` through the main window (opening it
/// on Rules when none is), then switches the router's source as Choose
/// source said. The source changes only when the write went through; a
/// refused write comes back as the reason.
pub fn finish(model: &ImportModel, cx: &mut App) -> Result<(), String> {
    if main_window(cx).is_none()
        && let Some(host) = cx.try_global::<ImporterHost>().cloned()
    {
        (host.show_rules)(cx);
    }
    let main = main_window(cx).ok_or("No PwrRoute window to import into.")?;
    let previous_source =
        settings_store::store(cx).map(|store| store.read(cx).settings().source.clone());
    main.update(cx, |app, window, cx| {
        app.navigate(Screen::Rules, window, cx);
        app.apply_import(model, previous_source.clone(), cx)
    })
    .map_err(|error| format!("{error:#}"))??;
    if let Some(config) = switch_source(model, cx) {
        let _ = main.update(cx, |app, _, cx| app.set_config(config, cx));
    }
    Ok(())
}

/// Saves the source Choose source picked (`ImportModel::source_after`) and
/// hands it to the router. Keep never touches it, so a file that is not
/// the config in charge never takes its place. Returns the config now in
/// charge when that changed; `None` when nothing did, or there is no
/// settings store (the window harness).
fn switch_source(model: &ImportModel, cx: &mut App) -> Option<Option<PathBuf>> {
    let store = settings_store::store(cx)?;
    let current = store.read(cx).settings().source.clone();
    let wanted = model.source_after(&current);
    if wanted == current && !model.stops_config() {
        return None;
    }
    set_source(wanted, cx)
}

/// Saves `source` to `settings.json` and switches the router to it.
/// Returns the config it puts in charge.
fn set_source(source: SourceSettings, cx: &mut App) -> Option<Option<PathBuf>> {
    let store = settings_store::store(cx)?;
    let choice = source.choice(discovery::config_candidates(
        &discovery::Environment::current(),
    ));
    store.update(cx, |store, _| {
        if let Err(error) = store.update(|settings| settings.source = source) {
            eprintln!("pwrroute: cannot save the rules source: {error:#}");
        }
    });
    let config = match &choice {
        ConfigChoice::None => None,
        ConfigChoice::Path(path) => Some(path.canonicalize().unwrap_or_else(|_| path.clone())),
        ConfigChoice::Discover(candidates) => discovery::find_config(candidates),
    };
    crate::router::choose_source(choice, settings_store::router(cx).as_ref());
    Some(config)
}

/// Undo import's source half: the source before the import, when it
/// changed.
pub fn restore_source(source: SourceSettings, cx: &mut App) -> Option<Option<PathBuf>> {
    let store = settings_store::store(cx)?;
    if store.read(cx).settings().source == source {
        return None;
    }
    set_source(source, cx)
}

/// 1a's "Not now": the notice for this file does not come back.
fn dismiss(file: &Path, cx: &mut App) {
    if let Some(store) = settings_store::store(cx) {
        let file = file.to_path_buf();
        store.update(cx, |store, _| {
            let saved = store.update(|settings| {
                if !settings.source.dismissed.contains(&file) {
                    settings.source.dismissed.push(file);
                }
            });
            if let Err(error) = saved {
                eprintln!("pwrroute: cannot save the dismissal: {error:#}");
            }
        });
    }
}

/// 1a: the config discovery finds while visual rules are in charge, when
/// its notice was not dismissed.
pub fn found_config(home: Option<&Path>, cx: &App) -> Option<FoundConfig> {
    let source = settings_store::store(cx)?
        .read(cx)
        .settings()
        .source
        .clone();
    if !source.visual_only {
        return None;
    }
    let file = discovery::find_config(&discovery::config_candidates(
        &discovery::Environment::current(),
    ))?;
    if source
        .dismissed
        .iter()
        .any(|dismissed| same_file(dismissed, &file))
    {
        return None;
    }
    let holds = pwrroute_engine::Engine::load(
        &file,
        Rc::new(pwrroute_engine::DefaultHost),
        pwrroute_engine::Limits::default(),
    )
    .ok()
    .and_then(|engine| engine.describe().ok().flatten())
    .map_or_else(
        || "It did not load".to_owned(),
        |config| {
            format!(
                "{}, {} and a default browser",
                plural(config.rewrites.len(), "rewrite"),
                plural(config.handlers.len(), "handler")
            )
        },
    );
    Some(FoundConfig {
        shown: display_path(&file, home),
        file,
        holds,
    })
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

// --- the Rules screen's banners ---------------------------------------------

/// `.banner` in a tone: the accent (3b) or info (1a).
fn banner_frame(border: Hsla, fill: Hsla) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(14.0))
        .py(px(12.0))
        .px(px(14.0))
        .border_1()
        .border_color(border)
        .rounded(px(10.0))
        .bg(fill)
}

fn banner_text(title: String, body: String) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(text(title, 12.5, 600.0, theme::text_primary()).line_height(px(16.25)))
        .child(paragraph(body, 11.0, 16.5, theme::text_muted()))
}

/// The banners above the Rules screen's lists: the last import (3b), and
/// a config found while visual rules are in charge (1a).
pub fn rules_banners(app: &PwrRoute, cx: &mut Context<'_, PwrRoute>) -> Vec<Div> {
    let mut banners = Vec::new();
    if let Some(done) = &app.last_import {
        let saved = clock(done.at);
        let mut body = format!("{} Saved {saved}; reloaded.", done.outcome);
        if done.suggestions > 0 {
            body.push_str(&format!(" {} from a suggestion.", done.suggestions));
        }
        let file = done.file.clone();
        let actions = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .when(done.undoable, |row| {
                row.child(
                    small_button(UNDO_ID, "Undo import", false).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            if let Some(source) = this.undo_import(cx)
                                && let Some(config) = restore_source(source, cx)
                            {
                                this.set_config(config, cx);
                            }
                        },
                    )),
                )
            })
            .when(done.stays > 0, |row| {
                row.child(
                    small_button("importer-show-js", "Show what stayed JS", true).on_click(
                        move |_: &ClickEvent, _, cx| {
                            let file = file.clone();
                            cx.defer(move |cx| open(file, false, cx));
                        },
                    ),
                )
            });
        banners.push(
            banner_frame(theme::accent_border(), theme::bg_row_active())
                .debug_selector(|| "import-done".into())
                .child(text("\u{2713}", 16.0, 700.0, theme::accent()))
                .child(banner_text(
                    format!(
                        "Imported {} from {}",
                        plural(done.count, "rule"),
                        done.shown
                    ),
                    body,
                ))
                .child(actions),
        );
    }
    if app.last_import.is_none()
        && let Some(found) = &app.found_config
    {
        let review = found.file.clone();
        let dismissed = found.file.clone();
        banners.push(
            banner_frame(theme::info_soft(), theme::info_soft())
                .debug_selector(|| "import-found".into())
                .child(theme::icon("pwrroute/code.svg", 18.0, theme::info_text()))
                .child(banner_text(
                    format!("Found {}", found.shown),
                    format!(
                        "{}. Review what the visual rules can hold; nothing changes until you \
                         import.",
                        found.holds
                    ),
                ))
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            small_button(NOTICE_REVIEW_ID, "Review import\u{2026}", false)
                                .on_click(move |_: &ClickEvent, _, cx| {
                                    let file = review.clone();
                                    cx.defer(move |cx| open(file, false, cx));
                                }),
                        )
                        .child(small_button(NOTICE_DISMISS_ID, "Not now", true).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                dismiss(&dismissed, cx);
                                this.found_config = None;
                                cx.notify();
                            }),
                        )),
                ),
        );
    }
    banners
}

// --- the window -------------------------------------------------------------

/// `.btn.sm`, quiet or default, clickable.
fn small_button(id: &'static str, label: &'static str, quiet: bool) -> Stateful<Div> {
    action_button(
        id,
        label.into(),
        if quiet { Look::Quiet } else { Look::Default },
        true,
    )
}

/// Which `.btn` a clickable button is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Look {
    Primary,
    Default,
    Quiet,
}

fn action_button(id: &'static str, label: SharedString, kind: Look, small: bool) -> Stateful<Div> {
    let (fill, border, color) = match kind {
        Look::Primary => (theme::accent(), theme::transparent(), theme::accent_on()),
        Look::Default => (
            theme::bg_elevated(),
            theme::border_default(),
            theme::text_primary(),
        ),
        Look::Quiet => (
            theme::transparent(),
            theme::transparent(),
            theme::text_secondary(),
        ),
    };
    let (py, px_, size) = if small {
        (5.0, 9.0, 11.0)
    } else {
        (7.0, 12.0, 11.5)
    };
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex()
        .flex_none()
        .items_center()
        .py(px(py))
        .px(px(px_))
        .border_1()
        .border_color(border)
        .rounded(px(7.0))
        .bg(fill)
        .cursor_pointer()
        .hover(|button| button.opacity(0.9))
        .child(text(label, size, 550.0, color))
}

/// `.ck2`: a 14px box, `.on` ticked, `.na` not available.
fn checkbox(checked: bool, available: bool) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(14.0))
        .rounded(px(4.0))
        .border_1()
        .map(|box_| {
            if checked {
                box_.bg(theme::accent())
                    .border_color(theme::transparent())
                    .child(text("\u{2713}", 9.0, 700.0, theme::accent_on()))
            } else {
                box_.border_color(theme::border_strong())
            }
        })
        .when(!available, |box_| box_.opacity(0.35))
}

/// `.chip` in the verdict's tone: `.ok`, `.warn` or `.acc`.
fn verdict_chip(label: Label) -> Div {
    let (word, fill, color) = match label {
        Label::Imports => ("Imports", theme::success_soft(), theme::success_text()),
        Label::StaysJs => ("Stays JS", theme::warn_soft(), theme::warn_text()),
        Label::Suggestion => ("Suggestion", theme::accent_soft(), theme::accent_bright()),
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .px(px(9.0))
        .py(px(4.0))
        .rounded_full()
        .bg(fill)
        .child(text(word, 10.0, 600.0, color))
}

/// `.stepper`: the three steps, done, current or to come.
fn stepper(model: &ImportModel) -> Div {
    let order = [Step::Review, Step::Place, Step::Source];
    let at = order.iter().position(|s| *s == model.step).unwrap_or(0);
    let names = ["Review", "Place", "Choose source"];
    let mut row = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .ml(px(4.0));
    for (i, name) in names.iter().enumerate() {
        let (mark, border, fill, color, mark_color) = if i < at {
            (
                "\u{2713}".to_owned(),
                theme::border_subtle(),
                theme::transparent(),
                theme::success_text(),
                theme::success_text(),
            )
        } else if i == at {
            (
                (i + 1).to_string(),
                theme::accent_border(),
                theme::bg_row_active(),
                theme::accent_bright(),
                theme::accent(),
            )
        } else {
            (
                (i + 1).to_string(),
                theme::border_subtle(),
                theme::transparent(),
                theme::text_muted(),
                theme::text_subtle(),
            )
        };
        row = row.child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.0))
                .h(px(24.0))
                .px(px(9.0))
                .rounded_full()
                .border_1()
                .border_color(border)
                .bg(fill)
                .child(mono(mark, 10.0, 600.0, mark_color))
                .child(text(*name, 11.0, 550.0, color)),
        );
    }
    row
}

/// The strip: brand, "Import ~/.finicky.ts" and the stepper.
fn title_strip(model: &ImportModel) -> Div {
    crate::chrome::strip()
        .child(text("\u{203a}", 14.0, 500.0, theme::text_subtle()))
        .child(text(
            format!("Import {}", model.shown),
            13.0,
            500.0,
            theme::text_secondary(),
        ))
        .child(stepper(model))
}

/// What a row of Review's list says between its line number and its chip.
fn row_text(item: &Item) -> Div {
    let row = div()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(5.0))
        .overflow_hidden();
    if item.kind == Kind::Default {
        return row
            .child(text("defaultBrowser", 11.5, 400.0, theme::text_secondary()))
            .child(arrow())
            .child(destination_text(&item.row.destination));
    }
    let mut row = row;
    for matcher in &item.row.matchers {
        row = match matcher {
            crate::rules_model::Matcher::Pattern(p) => row.child(tag(p.clone())),
            crate::rules_model::Matcher::Regex(p) => row.child(regex_tag(p.clone())),
            crate::rules_model::Matcher::Function(source) => {
                // A function's row is all source: no arrow, as the design.
                return row.child(fn_tag()).child(
                    one_line(
                        source.clone(),
                        theme::mono_family(400.0),
                        400.0,
                        10.5,
                        13.65,
                        theme::text_muted(),
                    )
                    .flex_1(),
                );
            }
            _ => row,
        };
    }
    row.child(arrow())
        .child(destination_text(&item.row.destination))
}

fn arrow() -> Div {
    mono("\u{2192}", 12.0, 500.0, theme::text_subtle())
}

fn destination_text(destination: &crate::rules_model::Destination) -> Div {
    use crate::rules_model::Destination;
    match destination {
        Destination::Browser { name, profile, .. } => text(
            match profile {
                Some(profile) => format!("{name} \u{00b7} {profile}"),
                None => name.to_string(),
            },
            11.5,
            400.0,
            theme::text_secondary(),
        ),
        Destination::Rewrite(changes) => changes
            .iter()
            .fold(div().flex().gap(px(5.0)), |row, change| {
                row.child(action_tag(change.kind, change.value.clone()))
            }),
        Destination::Function(_) => div().flex().gap(px(5.0)).child(fn_tag()),
        Destination::Unset(label) => text(*label, 11.5, 400.0, theme::text_subtle()),
    }
}

/// `.ilist`: the found rules by group, each with its box, line, text and
/// verdict.
fn list(model: &ImportModel, cx: &mut Context<'_, Importer>) -> Stateful<Div> {
    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .py(px(12.0))
        .px(px(14.0))
        .border_b_1()
        .border_color(theme::border_faint())
        .child(
            div()
                .id(ALL_ID)
                .debug_selector(|| ALL_ID.into())
                .cursor_pointer()
                .child(checkbox(model.all_checked(), true))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_all(cx))),
        )
        .child(text(
            format!("{} found", model.found()),
            11.5,
            600.0,
            theme::text_secondary(),
        ))
        .child(div().ml_auto().child(text(
            model.counts_label(),
            10.5,
            400.0,
            theme::text_subtle(),
        )));
    let mut column = div()
        .id("importer-list")
        .flex_none()
        .w(px(440.0))
        .h_full()
        .flex()
        .flex_col()
        .border_r_1()
        .border_color(theme::border_subtle())
        .bg(theme::bg_sidebar())
        .overflow_y_scroll()
        .child(header);
    let mut group = None;
    for (index, item) in model.items.iter().enumerate() {
        if group != Some(item.kind) {
            group = Some(item.kind);
            let name = match item.kind {
                Kind::Rewrite => "Rewrites",
                Kind::Handler => "Handlers",
                Kind::Default => "Default",
            };
            column = column.child(div().pt(px(10.0)).pb(px(4.0)).px(px(14.0)).child(tracked(
                name,
                9.5,
                600.0,
                0.14,
                theme::text_subtle(),
            )));
        }
        let selected = index == model.selected;
        let line = item
            .line()
            .map_or_else(String::new, |line| format!(":{line}"));
        let check = div()
            .id(ElementId::named_usize("importer-check", index))
            .debug_selector(move || format!("importer-check-{index}"))
            .flex_none()
            .w(px(14.0))
            .when(item.checkable(), |cell| cell.cursor_pointer())
            .child(checkbox(item.checked, item.checkable()))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.toggle(index, cx);
            }));
        column = column.child(
            div()
                .id(ElementId::named_usize("importer-item", index))
                .debug_selector(move || format!("importer-item-{index}"))
                .relative()
                .flex()
                .items_center()
                .gap(px(10.0))
                .py(px(8.0))
                .px(px(14.0))
                .cursor_pointer()
                .when(selected, |row| {
                    row.bg(theme::bg_row_active()).child(
                        div()
                            .absolute()
                            .left_0()
                            .top(px(5.0))
                            .bottom(px(5.0))
                            .w(px(3.0))
                            .rounded_r(px(2.0))
                            .bg(theme::accent()),
                    )
                })
                .when(!selected, |row| row.hover(|s| s.bg(theme::bg_hover())))
                .child(check)
                .child(div().w(px(20.0)).flex_none().child(mono(
                    line,
                    10.0,
                    500.0,
                    theme::text_subtle(),
                )))
                .child(row_text(item))
                .child(verdict_chip(item.label()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.select(index, cx))),
        );
    }
    column
}

/// `.frame`: the rule's lines from the file, the rule's own lit.
fn source_frame(shown: &str, lines: &SourceLines) -> Div {
    let last = lines.first + lines.lines.len().saturating_sub(1);
    let header = div()
        .flex()
        .gap(px(10.0))
        .py(px(6.0))
        .px(px(10.0))
        .border_b_1()
        .border_color(theme::border_faint())
        .child(mono(shown.to_owned(), 11.0, 500.0, theme::text_muted()))
        .child(div().ml_auto().child(mono(
            format!("lines {}\u{2013}{}", lines.first, last),
            11.0,
            500.0,
            theme::text_subtle(),
        )));
    lines.lines.iter().enumerate().fold(
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .border_1()
            .border_color(theme::border_subtle())
            .rounded(px(8.0))
            .bg(theme::bg_app())
            .overflow_hidden()
            .child(header),
        |frame, (i, line)| {
            let number = lines.first + i;
            let hit = lines.hit.contains(&number);
            frame.child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .flex()
                    .px(px(10.0))
                    .h(px(18.7))
                    .items_center()
                    .when(hit, |row| row.bg(theme::accent_tint()))
                    .child(
                        div()
                            .w(px(34.0))
                            .flex_none()
                            .pr(px(12.0))
                            .flex()
                            .justify_end()
                            .child(mono(number.to_string(), 11.0, 500.0, theme::text_subtle())),
                    )
                    .child(mono(
                        line.replace('\t', "    "),
                        11.0,
                        500.0,
                        if hit {
                            theme::text_primary()
                        } else {
                            theme::text_secondary()
                        },
                    )),
            )
        },
    )
}

/// `.verdict`: the pin and what the verdict means.
fn verdict_box(item: &Item) -> Div {
    let (ok, title, body) = match &item.verdict {
        Verdict::Imports(visual) => (
            true,
            "Imports exactly.",
            match visual {
                Visual::Handler(_) => {
                    "A wildcard matcher and a browser name are what a visual handler stores; the \
                     router passes them to the same Finicky API, so the result cannot differ."
                }
                Visual::Rewrite(_) => {
                    "A wildcard matcher and a fixed URL are what a visual rewrite stores; the \
                     router passes them to the same Finicky API, so the result cannot differ."
                }
                Visual::Default { .. } => {
                    "A browser name is what the visual default stores; the router passes it to \
                     the same Finicky API, so the result cannot differ."
                }
            }
            .to_owned(),
        ),
        Verdict::StaysJs { why, .. } => (
            false,
            "Stays JS.",
            format!("{why} Keep the config in charge (step 3) and this keeps running as written."),
        ),
    };
    let (fill, pin_fill, pin_color, pin) = if ok {
        (
            theme::success_soft(),
            theme::success_soft(),
            theme::success_text(),
            "\u{2713}",
        )
    } else {
        (
            theme::warn_soft(),
            theme::warn_soft(),
            theme::warn_text(),
            "fn",
        )
    };
    div()
        .min_w_0()
        .flex()
        .items_start()
        .gap(px(10.0))
        .py(px(11.0))
        .px(px(13.0))
        .rounded(px(8.0))
        .bg(fill)
        .child(
            div()
                .mt(px(1.0))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(20.0))
                .rounded(px(6.0))
                .bg(pin_fill)
                .child(mono(pin, 10.0, 700.0, pin_color)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_family(theme::SANS)
                .text_size(px(11.5))
                .line_height(px(17.8))
                .text_color(theme::text_secondary())
                .child(
                    gpui_kit::StyledText::new(format!("{title} {body}")).with_highlights([(
                        0..title.len(),
                        gpui_kit::HighlightStyle {
                            color: Some(theme::text_primary()),
                            font_weight: Some(theme::weight(600.0)),
                            ..Default::default()
                        },
                    )]),
                ),
        )
}

fn heading(label: impl Into<SharedString>) -> Div {
    let label: SharedString = label.into();
    tracked(&label, 9.5, 600.0, 0.14, theme::text_muted())
}

/// `.becomes`: the visual row a rule turns into.
fn becomes(item: &Item) -> Option<Div> {
    let mut row = item.becomes()?;
    row.hits = "new".into();
    let default = item.kind == Kind::Default;
    let matchers = default.then(|| {
        div()
            .flex_1()
            .min_w_0()
            .child(text("Everything else", 11.0, 600.0, theme::text_primary()))
    });
    Some(
        card().child(
            rule_row(
                "importer-becomes",
                if default { "" } else { "+" },
                &row,
                RowState {
                    last: true,
                    ..RowState::default()
                },
                matchers,
            )
            .bg(theme::accent_tint()),
        ),
    )
}

/// 2b's `.sugg`: the suggested rule, its box, and its evidence.
fn suggestion(model: &ImportModel, index: usize, cx: &mut Context<'_, Importer>) -> Option<Div> {
    let item = model.items.get(index)?;
    let Verdict::StaysJs {
        suggestion: Some(suggested),
        ..
    } = &item.verdict
    else {
        return None;
    };
    let rule = &suggested.rule;
    let to = if rule.profile.is_empty() {
        rule.browser.clone()
    } else {
        format!("{} \u{00b7} {}", rule.browser, rule.profile)
    };
    let patterns = rule
        .patterns
        .iter()
        .fold(div().flex().items_center().gap(px(5.0)), |row, p| {
            row.child(tag(p.clone()))
        });
    let built = model.probes - model.recent;
    Some(
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .py(px(12.0))
            .px(px(13.0))
            .border_1()
            .border_dashed()
            .border_color(theme::border_strong())
            .rounded(px(8.0))
            .child(
                div()
                    .id(SUGGESTION_ID)
                    .debug_selector(|| SUGGESTION_ID.into())
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.0))
                    .cursor_pointer()
                    .child(checkbox(item.checked, true))
                    .child(text(
                        "Use a visual rule instead:",
                        12.0,
                        600.0,
                        theme::text_primary(),
                    ))
                    .child(patterns)
                    .child(arrow())
                    .child(badge(crate::widgets::badge_for(&rule.browser)))
                    .child(text(to, 12.0, 600.0, theme::text_primary()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.toggle(index, cx);
                    })),
            )
            .child(paragraph(
                "Tested, not proved. The function and the pattern were run on the same links \
                 and agreed on each; a link unlike these may still route differently.",
                11.0,
                17.0,
                theme::text_muted(),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(evidence(
                        "built from the config",
                        format!("{built} links from the literals in this file"),
                    ))
                    .child(evidence(
                        "recent links",
                        if model.recent == 0 {
                            "none in Activity".to_owned()
                        } else {
                            format!("{} from Activity", model.recent)
                        },
                    ))
                    .child(evidence(
                        "agrees on",
                        format!(
                            "{} links; not a proof \u{00b7} {} matched both, the rest neither",
                            suggested.agreed, suggested.matched
                        ),
                    )),
            ),
    )
}

/// `.ev`: one line of a suggestion's evidence.
fn evidence(label: &'static str, value: String) -> Div {
    div()
        .flex()
        .gap(px(12.0))
        .child(
            div()
                .w(px(150.0))
                .flex_none()
                .child(mono(label, 10.5, 500.0, theme::text_subtle())),
        )
        .child(mono(value, 10.5, 500.0, theme::text_secondary()))
}

/// `.idet`: the selected rule's source, verdict and what it becomes.
fn detail(model: &ImportModel, cx: &mut Context<'_, Importer>) -> Stateful<Div> {
    let pane = div()
        .id("importer-detail")
        .flex_1()
        .min_w_0()
        .overflow_x_hidden()
        .h_full()
        .flex()
        .flex_col()
        .gap(px(14.0))
        .py(px(18.0))
        .px(px(22.0))
        .overflow_y_scroll();
    let Some(item) = model.items.get(model.selected) else {
        return pane;
    };
    let kind = match item.kind {
        Kind::Rewrite => "Rewrite",
        Kind::Handler => "Handler",
        Kind::Default => "Default browser",
    };
    let at = item
        .line()
        .map_or_else(String::new, |line| format!(" line {line}"));
    let becomes_label = match item.kind {
        Kind::Rewrite => "Becomes a visual rewrite \u{00b7} after yours",
        Kind::Handler => "Becomes a visual handler \u{00b7} position set in Place",
        Kind::Default => "Becomes Everything else",
    };
    pane.child(heading(format!("{kind} \u{00b7} {}{at}", model.shown)))
        .when_some(item.source.as_ref(), |pane, lines| {
            pane.child(source_frame(&model.shown, lines))
        })
        .child(verdict_box(item))
        .when(item.label() == Label::Imports, |pane| {
            pane.child(heading(becomes_label)).children(becomes(item))
        })
        .children(suggestion(model, model.selected, cx))
}

/// 3c: the config did not evaluate, so there is nothing to review.
fn problem(model: &ImportModel, problem: &str, cx: &mut Context<'_, Importer>) -> Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .gap(px(14.0))
        .py(px(20.0))
        .px(px(24.0))
        .child(
            banner_frame(theme::danger_soft(), theme::danger_soft())
                .items_start()
                .debug_selector(|| "importer-problem".into())
                .child(text("!", 16.0, 700.0, theme::danger_text()))
                .child(banner_text(
                    format!("Can't read {}", model.shown),
                    "The importer reads what the config produces, so it has to run first. It \
                     stopped here:"
                        .to_owned(),
                )),
        )
        .child(
            div()
                .border_1()
                .border_color(theme::border_subtle())
                .rounded(px(8.0))
                .bg(theme::bg_app())
                .py(px(8.0))
                .px(px(10.0))
                .child(paragraph(
                    problem.to_owned(),
                    11.0,
                    18.7,
                    theme::danger_text(),
                )),
        )
        .child(
            div().flex().items_center().gap(px(8.0)).child(
                action_button(
                    "importer-choose",
                    "Choose another file\u{2026}".into(),
                    Look::Quiet,
                    true,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.choose_another(window, cx);
                })),
            ),
        )
}

/// `.slot`: a radio tile.
fn slot(
    id: ElementId,
    selector: String,
    title: String,
    selected: bool,
    lines: Vec<(String, bool)>,
) -> Stateful<Div> {
    lines.into_iter().fold(
        div()
            .id(id)
            .debug_selector(move || selector.clone())
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(7.0))
            .py(px(12.0))
            .px(px(13.0))
            .border_1()
            .rounded(px(8.0))
            .cursor_pointer()
            .map(|slot| {
                if selected {
                    slot.bg(theme::bg_row_active())
                        .border_color(theme::accent_border())
                } else {
                    slot.bg(theme::bg_input())
                        .border_color(theme::border_subtle())
                }
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .child(text(
                        title,
                        12.5,
                        600.0,
                        if selected {
                            theme::accent_bright()
                        } else {
                            theme::text_primary()
                        },
                    ))
                    .child(
                        div()
                            .flex_none()
                            .size(px(14.0))
                            .rounded_full()
                            .map(|radio| {
                                if selected {
                                    radio.border_4().border_color(theme::accent())
                                } else {
                                    radio.border_1().border_color(theme::border_strong())
                                }
                            }),
                    ),
            ),
        |slot, (line, warn)| {
            slot.child(paragraph(
                line,
                if warn { 10.5 } else { 11.0 },
                16.0,
                if warn {
                    theme::warn_text()
                } else {
                    theme::text_muted()
                },
            ))
        },
    )
}

/// `.ipane`'s heading: eyebrow, title and help.
fn step_heading(eyebrow: &str, title: String, help: String) -> Div {
    div()
        .flex()
        .flex_col()
        .child(tracked(eyebrow, 9.5, 600.0, 0.14, theme::accent()))
        .child(
            text(title, 17.0, 700.0, theme::text_primary())
                .mt(px(6.0))
                .line_height(px(20.4)),
        )
        .child(
            paragraph(help, 11.5, 18.4, theme::text_muted())
                .mt(px(6.0))
                .max_w(px(420.0)),
        )
}

/// 2c: where the imported handlers go, previewed.
fn place(model: &ImportModel, cx: &mut Context<'_, Importer>) -> Stateful<Div> {
    let yours = model.existing.rules.len();
    let choices = [
        (
            Placement::Before,
            "Before yours",
            vec![(
                "The config's handlers win where both match. Closest to how the config routed \
                 until now."
                    .to_owned(),
                false,
            )],
        ),
        (
            Placement::After,
            "After yours",
            vec![("Your handlers win where both match.".to_owned(), false)],
        ),
        (
            Placement::Replace,
            "Replace yours",
            vec![(
                format!(
                    "Your {} removed. Undo import restores them.",
                    if yours == 1 {
                        "handler is".to_owned()
                    } else {
                        format!("{yours} handlers are")
                    }
                ),
                false,
            )],
        ),
    ];
    let mut slots = div().flex().gap(px(10.0));
    for (i, (placement, title, lines)) in choices.into_iter().enumerate() {
        slots = slots.child(
            slot(
                ElementId::named_usize("importer-place", i),
                format!("importer-place-{i}"),
                title.to_owned(),
                model.placement == placement,
                lines,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.model.placement = placement;
                cx.notify();
            })),
        );
    }
    let plan = model.plan(&model.existing);
    let duplicates = if model.placement == Placement::Replace {
        Vec::new()
    } else {
        model.duplicates(&model.existing)
    };
    let dupes = duplicates.iter().filter_map(|(item, at)| {
        let item = model.items.get(*item)?;
        let row = item.becomes()?;
        let what = row
            .matchers
            .iter()
            .filter_map(|m| match m {
                crate::rules_model::Matcher::Pattern(p) => Some(p.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(crate::widgets::chip("skipped"))
                .child(tag(what))
                .child(arrow())
                .child(paragraph(
                    format!(
                        "{} is already your handler {}.",
                        match &row.destination {
                            crate::rules_model::Destination::Browser { name, .. } =>
                                name.to_string(),
                            _ => String::new(),
                        },
                        at + 1
                    ),
                    11.0,
                    16.5,
                    theme::text_muted(),
                )),
        )
    });
    // The preview: the handlers as the file will hold them, the imported
    // ones tinted.
    let imported: Vec<pwrroute_engine::Rule> = plan
        .file
        .rules
        .iter()
        .filter(|rule| {
            !model.existing.rules.contains(rule) || model.placement == Placement::Replace
        })
        .cloned()
        .collect();
    let rows = plan.file.rules.len();
    let preview = plan
        .file
        .rules
        .iter()
        .enumerate()
        .fold(card(), |card, (i, rule)| {
            let new = imported.contains(rule);
            let mut row =
                crate::rules_model::handler(rule, crate::rules_model::RowBadge::None, Vec::new())
                    .rule;
            row.hits = if new { "imported" } else { "yours" }.into();
            card.child(
                rule_row(
                    ElementId::named_usize("importer-preview", i),
                    &format!("{:02}", i + 1),
                    &row,
                    RowState {
                        last: i + 1 == rows,
                        ..RowState::default()
                    },
                    None,
                )
                .when(new, |row| row.bg(theme::accent_tint())),
            )
        });
    div()
        .id("importer-place")
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(14.0))
        .py(px(20.0))
        .px(px(24.0))
        .overflow_y_scroll()
        .child(step_heading(
            "Place",
            format!("You already have {}", plural(yours, "handler")),
            "Imported handlers keep their order among themselves. Choose where the group goes."
                .to_owned(),
        ))
        .child(slots)
        .children(dupes)
        .child(tracked(
            "Handlers after import",
            9.5,
            600.0,
            0.14,
            theme::text_muted(),
        ))
        .child(preview)
}

/// 3a: what the router reads afterwards. The choice is about the config
/// in charge, which need not be the file under review.
fn choose_source(model: &ImportModel, cx: &mut Context<'_, Importer>) -> Stateful<Div> {
    let behind = model.left_behind();
    let plan = model.plan(&model.existing);
    let stays = behind.len();
    let imported = plural(plan.count(), "rule");
    let (in_charge, keep, lose, title, help) = match &model.charge {
        Charge::Other { shown, .. } => (
            shown.clone(),
            format!(
                "{shown} runs first, as now; the {imported} imported run after it. Nothing \
                 stops working."
            ),
            vec![(
                format!("The router stops reading {shown}: every rule in it stops running."),
                false,
            )],
            format!("{shown} is in charge"),
            format!(
                "{} is not the config the router reads, so its {} that stay in JavaScript do \
                 not run either way. Choose what the router reads after the import.",
                model.shown,
                plural(stays, "rule")
            ),
        ),
        Charge::This | Charge::NoConfig => {
            let mut lose = vec![(
                format!(
                    "The router stops reading {}. These stop running:",
                    model.shown
                ),
                false,
            )];
            lose.extend(behind.iter().map(|item| (item.describe(), true)));
            (
                model.shown.clone(),
                format!(
                    "The config runs first, as now, with the {}; the {imported} imported run \
                     after it. Nothing stops working.",
                    plural(stays, "JS rule")
                ),
                lose,
                format!(
                    "{} {} in JavaScript",
                    plural(stays, "rule"),
                    if stays == 1 { "stays" } else { "stay" }
                ),
                format!(
                    "They only run while {} is selected. Choose what the router reads after \
                     the import.",
                    model.shown
                ),
            )
        }
    };
    let choices = [
        (
            SourceChoice::KeepConfig,
            format!("Keep {in_charge} in charge"),
            vec![(keep, false)],
        ),
        (
            SourceChoice::VisualOnly,
            "Visual rules only".to_owned(),
            lose,
        ),
    ];
    let mut slots = div().flex().gap(px(10.0));
    for (i, (choice, title, lines)) in choices.into_iter().enumerate() {
        slots = slots.child(
            slot(
                ElementId::named_usize("importer-source", i),
                format!("importer-source-{i}"),
                title,
                model.source == choice,
                lines,
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.model.source = choice;
                cx.notify();
            })),
        );
    }
    div()
        .id("importer-source")
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(14.0))
        .py(px(20.0))
        .px(px(24.0))
        .overflow_y_scroll()
        .child(step_heading("Choose source", title, help))
        .child(slots)
        .child(paragraph(
            format!(
                "{} is not modified either way. rules.json is saved once; the previous version \
                 is kept for Undo import.",
                model.shown
            ),
            10.5,
            15.75,
            theme::text_subtle(),
        ))
}

/// `.imp-foot`: the summary and the step's buttons.
fn footer(model: &ImportModel, cx: &mut Context<'_, Importer>) -> Div {
    let plan = model.plan(&model.existing);
    let summary = match model.step {
        Step::Review => model.review_footer(),
        Step::Place => "Rewrites go after yours, since every rewrite runs anyway.".to_owned(),
        Step::Source => model.plan_summary(&plan),
    };
    let next_label: SharedString = match model.next_step() {
        Some(Step::Place) => "Next: Place".into(),
        Some(Step::Source) => "Next: Choose source".into(),
        _ => model.import_label(&plan).into(),
    };
    let first = model.step == Step::Review;
    let can_go = model.problem.is_none() && (model.next_step().is_some() || plan.count() > 0);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(10.0))
        .py(px(12.0))
        .px(px(16.0))
        .border_t_1()
        .border_color(theme::border_subtle())
        .bg(theme::bg_panel())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(8.0))
                .when(first && model.problem.is_none(), |row| {
                    row.child(checkbox(model.checked() > 0, true))
                })
                .map(|row| match &model.refused {
                    Some(reason) => row.child(
                        div()
                            .debug_selector(|| "importer-refused".into())
                            .child(text(
                                format!("Nothing was imported: {reason}"),
                                11.5,
                                500.0,
                                theme::danger_text(),
                            )),
                    ),
                    None => row.child(text(summary, 11.5, 500.0, theme::text_secondary())),
                }),
        )
        .map(|row| {
            if first {
                row.child(
                    action_button(CANCEL_ID, "Cancel".into(), Look::Quiet, false).on_click(
                        cx.listener(|_, _: &ClickEvent, window, _| window.remove_window()),
                    ),
                )
            } else {
                row.child(
                    action_button(BACK_ID, "Back".into(), Look::Quiet, false)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.back(cx))),
                )
            }
        })
        .child(
            action_button(NEXT_ID, next_label, Look::Primary, false)
                .when(!can_go, |button| button.opacity(0.5))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    if can_go {
                        this.next(window, cx);
                    }
                })),
        )
}

impl Render for Importer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let model = self.model.clone();
        let body = match (&model.problem, model.step) {
            (Some(problem_text), _) => problem(&model, problem_text, cx).into_any_element(),
            (None, Step::Review) => div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .flex()
                .child(list(&model, cx))
                .child(detail(&model, cx))
                .into_any_element(),
            (None, Step::Place) => place(&model, cx).into_any_element(),
            (None, Step::Source) => choose_source(&model, cx).into_any_element(),
        };
        div()
            .id(ROOT_ID)
            .debug_selector(|| ROOT_ID.into())
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::bg_app())
            .text_color(theme::text_primary())
            .font_family(theme::SANS)
            .child(title_strip(&model))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .flex()
                    .child(body),
            )
            .child(footer(&model, cx))
    }
}
