//! The Settings window: its own window, one at a time, as in PwrSnap and
//! PwrGit. A 208px nav on the left, and panes of cards at most 760px wide.
//!
//! The window draws a [`SettingsModel`] from [`SettingsHost`] and reports
//! what the reader asks for as [`SettingsRequest`]s to its handler. The
//! shipped app installs `settings_live`, which builds every pane from its
//! producer (`*_host` here) and routes each request to the one that
//! answers it; without a host the window draws [`fixtures`] and the
//! handler only logs. A producer that answers later goes through
//! [`update`].

mod about;
mod browsers;
pub mod browsers_host;
mod configuration;
pub mod configuration_host;
mod controls;
mod diagnostics;
pub mod fixtures;
mod local_agents;
pub mod model;
pub mod nav;
pub mod privacy;
pub mod privacy_host;
mod short_links;
mod style;
mod sync;
pub mod sync_host;
pub mod system_host;
#[cfg(test)]
mod tests;
mod updates;
pub mod updates_host;

use std::collections::HashSet;
use std::rc::Rc;

use crate::theme::px;
use gpui_kit::base::{Button, TestSupportExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, ClipboardItem, Context, Div, ElementId, Entity,
    FocusHandle, Global, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _,
    Render, ScrollAnchor, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _,
    TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions, div, point,
};

use self::controls::{Heading, Kind, button, card, separated};
use self::model::{SettingsModel, SettingsRequest};
use crate::menu_bar::MenuBar;
use crate::theme::{self, text};

pub use settings_actions::OpenSettings;

#[allow(
    clippy::derive_partial_eq_without_eq,
    reason = "GPUI's actions! macro writes the derives, not us"
)]
mod settings_actions {
    use gpui_kit::actions;

    actions!(
        pwrroute,
        [
            /// Open the Settings window, or bring it forward.
            OpenSettings,
        ]
    );
}

/// Cards and pane heads stop growing here.
pub const CARD_MAX_WIDTH: f32 = 760.0;

/// `.snav`: wide enough for a cloud folder's child row (Settings
/// Sections 1a).
const NAV_WIDTH: f32 = 232.0;

/// The design's 1060px canvas; tall enough for a pane's first cards.
const WINDOW_SIZE: (f32, f32) = (1060.0, 760.0);

/// PwrGit's Settings minimum.
const MIN_SIZE: (f32, f32) = (760.0, 520.0);

/// Where AppKit puts the close button: the main window's place.
const TRAFFIC_LIGHTS: (f32, f32) = (16.0, 13.0);

/// The element id of the window's root, for tests and the inspector.
pub const ROOT_ID: &str = "settings-root";

/// The pane's scroller.
pub const SCROLL_ID: &str = "settings-scroll";

/// The shortcut that opens Settings: ⌘, on macOS, Ctrl+, elsewhere.
pub const SHORTCUT: &str = if cfg!(target_os = "macos") {
    "cmd-,"
} else {
    "ctrl-,"
};

/// The nav's sections, in the design's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pane {
    About,
    Browsers,
    Configuration,
    ShortLinks,
    Sync,
    Privacy,
    LocalAgents,
    Updates,
    Diagnostics,
}

impl Pane {
    pub const ALL: [Self; 9] = [
        Self::About,
        Self::Browsers,
        Self::Configuration,
        Self::ShortLinks,
        Self::Sync,
        Self::Privacy,
        Self::LocalAgents,
        Self::Updates,
        Self::Diagnostics,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Browsers => "Browsers",
            Self::Configuration => "Config Files",
            Self::ShortLinks => "Short Links",
            Self::Sync => "Cloud Sync",
            Self::Privacy => "Privacy",
            Self::LocalAgents => "Local agents",
            Self::Updates => "Updates",
            Self::Diagnostics => "Diagnostics",
            Self::About => "About",
        }
    }

    /// The group label drawn above this pane, when it starts a group
    /// (Settings Sections 1a).
    pub fn group(self) -> Option<&'static str> {
        match self {
            Self::Browsers => Some("Routing"),
            Self::Sync => Some("Across machines"),
            Self::Privacy => Some("Data and agents"),
            Self::Updates => Some("App"),
            _ => None,
        }
    }

    /// The nav item's element id.
    pub fn nav_id(self) -> &'static str {
        match self {
            Self::Browsers => "settings-nav-browsers",
            Self::Configuration => "settings-nav-configuration",
            Self::ShortLinks => "settings-nav-short-links",
            Self::Sync => "settings-nav-sync",
            Self::Privacy => "settings-nav-privacy",
            Self::LocalAgents => "settings-nav-local-agents",
            Self::Updates => "settings-nav-updates",
            Self::Diagnostics => "settings-nav-diagnostics",
            Self::About => "settings-nav-about",
        }
    }

    /// The pane body's element id.
    pub fn pane_id(self) -> &'static str {
        match self {
            Self::Browsers => "settings-pane-browsers",
            Self::Configuration => "settings-pane-configuration",
            Self::ShortLinks => "settings-pane-short-links",
            Self::Sync => "settings-pane-sync",
            Self::Privacy => "settings-pane-privacy",
            Self::LocalAgents => "settings-pane-local-agents",
            Self::Updates => "settings-pane-updates",
            Self::Diagnostics => "settings-pane-diagnostics",
            Self::About => "settings-pane-about",
        }
    }
}

