//! First run: when the window opens on its own, what it is told, and what
//! its choices do.
//!
//! - [`State`] is `state.json` in the data directory: whether first run was
//!   finished or dismissed (it never opens on its own again once it was),
//!   and the choices made. `--first-run` still forces the window.
//! - [`Facts`] is what the window is built from: who handles links today,
//!   the installed browsers, the Finicky config the router found (its
//!   counts, its fixed `defaultBrowser`, or why it does not load), and
//!   `rules.json`'s default, rules and `precedence`. [`Facts::probe`] reads
//!   the machine; tests write their own and build the model with [`model`].
//! - [`hooks`] makes the window's choices real. The Everything else browser
//!   goes to `rules.json`'s `defaultBrowser` and `defaultProfile` (empty for
//!   the browser's own default profile), and is written as soon as the
//!   window preselects it; the owner of everything else goes to its
//!   `precedence`. Keep records the choice; Don't use it also turns the
//!   config off (`settings.json`'s `source.visualOnly`, and
//!   `router::choose_source`); Import N rules opens the importer on Choose
//!   source and tells the window how it was left, without opening the main
//!   window. Look again rescans the browsers. "Set as default browser" asks
//!   the OS through [`crate::default_browser`], and "Open PwrRoute" or
//!   "Later" marks first run done and opens the main window on the
//!   first-run window's display, so leaving the window never leaves the
//!   app without one. Closing the window instead only marks first run
//!   done: the menu bar item stays, and Overview shows what is left.

use std::{
    fs, io,
    path::{Path, PathBuf},
    rc::Rc,
};

use anyhow::{Context as _, Result};
use gpui_kit::{
    App, AppContext as _, Context, DisplayId, ParentElement as _, Render, Styled as _,
    Subscription, Window, WindowHandle,
};
use pwrroute_engine::{
    AppType, BrowserSpec, Engine, Limits, NativeHost, Precedence, RulesFile, discovery,
};
use serde::{Deserialize, Serialize};

use crate::app::Screen;
use crate::default_browser::{self, DefaultState};
use crate::first_run::{
    self, BrowserChoice, FinickyConfig, FirstRun, FirstRunHooks, FirstRunModel, ImportChoice,
    ImportOutcome, Imported, ProfileAccess,
};
use crate::tray::model::BrowserLabel;
use crate::widgets::badge_for;

/// The file in `pwrroute_os::data_dir()`.
pub const STATE_FILE: &str = "state.json";

/// What first run has done so far. Field order is the file's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct State {
    /// The window was finished, or closed: it does not open on its own
    /// again.
    pub first_run_completed: bool,
    /// The Everything else browser, as written to `rules.json`'s
    /// `defaultBrowser`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    /// Its profile, as written to `rules.json`'s `defaultProfile`; `None`
    /// for the browser's own default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_profile: Option<String>,
    /// The config step's answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub import: Option<ImportChoice>,
}

impl State {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join(STATE_FILE)
    }

    /// Reads the file. A missing file is a fresh install.
    pub fn load(dir: &Path) -> Result<Self> {
        let path = Self::path(dir);
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).with_context(|| path.display().to_string()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| path.display().to_string()),
        }
    }

    /// Writes the file, creating the directory. Through a sibling file and a
    /// rename, so a crash leaves the old file or the new one, never half.
    pub fn save(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir).with_context(|| dir.display().to_string())?;
        let path = Self::path(dir);
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        let mut staging = path.as_os_str().to_owned();
        staging.push(".saving");
        fs::write(&staging, text).with_context(|| path.display().to_string())?;
        fs::rename(&staging, &path).with_context(|| path.display().to_string())
    }

    /// Whether first run should open on its own in `dir`'s install: not once
    /// it was completed or dismissed. A file that cannot be read counts as
    /// completed, so a bad file does not nag on every launch.
    pub fn pending(dir: &Path) -> bool {
        match Self::load(dir) {
            Ok(state) => !state.first_run_completed,
            Err(error) => {
                log::warn!("cannot read {}: {error:#}", Self::path(dir).display());
                false
            }
        }
    }
}

/// First run is over: it does not open on its own again.
fn mark_completed(dir: Option<&Path>) {
    edit_state(dir, |state| state.first_run_completed = true);
}

/// Loads, edits and saves the state, reporting a failure on stderr: the
/// window's choices must not fail over a file.
fn edit_state(dir: Option<&Path>, edit: impl FnOnce(&mut State)) {
    let Some(dir) = dir else {
        return;
    };
    let result = save_state(Some(dir), edit);
    if let Err(error) = result {
        log::warn!("cannot save first-run state: {error:#}");
    }
}

/// Persist a source-choice answer before reporting that step complete.
fn save_state(dir: Option<&Path>, edit: impl FnOnce(&mut State)) -> Result<(), String> {
    let dir = dir.ok_or("no data directory: first-run choice is not saved")?;
    State::load(dir)
        .and_then(|mut state| {
            edit(&mut state);
            state.save(dir)
        })
        .map_err(|error| format!("{error:#}"))
}

/// Loads, edits and saves `rules.json`; `Err` is why it was not saved,
/// for the window to show (a `rules.json` that does not load is never
/// overwritten). Written directly, as the first-run window runs before the
/// main window's store; the store follows the file when it opens. With no
/// home directory there is no file to write.
pub fn edit_rules(path: Option<&Path>, edit: impl FnOnce(&mut RulesFile)) -> Result<(), String> {
    let Some(path) = path else {
        return Ok(());
    };
    let held = pwrroute_sync::LocalFile::try_lock(path).map_err(|error| format!("{error:#}"))?;
    let path = held.path();
    let mut rules = RulesFile::load(path)
        .map_err(|_| "rules.json did not load, so it is not overwritten".to_owned())?;
    edit(&mut rules);
    rules.save(path).map_err(|error| format!("{error:#}"))
}

/// A Finicky config the router found, and what it holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinickyFound {
    pub path: PathBuf,
    pub rewrites: usize,
    pub handlers: usize,
    /// The rules the importer proves visual rules hold exactly.
    pub importable: usize,
    /// Its fixed `defaultBrowser`, as the config names it, and the
    /// profile; `None` for a function or `null`.
    pub default_browser: Option<String>,
    pub default_profile: Option<String>,
    /// Why it does not load, as the engine said it.
    pub error: Option<String>,
}

impl FinickyFound {
    #[cfg(test)]
    fn read(path: PathBuf) -> Self {
        Self::read_with_stamps(path).0
    }

    /// Evaluate and review on the engine owner, retaining every module the
    /// review depends on until first run can actually open.
    fn read_with_stamps(path: PathBuf) -> (Self, crate::engine_worker::Stamps) {
        let before = crate::engine_worker::primary(Some(&path));
        let engine = match Engine::load(&path, Rc::new(NativeHost::new()), Limits::default()) {
            Ok(engine) => engine,
            Err(error) => {
                return (
                    Self {
                        error: Some(format!("{error:#}")),
                        path,
                        ..Self::default()
                    },
                    before,
                );
            }
        };
        let stamps = crate::engine_worker::dependencies(before, &engine);
        let description = match engine.describe() {
            Ok(description) => description,
            Err(error) => {
                return (
                    Self {
                        error: Some(format!("{error:#}")),
                        path,
                        ..Self::default()
                    },
                    stamps,
                );
            }
        };
        let (rewrites, handlers, default) = description.map_or((0, 0, None), |config| {
            (
                config.rewrites.len(),
                config.handlers.len(),
                match config.default_browser {
                    BrowserSpec::Browser(browser) if browser.app_type != AppType::None => {
                        Some((browser.name, browser.profile))
                    }
                    _ => None,
                },
            )
        });
        let importable = engine
            .review_import(&[])
            .ok()
            .flatten()
            .map_or(0, |review| {
                review
                    .rewrites
                    .iter()
                    .chain(&review.handlers)
                    .chain([&review.default_browser])
                    .filter(|r| matches!(r.verdict, pwrroute_engine::Verdict::Imports(_)))
                    .count()
            });
        let (default_browser, default_profile) = match default {
            Some((name, profile)) => (
                Some(name).filter(|name| !name.is_empty()),
                Some(profile).filter(|profile| !profile.is_empty()),
            ),
            None => (None, None),
        };
        (
            Self {
                path,
                rewrites,
                handlers,
                importable,
                default_browser,
                default_profile,
                error: None,
            },
            stamps,
        )
    }
}

/// What first run is built from.
#[derive(Clone, Debug)]
pub struct Facts {
    /// Who handles links today.
    pub handler: DefaultState,
    /// The browsers on this machine and their profiles, for the
    /// Everything else selects.
    pub browsers: Vec<BrowserChoice>,
    pub finicky: Option<FinickyFound>,
    pub finicky_rules: Option<PathBuf>,
    /// `rules.json`'s `defaultBrowser`, empty when it names none.
    pub rules_default: String,
    /// `rules.json`'s `defaultProfile`, empty when it names none.
    pub rules_profile: String,
    /// `rules.json`'s rules.
    pub rules: usize,
    /// `rules.json`'s `precedence`.
    pub precedence: Precedence,
    pub state: State,
    /// The OS has a pane where PwrRoute is let read a browser's profile
    /// list (macOS's Files & Folders): a browser whose list was kept from
    /// PwrRoute then gets first run's access step.
    pub access_pane: bool,
    rules_path: Option<PathBuf>,
    state_dir: Option<PathBuf>,
}

impl Facts {
    fn refresh_saved(&mut self) {
        if let Some(rules) = self
            .rules_path
            .as_deref()
            .and_then(|path| RulesFile::load(path).ok())
        {
            self.rules_default = rules.default_browser;
            self.rules_profile = rules.default_profile;
            self.rules = rules.rules.len();
            self.precedence = rules.precedence;
        }
        if let Some(state) = self
            .state_dir
            .as_deref()
            .and_then(|dir| State::load(dir).ok())
        {
            self.state = state;
        }
    }

    /// Reads the machine: the OS's answer on the default browser (already
    /// read by [`default_browser`]), the installed browsers (the shared
    /// scan), the config the router chose, `rules.json` and `state.json`.
    pub fn probe(cx: &mut App) -> Self {
        let env = discovery::Environment::current();
        let rules = discovery::rules_path(&env)
            .and_then(|path| RulesFile::load(&path).ok())
            .unwrap_or_default();
        let state = pwrroute_os::data_dir()
            .and_then(|dir| State::load(&dir).ok())
            .unwrap_or_default();
        Self {
            handler: default_browser::state(cx),
            browsers: choices(crate::browser_scan::apps(cx), &hidden(cx)),
            finicky: None,
            finicky_rules: discovery::find_finicky_rules(&env),
            rules_default: rules.default_browser,
            rules_profile: rules.default_profile,
            rules: rules.rules.len(),
            precedence: rules.precedence,
            state,
            access_pane: cfg!(target_os = "macos"),
            rules_path: discovery::rules_path(&env),
            state_dir: pwrroute_os::data_dir(),
        }
    }
}

/// The apps Settings › Browsers hides from the pickers.
fn hidden(cx: &App) -> crate::settings_store::BrowserSettings {
    crate::settings_store::store(cx)
        .map(|store| store.read(cx).settings().browsers.clone())
        .unwrap_or_default()
}

/// The Everything else select's browsers from a scan, less the apps
/// `hidden` keeps out of the pickers: names, ids and profile names, as
/// Settings › Browsers reads them.
fn choices(
    apps: Vec<crate::browser_scan::DetectedApp>,
    hidden: &crate::settings_store::BrowserSettings,
) -> Vec<BrowserChoice> {
    apps.into_iter()
        .filter(|app| !hidden.is_hidden(app.id()))
        .map(|app| BrowserChoice {
            label: BrowserLabel::new(badge_for(&app.name), &app.name, None),
            value: app.id().to_owned(),
            profiles: app.profiles,
            profiles_blocked: app.profiles_blocked,
        })
        .collect()
}

