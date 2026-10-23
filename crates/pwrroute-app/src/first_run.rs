//! The first-run window: `design/PwrRoute Tray and First Run.dc.html`
//! option 2a.
//!
//! Three steps, in the order that makes the last one safe: a fallback
//! browser, what to do with an existing Finicky config, and only then the OS
//! default. This window never touches the OS itself. Its choices go to
//! [`FirstRunHooks`]; the default-browser step calls
//! [`FirstRunHooks::on_set_default_browser`] and nothing else, so whoever
//! opens the window decides what that does (`crate::onboarding` wires
//! `pwrroute_os::set_default_browser` in the app; tests count the calls).
//! The OS's answer comes back through [`FirstRun::set_default_state`].
//! Leaving the window, by "Open PwrRoute" once every step is done or by
//! "Later" before that, goes through [`FirstRunHooks::on_finish`].

use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Bounds, ClickEvent, Context, DisplayId, Div, FocusHandle, HighlightStyle,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledText, TitlebarOptions, Window,
    WindowBounds, WindowHandle, WindowOptions, div, point, px, size,
};
use serde::{Deserialize, Serialize};

use crate::chrome::{BRAND_X, STRIP_HEIGHT};
use crate::default_browser::DefaultState;
use crate::theme::{self, icon, mono, text, tracked};
use crate::tray::model::BrowserLabel;
use crate::widgets::{badge, caret, input_box};

/// The element id of the window's root, for tests and the inspector.
pub const ROOT_ID: &str = "first-run-root";

/// `.dv-card.win { width: 680px }`, tall enough for the three steps.
const WINDOW_SIZE: (f32, f32) = (680.0, 600.0);

/// What to do with a Finicky config that was found. Serialized into the
/// first-run state file by its camelCase name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportChoice {
    /// Import what the visual rules can hold.
    Import,
    /// Keep the file in charge, as it is.
    KeepFile,
    StartEmpty,
}

/// A Finicky config found on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinickyConfig {
    /// As shown: `~/.finicky.ts`.
    pub path: String,
    pub rewrites: usize,
    pub handlers: usize,
    /// How many of those the visual rules can hold. Zero while there is no
    /// importer: the step then only offers to keep the file.
    pub importable: usize,
}

/// A browser the fallback picker offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserChoice {
    /// What `rules.json`'s `defaultBrowser` gets: the bundle id on macOS,
    /// the name elsewhere.
    pub value: String,
    pub label: BrowserLabel,
    /// Its profiles by the names the browser shows (as Settings › Browsers
    /// lists them), for step 1's profile menu. Empty: no menu.
    pub profiles: Vec<String>,
}

/// Step 1's choice: a browser, and one of its profiles or its default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fallback {
    pub browser: BrowserChoice,
    /// What `rules.json`'s `defaultProfile` gets. `None`: the browser's
    /// own default profile.
    pub profile: Option<String>,
}

/// Which of step 1's menus is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picking {
    None,
    Browser,
    Profile,
}

/// A step's state, as the design draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    Done,
    Current,
    Todo,
}

/// What the window shows.
#[derive(Clone, Debug)]
pub struct FirstRunModel {
    /// Step 1, with the chosen profile as the label's. `None` until one is
    /// picked.
    pub fallback: Option<BrowserLabel>,
    /// What step 1 offers: the browsers found on this machine.
    pub browsers: Vec<BrowserChoice>,
    /// Step 2's subject. `None`: no Finicky config was found.
    pub finicky: Option<FinickyConfig>,
    pub import: Option<ImportChoice>,
    /// The browser links open in today, before PwrRoute is the default.
    pub current_default: String,
    /// Step 3 was asked for at least once. The OS answers on its own, in
    /// its own dialog; the button then reads "Ask again".
    pub default_requested: bool,
    /// A request is out and the OS has not answered yet: whoever opened the
    /// window is reading the answer back, and the step says so.
    pub waiting: bool,
    /// The OS reports PwrRoute is the default browser.
    pub is_default: bool,
}

