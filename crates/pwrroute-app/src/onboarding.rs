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
//!   `rules.json`'s `defaultBrowser`, the Finicky choice is recorded (there
//!   is no importer yet, so the config stays in charge either way), and
//!   "Set as default browser" asks the OS through [`crate::default_browser`].

use std::{
    fs, io,
    path::{Path, PathBuf},
    rc::Rc,
};

use anyhow::{Context as _, Result};
use gpui_kit::{App, Subscription, WindowHandle};
use pwrroute_browsers::{Env, detect_installed};
use pwrroute_engine::{DefaultHost, Engine, Limits, RulesFile, discovery};
use serde::{Deserialize, Serialize};

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
}

/// What first run is built from.
#[derive(Clone, Debug)]
pub struct Facts {
    /// Who handles links today.
    pub handler: DefaultState,
    /// The browsers on this machine, for the fallback picker.
    pub browsers: Vec<BrowserChoice>,
    pub finicky: Option<FinickyFound>,
    /// `rules.json`'s `defaultBrowser`, empty when it names none.
    pub rules_default: String,
    pub state: State,
}

impl Facts {
    /// Reads the machine: the OS's answer on the default browser (already
    /// read by [`default_browser`]), the installed browsers, the config the
    /// router chose, `rules.json` and `state.json`.
    pub fn probe(cx: &App) -> Self {
        let env = discovery::Environment::current();
        let rules_default = discovery::rules_path(&env)
            .and_then(|path| RulesFile::load(&path).ok())
            .map(|rules| rules.default_browser)
            .unwrap_or_default();
        let state = pwrroute_os::data_dir()
            .and_then(|dir| State::load(&dir).ok())
            .unwrap_or_default();
        Self {
            handler: default_browser::state(cx),
            browsers: detect_installed(&Env::system())
                .into_iter()
                .map(|browser| {
                    let value = browser
                        .bundle_id
                        .clone()
                        .unwrap_or_else(|| browser.name.clone());
                    BrowserChoice {
                        label: BrowserLabel::new(&badge_for(&browser.name), &browser.name, None),
                        value,
                    }
                })
                .collect(),
            finicky: crate::router::config_in_charge().map(|path| {
                // Counting the config's rules means evaluating it, as the
                // Rules screen does. A config that fails to load is still
                // found; it just has no counts.
                let counts = Engine::load(&path, Rc::new(DefaultHost), Limits::default())
                    .ok()
                    .and_then(|engine| engine.describe().ok().flatten())
                    .map_or((0, 0), |config| {
                        (config.rewrites.len(), config.handlers.len())
                    });
                FinickyFound {
                    path,
                    rewrites: counts.0,
                    handlers: counts.1,
                }
            }),
            rules_default,
            state,
        }
    }
}

/// The window's model for these facts. `home` shortens the config's path
/// to `~/…`.
pub fn model(facts: &Facts, home: Option<&Path>) -> FirstRunModel {
    let chosen = facts
        .state
        .fallback
        .clone()
        .filter(|value| !value.is_empty())
        .or_else(|| Some(facts.rules_default.clone()).filter(|value| !value.is_empty()));
    let fallback = chosen.map(|value| {
        facts
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
            )
    });
    FirstRunModel {
        fallback,
        browsers: facts.browsers.clone(),
        finicky: facts.finicky.as_ref().map(|found| FinickyConfig {
            path: tilde(&found.path, home),
            rewrites: found.rewrites,
            handlers: found.handlers,
            // No importer yet: the window says so and offers to keep the file.
            importable: 0,
        }),
        import: facts.state.import,
        current_default: facts.handler.current_name(),
        default_requested: false,
        is_default: facts.handler.is_default(),
    }
}

/// `~/.finicky.ts` for a path under `home`; the path as it is otherwise.
fn tilde(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// The window's choices, made real.
pub fn hooks() -> FirstRunHooks {
    let data_dir = pwrroute_os::data_dir();
    let rules_path = discovery::rules_path(&discovery::Environment::current());
    let fallback_dir = data_dir.clone();
    let import_dir = data_dir;
    FirstRunHooks {
        on_fallback: Rc::new(move |choice, _| {
            if let Some(path) = &rules_path {
                let saved = RulesFile::load(path).and_then(|mut rules| {
                    rules.default_browser = choice.value.clone();
                    rules.save(path)
                });
                if let Err(error) = saved {
                    eprintln!("pwrroute: cannot save the fallback browser: {error:#}");
                }
            }
            edit_state(fallback_dir.as_deref(), |state| {
                state.fallback = Some(choice.value.clone());
            });
        }),
        on_import: Rc::new(move |choice, _| {
            // Recorded only: with no importer the config stays in charge,
            // which is what the router does on its own.
            edit_state(import_dir.as_deref(), |state| state.import = Some(choice));
        }),
        on_set_default_browser: Rc::new(|(), cx| match default_browser::request(cx) {
            Ok(outcome) => {
                eprintln!("pwrroute: first run asked the OS for the default browser: {outcome:?}");
            }
            Err(error) => {
                eprintln!("pwrroute: first run could not ask for the default browser: {error}");
            }
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
/// answer, and closing the window, however, marks first run done.
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
            let state = registrar.read(cx).state.clone();
            let _ = window.update(cx, |view, _, cx| view.set_default_state(&state, cx));
            if state.is_default() {
                edit_state(pwrroute_os::data_dir().as_deref(), |state| {
                    state.first_run_completed = true;
                });
            }
        })
    });
    cx.on_window_closed(move |cx, id| {
        let open = cx.try_global::<Open>().map(|open| open.window.window_id());
        if open == Some(id) {
            cx.remove_global::<Open>();
            edit_state(pwrroute_os::data_dir().as_deref(), |state| {
                state.first_run_completed = true;
            });
        }
    })
    .detach();
    cx.set_global(Open {
        window,
        _answers: answers.unwrap_or_else(|| Subscription::new(|| {})),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::first_run::StepState;

    fn chrome() -> BrowserChoice {
        BrowserChoice {
            value: "com.google.Chrome".to_owned(),
            label: BrowserLabel::new("Ch", "Google Chrome", None),
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
                },
            ],
            finicky: Some(FinickyFound {
                path: PathBuf::from("/Users/u/.finicky.ts"),
                rewrites: 2,
                handlers: 6,
            }),
            rules_default: String::new(),
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
            import: Some(ImportChoice::KeepFile),
        };
        state.save(&dir).unwrap();
        assert_eq!(State::load(&dir).unwrap(), state);
        assert!(!State::pending(&dir));
        let text = fs::read_to_string(State::path(&dir)).unwrap();
        assert!(text.contains("\"firstRunCompleted\": true"));
        assert!(text.contains("\"import\": \"keepFile\""));

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
        assert_eq!(built.browsers.len(), 2);
        assert_eq!(built.current_default, "Google Chrome");
        assert!(!built.is_default);
        let finicky = built.finicky.as_ref().unwrap();
        assert_eq!(finicky.path, "~/.finicky.ts");
        assert_eq!((finicky.rewrites, finicky.handlers), (2, 6));
        assert_eq!(finicky.importable, 0);
        assert_eq!(
            built.steps(),
            [StepState::Current, StepState::Todo, StepState::Todo]
        );
        assert!(
            built
                .import_detail()
                .contains("importing is not available yet")
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
}