/// The window's model for these facts. `home` shortens the config's path
/// to `~/…`.
pub fn model(facts: &Facts, home: Option<&Path>) -> FirstRunModel {
    // state.json's choice when first run made one, else rules.json's; the
    // profile comes from the same place as the browser.
    let non_empty = |value: &str| Some(value.to_owned()).filter(|value| !value.is_empty());
    let chosen = match facts.state.fallback.as_deref().and_then(non_empty) {
        Some(value) => Some((
            value,
            facts.state.fallback_profile.clone().unwrap_or_default(),
        )),
        None => non_empty(&facts.rules_default).map(|value| (value, facts.rules_profile.clone())),
    };
    let fallback =
        chosen.map(|(value, profile)| first_run::label_for(&facts.browsers, &value, &profile));
    let finicky = facts.finicky.as_ref().map(|found| {
        let path = tilde(&found.path, home);
        let default_name = found.default_browser.as_deref().map(|value| {
            facts
                .browsers
                .iter()
                .find(|choice| {
                    choice.value.eq_ignore_ascii_case(value)
                        || choice.label.name.eq_ignore_ascii_case(value)
                })
                .map_or_else(
                    || default_browser::browser_name(value),
                    |choice| choice.label.name.clone(),
                )
        });
        FinickyConfig {
            error: found
                .error
                .as_deref()
                .map(|error| load_error(error, &found.path, &path)),
            path,
            rewrites: found.rewrites,
            handlers: found.handlers,
            importable: found.importable,
            default_browser: found.default_browser.clone(),
            default_profile: found.default_profile.clone(),
            default_name,
        }
    });
    // Only Keep carries over from an earlier run. An import's count is not
    // kept, and a config still in charge was never turned off (older
    // builds' Start empty did not).
    let import = facts
        .state
        .import
        .filter(|choice| *choice == ImportChoice::KeepFile && finicky.is_some());
    let blocked: Vec<BrowserLabel> = facts
        .browsers
        .iter()
        .filter(|choice| choice.profiles_blocked)
        .map(|choice| choice.label.clone())
        .collect();
    let access = (facts.access_pane && !blocked.is_empty()).then(|| ProfileAccess {
        browsers: blocked,
        ..ProfileAccess::default()
    });
    FirstRunModel {
        access,
        fallback,
        browsers: facts.browsers.clone(),
        finicky,
        finicky_rules: facts
            .finicky_rules
            .as_ref()
            .map(|path| (path.clone(), tilde(path, home))),
        import,
        precedence: facts.precedence,
        rules: facts.rules,
        current_default: match &facts.handler {
            DefaultState::Other(_) | DefaultState::OtherCopy(_) => {
                Some(facts.handler.current_name())
            }
            DefaultState::PwrRoute | DefaultState::Unknown => None,
        },
        is_default: facts.handler.is_default(),
        ..FirstRunModel::default()
    }
}

/// Why the config does not load, as one line: the error's first line with
/// the config's full path shown as `shown`, and its position as "· shown
/// line N" when the engine gave one.
pub fn load_error(error: &str, path: &Path, shown: &str) -> String {
    let mut line = error
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_owned();
    // The engine may name the file by its canonical path (`/private/var`
    // for `/var`); the longer spelling goes first.
    let mut spellings = vec![path.display().to_string()];
    if let Ok(canonical) = path.canonicalize() {
        spellings.insert(0, canonical.display().to_string());
    }
    for full in spellings.iter().filter(|full| !full.is_empty()) {
        line = line.replace(full.as_str(), shown);
    }
    // "{shown}: {message}" reads better as "{message} · {shown}".
    match line.strip_prefix(&format!("{shown}: ")) {
        Some(message) => format!("{message} \u{00b7} {shown}"),
        None => line,
    }
}