impl FirstRunModel {
    /// The design's 2a: step 1 done, a Finicky config found, step 3 waiting.
    #[cfg(test)]
    pub fn fixture() -> Self {
        let chrome = BrowserLabel::new("Ch", "Google Chrome", Some("Personal"));
        Self {
            fallback: Some(chrome.clone()),
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
                    label: BrowserLabel::new("Sa", "Safari", None),
                    profiles: Vec::new(),
                },
            ],
            finicky: Some(FinickyConfig {
                path: "~/.finicky.ts".to_owned(),
                rewrites: 2,
                handlers: 6,
                importable: 5,
            }),
            import: None,
            current_default: "Google Chrome".to_owned(),
            default_requested: false,
            waiting: false,
            is_default: false,
        }
    }

    /// The browser step 1 chose, as this machine lists it.
    pub fn fallback_choice(&self) -> Option<&BrowserChoice> {
        let chosen = self.fallback.as_ref()?;
        self.browsers
            .iter()
            .find(|choice| choice.label.name == chosen.name)
    }

    /// The profiles step 1's profile menu offers after its "Default
    /// profile": the chosen browser's. Empty, and no menu, for a browser
    /// without profiles or one not found on this machine.
    pub fn fallback_profiles(&self) -> &[String] {
        self.fallback_choice()
            .map_or(&[], |choice| choice.profiles.as_slice())
    }

    pub fn steps(&self) -> [StepState; 3] {
        let fallback = if self.fallback.is_some() {
            StepState::Done
        } else {
            StepState::Current
        };
        let import = match (self.import, fallback) {
            (Some(_), _) => StepState::Done,
            (None, StepState::Done) => StepState::Current,
            (None, _) => StepState::Todo,
        };
        let default = if self.is_default {
            StepState::Done
        } else if import == StepState::Done {
            StepState::Current
        } else {
            StepState::Todo
        };
        [fallback, import, default]
    }

    /// Links route only once PwrRoute is the default browser.
    pub fn routing(&self) -> bool {
        self.is_default
    }

    /// Every step is done: the window offers to open PwrRoute.
    pub fn complete(&self) -> bool {
        self.steps().iter().all(|step| *step == StepState::Done)
    }

    /// Step 3's line beside its button: the OS has the question, or it
    /// answered no (or was never answered) and the step offers to ask again.
    pub fn default_note(&self) -> Option<String> {
        if self.is_default {
            None
        } else if self.waiting {
            Some(format!("Waiting for {}\u{2026}", os_name()))
        } else if self.default_requested {
            Some(format!("{} is still the default.", self.current_default))
        } else {
            None
        }
    }

    /// Step 1's paragraph. With a Finicky config, its `defaultBrowser`
    /// decides where unmatched links go while it is in charge; this choice
    /// is for when it is not.
    pub fn fallback_detail(&self) -> String {
        match &self.finicky {
            Some(config) => format!(
                "{} decides where unmatched links go while it is in charge. This choice takes \
                 over when the config does not load or a browser cannot be opened, and while \
                 routing is paused.",
                config.path
            ),
            None => "Where links go when no rule matches or a browser cannot be opened. Also \
                     what Pause routing uses."
                .to_owned(),
        }
    }

    /// Step 2's paragraph.
    pub fn import_detail(&self) -> String {
        match (&self.finicky, self.import) {
            (Some(config), None) if config.importable == 0 => format!(
                "Found {} with {} rewrites and {} handlers, none of which a visual rule can \
                 hold exactly. It stays in charge and runs as it did under Finicky; the file \
                 is never modified. Import to visual rules\u{2026} on the Rules screen offers \
                 tested suggestions.",
                config.path, config.rewrites, config.handlers
            ),
            (Some(config), None) => format!(
                "Found {} with {} rewrites and {} handlers. Import what the visual rules can \
                 hold, keep the file in charge as it is, or start empty. The file is never \
                 modified.",
                config.path, config.rewrites, config.handlers
            ),
            (Some(config), Some(ImportChoice::Import)) => format!(
                "Imported {} rules from {}. The file is unchanged.",
                config.importable, config.path
            ),
            (Some(config), Some(ImportChoice::KeepFile)) => format!(
                "{} stays in charge. Edit it as you do now; PwrRoute reloads it.",
                config.path
            ),
            (_, Some(ImportChoice::StartEmpty)) => {
                "Starting with no rules. Every link goes to the fallback until you add one."
                    .to_owned()
            }
            (None, _) => "No Finicky config found. Rules start empty; add them in the Rules \
                          window."
                .to_owned(),
        }
    }

    /// Step 3's paragraph.
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
        format!(
            "{asks} Until then, links keep opening in {} as they do now.",
            self.current_default
        )
    }
}

