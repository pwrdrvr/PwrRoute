//! The tray: the macOS menu bar item and its dropdown, or the Windows and
//! Linux notification-area icon and its native menu.
//!
//! [`install`] puts the icon up and owns a [`TrayModel`] entity that both
//! surfaces draw. Its recents and count follow the dispatch feed
//! (`activity::feed`) when one is installed, and its header chip the OS
//! ([`follow_default_browser`]). Pausing from either surface pauses the
//! router ([`RouterHandle::set_paused`]) with the target the dropdown
//! names, read from the files each time the dropdown opens or a pause
//! begins ([`pause_target`]). The Incoming links switch ([`set_mode`] and
//! the rest, design: Incoming Links) also sends the next few links, or all
//! of them for a while, to a destination the user picks, or holds them:
//! records them and opens none until the user opens them ([`open_held`]).
//! The router counts them down ([`RouterHandle::set_send_to`],
//! [`RouterHandle::set_hold`]) and the model follows it after every
//! dispatch and every second while a time runs ([`sync_override`]). A
//! pause, a send and a hold share the router's one slot, so turning one on
//! turns the others off.

mod dropdown;
mod icon;
pub mod model;
mod native;

#[cfg(test)]
mod tests;

use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime};

use gpui_kit::{App, AppContext as _, Entity, Global, WindowHandle};
use pwrroute_engine::{AppType, BrowserSpec, Engine, Limits, NativeHost, RulesFile, discovery};
use pwrroute_router::{
    DispatchId, FALLBACK_BROWSER, OpenAs, Override, PauseTarget, RouterHandle, SendScope, SendTo,
};

use self::dropdown::Dropdown;
use self::model::{
    BrowserLabel, DestinationChoices, Dispatched, ForKind, Mode, Routing, SendDestination,
    TIME_STEPS, TrayModel,
};
use self::native::{NativeTray, TrayEvent};
use crate::activity::{self, Feed};
use crate::app::{PwrRoute, Screen};
use crate::default_browser;
use crate::widgets::badge_for;

/// What a tray row or menu item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrayCommand {
    OpenMain,
    TestLink,
    Rules,
    Activity,
    Settings,
    About,
    TogglePause,
    /// Incoming links: back to the rules (ends a pause, a send or a hold).
    SendRules,
    /// The native menu's "Send next link → …": the next link to the
    /// destination.
    SendNext,
    /// The native menu's "Hold next link".
    HoldNext,
    /// The native menu's "Hold for 1 hour".
    HoldHour,
    /// The native menu's "Choose…": the Overview, whose hero has the whole
    /// switch and whose nodes offer Next link here.
    ChooseWhere,
    /// The native menu's "Held links (N)…": Activity, where they wait.
    HeldLinks,
    /// Ask the OS to make PwrRoute the default browser: the dropdown's row
    /// while another app has it.
    MakeDefault,
    Quit,
}

impl TrayCommand {
    const ALL: [Self; 15] = [
        Self::OpenMain,
        Self::TestLink,
        Self::Rules,
        Self::Activity,
        Self::Settings,
        Self::About,
        Self::TogglePause,
        Self::SendRules,
        Self::SendNext,
        Self::HoldNext,
        Self::HoldHour,
        Self::ChooseWhere,
        Self::HeldLinks,
        Self::MakeDefault,
        Self::Quit,
    ];

    /// The native menu item's id. Fixed strings, so a menu event maps back
    /// without any state shared with tray-icon's threads.
    fn menu_id(self) -> &'static str {
        match self {
            Self::OpenMain => "pwrroute.open",
            Self::TestLink => "pwrroute.test-link",
            Self::Rules => "pwrroute.rules",
            Self::Activity => "pwrroute.activity",
            Self::Settings => "pwrroute.settings",
            Self::About => "pwrroute.about",
            Self::TogglePause => "pwrroute.pause",
            Self::SendRules => "pwrroute.send-rules",
            Self::SendNext => "pwrroute.send-next",
            Self::HoldNext => "pwrroute.hold-next",
            Self::HoldHour => "pwrroute.hold-hour",
            Self::ChooseWhere => "pwrroute.send-choose",
            Self::HeldLinks => "pwrroute.held",
            Self::MakeDefault => "pwrroute.make-default",
            Self::Quit => "pwrroute.quit",
        }
    }

    fn from_menu_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|command| command.menu_id() == id)
    }
}

/// A tray click this soon after the dropdown closed itself is the click that
/// closed it (focus left for the menu bar), so it must not reopen it.
#[cfg_attr(
    not(target_os = "macos"),
    allow(dead_code, reason = "only macOS has the dropdown")
)]
const REOPEN_GUARD: Duration = Duration::from_millis(300);

