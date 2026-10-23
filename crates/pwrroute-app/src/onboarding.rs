//! First run: when the window opens on its own, what it is told, and what
//! its choices do.
//!
//! - [`State`] is `state.json` in the data directory: whether first run was
//!   finished or dismissed (it never opens on its own again once it was),
//!   and the choices made. `--first-run` still forces the window.
//! - [`Facts`] is what the window is built from: who handles links today,
//!   the installed browsers, the Finicky config the router found, and the
//!   fallback already in `rules.json`. [`Facts::probe`] reads the machine;
//!   tests write their own and build the model with [`model`].
//! - [`hooks`] makes the window's choices real: the fallback goes to
//!   `rules.json`'s `defaultBrowser` and `defaultProfile` (empty for the
//!   browser's own default profile), the Finicky choice is recorded and
//!   "Import N rules" opens the importer on Choose source, "Set as
//!   default browser" asks the OS through [`crate::default_browser`], and
//!   "Open PwrRoute" or "Later" marks first run done and opens the main
//!   window on Rules, on the display the first-run window was on, so
//!   leaving the window never leaves the app without one. Closing the
//!   window instead only marks first run done: the menu bar item stays, as
//!   the window's own help line says.

use std::{
    fs, io,
    path::{Path, PathBuf},
    rc::Rc,
};

use anyhow::{Context as _, Result};
use gpui_kit::{App, DisplayId, Subscription, WindowHandle};
use pwrroute_browsers::Env;
use pwrroute_engine::{DefaultHost, Engine, Limits, RulesFile, discovery};
use serde::{Deserialize, Serialize};

use crate::app::Screen;
use crate::default_browser::{self, DefaultState};
use crate::first_run::{
    self, BrowserChoice, FinickyConfig, FirstRun, FirstRunHooks, FirstRunModel, ImportChoice,
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
    /// Step 1's choice, as written to `rules.json`'s `defaultBrowser`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    /// Step 1's profile, as written to `rules.json`'s `defaultProfile`;
    /// `None` for the browser's own default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_profile: Option<String>,
    /// Step 2's choice.
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

/// A Finicky config the router found, and what it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinickyFound {
    pub path: PathBuf,
    pub rewrites: usize,
    pub handlers: usize,
    /// The rules the importer proves visual rules hold exactly.
    pub importable: usize,
}

/// What first run is built from.
#[derive(Clone, Debug)]
pub struct Facts {
    /// Who handles links today.
    pub handler: DefaultState,
    /// The browsers on this machine and their profiles, for the fallback
    /// pickers.
    pub browsers: Vec<BrowserChoice>,
    pub finicky: Option<FinickyFound>,
    /// `rules.json`'s `defaultBrowser`, empty when it names none.
    pub rules_default: String,
    /// `rules.json`'s `defaultProfile`, empty when it names none.
    pub rules_profile: String,
    pub state: State,
}