/// Receives what the reader asks for. Runs after the window's own update,
/// so it may update the window's entity.
pub type RequestHandler = Rc<dyn Fn(&SettingsRequest, &mut Window, &mut App)>;

/// Fills the model as the window opens. The producers may also hook the
/// window here (a refresh when it comes to the front).
pub type ModelSource = Rc<dyn Fn(&mut Window, &mut App) -> SettingsModel>;

/// What the Settings window is opened with: its model, and the handler for
/// its requests. The app sets this once its producers exist; until then the
/// window draws [`fixtures`], requests are logged and nothing else happens.
pub struct SettingsHost {
    pub model: ModelSource,
    pub on_request: RequestHandler,
}

impl Global for SettingsHost {}

impl Default for SettingsHost {
    fn default() -> Self {
        Self {
            model: Rc::new(|_, _| fixtures::model()),
            on_request: Rc::new(|request, _, _| {
                // Never act on the OS from here: registration in particular
                // is the explicit action the router's owner wires up.
                log::warn!("settings: {request:?} is not wired up yet");
            }),
        }
    }
}

/// The open Settings window, if any.
#[derive(Default)]
struct OpenWindow(Option<WindowHandle<SettingsWindow>>);

impl Global for OpenWindow {}

/// Register the Settings action, its shortcut and gpui-kit's control
/// bindings (the select's arrow keys, Escape).
pub fn install(cx: &mut App) {
    gpui_kit::base::init(cx);
    // Deferred: from inside Settings, the window is still held by the
    // dispatch, so bringing it forward would fail and open a second one.
    cx.on_action(|_: &OpenSettings, cx| cx.defer(|cx| open(cx, None)));
    cx.bind_keys([KeyBinding::new(SHORTCUT, OpenSettings, None)]);
}

/// Open Settings, or bring the open window forward; `pane` picks a page.
pub fn open(cx: &mut App, pane: Option<Pane>) {
    let existing = cx.try_global::<OpenWindow>().and_then(|open| open.0);
    if let Some(handle) = existing {
        let shown = handle.update(cx, |view, window, cx| {
            if let Some(pane) = pane {
                view.show(pane, cx);
            }
            crate::menu_bar::bring_forward(window, cx);
        });
        if shown.is_ok() {
            cx.activate(true);
            return;
        }
    }
    let (model, on_request) = cx.try_global::<SettingsHost>().map_or_else(
        || {
            let host = SettingsHost::default();
            (host.model, host.on_request)
        },
        |host| (host.model.clone(), host.on_request.clone()),
    );
    let display = crate::placement::display_for_new_window(cx);
    let bounds = Bounds::centered(
        display,
        crate::zoom::window_size(display, WINDOW_SIZE, cx),
        cx,
    );
    let handle = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            display_id: display,
            titlebar: Some(TitlebarOptions {
                title: Some("PwrRoute Settings".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(TRAFFIC_LIGHTS.0), px(TRAFFIC_LIGHTS.1))),
            }),
            app_id: Some("pwrroute".to_owned()),
            window_min_size: Some(crate::zoom::min_size(display, MIN_SIZE, cx)),
            ..Default::default()
        },
        |window, cx| {
            let model = model(window, cx);
            cx.new(|cx| {
                let mut view = SettingsWindow::new(model, on_request, cx);
                if let Some(pane) = pane {
                    view.pane = pane;
                }
                view
            })
        },
    );
    match handle {
        Ok(handle) => {
            let _ = handle.update(cx, |view, window, cx| {
                let focus = view.focus.clone();
                window.focus(&focus, cx);
            });
            cx.set_global(OpenWindow(Some(handle)));
            cx.activate(true);
        }
        Err(error) => log::warn!("could not open Settings: {error}"),
    }
}