struct Tray {
    model: Entity<TrayModel>,
    /// The model follows the feed, the OS and the files; `false` draws a
    /// fixture (`PWRROUTE_TRAY_FIXTURE`) that ignores them all.
    live: bool,
    /// The router Pause routing pauses, when the app has one (the window
    /// harness runs without).
    router: Option<RouterHandle>,
    /// The config's default browser, from the last time [`pause_target`]
    /// had to evaluate the config for it.
    config_default: ConfigDefault,
    native: Option<NativeTray>,
    /// The open dropdown, and when it last closed itself. macOS only.
    #[cfg_attr(
        not(target_os = "macos"),
        allow(dead_code, reason = "only macOS has the dropdown")
    )]
    dropdown: Option<WindowHandle<Dropdown>>,
    #[cfg_attr(
        not(target_os = "macos"),
        allow(dead_code, reason = "only macOS has the dropdown")
    )]
    dropdown_closed_at: Option<Instant>,
    /// Ticks every second while a send or a hold has a time left.
    clock: Option<gpui_kit::Task<()>>,
}

impl Global for Tray {}

/// The model both tray surfaces draw, once [`install`] has run.
pub fn model(cx: &App) -> Option<Entity<TrayModel>> {
    cx.try_global::<Tray>().map(|tray| tray.model.clone())
}

/// Put the tray icon up. A failure (no StatusNotifierItem host on a Linux
/// desktop, say) is reported and the app runs without it. Pause routing
/// pauses `router`; without one it only changes the tray.
pub fn install(router: Option<RouterHandle>, cx: &mut App) {
    dropdown::bind_keys(cx);
    let preset = fixture(SystemTime::now());
    let live = preset.is_none();
    let mut config_default = ConfigDefault::default();
    let model = cx.new(|_| {
        preset.unwrap_or_else(|| TrayModel::starting(live_pause_target(&mut config_default)))
    });
    if live {
        follow_feed(&model, cx);
    }
    let (events, received) = async_channel::unbounded();
    let native = match NativeTray::create(model.read(cx), events) {
        Ok(native) => Some(native),
        Err(error) => {
            log::warn!("no tray icon: {error}");
            None
        }
    };
    cx.observe(&model, |model, cx| {
        let model = model.read(cx).clone();
        if let Some(native) = &cx.global::<Tray>().native {
            native.sync(&model);
        }
    })
    .detach();
    cx.set_global(Tray {
        model,
        live,
        router,
        config_default,
        native,
        dropdown: None,
        dropdown_closed_at: None,
        clock: None,
    });
    cx.spawn(async move |cx| {
        while let Ok(event) = received.recv().await {
            cx.update(|cx| handle(event, cx));
        }
    })
    .detach();
    if std::env::var_os("PWRROUTE_TRAY_OPEN").is_some_and(|v| v == "1") {
        open_at_launch(cx);
    }
}

/// `PWRROUTE_TRAY_OPEN=1`: click the icon once the app is up, for
/// screenshots and manual checks where the item cannot be reached (behind
/// the notch on a full menu bar).
fn open_at_launch(cx: &mut App) {
    cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(Duration::from_millis(800))
            .await;
        cx.update(|cx| {
            let click = cx
                .global::<Tray>()
                .native
                .as_ref()
                .and_then(NativeTray::click_here);
            if let Some(click) = click {
                handle(click, cx);
            }
        });
    })
    .detach();
}

/// Where Pause routing would send every link, as the files stand now:
/// [`pause_target`] over `rules.json` and the config the router routes
/// with.
fn live_pause_target(config_default: &mut ConfigDefault) -> BrowserLabel {
    let rules = discovery::rules_path(&discovery::Environment::current())
        .and_then(|path| RulesFile::load(&path).ok());
    let source = crate::router::routing_source();
    pause_target(rules.as_ref(), source.config.as_deref(), config_default)
}

/// Where a pause sends every link: `rules.json`'s `defaultBrowser` (with
/// its `defaultProfile`), else the Finicky config's own `defaultBrowser`
/// when it names a browser rather than a function, else the router's
/// fallback browser. The same chain as the first-run window's "Fallback
/// browser" step describes. Don't open (`"none"`) is no place to pause
/// to: a pause opens every link, so it is passed over like an empty one.
fn pause_target(
    rules: Option<&RulesFile>,
    config: Option<&Path>,
    config_default: &mut ConfigDefault,
) -> BrowserLabel {
    if let Some(rules) = rules.filter(|rules| {
        !rules.default_browser.trim().is_empty()
            && !pwrroute_engine::is_dont_open(&rules.default_browser)
    }) {
        return label(&rules.default_browser, &rules.default_profile);
    }
    config_default
        .label(config)
        .unwrap_or_else(|| label(FALLBACK_BROWSER, ""))
}

/// The config's default browser, kept with the file's modification time
/// so the config is evaluated again only when it changes: opening the
/// dropdown is frequent, loading a config is not free.
#[derive(Default)]
struct ConfigDefault {
    config: Option<PathBuf>,
    modified: Option<SystemTime>,
    label: Option<BrowserLabel>,
}

impl ConfigDefault {
    /// The config's `defaultBrowser` when it names a browser; `None` for a
    /// function, a config that does not load, or no config.
    fn label(&mut self, config: Option<&Path>) -> Option<BrowserLabel> {
        let modified = config.and_then(|path| fs::metadata(path).ok()?.modified().ok());
        if self.config.as_deref() != config || self.modified != modified {
            self.config = config.map(Path::to_path_buf);
            self.modified = modified;
            self.label = config.and_then(config_default);
        }
        self.label.clone()
    }
}

