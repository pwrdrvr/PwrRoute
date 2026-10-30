//! The family window chrome: the title strip across the top and the
//! navigation rail down the left.

use crate::theme::px;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, Div, Entity, Global, HighlightStyle, InteractiveElement as _,
    IntoElement as _, ParentElement as _, Role, Stateful, StatefulInteractiveElement as _,
    Styled as _, StyledText, Window, WindowControlArea, div, svg,
};

use crate::app::{PwrRoute, Screen};
use crate::app_menu::Platform;
use crate::build_info::Part;
use crate::default_browser::DefaultState;
use crate::e2e;
use crate::menu_bar::MenuBar;
use crate::overview_model::{ConfigTrouble, State, Status, plural};
use crate::theme::{self, icon, mono, text, tracked};
use crate::widgets::{ButtonKind, button, key_cap, keyboard_control, one_line, status_chip};

/// The rail's "Open Settings…" item.
pub const OPEN_SETTINGS_ID: &str = "nav-settings";

/// The rail foot's way out when `rules.json` changed under unsaved edits.
pub const RELOAD_ID: &str = "rules-reload";
pub const OVERWRITE_ID: &str = "rules-overwrite";
/// The rail's build block (design: Build Identity 1a-1d): copy all, then
/// a row each for the checkout, the branch and the commit; a release's one
/// line is `BUILD_ALL_ID`.
pub const BUILD_ALL_ID: &str = "build-copy-all";
pub const BUILD_CHECKOUT_ID: &str = "build-checkout";
pub const BUILD_BRANCH_ID: &str = "build-branch";
pub const BUILD_COMMIT_ID: &str = "build-commit";
/// How many characters of the checkout fit a row before it is cut to its
/// last two folders: the rail's 160px less the glyph, in Geist Mono 10.5.
const CHECKOUT_FITS: usize = 22;

/// Where the brand starts on macOS, after the traffic lights. The family
/// spec.
pub const MAC_BRAND_X: f32 = 96.0;

/// Where it starts on Windows and Linux, at the rail's 16px inset: there
/// are no traffic lights to clear (design: Windows and Linux Chrome 1a/1b).
pub const OTHER_BRAND_X: f32 = 16.0;

/// The Windows caption buttons: three of Windows' 46px buttons, the full
/// height of the strip's fill.
pub const CAPTION_WIDTH: f32 = 46.0;

/// What the strip reserves for them on the right (design 1a's
/// `padding-right: 150px`): the buttons and a gap before the last chip.
pub const CAPTIONS_RESERVE: f32 = 150.0;

/// Whose window chrome the strip draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chrome {
    /// The traffic lights are AppKit's and the menu bar is the system's.
    Mac,
    /// The strip draws the menu bar and the caption buttons, and is the
    /// window's drag area: GPUI hides Windows' own title bar.
    Windows,
    /// The strip draws the menu bar; the desktop draws the title bar and
    /// its buttons (GPUI asks for server-side decorations).
    Linux,
}

impl Chrome {
    /// This build's OS.
    pub fn host() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    /// The menu template it uses.
    pub fn menus(self) -> Platform {
        match self {
            Self::Mac => Platform::Mac,
            Self::Windows | Self::Linux => Platform::Other,
        }
    }

    /// Where the brand starts.
    pub fn brand_x(self) -> f32 {
        match self {
            Self::Mac => MAC_BRAND_X,
            Self::Windows | Self::Linux => OTHER_BRAND_X,
        }
    }

    /// The Settings shortcut as the rail's key cap shows it.
    pub fn settings_key(self) -> &'static str {
        match self {
            Self::Mac => "\u{2318},",
            Self::Windows | Self::Linux => "Ctrl+,",
        }
    }
}

/// The chrome `app_menu::install_for` was given, when it differs from the
/// host's.
struct ChromeOverride(Chrome);

impl Global for ChromeOverride {}

/// Draw `chrome`'s strip in every window from now on.
pub fn set(chrome: Chrome, cx: &mut App) {
    cx.set_global(ChromeOverride(chrome));
}

/// The chrome the strip draws: the one installed, else the host's.
pub fn current(cx: &App) -> Chrome {
    cx.try_global::<ChromeOverride>()
        .map_or_else(Chrome::host, |chrome| chrome.0)
}

/// The strip's fill. The 1px bottom border makes the strip 41px in all.
pub const STRIP_HEIGHT: f32 = 40.0;

/// The strip's status chips for what is on screen.
fn strip_chips(app: &PwrRoute, status: &Status) -> Vec<Div> {
    let mut chips = screen_chips(app, status);
    // Only a sync conflict or pause earns a chip (design: Synced Rules 3b).
    if let Some(line) = &app.sync_line
        && let Some(chip) = line.chip
    {
        let dot = match line.tone {
            crate::sync::Tone::Bad => theme::status_error(),
            _ => theme::status_warning(),
        };
        chips.push(status_chip(chip, dot));
    }
    chips
}