/// Who answers step 3, for the middle of a sentence ("Waiting for
/// macOS…"): the OS by name, or the desktop on Linux, whose own name is
/// not known here.
fn os_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "your desktop"
    }
}

/// The header's line under the title.
fn help_line() -> &'static str {
    if cfg!(target_os = "macos") {
        "Nothing changes on this Mac until the last step. You can close this window and come \
         back from the menu bar."
    } else if cfg!(target_os = "windows") {
        "Nothing changes on this PC until the last step. You can close this window and come \
         back from the notification area."
    } else {
        "Nothing changes on this computer until the last step. You can close this window and \
         come back from the tray."
    }
}

type Hook<T> = Rc<dyn Fn(T, &mut App)>;

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
    /// Step 1 picked a browser, or a profile of it.
    pub on_fallback: Hook<Fallback>,
    pub on_import: Hook<ImportChoice>,
    /// "Set as default browser". Only this asks the OS anything.
    pub on_set_default_browser: Hook<()>,
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
    let bounds = Bounds::centered(None, size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)), cx);
    let window = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
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
    /// Step 1's list of browsers, or of the chosen browser's profiles, is
    /// open.
    pub picking: Picking,
}

impl FirstRun {
    pub fn new(model: FirstRunModel, hooks: FirstRunHooks, cx: &mut Context<'_, Self>) -> Self {
        Self {
            model,
            hooks,
            focus: cx.focus_handle(),
            picking: Picking::None,
        }
    }

    fn toggle_picker(&mut self, which: Picking, cx: &mut Context<'_, Self>) {
        self.picking = if self.picking == which {
            Picking::None
        } else {
            which
        };
        cx.notify();
    }