/// Evaluates the config at `path` for its `defaultBrowser`, as the Rules
/// screen does to list its rules. With the native host, so a config that
/// asks the OS while it loads names the default it routes with; its
/// console output goes nowhere.
fn config_default(path: &Path) -> Option<BrowserLabel> {
    let engine = Engine::load(path, Rc::new(NativeHost::new()), Limits::default()).ok()?;
    match engine.describe().ok()??.default_browser {
        BrowserSpec::Browser(fixed) if fixed.app_type != AppType::None => {
            Some(label(&fixed.name, &fixed.profile))
        }
        _ => None,
    }
}

/// `browser` as `rules.json`, a config or the router names it, labeled
/// with the name the reader knows: the catalog's, else the installed
/// app's, else the id as written. First run saves a bundle id.
fn label(browser: &str, profile: &str) -> BrowserLabel {
    BrowserLabel::new(
        badge_for(browser),
        &default_browser::browser_name(browser),
        Some(profile).filter(|profile| !profile.is_empty()),
    )
    .for_app(browser)
}

/// The label's browser and profile, as the router is told to open them.
fn target_of(label: &BrowserLabel) -> PauseTarget {
    PauseTarget {
        browser: label.app().to_owned(),
        profile: label.profile.clone().unwrap_or_default(),
    }
}

/// Bring the model's pause target up to date: the target in force while
/// paused, else where a pause would send links now. A fixture keeps its
/// own.
fn refresh_pause_target(model: &Entity<TrayModel>, cx: &mut App) {
    if !cx.try_global::<Tray>().is_some_and(|tray| tray.live) {
        return;
    }
    let target = {
        let tray = cx.global_mut::<Tray>();
        match tray.router.as_ref().and_then(RouterHandle::paused) {
            Some(target) => label(&target.browser, &target.profile),
            None => live_pause_target(&mut tray.config_default),
        }
    };
    model.update(cx, |model, cx| {
        if model.pause_target != target {
            // Until the user picks one, Send offers the Everything
            // else browser.
            if !model.destination_chosen && model.sending().is_none() {
                model.destination = SendDestination::profile(target.clone());
            }
            model.pause_target = target;
            cx.notify();
        }
    });
}

/// Keep the header chip at what the OS says about the default browser.
/// Call after `default_browser::install`.
pub fn follow_default_browser(cx: &mut App) {
    let Some(registrar) = default_browser::registrar(cx) else {
        return;
    };
    let Some(model) = cx
        .try_global::<Tray>()
        .filter(|tray| tray.live)
        .map(|tray| tray.model.clone())
    else {
        return;
    };
    fn sync(model: &Entity<TrayModel>, is_default: bool, cx: &mut App) {
        model.update(cx, |model, cx| {
            if model.is_default_browser != is_default {
                model.is_default_browser = is_default;
                cx.notify();
            }
        });
    }
    sync(&model, registrar.read(cx).state.is_default(), cx);
    cx.observe(&registrar, move |registrar, cx| {
        let is_default = registrar.read(cx).state.is_default();
        sync(&model, is_default, cx);
    })
    .detach();
}

/// `PWRROUTE_TRAY_FIXTURE=routing` draws the design's 1a and `paused` its
/// 1b, for screenshots and manual checks; the model then ignores the feed
/// and the OS.
fn fixture(now: SystemTime) -> Option<TrayModel> {
    match std::env::var("PWRROUTE_TRAY_FIXTURE").as_deref() {
        Ok("routing") => Some(TrayModel::fixture(now)),
        Ok("paused") => Some(TrayModel::paused_fixture(now)),
        _ => None,
    }
}

/// Keep the recents and the count at what the router has dispatched.
fn follow_feed(model: &Entity<TrayModel>, cx: &mut App) {
    let Some(feed) = activity::feed(cx) else {
        return;
    };
    fn sync(model: &Entity<TrayModel>, feed: &Entity<Feed>, cx: &mut App) {
        let dispatched = Dispatched::of(feed.read(cx), SystemTime::now());
        model.update(cx, |model, cx| {
            model.sync(dispatched);
            cx.notify();
        });
        // A dispatch may have used up the last of a count.
        sync_override(model, cx);
    }
    sync(model, &feed, cx);
    let model = model.clone();
    cx.observe(&feed, move |feed, cx| sync(&model, &feed, cx))
        .detach();
}

fn handle(event: TrayEvent, cx: &mut App) {
    match event {
        TrayEvent::Command(command) => run(command, cx),
        TrayEvent::Click {
            position,
            size,
            pointer,
        } => click(position, size, pointer, cx),
    }
}