fn screen_chips(app: &PwrRoute, status: &Status) -> Vec<Div> {
    match (app.screen, &app.config) {
        (Screen::Overview, _) => overview_chips(app, status),
        // The config's chip, as Overview draws it (design 5a/5b).
        (Screen::Rules | Screen::TestLink, Some(_)) => vec![source_chip(app, status)],
        (Screen::Rules, None) => vec![source_chip(app, status), crate::default_browser::chip()],
        (Screen::TestLink, None) => vec![source_chip(app, status)],
        (Screen::Activity, _) => vec![status_chip(app.activity.today.label(), theme::status_ok())],
    }
}

/// What the source chip reads from: the config in charge by its shown
/// path, the router's word on it, the saved order, and `rules.json`.
#[derive(Clone, Copy, Debug)]
pub struct Source<'a> {
    pub config: Option<&'a str>,
    pub trouble: Option<&'a ConfigTrouble>,
    /// `rules.json`'s `precedence` is `"pwrroute"`.
    pub rules_first: bool,
    /// `rules.json` exists.
    pub exists: bool,
    pub rewrites: usize,
    pub rules: usize,
}

/// Where the rules come from, and the chip's tone. A config in charge is
/// the accent, not a warning: warning only when its last reload failed,
/// and the error tone when it never loaded.
pub fn source_label(source: Source<'_>) -> (String, gpui_kit::Hsla) {
    match source.config {
        Some(path)
            if matches!(
                source.trouble,
                Some(ConfigTrouble::NotLoaded(_) | ConfigTrouble::NothingLoaded(_))
            ) =>
        {
            (
                format!("{path} \u{00b7} didn't load"),
                theme::status_error(),
            )
        }
        Some(path) if source.trouble.is_some() => (
            format!("{path} \u{00b7} didn't reload"),
            theme::status_warning(),
        ),
        Some(path) if source.rules_first => {
            (format!("{path} \u{00b7} after your rules"), theme::accent())
        }
        Some(path) => (format!("{path} \u{00b7} runs first"), theme::accent()),
        None if !source.exists => ("no rules.json yet".to_owned(), theme::status_suspended()),
        None if source.rewrites == 0 && source.rules == 0 => (
            "rules.json \u{00b7} no rules yet".to_owned(),
            theme::status_ok(),
        ),
        None => (
            format!(
                "rules.json \u{00b7} {} \u{00b7} {}",
                plural(source.rewrites, "rewrite"),
                plural(source.rules, "rule")
            ),
            theme::status_ok(),
        ),
    }
}

/// [`source_label`]'s chip over the window. The order is the store's, which
/// only ever holds what was written (a refused or failed write is put
/// back).
fn source_chip(app: &PwrRoute, status: &Status) -> Div {
    let (label, tone) = source_label(Source {
        config: app.config.as_ref().map(|config| config.path.as_str()),
        trouble: status.config_error.as_ref(),
        rules_first: !app.store.precedence().is_config(),
        exists: app.store.exists(),
        rewrites: app.rewrites.len(),
        rules: app.handlers.len(),
    });
    status_chip(label, tone)
}

/// Overview's chips: where the rules come from, then whether links reach
/// PwrRoute, in the words of `TrayModel::chip()` (paused / default browser
/// / not default), so the tray and the window agree.
fn overview_chips(app: &PwrRoute, status: &Status) -> Vec<Div> {
    let source = source_chip(app, status);
    let state = if status.paused.is_some() {
        status_chip("paused", theme::status_warning())
    } else if let Some(holding) = &status.holding {
        status_chip(
            holding.short(std::time::SystemTime::now()),
            theme::info_text(),
        )
    } else if let Some(sending) = &status.sending {
        status_chip(
            format!("{} \u{00b7} {}", sending.short(), sending.name),
            theme::accent(),
        )
    } else {
        match status.default {
            DefaultState::PwrRoute => status_chip("default browser", theme::status_ok()),
            DefaultState::Other(_) => status_chip("not default", theme::status_warning()),
            DefaultState::Unknown => status_chip("default unknown", theme::status_suspended()),
        }
    };
    vec![source, state]
}