    /// A browser from step 1's menu, at its default profile.
    fn choose_fallback(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        self.picking = Picking::None;
        if let Some(choice) = self.model.browsers.get(index).cloned() {
            self.model.fallback = Some(choice.label.clone());
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

    /// A profile from step 1's profile menu: 0 is "Default profile", then
    /// the chosen browser's profiles in order.
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
        (self.hooks.on_fallback)(Fallback { browser, profile }, cx);
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
        let current = state.current_name();
        if self.model.is_default == is_default
            && self.model.current_default == current
            && self.model.waiting == waiting
        {
            return;
        }
        self.model.is_default = is_default;
        self.model.current_default = current;
        self.model.waiting = waiting;
        cx.notify();
    }

    fn choose(&mut self, choice: ImportChoice, cx: &mut Context<'_, Self>) {
        if self.model.steps()[1] != StepState::Current {
            return;
        }
        self.model.import = Some(choice);
        (self.hooks.on_import)(choice, cx);
        cx.notify();
    }

    fn set_default(&mut self, cx: &mut Context<'_, Self>) {
        if self.model.steps()[2] != StepState::Current || self.model.waiting {
            return;
        }
        self.model.default_requested = true;
        (self.hooks.on_set_default_browser)((), cx);
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
        let [fallback, import, default] = model.steps();
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
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap(px(16.0))
                    .pt(px(26.0))
                    .px(px(28.0))
                    .pb(px(22.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(tracked("First run", 9.5, 600.0, 0.14, theme::accent()))
                            .child(
                                text(
                                    "Three steps, then links start routing",
                                    21.0,
                                    700.0,
                                    theme::text_primary(),
                                )
                                .line_height(px(25.2))
                                .mt(px(6.0)),
                            )
                            .child(paragraph(help_line()).mt(px(6.0)).max_w(px(390.0))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(10.0))
                            .child(step(
                                1,
                                fallback,
                                "Fallback browser",
                                paragraph(model.fallback_detail()),
                                fallback_pickers(model, self.picking, cx),
                            ))
                            .child(step(
                                2,
                                import,
                                "Bring your Finicky rules",
                                import_paragraph(model),
                                import_actions(model, import, cx),
                            ))
                            .child(step(
                                3,
                                default,
                                "Make PwrRoute the default browser",
                                paragraph(model.default_detail()),
                                default_actions(model, default, cx),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(10.0))
                            .child(
                                text(
                                    "Change any of this later in Settings \u{203a} Browsers and \
                                     Configuration.",
                                    10.5,
                                    400.0,
                                    theme::text_subtle(),
                                )
                                .line_height(px(15.75)),
                            )
                            .child(finish_button(model, cx)),
                    ),
            )
    }
}

/// `.strip` for this window: the brand and whether links route yet.
fn strip(routing: bool) -> Div {
    let wordmark = StyledText::new("PwrRoute").with_highlights([(
        3..8,
        HighlightStyle {
            color: Some(theme::accent()),
            ..Default::default()
        },
    )]);
    let (dot, label) = if routing {
        (theme::status_ok(), "routing")
    } else {
        (theme::status_suspended(), "not routing yet")
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
                .child(theme::dot(dot))
                .child(mono(label, 10.5, 600.0, theme::text_muted())),
        )
}

/// `.step`: the number, the title, what it means, and its control.
fn step(
    number: usize,
    state: StepState,
    title: &'static str,
    body: impl IntoElement,
    control: impl IntoElement,
) -> Div {
    let (fill, border, color) = match state {
        StepState::Done => (
            theme::success_soft(),
            theme::transparent(),
            theme::success_text(),
        ),
        StepState::Current => (theme::accent(), theme::transparent(), theme::accent_on()),
        StepState::Todo => (
            theme::bg_elevated(),
            theme::border_default(),
            theme::text_muted(),
        ),
    };
    let label = if state == StepState::Done {
        "\u{2713}".to_owned()
    } else {
        number.to_string()
    };
    div()
        .debug_selector(move || format!("first-run-step-{number}"))
        .flex()
        .gap(px(14.0))
        .py(px(14.0))
        .px(px(16.0))
        .border_1()
        .rounded(px(10.0))
        .map(|step| match state {
            StepState::Current => step
                .border_color(theme::accent_border())
                .bg(theme::bg_row_active()),
            StepState::Done => step
                .border_color(theme::border_subtle())
                .bg(theme::bg_panel()),
            StepState::Todo => step
                .border_color(theme::border_subtle())
                .bg(theme::bg_panel())
                .opacity(0.6),
        })
        .child(
            div().w(px(28.0)).flex_none().child(
                div()
                    .size(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border_1()
                    .border_color(border)
                    .bg(fill)
                    .child(mono(label, 11.0, 600.0, color)),
            ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(text(title, 13.0, 600.0, theme::text_primary()).line_height(px(16.9)))
                .child(body)
                .child(control),
        )
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

/// Step 2's paragraph, the config path set off as the design's `.tag`.
fn import_paragraph(model: &FirstRunModel) -> Div {
    let detail = model.import_detail();
    let tag = model.finicky.as_ref().and_then(|config| {
        detail
            .find(&config.path)
            .map(|at| at..at + config.path.len())
    });
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

/// The element id of step 1's select.
pub const FALLBACK_ID: &str = "first-run-fallback";
/// The element id of step 1's profile select.
pub const PROFILE_ID: &str = "first-run-profile";

/// What the profile select says for no profile: the browser opens its own.
const DEFAULT_PROFILE: &str = "Default profile";

/// Step 1's controls: the browser select and, when the chosen browser has
/// profiles, the profile select beside it, each the design's `.inp` with
/// the list it opens under it.
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

/// Step 1's browser select: the browsers found on this machine, one row
/// each. With a profile select beside it, it shows the name alone.
fn browser_picker(
    model: &FirstRunModel,
    profile_beside: bool,
    open: bool,
    cx: &mut Context<'_, FirstRun>,
) -> Div {
    let input = input_box()
        .id(FALLBACK_ID)
        .debug_selector(|| FALLBACK_ID.into())
        .w(px(300.0))
        .cursor_pointer()
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.toggle_picker(Picking::Browser, cx);
        }));
    let input = match (&model.fallback, model.browsers.is_empty()) {
        (Some(browser), _) => {
            let shown = if profile_beside {
                browser.name.clone()
            } else {
                browser.display()
            };
            input
                .child(badge(browser.badge.clone()))
                .child(crate::widgets::field_label(shown, theme::text_primary()))
                .child(caret())
        }
        (None, false) => input
            .child(text("Choose a browser", 12.5, 400.0, theme::text_subtle()))
            .child(caret()),
        (None, true) => input.child(text("No browsers found", 12.5, 400.0, theme::text_subtle())),
    };
    let column = div().flex().flex_col().gap(px(6.0)).child(input);
    if !open || model.browsers.is_empty() {
        return column;
    }
    let rows = model.browsers.iter().enumerate().map(|(index, choice)| {
        menu_row(("first-run-browser", index))
            .debug_selector(move || format!("first-run-browser-{index}"))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.choose_fallback(index, cx);
            }))
            .child(badge(choice.label.badge.clone()))
            .child(crate::widgets::field_label(
                choice.label.name.clone(),
                theme::text_primary(),
            ))
    });
    column.child(menu("first-run-browser-menu", 300.0).children(rows))
}

