//! The tray: the macOS menu bar item and its dropdown, or the Windows and
//! Linux notification-area icon and its native menu.
//!
//! [`install`] puts the icon up and owns a [`TrayModel`] entity that both
//! surfaces draw. Its recents and count follow the dispatch feed
//! (`activity::feed`) when one is installed, and its header chip the OS
//! ([`follow_default_browser`]). Pausing from either surface pauses the
//! router ([`RouterHandle::set_paused`]) with the target the dropdown
//! names, read from the files each time the dropdown opens or a pause
//! begins ([`pause_target`]). Send links ([`set_send_mode`] and the
//! rest, design: Send Links) sends the next few links, or all of them, to
//! a destination the user picks; the router counts them down
//! ([`RouterHandle::set_send_to`]) and the model follows it after every
//! dispatch ([`sync_send`]). A pause and a send share the router's one
//! slot, so turning one on turns the other off.

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
use pwrroute_router::{FALLBACK_BROWSER, PauseTarget, RouterHandle, SendScope};

use self::dropdown::Dropdown;
use self::model::{
    BrowserLabel, DestinationChoices, Dispatched, Routing, SendDestination, SendMode, TrayModel,
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
    /// Send links: back to the rules (ends a pause too).
    SendRules,
    /// Send links: the next link to the destination.
    SendNext,
    /// Send links: all links to the destination.
    SendAll,
    /// The native menu's "Choose where…": the Overview, whose nodes offer
    /// Next link here.
    ChooseWhere,
    /// Ask the OS to make PwrRoute the default browser: the dropdown's row
    /// while another app has it.
    MakeDefault,
    Quit,
}