/// The rail foot's first line: the routing state, in its tone.
pub fn routing_line(status: &Status) -> (gpui_kit::Hsla, String) {
    match status.state() {
        State::ConfigError => (
            theme::status_error(),
            "routing \u{00b7} config didn't load".to_owned(),
        ),
        State::ConfigStale => (
            theme::status_warning(),
            "routing \u{00b7} config didn't reload".to_owned(),
        ),
        State::Paused => (
            theme::status_warning(),
            format!(
                "paused \u{00b7} {}",
                status
                    .paused
                    .as_ref()
                    .map_or("", |pause| pause.name.as_str())
            ),
        ),
        State::Holding => (
            theme::info_text(),
            match status.held {
                0 => "holding".to_owned(),
                held => format!("holding \u{00b7} {held} held"),
            },
        ),
        State::Sending => (
            theme::accent(),
            status.sending.as_ref().map_or_else(String::new, |sending| {
                format!("{} \u{00b7} {}", sending.short(), sending.name)
            }),
        ),
        State::NotDefault => (
            theme::status_warning(),
            "not the default browser".to_owned(),
        ),
        State::Unknown => (
            theme::status_suspended(),
            "can't tell the default browser".to_owned(),
        ),
        State::Routing => (theme::status_ok(), "routing".to_owned()),
    }
}

/// `.strip`: the brand, the breadcrumb and the status chips.
///
/// The traffic lights are AppKit's own, placed by the window's
/// `TitlebarOptions` (see `main`), so the strip only leaves room for them.
pub fn title_strip(app: &PwrRoute, status: &Status, window: &Window, cx: &App) -> Div {
    strip(&app.menu_bar, window, cx)
        .child(text("\u{203a}", 14.0, 500.0, theme::text_subtle()))
        .child(text(
            app.screen.label(),
            13.0,
            500.0,
            theme::text_secondary(),
        ))
        .child(div().flex_1())
        .children(strip_chips(app, status))
}

/// The strip's children, by chrome, for `e2e::regions`: what the callers
/// add follows the brand (and the menu bar off macOS).
const MAC_REGIONS: &[&str] = &[
    "titleStrip.brand",
    "titleStrip.chevron",
    "titleStrip.breadcrumb",
    "titleStrip.spacer",
    "titleStrip.chip1",
    "titleStrip.chip2",
];
const LINUX_REGIONS: &[&str] = &[
    "titleStrip.brand",
    "titleStrip.menuBar",
    "titleStrip.chevron",
    "titleStrip.breadcrumb",
    "titleStrip.spacer",
    "titleStrip.chip1",
    "titleStrip.chip2",
];
const WINDOWS_REGIONS: &[&str] = &[
    "titleStrip.captions",
    "titleStrip.brand",
    "titleStrip.menuBar",
    "titleStrip.chevron",
    "titleStrip.breadcrumb",
    "titleStrip.spacer",
    "titleStrip.chip1",
    "titleStrip.chip2",
];

/// `.strip` with its `.brand`: the mark and the wordmark, after the
/// traffic lights on macOS; elsewhere at x=16 with `menu_bar` after it
/// and, on Windows, the caption buttons on the right. The crumb and
/// whatever follows go in as children.
pub fn strip(menu_bar: &Entity<MenuBar>, window: &Window, cx: &App) -> Div {
    let chrome = current(cx);
    let wordmark = StyledText::new("PwrRoute").with_highlights([(
        3..8,
        HighlightStyle {
            color: Some(theme::accent()),
            ..Default::default()
        },
    )]);
    let regions = match chrome {
        Chrome::Mac => MAC_REGIONS,
        Chrome::Linux => LINUX_REGIONS,
        Chrome::Windows => WINDOWS_REGIONS,
    };
    div()
        .on_children_prepainted(e2e::regions(regions))
        .relative()
        .h(px(STRIP_HEIGHT + 1.0))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(12.0))
        .pl(px(chrome.brand_x()))
        .pr(px(if chrome == Chrome::Windows {
            CAPTIONS_RESERVE
        } else {
            16.0
        }))
        .bg(theme::bg_titlebar())
        .border_b_1()
        .border_color(theme::border_subtle())
        // GPUI hides Windows' title bar, so the strip is where the window
        // is dragged from (and double-clicked to maximize). The menu
        // titles, the caption buttons and any child a caller adds that
        // takes a click (`.occlude()`) block it.
        .when(chrome == Chrome::Windows, |strip| {
            strip
                .window_control_area(WindowControlArea::Drag)
                .child(captions(window))
        })
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(8.0))
                .child(icon("pwrroute/mark.svg", 20.0, theme::accent()))
                .child(
                    div()
                        .font_family(theme::SANS)
                        .font_weight(theme::weight(700.0))
                        .text_size(px(17.0))
                        .line_height(px(17.0))
                        .text_color(theme::text_primary())
                        .whitespace_nowrap()
                        .child(wordmark),
                ),
        )
        .when(chrome != Chrome::Mac, |strip| strip.child(menu_bar.clone()))
}