/// Step 1's profile select: "Default profile", then the chosen browser's
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
        Some(profile) => crate::widgets::field_label(profile, theme::text_primary()),
        None => text(DEFAULT_PROFILE, 12.5, 400.0, theme::text_subtle()),
    };
    let input = input_box()
        .id(PROFILE_ID)
        .debug_selector(|| PROFILE_ID.into())
        .w(px(PROFILE_WIDTH))
        .cursor_pointer()
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.toggle_picker(Picking::Profile, cx);
        }))
        .child(label)
        .child(caret());
    let column = div().flex().flex_col().gap(px(6.0)).child(input);
    if !open {
        return column;
    }
    let names = std::iter::once(DEFAULT_PROFILE.to_owned()).chain(profiles.iter().cloned());
    let rows = names.enumerate().map(|(index, name)| {
        let color = if index == 0 {
            theme::text_secondary()
        } else {
            theme::text_primary()
        };
        menu_row(("first-run-profile", index))
            .debug_selector(move || format!("first-run-profile-{index}"))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.choose_profile(index, cx);
            }))
            .child(crate::widgets::field_label(name, color))
    });
    column.child(menu("first-run-profile-menu", PROFILE_WIDTH).children(rows))
}

/// The profile select's width: room for "Default profile" and a
/// Firefox profile name beside the 300px browser select.
const PROFILE_WIDTH: f32 = 180.0;

/// The list a select opens: the popover card under it. A long list scrolls
/// past `widgets::MENU_MAX_HEIGHT`.
fn menu(id: &'static str, width: f32) -> gpui_kit::Stateful<Div> {
    div()
        .id(id)
        .w(px(width))
        .max_h(px(crate::widgets::MENU_MAX_HEIGHT))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .p(px(4.0))
        .bg(theme::bg_popover())
        .border_1()
        .border_color(theme::border_default())
        .rounded(px(7.0))
}

