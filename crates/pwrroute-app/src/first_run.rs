//! The first-run window: `design/PwrRoute Overview and Setup.dc.html` turn
//! 4 (artboards 4a-4i).
//!
//! Two or three steps, in the order that makes the last one safe: what to
//! do with a Finicky config, when one was found; where everything else goes
//! (the browser that takes links no rule claims, and for a config kept in
//! charge, who owns that default); and only then the OS default. This
//! window never touches the OS or a file itself. Its choices go to
//! [`FirstRunHooks`]; the default-browser step calls
//! [`FirstRunHooks::on_set_default_browser`] and nothing else, so whoever
//! opens the window decides what that does (`crate::onboarding` wires
//! `pwrroute_os::set_default_browser` in the app; tests count the calls).
//! The OS's answer comes back through [`FirstRun::set_default_state`], and
//! an import's through the [`ImportReply`] the window hands
//! [`FirstRunHooks::on_import`]. Leaving the window, by "Open PwrRoute" once
//! every step is done or by "Later" before that, goes through
//! [`FirstRunHooks::on_finish`].

use std::f32::consts::{PI, TAU};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Animation, AnimationExt as _, App, AppContext as _, Bounds, BoxShadow, ClickEvent, Context,
    DisplayId, Div, FocusHandle, HighlightStyle, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, PathBuilder, Render, Rgba, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, StyledText, TitlebarOptions, Window,
    WindowBounds, WindowHandle, WindowOptions, anchored, canvas, deferred, div, point, px, size,
};
use pwrroute_engine::Precedence;
use serde::{Deserialize, Serialize};

use crate::chrome::{BRAND_X, STRIP_HEIGHT};
use crate::default_browser::DefaultState;
use crate::theme::{self, icon, mono, text, tracked};
use crate::tray::model::BrowserLabel;
use crate::widgets::{
    badge, badge_for, caret, field_label, input_box, menu_card, menu_row, profiles_hint, segment,
    segmented,
};

/// The element id of the window's root, for tests and the inspector.
pub const ROOT_ID: &str = "first-run-root";

/// `.dv-card.win { width: 680px; min-height: 600px }`. The body scrolls
/// past that, so the footer is never cut off.
const WINDOW_SIZE: (f32, f32) = (680.0, 600.0);

/// What to do with a Finicky config that was found. Serialized into the
/// first-run state file by its camelCase name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportChoice {
    /// Copy what the visual rules can hold into `rules.json`, through the
    /// importer. Recorded only once an import finished.
    Import,
    /// Keep the file in charge, as it is.
    KeepFile,
    /// The router stops reading the file (`settings.json`'s
    /// `source.visualOnly`). Older builds wrote "startEmpty" for their
    /// Start empty, which never did that; it reads as this and
    /// `onboarding::model` does not carry it over.
    #[serde(alias = "startEmpty")]
    DontUse,
}

/// A Finicky config found on this machine.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinickyConfig {
    /// As shown: `~/.finicky.ts`.
    pub path: String,
    pub rewrites: usize,
    pub handlers: usize,
    /// How many rules the importer proves visual rules hold exactly. Zero:
    /// the step offers only to keep the file, or not use it.
    pub importable: usize,
    /// Its `defaultBrowser` as the config names it (a name or a bundle id),
    /// when that is a fixed browser; `None` for a function or `null`.
    pub default_browser: Option<String>,
    /// That browser's profile, when the config names one.
    pub default_profile: Option<String>,
    /// That browser by the name the reader knows ("Safari"), for "the
    /// config's default · Safari".
    pub default_name: Option<String>,
    /// Why the file does not load, as one line ("SyntaxError: … ·
    /// ~/.finicky.ts line 42"). `None`: it loads.
    pub error: Option<String>,
}

/// A browser the Everything else select offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserChoice {
    /// What `rules.json`'s `defaultBrowser` gets: the bundle id on macOS,
    /// the name elsewhere.
    pub value: String,
    pub label: BrowserLabel,
    /// Its profiles by the names the browser shows (as Settings › Browsers
    /// lists them), for the profile select. Empty: no select.
    pub profiles: Vec<String>,
}

/// The Everything else choice: a browser, and one of its profiles or its
/// default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fallback {
    pub browser: BrowserChoice,
    /// What `rules.json`'s `defaultProfile` gets. `None`: the browser's
    /// own default profile.
    pub profile: Option<String>,
}

/// Where the Everything else browser came from, when the window picked it
/// rather than the reader: the step's detail line says so until the
/// reader picks another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picked {
    /// The config's own fixed `defaultBrowser`.
    Config,
    /// The browser that is the OS default today.
    OsDefault,
    /// The first browser the scan found.
    FirstFound,
}

/// Which of the Everything else selects is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picking {
    None,
    Browser,
    Profile,
}

/// The window's steps, in order. [`Step::Config`] is there only when a
/// Finicky config was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// "Bring your Finicky config".
    Config,
    /// "Where should everything else go?"
    EverythingElse,
    /// "Make PwrRoute the default browser".
    MakeDefault,
}

/// A step's state, as the design draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    Done,
    Current,
    Todo,
}

/// What a finished import left behind, as the importer and `rules.json`
/// report it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Imported {
    /// The rules written, as the importer counts them.
    pub count: usize,
    /// Choose source kept the config in charge; otherwise the router
    /// stopped reading it.
    pub keeps_config: bool,
    /// `rules.json`'s rules after the import.
    pub rules: usize,
    /// `rules.json`'s `defaultBrowser` and `defaultProfile` after it: an
    /// import can bring the config's default.
    pub default_browser: String,
    pub default_profile: String,
}

/// How the importer that "Import N rules" opened was left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportOutcome {
    /// Closed without importing: the step comes back as it was.
    Cancelled,
    Imported(Imported),
}

/// How the window hears back from an import it started.
pub type ImportReply = Rc<dyn Fn(ImportOutcome, &mut App)>;

/// What the window shows.
#[derive(Clone, Debug, Default)]
pub struct FirstRunModel {
    /// The Everything else browser, with the chosen profile as the label's.
    /// `None` until one is picked or preselected.
    pub fallback: Option<BrowserLabel>,
    /// Why `fallback` is what it is, while the window chose it.
    pub picked: Option<Picked>,
    /// What the Everything else select offers: the browsers found on this
    /// machine.
    pub browsers: Vec<BrowserChoice>,
    /// The config step's subject. `None`: no Finicky config was found, and
    /// the window has two steps.
    pub finicky: Option<FinickyConfig>,
    pub import: Option<ImportChoice>,
    /// The importer is open, from "Import N rules".
    pub importing: bool,
    /// What a finished import left behind.
    pub imported: Option<Imported>,
    /// `rules.json`'s `precedence`: who owns everything else while a
    /// config is in charge.
    pub precedence: Precedence,
    /// `rules.json`'s rules, for Where links go.
    pub rules: usize,
    /// The browser links open in today, before PwrRoute is the default.
    /// `None` when the OS did not say, or it is PwrRoute.
    pub current_default: Option<String>,
    /// The default-browser step was asked for at least once. The OS
    /// answers on its own, in its own dialog; the button then reads "Ask
    /// again".
    pub default_requested: bool,
    /// A request is out and the OS has not answered yet: whoever opened the
    /// window is reading the answer back, and the step says so.
    pub waiting: bool,
    /// The OS turned the request down with an error (4i).
    pub default_error: Option<String>,
    /// The OS reports PwrRoute is the default browser.
    pub is_default: bool,
}

impl FirstRunModel {
    /// The design's 4a: a config found with 3 of its rules importable, the
    /// Everything else browser preselected from its default.
    #[cfg(test)]
    pub fn fixture() -> Self {
        Self {
            fallback: Some(BrowserLabel::new("Sf", "Safari", None)),
            picked: Some(Picked::Config),
            browsers: vec![
                BrowserChoice {
                    value: "com.google.Chrome".to_owned(),
                    label: BrowserLabel::new("Ch", "Google Chrome", None),
                    profiles: vec!["Personal".to_owned(), "Work".to_owned()],
                },
                BrowserChoice {
                    value: "org.mozilla.firefox".to_owned(),
                    label: BrowserLabel::new("Ff", "Firefox", None),
                    profiles: vec!["default-release".to_owned()],
                },
                BrowserChoice {
                    value: "com.apple.Safari".to_owned(),
                    label: BrowserLabel::new("Sf", "Safari", None),
                    profiles: Vec::new(),
                },
            ],
            finicky: Some(FinickyConfig {
                path: "~/.finicky.ts".to_owned(),
                rewrites: 2,
                handlers: 5,
                importable: 3,
                default_browser: Some("com.apple.Safari".to_owned()),
                default_profile: None,
                default_name: Some("Safari".to_owned()),
                error: None,
            }),
            current_default: Some("Safari".to_owned()),
            ..Self::default()
        }
    }

    /// The Everything else browser, as this machine lists it.
    pub fn fallback_choice(&self) -> Option<&BrowserChoice> {
        let chosen = self.fallback.as_ref()?;
        self.browsers
            .iter()
            .find(|choice| choice.label.name == chosen.name)
    }

    /// The profiles the profile select offers after its "Default profile":
    /// the chosen browser's. Empty, and no select, for a browser without
    /// profiles or one not found on this machine.
    pub fn fallback_profiles(&self) -> &[String] {
        self.fallback_choice()
            .map_or(&[], |choice| choice.profiles.as_slice())
    }

    /// The browser the window picks for Everything else when nothing is
    /// chosen yet, in this order: the config's fixed `defaultBrowser`, the
    /// browser that is the OS default today (never PwrRoute), the first
    /// browser found. With the config's browser comes its profile, when
    /// the browser has one by that name. `None` with no browsers at all.
    pub fn preselection(&self) -> Option<(Fallback, Picked)> {
        let find = |value: &str| {
            self.browsers.iter().find(|choice| {
                choice.value.eq_ignore_ascii_case(value)
                    || choice.label.name.eq_ignore_ascii_case(value)
            })
        };
        if let Some(config) = &self.finicky
            && let Some(browser) = config.default_browser.as_deref().and_then(find)
        {
            let profile = config.default_profile.as_deref().and_then(|wanted| {
                browser
                    .profiles
                    .iter()
                    .find(|profile| profile.eq_ignore_ascii_case(wanted))
                    .cloned()
            });
            let fallback = Fallback {
                browser: browser.clone(),
                profile,
            };
            return Some((fallback, Picked::Config));
        }
        if let Some(browser) = self.current_default.as_deref().and_then(find) {
            let fallback = Fallback {
                browser: browser.clone(),
                profile: None,
            };
            return Some((fallback, Picked::OsDefault));
        }
        self.browsers.first().map(|browser| {
            let fallback = Fallback {
                browser: browser.clone(),
                profile: None,
            };
            (fallback, Picked::FirstFound)
        })
    }

    /// The steps this window has, each with its state. A step is to do
    /// until every step before it is done; the OS step is done whenever the
    /// OS says PwrRoute is the default.
    pub fn steps(&self) -> Vec<(Step, StepState)> {
        use StepState::{Current, Done, Todo};
        let mut steps = Vec::with_capacity(3);
        let mut open = true;
        if self.finicky.is_some() {
            let state = if self.import.is_some() { Done } else { Current };
            open = state == Done;
            steps.push((Step::Config, state));
        }
        let everything_else = match (open, self.fallback.is_some()) {
            (false, _) => Todo,
            (true, true) => Done,
            (true, false) => Current,
        };
        open = everything_else == Done;
        steps.push((Step::EverythingElse, everything_else));
        let default = if self.is_default {
            Done
        } else if open {
            Current
        } else {
            Todo
        };
        steps.push((Step::MakeDefault, default));
        steps
    }

    /// One step's state; a step the window does not have is done.
    pub fn state_of(&self, step: Step) -> StepState {
        self.steps()
            .into_iter()
            .find_map(|(each, state)| (each == step).then_some(state))
            .unwrap_or(StepState::Done)
    }

    /// Links route only once PwrRoute is the default browser.
    pub fn routing(&self) -> bool {
        self.is_default
    }

    /// Every step is done: the window offers to open PwrRoute.
    pub fn complete(&self) -> bool {
        self.steps()
            .iter()
            .all(|(_, state)| *state == StepState::Done)
    }

    /// The config step kept the config in charge: Keep, or an import whose
    /// Choose source kept it. Until the step is answered the window does
    /// not draw it as kept.
    pub fn config_in_charge(&self) -> bool {
        self.finicky.is_some()
            && match self.import {
                None => false,
                Some(ImportChoice::KeepFile) => true,
                Some(ImportChoice::Import) => {
                    self.imported.as_ref().is_none_or(|done| done.keeps_config)
                }
                Some(ImportChoice::DontUse) => false,
            }
    }