impl TrayCommand {
    const ALL: [Self; 13] = [
        Self::OpenMain,
        Self::TestLink,
        Self::Rules,
        Self::Activity,
        Self::Settings,
        Self::About,
        Self::TogglePause,
        Self::SendRules,
        Self::SendNext,
        Self::SendAll,
        Self::ChooseWhere,
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
            Self::SendAll => "pwrroute.send-all",
            Self::ChooseWhere => "pwrroute.send-choose",
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
    /// Ticks while All links runs for an hour ([`follow_send_clock`]);
    /// dropped, it stops.
    send_clock: Option<gpui_kit::Task<()>>,
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
        send_clock: None,
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
/// browser" step describes.
fn pause_target(
    rules: Option<&RulesFile>,
    config: Option<&Path>,
    config_default: &mut ConfigDefault,
) -> BrowserLabel {
    if let Some(rules) = rules.filter(|rules| !rules.default_browser.is_empty()) {
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
            // Until the user picks one, Send links offers the Everything
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
        // A dispatch may have used up the last of Next link.
        sync_send(model, cx);
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
    sync_send(&model, cx);
    let anchor = crate::macos::status_item_anchor(position, size, pointer);
    let (origin, display, max_height) = match &anchor {
        Some(anchor) => {
            let origin = crate::macos::dropdown_origin(anchor, gpui_kit::px(dropdown::WIDTH));
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
        TrayCommand::SendRules => set_send_mode(SendMode::Rules, cx),
        // The native menu's rows: one link, and the hour as last set.
        TrayCommand::SendNext => start_send(SendMode::Next, Some(1), cx),
        TrayCommand::SendAll => set_send_mode(SendMode::All, cx),
        TrayCommand::ChooseWhere => show_screen(Screen::Overview, cx),
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
    if pausing {
        refresh_pause_target(model, cx);
    }
    if let Some(router) = cx.try_global::<Tray>().and_then(|tray| tray.router.clone()) {
        router.set_paused(pausing.then(|| target_of(&model.read(cx).pause_target)));
    }
    model.update(cx, |model, cx| {
        model.set_paused(pausing);
        cx.notify();
    });
}

/// The tray over `model` and `router`, without an icon or the files: for
/// window tests that drive Send links through the Overview.
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
        send_clock: None,
    });
    model
}

/// The router Send links drives, when the app has one.
fn router(cx: &App) -> Option<RouterHandle> {
    cx.try_global::<Tray>().and_then(|tray| tray.router.clone())
}

/// Turn Send links to `mode`: Rules ends a send and a pause; Next link and
/// All links send to the model's destination, with the stepper's count or
/// the hour, ending a pause. The router first, then the model.
pub fn set_send_mode(mode: SendMode, cx: &mut App) {
    start_send(mode, None, cx);
}

/// [`set_send_mode`], with Next link taking `next` links instead of the
/// stepper's count, which stays as the user set it.
fn start_send(mode: SendMode, next: Option<u32>, cx: &mut App) {
    let Some(model) = model(cx) else {
        return;
    };
    let mut send = model.read(cx).send_for(mode, SystemTime::now());
    if let (Some(send), Some(n)) = (send.as_mut(), next)
        && matches!(send.scope, SendScope::Next { .. })
    {
        send.scope = SendScope::Next { left: n, of: n };
    }
    if let Some(router) = router(cx) {
        match &send {
            Some(send) => router.set_send_to(Some(send.clone())),
            None => {
                router.set_send_to(None);
                router.set_paused(None);
            }
        }
    }
    model.update(cx, |model, cx| {
        model.routing = match send {
            Some(send) => Routing::Sending(send.scope),
            None => Routing::Active,
        };
        cx.notify();
    });
    follow_send_clock(&model, cx);
}

/// Send the next link to `destination`: the Overview's Next link here
/// (design: Send Links 2c). Always one link, whatever the stepper says.
pub fn send_next_to(destination: SendDestination, cx: &mut App) {
    let Some(model) = model(cx) else {
        return;
    };
    model.update(cx, |model, _| {
        model.destination = destination;
        model.destination_chosen = true;
    });
    start_send(SendMode::Next, Some(1), cx);
}

/// Pick where Send links sends links. A send in force goes there from the
/// next link on, keeping its count or time.
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

/// The stepper: while Next link is on, more or fewer links left (the
/// router changes the count under its lock); otherwise how many it will
/// take.
pub fn step_next(delta: i64, cx: &mut App) {
    let Some(model) = model(cx) else {
        return;
    };
    let running = matches!(model.read(cx).sending(), Some(SendScope::Next { .. }));
    let router = router(cx);
    model.update(cx, |model, cx| {
        let shown = i64::from(model.next_shown());
        let wanted = (shown + delta).clamp(1, i64::from(pwrroute_router::MAX_NEXT));
        let wanted = u32::try_from(wanted).unwrap_or(1);
        model.next_count = wanted;
        if running {
            let scope = match router.and_then(|router| router.add_to_next(delta)) {
                Some(send) => send.scope,
                // No router (the window harness): the model counts alone,
                // as the router would.
                None => match model.sending() {
                    Some(scope) => scope.stepped(delta),
                    None => return,
                },
            };
            model.routing = Routing::Sending(scope);
        }
        cx.notify();
    });
}

/// All links' length: until switched back, or for [`model::ALL_FOR`]. While
/// All links is on, the hour starts again from now.
pub fn set_all_for_hour(for_hour: bool, cx: &mut App) {
    let Some(model) = model(cx) else {
        return;
    };
    let running = matches!(model.read(cx).sending(), Some(SendScope::All { .. }));
    model.update(cx, |model, cx| {
        model.all_for_hour = for_hour;
        cx.notify();
    });
    if running {
        set_send_mode(SendMode::All, cx);
    }
}

/// Bring the model's send up to date with the router's: a Next link
/// counted down by a dispatch, used up, or an hour that ran out. A fixture
/// or a tray without a router keeps its own.
pub fn sync_send(model: &Entity<TrayModel>, cx: &mut App) {
    if !cx.try_global::<Tray>().is_some_and(|tray| tray.live) {
        return;
    }
    let Some(router) = router(cx) else {
        return;
    };
    let routing = match (router.send_to(), router.paused()) {
        (Some(send), _) => Routing::Sending(send.scope),
        (None, Some(_)) => Routing::Paused,
        (None, None) => Routing::Active,
    };
    model.update(cx, |model, cx| {
        if model.routing != routing {
            model.routing = routing;
            cx.notify();
        }
    });
}

/// While All links runs for an hour, check every second so the line
/// counts down and every surface goes back to Rules within a second of the
/// hour running out (design: Send Links 3a). The clock runs only then: it
/// stops itself, and any other mode drops it.
fn follow_send_clock(model: &Entity<TrayModel>, cx: &mut App) {
    let timed =
        |model: &TrayModel| matches!(model.sending(), Some(SendScope::All { until: Some(_) }));
    let live = cx.try_global::<Tray>().is_some_and(|tray| tray.live);
    let clock = (live && timed(model.read(cx))).then(|| {
        let model = model.clone();
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let ticking = cx.update(|cx| {
                    sync_send(&model, cx);
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
        cx.global_mut::<Tray>().send_clock = clock;
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

    /// First run saves the default as a bundle id. The pause line and the
    /// tooltip name the browser as the reader knows it; the router is told
    /// the id, as before.
    #[test]
    fn the_pause_line_names_a_bundle_id_default_by_its_name() {
        let mut cache = ConfigDefault::default();
        let target = pause_target(Some(&rules("com.microsoft.edgemac", "")), None, &mut cache);
        let mut model = TrayModel::fixture(SystemTime::now());
        model.pause_target = target.clone();
        assert_eq!(
            model.pause_detail(),
            "Send every link to Microsoft Edge until switched back."
        );
        model.set_paused(true);
        assert_eq!(
            model.pause_detail(),
            "Everything goes to Microsoft Edge. Rewrites still run."
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