/// Windows' minimize, maximize (restore while maximized) and close, on
/// the strip's right edge: Windows draws them itself only with its own
/// title bar, which GPUI hides. Each is a `WindowControlArea`, so Windows
/// presses it (and offers Snap layouts on maximize) as its own.
fn captions(window: &Window) -> Div {
    let button = |id: &'static str, path: &'static str, area: WindowControlArea| {
        let close = area == WindowControlArea::Close;
        div()
            .id(id)
            .debug_selector(|| id.into())
            .group(id)
            .occlude()
            .window_control_area(area)
            .w(px(CAPTION_WIDTH))
            .h(px(STRIP_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .when(close, |button| button.rounded_tr(px(7.0)))
            .hover(move |style| {
                style.bg(if close {
                    theme::caption_close()
                } else {
                    theme::bg_hover()
                })
            })
            .child(
                svg()
                    .path(path)
                    .flex_none()
                    .size(px(10.0))
                    .text_color(theme::text_secondary())
                    .when(close, |glyph| {
                        glyph.group_hover(id, |style| style.text_color(gpui_kit::white()))
                    }),
            )
    };
    let maximize = if window.is_maximized() {
        "pwrroute/caption-restore.svg"
    } else {
        "pwrroute/caption-maximize.svg"
    };
    div()
        .absolute()
        .top_0()
        .right_0()
        .flex()
        .child(button(
            "caption-minimize",
            "pwrroute/caption-minimize.svg",
            WindowControlArea::Min,
        ))
        .child(button("caption-maximize", maximize, WindowControlArea::Max))
        .child(button(
            "caption-close",
            "pwrroute/caption-close.svg",
            WindowControlArea::Close,
        ))
}

/// The nav rail's width, and its width as icons (`app::Layout`).
pub const RAIL_WIDTH: f32 = 196.0;
pub const RAIL_COMPACT: f32 = 56.0;

/// `.rail-item`; `.on` when `selected`; the icon alone when `compact`.
fn rail_item(
    id: &'static str,
    icon_path: &'static str,
    label: &'static str,
    selected: bool,
    trailing: Option<AnyElement>,
    compact: bool,
) -> Stateful<Div> {
    let (foreground, icon_color) = if selected {
        (theme::accent_bright(), theme::accent())
    } else {
        (theme::text_secondary(), theme::text_muted())
    };
    // A Tab stop; its 1px edge is the focus ring's, out of the design's
    // 8/10 padding.
    keyboard_control(div(), false)
        .id(id)
        .debug_selector(|| id.into())
        .role(Role::Button)
        .aria_label(label)
        .aria_selected(selected)
        .relative()
        .flex()
        .items_center()
        .gap(px(9.0))
        .px(px(9.0))
        .py(px(7.0))
        .border_1()
        .border_color(theme::transparent())
        .rounded(px(7.0))
        .cursor_pointer()
        .when(!selected, |item| {
            item.hover(|style| style.bg(theme::bg_hover()))
        })
        .when(selected, |item| {
            // `.rail-item.on::before`: the 3px bar on the rail's edge.
            item.bg(theme::bg_row_active()).child(
                div()
                    .absolute()
                    .left(px(-10.0))
                    .top(px(7.0))
                    .bottom(px(7.0))
                    .w(px(3.0))
                    .rounded_r(px(2.0))
                    .bg(theme::accent()),
            )
        })
        .child(icon(icon_path, 15.0, icon_color))
        .map(|item| {
            if compact {
                // The icon alone, named by its tooltip and its label.
                item.justify_center()
                    .tooltip(crate::overview::tip(label, None))
            } else {
                item.child(text(label, 13.0, 500.0, foreground))
                    .when_some(trailing, |item, trailing| {
                        item.child(div().ml_auto().child(trailing))
                    })
            }
        })
}

/// The section a rail item opens.
fn section(
    app: &PwrRoute,
    cx: &mut Context<'_, PwrRoute>,
    screen: Screen,
    trailing: Option<AnyElement>,
    compact: bool,
) -> Stateful<Div> {
    let (id, icon_path) = match screen {
        Screen::Overview => ("nav-overview", "pwrroute/map.svg"),
        Screen::Rules => ("nav-rules", "pwrroute/rules.svg"),
        Screen::TestLink => ("nav-test", "pwrroute/test.svg"),
        Screen::Activity => ("nav-activity", "pwrroute/activity.svg"),
    };
    rail_item(
        id,
        icon_path,
        screen.label(),
        app.screen == screen,
        trailing,
        compact,
    )
    .map(crate::accessibility::clickable)
    .on_click(cx.listener(move |this, _, window, cx| this.navigate(screen, window, cx)))
}

/// The foot's Reload and Overwrite, under the status, while the file
/// changed on disk under unsaved edits: Reload drops the edits and reads
/// the file; Overwrite writes them over it.
fn changed_actions(cx: &mut Context<'_, PwrRoute>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .child(
            button("Reload", ButtonKind::Default, true)
                .id(RELOAD_ID)
                .debug_selector(|| RELOAD_ID.into())
                .on_click(cx.listener(|this, _, _, cx| this.reload_rules(cx))),
        )
        .child(
            button("Overwrite", ButtonKind::Danger, true)
                .id(OVERWRITE_ID)
                .debug_selector(|| OVERWRITE_ID.into())
                .on_click(cx.listener(|this, _, _, cx| this.overwrite_rules(cx))),
        )
}

/// The rail foot's second line: what was last read or written. A config
/// in charge that loaded says when it was (re)loaded; else `rules.json`
/// when it was saved. Anything the store has to say about an edit
/// (unsaved, saving, a failed save, a change on disk) wins.
pub fn foot_label(app: &PwrRoute, status: &Status) -> String {
    use crate::rules_store::{Status as Store, clock};
    match (app.store.status(), &app.config) {
        (Store::Saved { .. }, Some(config))
            if config.problem.is_none() && status.config_error.is_none() =>
        {
            // The router reloads the file soon after it changes; this
            // window read it when it opened, or when the source changed.
            let reloaded = config.modified.map_or(app.config_loaded, |modified| {
                modified.max(app.config_loaded)
            });
            format!("reloaded {}", clock(reloaded))
        }
        (Store::Saved { at: Some(at) }, _) => format!("rules.json \u{00b7} saved {}", clock(*at)),
        _ => app.store.status_label(),
    }
}

/// The element id of the rail foot's sync line.
pub const SYNC_LINE_ID: &str = "rail-sync";

/// Sync's line under the routing state (design: Synced Rules 3a): mono and
/// lower case like its neighbours, quiet when all is well, a click to
/// Settings › Sync.
fn sync_line(line: &crate::sync::Line, cx: &mut Context<'_, PwrRoute>) -> AnyElement {
    let color = match line.tone {
        crate::sync::Tone::Quiet => theme::text_subtle(),
        crate::sync::Tone::Warn => theme::status_warning(),
        crate::sync::Tone::Bad => theme::status_error(),
    };
    div()
        .id(SYNC_LINE_ID)
        .debug_selector(|| SYNC_LINE_ID.into())
        .cursor_pointer()
        .font_family(theme::mono_family(500.0))
        .font_weight(theme::weight(500.0))
        .text_size(px(10.5))
        .line_height(px(14.7))
        .text_color(color)
        .hover(|style| style.text_color(theme::text_secondary()))
        .child(line.text.clone())
        .on_click(cx.listener(|_, _, _, cx| {
            cx.defer(|cx| crate::settings::open(cx, Some(crate::settings::Pane::Sync)));
        }))
        .into_any_element()
}

/// Which build this is, at the foot of the rail (design: Build Identity).
/// A release is one muted line; a dev build is a block under a hairline:
/// "Dev build" with the version, then the checkout, branch and commit, each
/// a click to copy its full value, and a button that copies them all.
fn build_block(app: &PwrRoute, cx: &mut Context<'_, PwrRoute>) -> AnyElement {
    let build = &app.build;
    let copied = app.build_copied.map(|(part, _)| part);
    if build.is_release() {
        return div()
            .id(BUILD_ALL_ID)
            .aria_label("Copy build information")
            .debug_selector(|| BUILD_ALL_ID.into())
            .cursor_pointer()
            .child(
                mono(
                    if copied == Some(Part::All) {
                        "Copied".to_owned()
                    } else {
                        build.release_line()
                    },
                    10.5,
                    500.0,
                    if copied == Some(Part::All) {
                        theme::success_text()
                    } else {
                        theme::text_subtle()
                    },
                )
                .hover(|style| style.text_color(theme::text_primary())),
            )
            .tooltip(crate::overview::tip(
                "Click to copy the version and commit",
                build.commit.clone(),
            ))
            .map(crate::accessibility::clickable)
            .on_click(cx.listener(|this, _, _, cx| this.copy_build(Part::All, cx)))
            .into_any_element();
    }
    let header = div()
        .flex()
        .items_center()
        .gap(px(6.0))
        .h(px(20.0))
        .pb(px(2.0))
        .child(tracked("Dev build", 9.5, 600.0, 0.12, theme::warn_text()))
        .child(mono(
            build.version_and_profile(),
            10.5,
            500.0,
            theme::text_subtle(),
        ))
        .child(
            div()
                .id(BUILD_ALL_ID)
                .aria_label("Copy build information")
                .debug_selector(|| BUILD_ALL_ID.into())
                .ml_auto()
                .size(px(20.0))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .border_1()
                .border_color(theme::transparent())
                .cursor_pointer()
                .hover(|style| {
                    style
                        .bg(theme::bg_hover())
                        .border_color(theme::border_subtle())
                })
                .child(if copied == Some(Part::All) {
                    icon("pwrroute/check.svg", 12.0, theme::success_text())
                } else {
                    icon("pwrroute/copy.svg", 12.0, theme::text_muted())
                })
                .tooltip(crate::overview::tip(
                    "Copy the version, checkout, branch and commit",
                    None,
                ))
                .map(crate::accessibility::clickable)
                .on_click(cx.listener(|this, _, _, cx| this.copy_build(Part::All, cx))),
        );
    let checkout = build
        .checkout_label(app.home.as_deref(), CHECKOUT_FITS)
        .map(|label| {
            build_row(
                BUILD_CHECKOUT_ID,
                Part::Checkout,
                "pwrroute/folder.svg",
                value(label, theme::text_secondary()),
                (
                    "Click to copy the checkout\u{2019}s path",
                    build.checkout.clone(),
                ),
                app,
                cx,
            )
        });
    let branch = match &build.branch {
        Some(branch) => build_row(
            BUILD_BRANCH_ID,
            Part::Branch,
            "pwrroute/branch.svg",
            value(branch.clone(), theme::text_secondary()),
            ("Click to copy the branch", Some(branch.clone())),
            app,
            cx,
        ),
        // Nothing to copy: the commit below names it.
        None => build_row(
            BUILD_BRANCH_ID,
            Part::Branch,
            "pwrroute/branch.svg",
            value("detached HEAD", theme::text_subtle()),
            ("Built from a detached HEAD: the commit names it", None),
            app,
            cx,
        ),
    };
    let commit = build.short_commit().map(|short| {
        let after = if build.dirty {
            value("+ changes", theme::warn_text())
        } else {
            value(
                build.subject.clone().unwrap_or_default(),
                theme::text_subtle(),
            )
        };
        let mut source = build.commit.clone().unwrap_or_default();
        if let Some(line) = build.commit_line() {
            source.push('\n');
            source.push_str(&line);
        }
        if let Some(line) = build.changes_line() {
            source.push('\n');
            source.push_str(&line);
        }
        build_row(
            BUILD_COMMIT_ID,
            Part::Commit,
            "pwrroute/commit.svg",
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .min_w_0()
                .child(mono(short.to_owned(), 10.5, 500.0, theme::text_secondary()))
                .child(after.flex_1())
                .into_any_element(),
            ("Click to copy the commit", Some(source)),
            app,
            cx,
        )
    });
    div()
        .mt(px(4.0))
        .pt(px(10.0))
        .border_t_1()
        .border_color(theme::border_faint())
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(header)
        .children(checkout)
        .child(branch)
        .children(commit)
        .into_any_element()
}

/// A row's value: mono 10.5, cut with an ellipsis.
fn value(content: impl Into<gpui_kit::SharedString>, color: gpui_kit::Hsla) -> Div {
    one_line(content, theme::mono_family(500.0), 500.0, 10.5, 14.0, color).min_w_0()
}

/// One row of the build block: the glyph, the value, and on hover the copy
/// glyph (a check and "Copied" once clicked). The whole row is the target;
/// a part with nothing to copy (a detached HEAD's branch) is only shown.
fn build_row(
    id: &'static str,
    part: Part,
    glyph: &'static str,
    content: impl gpui_kit::IntoElement,
    (tip, source): (&'static str, Option<String>),
    app: &PwrRoute,
    cx: &mut Context<'_, PwrRoute>,
) -> AnyElement {
    let copyable = app.build.copy_text(part).is_some();
    let done = app.build_copied.is_some_and(|(copied, _)| copied == part);
    let row = div()
        .id(id)
        .debug_selector(|| id.into())
        .group(id)
        .flex()
        .items_center()
        .gap(px(6.0))
        .h(px(20.0))
        .mx(px(-6.0))
        .px(px(6.0))
        .rounded(px(5.0))
        .min_w_0()
        .child(icon(glyph, 11.0, theme::text_subtle()))
        .child(div().flex_1().min_w_0().child(content))
        .tooltip(crate::overview::tip(tip, source));
    if !copyable {
        return row.into_any_element();
    }
    row.cursor_pointer()
        .hover(|style| style.bg(theme::bg_hover()))
        .child(if done {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(4.0))
                .child(text("Copied", 9.5, 600.0, theme::success_text()))
                .child(icon("pwrroute/check.svg", 11.0, theme::success_text()))
                .into_any_element()
        } else {
            svg()
                .path("pwrroute/copy.svg")
                .flex_none()
                .size(px(11.0))
                .text_color(theme::transparent())
                .group_hover(id, |style| style.text_color(theme::text_muted()))
                .into_any_element()
        })
        .map(crate::accessibility::clickable)
        .on_click(cx.listener(move |this, _, _, cx| this.copy_build(part, cx)))
        .into_any_element()
}

/// `.rail`: the sections, Settings, and the routing state at the foot.
/// `compact` draws it as icons, with the foot's routing line as its dot;
/// `app::Layout` keeps it full while the foot asks to Reload or Overwrite
/// `rules.json`.
pub fn nav_rail(
    app: &PwrRoute,
    status: &Status,
    compact: bool,
    cx: &mut Context<'_, PwrRoute>,
) -> Div {
    let count = |label: String| mono(label, 10.5, 600.0, theme::text_subtle()).into_any_element();
    // Rewrites plus rules plus the config's, across both sources; a config
    // that did not load is a warning dot instead.
    let rules_count = app.rewrites.len()
        + app.handlers.len()
        + app
            .config
            .as_ref()
            .map_or(0, |config| config.rewrites.len() + config.handlers.len());
    let rules_trailing = if app.config.is_some() && status.config_error.is_some() {
        Some(
            div()
                .ml_auto()
                .mr(px(6.0))
                .child(theme::dot(theme::status_warning()))
                .into_any_element(),
        )
    } else {
        (rules_count > 0).then(|| count(rules_count.to_string()))
    };
    let (dot, line) = routing_line(status);
    if compact {
        let warning = app.config.is_some() && status.config_error.is_some();
        return compact_rail(app, warning, dot, line, cx);
    }
    let foot = div()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(theme::dot(dot))
                // `.rail-foot .st` loses to the later `.st` rule in the
                // stylesheet, so this line is sans 11px.
                .child(text(line, 11.0, 500.0, theme::text_secondary())),
        )
        .child(
            div()
                .font_family(theme::mono_family(500.0))
                .font_weight(theme::weight(500.0))
                .text_size(px(10.5))
                .line_height(px(14.7))
                .text_color(if app.store.changed_on_disk() {
                    theme::status_error()
                } else {
                    theme::text_subtle()
                })
                .child(foot_label(app, status)),
        )
        .children(app.store.changed_on_disk().then(|| changed_actions(cx)))
        .children(app.sync_line.as_ref().map(|line| sync_line(line, cx)))
        .child(build_block(app, cx));
    div()
        .on_children_prepainted(e2e::regions(&[
            "rail.overview",
            "rail.rules",
            "rail.test",
            "rail.activity",
            "rail.settingsHeading",
            "rail.openSettings",
            "rail.foot",
        ]))
        .w(px(RAIL_WIDTH))
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .pt(px(10.0))
        .px(px(10.0))
        .pb(px(12.0))
        .bg(theme::bg_sidebar())
        .border_r_1()
        .border_color(theme::border_subtle())
        .child(section(app, cx, Screen::Overview, None, false))
        .child(section(app, cx, Screen::Rules, rules_trailing, false))
        .child(section(app, cx, Screen::TestLink, None, false))
        .child(section(
            app,
            cx,
            Screen::Activity,
            // Held links that wait say so first (design: Incoming Links
            // 3b), in Hold's blue.
            if status.held > 0 {
                Some(
                    div()
                        .px(px(6.0))
                        .py(px(1.0))
                        .rounded(px(4.0))
                        .bg(theme::info_soft())
                        .child(mono(
                            format!("{} held", status.held),
                            10.5,
                            600.0,
                            theme::info_text(),
                        ))
                        .into_any_element(),
                )
            } else {
                app.activity.today.compact().map(count)
            },
            false,
        ))
        .child(div().mt(px(10.0)).mx(px(8.0)).mb(px(4.0)).child(tracked(
            "Settings",
            10.5,
            600.0,
            0.09,
            theme::text_subtle(),
        )))
        .child(
            rail_item(
                OPEN_SETTINGS_ID,
                "pwrroute/settings.svg",
                "Open Settings\u{2026}",
                false,
                Some(key_cap(current(cx).settings_key()).into_any_element()),
                false,
            )
            .map(crate::accessibility::clickable)
            .on_click(|_, _, cx| crate::settings::open(cx, None)),
        )
        .child(div().mt_auto().pt(px(10.0)).px(px(8.0)).child(foot))
}