/// `~/.finicky.ts` for a path under `home`; the path as it is otherwise.
fn tilde(path: &Path, home: Option<&Path>) -> String {
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

/// The window's choices, made real.
pub fn hooks() -> FirstRunHooks {
    hooks_with(
        pwrroute_os::data_dir(),
        discovery::rules_path(&discovery::Environment::current()),
        crate::router::config_in_charge(),
        Rc::new(crate::importer_view::set_source_and_show),
        Rc::new(|first_run, cx| {
            let connected: Vec<DisplayId> =
                cx.displays().iter().map(|display| display.id()).collect();
            #[cfg(target_os = "macos")]
            let under_mouse = crate::macos::display_under_mouse();
            #[cfg(not(target_os = "macos"))]
            let under_mouse = None;
            let display = main_window_display(first_run, under_mouse, &connected);
            crate::tray::show_screen_on(Screen::Overview, display, cx);
        }),
    )
}

/// Where the main window opens when first run hands over to it: the
/// display the first-run window was on, else the one under the mouse,
/// else (`None`) the primary display, as a cold launch opens it. A display
/// that is no longer connected is skipped.
pub fn main_window_display(
    first_run: Option<DisplayId>,
    under_mouse: Option<DisplayId>,
    connected: &[DisplayId],
) -> Option<DisplayId> {
    [first_run, under_mouse]
        .into_iter()
        .flatten()
        .find(|display| connected.contains(display))
}

/// What follows the first-run window: the main window, given the display
/// the first-run window was on.
pub type OpenMain = Rc<dyn Fn(Option<DisplayId>, &mut App)>;

/// Saves `settings.json`'s source and switches the router to it
/// (`importer_view::set_source`, which calls `router::choose_source`), and
/// shows an open main window the config now in charge
/// (`importer_view::set_source_and_show`).
pub type SetSource =
    Rc<dyn Fn(crate::settings_store::SourceSettings, &mut App) -> Result<(), String>>;

/// `rules.json` after an import, for the window: its rules and default.
fn imported(count: usize, keeps_config: bool, rules_path: Option<&Path>) -> Imported {
    let rules = rules_path
        .and_then(|path| RulesFile::load(path).ok())
        .unwrap_or_default();
    Imported {
        count,
        keeps_config,
        rules: rules.rules.len(),
        default_browser: rules.default_browser,
        default_profile: rules.default_profile,
    }
}

/// [`hooks`] over these files and this config, with `set_source` as what
/// turns the config off and `open_main` as what follows the window, given
/// the display the first-run window was on: tests give it a scratch
/// directory and count the calls (the router's source is one for the whole
/// process, so tests never switch it).
pub fn hooks_with(
    data_dir: Option<PathBuf>,
    rules_path: Option<PathBuf>,
    config: Option<PathBuf>,
    set_source: SetSource,
    open_main: OpenMain,
) -> FirstRunHooks {
    let fallback_dir = data_dir.clone();
    let import_dir = data_dir.clone();
    let finish_dir = data_dir;
    let fallback_rules = rules_path.clone();
    let precedence_rules = rules_path.clone();
    let copy_rules_path = rules_path.clone();
    let pending_import = Rc::new(std::cell::RefCell::new(None::<Imported>));
    let import_session = crate::importer_view::ImportSession::default();
    FirstRunHooks {
        on_import_rules: Rc::new(move |path, reply, cx| {
            let rules = copy_rules_path.clone();
            let session = crate::importer_view::ImportSession::default();
            let completed = session.clone();
            let on_done: crate::importer_view::OnDone = Rc::new(move |outcome, cx| {
                let result = match outcome {
                    crate::importer_view::ImportOutcome::Cancelled => None,
                    crate::importer_view::ImportOutcome::Imported {
                        count,
                        keeps_config,
                        source_error,
                    } => {
                        let config = crate::router::config_in_charge();
                        let file = rules
                            .as_deref()
                            .and_then(|path| RulesFile::load(path).ok())
                            .unwrap_or_default();

                        let result = crate::engine_worker::service(cx).submit(
                            crate::engine_worker::Lane::Onboarding,
                            std::time::Duration::ZERO,
                            move |_| crate::try_it::test_steps(config, file),
                        );
                        let reply = reply.clone();
                        let completed = completed.clone();
                        let rules = rules.clone();
                        let task = cx.spawn(async move |cx| {
                            if let Ok((report, _stamps)) = crate::engine_worker::receive(result).await {
                                cx.update(|cx| {
                                    let unchanged = completed.validate_completed(cx).is_ok()
                                        && crate::engine_worker::unchanged(&_stamps);
                                    let summary = if !unchanged {
                                        format!("Copied {count} entries. Rules, source or config changed while tests ran; run saved examples and tests again in Rules.")
                                    } else {
                                        match report {
                                            Ok(report) => format!("Copied {count} entries. Saved examples and tests: {} of {} passed. See Rules for any failures.", report.passed, report.total),
                                            Err(_) => format!("Copied {count} entries. Saved examples and tests could not run; see Rules for details."),
                                        }
                                    };
                                    let summary = match &source_error {
                                        Some(reason) => format!("{summary} {reason}"),
                                        None => summary,
                                    };
                                    // The copy count is historical; these facts describe the file
                                    // now, including a later fallback choice, edit or Undo.
                                    let current = rules.as_deref().and_then(|path| RulesFile::load(path).ok()).map(|file| Imported {
                                        count, keeps_config, rules: file.rules.len(),
                                        default_browser: file.default_browser,
                                        default_profile: file.default_profile,
                                    });
                                    reply(Some(first_run::RulesImportResult { current, summary }), cx);
                                });
                            }
                        });
                        crate::engine_worker::hold(
                            cx,
                            crate::engine_worker::Lane::Onboarding,
                            task,
                        );
                        return;
                    }
                };
                reply(result, cx);
            });
            let rules = copy_rules_path.clone();
            cx.defer(move |cx| crate::importer_view::open_from_first_run_session(path, rules, on_done, session, cx));
        }),
        on_fallback: Rc::new(move |choice: first_run::Fallback, _| {
            edit_rules(fallback_rules.as_deref(), |rules| {
                rules.default_browser.clone_from(&choice.browser.value);
                rules.default_profile = choice.profile.clone().unwrap_or_default();
            })?;
            edit_state(fallback_dir.as_deref(), |state| {
                state.fallback = Some(choice.browser.value.clone());
                state.fallback_profile.clone_from(&choice.profile);
            });
            Ok(())
        }),
        on_precedence: Rc::new(move |precedence, _| {
            edit_rules(precedence_rules.as_deref(), |rules| {
                rules.precedence = precedence;
            })
        }),
        on_import: Rc::new(move |choice, reply, cx| match choice {
            ImportChoice::KeepFile | ImportChoice::DontUse => {
                let result = (|| {
                    let mut source = crate::settings_store::store(cx)
                        .map(|store| store.read(cx).settings().source.clone())
                        .unwrap_or_default();
                    source.visual_only = choice == ImportChoice::DontUse;
                    source.config = if source.visual_only {
                        None
                    } else {
                        config.clone()
                    };
                    set_source(source, cx)?;
                    save_state(import_dir.as_deref(), |state| state.import = Some(choice))
                        .map_err(|reason| format!("Source saved, but the first-run choice was not saved: {reason}. Try again."))
                })();
                let outcome = match result {
                    Ok(()) => ImportOutcome::Chosen(choice),
                    Err(reason) => ImportOutcome::Failed {
                        imported: None,
                        reason,
                    },
                };
                cx.defer(move |cx| reply(outcome, cx));
            }
            // The importer on Choose source, every proven rule checked; it
            // writes rules.json and answers when it closes.
            ImportChoice::Import => {
                let retry = pending_import.borrow().clone();
                if let Some(done) = retry {
                    let outcome = match import_session.revalidate_completed(cx) {
                        Ok(()) => record_import(import_dir.as_deref(), done, &pending_import),
                        Err(reason) => {
                            pending_import.borrow_mut().take();
                            import_session.clear();
                            ImportOutcome::Failed {
                                imported: None,
                                reason,
                            }
                        }
                    };
                    if matches!(outcome, ImportOutcome::Imported(_)) {
                        import_session.clear();
                    }
                    cx.defer(move |cx| reply(outcome, cx));
                    return;
                }
                // Answered on the next turn: the window is still mid-update.
                let Some(path) = config.clone() else {
                    cx.defer(move |cx| reply(ImportOutcome::Cancelled, cx));
                    return;
                };
                let dir = import_dir.clone();
                let rules = rules_path.clone();
                let pending = pending_import.clone();
                let completion_session = import_session.clone();
                let on_done: crate::importer_view::OnDone = Rc::new(move |outcome, cx| {
                    let outcome = match outcome {
                        crate::importer_view::ImportOutcome::Cancelled => ImportOutcome::Cancelled,
                        crate::importer_view::ImportOutcome::Imported {
                            count,
                            keeps_config,
                            source_error,
                        } => {
                            let done = imported(count, keeps_config, rules.as_deref());
                            match source_error {
                                Some(reason) => ImportOutcome::Failed {
                                    imported: Some(done),
                                    reason,
                                },
                                None => record_import(dir.as_deref(), done, &pending),
                            }
                        }
                    };
                    if matches!(outcome, ImportOutcome::Imported(_)) {
                        completion_session.clear();
                    }
                    reply(outcome, cx);
                });
                let rules = rules_path.clone();
                let session = import_session.clone();
                cx.defer(move |cx| {
                    crate::importer_view::open_from_first_run_session(
                        path, rules, on_done, session, cx,
                    );
                });
            }
        }),
        on_rescan: Rc::new(|cx| {
            let hidden = hidden(cx);
            choices(crate::browser_scan::rescan(cx), &hidden)
        }),
        on_open_access_settings: Rc::new(|_| {
            pwrroute_os::open_files_and_folders_settings().map_err(|error| {
                log::warn!("first run could not open Files & Folders: {error:#}");
                format!("{error:#}")
            })
        }),
        // The pickers and Settings read the same scan, so they get the
        // profiles too.
        on_reread: Rc::new(|cx| {
            let hidden = hidden(cx);
            let apps = crate::browser_scan::reread(cx);
            crate::browser_scan::refresh_windows(cx);
            choices(apps, &hidden)
        }),
        on_set_default_browser: Rc::new(|cx| match default_browser::request(cx) {
            Ok(outcome) => {
                log::info!("first run asked the OS for the default browser: {outcome:?}");
                Ok(())
            }
            Err(error) => {
                log::warn!("first run could not ask for the default browser: {error}");
                Err(format!("{error:#}"))
            }
        }),
        // Done or Later alike: the window closes itself next, and the main
        // window is what the reader is left with.
        on_finish: Rc::new(move |finished: first_run::Finished, cx| {
            mark_completed(finish_dir.as_deref());
            open_main(finished.display, cx);
        }),
    }
}

/// Rules and source are already saved. Retrying a failed state write must
/// only record the answer, without importing the same group a second time.
fn record_import(
    dir: Option<&Path>,
    done: Imported,
    pending: &std::cell::RefCell<Option<Imported>>,
) -> ImportOutcome {
    match save_state(dir, |state| state.import = Some(ImportChoice::Import)) {
        Ok(()) => {
            pending.take();
            ImportOutcome::Imported(done)
        }
        Err(reason) => {
            *pending.borrow_mut() = Some(done.clone());
            ImportOutcome::Failed {
                imported: Some(done),
                reason: format!(
                    "Rules and source saved, but the first-run choice was not saved: {reason}. Try Import again to save the choice."
                ),
            }
        }
    }
}

/// The open first-run window and what follows it.
struct Open {
    window: WindowHandle<FirstRun>,
    _answers: Subscription,
}

/// Follow only completed snapshots; completion must never start a reread.
pub fn install(cx: &mut App) {
    crate::browser_scan::on_complete(cx, Rc::new(refresh_browsers));
}

fn refresh_browser_model(
    model: &mut FirstRunModel,
    browsers: Vec<BrowserChoice>,
    access_pane: bool,
) {
    let before = model.still_blocked().len();
    model.browsers = browsers;
    let blocked: Vec<_> = model
        .browsers
        .iter()
        .filter(|browser| browser.profiles_blocked)
        .map(|browser| browser.label.clone())
        .collect();
    if model.access.is_none() && access_pane && !blocked.is_empty() {
        model.access = Some(ProfileAccess {
            browsers: blocked,
            ..Default::default()
        });
    } else if let Some(access) = model.access.as_mut() {
        for label in blocked {
            if !access.browsers.iter().any(|known| known.name == label.name) {
                access.browsers.push(label);
            }
        }
        if before > model.still_blocked().len() {
            model.access.as_mut().unwrap().still_blocked = false;
        }
    }
}

/// Refresh only discovered browsers; retain the person's setup choices.
pub fn refresh_browsers(cx: &mut App) {
    if crate::browser_scan::is_loading(cx) || crate::browser_scan::error(cx).is_some() {
        return;
    }
    let browsers = choices(crate::browser_scan::apps(cx), &hidden(cx));
    for window in cx.windows() {
        if let Some(window) = window.downcast::<FirstRun>() {
            let _ = window.update(cx, |view, _, cx| {
                refresh_browser_model(&mut view.model, browsers.clone(), cfg!(target_os = "macos"));
                view.preselect(cx);
                cx.notify();
            });
        }
    }
}

impl gpui_kit::Global for Open {}

/// Opens first run over the machine's facts, with the Everything else
/// browser preselected and saved when none was chosen. The default-browser
/// step follows the OS's answer (read every second while the window is open
/// and PwrRoute is not the default, and whenever the window comes to the
/// front), and closing the window, however, marks first run done.
pub fn open(cx: &mut App) {
    // Help › Replay Onboarding… while it is open brings it forward.
    if let Some(open) = cx.try_global::<Open>().map(|open| open.window)
        && open
            .update(cx, |_, window, cx| {
                crate::menu_bar::bring_forward(window, cx)
            })
            .is_ok()
    {
        cx.activate(true);
        return;
    }
    if let Some(window) = cx.try_global::<Opening>().map(|opening| opening.window) {
        let _ = window.update(cx, |_, window, cx| {
            crate::menu_bar::bring_forward(window, cx)
        });
        return;
    }
    let facts = Facts::probe(cx);
    let config = crate::router::config_in_charge();
    prepare(facts, config, cx);
}

fn prepare(facts: Facts, config: Option<PathBuf>, cx: &mut App) {
    if config.is_none()
        && !crate::browser_scan::is_loading(cx)
        && crate::browser_scan::error(cx).is_none()
    {
        open_ready(facts, cx);
        return;
    }
    let request = Rc::new(());
    let display = crate::placement::display_for_new_window(cx);
    let bounds = gpui_kit::Bounds::centered(
        display,
        gpui_kit::size(crate::theme::px(680.0), crate::theme::px(320.0)),
        cx,
    );
    let token = request.clone();
    let Ok(window) = cx.open_window(
        gpui_kit::WindowOptions {
            window_bounds: Some(gpui_kit::WindowBounds::Windowed(bounds)),
            display_id: display,
            titlebar: Some(gpui_kit::TitlebarOptions {
                title: Some("Set up PwrRoute".into()),
                ..Default::default()
            }),
            app_id: Some("pwrroute".into()),
            ..Default::default()
        },
        |_, cx| {
            cx.new(|cx| {
                cx.on_release(move |_, cx| {
                    if cx
                        .try_global::<Opening>()
                        .is_some_and(|opening| Rc::ptr_eq(&opening.request, &token))
                    {
                        cx.remove_global::<Opening>();
                        crate::engine_worker::cancel_service(
                            cx,
                            crate::engine_worker::Lane::Config,
                        );
                    }
                })
                .detach();
                Preparing { failure: None }
            })
        },
    ) else {
        return;
    };
    cx.set_global(Opening {
        request: request.clone(),
        window,
        facts,
        config: config.clone(),
        config_ready: config.is_none(),
        stamps: Vec::new(),
    });
    cx.activate(true);
    if config.is_some() {
        let result =
            crate::engine_worker::service(cx).once(crate::engine_worker::Lane::Config, move || {
                let (found, stamps) = FinickyFound::read_with_stamps(config.unwrap());
                (Some(found), stamps)
            });
        let token = request.clone();
        let task = cx.spawn(async move |cx| {
            if let Ok((finicky, stamps)) = crate::engine_worker::receive(result).await {
                cx.update(|cx| {
                    if !cx
                        .try_global::<Opening>()
                        .is_some_and(|opening| Rc::ptr_eq(&opening.request, &token))
                    {
                        return;
                    }
                    if !crate::engine_worker::unchanged(&stamps) {
                        restart_opening(cx);
                        return;
                    }
                    let opening = cx.global_mut::<Opening>();
                    opening.facts.finicky = finicky;
                    opening.config_ready = true;
                    opening.stamps = stamps;
                    finish_opening(cx);
                });
            }
        });
        crate::engine_worker::hold(cx, crate::engine_worker::Lane::Config, task);
    }
    // Completion callbacks handle successful scans; this also exposes a
    // failed scan as an error with Retry instead of an endless spinner.
    let task = cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(250))
                .await;
            let keep = cx.update(|cx| {
                if !cx
                    .try_global::<Opening>()
                    .is_some_and(|opening| Rc::ptr_eq(&opening.request, &request))
                {
                    return false;
                }
                finish_opening(cx);
                cx.try_global::<Opening>().is_some()
            });
            if !keep {
                break;
            }
        }
    });
    crate::engine_worker::hold(cx, crate::engine_worker::Lane::Onboarding, task);
}

struct Opening {
    request: Rc<()>,
    window: WindowHandle<Preparing>,
    facts: Facts,
    config: Option<PathBuf>,
    config_ready: bool,
    stamps: crate::engine_worker::Stamps,
}
impl gpui_kit::Global for Opening {}
pub(crate) struct Preparing {
    failure: Option<String>,
}
impl Render for Preparing {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl gpui_kit::IntoElement {
        use gpui_kit::{InteractiveElement as _, StatefulInteractiveElement as _};
        gpui_kit::div()
            .size_full()
            .flex()
            .flex_col()
            .justify_center()
            .p(crate::theme::px(32.0))
            .gap(crate::theme::px(16.0))
            .bg(crate::theme::bg_app())
            .child(crate::theme::text(
                "Set up PwrRoute",
                20.0,
                600.0,
                crate::theme::text_primary(),
            ))
            .child(crate::theme::text(
                self.failure
                    .clone()
                    .unwrap_or_else(|| "Reading the config and looking for browsers…".into()),
                13.0,
                500.0,
                crate::theme::text_muted(),
            ))
            .children(self.failure.as_ref().map(|_| {
                crate::widgets::button("Try again", crate::widgets::ButtonKind::Default, true)
                    .id("setup-retry-scan")
                    .on_click(cx.listener(|this, _, _, cx| {
                        crate::browser_scan::rescan(cx);
                        this.failure = None;
                        cx.notify();
                    }))
            }))
    }
}

pub fn install(cx: &mut App) {
    crate::browser_scan::on_complete(cx, Rc::new(refresh_browsers));
}

fn restart_opening(cx: &mut App) {
    if let Some(window) = cx.try_global::<Opening>().map(|opening| opening.window) {
        cx.remove_global::<Opening>();
        let _ = window.update(cx, |_, window, _| window.remove_window());
        open(cx);
    }
}

