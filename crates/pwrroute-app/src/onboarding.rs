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
use gpui_kit::{App, DisplayId, Subscription, WindowHandle};
use pwrroute_engine::{
    AppType, BrowserSpec, DefaultHost, Engine, Limits, Precedence, RulesFile, discovery,
};
use serde::{Deserialize, Serialize};

use crate::app::Screen;
use crate::default_browser::{self, DefaultState};
use crate::first_run::{
    self, BrowserChoice, FinickyConfig, FirstRun, FirstRunHooks, FirstRunModel, ImportChoice,
    ImportOutcome, Imported,
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
                eprintln!(
                    "pwrroute: cannot read {}: {error:#}",
                    Self::path(dir).display()
                );
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
    let result = State::load(dir).and_then(|mut state| {
        edit(&mut state);
        state.save(dir)
    });
    if let Err(error) = result {
        eprintln!("pwrroute: cannot save first-run state: {error:#}");
    }
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
    /// Reads the config at `path`: evaluating it, as the Rules screen does,
    /// to count its rules and the ones that import, and to read its
    /// default. A config that fails to load is still found, with the
    /// reason.
    pub fn read(path: PathBuf) -> Self {
        let engine = match Engine::load(&path, Rc::new(DefaultHost), Limits::default()) {
            Ok(engine) => engine,
            Err(error) => {
                return Self {
                    error: Some(format!("{error:#}")),
                    path,
                    ..Self::default()
                };
            }
        };
        let description = match engine.describe() {
            Ok(description) => description,
            Err(error) => {
                return Self {
                    error: Some(format!("{error:#}")),
                    path,
                    ..Self::default()
                };
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
        Self {
            path,
            rewrites,
            handlers,
            importable,
            default_browser,
            default_profile,
            error: None,
        }
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
    /// `rules.json`'s `defaultBrowser`, empty when it names none.
    pub rules_default: String,
    /// `rules.json`'s `defaultProfile`, empty when it names none.
    pub rules_profile: String,
    /// `rules.json`'s rules.
    pub rules: usize,
    /// `rules.json`'s `precedence`.
    pub precedence: Precedence,
    pub state: State,
}

impl Facts {
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
            finicky: crate::router::config_in_charge().map(FinickyFound::read),
            rules_default: rules.default_browser,
            rules_profile: rules.default_profile,
            rules: rules.rules.len(),
            precedence: rules.precedence,
            state,
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
    FirstRunModel {
        fallback,
        browsers: facts.browsers.clone(),
        finicky,
        import,
        precedence: facts.precedence,
        rules: facts.rules,
        current_default: match &facts.handler {
            DefaultState::Other(_) => Some(facts.handler.current_name()),
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
pub type SetSource = Rc<dyn Fn(crate::settings_store::SourceSettings, &mut App)>;

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
    FirstRunHooks {
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
            ImportChoice::KeepFile => {
                edit_state(import_dir.as_deref(), |state| state.import = Some(choice));
            }
            // The router stops reading the config, from the next link.
            ImportChoice::DontUse => {
                edit_state(import_dir.as_deref(), |state| state.import = Some(choice));
                let mut source = crate::settings_store::store(cx)
                    .map(|store| store.read(cx).settings().source.clone())
                    .unwrap_or_default();
                source.visual_only = true;
                source.config = None;
                set_source(source, cx);
            }
            // The importer on Choose source, every proven rule checked; it
            // writes rules.json and answers when it closes.
            ImportChoice::Import => {
                // Answered on the next turn: the window is still mid-update.
                let Some(path) = config.clone() else {
                    cx.defer(move |cx| reply(ImportOutcome::Cancelled, cx));
                    return;
                };
                let dir = import_dir.clone();
                let rules = rules_path.clone();
                let on_done: crate::importer_view::OnDone = Rc::new(move |outcome, cx| {
                    let outcome = match outcome {
                        crate::importer_view::ImportOutcome::Cancelled => ImportOutcome::Cancelled,
                        crate::importer_view::ImportOutcome::Imported {
                            count,
                            keeps_config,
                        } => {
                            edit_state(dir.as_deref(), |state| {
                                state.import = Some(ImportChoice::Import);
                            });
                            ImportOutcome::Imported(imported(count, keeps_config, rules.as_deref()))
                        }
                    };
                    reply(outcome, cx);
                });
                let rules = rules_path.clone();
                cx.defer(move |cx| {
                    crate::importer_view::open_from_first_run(path, rules, on_done, cx);
                });
            }
        }),
        on_rescan: Rc::new(|cx| {
            let hidden = hidden(cx);
            choices(crate::browser_scan::rescan(cx), &hidden)
        }),
        on_set_default_browser: Rc::new(|cx| match default_browser::request(cx) {
            Ok(outcome) => {
                eprintln!("pwrroute: first run asked the OS for the default browser: {outcome:?}");
                Ok(())
            }
            Err(error) => {
                eprintln!("pwrroute: first run could not ask for the default browser: {error}");
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

/// The open first-run window and what follows it.
struct Open {
    window: WindowHandle<FirstRun>,
    _answers: Subscription,
}

impl gpui_kit::Global for Open {}

/// Opens first run over the machine's facts, with the Everything else
/// browser preselected and saved when none was chosen. The default-browser
/// step follows the OS's answer (read every second while the window is open
/// and PwrRoute is not the default, and whenever the window comes to the
/// front), and closing the window, however, marks first run done.
pub fn open(cx: &mut App) {
    let facts = Facts::probe(cx);
    let model = model(&facts, std::env::home_dir().as_deref());
    let window = match first_run::open(model, hooks(), cx) {
        Ok(window) => window,
        Err(error) => {
            eprintln!("pwrroute: could not open the first-run window: {error}");
            return;
        }
    };
    let _ = window.update(cx, |view, window, cx| {
        view.preselect(cx);
        default_browser::watch(window, cx);
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

    fn chrome() -> BrowserChoice {
        BrowserChoice {
            value: "com.google.Chrome".to_owned(),
            label: BrowserLabel::new("Ch", "Google Chrome", None),
            profiles: vec!["Personal".to_owned(), "Work".to_owned()],
        }
    }

    fn safari() -> BrowserChoice {
        BrowserChoice {
            value: "com.apple.Safari".to_owned(),
            label: BrowserLabel::new("Sf", "Safari", None),
            profiles: Vec::new(),
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
                },
                safari(),
            ],
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
        }
    }

    /// A `set_source` for tests that never turn the config off.
    fn no_source() -> SetSource {
        Rc::new(|_, _| panic!("this test turns no config off"))
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
            Rc::new(move |source, _| saved.borrow_mut().push(source)),
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
        // Answered at once: nothing to reply.
        assert!(replies.borrow().is_empty());

        // Keep changes no source.
        cx.update(|cx| (hooks.on_import)(ImportChoice::KeepFile, Rc::new(|_, _| {}), cx));
        assert_eq!(sources.borrow().len(), 1);
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
}