/// Whether a Settings window is open: a producer that reads files to
/// report back asks first, so a closed window costs nothing.
pub fn is_open(cx: &App) -> bool {
    cx.windows()
        .into_iter()
        .any(|window| window.downcast::<SettingsWindow>().is_some())
}

/// Changes the open window's model, if there is one: how a producer reports
/// back (the OS's answer on the default browser, say). Nothing happens when
/// Settings is closed; the next open reads the producer afresh.
pub fn update(cx: &mut App, edit: impl FnOnce(&mut SettingsModel)) {
    let open = cx.try_global::<OpenWindow>().and_then(|open| open.0);
    if let Some(handle) = open {
        let _ = handle.update(cx, |view, _, cx| {
            edit(&mut view.model);
            cx.notify();
        });
    }
}

/// The Settings window's view.
pub struct SettingsWindow {
    pub focus: FocusHandle,
    menu_bar: Entity<MenuBar>,
    pub pane: Pane,
    pub model: SettingsModel,
    /// Cards folded shut, by id. A reading position, not a preference, so
    /// it lives as long as the window.
    collapsed: HashSet<&'static str>,
    /// The command whose Copy button reads "Copied".
    copied: Option<usize>,
    pub(crate) agents: local_agents::State,
    pub(crate) privacy: privacy::State,
    pub(crate) configuration: configuration::State,
    pub(crate) sync: sync::State,
    /// Nav groups folded shut, for as long as the window is open.
    folded: HashSet<Pane>,
    /// The row a nav child brought into view (a folder's root, a config
    /// file's path), marked until the pane changes.
    pub target: Option<SharedString>,
    /// Scrolls the pane to `target`'s row.
    anchor: ScrollAnchor,
    scroll: ScrollHandle,
    on_request: RequestHandler,
}

impl SettingsWindow {
    pub fn new(
        model: SettingsModel,
        on_request: RequestHandler,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let agents = local_agents::State::new(cx);
        let privacy = privacy::State::new(&model.privacy, cx);
        let configuration = configuration::State::new(cx);
        let sync = sync::State::new(&model.sync, cx);
        let scroll = ScrollHandle::new();
        Self {
            focus: cx.focus_handle(),
            menu_bar: MenuBar::entity("Settings", cx),
            // Browsers, not About: it is the pane people open Settings for.
            pane: Pane::Browsers,
            model,
            collapsed: HashSet::new(),
            copied: None,
            agents,
            privacy,
            configuration,
            sync,
            folded: HashSet::new(),
            target: None,
            anchor: ScrollAnchor::for_handle(scroll.clone()),
            scroll,
            on_request,
        }
    }

    /// Switch panes. A new pane starts at the top.
    pub fn show(&mut self, pane: Pane, cx: &mut Context<'_, Self>) {
        if pane != self.pane {
            self.pane = pane;
            self.copied = None;
            self.agents.close_menu();
            self.privacy.close_sheet();
            self.configuration.close_menu();
            self.sync.close();
            self.target = None;
            self.scroll.set_offset(point(px(0.0), px(0.0)));
            cx.notify();
        }
    }

    /// Hand a request to the host once this update is done.
    fn request(&self, request: SettingsRequest, window: &mut Window, cx: &mut App) {
        let on_request = self.on_request.clone();
        window.defer(cx, move |window, cx| on_request(&request, window, cx));
    }

    fn copy(&mut self, index: usize, content: String, cx: &mut Context<'_, Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(content));
        self.copied = Some(index);
        cx.notify();
    }

    fn toggle_card(&mut self, id: &'static str, cx: &mut Context<'_, Self>) {
        if !self.collapsed.remove(id) {
            self.collapsed.insert(id);
        }
        cx.notify();
    }