fn finish_opening(cx: &mut App) {
    let Some(opening) = cx.try_global::<Opening>() else {
        return;
    };
    let window = opening.window;
    if opening.config != crate::router::config_in_charge()
        || (opening.config_ready && !crate::engine_worker::unchanged(&opening.stamps))
    {
        restart_opening(cx);
        return;
    }
    if let Some(error) = crate::browser_scan::error(cx) {
        let _ = window.update(cx, |view, _, cx| {
            view.failure = Some(error);
            cx.notify();
        });
        return;
    }
    if !opening.config_ready || crate::browser_scan::is_loading(cx) {
        return;
    }
    let mut facts = opening.facts.clone();
    facts.refresh_saved();
    facts.browsers = choices(crate::browser_scan::apps(cx), &hidden(cx));
    facts.handler = default_browser::state(cx);
    cx.remove_global::<Opening>();
    let _ = window.update(cx, |_, window, _| window.remove_window());
    open_ready(facts, cx);
}

/// Updates browser facts only; setup choices and config review stay intact.
pub fn refresh_browsers(cx: &mut App) {
    let loading = crate::browser_scan::is_loading(cx);
    let error = crate::browser_scan::error(cx);
    let browsers = choices(crate::browser_scan::apps(cx), &hidden(cx));
    for window in cx.windows() {
        if let Some(window) = window.downcast::<FirstRun>() {
            let _ = window.update(cx, |view, _, cx| {
                view.browser_scan_completed(browsers.clone(), loading, error.clone(), cx);
                if !loading && error.is_none() {
                    view.preselect(cx);
                }
            });
        }
    }
    finish_opening(cx);
}