/// One row of a select's list.
fn menu_row(id: impl Into<gpui_kit::ElementId>) -> gpui_kit::Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .min_w_0()
        .overflow_hidden()
        .items_center()
        .gap(px(8.0))
        .py(px(6.0))
        .px(px(10.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(|row| row.bg(theme::bg_hover()))
}

fn import_actions(
    model: &FirstRunModel,
    state: StepState,
    cx: &mut Context<'_, FirstRun>,
) -> impl IntoElement {
    let pick = |choice| {
        cx.listener(move |this: &mut FirstRun, _: &ClickEvent, _, cx| this.choose(choice, cx))
    };
    let row = div().flex().flex_wrap().items_center().gap(px(8.0));
    if state != StepState::Current {
        return row;
    }
    match &model.finicky {
        // Nothing imports exactly: the file stays in charge, and the step says so.
        Some(config) if config.importable == 0 => row.child(
            button(
                "first-run-keep",
                format!("Keep using {}", config.path),
                Kind::Primary,
            )
            .on_click(pick(ImportChoice::KeepFile)),
        ),
        Some(config) => row
            .child(
                button(
                    "first-run-import",
                    format!("Import {} rules", config.importable),
                    Kind::Primary,
                )
                .on_click(pick(ImportChoice::Import)),
            )
            .child(
                button(
                    "first-run-keep",
                    format!("Keep using {}", config.path),
                    Kind::Default,
                )
                .on_click(pick(ImportChoice::KeepFile)),
            )
            .child(
                button("first-run-empty", "Start empty", Kind::Quiet)
                    .on_click(pick(ImportChoice::StartEmpty)),
            ),
        None => row.child(
            button("first-run-empty", "Continue", Kind::Primary)
                .on_click(pick(ImportChoice::StartEmpty)),
        ),
    }
}

/// Step 3's control: the request button while the step is current and no
/// request is out, and the line that says what the OS said.
fn default_actions(
    model: &FirstRunModel,
    state: StepState,
    cx: &mut Context<'_, FirstRun>,
) -> impl IntoElement {
    let label = if model.default_requested {
        "Ask again"
    } else {
        "Set as default browser"
    };
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .when(state != StepState::Done && !model.waiting, |row| {
            row.child(
                button("first-run-default", label, Kind::Default)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.set_default(cx))),
            )
        })
        .when_some(model.default_note(), |row, note| {
            row.child(
                text(note, 11.0, 400.0, theme::text_subtle())
                    .debug_selector(|| "first-run-default-note".into()),
            )
        })
}

/// The footer's button: "Open PwrRoute" once every step is done, "Later"
/// before that. The design draws only Later; the primary is what the
/// finished window needs, since nothing else opens on its own.
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

