//! The tray: the macOS menu bar item and its dropdown, or the Windows and
//! Linux notification-area icon and its native menu.
//!
//! [`install`] puts the icon up and owns a [`TrayModel`] entity that both
//! surfaces draw. Its recents and count follow the dispatch feed
//! (`activity::feed`) when one is installed; the pause state and whether
//! PwrRoute is the default browser wait for their wiring. Pausing from
//! either surface updates the model; whoever routes observes it.

mod dropdown;
mod icon;
pub mod model;
mod native;

#[cfg(test)]
mod tests;

use std::time::{Duration, Instant, SystemTime};

use gpui_kit::{App, AppContext as _, Entity, Global, WindowHandle};
use pwrroute_engine::{RulesFile, discovery};

use self::dropdown::Dropdown;
use self::model::{BrowserLabel, Dispatched, TrayModel};
use self::native::{NativeTray, TrayEvent};
use crate::activity::{self, Feed};
use crate::app::{PwrRoute, Screen};
use crate::default_browser;
use crate::widgets::badge_for;

/// What a tray row or menu item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    OpenMain,
    TestLink,
    Rules,
    Activity,
    Settings,
    About,
    TogglePause,
    Quit,
}

impl TrayCommand {
    const ALL: [Self; 8] = [
        Self::OpenMain,
        Self::TestLink,
        Self::Rules,
        Self::Activity,
        Self::Settings,
        Self::About,
        Self::TogglePause,
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
    /// The model follows the feed and the OS; `false` draws a fixture
    /// (`PWRROUTE_TRAY_FIXTURE`) that ignores both.
    live: bool,
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
}

impl Global for Tray {}

/// The model both tray surfaces draw, once [`install`] has run.
pub fn model(cx: &App) -> Option<Entity<TrayModel>> {
    cx.try_global::<Tray>().map(|tray| tray.model.clone())
}

/// Put the tray icon up. A failure (no StatusNotifierItem host on a Linux
/// desktop, say) is reported and the app runs without it.
pub fn install(cx: &mut App) {
    dropdown::bind_keys(cx);
    let preset = fixture(SystemTime::now());
    let live = preset.is_none();
    let model = cx.new(|_| preset.unwrap_or_else(|| TrayModel::starting(pause_target())));
    if live {
        follow_feed(&model, cx);
    }
    let (events, received) = async_channel::unbounded();
    let native = match NativeTray::create(model.read(cx), events) {
        Ok(native) => Some(native),
        Err(error) => {
            eprintln!("pwrroute: no tray icon: {error}");
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
        native,
        dropdown: None,
        dropdown_closed_at: None,
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

/// Where Pause routing would send every link: `rules.json`'s fallback, as
/// it stands at launch. (Pausing is not wired to the router yet.)
fn pause_target() -> BrowserLabel {
    let rules = discovery::rules_path(&discovery::Environment::current())
        .and_then(|path| RulesFile::load(&path).ok())
        .filter(|rules| !rules.default_browser.is_empty());
    match rules {
        Some(rules) => BrowserLabel::new(
            &badge_for(&rules.default_browser),
            &rules.default_browser,
            Some(rules.default_profile.as_str()).filter(|profile| !profile.is_empty()),
        ),
        None => BrowserLabel::new("", "the fallback browser", None),
    }
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
    }
    sync(model, &feed, cx);
    let model = model.clone();
    cx.observe(&feed, move |feed, cx| sync(&model, &feed, cx))
        .detach();
}

fn handle(event: TrayEvent, cx: &mut App) {
    match event {
        TrayEvent::Command(command) => run(command, cx),
        TrayEvent::Click { position, size } => click(position, size, cx),
    }
}

/// A left click on the icon.
#[cfg(target_os = "macos")]
fn click(position: (f64, f64), size: (f64, f64), cx: &mut App) {
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
    let anchor = crate::macos::status_item_anchor(position, size);
    let (origin, display) = match &anchor {
        Some(anchor) => (
            crate::macos::dropdown_origin(anchor, gpui_kit::px(dropdown::WIDTH)),
            Some(anchor.display),
        ),
        // Not on any screen we know: under the menu bar, top right.
        None => (gpui_kit::point(gpui_kit::px(0.0), gpui_kit::px(28.0)), None),
    };
    match dropdown::open(model, origin, display, cx) {
        Ok(window) => cx.global_mut::<Tray>().dropdown = Some(window),
        Err(error) => eprintln!("pwrroute: could not open the tray dropdown: {error}"),
    }
}

/// A left click on the icon: Windows and Linux open the window, as a
/// notification-area icon does; the menu is the right click.
#[cfg(not(target_os = "macos"))]
fn click(_position: (f64, f64), _size: (f64, f64), cx: &mut App) {
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

/// Do what a row or menu item says. About opens the main window: there is
/// no About panel yet.
pub fn run(command: TrayCommand, cx: &mut App) {
    match command {
        TrayCommand::OpenMain | TrayCommand::About => crate::show_main_window(cx),
        TrayCommand::Settings => crate::settings::open(cx, None),
        TrayCommand::Rules => show_screen(Screen::Rules, cx),
        TrayCommand::TestLink => show_screen(Screen::TestLink, cx),
        TrayCommand::Activity => show_screen(Screen::Activity, cx),
        TrayCommand::TogglePause => {
            if let Some(model) = model(cx) {
                toggle_pause(&model, cx);
            }
        }
        TrayCommand::Quit => crate::app_menu::quit(cx),
    }
}

/// Bring the main window forward, opening it if need be, on `screen`.
pub fn show_screen(screen: Screen, cx: &mut App) {
    crate::show_main_window(cx);
    let main = cx
        .windows()
        .iter()
        .find_map(|window| window.downcast::<PwrRoute>());
    if let Some(main) = main {
        let _ = main.update(cx, |app, window, cx| app.navigate(screen, window, cx));
    }
}

fn toggle_pause(model: &Entity<TrayModel>, cx: &mut App) {
    model.update(cx, |model, cx| {
        model.set_paused(!model.is_paused());
        cx.notify();
    });
}