    /// The header's title.
    pub fn heading(&self) -> &'static str {
        if self.complete() {
            "Links are routing."
        } else if self.finicky.is_some() {
            "Three steps, then links start routing"
        } else {
            "Two steps, then links start routing"
        }
    }

    /// The header's line under the title.
    pub fn help(&self) -> String {
        if !self.complete() {
            return "Links keep opening the way they do now until the last step. Close this any \
                    time; PwrRoute's Overview shows what's left."
                .to_owned();
        }
        let sources = match &self.finicky {
            Some(config) if self.config_in_charge() => format!("{} and your rules", config.path),
            _ => "your rules".to_owned(),
        };
        format!("Every link now goes where {sources} say. {}", tray_line())
    }

    /// The config step's paragraph.
    pub fn config_detail(&self) -> String {
        let Some(config) = &self.finicky else {
            return String::new();
        };
        let path = &config.path;
        if self.importing {
            return "Importing\u{2026} Finish or cancel in the importer window. Cancel brings \
                    this step back unchanged."
                .to_owned();
        }
        match self.import {
            Some(ImportChoice::Import) => {
                let count = self.imported.as_ref().map_or(0, |done| done.count);
                format!(
                    "Imported {}. {path} is unchanged.",
                    plural(count, "rule", "rules")
                )
            }
            Some(ImportChoice::KeepFile) if config.error.is_some() => format!(
                "PwrRoute uses {path} once it loads. Until then links go to the browser below."
            ),
            Some(ImportChoice::KeepFile) => {
                format!("{path} stays in charge. Edit it as you do now; PwrRoute reloads it.")
            }
            Some(ImportChoice::DontUse) => format!(
                "PwrRoute won't read {path}. Turn it back on in Settings \u{203a} Configuration."
            ),
            None if config.error.is_some() => format!(
                "Found {path}, but it doesn't load. Keep it and PwrRoute uses it once it loads; \
                 until then links go to the browser you pick in the next step."
            ),
            None if config.importable == 0 => format!(
                "Found {path}: {}. They use JavaScript that visual rules can't hold, so \
                 PwrRoute runs the file as Finicky did. The file is never changed.",
                holds(config)
            ),
            None => format!(
                "Found {path}: {}. Keep it running exactly as Finicky did, or copy the {} \
                 PwrRoute can show as visual rules. The file is never changed.",
                holds(config),
                plural(config.importable, "rule", "rules")
            ),
        }
    }

    /// The segmented control's first answer: "~/.finicky.ts's default ·
    /// Safari", or without the browser for a function or `null` default.
    pub fn config_owner_label(&self) -> String {
        let Some(config) = &self.finicky else {
            return String::new();
        };
        match &config.default_name {
            Some(name) => format!("{}'s default \u{00b7} {name}", config.path),
            None => format!("{}'s default", config.path),
        }
    }

    /// The Everything else step's paragraph.
    pub fn everything_else_detail(&self) -> String {
        if self.browsers.is_empty() {
            return format!(
                "No browsers found on {}. PwrRoute needs at least one to send links to.",
                this_machine()
            );
        }
        match &self.finicky {
            Some(config) if self.config_in_charge() => match self.precedence {
                Precedence::Config => format!(
                    "{}'s defaultBrowser handles links its handlers don't claim. The browser \
                     below still catches config errors, browsers that can't open, and every \
                     link while paused.",
                    config.path
                ),
                Precedence::PwrRoute => {
                    let instead = config
                        .default_name
                        .as_ref()
                        .map_or_else(String::new, |name| format!(" instead of {name}"));
                    format!(
                        "Your rules run first, then {}'s handlers. Links nothing claims go \
                         here{instead}.",
                        config.path
                    )
                }
            },
            _ => "Links no rule claims open here. It also takes links whose browser can't open, \
                  and every link while routing is paused."
                .to_owned(),
        }
    }

    /// The line under the select while the window's own pick stands.
    pub fn picked_note(&self) -> Option<String> {
        let line = match self.picked? {
            Picked::Config => format!(
                "Picked from {}'s defaultBrowser.",
                self.finicky.as_ref()?.path
            ),
            Picked::OsDefault => "Picked from your current default browser.".to_owned(),
            Picked::FirstFound => format!("Picked the first browser found on {}.", this_machine()),
        };
        Some(format!("{line} Change it any time."))
    }

    /// The default-browser step's paragraph.
    pub fn default_detail(&self) -> String {
        if self.is_default {
            return "PwrRoute is the default browser. Links route by your rules.".to_owned();
        }
        let asks = if cfg!(target_os = "macos") {
            "macOS will ask you to confirm."
        } else if cfg!(target_os = "windows") {
            "Windows opens Default apps, where you choose PwrRoute."
        } else {
            "Your desktop is asked directly."
        };
        match &self.current_default {
            Some(current) => {
                format!("{asks} Until then, links keep opening in {current} as they do now.")
            }
            None => format!("{asks} Until then, links keep opening the way they do now."),
        }
    }

    /// The default-browser step's line beside its button: the OS has the
    /// question, or it answered no (or was never answered) and the step
    /// offers to ask again.
    pub fn default_note(&self) -> Option<String> {
        if self.is_default {
            None
        } else if self.waiting {
            Some(waiting_line().to_owned())
        } else if self.default_requested {
            Some(match &self.current_default {
                Some(current) => format!("{current} is still the default."),
                None => "Your current browser is still the default.".to_owned(),
            })
        } else {
            None
        }
    }

    /// The warning under the default-browser step after the OS refused the
    /// request with an error (4i).
    pub fn default_warning(&self) -> Option<String> {
        let error = self.default_error.as_deref()?;
        let error = error.trim().trim_end_matches('.');
        Some(format!(
            "{} didn't take the request: {error}. Try Settings \u{203a} Browsers.",
            os_subject()
        ))
    }

    /// The Everything else answer in a few words: "~/.finicky.ts's default
    /// · Safari", "PwrRoute's default · Google Chrome · Personal", or the
    /// browser alone with no config in charge.
    pub fn everything_else_result(&self) -> String {
        let browser = self
            .fallback
            .as_ref()
            .map_or_else(String::new, BrowserLabel::display);
        if self.config_owns_everything_else() {
            self.config_owner_label()
        } else if self.config_in_charge() && self.precedence == Precedence::PwrRoute {
            format!("PwrRoute's default \u{00b7} {browser}")
        } else {
            browser
        }
    }

    /// The config's own `defaultBrowser` takes everything else: it is kept
    /// in charge, owns the default, and loads. While it does not load, the
    /// Everything else browser takes its links.
    fn config_owns_everything_else(&self) -> bool {
        self.config_in_charge()
            && self.precedence == Precedence::Config
            && self
                .finicky
                .as_ref()
                .is_some_and(|config| config.error.is_none())
    }

    /// A finished step folded to one line (4g): its short title and result.
    pub fn folded(&self, step: Step) -> (&'static str, String) {
        match step {
            Step::Config => {
                let path = self
                    .finicky
                    .as_ref()
                    .map_or("the config", |config| config.path.as_str());
                let result = match self.import {
                    Some(ImportChoice::Import) => format!(
                        "Imported {}.",
                        plural(
                            self.imported.as_ref().map_or(0, |done| done.count),
                            "rule",
                            "rules"
                        )
                    ),
                    Some(ImportChoice::DontUse) => format!("PwrRoute won't read {path}."),
                    Some(ImportChoice::KeepFile) | None => format!("{path} stays in charge."),
                };
                ("Bring your Finicky config", result)
            }
            Step::EverythingElse => ("Everything else", self.everything_else_result()),
            Step::MakeDefault => (
                "Default browser",
                "PwrRoute is the default browser.".to_owned(),
            ),
        }
    }

    /// The done panel's lines (4g), at most three, in the order links meet
    /// them.
    pub fn where_links_go(&self) -> Vec<WhereLine> {
        let rules = if self.rules == 0 {
            "none yet".to_owned()
        } else {
            plural(self.rules, "rule", "rules")
        };
        let fallback = self
            .fallback
            .as_ref()
            .map(|label| (label.badge.clone(), label.display()));
        let (rest, browser) = match &self.finicky {
            Some(config) if self.config_owns_everything_else() => (
                format!(" \u{2192} {}'s default", config.path),
                config
                    .default_name
                    .clone()
                    .map(|name| (badge_for(&name).to_string(), name)),
            ),
            Some(_) if self.config_in_charge() && self.precedence == Precedence::PwrRoute => {
                (" \u{2192} PwrRoute's default".to_owned(), fallback)
            }
            _ => (" \u{2192}".to_owned(), fallback),
        };
        let everything_else = WhereLine {
            done: true,
            subject: "Everything else".to_owned(),
            rest,
            browser,
        };
        let Some(config) = self.finicky.as_ref().filter(|_| self.config_in_charge()) else {
            return vec![
                WhereLine {
                    done: self.rules > 0,
                    subject: "Your rules".to_owned(),
                    rest: format!(" \u{00b7} {rules}"),
                    browser: None,
                },
                everything_else,
            ];
        };
        let config_line = if config.error.is_some() {
            WhereLine {
                done: false,
                subject: config.path.clone(),
                rest: " runs once it loads".to_owned(),
                browser: None,
            }
        } else {
            let claim = if config.handlers == 1 {
                "claims"
            } else {
                "claim"
            };
            WhereLine {
                done: true,
                subject: config.path.clone(),
                rest: format!(
                    " handles links its {} {claim}",
                    plural(config.handlers, "handler", "handlers")
                ),
                browser: None,
            }
        };
        let order = match self.precedence {
            Precedence::Config => "run after it",
            Precedence::PwrRoute => "run first",
        };
        let rules_line = WhereLine {
            done: self.rules > 0,
            subject: "Your rules".to_owned(),
            rest: format!(" {order} \u{00b7} {rules}"),
            browser: None,
        };
        match self.precedence {
            Precedence::Config => vec![config_line, rules_line, everything_else],
            Precedence::PwrRoute => vec![rules_line, config_line, everything_else],
        }
    }
}

/// One line of Where links go: a tick (or a neutral dot while there is
/// nothing in it), the subject in bold, the rest, and a browser badge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WhereLine {
    pub done: bool,
    pub subject: String,
    pub rest: String,
    /// The badge and the name, after the rest.
    pub browser: Option<(String, String)>,
}

/// The label for a browser `rules.json` names (a bundle id or a name), as
/// this machine lists it, with `profile` when it is not empty. A browser
/// not found here is shown as written.
pub fn label_for(browsers: &[BrowserChoice], value: &str, profile: &str) -> BrowserLabel {
    let mut label = browsers
        .iter()
        .find(|choice| {
            choice.value.eq_ignore_ascii_case(value)
                || choice.label.name.eq_ignore_ascii_case(value)
        })
        .map_or_else(
            || BrowserLabel::new(&badge_for(value), value, None),
            |choice| choice.label.clone(),
        );
    label.profile = Some(profile.to_owned()).filter(|profile| !profile.is_empty());
    label
}

/// "1 rule", "3 rules".
pub fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

/// "2 rewrites and 5 handlers", leaving out a kind the config has none of.
fn holds(config: &FinickyConfig) -> String {
    let rewrites = plural(config.rewrites, "rewrite", "rewrites");
    let handlers = plural(config.handlers, "handler", "handlers");
    match (config.rewrites, config.handlers) {
        (0, _) => handlers,
        (_, 0) => rewrites,
        _ => format!("{rewrites} and {handlers}"),
    }
}

/// The machine, for a sentence: "this Mac".
fn this_machine() -> &'static str {
    if cfg!(target_os = "macos") {
        "this Mac"
    } else if cfg!(target_os = "windows") {
        "this PC"
    } else {
        "this computer"
    }
}

/// Who answers the default-browser request, to start a sentence.
fn os_subject() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "Your desktop"
    }
}

/// The default-browser step while the OS has the question. Windows answers
/// in its Settings, where the reader has to finish (Settings › Browsers
/// says the same).
fn waiting_line() -> &'static str {
    if cfg!(target_os = "macos") {
        "Waiting for macOS\u{2026}"
    } else if cfg!(target_os = "windows") {
        "Finish in Windows Settings: select Set default on PwrRoute\u{2019}s page."
    } else {
        "Waiting for your desktop\u{2026}"
    }
}