/// A left click on the icon.
#[cfg(target_os = "macos")]
fn click(position: (f64, f64), size: (f64, f64), pointer: Option<(f64, f64)>, cx: &mut App) {
    let tray = cx.global_mut::<Tray>();
    if let Some(window) = tray.dropdown.take() {
        tray.dropdown_closed_at = Some(Instant::now());
        let _ = window.update(cx, |_, window, _| window.remove_window());
        return;
    }
    if tray
        .dropdown_closed_at
        .is_some_and(|at| at.elapsed() < REOPEN_GUARD)
    {
        return;
    }
    let model = tray.model.clone();
    // The chip and the Make default row draw what the OS says now, not what
    // it said at the last tick.
    default_browser::refresh(false, cx);
    // The pause row names where links would go now, not at launch.
    refresh_pause_target(&model, cx);
    refresh_destinations(&model, cx);
    refresh_can_hold(&model, cx);
    sync_override(&model, cx);
    let anchor = crate::macos::status_item_anchor(position, size, pointer);
    let (origin, display, max_height) = match &anchor {
        Some(anchor) => {
            let origin = crate::macos::dropdown_origin(anchor, crate::theme::px(dropdown::WIDTH));
            (
                origin,
                Some(anchor.display),
                crate::macos::dropdown_max_height(anchor, origin),
            )
        }
        // No screen to read (off the main thread): under the menu bar at
        // the left, as tall as the card.
        None => (
            gpui_kit::point(gpui_kit::px(0.0), gpui_kit::px(28.0)),
            None,
            gpui_kit::px(f32::MAX),
        ),
    };
    match dropdown::open(model, origin, display, max_height, cx) {
        Ok(window) => cx.global_mut::<Tray>().dropdown = Some(window),
        Err(error) => log::warn!("could not open the tray dropdown: {error}"),
    }
}

/// A left click on the icon: Windows and Linux open the window, as a
/// notification-area icon does; the menu is the right click.
#[cfg(not(target_os = "macos"))]
fn click(_position: (f64, f64), _size: (f64, f64), _pointer: Option<(f64, f64)>, cx: &mut App) {
    run(TrayCommand::OpenMain, cx);
}

/// The dropdown closed itself.
fn dropdown_closed(cx: &mut App) {
    if cx.has_global::<Tray>() {
        let tray = cx.global_mut::<Tray>();
        tray.dropdown = None;
        tray.dropdown_closed_at = Some(Instant::now());
    }
}

/// Do what a row or menu item says. About is Settings › About, as the app
/// menu's About PwrRoute is (the family's decision A).
pub fn run(command: TrayCommand, cx: &mut App) {
    match command {
        TrayCommand::OpenMain => crate::show_main_window(cx),
        TrayCommand::About => crate::settings::open(cx, Some(crate::settings::Pane::About)),
        TrayCommand::Settings => crate::settings::open(cx, None),
        TrayCommand::Rules => show_screen(Screen::Rules, cx),
        TrayCommand::TestLink => show_screen(Screen::TestLink, cx),
        TrayCommand::Activity => show_screen(Screen::Activity, cx),
        TrayCommand::TogglePause => {
            if let Some(model) = model(cx) {
                toggle_pause(&model, cx);
            }
        }
        TrayCommand::SendRules => set_mode(Mode::Rules, cx),
        // The native menu's rows: one link, or an hour.
        TrayCommand::SendNext => start(Mode::Send, Some(ForKind::Links), Some(1), cx),
        TrayCommand::HoldNext => start(Mode::Hold, Some(ForKind::Links), Some(1), cx),
        TrayCommand::HoldHour => {
            if let Some(model) = model(cx) {
                model.update(cx, |model, _| {
                    model.time_step = model::DEFAULT_TIME_STEP;
                });
            }
            start(Mode::Hold, Some(ForKind::Time), None, cx);
        }
        TrayCommand::ChooseWhere => show_screen(Screen::Overview, cx),
        TrayCommand::HeldLinks => show_screen(Screen::Activity, cx),
        // A click, as the Settings button is: the one way to the OS.
        TrayCommand::MakeDefault => {
            default_browser::handle_settings_request(
                &crate::settings::model::SettingsRequest::RegisterDefaultBrowser,
                cx,
            );
        }
        TrayCommand::Quit => crate::app_menu::quit(cx),
    }
}

/// Bring the main window forward, opening it if need be, on `screen`.
pub fn show_screen(screen: Screen, cx: &mut App) {
    show_screen_on(screen, None, cx);
}

/// [`show_screen`], opening the window on `display` when it is closed:
/// first run hands over on the display its own window was on.
pub fn show_screen_on(screen: Screen, display: Option<gpui_kit::DisplayId>, cx: &mut App) {
    crate::show_main_window_on(display, cx);
    let main = cx
        .windows()
        .iter()
        .find_map(|window| window.downcast::<PwrRoute>());
    if let Some(main) = main {
        let _ = main.update(cx, |app, window, cx| app.navigate(screen, window, cx));
    }
}

/// Pause routing, or resume it: the router first, then the model both
/// surfaces draw. A pause takes the target as the files name it now, and
/// keeps it until resumed.
fn toggle_pause(model: &Entity<TrayModel>, cx: &mut App) {
    let pausing = !model.read(cx).is_paused();
    set_mode(if pausing { Mode::Pause } else { Mode::Rules }, cx);
}

