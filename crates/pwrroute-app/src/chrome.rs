//! The family window chrome: the title strip across the top and the
//! navigation rail down the left.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, Div, HighlightStyle, InteractiveElement as _, IntoElement as _,
    ParentElement as _, Stateful, StatefulInteractiveElement as _, Styled as _, StyledText, div,
    px,
};

use crate::app::{PwrRoute, Screen};
use crate::e2e;
use crate::theme::{self, icon, mono, text, tracked};
use crate::widgets::{ButtonKind, button, key_cap, status_chip};

/// The rail's "Open Settings…" item.
pub const OPEN_SETTINGS_ID: &str = "nav-settings";

/// The rail foot's way out when `rules.json` changed under unsaved edits.
pub const RELOAD_ID: &str = "rules-reload";
pub const OVERWRITE_ID: &str = "rules-overwrite";

/// Where the brand starts, after the traffic lights. The family spec.
pub const BRAND_X: f32 = 96.0;

/// The strip's fill. The 1px bottom border makes the strip 41px in all.
pub const STRIP_HEIGHT: f32 = 40.0;

/// The strip's status chips for what is on screen.
fn strip_chips(app: &PwrRoute) -> Vec<Div> {
    let rules = || {
        status_chip(
            format!(
                "rules.json \u{00b7} {} rewrites \u{00b7} {} handlers",
                app.rewrites.len(),
                app.handlers.len()
            ),
            theme::status_ok(),
        )
    };
    let config = |path: &str| {
        status_chip(
            format!("{path} \u{00b7} in charge"),
            theme::status_warning(),
        )
    };
    match (app.screen, &app.config) {
        (Screen::Rules | Screen::TestLink, Some(in_charge)) => vec![config(&in_charge.path)],
        (Screen::Rules, None) => vec![rules(), crate::default_browser::chip()],
        (Screen::TestLink, None) => vec![rules()],
        (Screen::Activity, _) => vec![status_chip(app.activity.today.label(), theme::status_ok())],
    }
}

/// `.strip`: the brand, the breadcrumb and the status chips.
///
/// The traffic lights are AppKit's own, placed by the window's
/// `TitlebarOptions` (see `main`), so the strip only leaves room for them.
pub fn title_strip(app: &PwrRoute) -> Div {
    let wordmark = StyledText::new("PwrRoute").with_highlights([(
        3..8,
        HighlightStyle {
            color: Some(theme::accent()),
            ..Default::default()
        },
    )]);
    div()
        .on_children_prepainted(e2e::regions(&[
            "titleStrip.brand",
            "titleStrip.chevron",
            "titleStrip.breadcrumb",
            "titleStrip.spacer",
            "titleStrip.chip1",
            "titleStrip.chip2",
        ]))
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
        .child(text("\u{203a}", 14.0, 500.0, theme::text_subtle()))
        .child(text(
            app.screen.label(),
            13.0,
            500.0,
            theme::text_secondary(),
        ))
        .child(div().flex_1())
        .children(strip_chips(app))
}

/// `.rail-item`; `.on` when `selected`.
fn rail_item(
    id: &'static str,
    icon_path: &'static str,
    label: &'static str,
    selected: bool,
    trailing: Option<AnyElement>,
) -> Stateful<Div> {
    let (foreground, icon_color) = if selected {
        (theme::accent_bright(), theme::accent())
    } else {
        (theme::text_secondary(), theme::text_muted())
    };
    div()
        .id(id)
        .debug_selector(|| id.into())
        .relative()
        .flex()
        .items_center()
        .gap(px(9.0))
        .px(px(10.0))
        .py(px(8.0))
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
        .child(text(label, 13.0, 500.0, foreground))
        .when_some(trailing, |item, trailing| {
            item.child(div().ml_auto().child(trailing))
        })
}

/// The section a rail item opens.
fn section(
    app: &PwrRoute,
    cx: &mut Context<'_, PwrRoute>,
    screen: Screen,
    trailing: Option<AnyElement>,
) -> Stateful<Div> {
    let (id, icon_path) = match screen {
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
    )
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

/// `.rail`: the sections, Settings, and the router's status at the foot.
pub fn nav_rail(app: &PwrRoute, cx: &mut Context<'_, PwrRoute>) -> Div {
    let count = |label: String| mono(label, 10.5, 600.0, theme::text_subtle()).into_any_element();
    let rules_trailing = if app.config.is_some() {
        // `.wdot`: the config in charge, not the visual rules, is routing.
        div()
            .ml_auto()
            .mr(px(6.0))
            .child(theme::dot(theme::status_warning()))
            .into_any_element()
    } else {
        mono(
            (app.rewrites.len() + app.handlers.len()).to_string(),
            10.5,
            600.0,
            theme::text_subtle(),
        )
        .into_any_element()
    };
    let foot = if app.config.is_some() {
        div().child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(theme::dot(theme::status_warning()))
                .child(text(
                    "last good config active",
                    11.0,
                    500.0,
                    theme::text_secondary(),
                )),
        )
    } else {
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(theme::dot(theme::status_ok()))
                    // `.rail-foot .st` loses to the later `.st` rule in
                    // the stylesheet, so this line is sans 11px.
                    // The first launch is the router, so the pid is this
                    // process's.
                    .child(text(
                        format!("router \u{00b7} running \u{00b7} pid {}", std::process::id()),
                        11.0,
                        500.0,
                        theme::text_secondary(),
                    )),
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
                    .child(app.store.status_label()),
            )
            .children(app.store.changed_on_disk().then(|| changed_actions(cx)))
    };
    div()
        .on_children_prepainted(e2e::regions(&[
            "rail.rules",
            "rail.test",
            "rail.activity",
            "rail.settingsHeading",
            "rail.openSettings",
            "rail.foot",
        ]))
        .w(px(196.0))
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
        .child(section(app, cx, Screen::Rules, Some(rules_trailing)))
        .child(section(app, cx, Screen::TestLink, None))
        .child(section(
            app,
            cx,
            Screen::Activity,
            app.activity.today.compact().map(count),
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
                Some(key_cap("\u{2318},").into_any_element()),
            )
            .on_click(|_, _, cx| crate::settings::open(cx, None)),
        )
        .child(div().mt_auto().pt(px(10.0)).px(px(8.0)).child(foot))
}