/// Where the app lives once the window is gone, for the done help line.
fn tray_line() -> &'static str {
    if cfg!(target_os = "macos") {
        "The menu bar icon shows recent links and can pause routing."
    } else if cfg!(target_os = "windows") {
        "The notification area icon shows recent links and can pause routing."
    } else {
        "The tray icon shows recent links and can pause routing."
    }
}

type Hook<T> = Rc<dyn Fn(T, &mut App)>;
/// The config step's answer, and how an import it starts answers back.
type ImportHook = Rc<dyn Fn(ImportChoice, ImportReply, &mut App)>;
/// A hook that answers: the browsers read again, or why a request failed.
type Ask<T> = Rc<dyn Fn(&mut App) -> T>;

/// How the window was left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Finish {
    /// "Open PwrRoute", once every step is done.
    Done,
    /// "Later", before that.
    Later,
}

/// How the window was left, and where it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Finished {
    pub how: Finish,
    /// The display the window was on, for the main window to open on.
    pub display: Option<DisplayId>,
}

/// What the window's choices do. The window itself changes nothing.
#[derive(Clone)]
pub struct FirstRunHooks {
    /// The Everything else browser, or a profile of it, was picked: by the
    /// reader, or by the window's preselection.
    pub on_fallback: Hook<Fallback>,
    /// The segmented control: who owns everything else while a config is
    /// in charge (`rules.json`'s `precedence`).
    pub on_precedence: Hook<Precedence>,
    /// The config step's answer. Keep and Don't use it are done at once;
    /// Import opens the importer, and the reply says how it was left.
    pub on_import: ImportHook,
    /// "Look again": the browsers on this machine, read again.
    pub on_rescan: Ask<Vec<BrowserChoice>>,
    /// "Set as default browser". Only this asks the OS anything. An error
    /// is the OS refusing the request outright; a refused prompt is not.
    pub on_set_default_browser: Ask<Result<(), String>>,
    /// "Open PwrRoute" or "Later": first run is over, and the window closes
    /// right after. The app marks it done and shows the main window, on
    /// the display this window was on.
    pub on_finish: Hook<Finished>,
}

/// Open the first-run window.
pub fn open(
    model: FirstRunModel,
    hooks: FirstRunHooks,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<FirstRun>> {
    let display = crate::placement::display_for_new_window(cx);
    let bounds = Bounds::centered(display, size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)), cx);
    let window = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            display_id: display,
            titlebar: Some(TitlebarOptions {
                title: Some("PwrRoute".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(
                    px(crate::TRAFFIC_LIGHTS.0),
                    px(crate::TRAFFIC_LIGHTS.1),
                )),
            }),
            app_id: Some("pwrroute".to_owned()),
            window_min_size: Some(size(px(WINDOW_SIZE.0), px(560.0))),
            ..Default::default()
        },
        |_, cx| cx.new(|cx| FirstRun::new(model, hooks, cx)),
    )?;
    window.update(cx, |this, window, cx| window.focus(&this.focus, cx))?;
    cx.activate(true);
    Ok(window)
}

pub struct FirstRun {
    pub model: FirstRunModel,
    hooks: FirstRunHooks,
    pub focus: FocusHandle,
    /// The Everything else browser list, or the chosen browser's profile
    /// list, is open.
    pub picking: Picking,
    /// What was open when the press began: a press on the open select's
    /// field closes its list (the press outside it) and the click must not
    /// open it again.
    was_picking: Picking,
}

impl FirstRun {
    pub fn new(model: FirstRunModel, hooks: FirstRunHooks, cx: &mut Context<'_, Self>) -> Self {
        Self {
            model,
            hooks,
            focus: cx.focus_handle(),
            picking: Picking::None,
            was_picking: Picking::None,
        }
    }

    /// With nothing chosen for Everything else, picks the browser
    /// [`FirstRunModel::preselection`] names and saves it at once, so the
    /// step starts done and stays editable.
    pub fn preselect(&mut self, cx: &mut Context<'_, Self>) {
        if self.model.fallback.is_some() {
            return;
        }
        let Some((fallback, picked)) = self.model.preselection() else {
            return;
        };
        let mut label = fallback.browser.label.clone();
        label.profile.clone_from(&fallback.profile);
        self.model.fallback = Some(label);
        self.model.picked = Some(picked);
        (self.hooks.on_fallback)(fallback, cx);
        cx.notify();
    }

    fn toggle_picker(&mut self, which: Picking, cx: &mut Context<'_, Self>) {
        self.picking = if self.was_picking == which {
            Picking::None
        } else {
            which
        };
        cx.notify();
    }

    /// A browser from the Everything else list, at its default profile.
    fn choose_fallback(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        self.picking = Picking::None;
        if let Some(choice) = self.model.browsers.get(index).cloned() {
            self.model.fallback = Some(choice.label.clone());
            self.model.picked = None;
            (self.hooks.on_fallback)(
                Fallback {
                    browser: choice,
                    profile: None,
                },
                cx,
            );
        }
        cx.notify();
    }

    /// A profile from the profile list: 0 is "Default profile", then the
    /// chosen browser's profiles in order.
    fn choose_profile(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        self.picking = Picking::None;
        let Some(browser) = self.model.fallback_choice().cloned() else {
            cx.notify();
            return;
        };
        let profile = match index {
            0 => None,
            n => match browser.profiles.get(n - 1) {
                Some(profile) => Some(profile.clone()),
                None => {
                    cx.notify();
                    return;
                }
            },
        };
        if let Some(fallback) = &mut self.model.fallback {
            fallback.profile.clone_from(&profile);
        }
        self.model.picked = None;
        (self.hooks.on_fallback)(Fallback { browser, profile }, cx);
        cx.notify();
    }

    /// The segmented control: who owns everything else.
    fn choose_owner(&mut self, precedence: Precedence, cx: &mut Context<'_, Self>) {
        if self.model.precedence == precedence {
            return;
        }
        self.model.precedence = precedence;
        (self.hooks.on_precedence)(precedence, cx);
        cx.notify();
    }

    /// "Look again": read the machine's browsers again, and pick one when
    /// there is one now.
    fn look_again(&mut self, cx: &mut Context<'_, Self>) {
        self.model.browsers = (self.hooks.on_rescan)(cx);
        self.preselect(cx);
        cx.notify();
    }

    /// The OS's answer on who handles links, as it changes, and whether a
    /// request is still out (`waiting`).
    pub fn set_default_state(
        &mut self,
        state: &DefaultState,
        waiting: bool,
        cx: &mut Context<'_, Self>,
    ) {
        let is_default = state.is_default();
        let current = match state {
            DefaultState::Other(_) => Some(state.current_name()),
            DefaultState::PwrRoute | DefaultState::Unknown => None,
        };
        if self.model.is_default == is_default
            && self.model.current_default == current
            && self.model.waiting == waiting
        {
            return;
        }
        self.model.is_default = is_default;
        self.model.current_default = current;
        self.model.waiting = waiting;
        if is_default {
            self.model.default_error = None;
        }
        cx.notify();
    }

    /// The config step's buttons. Import waits for the importer: the step
    /// shows "Importing…" until it reports how it was left.
    fn choose(&mut self, choice: ImportChoice, window: &Window, cx: &mut Context<'_, Self>) {
        if self.model.state_of(Step::Config) != StepState::Current || self.model.importing {
            return;
        }
        let reply = self.import_reply(window, cx);
        match choice {
            ImportChoice::Import => self.model.importing = true,
            ImportChoice::KeepFile | ImportChoice::DontUse => self.model.import = Some(choice),
        }
        (self.hooks.on_import)(choice, reply, cx);
        cx.notify();
    }

    /// The reply an import answers through: it updates this view and brings
    /// its window back to the front, if both are still there.
    fn import_reply(&self, window: &Window, cx: &mut Context<'_, Self>) -> ImportReply {
        let view = cx.entity().downgrade();
        let handle = window.window_handle();
        Rc::new(move |outcome: ImportOutcome, cx: &mut App| {
            if view
                .update(cx, |this, cx| this.import_finished(outcome, cx))
                .is_ok()
            {
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, _| window.activate_window());
                });
            }
        })
    }

    /// How the importer was left. Cancel brings the step back as it was; a
    /// finished import marks it done with the count written.
    pub fn import_finished(&mut self, outcome: ImportOutcome, cx: &mut Context<'_, Self>) {
        if !self.model.importing {
            return;
        }
        self.model.importing = false;
        if let ImportOutcome::Imported(done) = outcome {
            if !done.default_browser.is_empty() {
                let label = label_for(
                    &self.model.browsers,
                    &done.default_browser,
                    &done.default_profile,
                );
                if self.model.fallback.as_ref() != Some(&label) {
                    self.model.fallback = Some(label);
                    self.model.picked = None;
                }
            }
            self.model.rules = done.rules;
            self.model.import = Some(ImportChoice::Import);
            self.model.imported = Some(done);
        }
        cx.notify();
    }

    fn set_default(&mut self, cx: &mut Context<'_, Self>) {
        if self.model.state_of(Step::MakeDefault) != StepState::Current || self.model.waiting {
            return;
        }
        match (self.hooks.on_set_default_browser)(cx) {
            Ok(()) => {
                self.model.default_requested = true;
                self.model.default_error = None;
            }
            Err(error) => self.model.default_error = Some(error),
        }
        cx.notify();
    }

    /// "Open PwrRoute" or "Later": tell the app, and which display this
    /// window is on, then close.
    fn finish(&mut self, how: Finish, window: &mut Window, cx: &mut Context<'_, Self>) {
        let display = window.display(cx).map(|display| display.id());
        (self.hooks.on_finish)(Finished { how, display }, cx);
        window.remove_window();
    }
}

impl Render for FirstRun {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let model = &self.model;
        let complete = model.complete();
        let steps = model.steps();
        let header = if complete {
            done_header(model).into_any_element()
        } else {
            header(model).into_any_element()
        };
        let body = if complete {
            div()
                .flex()
                .flex_col()
                .gap(px(16.0))
                .child(where_links_go(model))
                .child(
                    div().flex().flex_col().gap(px(10.0)).children(
                        steps
                            .iter()
                            .enumerate()
                            .map(|(index, (step, _))| folded_step(model, *step, index)),
                    ),
                )
        } else {
            let numbered = steps.iter().enumerate().map(|(index, (step, state))| {
                let number = index + 1;
                match step {
                    Step::Config => step_card(
                        number,
                        config_look(model, *state),
                        "Bring your Finicky config",
                        config_body(model, *state, cx),
                    ),
                    Step::EverythingElse => step_card(
                        number,
                        Look::from(*state),
                        "Where should everything else go?",
                        everything_else_body(model, *state, self.picking, cx),
                    ),
                    Step::MakeDefault => step_card(
                        number,
                        Look::from(*state),
                        "Make PwrRoute the default browser",
                        default_body(model, *state, cx),
                    ),
                }
            });
            div().flex().flex_col().gap(px(10.0)).children(numbered)
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
            .child(strip(model.routing()))
            .child(
                div()
                    .id("first-run-body")
                    .debug_selector(|| "first-run-body".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .min_h_full()
                            .flex()
                            .flex_col()
                            .gap(px(16.0))
                            .pt(px(26.0))
                            .px(px(28.0))
                            .pb(px(22.0))
                            .child(header)
                            .child(body)
                            .child(footer(model, cx)),
                    ),
            )
    }
}

// --- motion ----------------------------------------------------------------

/// `ease-out`, cubic.
fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// `a` to `b` by `t`, through RGB so grey to green never passes another
/// hue on the way.
fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (a, b) = (Rgba::from(a), Rgba::from(b));
    let at = |x: f32, y: f32| x + (y - x) * t;
    Rgba {
        r: at(a.r, b.r),
        g: at(a.g, b.g),
        b: at(a.b, b.b),
        a: at(a.a, b.a),
    }
    .into()
}

/// The same color at `alpha` of its own opacity.
fn faded(color: Hsla, alpha: f32) -> Hsla {
    Hsla {
        a: color.a * alpha,
        ..color
    }
}

/// An accent glow: `0 0 blur spread rgba(255,138,31,alpha)`.
fn accent_shadow(blur: f32, spread: f32, alpha: f32) -> BoxShadow {
    BoxShadow {
        color: Hsla {
            a: alpha,
            ..theme::accent()
        },
        offset: point(px(0.0), px(0.0)),
        blur_radius: px(blur),
        spread_radius: px(spread),
        inset: false,
    }
}