/// The tray over `model` and `router`, without an icon or the files: for
/// window tests that drive Incoming links through the Overview.
#[cfg(test)]
pub fn install_for_test(
    model: TrayModel,
    router: Option<RouterHandle>,
    cx: &mut App,
) -> Entity<TrayModel> {
    let model = cx.new(|_| model);
    cx.set_global(Tray {
        model: model.clone(),
        live: false,
        router,
        config_default: ConfigDefault::default(),
        native: None,
        dropdown: None,
        dropdown_closed_at: None,
        clock: None,
    });
    model
}

/// The router the switch drives, when the app has one.
fn router(cx: &App) -> Option<RouterHandle> {
    cx.try_global::<Tray>().and_then(|tray| tray.router.clone())
}

/// A segment of the Incoming links switch: Rules, Send, Hold or Pause,
/// with the destination and the For row as they stand. The router first,
/// then the model.
pub fn set_mode(mode: Mode, cx: &mut App) {
    start(mode, None, None, cx);
}

/// Turns `mode` on, with `kind` and a count of `next` in place of the For
/// row's when given (the native menu's fixed rows).
fn start(mode: Mode, kind: Option<ForKind>, next: Option<u32>, cx: &mut App) {
    let Some(model) = model(cx) else {
        return;
    };
    if mode == Mode::Hold && !model.read(cx).can_hold {
        return;
    }
    if mode == Mode::Pause {
        refresh_pause_target(&model, cx);
    }
    let now = SystemTime::now();
    let (routing, wanted) = {
        let model = model.read(cx);
        let scope = match (kind.unwrap_or(model.for_kind), next) {
            (ForKind::Links, Some(n)) => SendScope::Next { left: n, of: n },
            (kind, _) => model.scope_for(kind, now),
        };
        match mode {
            Mode::Rules => (Routing::Active, None),
            Mode::Pause => (
                Routing::Paused,
                Some(Override::Paused(target_of(&model.pause_target))),
            ),
            Mode::Send => (
                Routing::Sending(scope),
                Some(Override::Sending(SendTo {
                    destination: model.destination.target(),
                    scope,
                })),
            ),
            Mode::Hold => (Routing::Holding(scope), Some(Override::Holding(scope))),
        }
    };
    if let Some(router) = router(cx) {
        router.set_override(wanted);
    }
    model.update(cx, |model, cx| {
        model.routing = routing;
        if let Some(kind) = kind {
            model.for_kind = kind;
        }
        cx.notify();
    });
    follow_clock(&model, cx);
}

/// The Overview node's Next link here: always the one next link.
pub fn send_next_to(destination: SendDestination, cx: &mut App) {
    let Some(model) = model(cx) else {
        return;
    };
    model.update(cx, |model, _| {
        model.destination = destination;
        model.destination_chosen = true;
    });
    start(Mode::Send, Some(ForKind::Links), Some(1), cx);
}

pub fn choose_destination(destination: SendDestination, cx: &mut App) {
    let Some(model) = model(cx) else {
        return;
    };
    if let Some(router) = router(cx) {
        router.retarget_send(destination.target());
    }
    model.update(cx, |model, cx| {
        model.destination = destination;
        model.destination_chosen = true;
        cx.notify();
    });
}

/// A For choice. While Send or Hold is on it applies from the next link:
/// a count starts again, a time from now.
pub fn set_for(kind: ForKind, cx: &mut App) {
    let Some(model) = model(cx) else {
        return;
    };
    model.update(cx, |model, cx| {
        model.for_kind = kind;
        cx.notify();
    });
    let scope = model.read(cx).scope_for(kind, SystemTime::now());
    rescope(&model, scope, cx);
}

/// The stepper, or Up and Down on it: a count of links, or a time. A
/// running count keeps what was used (1 of 3 used, stepped to 5, is 4 of 5
/// left); a running time starts again from now.
pub fn step(delta: i64, cx: &mut App) {
    let Some(model) = model(cx) else {
        return;
    };
    let (kind, running) = {
        let model = model.read(cx);
        (model.for_shown(), model.scope())
    };
    match kind {
        ForKind::Links => {
            let stepped = match running {
                Some(scope @ SendScope::Next { .. }) => {
                    let stepped = router(cx)
                        .and_then(|router| router.step_next(delta))
                        .and_then(|now| now.scope())
                        .unwrap_or_else(|| scope.stepped(delta));
                    model.update(cx, |model, _| set_scope(model, stepped));
                    stepped
                }
                _ => {
                    let shown = i64::from(model.read(cx).next_count);
                    let n = (shown + delta).clamp(1, i64::from(pwrroute_router::MAX_NEXT));
                    let n = u32::try_from(n).unwrap_or(1);
                    SendScope::Next { left: n, of: n }
                }
            };
            if let SendScope::Next { of, .. } = stepped {
                model.update(cx, |model, cx| {
                    model.next_count = of;
                    cx.notify();
                });
            }
        }
        ForKind::Time => {
            model.update(cx, |model, cx| {
                let last = i64::try_from(TIME_STEPS.len() - 1).unwrap_or(0);
                let at = i64::try_from(model.time_step).unwrap_or(0);
                model.time_step = usize::try_from((at + delta).clamp(0, last)).unwrap_or(0);
                cx.notify();
            });
            if running.is_some() {
                let scope = model.read(cx).scope_for(ForKind::Time, SystemTime::now());
                rescope(&model, scope, cx);
            }
        }
        ForKind::UntilOff => {}
    }
}