/// The rail as icons. The counts go (Rules keeps its warning as a dot on
/// the icon's corner) and the foot is the routing dot, its line in the
/// tooltip; the build is in Settings › About.
fn compact_rail(
    app: &PwrRoute,
    rules_warning: bool,
    dot: gpui_kit::Hsla,
    line: String,
    cx: &mut Context<'_, PwrRoute>,
) -> Div {
    div()
        .on_children_prepainted(e2e::regions(&[
            "rail.overview",
            "rail.rules",
            "rail.test",
            "rail.activity",
            "rail.settingsHeading",
            "rail.openSettings",
            "rail.foot",
        ]))
        .w(px(RAIL_COMPACT))
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .pt(px(10.0))
        .px(px(10.0))
        .pb(px(12.0))
        .bg(theme::bg_sidebar())
        .border_r_1()
        .border_color(theme::border_subtle())
        .child(section(app, cx, Screen::Overview, None, true))
        .child(
            section(app, cx, Screen::Rules, None, true).when(rules_warning, |item| {
                item.child(
                    div()
                        .absolute()
                        .top(px(5.0))
                        .right(px(6.0))
                        .child(theme::dot(theme::status_warning())),
                )
            }),
        )
        .child(section(app, cx, Screen::TestLink, None, true))
        .child(section(app, cx, Screen::Activity, None, true))
        // "Settings" as a rule: a heading does not fit.
        .child(
            div()
                .mt(px(10.0))
                .mb(px(6.0))
                .mx(px(6.0))
                .h(px(1.0))
                .bg(theme::border_subtle()),
        )
        .child(
            rail_item(
                OPEN_SETTINGS_ID,
                "pwrroute/settings.svg",
                "Open Settings\u{2026}",
                false,
                None,
                true,
            )
            .on_click(|_, _, cx| crate::settings::open(cx, None)),
        )
        .child(
            div()
                .id("rail-foot-dot")
                .mt_auto()
                .flex()
                .justify_center()
                .py(px(6.0))
                .tooltip(crate::overview::tip(line, None))
                .child(theme::dot(dot)),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overview_model::Pause;

    /// The rail foot names each routing state in its own words and tone.
    #[test]
    fn routing_line_names_each_state() {
        let with = |change: &dyn Fn(&mut Status)| {
            let mut status = Status::routing();
            change(&mut status);
            routing_line(&status)
        };
        assert_eq!(with(&|_| {}), (theme::status_ok(), "routing".to_owned()));
        assert_eq!(
            with(&|status| status.default = DefaultState::Unknown),
            (
                theme::status_suspended(),
                "can't tell the default browser".to_owned()
            )
        );
        assert_eq!(
            with(&|status| status.default = DefaultState::Other("Safari".into())),
            (
                theme::status_warning(),
                "not the default browser".to_owned()
            )
        );
        assert_eq!(
            with(&|status| {
                status.paused = Some(Pause {
                    name: "Google Chrome".into(),
                    display: "Google Chrome \u{00b7} Personal".into(),
                });
            }),
            (
                theme::status_warning(),
                "paused \u{00b7} Google Chrome".to_owned()
            )
        );
        assert_eq!(
            with(&|status| {
                status.config_error = Some(ConfigTrouble::ReloadFailed("x".into()));
            }),
            (
                theme::status_warning(),
                "routing \u{00b7} config didn't reload".to_owned()
            )
        );
        assert_eq!(
            with(&|status| {
                status.config_error = Some(ConfigTrouble::NotLoaded("x".into()));
            }),
            (
                theme::status_error(),
                "routing \u{00b7} config didn't load".to_owned()
            )
        );
    }

    /// The source chip follows the router's word on the config and the
    /// order as written; without a config, what rules.json holds.
    #[test]
    fn the_source_chip_follows_the_router_and_the_saved_precedence() {
        let source = Source {
            config: Some("~/.finicky.ts"),
            trouble: None,
            rules_first: false,
            exists: true,
            rewrites: 1,
            rules: 3,
        };
        let label = |source: Source<'_>| source_label(source).0;
        assert_eq!(label(source), "~/.finicky.ts \u{00b7} runs first");
        assert_eq!(
            label(Source {
                rules_first: true,
                ..source
            }),
            "~/.finicky.ts \u{00b7} after your rules"
        );
        let stale = ConfigTrouble::ReloadFailed("x".into());
        assert_eq!(
            source_label(Source {
                trouble: Some(&stale),
                rules_first: true,
                ..source
            }),
            (
                "~/.finicky.ts \u{00b7} didn't reload".to_owned(),
                theme::status_warning()
            )
        );
        let broken = ConfigTrouble::NotLoaded("x".into());
        assert_eq!(
            source_label(Source {
                trouble: Some(&broken),
                ..source
            }),
            (
                "~/.finicky.ts \u{00b7} didn't load".to_owned(),
                theme::status_error()
            )
        );
        let alone = Source {
            config: None,
            ..source
        };
        assert_eq!(
            label(alone),
            "rules.json \u{00b7} 1 rewrite \u{00b7} 3 rules"
        );
        assert_eq!(
            label(Source {
                rewrites: 0,
                rules: 0,
                ..alone
            }),
            "rules.json \u{00b7} no rules yet"
        );
        assert_eq!(
            label(Source {
                exists: false,
                ..alone
            }),
            "no rules.json yet"
        );
    }
}