fn button(id: &'static str, label: impl Into<SharedString>, kind: Kind) -> gpui_kit::Stateful<Div> {
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

    #[test]
    fn steps_unlock_in_order() {
        let mut model = FirstRunModel::fixture();
        use StepState::{Current, Done, Todo};
        assert_eq!(model.steps(), [Done, Current, Todo]);
        model.import = Some(ImportChoice::KeepFile);
        assert_eq!(model.steps(), [Done, Done, Current]);
        model.is_default = true;
        assert_eq!(model.steps(), [Done, Done, Done]);
        assert!(model.routing());

        let mut fresh = FirstRunModel::fixture();
        fresh.fallback = None;
        assert_eq!(fresh.steps(), [Current, Todo, Todo]);
    }

    #[test]
    fn step_one_says_who_owns_the_default_while_a_config_is_in_charge() {
        let mut model = FirstRunModel::fixture();
        assert_eq!(
            model.fallback_detail(),
            "~/.finicky.ts decides where unmatched links go while it is in charge. This choice \
             takes over when the config does not load or a browser cannot be opened, and while \
             routing is paused."
        );
        model.finicky = None;
        assert!(
            model
                .fallback_detail()
                .starts_with("Where links go when no rule matches")
        );
    }

    #[test]
    fn step_two_says_what_was_found_and_chosen() {
        let mut model = FirstRunModel::fixture();
        assert!(
            model
                .import_detail()
                .starts_with("Found ~/.finicky.ts with 2 rewrites and 6 handlers.")
        );
        model.import = Some(ImportChoice::Import);
        assert_eq!(
            model.import_detail(),
            "Imported 5 rules from ~/.finicky.ts. The file is unchanged."
        );
        model.finicky = None;
        model.import = None;
        assert!(
            model
                .import_detail()
                .starts_with("No Finicky config found.")
        );
    }

    struct Calls {
        fallbacks: Rc<RefCell<Vec<Fallback>>>,
        imports: Rc<RefCell<Vec<ImportChoice>>>,
        defaults: Rc<Cell<usize>>,
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
            imports: Rc::default(),
            defaults: Rc::default(),
            finishes: Rc::default(),
        };
        let fallbacks = calls.fallbacks.clone();
        let imports = calls.imports.clone();
        let defaults = calls.defaults.clone();
        let finishes = calls.finishes.clone();
        let hooks = FirstRunHooks {
            on_fallback: Rc::new(move |choice, _| fallbacks.borrow_mut().push(choice)),
            on_import: Rc::new(move |choice, _| imports.borrow_mut().push(choice)),
            on_set_default_browser: Rc::new(move |(), _| defaults.set(defaults.get() + 1)),
            on_finish: Rc::new(move |finished: Finished, _| {
                finishes.borrow_mut().push(finished.how)
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

    #[gpui_kit::test]
    fn the_default_browser_step_only_calls_its_hook(cx: &mut TestAppContext) {
        let (view, calls, cx) = open(cx);
        assert!(cx.debug_bounds(ROOT_ID).is_some());

        // Step 3 waits for step 2.
        click("first-run-default", cx);
        assert_eq!(calls.defaults.get(), 0);

        click("first-run-import", cx);
        assert_eq!(*calls.imports.borrow(), [ImportChoice::Import]);
        assert_eq!(
            view.read_with(cx, |this, _| this.model.steps()),
            [StepState::Done, StepState::Done, StepState::Current]
        );
        // The choice buttons are gone once one is made.
        assert!(cx.debug_bounds("first-run-keep").is_none());

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
        assert_eq!(
            view.read_with(cx, |this, _| this.model.steps()),
            [StepState::Done, StepState::Done, StepState::Done]
        );
        assert!(cx.debug_bounds("first-run-default").is_none());
        assert!(cx.debug_bounds("first-run-default-note").is_none());
    }

    /// Step 3 while the OS has the question: no button, a waiting line;
    /// once the answer settles as no, "Ask again" and who is still default.
    #[gpui_kit::test]
    fn step_three_waits_for_the_os_and_then_offers_to_ask_again(cx: &mut TestAppContext) {
        let mut model = FirstRunModel::fixture();
        model.import = Some(ImportChoice::KeepFile);
        let (view, calls, cx) = open_with(cx, model);
        let chrome = DefaultState::Other("Google Chrome".into());
        let note =
            |cx: &mut VisualTestContext| view.read_with(cx, |this, _| this.model.default_note());
        assert_eq!(note(cx), None);

        click("first-run-default", cx);
        assert_eq!(calls.defaults.get(), 1);
        // The request is out: the app says so, and the button steps aside.
        view.update(cx, |this, cx| this.set_default_state(&chrome, true, cx));
        draw(cx);
        // The platform's own name, as the view writes it.
        let waiting = format!("Waiting for {}\u{2026}", os_name());
        assert_eq!(note(cx).as_deref(), Some(waiting.as_str()));
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
        assert!(cx.debug_bounds("first-run-default").is_none());
        assert!(cx.debug_bounds("first-run-later").is_none());
        assert!(cx.debug_bounds("first-run-done").is_some());
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
        model.import = Some(ImportChoice::StartEmpty);
        model.is_default = true;
        let (_view, calls, cx) = open_with(cx, model);
        assert!(cx.debug_bounds("first-run-later").is_none());
        press("first-run-done", cx);
        assert_eq!(*calls.finishes.borrow(), [Finish::Done]);
        assert!(cx.windows().is_empty());
    }

    #[gpui_kit::test]
    fn the_fallback_is_picked_from_the_browsers_found(cx: &mut TestAppContext) {
        let mut model = FirstRunModel::fixture();
        model.fallback = None;
        let (view, calls, cx) = open_with(cx, model);
        assert_eq!(
            view.read_with(cx, |this, _| this.model.steps()),
            [StepState::Current, StepState::Todo, StepState::Todo]
        );
        assert!(cx.debug_bounds("first-run-browser-1").is_none());

        click(FALLBACK_ID, cx);
        assert!(cx.debug_bounds("first-run-browser-1").is_some());
        click("first-run-browser-1", cx);
        assert_eq!(calls.fallbacks.borrow().len(), 1);
        assert_eq!(
            calls.fallbacks.borrow()[0].browser.value,
            "org.mozilla.firefox"
        );
        assert_eq!(calls.fallbacks.borrow()[0].profile, None);
        assert_eq!(
            view.read_with(cx, |this, _| this.model.fallback.clone()),
            Some(BrowserLabel::new("Ff", "Firefox", None))
        );
        assert!(cx.debug_bounds("first-run-browser-1").is_none());
        assert_eq!(
            view.read_with(cx, |this, _| this.model.steps()),
            [StepState::Done, StepState::Current, StepState::Todo]
        );
    }

    /// A browser with profiles gets a profile select beside it, starting at
    /// the browser's own default; each pick goes to the hook with the
    /// browser.
    #[gpui_kit::test]
    fn step_one_picks_a_profile_for_a_browser_that_has_them(cx: &mut TestAppContext) {
        let mut model = FirstRunModel::fixture();
        model.fallback = None;
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
        assert_eq!(
            view.read_with(cx, |this, _| this.model.fallback.clone()),
            Some(BrowserLabel::new("Ch", "Google Chrome", None))
        );
        assert_eq!(calls.fallbacks.borrow().len(), 3);
    }

    /// Safari has no profiles: no profile select, and switching to it from
    /// a profiled browser drops the profile.
    #[gpui_kit::test]
    fn the_profile_menu_is_hidden_for_a_browser_without_profiles(cx: &mut TestAppContext) {
        let (view, calls, cx) = open(cx);
        // The fixture's fallback is Chrome · Personal.
        assert!(cx.debug_bounds(PROFILE_ID).is_some());
        click(FALLBACK_ID, cx);
        click("first-run-browser-2", cx);
        assert!(cx.debug_bounds(PROFILE_ID).is_none());
        assert_eq!(
            calls.fallbacks.borrow()[0].browser.value,
            "com.apple.Safari"
        );
        assert_eq!(calls.fallbacks.borrow()[0].profile, None);
        assert_eq!(
            view.read_with(cx, |this, _| this.model.fallback_profiles().len()),
            0
        );
        assert_eq!(
            view.read_with(cx, |this, _| this.model.fallback.clone()),
            Some(BrowserLabel::new("Sa", "Safari", None))
        );
    }

    #[gpui_kit::test]
    fn without_an_importer_the_config_can_only_be_kept(cx: &mut TestAppContext) {
        let mut model = FirstRunModel::fixture();
        model.finicky.as_mut().unwrap().importable = 0;
        let (view, calls, cx) = open_with(cx, model);
        assert!(cx.debug_bounds("first-run-import").is_none());
        assert!(cx.debug_bounds("first-run-empty").is_none());
        click("first-run-keep", cx);
        assert_eq!(*calls.imports.borrow(), [ImportChoice::KeepFile]);
        assert!(
            view.read_with(cx, |this, _| this.model.import_detail())
                .starts_with("~/.finicky.ts stays in charge.")
        );
    }
}