/// Gives the send or hold in force `scope`, when one is on.
fn rescope(model: &Entity<TrayModel>, scope: SendScope, cx: &mut App) {
    if model.read(cx).scope().is_none() {
        return;
    }
    let scope = router(cx)
        .and_then(|router| router.rescope(scope))
        .and_then(|now| now.scope())
        .unwrap_or(scope);
    model.update(cx, |model, cx| {
        set_scope(model, scope);
        cx.notify();
    });
    follow_clock(model, cx);
}

fn set_scope(model: &mut TrayModel, scope: SendScope) {
    match &mut model.routing {
        Routing::Sending(running) | Routing::Holding(running) => *running = scope,
        Routing::Active | Routing::Paused => {}
    }
}

/// What the router's slot holds, as the tray draws it.
fn routing_of(slot: Option<&Override>) -> Routing {
    match slot {
        None => Routing::Active,
        Some(Override::Paused(_)) => Routing::Paused,
        Some(Override::Sending(send)) => Routing::Sending(send.scope),
        Some(Override::Holding(scope)) => Routing::Holding(*scope),
    }
}

/// Reads the router's slot back: it counts links down and ends a time that
/// ran out.
pub fn sync_override(model: &Entity<TrayModel>, cx: &mut App) {
    if !cx.try_global::<Tray>().is_some_and(|tray| tray.live) {
        return;
    }
    let Some(router) = router(cx) else {
        return;
    };
    let routing = routing_of(router.override_in_force().as_ref());
    model.update(cx, |model, cx| {
        if model.routing != routing {
            model.routing = routing;
            cx.notify();
        }
    });
}

/// Hold needs somewhere to keep what it holds: with Activity keeping
/// nothing in memory, a held link could never be opened.
#[cfg_attr(
    not(target_os = "macos"),
    allow(dead_code, reason = "only macOS has the dropdown")
)]
fn refresh_can_hold(model: &Entity<TrayModel>, cx: &mut App) {
    let Some(router) = router(cx) else {
        return;
    };
    let can_hold = router.retention().memory.count > 0;
    model.update(cx, |model, cx| {
        if model.can_hold != can_hold {
            model.can_hold = can_hold;
            cx.notify();
        }
    });
}

/// Every second while a send or a hold has a time left: the countdown,
/// and the end of it.
fn follow_clock(model: &Entity<TrayModel>, cx: &mut App) {
    let timed =
        |model: &TrayModel| matches!(model.scope(), Some(SendScope::All { until: Some(_) }));
    let live = cx.try_global::<Tray>().is_some_and(|tray| tray.live);
    let clock = (live && timed(model.read(cx))).then(|| {
        let model = model.clone();
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let ticking = cx.update(|cx| {
                    sync_override(&model, cx);
                    model.update(cx, |_, cx| cx.notify());
                    timed(model.read(cx))
                });
                if !ticking {
                    break;
                }
            }
        })
    });
    if cx.has_global::<Tray>() {
        cx.global_mut::<Tray>().clock = clock;
    }
}

/// Opens a held link: where the rules send it now, or where the user
/// picked. The router does it on its own thread; the feed then says it no
/// longer waits.
pub fn open_held(id: DispatchId, how: OpenAs, cx: &mut App) {
    open_held_all(vec![(id, how)], cx);
}

/// Open all: every held link the tray lists, each where the rules send it.
pub fn open_all_held(cx: &mut App) {
    let Some(model) = model(cx) else {
        return;
    };
    let ids = model
        .read(cx)
        .held
        .iter()
        .map(|held| (held.id, OpenAs::Rules))
        .collect();
    open_held_all(ids, cx);
}

fn open_held_all(links: Vec<(DispatchId, OpenAs)>, cx: &mut App) {
    let Some(router) = router(cx) else {
        return;
    };
    cx.background_executor()
        .spawn(async move {
            for (id, how) in links {
                if let Err(error) = router.open_not_opened(id, how) {
                    log::warn!("could not open held link {}: {error}", id.0);
                }
            }
        })
        .detach();
}

/// Make a rule…: the main window on Activity, with Add a rule open for the
/// held link's record.
pub fn make_rule_for(id: DispatchId, cx: &mut App) {
    show_screen(Screen::Activity, cx);
    let main = cx
        .windows()
        .iter()
        .find_map(|window| window.downcast::<PwrRoute>());
    if let Some(main) = main {
        let key = crate::activity::RowKey {
            id,
            journaled: false,
        };
        let _ = main.update(cx, |app, window, cx| app.add_rule(key, window, cx));
    }
}

/// Copy link: the held link as its record keeps it.
pub fn copy_held(id: DispatchId, cx: &mut App) {
    let url = activity::feed(cx).and_then(|feed| {
        feed.read(cx)
            .records()
            .iter()
            .find(|record| record.id == id)
            .map(|record| record.link.url.clone())
    });
    if let Some(url) = url {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(url));
    }
}