    /// Fold every card in `ids`, or unfold them all if all are folded.
    fn collapse_all(&mut self, ids: &[&'static str], cx: &mut Context<'_, Self>) {
        if ids.iter().all(|id| self.collapsed.contains(id)) {
            for id in ids {
                self.collapsed.remove(id);
            }
        } else {
            self.collapsed.extend(ids.iter().copied());
        }
        cx.notify();
    }

    fn all_collapsed(&self, ids: &[&'static str]) -> bool {
        ids.iter().all(|id| self.collapsed.contains(id))
    }

    /// A `.card` whose header folds it: `heading` toggles, `trailing` (a
    /// chip or a button) sits on the right, and `rows` are separated.
    fn disclosure(
        &self,
        id: &'static str,
        heading: Heading,
        trailing: Option<AnyElement>,
        rows: Vec<Div>,
        cx: &mut Context<'_, Self>,
    ) -> gpui_kit::Stateful<Div> {
        let open = !self.collapsed.contains(id);
        let toggle_id = ElementId::Name(SharedString::from(format!("{id}-toggle")));
        let name = heading.title;
        card(id)
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(12.0))
                    .py(px(13.0))
                    .px(px(16.0))
                    .when(open && !rows.is_empty(), |header| {
                        header.border_b_1().border_color(theme::border_faint())
                    })
                    .child(
                        Button::new(toggle_id)
                            .flex_1()
                            .min_w_0()
                            .justify_start()
                            .items_start()
                            .line_height(px(13.5))
                            .accessibility_label(name)
                            .aria_expanded(open)
                            .on_click(cx.listener(move |this, _, _, cx| this.toggle_card(id, cx)))
                            .child(heading.render()),
                    )
                    .children(trailing),
            )
            .when(open, |card| card.children(separated(rows)))
    }

    fn copy_button(&self, index: usize, content: String, cx: &mut Context<'_, Self>) -> AnyElement {
        self.copy_button_labeled(index, "Copy", content, cx)
    }

    /// [`Self::copy_button`] with its own label until it is clicked.
    fn copy_button_labeled(
        &self,
        index: usize,
        idle: &'static str,
        content: String,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let label = if self.copied == Some(index) {
            "Copied"
        } else {
            idle
        };
        let id = ElementId::Name(SharedString::from(format!("settings-copy-{index}")));
        button(id, label, Kind::Default, true)
            .on_click(cx.listener(move |this, _, _, cx| this.copy(index, content.clone(), cx)))
            .into_any_element()
    }

    fn pane_body(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Div {
        match self.pane {
            Pane::Browsers => browsers::pane(self, cx),
            Pane::Configuration => configuration::pane(self, cx),
            Pane::ShortLinks => short_links::pane(self, cx),
            Pane::Sync => sync::pane(self, window, cx),
            Pane::Privacy => privacy::pane(self, window, cx),
            Pane::LocalAgents => local_agents::pane(self, window, cx),
            Pane::Updates => updates::pane(self, cx),
            Pane::Diagnostics => diagnostics::pane(self, cx),
            Pane::About => about::pane(self, cx),
        }
    }
}

/// A button whose only job is to send `request`.
fn request_button(
    id: &'static str,
    label: &'static str,
    kind: Kind,
    small: bool,
    request: SettingsRequest,
    cx: &mut Context<'_, SettingsWindow>,
) -> Button {
    button(id, label, kind, small).on_click(cx.listener(move |this, _, window, cx| {
        this.request(request.clone(), window, cx);
    }))
}

#[cfg(test)]
use nav::SYNC_DOT_ID;

/// `.strip`: the family title strip, with "Settings" as its crumb.
fn title_strip(menu_bar: &Entity<MenuBar>, window: &Window, cx: &App) -> Div {
    crate::chrome::strip(menu_bar, window, cx)
        .child(text("\u{203a}", 14.0, 500.0, theme::text_subtle()))
        .child(text("Settings", 13.0, 500.0, theme::text_secondary()))
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let pane = self.pane;
        let body = self.pane_body(window, cx);
        div()
            .id(ROOT_ID)
            .role(gpui_kit::Role::Pane)
            .aria_label("Settings")
            .debug_selector(|| ROOT_ID.into())
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::bg_app())
            .text_color(theme::text_primary())
            .font_family(theme::SANS)
            .child(title_strip(&self.menu_bar, window, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(nav::draw(self, cx))
                    .child(
                        div()
                            .id(SCROLL_ID)
                            .flex_1()
                            .min_w_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .child(
                                body.id(pane.pane_id())
                                    .flex()
                                    .flex_col()
                                    .gap(px(14.0))
                                    .py(px(22.0))
                                    .px(px(24.0))
                                    .test_support(),
                            ),
                    ),
            )
    }
}