/// Fades `element` in over `ms` the first time it is drawn under `id`: the
/// heading's 150ms cross-fade and the folded steps' 160ms.
fn fade_in<E: IntoElement + gpui_kit::Styled + 'static>(
    element: E,
    id: impl Into<gpui_kit::ElementId>,
    ms: u64,
) -> impl IntoElement {
    element.with_animation(
        id,
        Animation::new(Duration::from_millis(ms)).with_easing(ease_out),
        |element, t| element.opacity(t),
    )
}

/// A ✓ in its circle, scaling 0.6 → 1 and fading in over 180ms, `delay`
/// after it is first drawn.
fn tick(
    id: impl Into<gpui_kit::ElementId>,
    diameter: f32,
    glyph: f32,
    delay: Duration,
) -> impl IntoElement {
    let pop = Animation::new(Duration::from_millis(180)).with_easing(ease_out);
    let draw = move |t: f32| {
        let scale = 0.6 + 0.4 * t;
        div()
            .size(px(diameter * scale))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(theme::success_soft())
            .opacity(t)
            .child(mono(
                "\u{2713}",
                glyph * scale,
                600.0,
                theme::success_text(),
            ))
    };
    let frame = div()
        .size(px(diameter))
        .flex_none()
        .flex()
        .items_center()
        .justify_center();
    if delay.is_zero() {
        frame
            .with_animations(id, vec![pop], move |frame, _, t| frame.child(draw(t)))
            .into_any_element()
    } else {
        frame
            .with_animations(
                id,
                vec![Animation::new(delay), pop],
                move |frame, step, t| frame.child(draw(if step == 0 { 0.0 } else { t })),
            )
            .into_any_element()
    }
}

/// `.spin`: the importer is open. A quarter arc turning on a faint ring.
fn spinner(color: Hsla) -> impl IntoElement {
    div().size(px(12.0)).flex_none().with_animation(
        "first-run-spinner",
        Animation::new(Duration::from_millis(900)).repeat(),
        move |frame, t| {
            frame.child(
                canvas(
                    |_, _, _| {},
                    move |bounds, (), window, _| {
                        let center = bounds.center();
                        let radius = 5.0;
                        let arc = |start: f32, sweep: f32| {
                            let mut path = PathBuilder::stroke(px(2.0));
                            let steps = 16;
                            for i in 0..=steps {
                                let angle = start + sweep * (i as f32 / steps as f32);
                                let at = point(
                                    center.x + px(radius * angle.cos()),
                                    center.y + px(radius * angle.sin()),
                                );
                                if i == 0 {
                                    path.move_to(at);
                                } else {
                                    path.line_to(at);
                                }
                            }
                            path.build().ok()
                        };
                        if let Some(ring) = arc(0.0, TAU) {
                            window.paint_path(ring, faded(color, 0.3));
                        }
                        if let Some(head) = arc(TAU * t - PI / 2.0, PI / 2.0) {
                            window.paint_path(head, color);
                        }
                    },
                )
                .size_full(),
            )
        },
    )
}

// --- chrome ------------------------------------------------------------------

/// `.strip` for this window: the brand and whether links route yet. The
/// chip's dot fades from grey to green over 200ms when routing starts.
fn strip(routing: bool) -> Div {
    let wordmark = StyledText::new("PwrRoute").with_highlights([(
        3..8,
        HighlightStyle {
            color: Some(theme::accent()),
            ..Default::default()
        },
    )]);
    let label = if routing {
        "routing"
    } else {
        "not routing yet"
    };
    let dot = if routing {
        theme::dot(theme::status_ok())
            .with_animation(
                "first-run-chip-routing",
                Animation::new(Duration::from_millis(200)).with_easing(ease_out),
                |dot, t| dot.bg(mix(theme::status_suspended(), theme::status_ok(), t)),
            )
            .into_any_element()
    } else {
        theme::dot(theme::status_suspended()).into_any_element()
    };
    div()
        .h(px(STRIP_HEIGHT + 1.0))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(12.0))
        .pl(px(BRAND_X))
        .pr(px(16.0))
        .bg(theme::bg_titlebar())
        .border_b_1()
        .border_color(theme::border_subtle())
        .child(icon("pwrroute/mark.svg", 20.0, theme::accent()))
        .child(
            div()
                .font_family(theme::SANS)
                .font_weight(theme::weight(700.0))
                .text_size(px(17.0))
                .line_height(px(17.0))
                .whitespace_nowrap()
                .child(wordmark),
        )
        .child(div().flex_1())
        .child(
            div()
                .debug_selector(|| "first-run-chip".into())
                .flex()
                .flex_none()
                .items_center()
                .gap(px(6.0))
                .h(px(24.0))
                .px(px(9.0))
                .border_1()
                .border_color(theme::border_subtle())
                .rounded_full()
                .bg(theme::bg_elevated())
                .child(dot)
                .child(mono(label, 10.5, 600.0, theme::text_muted())),
        )
}

/// The eyebrow, the title and the help line, before routing starts.
fn header(model: &FirstRunModel) -> impl IntoElement {
    fade_in(
        div()
            .flex()
            .flex_col()
            .child(tracked("Set up", 9.5, 600.0, 0.14, theme::accent()))
            .child(
                text(model.heading(), 21.0, 700.0, theme::text_primary())
                    .debug_selector(|| "first-run-heading".into())
                    .line_height(px(25.2))
                    .mt(px(6.0)),
            )
            .child(paragraph(model.help()).mt(px(6.0)).max_w(px(470.0))),
        "first-run-heading",
        150,
    )
}

/// `.frhero` (4g): the orb, which pings once as it arrives, and the outcome
/// as the title.
fn done_header(model: &FirstRunModel) -> impl IntoElement {
    let rest = [accent_shadow(0.0, 4.0, 0.06), accent_shadow(28.0, 0.0, 0.2)];
    let orb = div()
        .size(px(56.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(14.0))
        .bg(theme::accent_soft())
        .border_1()
        .border_color(theme::accent_border())
        .shadow(rest.to_vec())
        .child(icon("pwrroute/mark.svg", 30.0, theme::accent()))
        .with_animation(
            "first-run-orb-ping",
            Animation::new(Duration::from_millis(600)).with_easing(ease_out),
            move |orb, t| {
                let mut shadows = rest.to_vec();
                shadows.push(accent_shadow(0.0, 4.0 + 10.0 * t, 0.3 * (1.0 - t)));
                orb.shadow(shadows)
            },
        );
    div()
        .flex()
        .items_center()
        .gap(px(16.0))
        .child(orb)
        .child(fade_in(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(tracked("Set up", 9.5, 600.0, 0.14, theme::accent()))
                .child(
                    text(model.heading(), 24.0, 700.0, theme::text_primary())
                        .debug_selector(|| "first-run-heading".into())
                        .line_height(px(28.8))
                        .mt(px(6.0)),
                )
                .child(paragraph(model.help()).mt(px(6.0)).max_w(px(420.0))),
            "first-run-heading-done",
            150,
        ))
}

/// `.card.wl`: Where links go, at most three lines, their ticks arriving
/// one after another.
fn where_links_go(model: &FirstRunModel) -> Div {
    let lines = model.where_links_go();
    let last = lines.len().saturating_sub(1);
    let rows = lines.into_iter().enumerate().map(|(index, line)| {
        let marker = if line.done {
            let delay = Duration::from_millis(60 * index as u64);
            tick(("first-run-where-tick", index), 18.0, 10.0, delay).into_any_element()
        } else {
            neutral_dot().into_any_element()
        };
        div()
            .debug_selector(move || format!("first-run-where-{index}"))
            .flex()
            .items_center()
            .gap(px(10.0))
            .py(px(10.0))
            .px(px(16.0))
            .when(index < last, |row| {
                row.border_b_1().border_color(theme::border_faint())
            })
            .child(marker)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .overflow_hidden()
                    .child(text(line.subject, 12.0, 600.0, theme::text_primary()))
                    .child(text(line.rest, 12.0, 500.0, theme::text_secondary()))
                    .when_some(line.browser, |row, (letters, name)| {
                        row.child(text(" \u{00b7} ", 12.0, 500.0, theme::text_secondary()))
                            .child(badge(letters).mr(px(5.0)))
                            .child(text(name, 12.0, 500.0, theme::text_secondary()))
                    }),
            )
    });
    div()
        .debug_selector(|| "first-run-where".into())
        .flex_none()
        .flex()
        .flex_col()
        .border_1()
        .border_color(theme::accent_border())
        .rounded(px(10.0))
        .bg(theme::bg_panel())
        .shadow(vec![accent_shadow(0.0, 3.0, 0.06)])
        .child(
            div()
                .py(px(11.0))
                .px(px(16.0))
                .border_b_1()
                .border_color(theme::border_faint())
                .child(tracked(
                    "Where links go",
                    9.5,
                    600.0,
                    0.14,
                    theme::text_muted(),
                )),
        )
        .children(rows)
}