impl Facts {
    /// Reads the machine: the OS's answer on the default browser (already
    /// read by [`default_browser`]), the installed browsers, the config the
    /// router chose, `rules.json` and `state.json`.
    pub fn probe(cx: &App) -> Self {
        let env = discovery::Environment::current();
        let (rules_default, rules_profile) = discovery::rules_path(&env)
            .and_then(|path| RulesFile::load(&path).ok())
            .map(|rules| (rules.default_browser, rules.default_profile))
            .unwrap_or_default();
        let state = pwrroute_os::data_dir()
            .and_then(|dir| State::load(&dir).ok())
            .unwrap_or_default();
        Self {
            handler: default_browser::state(cx),
            // The same read as Settings › Browsers: names, ids and profile
            // names.
            browsers: crate::browser_scan::scan(&Env::system())
                .into_iter()
                .map(|app| BrowserChoice {
                    label: BrowserLabel::new(&badge_for(&app.name), &app.name, None),
                    value: app.id().to_owned(),
                    profiles: app.profiles,
                })
                .collect(),
            finicky: crate::router::config_in_charge().map(|path| {
                // Counting the config's rules means evaluating it, as the
                // Rules screen does. A config that fails to load is still
                // found; it just has no counts.
                let engine = Engine::load(&path, Rc::new(DefaultHost), Limits::default()).ok();
                let counts = engine
                    .as_ref()
                    .and_then(|engine| engine.describe().ok().flatten())
                    .map_or((0, 0), |config| {
                        (config.rewrites.len(), config.handlers.len())
                    });
                let importable = engine
                    .and_then(|engine| engine.review_import(&[]).ok().flatten())
                    .map_or(0, |review| {
                        review
                            .rewrites
                            .iter()
                            .chain(&review.handlers)
                            .chain([&review.default_browser])
                            .filter(|r| matches!(r.verdict, pwrroute_engine::Verdict::Imports(_)))
                            .count()
                    });
                FinickyFound {
                    path,
                    rewrites: counts.0,
                    handlers: counts.1,
                    importable,
                }
            }),
            rules_default,
            rules_profile,
            state,
        }
    }
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
            facts.state.fallback_profile.as_deref().and_then(non_empty),
        )),
        None => {
            non_empty(&facts.rules_default).map(|value| (value, non_empty(&facts.rules_profile)))
        }
    };
    let fallback = chosen.map(|(value, profile)| {
        let mut label = facts
            .browsers
            .iter()
            .find(|choice| {
                choice.value.eq_ignore_ascii_case(&value)
                    || choice.label.name.eq_ignore_ascii_case(&value)
            })
            .map_or_else(
                // Named in rules.json but not found here: shown as written.
                || BrowserLabel::new(&badge_for(&value), &value, None),
                |choice| choice.label.clone(),
            );
        label.profile = profile;
        label
    });
    FirstRunModel {
        fallback,
        browsers: facts.browsers.clone(),
        finicky: facts.finicky.as_ref().map(|found| FinickyConfig {
            path: tilde(&found.path, home),
            rewrites: found.rewrites,
            handlers: found.handlers,
            importable: found.importable,
        }),
        import: facts.state.import,
        current_default: facts.handler.current_name(),
        default_requested: false,
        waiting: false,
        is_default: facts.handler.is_default(),
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

/// [`hooks`] over these files, with `open_main` as what follows the
/// window, given the display the first-run window was on: tests give it a
/// scratch directory and count the calls.
pub fn hooks_with(
    data_dir: Option<PathBuf>,
    rules_path: Option<PathBuf>,
    open_main: OpenMain,
) -> FirstRunHooks {
    let fallback_dir = data_dir.clone();
    let import_dir = data_dir.clone();
    let config = crate::router::config_in_charge();
    let finish_dir = data_dir;
    FirstRunHooks {
        on_fallback: Rc::new(move |choice: first_run::Fallback, _| {
            if let Some(path) = &rules_path {
                let saved = RulesFile::load(path).and_then(|mut rules| {
                    rules.default_browser.clone_from(&choice.browser.value);
                    rules.default_profile = choice.profile.clone().unwrap_or_default();
                    rules.save(path)
                });
                if let Err(error) = saved {
                    eprintln!("pwrroute: cannot save the fallback browser: {error:#}");
                }
            }
            edit_state(fallback_dir.as_deref(), |state| {
                state.fallback = Some(choice.browser.value.clone());
                state.fallback_profile.clone_from(&choice.profile);
            });
        }),
        on_import: Rc::new(move |choice, cx| {
            edit_state(import_dir.as_deref(), |state| state.import = Some(choice));
            // "Import N rules" goes straight to Choose source with every
            // proven rule checked; the importer writes rules.json.
            if choice == ImportChoice::Import
                && let Some(path) = config.clone()
            {
                cx.defer(move |cx| crate::importer_view::open(path, true, cx));
            }
        }),
        on_set_default_browser: Rc::new(|(), cx| match default_browser::request(cx) {
            Ok(outcome) => {
                eprintln!("pwrroute: first run asked the OS for the default browser: {outcome:?}");
            }
            Err(error) => {
                eprintln!("pwrroute: first run could not ask for the default browser: {error}");
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

/// Opens first run over the machine's facts. Step 3 follows the OS's
/// answer (read every second while the window is open and PwrRoute is not
/// the default, and whenever the window comes to the front), and closing
/// the window, however, marks first run done.
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
    let _ = window.update(cx, |_, window, cx| default_browser::watch(window, cx));
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
    use gpui_kit::TestAppContext;

    use super::*;
    use crate::first_run::StepState;

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
            label: BrowserLabel::new("Sa", "Safari", None),
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
                handlers: 6,
                importable: 5,
            }),
            rules_default: String::new(),
            rules_profile: String::new(),
            state: State::default(),
        }
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
        assert_eq!(built.current_default, "Google Chrome");
        assert!(!built.is_default);
        let finicky = built.finicky.as_ref().unwrap();
        assert_eq!(finicky.path, "~/.finicky.ts");
        assert_eq!((finicky.rewrites, finicky.handlers), (2, 6));
        assert_eq!(finicky.importable, 5);
        assert_eq!(
            built.steps(),
            [StepState::Current, StepState::Todo, StepState::Todo]
        );
        assert!(
            built
                .import_detail()
                .contains("Import what the visual rules can hold")
        );
    }

    #[test]
    fn a_fallback_already_in_rules_json_is_step_one_done() {
        let mut facts = facts();
        facts.rules_default = "google chrome".to_owned();
        let built = model(&facts, None);
        assert_eq!(built.fallback, Some(chrome().label));
        assert_eq!(built.steps()[0], StepState::Done);

        // state.json's choice wins, and an app not found here is shown as
        // written.
        facts.state.fallback = Some("com.example.Browser".to_owned());
        let built = model(&facts, None);
        assert_eq!(
            built.fallback,
            Some(BrowserLabel::new(
                &badge_for("com.example.Browser"),
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
        let built = model(&facts, None);
        assert_eq!(built.current_default, "your current browser");
        assert_eq!(built.import, Some(ImportChoice::KeepFile));

        facts.handler = DefaultState::PwrRoute;
        facts.finicky = None;
        let built = model(&facts, None);
        assert!(built.is_default);
        assert_eq!(built.finicky, None);
        assert_eq!(tilde(Path::new("/x/y"), Some(Path::new("/z"))), "/x/y");
    }

    /// Step 1's browser and profile go to rules.json's `defaultBrowser`
    /// and `defaultProfile` (the router's fallback) and to state.json, and
    /// the next first run reads them back from either.
    #[gpui_kit::test]
    fn step_one_saves_the_browser_and_profile(cx: &mut TestAppContext) {
        use crate::first_run::Fallback;
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("PwrRoute");
        let rules_path = data_dir.join("rules.json");
        let hooks = hooks_with(
            Some(data_dir.clone()),
            Some(rules_path.clone()),
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
            );
        });
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
            );
        });
        let (rules, state) = saved();
        assert_eq!(rules.default_browser, "com.apple.Safari");
        assert_eq!(rules.default_profile, "");
        assert_eq!(state.fallback_profile, None);
        let mut after = facts();
        after.state = state;
        let built = model(&after, None);
        assert_eq!(
            built.fallback,
            Some(BrowserLabel::new("Sa", "Safari", None))
        );
        assert!(built.fallback_profiles().is_empty());
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