fn open_ready(facts: Facts, cx: &mut App) {
    let model = model(&facts, std::env::home_dir().as_deref());
    let window = match first_run::open(model, hooks(), cx) {
        Ok(window) => window,
        Err(error) => {
            log::warn!("could not open the first-run window: {error}");
            return;
        }
    };
    let _ = window.update(cx, |view, window, cx| {
        view.preselect(cx);
        default_browser::watch(window, cx);
        // Back from System Settings: read the profiles again. Not on the
        // window's first activation, which follows the scan it opened with.
        let left = std::cell::Cell::new(false);
        cx.observe_window_activation(window, move |view, window, cx| {
            if !window.is_window_active() {
                left.set(true);
            } else if left.replace(false) {
                view.recheck_access(false, cx);
            }
        })
        .detach();
    });
    let answers = default_browser::registrar(cx).map(|registrar| {
        cx.observe(&registrar, move |registrar, cx| {
            let (state, waiting) = {
                let registrar = registrar.read(cx);
                (registrar.state.clone(), registrar.waiting())
            };
            let _ = window.update(cx, |view, _, cx| {
                view.set_default_state(&state, waiting, cx);
            });
            if state.is_default() {
                mark_completed(pwrroute_os::data_dir().as_deref());
            }
        })
    });
    cx.on_window_closed(move |cx, id| {
        let open = cx.try_global::<Open>().map(|open| open.window.window_id());
        if open == Some(id) {
            crate::engine_worker::cancel_service(cx, crate::engine_worker::Lane::Onboarding);
            cx.remove_global::<Open>();
            mark_completed(pwrroute_os::data_dir().as_deref());
        }
    })
    .detach();
    cx.set_global(Open {
        window,
        _answers: answers.unwrap_or_else(|| Subscription::new(|| {})),
    });
    let id = window.window_id();
    default_browser::follow_while(
        move |cx| {
            cx.try_global::<Open>()
                .is_some_and(|open| open.window.window_id() == id)
        },
        cx,
    );
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui_kit::{Modifiers, TestAppContext, VisualTestContext};

    use super::*;
    use crate::first_run::{Picked, Step, StepState};

    #[gpui_kit::test]
    fn review_completed_scans_refresh_first_run_without_restarting_or_losing_choices(
        cx: &mut TestAppContext,
    ) {
        let (release, wait) = async_channel::bounded(3);
        let scans = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = scans.clone();
        let writes = Rc::new(std::cell::Cell::new(0));
        let written = writes.clone();
        let window = cx.update(|cx| {
            crate::assets::load_fonts(cx);
            crate::browser_scan::install(
                pwrroute_browsers::Env::fixture(
                    pwrroute_browsers::Platform::current(),
                    std::env::temp_dir(),
                ),
                cx,
            );
            crate::browser_scan::set_test_producer(
                cx,
                std::sync::Arc::new(move || {
                    observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let wait = wait.clone();
                    Box::pin(async move { wait.recv().await.unwrap() })
                }),
            );
            install(cx);
            assert!(crate::browser_scan::apps(cx).is_empty());
            let mut hooks =
                hooks_with(None, None, None, Rc::new(|_, _| Ok(())), Rc::new(|_, _| {}));
            hooks.on_fallback = Rc::new(move |_, _| {
                written.set(written.get() + 1);
                Ok(())
            });
            hooks.on_reread =
                Rc::new(|_| panic!("completion must consume the snapshot, never rescan"));
            first_run::open(FirstRunModel::default(), hooks, cx).unwrap()
        });
        let detected = |blocked| crate::browser_scan::DetectedApp {
            name: "Fixture Browser".into(),
            bundle_id: None,
            location: "/fixture/browser".into(),
            profiles: vec!["Work".into()],
            directories: vec!["Profile 1".into()],
            profiles_blocked: blocked,
        };
        cx.run_until_parked();
        release.try_send(Ok(vec![detected(true)])).unwrap();
        cx.run_until_parked();
        window
            .update(cx, |view, _, _| {
                assert_eq!(view.model.browsers.len(), 1);
                assert!(view.model.browsers[0].profiles_blocked);
                assert_eq!(
                    view.model.fallback.as_ref().unwrap().name,
                    "Fixture Browser"
                );
                view.model.fallback.as_mut().unwrap().profile = Some("Work".into());
                view.model.import = Some(ImportChoice::DontUse);
            })
            .unwrap();
        assert_eq!(writes.get(), 1);
        cx.update(|cx| {
            crate::browser_scan::reread(cx);
        });
        cx.run_until_parked();
        release.try_send(Ok(vec![detected(false)])).unwrap();
        cx.run_until_parked();
        window
            .update(cx, |view, _, _| {
                assert!(!view.model.browsers[0].profiles_blocked);
                assert_eq!(
                    view.model.fallback.as_ref().unwrap().profile.as_deref(),
                    Some("Work")
                );
                assert_eq!(view.model.import, Some(ImportChoice::DontUse));
            })
            .unwrap();
        assert_eq!(writes.get(), 1, "refresh must retain the chosen fallback");
        cx.update(|cx| {
            crate::browser_scan::reread(cx);
        });
        cx.run_until_parked();
        window
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
        release
            .try_send(Err("fixture scan failure".into()))
            .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(
                cx.windows()
                    .into_iter()
                    .all(|window| window.downcast::<FirstRun>().is_none())
            )
        });
        assert_eq!(
            scans.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "completion must not loop scans"
        );
    }

    #[test]
    fn review_late_profile_access_facts_preserve_setup_and_skip_choices() {
        let mut model = FirstRunModel::fixture();
        let fallback = model.fallback.clone();
        let mut browser = chrome();
        browser.profiles_blocked = true;
        refresh_browser_model(&mut model, vec![browser.clone()], true);
        assert_eq!(model.access.as_ref().unwrap().browsers.len(), 1);
        assert_eq!(model.still_blocked().len(), 1);
        model.access.as_mut().unwrap().skipped = true;
        model.access.as_mut().unwrap().still_blocked = true;
        browser.profiles_blocked = false;
        refresh_browser_model(&mut model, vec![browser], true);
        assert!(model.access.as_ref().unwrap().skipped);
        assert!(!model.access.as_ref().unwrap().still_blocked);
        assert!(model.still_blocked().is_empty());
        assert_eq!(model.fallback, fallback);
    }

    fn chrome() -> BrowserChoice {
        BrowserChoice {
            value: "com.google.Chrome".to_owned(),
            label: BrowserLabel::new("Ch", "Google Chrome", None),
            profiles: vec!["Personal".to_owned(), "Work".to_owned()],
            profiles_blocked: false,
        }
    }

    fn safari() -> BrowserChoice {
        BrowserChoice {
            value: "com.apple.Safari".to_owned(),
            label: BrowserLabel::new("Sf", "Safari", None),
            profiles: Vec::new(),
            profiles_blocked: false,
        }
    }

    fn facts() -> Facts {
        Facts {
            handler: DefaultState::Other("Google Chrome".into()),
            browsers: vec![
                chrome(),
                BrowserChoice {
                    value: "org.mozilla.firefox".to_owned(),
                    label: BrowserLabel::new("Ff", "Firefox", None),
                    profiles: vec!["default-release".to_owned()],
                    profiles_blocked: false,
                },
                safari(),
            ],
            finicky_rules: None,
            finicky: Some(FinickyFound {
                path: PathBuf::from("/Users/u/.finicky.ts"),
                rewrites: 2,
                handlers: 5,
                importable: 3,
                default_browser: Some("com.apple.Safari".to_owned()),
                default_profile: None,
                error: None,
            }),
            rules_default: String::new(),
            rules_profile: String::new(),
            rules: 0,
            precedence: Precedence::Config,
            state: State::default(),
            access_pane: true,
            rules_path: None,
            state_dir: None,
        }
    }

    /// A successful source sink for tests; never changes the process router.
    fn no_source() -> SetSource {
        Rc::new(|_, _| Ok(()))
    }

    /// A private directory at the staging-file path makes the save fail on
    /// every platform, including when the test runs as root.
    #[gpui_kit::test]
    fn keep_file_is_not_complete_when_its_state_cannot_be_saved(cx: &mut TestAppContext) {
        cx.update(|cx| crate::assets::load_fonts(cx));
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("state.json.saving")).unwrap();
        let hooks = hooks_with(
            Some(dir.path().to_path_buf()),
            None,
            Some(importer_config()),
            no_source(),
            Rc::new(|_, _| {}),
        );
        let window = cx.update(|cx| first_run::open(FirstRunModel::fixture(), hooks, cx).unwrap());
        let mut view = VisualTestContext::from_window(window.into(), cx);
        press(&mut view, "first-run-keep");
        assert_eq!(
            window
                .read_with(&view, |view, _| view.model.import)
                .unwrap(),
            None,
            "an unsaved choice must leave the config step pending"
        );
        assert_eq!(State::load(dir.path()).unwrap().import, None);
        assert!(
            window
                .read_with(&view, |view, _| view.model.import_error.is_some())
                .unwrap()
        );
        assert!(view.debug_bounds("first-run-config-error").is_some());
        fs::remove_dir(dir.path().join("state.json.saving")).unwrap();
        press(&mut view, "first-run-keep");
        assert_eq!(
            window
                .read_with(&view, |view, _| view.model.import)
                .unwrap(),
            Some(ImportChoice::KeepFile)
        );
        assert!(
            window
                .read_with(&view, |view, _| view.model.import_error.is_none())
                .unwrap()
        );
        assert_eq!(
            State::load(dir.path()).unwrap().import,
            Some(ImportChoice::KeepFile)
        );
    }

    #[gpui_kit::test]
    fn dont_use_reports_session_only_and_retries_the_equal_runtime_source(cx: &mut TestAppContext) {
        cx.update(|cx| crate::assets::load_fonts(cx));
        let dir = tempfile::tempdir().unwrap();
        let config = importer_config();
        let mut store = crate::settings_store::SettingsStore::load(Some(dir.path().to_path_buf()));
        store
            .update(|settings| settings.source.config = Some(config.clone()))
            .unwrap();
        cx.update(|cx| crate::settings_store::install(store, None, cx));
        fs::create_dir(dir.path().join("settings.json.saving")).unwrap();
        let hooks = hooks_with(
            Some(dir.path().to_path_buf()),
            None,
            Some(config.clone()),
            Rc::new(crate::importer_view::set_source_and_show),
            Rc::new(|_, _| {}),
        );
        let window = cx.update(|cx| first_run::open(FirstRunModel::fixture(), hooks, cx).unwrap());
        let mut view = VisualTestContext::from_window(window.into(), cx);
        press(&mut view, "first-run-dont-use");
        assert_eq!(
            window
                .read_with(&view, |view, _| view.model.import)
                .unwrap(),
            None
        );
        let error = window
            .read_with(&view, |view, _| view.model.import_error.clone())
            .unwrap()
            .unwrap();
        assert!(error.contains("session only"), "{error}");
        assert!(view.update(|_, cx| {
            crate::settings_store::store(cx)
                .unwrap()
                .read(cx)
                .settings()
                .source
                .visual_only
        }));
        let disk = crate::settings_store::Settings::load(dir.path()).unwrap();
        assert!(!disk.source.visual_only);
        assert_eq!(disk.source.config, Some(config));
        assert_eq!(State::load(dir.path()).unwrap().import, None);
        fs::remove_dir(dir.path().join("settings.json.saving")).unwrap();
        press(&mut view, "first-run-dont-use");
        assert_eq!(
            window
                .read_with(&view, |view, _| view.model.import)
                .unwrap(),
            Some(ImportChoice::DontUse)
        );
        assert!(
            crate::settings_store::Settings::load(dir.path())
                .unwrap()
                .source
                .visual_only
        );
        assert_eq!(
            State::load(dir.path()).unwrap().import,
            Some(ImportChoice::DontUse)
        );
    }

    #[gpui_kit::test]
    fn source_retry_after_import_never_writes_the_rules_again(cx: &mut TestAppContext) {
        cx.update(|cx| crate::assets::load_fonts(cx));
        for close_before_retry in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let rules = dir.path().join("rules.json");
            let mut settings =
                crate::settings_store::SettingsStore::load(Some(dir.path().to_path_buf()));
            settings
                .update(|settings| settings.source.config = Some(importer_config()))
                .unwrap();
            cx.update(|cx| crate::settings_store::install(settings, None, cx));
            fs::create_dir(dir.path().join("settings.json.saving")).unwrap();
            let hooks = hooks_with(
                Some(dir.path().to_path_buf()),
                Some(rules.clone()),
                Some(importer_config()),
                no_source(),
                Rc::new(|_, _| panic!("import opens no main window")),
            );
            let replies = Rc::new(RefCell::new(Vec::new()));
            let heard = replies.clone();
            cx.update(|cx| {
                (hooks.on_import)(
                    ImportChoice::Import,
                    Rc::new(move |outcome, _| heard.borrow_mut().push(outcome)),
                    cx,
                )
            });
            let window = importer_window(cx).unwrap();
            window
                .update(cx, |view, _, _| {
                    view.model.source = crate::importer::SourceChoice::VisualOnly
                })
                .unwrap();
            let mut review = VisualTestContext::from_window(window.into(), cx);
            press(&mut review, crate::importer_view::NEXT_ID);
            let window = importer_window(cx).expect("source failure keeps the importer open");
            let written = fs::read(&rules).unwrap();
            let count = RulesFile::load(&rules).unwrap().rules.len();
            assert!(count > 0);
            let reason = window
                .read_with(cx, |view, _| view.model.refused.clone())
                .unwrap()
                .unwrap();
            assert!(
                reason.contains("saved to rules.json") && reason.contains("session only"),
                "{reason}"
            );
            assert!(
                replies.borrow().is_empty(),
                "not reported as a completed import"
            );
            assert_eq!(State::load(dir.path()).unwrap().import, None);
            let window = if close_before_retry {
                window
                    .update(cx, |_, window, _| window.remove_window())
                    .unwrap();
                cx.run_until_parked();
                assert!(matches!(
                    replies.borrow().as_slice(),
                    [ImportOutcome::Failed {
                        imported: Some(_),
                        ..
                    }]
                ));
                let heard = replies.clone();
                cx.update(|cx| {
                    (hooks.on_import)(
                        ImportChoice::Import,
                        Rc::new(move |outcome, _| heard.borrow_mut().push(outcome)),
                        cx,
                    )
                });
                importer_window(cx).expect("reopening resumes the saved source stage")
            } else {
                window
            };
            let mut review = VisualTestContext::from_window(window.into(), cx);
            press(&mut review, "importer-source-0");
            assert_eq!(
                window.read_with(cx, |view, _| view.model.source).unwrap(),
                crate::importer::SourceChoice::VisualOnly,
                "a pending source stage retains the choice that already applied to the session"
            );
            // A source-only retry succeeds even though any rules write would fail.
            fs::create_dir(dir.path().join("rules.json.saving")).unwrap();
            fs::remove_dir(dir.path().join("settings.json.saving")).unwrap();
            let mut review = VisualTestContext::from_window(window.into(), cx);
            press(&mut review, crate::importer_view::NEXT_ID);
            assert!(importer_window(cx).is_none());
            assert_eq!(fs::read(&rules).unwrap(), written);
            let replies = replies.borrow();
            let Some(ImportOutcome::Imported(done)) = replies.last() else {
                panic!("{replies:?}")
            };
            assert_eq!(replies.len(), if close_before_retry { 2 } else { 1 });
            assert_eq!(done.rules, count);
            assert!(done.count > 0);
            assert!(!done.keeps_config);
            assert!(
                crate::settings_store::Settings::load(dir.path())
                    .unwrap()
                    .source
                    .visual_only
            );
            assert_eq!(
                State::load(dir.path()).unwrap().import,
                Some(ImportChoice::Import)
            );
        }
    }

    #[gpui_kit::test]
    fn import_state_retry_does_not_reopen_or_reimport(cx: &mut TestAppContext) {
        cx.update(|cx| crate::assets::load_fonts(cx));
        let dir = tempfile::tempdir().unwrap();
        let rules = dir.path().join("rules.json");
        let mut settings =
            crate::settings_store::SettingsStore::load(Some(dir.path().to_path_buf()));
        settings
            .update(|settings| settings.source.config = Some(importer_config()))
            .unwrap();
        cx.update(|cx| crate::settings_store::install(settings, None, cx));
        fs::create_dir(dir.path().join("state.json.saving")).unwrap();
        let hooks = hooks_with(
            Some(dir.path().to_path_buf()),
            Some(rules.clone()),
            Some(importer_config()),
            no_source(),
            Rc::new(|_, _| {}),
        );
        let replies = Rc::new(RefCell::new(Vec::new()));
        let start = |cx: &mut TestAppContext| {
            let heard = replies.clone();
            cx.update(|cx| {
                (hooks.on_import)(
                    ImportChoice::Import,
                    Rc::new(move |outcome, _| heard.borrow_mut().push(outcome)),
                    cx,
                )
            });
        };
        start(cx);
        let window = importer_window(cx).unwrap();
        let mut review = VisualTestContext::from_window(window.into(), cx);
        press(&mut review, crate::importer_view::NEXT_ID);
        assert!(importer_window(cx).is_none());
        assert!(matches!(
            replies.borrow().as_slice(),
            [ImportOutcome::Failed {
                imported: Some(_),
                ..
            }]
        ));
        let written = fs::read(&rules).unwrap();
        fs::create_dir(dir.path().join("rules.json.saving")).unwrap();
        fs::remove_dir(dir.path().join("state.json.saving")).unwrap();
        start(cx);
        assert!(importer_window(cx).is_none());
        assert!(matches!(
            replies.borrow().as_slice(),
            [_, ImportOutcome::Imported(_)]
        ));
        assert_eq!(fs::read(&rules).unwrap(), written);
        assert_eq!(
            State::load(dir.path()).unwrap().import,
            Some(ImportChoice::Import)
        );
    }

    #[gpui_kit::test]
    fn review_cached_import_cannot_complete_after_a_competing_source_choice(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| crate::assets::load_fonts(cx));
        let dir = tempfile::tempdir().unwrap();
        let rules = dir.path().join("rules.json");
        let mut settings =
            crate::settings_store::SettingsStore::load(Some(dir.path().to_path_buf()));
        settings
            .update(|settings| settings.source.config = Some(importer_config()))
            .unwrap();
        cx.update(|cx| crate::settings_store::install(settings, None, cx));
        fs::create_dir(dir.path().join("state.json.saving")).unwrap();
        let hooks = hooks_with(
            Some(dir.path().to_path_buf()),
            Some(rules.clone()),
            Some(importer_config()),
            Rc::new(crate::importer_view::set_source_and_show),
            Rc::new(|_, _| {}),
        );
        let replies = Rc::new(RefCell::new(Vec::new()));
        let choose = |choice, cx: &mut TestAppContext| {
            let heard = replies.clone();
            cx.update(|cx| {
                (hooks.on_import)(
                    choice,
                    Rc::new(move |outcome, _| heard.borrow_mut().push(outcome)),
                    cx,
                )
            });
        };
        choose(ImportChoice::Import, cx);
        let window = importer_window(cx).unwrap();
        window
            .update(cx, |view, _, _| {
                view.model.source = crate::importer::SourceChoice::VisualOnly
            })
            .unwrap();
        let mut importer = VisualTestContext::from_window(window.into(), cx);
        press(&mut importer, crate::importer_view::NEXT_ID);
        assert!(
            crate::settings_store::Settings::load(dir.path())
                .unwrap()
                .source
                .visual_only
        );
        let written = fs::read(&rules).unwrap();
        choose(ImportChoice::KeepFile, cx);
        assert!(
            !crate::settings_store::Settings::load(dir.path())
                .unwrap()
                .source
                .visual_only
        );
        fs::remove_dir(dir.path().join("state.json.saving")).unwrap();
        choose(ImportChoice::Import, cx);
        assert!(
            matches!(replies.borrow().last(), Some(ImportOutcome::Failed { .. })),
            "cached visual-only completion must be rejected after Keep changed the source: {:?}",
            replies.borrow()
        );
        assert_eq!(State::load(dir.path()).unwrap().import, None);
        assert_eq!(fs::read(&rules).unwrap(), written);
    }

    #[gpui_kit::test]
    fn review_cached_keep_detects_a_competing_equal_source(cx: &mut TestAppContext) {
        cx.update(|cx| crate::assets::load_fonts(cx));
        let dir = tempfile::tempdir().unwrap();
        let mut settings =
            crate::settings_store::SettingsStore::load(Some(dir.path().to_path_buf()));
        settings
            .update(|settings| settings.source.visual_only = true)
            .unwrap();
        let source = settings.settings().source.clone();
        cx.update(|cx| crate::settings_store::install(settings, None, cx));
        fs::create_dir(dir.path().join("state.json.saving")).unwrap();
        let hooks = hooks_with(
            Some(dir.path().to_path_buf()),
            Some(dir.path().join("rules.json")),
            Some(importer_config()),
            Rc::new(crate::importer_view::set_source_and_show),
            Rc::new(|_, _| {}),
        );
        let replies = Rc::new(RefCell::new(Vec::new()));
        let choose = |choice, cx: &mut TestAppContext| {
            let heard = replies.clone();
            cx.update(|cx| {
                (hooks.on_import)(
                    choice,
                    Rc::new(move |outcome, _| heard.borrow_mut().push(outcome)),
                    cx,
                )
            });
        };
        choose(ImportChoice::Import, cx);
        let window = importer_window(cx).unwrap();
        let mut importer = VisualTestContext::from_window(window.into(), cx);
        press(&mut importer, crate::importer_view::NEXT_ID);
        choose(ImportChoice::DontUse, cx);
        // Selecting Don't use replaces any CLI override even though the
        // source settings bytes equal the earlier Keep import's settings.
        assert_eq!(
            crate::settings_store::Settings::load(dir.path())
                .unwrap()
                .source,
            source
        );
        fs::remove_dir(dir.path().join("state.json.saving")).unwrap();
        choose(ImportChoice::Import, cx);
        assert!(matches!(
            replies.borrow().last(),
            Some(ImportOutcome::Failed { .. })
        ));
        assert_eq!(State::load(dir.path()).unwrap().import, None);
    }

    #[test]
    fn edit_rules_reports_sync_contention_without_running_the_edit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rules.json");
        let held = pwrroute_sync::LocalFile::lock(&path).unwrap();
        let error = edit_rules(Some(&path), |_| panic!("lock refused before editing")).unwrap_err();
        assert!(error.contains("try saving again"));
        assert!(!path.exists());
        drop(held);
        edit_rules(Some(&path), |file| file.default_browser = "Firefox".into()).unwrap();
        assert_eq!(RulesFile::load(&path).unwrap().default_browser, "Firefox");
    }

    fn fixture_home() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/home")
    }

    fn importer_config() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/importer/finicky.ts")
    }

    #[test]
    fn the_state_file_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().join("PwrRoute");
        assert_eq!(State::load(&dir).unwrap(), State::default());
        assert!(State::pending(&dir));

        let state = State {
            first_run_completed: true,
            fallback: Some("com.google.Chrome".to_owned()),
            fallback_profile: Some("Work".to_owned()),
            import: Some(ImportChoice::KeepFile),
        };
        state.save(&dir).unwrap();
        assert_eq!(State::load(&dir).unwrap(), state);
        assert!(!State::pending(&dir));
        let text = fs::read_to_string(State::path(&dir)).unwrap();
        assert!(text.contains("\"firstRunCompleted\": true"));
        assert!(text.contains("\"import\": \"keepFile\""));
        assert!(text.contains("\"fallbackProfile\": \"Work\""));

        // Don't use it, and older builds' Start empty, which reads as it.
        State {
            import: Some(ImportChoice::DontUse),
            ..State::default()
        }
        .save(&dir)
        .unwrap();
        let text = fs::read_to_string(State::path(&dir)).unwrap();
        assert!(text.contains("\"import\": \"dontUse\""));
        fs::write(State::path(&dir), r#"{"import":"startEmpty"}"#).unwrap();
        assert_eq!(
            State::load(&dir).unwrap().import,
            Some(ImportChoice::DontUse)
        );

        // A bad file does not nag.
        fs::write(State::path(&dir), "{").unwrap();
        assert!(State::load(&dir).is_err());
        assert!(!State::pending(&dir));
    }

    #[test]
    fn the_model_comes_from_the_facts() {
        let facts = facts();
        let built = model(&facts, Some(Path::new("/Users/u")));
        assert_eq!(built.fallback, None);
        assert_eq!(built.browsers.len(), 3);
        assert_eq!(built.current_default.as_deref(), Some("Google Chrome"));
        assert!(!built.is_default);
        let finicky = built.finicky.as_ref().unwrap();
        assert_eq!(finicky.path, "~/.finicky.ts");
        assert_eq!((finicky.rewrites, finicky.handlers), (2, 5));
        assert_eq!(finicky.importable, 3);
        // The config's default, by the name the machine lists it under.
        assert_eq!(finicky.default_name.as_deref(), Some("Safari"));
        assert_eq!(
            built
                .steps()
                .into_iter()
                .map(|(_, state)| state)
                .collect::<Vec<_>>(),
            [StepState::Current, StepState::Todo, StepState::Todo]
        );
        assert!(built.config_detail().contains("copy the 3 rules"));
        // Nothing chosen yet: the window picks the config's own default.
        let (fallback, picked) = built.preselection().unwrap();
        assert_eq!(fallback.browser, safari());
        assert_eq!(picked, Picked::Config);
    }

    #[test]
    fn a_browser_whose_profiles_were_kept_from_pwrroute_gets_the_access_step() {
        let mut facts = facts();
        assert_eq!(model(&facts, None).access, None);
        facts.browsers[1].profiles.clear();
        facts.browsers[1].profiles_blocked = true;
        let built = model(&facts, None);
        let access = built.access.as_ref().expect("an access step");
        assert_eq!(
            access
                .browsers
                .iter()
                .map(|label| label.name.as_str())
                .collect::<Vec<_>>(),
            ["Firefox"]
        );
        assert_eq!(built.steps()[0], (Step::Access, StepState::Current));
        // Where the OS has no pane to send the reader to, no step.
        facts.access_pane = false;
        assert_eq!(model(&facts, None).access, None);
    }

    #[test]
    fn a_fallback_already_in_rules_json_is_everything_else_done() {
        let mut facts = facts();
        facts.rules_default = "google chrome".to_owned();
        facts.state.import = Some(ImportChoice::KeepFile);
        let built = model(&facts, None);
        assert_eq!(built.fallback, Some(chrome().label));
        assert_eq!(built.state_of(Step::EverythingElse), StepState::Done);

        // state.json's choice wins, and an app not found here is shown as
        // written.
        facts.state.fallback = Some("com.example.Browser".to_owned());
        let built = model(&facts, None);
        assert_eq!(
            built.fallback,
            Some(BrowserLabel::new(
                badge_for("com.example.Browser"),
                "com.example.Browser",
                None
            ))
        );
    }

    /// Done and Later both end first run in `state.json` and open the main
    /// window; the file's other choices stay.
    #[gpui_kit::test]
    fn done_and_later_end_first_run_and_open_the_main_window(cx: &mut TestAppContext) {
        use crate::first_run::Finish;
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("PwrRoute");
        State {
            fallback: Some("com.google.Chrome".to_owned()),
            ..State::default()
        }
        .save(&data_dir)
        .unwrap();
        let opened = Rc::new(std::cell::Cell::new(0));
        let count = opened.clone();
        let hooks = hooks_with(
            Some(data_dir.clone()),
            None,
            None,
            no_source(),
            Rc::new(move |_, _| count.set(count.get() + 1)),
        );
        assert!(State::pending(&data_dir));

        let finished = |how| first_run::Finished { how, display: None };
        cx.update(|cx| (hooks.on_finish)(finished(Finish::Later), cx));
        assert_eq!(opened.get(), 1);
        assert!(!State::pending(&data_dir));
        let state = State::load(&data_dir).unwrap();
        assert!(state.first_run_completed);
        assert_eq!(state.fallback.as_deref(), Some("com.google.Chrome"));

        cx.update(|cx| (hooks.on_finish)(finished(Finish::Done), cx));
        assert_eq!(opened.get(), 2);
        assert!(!State::pending(&data_dir));
    }

    #[test]
    fn the_os_answer_and_the_choices_carry_over() {
        let mut facts = facts();
        facts.handler = DefaultState::Unknown;
        facts.state.import = Some(ImportChoice::KeepFile);
        facts.precedence = Precedence::PwrRoute;
        facts.rules = 4;
        let built = model(&facts, None);
        assert_eq!(built.current_default, None);
        assert_eq!(built.import, Some(ImportChoice::KeepFile));
        assert_eq!(built.precedence, Precedence::PwrRoute);
        assert_eq!(built.rules, 4);

        // Don't use it (or an older Start empty) with the config still in
        // charge never took effect, and an import's count is not kept: the
        // step asks again.
        for earlier in [ImportChoice::DontUse, ImportChoice::Import] {
            facts.state.import = Some(earlier);
            assert_eq!(model(&facts, None).import, None);
        }

        facts.handler = DefaultState::PwrRoute;
        facts.finicky = None;
        facts.state.import = Some(ImportChoice::KeepFile);
        let built = model(&facts, None);
        assert!(built.is_default);
        assert_eq!(built.finicky, None);
        assert_eq!(built.import, None);
        assert_eq!(built.current_default, None);
        assert_eq!(tilde(Path::new("/x/y"), Some(Path::new("/z"))), "/x/y");
    }

    /// The Everything else browser and profile go to rules.json's
    /// `defaultBrowser` and `defaultProfile` (the router's fallback) and to
    /// state.json, and the next first run reads them back from either.
    #[gpui_kit::test]
    fn everything_else_saves_the_browser_and_profile(cx: &mut TestAppContext) {
        use crate::first_run::Fallback;
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("PwrRoute");
        let rules_path = data_dir.join("rules.json");
        let mut settings = crate::settings_store::SettingsStore::load(Some(data_dir.clone()));
        settings
            .update(|settings| settings.source.config = Some(importer_config()))
            .unwrap();
        cx.update(|cx| crate::settings_store::install(settings, None, cx));
        let hooks = hooks_with(
            Some(data_dir.clone()),
            Some(rules_path.clone()),
            None,
            no_source(),
            Rc::new(|_, _| {}),
        );
        let saved = || {
            let rules = RulesFile::load(&rules_path).unwrap();
            let state = State::load(&data_dir).unwrap();
            (rules, state)
        };

        cx.update(|cx| {
            (hooks.on_fallback)(
                Fallback {
                    browser: chrome(),
                    profile: Some("Work".to_owned()),
                },
                cx,
            )
        })
        .unwrap();
        let (rules, state) = saved();
        assert_eq!(rules.default_browser, "com.google.Chrome");
        assert_eq!(rules.default_profile, "Work");
        assert_eq!(state.fallback.as_deref(), Some("com.google.Chrome"));
        assert_eq!(state.fallback_profile.as_deref(), Some("Work"));

        // Read back: from state.json, and from rules.json alone.
        let mut read_back = facts();
        read_back.state = state;
        let expected = Some(BrowserLabel::new("Ch", "Google Chrome", Some("Work")));
        assert_eq!(model(&read_back, None).fallback, expected);
        assert_eq!(
            model(&read_back, None).fallback_profiles(),
            ["Personal", "Work"]
        );
        read_back.state = State::default();
        read_back.rules_default = rules.default_browser;
        read_back.rules_profile = rules.default_profile;
        assert_eq!(model(&read_back, None).fallback, expected);

        // A browser without profiles clears the profile in both files.
        cx.update(|cx| {
            (hooks.on_fallback)(
                Fallback {
                    browser: safari(),
                    profile: None,
                },
                cx,
            )
        })
        .unwrap();
        let (rules, state) = saved();
        assert_eq!(rules.default_browser, "com.apple.Safari");
        assert_eq!(rules.default_profile, "");
        assert_eq!(state.fallback_profile, None);
        let mut after = facts();
        after.state = state;
        let built = model(&after, None);
        assert_eq!(built.fallback, Some(safari().label));
        assert!(built.fallback_profiles().is_empty());
    }

    /// The segmented control writes rules.json's `precedence`, and leaves
    /// the rest of the file as it was; the Go order is the field left out.
    #[gpui_kit::test]
    fn the_owner_of_everything_else_is_rules_json_precedence(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let rules_path = dir.path().join("rules.json");
        RulesFile {
            default_browser: "com.apple.Safari".to_owned(),
            ..RulesFile::default()
        }
        .save(&rules_path)
        .unwrap();
        let hooks = hooks_with(
            None,
            Some(rules_path.clone()),
            None,
            no_source(),
            Rc::new(|_, _| {}),
        );

        cx.update(|cx| (hooks.on_precedence)(Precedence::PwrRoute, cx))
            .unwrap();
        let rules = RulesFile::load(&rules_path).unwrap();
        assert_eq!(rules.precedence, Precedence::PwrRoute);
        assert_eq!(rules.default_browser, "com.apple.Safari");
        assert!(
            fs::read_to_string(&rules_path)
                .unwrap()
                .contains("\"precedence\": \"pwrroute\"")
        );

        cx.update(|cx| (hooks.on_precedence)(Precedence::Config, cx))
            .unwrap();
        assert_eq!(
            RulesFile::load(&rules_path).unwrap().precedence,
            Precedence::Config
        );
        assert!(
            !fs::read_to_string(&rules_path)
                .unwrap()
                .contains("precedence")
        );
    }

    /// A rules.json that does not load (left by an older install, or a
    /// sync) is never overwritten: both choices say why instead, and
    /// state.json does not record a fallback that was not written.
    #[gpui_kit::test]
    fn edit_rules_reports_a_rules_json_that_does_not_load(cx: &mut TestAppContext) {
        use crate::first_run::Fallback;
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("PwrRoute");
        let rules_path = dir.path().join("rules.json");
        fs::write(&rules_path, "{ not json").unwrap();
        let hooks = hooks_with(
            Some(data_dir.clone()),
            Some(rules_path.clone()),
            None,
            no_source(),
            Rc::new(|_, _| {}),
        );
        let fallback = cx.update(|cx| {
            (hooks.on_fallback)(
                Fallback {
                    browser: safari(),
                    profile: None,
                },
                cx,
            )
        });
        let precedence = cx.update(|cx| (hooks.on_precedence)(Precedence::PwrRoute, cx));
        for result in [fallback, precedence] {
            let reason = result.expect_err("not saved");
            assert!(reason.contains("did not load"), "{reason}");
        }
        assert_eq!(fs::read_to_string(&rules_path).unwrap(), "{ not json");
        assert_eq!(State::load(&data_dir).unwrap_or_default().fallback, None);
    }

    /// Don't use it with a config in charge: the source it saves is visual
    /// rules only, handed to `set_source` (`importer_view::set_source`,
    /// which writes `settings.json` and calls `router::choose_source`), and
    /// state.json records the answer.
    #[gpui_kit::test]
    fn dont_use_it_turns_the_config_off(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("PwrRoute");
        let config = fixture_home().join(".finicky.ts");
        // The settings as they stand: the config kept by path, a dismissal.
        let mut settings = crate::settings_store::SettingsStore::load(Some(data_dir.clone()));
        settings
            .update(|settings| {
                settings.source.config = Some(config.clone());
                settings.source.dismissed = vec![PathBuf::from("/x/.finicky.js")];
            })
            .unwrap();
        cx.update(|cx| crate::settings_store::install(settings, None, cx));

        let sources = Rc::new(RefCell::new(Vec::new()));
        let saved = sources.clone();
        let hooks = hooks_with(
            Some(data_dir.clone()),
            None,
            Some(config),
            Rc::new(move |source, _| {
                saved.borrow_mut().push(source);
                Ok(())
            }),
            Rc::new(|_, _| {}),
        );
        let replies = Rc::new(RefCell::new(Vec::new()));
        let heard = replies.clone();
        cx.update(|cx| {
            (hooks.on_import)(
                ImportChoice::DontUse,
                Rc::new(move |outcome, _| heard.borrow_mut().push(outcome)),
                cx,
            );
        });
        assert_eq!(
            *sources.borrow(),
            [crate::settings_store::SourceSettings {
                visual_only: true,
                config: None,
                dismissed: vec![PathBuf::from("/x/.finicky.js")],
            }]
        );
        assert_eq!(
            State::load(&data_dir).unwrap().import,
            Some(ImportChoice::DontUse)
        );
        assert_eq!(
            *replies.borrow(),
            [ImportOutcome::Chosen(ImportChoice::DontUse)]
        );

        // Keep explicitly restores the config, even after a session-only choice.
        cx.update(|cx| (hooks.on_import)(ImportChoice::KeepFile, Rc::new(|_, _| {}), cx));
        assert_eq!(sources.borrow().len(), 2);
    }

    /// The fixture config: its counts, its fixed default, what imports.
    #[test]
    fn a_config_is_read_for_its_counts_and_default() {
        let found = FinickyFound::read(importer_config());
        assert_eq!(found.error, None);
        assert!(found.handlers > 0);
        assert!(found.importable > 0);
        assert_eq!(found.default_browser.as_deref(), Some("Google Chrome"));
    }

    /// A config that does not load is still found, with why, as one line
    /// that names it by its short path.
    #[test]
    fn a_config_that_does_not_load_is_found_with_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".finicky.ts");
        fs::write(
            &path,
            "export default {\n  defaultBrowser: \"Safari\",\n  handlers: [\n}\n",
        )
        .unwrap();
        let found = FinickyFound::read(path.clone());
        assert_eq!(
            (found.rewrites, found.handlers, found.importable),
            (0, 0, 0)
        );
        let error = found.error.clone().expect("the error");
        let mut facts = facts();
        facts.finicky = Some(found);
        let built = model(&facts, Some(dir.path()));
        let line = built.finicky.unwrap().error.unwrap();
        assert!(!line.contains('\n'), "{line}");
        assert!(
            !line.contains(&dir.path().display().to_string()),
            "{line} from {error}"
        );
        assert!(
            line.ends_with(" \u{00b7} ~/.finicky.ts") && !line.starts_with('/'),
            "{line} from {error}"
        );

        assert_eq!(
            load_error(
                "/h/.finicky.ts: SyntaxError: unexpected token\n    at line 4",
                Path::new("/h/.finicky.ts"),
                "~/.finicky.ts"
            ),
            "SyntaxError: unexpected token \u{00b7} ~/.finicky.ts"
        );
    }

    fn importer_window(
        cx: &mut TestAppContext,
    ) -> Option<WindowHandle<crate::importer_view::Importer>> {
        cx.run_until_parked();
        cx.windows()
            .into_iter()
            .find_map(|window| window.downcast::<crate::importer_view::Importer>())
    }

    fn press(cx: &mut VisualTestContext, id: &'static str) {
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        let bounds = cx
            .debug_bounds(id)
            .unwrap_or_else(|| panic!("{id} is drawn"));
        cx.simulate_click(bounds.center(), Modifiers::none());
    }

    #[gpui_kit::test]
    fn first_run_copies_json_and_reports_saved_tests_without_opening_main(cx: &mut TestAppContext) {
        cx.update(|cx| crate::assets::load_fonts(cx));
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("Finicky/rules.json");
        let target = dir.path().join("PwrRoute/rules.json");
        let file: RulesFile = serde_json::from_str(r#"{
            "defaultBrowser":"Firefox", "rules":[{"match":"example.com/*", "browser":"Safari", "examples":["https://example.com/a"]}],
            "tests":[{"url":"https://example.com/a", "expect":{"browser":"Safari"}}]
        }"#).unwrap();
        file.save(&source).unwrap();
        let original = fs::read(&source).unwrap();
        let hooks = hooks_with(
            Some(dir.path().join("PwrRoute")),
            Some(target.clone()),
            None,
            no_source(),
            Rc::new(|_, _| panic!("copy opens no main window")),
        );
        let replies = Rc::new(RefCell::new(Vec::new()));
        let start = |cx: &mut TestAppContext| {
            let heard = replies.clone();
            cx.update(|cx| {
                (hooks.on_import_rules)(
                    source.clone(),
                    Rc::new(move |result, _| heard.borrow_mut().push(result)),
                    cx,
                )
            });
            importer_window(cx).expect("JSON copy review opened")
        };
        let window = start(cx);
        let mut review = VisualTestContext::from_window(window.into(), cx);
        press(&mut review, crate::importer_view::CANCEL_ID);
        assert_eq!(*replies.borrow(), [None]);
        assert!(!target.exists());
        let window = start(cx);
        let mut review = VisualTestContext::from_window(window.into(), cx);
        press(&mut review, crate::importer_view::NEXT_ID);
        assert!(importer_window(cx).is_none());
        assert_eq!(RulesFile::load(&target).unwrap(), file);
        assert_eq!(fs::read(source).unwrap(), original);
        let replies = replies.borrow();
        let Some(result) = &replies[1] else {
            panic!("import completed")
        };
        let imported = result.current.as_ref().unwrap();
        let summary = &result.summary;
        assert_eq!(imported.rules, 1);
        assert!(summary.contains("2 of 2 passed"), "{summary}");
    }

    /// Import N rules from first run: the importer opens on its own, a
    /// Cancel answers Cancelled, and an Import writes rules.json (no main
    /// window is opened for it) and answers with the count and the file as
    /// it now stands.
    #[gpui_kit::test]
    fn an_import_from_first_run_answers_how_the_importer_was_left(cx: &mut TestAppContext) {
        cx.update(|cx| crate::assets::load_fonts(cx));
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("PwrRoute");
        let rules_path = data_dir.join("rules.json");
        let mut settings = crate::settings_store::SettingsStore::load(Some(data_dir.clone()));
        settings
            .update(|settings| settings.source.config = Some(importer_config()))
            .unwrap();
        cx.update(|cx| crate::settings_store::install(settings, None, cx));
        let hooks = hooks_with(
            Some(data_dir.clone()),
            Some(rules_path.clone()),
            Some(importer_config()),
            no_source(),
            Rc::new(|_, _| panic!("first run's import opens no main window")),
        );
        let replies = Rc::new(RefCell::new(Vec::new()));
        let start = |cx: &mut TestAppContext| {
            let heard = replies.clone();
            cx.update(|cx| {
                (hooks.on_import)(
                    ImportChoice::Import,
                    Rc::new(move |outcome, _| heard.borrow_mut().push(outcome)),
                    cx,
                );
            });
            importer_window(cx).expect("the importer opened")
        };

        // It opens on Choose source (two rules stay JS). Back to Review,
        // then Cancel: the step comes back, nothing is written.
        let window = start(cx);
        let mut importer = VisualTestContext::from_window(window.into(), cx);
        assert_eq!(
            window
                .read_with(&importer, |importer, _| importer.model.step)
                .unwrap(),
            crate::importer::Step::Source
        );
        press(&mut importer, crate::importer_view::BACK_ID);
        press(&mut importer, crate::importer_view::CANCEL_ID);
        assert!(importer_window(cx).is_none());
        assert_eq!(*replies.borrow(), [ImportOutcome::Cancelled]);
        assert!(!rules_path.exists());
        assert_eq!(State::load(&data_dir).unwrap().import, None);

        // Import, keeping the config in charge for what stays JS.
        let window = start(cx);
        let mut importer = VisualTestContext::from_window(window.into(), cx);
        press(&mut importer, crate::importer_view::NEXT_ID);
        assert!(importer_window(cx).is_none(), "Import closes the importer");
        assert!(
            cx.windows()
                .into_iter()
                .all(|window| window.downcast::<crate::app::PwrRoute>().is_none())
        );
        let written = RulesFile::load(&rules_path).unwrap();
        let replies = replies.borrow();
        let [_, ImportOutcome::Imported(imported)] = replies.as_slice() else {
            panic!("{replies:?}");
        };
        assert!(imported.count > 0);
        assert!(imported.keeps_config);
        assert_eq!(imported.rules, written.rules.len());
        assert_eq!(imported.default_browser, written.default_browser);
        assert!(!written.rules.is_empty());
        assert_eq!(
            State::load(&data_dir).unwrap().import,
            Some(ImportChoice::Import)
        );
    }

    /// First run's importer over a `rules.json` the store would refuse to
    /// write (it does not load): Import writes nothing and opens the
    /// importer again with the reason, first run still waiting; Cancel then
    /// answers it.
    #[gpui_kit::test]
    fn an_import_the_store_refuses_reopens_and_still_answers_first_run(cx: &mut TestAppContext) {
        cx.update(|cx| crate::assets::load_fonts(cx));
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("PwrRoute");
        let rules_path = data_dir.join("rules.json");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(&rules_path, "{ not json").unwrap();
        let hooks = hooks_with(
            Some(data_dir.clone()),
            Some(rules_path.clone()),
            Some(importer_config()),
            no_source(),
            Rc::new(|_, _| panic!("first run's import opens no main window")),
        );
        let replies = Rc::new(RefCell::new(Vec::new()));
        let heard = replies.clone();
        cx.update(|cx| {
            (hooks.on_import)(
                ImportChoice::Import,
                Rc::new(move |outcome, _| heard.borrow_mut().push(outcome)),
                cx,
            );
        });
        let window = importer_window(cx).expect("the importer opened");
        let mut importer = VisualTestContext::from_window(window.into(), cx);
        press(&mut importer, crate::importer_view::NEXT_ID);

        let window = importer_window(cx).expect("the importer is back");
        let refused = window
            .read_with(cx, |importer, _| importer.model.refused.clone())
            .unwrap();
        assert!(refused.is_some_and(|reason| reason.contains("did not load")));
        assert!(replies.borrow().is_empty(), "first run is still waiting");
        assert_eq!(fs::read_to_string(&rules_path).unwrap(), "{ not json");

        // It is back on Choose source: Back to Review, then Cancel.
        let mut importer = VisualTestContext::from_window(window.into(), cx);
        press(&mut importer, crate::importer_view::BACK_ID);
        press(&mut importer, crate::importer_view::CANCEL_ID);
        cx.run_until_parked();
        assert_eq!(*replies.borrow(), [ImportOutcome::Cancelled]);
    }

    /// A first-run importer closed some other way than Cancel (its close
    /// button, the app quitting) answers Cancelled, so first run never
    /// waits on Importing… for good.
    #[gpui_kit::test]
    fn closing_the_importer_answers_cancelled(cx: &mut TestAppContext) {
        cx.update(|cx| crate::assets::load_fonts(cx));
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("PwrRoute");
        let hooks = hooks_with(
            Some(data_dir.clone()),
            Some(data_dir.join("rules.json")),
            Some(importer_config()),
            no_source(),
            Rc::new(|_, _| panic!("no main window")),
        );
        let replies = Rc::new(RefCell::new(Vec::new()));
        let heard = replies.clone();
        cx.update(|cx| {
            (hooks.on_import)(
                ImportChoice::Import,
                Rc::new(move |outcome, _| heard.borrow_mut().push(outcome)),
                cx,
            );
        });
        let window = importer_window(cx).expect("the importer opened");
        window
            .update(cx, |_, window, _| window.remove_window())
            .unwrap();
        cx.run_until_parked();
        assert!(importer_window(cx).is_none());
        assert_eq!(*replies.borrow(), [ImportOutcome::Cancelled]);
    }

    /// Import with no config to import: cancelled, answered on the next
    /// turn, since the window that asked is still mid-update.
    #[gpui_kit::test]
    fn an_import_without_a_config_is_cancelled_on_the_next_turn(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let hooks = hooks_with(
            Some(dir.path().join("PwrRoute")),
            None,
            None,
            no_source(),
            Rc::new(|_, _| panic!("no main window")),
        );
        let replies = Rc::new(RefCell::new(Vec::new()));
        let heard = replies.clone();
        cx.update(|cx| {
            (hooks.on_import)(
                ImportChoice::Import,
                Rc::new(move |outcome, _| heard.borrow_mut().push(outcome)),
                cx,
            );
            assert!(replies.borrow().is_empty(), "not answered mid-update");
        });
        assert_eq!(*replies.borrow(), [ImportOutcome::Cancelled]);
    }

    /// Apps hidden in Settings › Browsers stay out of the Everything else
    /// select, so the preselection never lands on one.
    #[test]
    fn hidden_browsers_are_not_offered() {
        let app = |name: &str, id: &str| crate::browser_scan::DetectedApp {
            name: name.to_owned(),
            bundle_id: Some(id.to_owned()),
            location: PathBuf::from(format!("/Applications/{name}.app")),
            profiles: Vec::new(),
            directories: Vec::new(),
            profiles_blocked: false,
        };
        let apps = vec![
            app("Firefox", "org.mozilla.firefox"),
            app("Safari", "com.apple.Safari"),
        ];
        let hidden = crate::settings_store::BrowserSettings {
            hidden: vec!["ORG.mozilla.firefox".to_owned()],
        };
        let offered = choices(apps.clone(), &hidden);
        assert_eq!(
            offered
                .iter()
                .map(|choice| choice.label.name.as_str())
                .collect::<Vec<_>>(),
            ["Safari"]
        );
        assert_eq!(choices(apps, &Default::default()).len(), 2);
    }

    /// After first run the main window opens where the first-run window
    /// was; with no first-run display, under the mouse; a display since
    /// unplugged is skipped; with nothing known, the primary (`None`), as a
    /// cold launch.
    #[test]
    fn the_main_window_opens_on_the_first_run_display() {
        let (laptop, studio, gone) = (DisplayId::new(1), DisplayId::new(2), DisplayId::new(3));
        let connected = [laptop, studio];
        assert_eq!(
            main_window_display(Some(studio), Some(laptop), &connected),
            Some(studio)
        );
        assert_eq!(
            main_window_display(None, Some(studio), &connected),
            Some(studio)
        );
        assert_eq!(
            main_window_display(Some(gone), Some(laptop), &connected),
            Some(laptop)
        );
        assert_eq!(main_window_display(Some(gone), None, &connected), None);
        assert_eq!(main_window_display(None, None, &connected), None);
    }
    #[gpui_kit::test]
    fn initial_scan_shows_preparing_and_close_cancels_without_opening_or_saving_setup(
        cx: &mut TestAppContext,
    ) {
        let (release, wait) = async_channel::bounded(1);
        cx.update(|cx| {
            crate::browser_scan::install(
                pwrroute_browsers::Env::fixture(
                    pwrroute_browsers::Platform::current(),
                    std::env::temp_dir(),
                ),
                cx,
            );
            crate::browser_scan::set_test_producer(
                cx,
                std::sync::Arc::new(move || {
                    let wait = wait.clone();
                    Box::pin(async move { wait.recv().await.unwrap() })
                }),
            );
            install(cx);
            let _ = crate::browser_scan::apps(cx);
            let mut facts = facts();
            facts.finicky = None;
            facts.browsers.clear();
            prepare(facts, None, cx);
            assert!(cx.has_global::<Opening>());
            assert!(!cx.has_global::<Open>());
            assert!(crate::settings_store::store(cx).is_none());
        });
        cx.run_until_parked();
        let window = cx.update(|cx| cx.global::<Opening>().window);
        let _ = window.update(cx, |view, window, _| {
            assert!(view.failure.is_none());
            window.remove_window();
        });
        cx.run_until_parked();
        release.try_send(Ok(vec![])).unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(!cx.has_global::<Opening>());
            assert!(!cx.has_global::<Open>());
            assert!(cx.windows().is_empty());
            assert!(crate::settings_store::store(cx).is_none());
        });
    }
    #[test]
    fn engine_review_first_run_config_tracks_imported_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.js");
        let dependency = dir.path().join("policy.js");
        fs::write(
            &config,
            "import browser from './policy.js'; export default { defaultBrowser: browser };",
        )
        .unwrap();
        fs::write(&dependency, "export default 'Safari';").unwrap();
        fs::File::options()
            .write(true)
            .open(&dependency)
            .unwrap()
            .set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(10))
            .unwrap();
        let (found, stamps) = FinickyFound::read_with_stamps(config);
        assert_eq!(found.default_browser.as_deref(), Some("Safari"));
        assert!(crate::engine_worker::unchanged(&stamps));
        fs::write(&dependency, "export default 'Firefox';").unwrap();
        fs::File::options()
            .write(true)
            .open(&dependency)
            .unwrap()
            .set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(20))
            .unwrap();
        assert!(
            !crate::engine_worker::unchanged(&stamps),
            "queued first-run review ignored its imported module"
        );
    }
    #[gpui_kit::test]
    fn engine_review_import_changed_while_waiting_for_scan_cannot_open_old_setup(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.js");
        let dependency = dir.path().join("policy.js");
        fs::write(
            &config,
            "import browser from './policy.js'; export default { defaultBrowser: browser };",
        )
        .unwrap();
        fs::write(&dependency, "export default 'Safari';").unwrap();
        fs::File::options()
            .write(true)
            .open(&dependency)
            .unwrap()
            .set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(10))
            .unwrap();
        let (found, stamps) = FinickyFound::read_with_stamps(config);
        let (release, wait) = async_channel::bounded(1);
        let original = cx.update(|cx| {
            crate::browser_scan::install(
                pwrroute_browsers::Env::fixture(
                    pwrroute_browsers::Platform::current(),
                    dir.path().to_path_buf(),
                ),
                cx,
            );
            crate::browser_scan::set_test_producer(
                cx,
                std::sync::Arc::new(move || {
                    let wait = wait.clone();
                    Box::pin(async move { wait.recv().await.unwrap() })
                }),
            );
            install(cx);
            let _ = crate::browser_scan::apps(cx);
            let mut facts = facts();
            facts.finicky = Some(found);
            facts.browsers.clear();
            prepare(facts, None, cx);
            let opening = cx.global_mut::<Opening>();
            // The completed worker review is retained while browser discovery
            // is still blocked. Its source identity is already accepted.
            opening.config = crate::router::config_in_charge();
            opening.stamps = stamps;
            let token = opening.request.clone();
            finish_opening(cx);
            assert!(Rc::ptr_eq(&cx.global::<Opening>().request, &token));
            assert!(!cx.has_global::<Open>());
            token
        });
        fs::write(&dependency, "export default 'Firefox';").unwrap();
        fs::File::options()
            .write(true)
            .open(&dependency)
            .unwrap()
            .set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(20))
            .unwrap();
        cx.update(|cx| {
            finish_opening(cx);
            assert!(!cx.has_global::<Open>(), "a stale review opened FirstRun");
            assert!(
                !cx.try_global::<Opening>()
                    .is_some_and(|opening| Rc::ptr_eq(&opening.request, &original)),
                "the old review survived its dependency change"
            );
            if let Some(window) = cx.try_global::<Opening>().map(|opening| opening.window) {
                let _ = window.update(cx, |_, window, _| window.remove_window());
            }
        });
        cx.run_until_parked();
        release.try_send(Ok(vec![])).unwrap();
        cx.run_until_parked();
        cx.update(|cx| assert!(!cx.has_global::<Open>()));
        assert!(!dir.path().join("rules.json").exists());
        assert!(!dir.path().join(STATE_FILE).exists());
    }
    #[test]
    fn review_regression_setup_refreshes_rules_and_state_before_preselection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rules.json");
        let mut snapshot = facts();
        snapshot.finicky = None;
        snapshot.rules_path = Some(path.clone());
        snapshot.state_dir = Some(dir.path().to_owned());
        RulesFile {
            default_browser: "org.mozilla.firefox".into(),
            default_profile: "Saved profile".into(),
            ..Default::default()
        }
        .save(&path)
        .unwrap();
        State::default().save(dir.path()).unwrap();
        snapshot.refresh_saved();
        let current = model(&snapshot, None);
        assert_eq!(current.fallback.as_ref().unwrap().name.as_str(), "Firefox");
        assert_eq!(snapshot.rules_profile, "Saved profile");
        State {
            fallback: Some("com.google.Chrome".into()),
            fallback_profile: Some("Saved state".into()),
            ..Default::default()
        }
        .save(dir.path())
        .unwrap();
        snapshot.refresh_saved();
        let current = model(&snapshot, None);
        assert_eq!(
            current.fallback.as_ref().unwrap().name.as_str(),
            "Google Chrome"
        );
        assert_eq!(
            snapshot.state.fallback_profile.as_deref(),
            Some("Saved state")
        );
        assert_eq!(
            RulesFile::load(&path).unwrap().default_browser,
            "org.mozilla.firefox"
        );
    }

    #[gpui_kit::test]
    fn review_regression_preparing_is_a_dock_window(cx: &mut TestAppContext) {
        cx.add_window_view(|_, _| Preparing { failure: None });
        cx.update(|cx| assert!(crate::dock::has_dock_window(cx)));
    }
}