/// The 18px outlined dot for a line with nothing in it yet.
fn neutral_dot() -> Div {
    div()
        .size(px(18.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(theme::border_default())
        .bg(theme::bg_elevated())
        .child(div().size(px(6.0)).rounded_full().bg(theme::text_muted()))
}

/// `.step.done.mini`: a finished step on one line, once routing starts.
fn folded_step(model: &FirstRunModel, step: Step, index: usize) -> impl IntoElement {
    let (title, result) = model.folded(step);
    fade_in(
        div()
            .debug_selector(move || format!("first-run-folded-{}", index + 1))
            .flex()
            .items_center()
            .gap(px(12.0))
            .py(px(10.0))
            .px(px(14.0))
            .border_1()
            .border_color(theme::border_subtle())
            .rounded(px(10.0))
            .bg(theme::bg_panel())
            .child(
                div()
                    .size(px(20.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(theme::success_soft())
                    .child(mono("\u{2713}", 10.0, 600.0, theme::success_text())),
            )
            .child(text(title, 12.0, 600.0, theme::text_primary()))
            .child(crate::widgets::one_line(
                result,
                theme::SANS,
                400.0,
                11.0,
                14.0,
                theme::text_muted(),
            )),
        ("first-run-folded", index),
        160,
    )
}

/// The footer: where to change this later, and the way out.
fn footer(model: &FirstRunModel, cx: &mut Context<'_, FirstRun>) -> Div {
    div()
        .mt_auto()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(10.0))
        .child(
            text(
                "Change any of this later in Settings \u{203a} Browsers and Configuration.",
                10.5,
                400.0,
                theme::text_subtle(),
            )
            .line_height(px(15.75)),
        )
        .child(finish_button(model, cx))
}

// --- steps -------------------------------------------------------------------

/// How a step is drawn: its state, or one of the config step's two
/// variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Look {
    Done,
    Current,
    Todo,
    /// The importer is open: a spinner in the accent circle (4b).
    Importing,
    /// The config does not load: `.step.warnc` (4c).
    Broken,
}

impl From<StepState> for Look {
    fn from(state: StepState) -> Self {
        match state {
            StepState::Done => Self::Done,
            StepState::Current => Self::Current,
            StepState::Todo => Self::Todo,
        }
    }
}

/// The config step's look, from its state and the config.
fn config_look(model: &FirstRunModel, state: StepState) -> Look {
    match state {
        StepState::Current if model.importing => Look::Importing,
        StepState::Current
            if model
                .finicky
                .as_ref()
                .is_some_and(|config| config.error.is_some()) =>
        {
            Look::Broken
        }
        state => Look::from(state),
    }
}

/// `.step`: the number (a ✓ once done, a spinner while importing), the
/// title, and what goes under it.
fn step_card(number: usize, look: Look, title: &'static str, body: Div) -> Div {
    let marker = match look {
        Look::Done => {
            tick(("first-run-step-tick", number), 26.0, 11.0, Duration::ZERO).into_any_element()
        }
        Look::Current => number_circle(number, theme::accent(), theme::accent_on(), None),
        Look::Todo => number_circle(
            number,
            theme::bg_elevated(),
            theme::text_muted(),
            Some(theme::border_default()),
        ),
        Look::Broken => number_circle(number, theme::danger_soft(), theme::danger_text(), None),
        Look::Importing => div()
            .debug_selector(|| "first-run-importing".into())
            .size(px(26.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .bg(theme::accent())
            .child(spinner(theme::accent_on()))
            .into_any_element(),
    };
    div()
        .debug_selector(move || format!("first-run-step-{number}"))
        .flex()
        .gap(px(14.0))
        .py(px(14.0))
        .px(px(16.0))
        .border_1()
        .rounded(px(10.0))
        .map(|step| match look {
            Look::Current | Look::Importing => step
                .border_color(theme::accent_border())
                .bg(theme::bg_row_active()),
            Look::Broken => step
                .border_color(faded(theme::status_error(), 0.5))
                .bg(faded(theme::danger_soft(), 0.33)),
            Look::Done => step
                .border_color(theme::border_subtle())
                .bg(theme::bg_panel()),
            Look::Todo => step
                .border_color(theme::border_subtle())
                .bg(theme::bg_panel())
                .opacity(0.6),
        })
        .child(div().w(px(28.0)).flex_none().child(marker))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(text(title, 13.0, 600.0, theme::text_primary()).line_height(px(16.9)))
                .child(body),
        )
}

/// A step's 26px number.
fn number_circle(
    number: usize,
    fill: Hsla,
    color: Hsla,
    border: Option<Hsla>,
) -> gpui_kit::AnyElement {
    div()
        .debug_selector(move || format!("first-run-number-{number}"))
        .size(px(26.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(border.unwrap_or(theme::transparent()))
        .bg(fill)
        .child(mono(number.to_string(), 11.0, 600.0, color))
        .into_any_element()
}

/// `.step .sb p`: 11.5px at 1.55, wrapping.
fn paragraph(content: impl Into<SharedString>) -> Div {
    paragraph_box().child(content.into())
}

fn paragraph_box() -> Div {
    div()
        .font_family(theme::SANS)
        .font_weight(theme::weight(400.0))
        .text_size(px(11.5))
        .line_height(px(17.8))
        .text_color(theme::text_muted())
}

/// A "Found ~/.finicky.ts…" paragraph, the path set off as the design's
/// `.tag`.
fn tagged_paragraph(detail: String, path: &str) -> Div {
    let tag = detail
        .starts_with("Found ")
        .then(|| detail.find(path).map(|at| at..at + path.len()))
        .flatten();
    let styled = StyledText::new(detail.clone()).with_highlights(tag.map(|range| {
        (
            range,
            HighlightStyle {
                color: Some(theme::text_secondary()),
                background_color: Some(theme::bg_elevated()),
                ..Default::default()
            },
        )
    }));
    paragraph_box().child(styled)
}

/// `.det`: the 10.5px line under a step's controls.
fn detail_line(content: impl Into<SharedString>) -> Div {
    div()
        .font_family(theme::SANS)
        .text_size(px(10.5))
        .line_height(px(15.75))
        .text_color(theme::text_subtle())
        .child(content.into())
}

/// The config step (4a-4c): what was found and the three answers, or what
/// the answer did.
fn config_body(model: &FirstRunModel, state: StepState, cx: &mut Context<'_, FirstRun>) -> Div {
    let Some(config) = &model.finicky else {
        return div();
    };
    let body = div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(tagged_paragraph(model.config_detail(), &config.path));
    if state != StepState::Current || model.importing {
        return body;
    }
    let pick = |choice| {
        cx.listener(move |this: &mut FirstRun, _: &ClickEvent, window, cx| {
            this.choose(choice, window, cx);
        })
    };
    let keep = format!("Keep using {}", config.path);
    let row = div().flex().flex_wrap().items_center().gap(px(8.0));
    let row = if config.error.is_some() {
        row.child(
            button("first-run-keep", keep, Kind::Default).on_click(pick(ImportChoice::KeepFile)),
        )
    } else if config.importable == 0 {
        row.child(
            button("first-run-keep", keep, Kind::Primary).on_click(pick(ImportChoice::KeepFile)),
        )
    } else {
        row.child(
            button(
                "first-run-import",
                format!("Import {}", plural(config.importable, "rule", "rules")),
                Kind::Primary,
            )
            .on_click(pick(ImportChoice::Import)),
        )
        .child(button("first-run-keep", keep, Kind::Default).on_click(pick(ImportChoice::KeepFile)))
    };
    let row = row.child(
        button("first-run-dont-use", "Don't use it", Kind::Quiet)
            .on_click(pick(ImportChoice::DontUse)),
    );
    body.when_some(config.error.clone(), |body, error| {
        body.child(error_line(error))
    })
    .child(row)
}

/// `.errl`: the first line of why the config does not load.
fn error_line(error: String) -> Div {
    div()
        .debug_selector(|| "first-run-config-error".into())
        .flex()
        .child(
            div()
                .py(px(6.0))
                .px(px(8.0))
                .rounded(px(5.0))
                .bg(theme::danger_soft())
                .font_family(theme::mono_family(500.0))
                .font_weight(theme::weight(500.0))
                .text_size(px(10.5))
                .line_height(px(14.7))
                .text_color(theme::danger_text())
                .child(error),
        )
}

/// The Everything else step (4d-4f, 4h): who owns it while a config is in
/// charge, the browser and profile, and where the pick came from.
fn everything_else_body(
    model: &FirstRunModel,
    state: StepState,
    picking: Picking,
    cx: &mut Context<'_, FirstRun>,
) -> Div {
    let body = div().flex().flex_col().gap(px(8.0));
    if model.browsers.is_empty() {
        return body.child(paragraph(model.everything_else_detail())).when(
            state != StepState::Todo,
            |body| {
                body.child(
                    div().flex().child(
                        button("first-run-look-again", "Look again", Kind::Primary).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.look_again(cx)),
                        ),
                    ),
                )
            },
        );
    }
    if state == StepState::Todo {
        return body.child(paragraph(model.everything_else_detail()));
    }
    let owner = model.config_in_charge().then(|| {
        let option = |id: &'static str, label: String, precedence: Precedence| {
            segment(label, model.precedence == precedence)
                .id(id)
                .debug_selector(move || id.into())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.choose_owner(precedence, cx);
                }))
        };
        div().flex().child(
            segmented()
                .child(option(
                    "first-run-owner-config",
                    model.config_owner_label(),
                    Precedence::Config,
                ))
                .child(option(
                    "first-run-owner-pwrroute",
                    "PwrRoute's default".to_owned(),
                    Precedence::PwrRoute,
                )),
        )
    });
    body.when_some(owner, |body, owner| body.child(owner))
        .child(paragraph(model.everything_else_detail()))
        .child(fallback_pickers(model, picking, cx))
        .when_some(model.picked_note(), |body, note| {
            body.child(detail_line(note).debug_selector(|| "first-run-picked".into()))
        })
}

/// The element id of the Everything else browser select.
pub const FALLBACK_ID: &str = "first-run-fallback";
/// The element id of its profile select.
pub const PROFILE_ID: &str = "first-run-profile";

/// What the profile select says for no profile: the browser opens its own.
const DEFAULT_PROFILE: &str = "Default profile";

/// The browser select's width, and the profile select's: room for
/// "Default profile" and a Firefox profile name beside it.
const BROWSER_WIDTH: f32 = 300.0;
const PROFILE_WIDTH: f32 = 180.0;
/// `.inp`'s height, which the lists open under.
const FIELD_HEIGHT: f32 = 30.0;

/// The browser select and, when the chosen browser has profiles, the
/// profile select beside it. Their lists open as popovers.
fn fallback_pickers(
    model: &FirstRunModel,
    picking: Picking,
    cx: &mut Context<'_, FirstRun>,
) -> Div {
    let profiles = model.fallback_profiles();
    let row = div()
        .flex()
        .items_start()
        .gap(px(8.0))
        .child(browser_picker(
            model,
            !profiles.is_empty(),
            picking == Picking::Browser,
            cx,
        ));
    if profiles.is_empty() {
        return row;
    }
    row.child(profile_picker(
        model,
        profiles,
        picking == Picking::Profile,
        cx,
    ))
}

/// A select's field: `.inp` with its value and caret. A press on it notes
/// what was open, so its click toggles rather than reopening the list the
/// press just closed.
fn select_field(
    id: &'static str,
    width: f32,
    which: Picking,
    cx: &mut Context<'_, FirstRun>,
) -> Stateful<Div> {
    input_box()
        .id(id)
        .debug_selector(move || id.into())
        .w(px(width))
        .cursor_pointer()
        .capture_any_mouse_down(cx.listener(|this, _, _, _| {
            this.was_picking = this.picking;
        }))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.toggle_picker(which, cx);
        }))
}

/// The list a select opens: `widgets::menu_card`, as wide as its field,
/// in a popover under it that stays on screen.
fn popover(
    id: &'static str,
    width: f32,
    rows: Vec<Stateful<Div>>,
    cx: &mut Context<'_, FirstRun>,
) -> impl IntoElement {
    deferred(
        anchored()
            // Under the field: an anchored element starts at its parent's
            // top left, and `.inp` is 30 high.
            .offset(point(px(0.0), px(FIELD_HEIGHT + 4.0)))
            .snap_to_window_with_margin(px(8.0))
            .child(
                menu_card(id)
                    .debug_selector(move || id.into())
                    .w(px(width))
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.picking = Picking::None;
                        cx.notify();
                    }))
                    .children(rows),
            ),
    )
    .with_priority(1)
}

/// The browser select: the browsers found on this machine, one row each,
/// with their profiles as the hint. With a profile select beside it, it
/// shows the name alone.
fn browser_picker(
    model: &FirstRunModel,
    profile_beside: bool,
    open: bool,
    cx: &mut Context<'_, FirstRun>,
) -> Div {
    let field = select_field(FALLBACK_ID, BROWSER_WIDTH, Picking::Browser, cx);
    let field = match &model.fallback {
        Some(browser) => {
            let shown = if profile_beside {
                browser.name.clone()
            } else {
                browser.display()
            };
            field
                .child(badge(browser.badge.clone()))
                .child(field_label(shown, theme::text_primary()))
                .child(caret())
        }
        None => field
            .child(text("Choose a browser", 12.5, 400.0, theme::text_subtle()))
            .child(caret()),
    };
    let column = div().flex().flex_col().child(field);
    if !open {
        return column;
    }
    let rows = model
        .browsers
        .iter()
        .enumerate()
        .map(|(index, choice)| {
            menu_row(
                ("first-run-browser", index),
                Some(choice.label.badge.clone().into()),
                choice.label.name.clone(),
                profiles_hint(&choice.profiles).map(Into::into),
            )
            .debug_selector(move || format!("first-run-browser-{index}"))
            .hover(|row| row.bg(theme::bg_hover()))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.choose_fallback(index, cx);
            }))
        })
        .collect();
    column.child(popover("first-run-browser-menu", BROWSER_WIDTH, rows, cx))
}

/// The profile select: "Default profile", then the chosen browser's
/// profiles.
fn profile_picker(
    model: &FirstRunModel,
    profiles: &[String],
    open: bool,
    cx: &mut Context<'_, FirstRun>,
) -> Div {
    let chosen = model
        .fallback
        .as_ref()
        .and_then(|fallback| fallback.profile.clone());
    let label = match chosen {
        Some(profile) => field_label(profile, theme::text_primary()),
        None => text(DEFAULT_PROFILE, 12.5, 400.0, theme::text_subtle()),
    };
    let field = select_field(PROFILE_ID, PROFILE_WIDTH, Picking::Profile, cx)
        .child(label)
        .child(caret());
    let column = div().flex().flex_col().child(field);
    if !open {
        return column;
    }
    let names = std::iter::once(DEFAULT_PROFILE.to_owned()).chain(profiles.iter().cloned());
    let rows = names
        .enumerate()
        .map(|(index, name)| {
            menu_row(("first-run-profile", index), None, name, None)
                .debug_selector(move || format!("first-run-profile-{index}"))
                .hover(|row| row.bg(theme::bg_hover()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.choose_profile(index, cx);
                }))
        })
        .collect();
    column.child(popover("first-run-profile-menu", PROFILE_WIDTH, rows, cx))
}

/// The default-browser step (4a, 4d, 4i): what the request does, the
/// button while no request is out, what the OS said, and its error.
fn default_body(model: &FirstRunModel, state: StepState, cx: &mut Context<'_, FirstRun>) -> Div {
    let label = if model.default_requested {
        "Ask again"
    } else {
        "Set as default browser"
    };
    let kind = if state == StepState::Current {
        Kind::Primary
    } else {
        Kind::Default
    };
    let actions = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .when(state != StepState::Done && !model.waiting, |row| {
            row.child(
                button("first-run-default", label, kind)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.set_default(cx))),
            )
        })
        .when_some(model.default_note(), |row, note| {
            row.child(
                text(note, 11.0, 400.0, theme::text_subtle())
                    .debug_selector(|| "first-run-default-note".into()),
            )
        });
    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(paragraph(model.default_detail()))
        .when_some(model.default_warning(), |body, warning| {
            body.child(warning_note(warning))
        })
        .when(state != StepState::Done, |body| body.child(actions))
}