/// What the destination lanes offer, as the scan has it now.
#[cfg_attr(
    not(target_os = "macos"),
    allow(dead_code, reason = "only macOS has the dropdown")
)]
fn refresh_destinations(model: &Entity<TrayModel>, cx: &mut App) {
    if !cx.try_global::<Tray>().is_some_and(|tray| tray.live) {
        return;
    }
    // The scan already leaves out PwrRoute and every other link router.
    let choices = DestinationChoices {
        browsers: crate::browser_scan::choices(cx),
        installed: crate::browser_scan::installed(cx),
        scanned: crate::browser_scan::has_run(cx),
    };
    model.update(cx, |model, cx| {
        if model.choices != choices {
            model.choices = choices;
            cx.notify();
        }
    });
}

/// A picked pill as the send's destination.
pub fn destination_of(choice: &crate::destinations::Choice) -> SendDestination {
    destination(&choice.browser, &choice.profile, choice.private)
}

/// A destination named as the Overview names a node: `browser` and
/// `profile` as written, or its private window.
pub fn destination(browser: &str, profile: &str, private: bool) -> SendDestination {
    if private {
        SendDestination {
            browser: label(browser, ""),
            private: crate::overview_model::private_term(browser)
                .or_else(|| Some("Private window".to_owned())),
        }
    } else {
        SendDestination::profile(label(browser, profile))
    }
}

#[cfg(test)]
mod target_tests {
    use std::time::Duration;

    use super::*;

    fn rules(default_browser: &str, default_profile: &str) -> RulesFile {
        RulesFile {
            default_browser: default_browser.into(),
            default_profile: default_profile.into(),
            ..RulesFile::default()
        }
    }

    fn write_config(dir: &Path, default_browser: &str, modified: SystemTime) -> PathBuf {
        let path = dir.join("finicky.js");
        fs::write(
            &path,
            format!("export default {{ defaultBrowser: {default_browser}, handlers: [] }};"),
        )
        .unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        path
    }

    #[test]
    fn rules_json_names_the_target_first_with_its_profile() {
        let mut cache = ConfigDefault::default();
        let target = pause_target(Some(&rules("Google Chrome", "Personal")), None, &mut cache);
        assert_eq!(
            target,
            BrowserLabel::new(
                badge_for("Google Chrome"),
                "Google Chrome",
                Some("Personal")
            )
        );
        // A bundle id, as first run saves it: named for the reader, and
        // the router still gets the id.
        let bundle = pause_target(Some(&rules("com.google.Chrome", "")), None, &mut cache);
        assert_eq!(
            bundle,
            BrowserLabel::new(badge_for("com.google.Chrome"), "Google Chrome", None)
                .for_app("com.google.Chrome")
        );
        assert_eq!(target_of(&bundle).browser, "com.google.Chrome");
        // Named in rules.json, the config is not consulted at all.
        assert!(cache.config.is_none());
    }

    #[test]
    fn a_dont_open_default_is_not_a_pause_target() {
        // A pause opens every link; Everything else on Don't open has no
        // browser to give it, so the chain goes on to the fallback.
        let mut cache = ConfigDefault::default();
        let target = pause_target(Some(&rules("none", "Work")), None, &mut cache);
        assert_eq!(target_of(&target).browser, FALLBACK_BROWSER);
        assert_eq!(target_of(&target).profile, "");
    }

    /// First run saves the default as a bundle id. The pause line and the
    /// tooltip name the browser as the reader knows it; the router is told
    /// the id, as before.
    #[test]
    fn the_pause_line_names_a_bundle_id_default_by_its_name() {
        let mut cache = ConfigDefault::default();
        let target = pause_target(Some(&rules("com.microsoft.edgemac", "")), None, &mut cache);
        let mut model = TrayModel::fixture(SystemTime::now());
        model.pause_target = target.clone();
        model.set_paused(true);
        assert_eq!(
            model::runs_text(&model.live(SystemTime::now()).unwrap().runs),
            "Every link to Microsoft Edge. Rewrites still run."
        );
        assert!(
            !model.tooltip().contains("com.microsoft"),
            "{}",
            model.tooltip()
        );
        assert_eq!(
            target_of(&target),
            PauseTarget {
                browser: "com.microsoft.edgemac".into(),
                profile: String::new(),
            }
        );
        // An id no catalog or app names is shown as written.
        let unknown = label("com.example.unlisted", "");
        assert_eq!(unknown.name, "com.example.unlisted");
        assert_eq!(target_of(&unknown).browser, "com.example.unlisted");
    }

