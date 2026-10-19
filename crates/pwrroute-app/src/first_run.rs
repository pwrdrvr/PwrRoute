//! The first-run window: `design/PwrRoute Tray and First Run.dc.html`
//! option 2a.
//!
//! Three steps, in the order that makes the last one safe: a fallback
//! browser, what to do with an existing Finicky config, and only then the OS
//! default. This window never touches the OS itself. Its choices go to
//! [`FirstRunHooks`]; the default-browser step calls
//! [`FirstRunHooks::on_set_default_browser`] and nothing else, so whoever
//! opens the window decides what that does (`pwrroute_os::set_default_browser`
//! in the real app, a log line in development and tests).

use std::rc::Rc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Bounds, ClickEvent, Context, Div, FocusHandle, HighlightStyle,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledText, TitlebarOptions, Window,
    WindowBounds, WindowHandle, WindowOptions, div, point, px, size,
};

use crate::chrome::{BRAND_X, STRIP_HEIGHT};
use crate::theme::{self, icon, mono, text, tracked};
use crate::tray::model::BrowserLabel;
use crate::widgets::{badge, caret, input_box};

/// The element id of the window's root, for tests and the inspector.
pub const ROOT_ID: &str = "first-run-root";

/// `.dv-card.win { width: 680px }`, tall enough for the three steps.
const WINDOW_SIZE: (f32, f32) = (680.0, 600.0);

/// What to do with a Finicky config that was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// How many of those the visual rules can hold.
    pub importable: usize,
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
    /// Step 1. `None` until one is picked.
    pub fallback: Option<BrowserLabel>,
    /// Step 2's subject. `None`: no Finicky config was found.
    pub finicky: Option<FinickyConfig>,
    pub import: Option<ImportChoice>,
    /// The browser links open in today, before PwrRoute is the default.
    pub current_default: String,
    /// Step 3 was asked for. The OS answers on its own, in its own dialog.
    pub default_requested: bool,
    /// The OS reports PwrRoute is the default browser.
    pub is_default: bool,
}

impl FirstRunModel {
    /// The design's 2a: step 1 done, a Finicky config found, step 3 waiting.
    pub fn fixture() -> Self {
        Self {
            fallback: Some(BrowserLabel::new("Ch", "Google Chrome", Some("Personal"))),
            finicky: Some(FinickyConfig {
                path: "~/.finicky.ts".to_owned(),
                rewrites: 2,
                handlers: 6,
                importable: 5,
            }),
            import: None,
            current_default: "Google Chrome".to_owned(),
            default_requested: false,
            is_default: false,
        }
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

    /// Step 2's paragraph.
    pub fn import_detail(&self) -> String {
        match (&self.finicky, self.import) {
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
        format!(
            "{} will ask you to confirm. Until then, links keep opening in {} as they do now.",
            os_name(),
            self.current_default
        )
    }
}

fn os_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "Your desktop"
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

/// What the window's choices do. The window itself changes nothing.
#[derive(Clone)]
pub struct FirstRunHooks {
    pub on_import: Hook<ImportChoice>,
    /// "Set as default browser". Only this asks the OS anything.
    pub on_set_default_browser: Hook<()>,
}

impl FirstRunHooks {
    /// Report each choice on stderr and do nothing: development, and until
    /// the importer and registration are wired to this window.
    pub fn logging() -> Self {
        Self {
            on_import: Rc::new(|choice, _| eprintln!("pwrroute: first run: {choice:?}")),
            on_set_default_browser: Rc::new(|(), _| {
                eprintln!("pwrroute: first run: would ask the OS to make PwrRoute the default");
            }),
        }
    }
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
}

impl FirstRun {
    pub fn new(model: FirstRunModel, hooks: FirstRunHooks, cx: &mut Context<'_, Self>) -> Self {
        Self {
            model,
            hooks,
            focus: cx.focus_handle(),
        }
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
        if self.model.steps()[2] != StepState::Current {
            return;
        }
        self.model.default_requested = true;
        (self.hooks.on_set_default_browser)((), cx);
        cx.notify();
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
                                paragraph(
                                    "Where links go when no rule matches. Also what Pause \
                                     routing uses.",
                                ),
                                fallback_picker(model.fallback.as_ref()),
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
                            .child(
                                button("first-run-later", "Later", Kind::QuietSmall).on_click(
                                    cx.listener(|_, _: &ClickEvent, window, _| {
                                        window.remove_window();
                                    }),
                                ),
                            ),
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

/// Step 1's `.inp` select.
fn fallback_picker(fallback: Option<&BrowserLabel>) -> Div {
    let input = input_box().w(px(300.0));
    match fallback {
        Some(browser) => input
            .child(badge(browser.badge.clone()))
            .child(text(browser.display(), 12.5, 400.0, theme::text_primary()))
            .child(caret()),
        None => input
            .child(text("Choose a browser", 12.5, 400.0, theme::text_subtle()))
            .child(caret()),
    }
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
        .when(state != StepState::Done, |row| {
            row.child(
                button("first-run-default", label, Kind::Default)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.set_default(cx))),
            )
        })
        .when(model.default_requested && !model.is_default, |row| {
            row.child(text(
                format!("Waiting for {}\u{2026}", os_name()),
                11.0,
                400.0,
                theme::text_subtle(),
            ))
        })
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
        imports: Rc<RefCell<Vec<ImportChoice>>>,
        defaults: Rc<Cell<usize>>,
    }

    fn open(cx: &mut TestAppContext) -> (Entity<FirstRun>, Calls, &mut VisualTestContext) {
        cx.update(|cx| crate::assets::load_fonts(cx));
        let calls = Calls {
            imports: Rc::default(),
            defaults: Rc::default(),
        };
        let imports = calls.imports.clone();
        let defaults = calls.defaults.clone();
        let hooks = FirstRunHooks {
            on_import: Rc::new(move |choice, _| imports.borrow_mut().push(choice)),
            on_set_default_browser: Rc::new(move |(), _| defaults.set(defaults.get() + 1)),
        };
        let (view, cx) =
            cx.add_window_view(|_, cx| FirstRun::new(FirstRunModel::fixture(), hooks, cx));
        draw(cx);
        (view, calls, cx)
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
    }

    fn click(id: &'static str, cx: &mut VisualTestContext) {
        let bounds = cx
            .debug_bounds(id)
            .unwrap_or_else(|| panic!("{id} is drawn"));
        cx.simulate_click(bounds.center(), Modifiers::none());
        draw(cx);
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
    }
}