/// `.wnote`: a warning line with its "!" badge.
fn warning_note(content: String) -> Div {
    div()
        .debug_selector(|| "first-run-default-error".into())
        .flex()
        .items_start()
        .gap(px(6.0))
        .child(
            div()
                .mt(px(2.0))
                .size(px(14.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(theme::warn_soft())
                .child(text("!", 9.0, 700.0, theme::warn_text())),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_family(theme::SANS)
                .font_weight(theme::weight(500.0))
                .text_size(px(11.0))
                .line_height(px(16.5))
                .text_color(theme::warn_text())
                .child(content),
        )
}

/// The footer's button: "Open PwrRoute" once every step is done, "Later"
/// before that.
fn finish_button(model: &FirstRunModel, cx: &mut Context<'_, FirstRun>) -> impl IntoElement {
    if model.complete() {
        button("first-run-done", "Open PwrRoute", Kind::Primary).on_click(
            cx.listener(|this, _: &ClickEvent, window, cx| this.finish(Finish::Done, window, cx)),
        )
    } else {
        button("first-run-later", "Later", Kind::QuietSmall).on_click(
            cx.listener(|this, _: &ClickEvent, window, cx| this.finish(Finish::Later, window, cx)),
        )
    }
}

/// `.btn` and its variants.
#[derive(Clone, Copy)]
enum Kind {
    Primary,
    Default,
    Quiet,
    QuietSmall,
}

fn button(id: &'static str, label: impl Into<SharedString>, kind: Kind) -> Stateful<Div> {
    let (fill, border, color) = match kind {
        Kind::Primary => (theme::accent(), theme::transparent(), theme::accent_on()),
        Kind::Default => (
            theme::bg_elevated(),
            theme::border_default(),
            theme::text_primary(),
        ),
        Kind::Quiet | Kind::QuietSmall => (
            theme::transparent(),
            theme::transparent(),
            theme::text_secondary(),
        ),
    };
    let (py, px_, size) = match kind {
        Kind::QuietSmall => (5.0, 9.0, 11.0),
        _ => (7.0, 12.0, 11.5),
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

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use gpui_kit::{Entity, Modifiers, TestAppContext, VisualTestContext};

    use super::*;
    use StepState::{Current, Done, Todo};

    fn states(model: &FirstRunModel) -> Vec<StepState> {
        model.steps().into_iter().map(|(_, state)| state).collect()
    }

    /// The fixture without a config: two steps.
    fn plain() -> FirstRunModel {
        FirstRunModel {
            finicky: None,
            fallback: Some(BrowserLabel::new("Ch", "Google Chrome", Some("Personal"))),
            picked: Some(Picked::OsDefault),
            current_default: Some("Google Chrome".to_owned()),
            ..FirstRunModel::fixture()
        }
    }

    #[test]
    fn a_config_comes_first_and_the_steps_unlock_in_order() {
        let mut model = FirstRunModel::fixture();
        assert_eq!(
            model
                .steps()
                .into_iter()
                .map(|(step, _)| step)
                .collect::<Vec<_>>(),
            [Step::Config, Step::EverythingElse, Step::MakeDefault]
        );
        // Everything else is already picked, but waits for the config step.
        assert_eq!(states(&model), [Current, Todo, Todo]);
        assert_eq!(model.heading(), "Three steps, then links start routing");
        model.import = Some(ImportChoice::KeepFile);
        assert_eq!(states(&model), [Done, Done, Current]);
        model.is_default = true;
        assert_eq!(states(&model), [Done, Done, Done]);
        assert!(model.routing() && model.complete());
        assert_eq!(model.heading(), "Links are routing.");

        let mut fresh = plain();
        fresh.fallback = None;
        assert_eq!(
            fresh
                .steps()
                .into_iter()
                .map(|(step, _)| step)
                .collect::<Vec<_>>(),
            [Step::EverythingElse, Step::MakeDefault]
        );
        assert_eq!(states(&fresh), [Current, Todo]);
        assert_eq!(fresh.heading(), "Two steps, then links start routing");
        assert_eq!(states(&plain()), [Done, Current]);
    }

    #[test]
    fn the_header_says_what_is_left_and_then_what_happened() {
        let mut model = FirstRunModel::fixture();
        assert_eq!(
            model.help(),
            "Links keep opening the way they do now until the last step. Close this any time; \
             PwrRoute's Overview shows what's left."
        );
        model.import = Some(ImportChoice::KeepFile);
        model.is_default = true;
        assert!(
            model
                .help()
                .starts_with("Every link now goes where ~/.finicky.ts and your rules say. ")
        );
        model.import = Some(ImportChoice::DontUse);
        assert!(
            model
                .help()
                .starts_with("Every link now goes where your rules say. ")
        );
    }

    #[test]
    fn the_config_step_says_what_was_found_and_what_each_answer_did() {
        let mut model = FirstRunModel::fixture();
        assert_eq!(
            model.config_detail(),
            "Found ~/.finicky.ts: 2 rewrites and 5 handlers. Keep it running exactly as Finicky \
             did, or copy the 3 rules PwrRoute can show as visual rules. The file is never \
             changed."
        );
        model.importing = true;
        assert_eq!(
            model.config_detail(),
            "Importing\u{2026} Finish or cancel in the importer window. Cancel brings this step \
             back unchanged."
        );
        model.importing = false;
        model.import = Some(ImportChoice::Import);
        model.imported = Some(Imported {
            count: 3,
            keeps_config: true,
            ..Imported::default()
        });
        assert_eq!(
            model.config_detail(),
            "Imported 3 rules. ~/.finicky.ts is unchanged."
        );
        model.import = Some(ImportChoice::KeepFile);
        assert_eq!(
            model.config_detail(),
            "~/.finicky.ts stays in charge. Edit it as you do now; PwrRoute reloads it."
        );
        model.import = Some(ImportChoice::DontUse);
        assert_eq!(
            model.config_detail(),
            "PwrRoute won't read ~/.finicky.ts. Turn it back on in Settings \u{203a} \
             Configuration."
        );

        // Nothing importable, and a config that does not load.
        let mut nothing = FirstRunModel::fixture();
        nothing.finicky.as_mut().unwrap().importable = 0;
        assert_eq!(
            nothing.config_detail(),
            "Found ~/.finicky.ts: 2 rewrites and 5 handlers. They use JavaScript that visual \
             rules can't hold, so PwrRoute runs the file as Finicky did. The file is never \
             changed."
        );
        let mut broken = FirstRunModel::fixture();
        broken.finicky.as_mut().unwrap().error = Some("SyntaxError: x".to_owned());
        assert_eq!(
            broken.config_detail(),
            "Found ~/.finicky.ts, but it doesn't load. Keep it and PwrRoute uses it once it \
             loads; until then links go to the browser you pick in the next step."
        );
    }

    /// One of anything is singular, in every sentence that counts.
    #[test]
    fn counts_of_one_are_singular() {
        let mut model = FirstRunModel::fixture();
        let config = model.finicky.as_mut().unwrap();
        config.rewrites = 1;
        config.handlers = 1;
        config.importable = 1;
        assert!(
            model
                .config_detail()
                .starts_with("Found ~/.finicky.ts: 1 rewrite and 1 handler. ")
        );
        assert!(model.config_detail().contains("copy the 1 rule PwrRoute"));
        model.finicky.as_mut().unwrap().rewrites = 0;
        assert!(
            model
                .config_detail()
                .starts_with("Found ~/.finicky.ts: 1 handler. ")
        );
        model.import = Some(ImportChoice::Import);
        model.imported = Some(Imported {
            count: 1,
            keeps_config: true,
            ..Imported::default()
        });
        assert!(model.config_detail().starts_with("Imported 1 rule. "));
        assert_eq!(model.folded(Step::Config).1, "Imported 1 rule.");
        model.is_default = true;
        model.rules = 1;
        let lines = model.where_links_go();
        assert_eq!(lines[0].rest, " handles links its 1 handler claims");
        assert_eq!(lines[1].rest, " run after it \u{00b7} 1 rule");
        assert_eq!(plural(0, "rule", "rules"), "0 rules");
        assert_eq!(plural(2, "rule", "rules"), "2 rules");
    }

    /// The config's own fixed default first, then the browser links open
    /// in today, then the first one found; PwrRoute itself never.
    #[test]
    fn everything_else_is_preselected_in_order() {
        let mut model = FirstRunModel::fixture();
        model.fallback = None;
        model.current_default = Some("Google Chrome".to_owned());
        let (fallback, picked) = model.preselection().unwrap();
        assert_eq!(
            (fallback.browser.value.as_str(), picked),
            ("com.apple.Safari", Picked::Config)
        );

        // The config's default, with its profile when the browser has one
        // by that name.
        let config = model.finicky.as_mut().unwrap();
        config.default_browser = Some("Google Chrome".to_owned());
        config.default_profile = Some("work".to_owned());
        let (fallback, picked) = model.preselection().unwrap();
        assert_eq!(fallback.browser.value, "com.google.Chrome");
        assert_eq!(fallback.profile.as_deref(), Some("Work"));
        assert_eq!(picked, Picked::Config);

        // A function default, or one not installed: the OS default.
        let config = model.finicky.as_mut().unwrap();
        config.default_browser = Some("com.example.NotHere".to_owned());
        config.default_profile = None;
        model.current_default = Some("Firefox".to_owned());
        let (fallback, picked) = model.preselection().unwrap();
        assert_eq!(
            (fallback.browser.value.as_str(), picked),
            ("org.mozilla.firefox", Picked::OsDefault)
        );
        assert_eq!(fallback.profile, None);

        // PwrRoute is the default (or the OS did not say), no config: the
        // first browser found.
        model.finicky = None;
        model.current_default = None;
        let (fallback, picked) = model.preselection().unwrap();
        assert_eq!(
            (fallback.browser.value.as_str(), picked),
            ("com.google.Chrome", Picked::FirstFound)
        );
        model.browsers.clear();
        assert_eq!(model.preselection(), None);
    }

    #[test]
    fn the_owner_of_everything_else_changes_what_the_step_and_the_summary_say() {
        let mut model = FirstRunModel::fixture();
        model.import = Some(ImportChoice::KeepFile);
        assert_eq!(
            model.config_owner_label(),
            "~/.finicky.ts's default \u{00b7} Safari"
        );
        assert!(
            model
                .everything_else_detail()
                .starts_with("~/.finicky.ts's defaultBrowser handles links its handlers don't")
        );
        assert_eq!(
            model.picked_note().as_deref(),
            Some("Picked from ~/.finicky.ts's defaultBrowser. Change it any time.")
        );
        assert_eq!(
            model.everything_else_result(),
            "~/.finicky.ts's default \u{00b7} Safari"
        );
        model.is_default = true;
        let lines = model.where_links_go();
        assert_eq!(
            lines
                .iter()
                .map(|line| (line.done, line.subject.as_str(), line.rest.as_str()))
                .collect::<Vec<_>>(),
            [
                (true, "~/.finicky.ts", " handles links its 5 handlers claim"),
                (false, "Your rules", " run after it \u{00b7} none yet"),
                (true, "Everything else", " \u{2192} ~/.finicky.ts's default"),
            ]
        );
        assert_eq!(
            lines[2].browser,
            Some(("Sf".to_owned(), "Safari".to_owned()))
        );

        model.precedence = Precedence::PwrRoute;
        model.fallback = Some(BrowserLabel::new("Ch", "Google Chrome", Some("Personal")));
        assert_eq!(
            model.everything_else_detail(),
            "Your rules run first, then ~/.finicky.ts's handlers. Links nothing claims go here \
             instead of Safari."
        );
        assert_eq!(
            model.everything_else_result(),
            "PwrRoute's default \u{00b7} Google Chrome \u{00b7} Personal"
        );
        let lines = model.where_links_go();
        assert_eq!(lines[0].subject, "Your rules");
        assert_eq!(lines[0].rest, " run first \u{00b7} none yet");
        assert_eq!(lines[2].rest, " \u{2192} PwrRoute's default");
        assert_eq!(
            lines[2].browser,
            Some((
                "Ch".to_owned(),
                "Google Chrome \u{00b7} Personal".to_owned()
            ))
        );

        // Kept while it does not load: the Everything else browser takes
        // its links until it does.
        let mut broken = model.clone();
        broken.precedence = Precedence::Config;
        broken.finicky.as_mut().unwrap().error = Some("SyntaxError: x".to_owned());
        let lines = broken.where_links_go();
        assert_eq!(
            (lines[0].done, lines[0].rest.as_str()),
            (false, " runs once it loads")
        );
        assert_eq!(lines[2].rest, " \u{2192}");
        assert_eq!(
            broken.everything_else_result(),
            "Google Chrome \u{00b7} Personal"
        );

        // A function default names no browser.
        model.finicky.as_mut().unwrap().default_name = None;
        assert_eq!(model.config_owner_label(), "~/.finicky.ts's default");

        // Without a config, two lines and no owner.
        let mut plain = plain();
        plain.is_default = true;
        plain.rules = 2;
        let lines = plain.where_links_go();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            (lines[0].done, lines[0].rest.as_str()),
            (true, " \u{00b7} 2 rules")
        );
        assert_eq!(
            plain.everything_else_result(),
            "Google Chrome \u{00b7} Personal"
        );
        assert_eq!(
            plain.picked_note().as_deref(),
            Some("Picked from your current default browser. Change it any time.")
        );
    }

    struct Calls {
        fallbacks: Rc<RefCell<Vec<Fallback>>>,
        precedences: Rc<RefCell<Vec<Precedence>>>,
        imports: Rc<RefCell<Vec<ImportChoice>>>,
        replies: Rc<RefCell<Vec<ImportReply>>>,
        rescans: Rc<Cell<usize>>,
        rescan_finds: Rc<RefCell<Vec<BrowserChoice>>>,
        defaults: Rc<Cell<usize>>,
        default_fails: Rc<RefCell<Option<String>>>,
        finishes: Rc<RefCell<Vec<Finish>>>,
    }

    fn open(cx: &mut TestAppContext) -> (Entity<FirstRun>, Calls, &mut VisualTestContext) {
        open_with(cx, FirstRunModel::fixture())
    }

    fn open_with(
        cx: &mut TestAppContext,
        model: FirstRunModel,
    ) -> (Entity<FirstRun>, Calls, &mut VisualTestContext) {
        cx.update(|cx| crate::assets::load_fonts(cx));
        let calls = Calls {
            fallbacks: Rc::default(),
            precedences: Rc::default(),
            imports: Rc::default(),
            replies: Rc::default(),
            rescans: Rc::default(),
            rescan_finds: Rc::default(),
            defaults: Rc::default(),
            default_fails: Rc::default(),
            finishes: Rc::default(),
        };
        let fallbacks = calls.fallbacks.clone();
        let precedences = calls.precedences.clone();
        let imports = calls.imports.clone();
        let replies = calls.replies.clone();
        let rescans = calls.rescans.clone();
        let rescan_finds = calls.rescan_finds.clone();
        let defaults = calls.defaults.clone();
        let default_fails = calls.default_fails.clone();
        let finishes = calls.finishes.clone();
        let hooks = FirstRunHooks {
            on_fallback: Rc::new(move |choice, _| fallbacks.borrow_mut().push(choice)),
            on_precedence: Rc::new(move |precedence, _| precedences.borrow_mut().push(precedence)),
            on_import: Rc::new(move |choice, reply, _| {
                imports.borrow_mut().push(choice);
                replies.borrow_mut().push(reply);
            }),
            on_rescan: Rc::new(move |_| {
                rescans.set(rescans.get() + 1);
                rescan_finds.borrow().clone()
            }),
            on_set_default_browser: Rc::new(move |_| {
                defaults.set(defaults.get() + 1);
                match default_fails.borrow().clone() {
                    Some(error) => Err(error),
                    None => Ok(()),
                }
            }),
            on_finish: Rc::new(move |finished: Finished, _| {
                finishes.borrow_mut().push(finished.how);
            }),
        };
        let (view, cx) = cx.add_window_view(|_, cx| FirstRun::new(model, hooks, cx));
        draw(cx);
        (view, calls, cx)
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
    }

    fn click(id: &'static str, cx: &mut VisualTestContext) {
        press(id, cx);
        draw(cx);
    }

    /// A click without the redraw: for a button that closes the window.
    fn press(id: &'static str, cx: &mut VisualTestContext) {
        let bounds = cx
            .debug_bounds(id)
            .unwrap_or_else(|| panic!("{id} is drawn"));
        cx.simulate_click(bounds.center(), Modifiers::none());
    }

    fn steps(view: &Entity<FirstRun>, cx: &mut VisualTestContext) -> Vec<StepState> {
        view.read_with(cx, |this, _| states(&this.model))
    }

    #[gpui_kit::test]
    fn the_default_browser_step_only_calls_its_hook(cx: &mut TestAppContext) {
        let (view, calls, cx) = open(cx);
        assert!(cx.debug_bounds(ROOT_ID).is_some());

        // The default-browser step waits for the config step.
        click("first-run-default", cx);
        assert_eq!(calls.defaults.get(), 0);

        click("first-run-keep", cx);
        assert_eq!(*calls.imports.borrow(), [ImportChoice::KeepFile]);
        assert_eq!(steps(&view, cx), [Done, Done, Current]);
        // The answers are gone once one is given.
        assert!(cx.debug_bounds("first-run-keep").is_none());
        assert!(cx.debug_bounds("first-run-dont-use").is_none());

        click("first-run-default", cx);
        assert_eq!(calls.defaults.get(), 1);
        assert!(view.read_with(cx, |this, _| this.model.default_requested));
        // Asking does not make it so: the OS answers in its own dialog.
        assert!(!view.read_with(cx, |this, _| this.model.routing()));

        // The answer arrives from whoever reads the OS.
        view.update(cx, |this, cx| {
            this.set_default_state(&DefaultState::PwrRoute, false, cx);
        });
        draw(cx);
        assert!(view.read_with(cx, |this, _| this.model.routing()));
        assert_eq!(steps(&view, cx), [Done, Done, Done]);
        assert!(cx.debug_bounds("first-run-default").is_none());
        assert!(cx.debug_bounds("first-run-default-note").is_none());
        // Done: the summary, the folded steps, and the way out.
        assert!(cx.debug_bounds("first-run-where").is_some());
        assert!(cx.debug_bounds("first-run-folded-3").is_some());
        assert!(cx.debug_bounds("first-run-step-1").is_none());
        assert!(cx.debug_bounds("first-run-done").is_some());
    }

    /// The default-browser step while the OS has the question: no button,
    /// a waiting line; once the answer settles as no, "Ask again" and who
    /// is still default.
    #[gpui_kit::test]
    fn the_default_step_waits_for_the_os_and_then_offers_to_ask_again(cx: &mut TestAppContext) {
        let (view, calls, cx) = open_with(cx, plain());
        let chrome = DefaultState::Other("Google Chrome".into());
        let note =
            |cx: &mut VisualTestContext| view.read_with(cx, |this, _| this.model.default_note());
        assert_eq!(note(cx), None);

        click("first-run-default", cx);
        assert_eq!(calls.defaults.get(), 1);
        // The request is out: the app says so, and the button steps aside.
        view.update(cx, |this, cx| this.set_default_state(&chrome, true, cx));
        draw(cx);
        assert_eq!(note(cx).as_deref(), Some(waiting_line()));
        assert!(cx.debug_bounds("first-run-default").is_none());
        assert!(cx.debug_bounds("first-run-default-note").is_some());
        // Nothing to click, and the model refuses a second request anyway.
        view.update(cx, |this, cx| this.set_default(cx));
        assert_eq!(calls.defaults.get(), 1);

        // The poll ran out, or the window came back, with the same answer.
        view.update(cx, |this, cx| this.set_default_state(&chrome, false, cx));
        draw(cx);
        assert_eq!(
            note(cx).as_deref(),
            Some("Google Chrome is still the default.")
        );
        assert!(cx.debug_bounds("first-run-default").is_some());
        assert!(!view.read_with(cx, |this, _| this.model.complete()));
        assert!(cx.debug_bounds("first-run-later").is_some());
        assert!(cx.debug_bounds("first-run-done").is_none());

        click("first-run-default", cx);
        assert_eq!(calls.defaults.get(), 2);
        view.update(cx, |this, cx| this.set_default_state(&chrome, true, cx));
        // This time the OS agreed, mid-poll.
        view.update(cx, |this, cx| {
            this.set_default_state(&DefaultState::PwrRoute, false, cx);
        });
        draw(cx);
        assert_eq!(note(cx), None);
        assert!(view.read_with(cx, |this, _| this.model.complete()));
        assert!(cx.debug_bounds("first-run-later").is_none());
        assert!(cx.debug_bounds("first-run-done").is_some());
    }

    /// 4i: a request the OS turns down with an error says so under the
    /// step, which stays current with its button.
    #[gpui_kit::test]
    fn a_refused_request_says_why_and_the_step_stays(cx: &mut TestAppContext) {
        let (view, calls, cx) = open_with(cx, plain());
        *calls.default_fails.borrow_mut() = Some("Launch Services returned error -54".to_owned());
        click("first-run-default", cx);
        assert_eq!(calls.defaults.get(), 1);
        assert!(cx.debug_bounds("first-run-default-error").is_some());
        assert_eq!(
            view.read_with(cx, |this, _| this.model.default_warning()),
            Some(format!(
                "{} didn't take the request: Launch Services returned error -54. Try Settings \
                 \u{203a} Browsers.",
                os_subject()
            ))
        );
        assert!(!view.read_with(cx, |this, _| this.model.default_requested));
        assert_eq!(steps(&view, cx), [Done, Current]);

        // Asked again, and it went through: the warning goes.
        *calls.default_fails.borrow_mut() = None;
        click("first-run-default", cx);
        assert_eq!(calls.defaults.get(), 2);
        assert!(cx.debug_bounds("first-run-default-error").is_none());
    }

    #[gpui_kit::test]
    fn later_tells_the_app_and_closes_the_window(cx: &mut TestAppContext) {
        let (_view, calls, cx) = open(cx);
        press("first-run-later", cx);
        assert_eq!(*calls.finishes.borrow(), [Finish::Later]);
        assert!(cx.windows().is_empty());
    }

    #[gpui_kit::test]
    fn open_pwrroute_tells_the_app_and_closes_the_window(cx: &mut TestAppContext) {
        let mut model = FirstRunModel::fixture();
        model.import = Some(ImportChoice::DontUse);
        model.is_default = true;
        let (_view, calls, cx) = open_with(cx, model);
        assert!(cx.debug_bounds("first-run-later").is_none());
        press("first-run-done", cx);
        assert_eq!(*calls.finishes.borrow(), [Finish::Done]);
        assert!(cx.windows().is_empty());
    }

    /// Import waits for the importer: "Importing…", no answers to click,
    /// and the next step still to do. Cancel brings the step back as it
    /// was.
    #[gpui_kit::test]
    fn a_cancelled_import_leaves_the_step_current(cx: &mut TestAppContext) {
        let (view, calls, cx) = open(cx);
        click("first-run-import", cx);
        assert_eq!(*calls.imports.borrow(), [ImportChoice::Import]);
        assert!(view.read_with(cx, |this, _| this.model.importing));
        assert_eq!(steps(&view, cx), [Current, Todo, Todo]);
        assert!(cx.debug_bounds("first-run-importing").is_some());
        assert!(cx.debug_bounds("first-run-import").is_none());
        assert!(cx.debug_bounds("first-run-keep").is_none());
        // A second click has nothing to land on, and the model refuses it.
        view.update_in(cx, |this, window, cx| {
            this.choose(ImportChoice::KeepFile, window, cx);
        });
        assert_eq!(calls.imports.borrow().len(), 1);

        let reply = calls.replies.borrow()[0].clone();
        cx.update(|_, cx| reply(ImportOutcome::Cancelled, cx));
        draw(cx);
        let model = view.read_with(cx, |this, _| this.model.clone());
        assert!(!model.importing);
        assert_eq!(model.import, None);
        assert_eq!(states(&model), [Current, Todo, Todo]);
        assert!(model.config_detail().starts_with("Found ~/.finicky.ts: "));
        assert!(cx.debug_bounds("first-run-importing").is_none());
        assert!(cx.debug_bounds("first-run-import").is_some());
        assert!(cx.debug_bounds("first-run-dont-use").is_some());
    }

    /// A finished import marks the step done with the count it wrote, and
    /// takes the default it may have brought.
    #[gpui_kit::test]
    fn a_completed_import_marks_the_step_done_with_its_count(cx: &mut TestAppContext) {
        let (view, calls, cx) = open(cx);
        click("first-run-import", cx);
        let reply = calls.replies.borrow()[0].clone();
        cx.update(|_, cx| {
            reply(
                ImportOutcome::Imported(Imported {
                    count: 4,
                    keeps_config: true,
                    rules: 6,
                    default_browser: "com.apple.Safari".to_owned(),
                    default_profile: String::new(),
                }),
                cx,
            );
        });
        draw(cx);
        let model = view.read_with(cx, |this, _| this.model.clone());
        assert_eq!(model.import, Some(ImportChoice::Import));
        assert_eq!(states(&model), [Done, Done, Current]);
        assert_eq!(
            model.config_detail(),
            "Imported 4 rules. ~/.finicky.ts is unchanged."
        );
        assert_eq!(model.rules, 6);
        // Safari was already the pick, so it still says where it came from.
        assert_eq!(model.picked, Some(Picked::Config));
        // The config stayed in charge: the owner choice is offered.
        assert!(cx.debug_bounds("first-run-owner-pwrroute").is_some());
        // A late second answer changes nothing.
        cx.update(|_, cx| reply(ImportOutcome::Cancelled, cx));
        assert_eq!(
            view.read_with(cx, |this, _| this.model.import),
            Some(ImportChoice::Import)
        );
    }

    /// An import that turned the config off: no owner choice, the plain
    /// Everything else wording.
    #[gpui_kit::test]
    fn an_import_that_stops_the_config_drops_the_owner_choice(cx: &mut TestAppContext) {
        let (view, calls, cx) = open(cx);
        click("first-run-import", cx);
        let reply = calls.replies.borrow()[0].clone();
        cx.update(|_, cx| {
            reply(
                ImportOutcome::Imported(Imported {
                    count: 5,
                    keeps_config: false,
                    rules: 5,
                    default_browser: "com.google.Chrome".to_owned(),
                    default_profile: "Work".to_owned(),
                }),
                cx,
            );
        });
        draw(cx);
        let model = view.read_with(cx, |this, _| this.model.clone());
        assert!(!model.config_in_charge());
        assert!(cx.debug_bounds("first-run-owner-config").is_none());
        // The import brought another default: shown, no longer "picked".
        assert_eq!(
            model.fallback,
            Some(BrowserLabel::new("Ch", "Google Chrome", Some("Work")))
        );
        assert_eq!(model.picked, None);
        assert!(
            model
                .everything_else_detail()
                .starts_with("Links no rule claims")
        );
    }

    #[gpui_kit::test]
    fn dont_use_it_answers_the_step_at_once(cx: &mut TestAppContext) {
        let (view, calls, cx) = open(cx);
        click("first-run-dont-use", cx);
        assert_eq!(*calls.imports.borrow(), [ImportChoice::DontUse]);
        let model = view.read_with(cx, |this, _| this.model.clone());
        assert_eq!(states(&model), [Done, Done, Current]);
        assert!(!model.config_in_charge());
        assert!(cx.debug_bounds("first-run-owner-config").is_none());
    }

    /// Kept in charge, the step asks who owns everything else, and each
    /// answer goes to the hook once.
    #[gpui_kit::test]
    fn the_owner_choice_goes_to_its_hook(cx: &mut TestAppContext) {
        let mut model = FirstRunModel::fixture();
        model.import = Some(ImportChoice::KeepFile);
        let (view, calls, cx) = open_with(cx, model);
        assert!(cx.debug_bounds("first-run-picked").is_some());
        click("first-run-owner-pwrroute", cx);
        assert_eq!(*calls.precedences.borrow(), [Precedence::PwrRoute]);
        assert_eq!(
            view.read_with(cx, |this, _| this.model.precedence),
            Precedence::PwrRoute
        );
        // The same answer again is no change.
        click("first-run-owner-pwrroute", cx);
        click("first-run-owner-config", cx);
        assert_eq!(
            *calls.precedences.borrow(),
            [Precedence::PwrRoute, Precedence::Config]
        );
        // The step stays done throughout.
        assert_eq!(steps(&view, cx), [Done, Done, Current]);
    }

    /// With nothing chosen, the window picks the browser and saves it at
    /// once, so the step starts done; a pick of the reader's own replaces
    /// the line that says where it came from.
    #[gpui_kit::test]
    fn the_preselection_is_saved_and_stays_editable(cx: &mut TestAppContext) {
        let mut model = plain();
        model.fallback = None;
        model.picked = None;
        let (view, calls, cx) = open_with(cx, model);
        assert_eq!(steps(&view, cx), [Current, Todo]);
        view.update(cx, |this, cx| this.preselect(cx));
        draw(cx);
        assert_eq!(
            *calls.fallbacks.borrow(),
            [Fallback {
                browser: view.read_with(cx, |this, _| this.model.browsers[0].clone()),
                profile: None,
            }]
        );
        assert_eq!(steps(&view, cx), [Done, Current]);
        assert!(cx.debug_bounds("first-run-picked").is_some());
        // Already chosen: nothing more is written.
        view.update(cx, |this, cx| this.preselect(cx));
        assert_eq!(calls.fallbacks.borrow().len(), 1);

        click(FALLBACK_ID, cx);
        click("first-run-browser-1", cx);
        assert_eq!(
            calls.fallbacks.borrow()[1].browser.value,
            "org.mozilla.firefox"
        );
        assert_eq!(view.read_with(cx, |this, _| this.model.picked), None);
        assert!(cx.debug_bounds("first-run-picked").is_none());
    }

    #[gpui_kit::test]
    fn the_browser_list_opens_and_closes_from_its_field(cx: &mut TestAppContext) {
        let (view, calls, cx) = open_with(cx, plain());
        assert!(cx.debug_bounds("first-run-browser-1").is_none());
        click(FALLBACK_ID, cx);
        assert!(cx.debug_bounds("first-run-browser-menu").is_some());
        // The list opens under its field, as wide as it.
        let field = cx.debug_bounds(FALLBACK_ID).unwrap();
        let menu = cx.debug_bounds("first-run-browser-menu").unwrap();
        assert!(menu.top() >= field.bottom(), "{menu:?} under {field:?}");
        assert_eq!(menu.size.width, field.size.width);
        // A second click on the field closes it.
        click(FALLBACK_ID, cx);
        assert!(cx.debug_bounds("first-run-browser-menu").is_none());
        assert!(calls.fallbacks.borrow().is_empty());
        assert_eq!(view.read_with(cx, |this, _| this.picking), Picking::None);
    }

    /// A browser with profiles gets a profile select beside it, starting at
    /// the browser's own default; each pick goes to the hook with the
    /// browser.
    #[gpui_kit::test]
    fn a_profile_is_picked_for_a_browser_that_has_them(cx: &mut TestAppContext) {
        let mut model = plain();
        model.fallback = Some(BrowserLabel::new("Sf", "Safari", None));
        let (view, calls, cx) = open_with(cx, model);
        assert!(cx.debug_bounds(PROFILE_ID).is_none());

        click(FALLBACK_ID, cx);
        click("first-run-browser-0", cx);
        assert!(cx.debug_bounds(PROFILE_ID).is_some());
        let chrome = view.read_with(cx, |this, _| this.model.browsers[0].clone());
        assert_eq!(
            *calls.fallbacks.borrow(),
            [Fallback {
                browser: chrome.clone(),
                profile: None
            }]
        );

        click(PROFILE_ID, cx);
        // "Default profile", Personal, Work.
        assert!(cx.debug_bounds("first-run-profile-2").is_some());
        assert!(cx.debug_bounds("first-run-profile-3").is_none());
        click("first-run-profile-2", cx);
        assert!(cx.debug_bounds("first-run-profile-2").is_none());
        assert_eq!(
            calls.fallbacks.borrow().last(),
            Some(&Fallback {
                browser: chrome.clone(),
                profile: Some("Work".to_owned())
            })
        );
        assert_eq!(
            view.read_with(cx, |this, _| this.model.fallback.clone()),
            Some(BrowserLabel::new("Ch", "Google Chrome", Some("Work")))
        );

        // Back to the browser's own default.
        click(PROFILE_ID, cx);
        click("first-run-profile-0", cx);
        assert_eq!(
            calls.fallbacks.borrow().last(),
            Some(&Fallback {
                browser: chrome,
                profile: None
            })
        );
        assert_eq!(calls.fallbacks.borrow().len(), 3);

        // Safari has no profiles: no profile select, and the profile goes.
        click(FALLBACK_ID, cx);
        click("first-run-browser-2", cx);
        assert!(cx.debug_bounds(PROFILE_ID).is_none());
        assert_eq!(calls.fallbacks.borrow()[3].profile, None);
        assert_eq!(
            view.read_with(cx, |this, _| this.model.fallback.clone()),
            Some(BrowserLabel::new("Sf", "Safari", None))
        );
    }

    #[gpui_kit::test]
    fn nothing_importable_offers_keep_and_dont_use_it(cx: &mut TestAppContext) {
        let mut model = FirstRunModel::fixture();
        model.finicky.as_mut().unwrap().importable = 0;
        let (view, calls, cx) = open_with(cx, model);
        assert!(cx.debug_bounds("first-run-import").is_none());
        assert!(cx.debug_bounds("first-run-dont-use").is_some());
        click("first-run-keep", cx);
        assert_eq!(*calls.imports.borrow(), [ImportChoice::KeepFile]);
        assert!(
            view.read_with(cx, |this, _| this.model.config_detail())
                .starts_with("~/.finicky.ts stays in charge.")
        );
    }

    /// 4c: the error line, and Keep or Don't use it; no import.
    #[gpui_kit::test]
    fn a_config_that_does_not_load_shows_why(cx: &mut TestAppContext) {
        let mut model = FirstRunModel::fixture();
        let config = model.finicky.as_mut().unwrap();
        config.error = Some("SyntaxError: x \u{00b7} ~/.finicky.ts line 4".to_owned());
        config.importable = 0;
        let (_view, _calls, cx) = open_with(cx, model);
        assert!(cx.debug_bounds("first-run-config-error").is_some());
        assert!(cx.debug_bounds("first-run-import").is_none());
        assert!(cx.debug_bounds("first-run-keep").is_some());
        assert!(cx.debug_bounds("first-run-dont-use").is_some());
    }

    /// 4h: no browsers, then Look again finds one, picks it and saves it.
    #[gpui_kit::test]
    fn look_again_rescans_and_picks_what_it_finds(cx: &mut TestAppContext) {
        let mut model = plain();
        let found = model.browsers[2].clone();
        model.browsers.clear();
        model.fallback = None;
        let (view, calls, cx) = open_with(cx, model);
        assert!(
            view.read_with(cx, |this, _| this.model.everything_else_detail())
                .starts_with("No browsers found on ")
        );
        assert!(cx.debug_bounds(FALLBACK_ID).is_none());

        // Still nothing.
        click("first-run-look-again", cx);
        assert_eq!(calls.rescans.get(), 1);
        assert!(cx.debug_bounds("first-run-look-again").is_some());
        assert!(calls.fallbacks.borrow().is_empty());

        calls.rescan_finds.borrow_mut().push(found.clone());
        click("first-run-look-again", cx);
        assert_eq!(calls.rescans.get(), 2);
        assert_eq!(
            *calls.fallbacks.borrow(),
            [Fallback {
                browser: found,
                profile: None
            }]
        );
        assert_eq!(steps(&view, cx), [Done, Current]);
        assert!(cx.debug_bounds("first-run-look-again").is_none());
        assert!(cx.debug_bounds(FALLBACK_ID).is_some());
    }

    /// The body scrolls: at the window's minimum height the footer is still
    /// reachable, not cut off.
    #[gpui_kit::test]
    fn the_body_scrolls_so_the_footer_is_never_cut_off(cx: &mut TestAppContext) {
        let mut model = FirstRunModel::fixture();
        model.import = Some(ImportChoice::KeepFile);
        model.default_error = Some("a long error".to_owned());
        let (_view, _calls, cx) = open_with(cx, model);
        cx.simulate_resize(size(px(680.0), px(560.0)));
        draw(cx);
        let body = cx.debug_bounds("first-run-body").unwrap();
        let later = cx.debug_bounds("first-run-later").unwrap();
        // Laid out past the window's bottom, inside the scrolled body.
        assert!(later.bottom() > body.bottom(), "{later:?} in {body:?}");
        cx.simulate_event(gpui_kit::ScrollWheelEvent {
            position: body.center(),
            delta: gpui_kit::ScrollDelta::Pixels(point(px(0.0), px(-2000.0))),
            ..Default::default()
        });
        draw(cx);
        let later = cx.debug_bounds("first-run-later").unwrap();
        assert!(later.bottom() <= body.bottom(), "{later:?} in {body:?}");
    }
}