    #[test]
    fn without_a_rules_default_the_configs_fixed_default_is_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let config = write_config(dir.path(), r#"{ name: "Firefox", profile: "Work" }"#, at);
        let mut cache = ConfigDefault::default();
        let target = pause_target(Some(&rules("", "")), Some(&config), &mut cache);
        assert_eq!(
            target,
            BrowserLabel::new(badge_for("Firefox"), "Firefox", Some("Work"))
        );
        assert_eq!(cache.config.as_deref(), Some(config.as_path()));

        // Unchanged, the config is not evaluated again: the cached label
        // answers even though the file now says otherwise.
        write_config(dir.path(), r#""Arc""#, at);
        let again = pause_target(None, Some(&config), &mut cache);
        assert_eq!(again, target);

        // Changed, it is.
        write_config(dir.path(), r#""Arc""#, at + Duration::from_secs(60));
        let changed = pause_target(None, Some(&config), &mut cache);
        assert_eq!(changed, BrowserLabel::new(badge_for("Arc"), "Arc", None));
    }

    #[test]
    fn a_function_default_a_broken_config_or_none_is_the_routers_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let fallback = label(FALLBACK_BROWSER, "");
        let mut cache = ConfigDefault::default();
        let function = write_config(dir.path(), r#"() => "Firefox""#, now);
        assert_eq!(
            pause_target(None, Some(&function), &mut cache),
            fallback,
            "a function picks per link: nothing to name"
        );
        let broken = dir.path().join("broken.js");
        fs::write(&broken, "export default {").unwrap();
        assert_eq!(pause_target(None, Some(&broken), &mut cache), fallback);
        assert_eq!(pause_target(None, None, &mut cache), fallback);
        assert_eq!(
            pause_target(None, Some(&dir.path().join("missing.js")), &mut cache),
            fallback
        );
    }

    /// The destination lanes: each browser the pickers offer, its
    /// profiles, then its private window in its word where it has one;
    /// the destination's pill lit, however the destination names its app.
    #[test]
    fn the_destination_lanes_follow_the_scan() {
        use crate::activity_actions::BrowserChoice;
        let mut model = TrayModel::starting(label("Google Chrome", "Work"));
        model.choices = DestinationChoices {
            browsers: vec![
                BrowserChoice {
                    name: "Google Chrome".into(),
                    profiles: vec!["Personal".into(), "Work".into()],
                },
                BrowserChoice {
                    name: "Safari".into(),
                    profiles: Vec::new(),
                },
            ],
            installed: vec![crate::overview_model::Installed {
                name: "Google Chrome".into(),
                bundle_id: Some("com.google.Chrome".into()),
                profiles: vec!["Personal".into(), "Work".into()],
                directories: Vec::new(),
                hidden: false,
            }],
            scanned: true,
        };
        let lanes = model.lanes();
        assert_eq!(
            lanes
                .iter()
                .map(|lane| lane.name.as_str())
                .collect::<Vec<_>>(),
            ["Google Chrome", "Safari"]
        );
        let lit: Vec<&str> = lanes
            .iter()
            .flat_map(|lane| lane.pills.iter().filter(|pill| pill.on))
            .map(|pill| pill.label.as_str())
            .collect();
        assert_eq!(lit, ["Work"], "the destination's pill, and only it");
        // A pill is the destination it names, private window included.
        let private = lanes[0]
            .pills
            .iter()
            .find(|pill| pill.kind == crate::destinations::PillKind::Private)
            .expect("Chrome opens a private window from a link");
        let picked = destination_of(&private.choice);
        assert_eq!(picked, destination("Google Chrome", "", true));
        model.destination = picked;
        let lit: Vec<String> = model
            .lanes()
            .iter()
            .flat_map(|lane| lane.pills.iter().filter(|pill| pill.on))
            .map(|pill| pill.label.clone())
            .collect();
        assert_eq!(lit, std::slice::from_ref(&private.label));
        // Named by its bundle id under a name the scan does not use, the
        // destination still lights its own lane, not a missing one.
        model.destination = SendDestination::profile(
            BrowserLabel::new("Ch", "Chrome", Some("Work")).for_app("com.google.Chrome"),
        );
        let lanes = model.lanes();
        assert_eq!(lanes.len(), 2, "no lane for the other name");
        assert!(
            lanes[0]
                .pills
                .iter()
                .any(|pill| pill.on && pill.label == "Work")
        );
        // A destination the scan lacks keeps a lane of its own.
        model.destination = destination("Vivaldi", "", false);
        assert_eq!(model.lanes()[0].name, "Vivaldi");
        assert!(model.lanes()[0].missing);
        // A node's destination, as the Overview names it.
        assert_eq!(
            destination("Google Chrome", "Work", false).target(),
            pwrroute_router::Destination {
                browser: "Google Chrome".into(),
                profile: "Work".into(),
                private: false,
            }
        );
        assert!(destination("Google Chrome", "", true).target().private);
    }

    #[test]
    fn the_target_is_what_the_router_is_told() {
        let target = target_of(&BrowserLabel::new(
            badge_for("Google Chrome"),
            "Google Chrome",
            Some("Personal"),
        ));
        assert_eq!(
            target,
            PauseTarget {
                browser: "Google Chrome".into(),
                profile: "Personal".into()
            }
        );
        assert_eq!(target_of(&label("Safari", "")).profile, "");
        // And back, for the dropdown while paused.
        assert_eq!(
            label(&target.browser, &target.profile),
            BrowserLabel::new(
                badge_for("Google Chrome"),
                "Google Chrome",
                Some("Personal")
            )
        );
    }
}
