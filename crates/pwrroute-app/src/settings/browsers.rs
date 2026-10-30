//! 1c Browsers: OS default registration, detected browsers and other apps.

use crate::theme::px;
use gpui_kit::{
    Context, Div, IntoElement, ParentElement as _, SharedString, Styled as _, Window, div,
};

use super::controls::{
    Kind, Tone, badge, button_when, card_description, chip, field, field_help, field_sub,
    field_sub_code, heading, note, pane_head, row_actions, status, switch, tag,
};
use super::model::{BrowserApp, DefaultState, SettingsRequest, Status, Usage};
use super::style::para;
use super::{SettingsWindow, request_button};
use crate::theme::{self, mono};

/// The explicit "make PwrRoute the default browser" button.
pub const REGISTER_ID: &str = "browsers-register-default";

/// The OS's own default-apps page, by its name on each platform.
const SYSTEM_SETTINGS: &str = if cfg!(target_os = "macos") {
    "Open System Settings"
} else if cfg!(target_os = "windows") {
    "Open Default Apps"
} else {
    "Open system settings"
};

/// Which list a row is in, for its element ids.
#[derive(Clone, Copy)]
enum List {
    Detected,
    Other,
}

impl SettingsWindow {
    fn set_app_visible(
        &mut self,
        list: List,
        index: usize,
        visible: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let browsers = &mut self.model.browsers;
        let apps = match list {
            List::Detected => &mut browsers.detected,
            List::Other => &mut browsers.other_apps,
        };
        let Some(app) = apps.get_mut(index) else {
            return;
        };
        app.visible = visible;
        let request = SettingsRequest::SetAppVisible {
            app: app.id.clone(),
            visible,
        };
        self.request(request, window, cx);
        cx.notify();
    }
}

pub fn pane(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> Div {
    div()
        .child(pane_head(
            "Destinations",
            "Browsers",
            "What PwrRoute can open, and whether the OS hands it links in the first place.",
            None,
        ))
        .child(default_card(this, cx))
        .child(detected_card(this, cx))
        .child(other_apps_card(this, cx))
}

fn default_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let default = &this.model.browsers.default;
    let register = |cx: &mut Context<'_, SettingsWindow>| {
        request_button(
            REGISTER_ID,
            "Register as default browser",
            Kind::Primary,
            false,
            SettingsRequest::RegisterDefaultBrowser,
            cx,
        )
        .into_any_element()
    };
    let (state, register, help) = match &default.state {
        DefaultState::PwrRoute => (
            chip("PwrRoute is default", Tone::Ok),
            button_when(
                REGISTER_ID,
                "PwrRoute is the default browser",
                Kind::Default,
                false,
                false,
            )
            .into_any_element(),
            format!(
                "{} If another app takes the default, the strip shows a warning dot and this \
                 button comes back.",
                default
                    .note
                    .as_deref()
                    .unwrap_or("PwrRoute is the default.")
            ),
        ),
        DefaultState::Other(name) => (
            chip(format!("{name} is default"), Tone::Warn),
            register(cx),
            default.note.as_deref().map_or_else(
                || {
                    format!(
                        "Until then the OS opens links in {name}, without PwrRoute\u{2019}s \
                         rules."
                    )
                },
                str::to_owned,
            ),
        ),
        DefaultState::OtherCopy(path) => (
            chip("Another PwrRoute is default", Tone::Warn),
            register(cx),
            default.note.as_deref().map_or_else(
                || {
                    format!(
                        "macOS opens links in the copy at {path}. Register this copy to take \
                         them back; the other copies leave macOS\u{2019}s list until they \
                         launch again."
                    )
                },
                str::to_owned,
            ),
        ),
        DefaultState::Unknown => (
            chip("Default browser unknown", Tone::Neutral),
            register(cx),
            default.note.as_deref().map_or_else(
                || {
                    "The OS did not say which app handles links. Until PwrRoute is the default, \
                     its rules do not run."
                        .to_owned()
                },
                str::to_owned,
            ),
        ),
    };
    let rows = vec![field(
        "Handle http and https",
        Some(field_sub(
            "Registration is explicit and never happens at launch. macOS asks you to confirm; \
             Windows opens its chooser.",
        )),
        [
            row_actions()
                .child(register)
                .child(request_button(
                    "browsers-system-settings",
                    SYSTEM_SETTINGS,
                    Kind::Quiet,
                    true,
                    SettingsRequest::OpenDefaultAppsSettings,
                    cx,
                ))
                .into_any_element(),
            field_help(help).into_any_element(),
        ],
    )];
    this.disclosure(
        "browsers-default",
        heading("OS registration", "Default browser"),
        Some(state.into_any_element()),
        rows,
        cx,
    )
}

/// The `.st` column's word and dot.
fn usage(app: &BrowserApp) -> (SharedString, Status) {
    if !app.visible {
        return ("hidden".into(), Status::Off);
    }
    match &app.usage {
        Usage::Rules { count, all_off } => {
            let rules = if *count == 1 { "rule" } else { "rules" };
            let off = if *all_off { " (off)" } else { "" };
            (format!("in {count} {rules}{off}").into(), Status::Ok)
        }
        Usage::Unused => ("unused".into(), Status::Off),
        Usage::NotInstalled => ("not installed".into(), Status::Error),
    }
}

/// `.brow`: one app. Columns: badge 22, name, profiles up to 220, status
/// 90, switch 36.
fn app_row(
    list: List,
    index: usize,
    app: &BrowserApp,
    cx: &mut Context<'_, SettingsWindow>,
) -> Div {
    // A hidden app is still listed to be shown again: dimmed, its switch off.
    theme::dimmed_if(!app.visible, || app_row_drawn(list, index, app, cx))
}

fn app_row_drawn(
    list: List,
    index: usize,
    app: &BrowserApp,
    cx: &mut Context<'_, SettingsWindow>,
) -> Div {
    let profiles = if app.profiles.is_empty() {
        div().children(app.note.clone().map(note))
    } else {
        app.profiles
            .iter()
            .fold(div().flex().flex_wrap().gap(px(4.0)), |chips, profile| {
                chips.child(tag(profile.clone()))
            })
    };
    let (word, dot) = usage(app);
    let id = match list {
        List::Detected => "browsers-visible",
        List::Other => "browsers-other-visible",
    };
    let visible = app.visible;
    let set = cx.listener(move |this, on: &bool, window, cx| {
        this.set_app_visible(list, index, *on, window, cx);
    });
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .py(px(10.0))
        .px(px(16.0))
        .child(
            div()
                .w(px(22.0))
                .flex_none()
                .child(badge(if app.usage == Usage::NotInstalled {
                    // Never an icon beside "not installed".
                    app.badge.without_icon()
                } else {
                    app.badge.clone()
                })),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .child(para(
                    app.name.clone(),
                    12.5,
                    600.0,
                    1.2,
                    theme::text_primary(),
                ))
                .child(
                    mono(app.location.clone(), 10.0, 500.0, theme::text_subtle())
                        .line_height(px(13.0))
                        .flex_shrink_1()
                        .min_w_0()
                        .text_ellipsis(),
                ),
        )
        .child(div().w(px(220.0)).flex_shrink_1().min_w_0().child(profiles))
        .child(div().w(px(90.0)).flex_none().child(status(word, dot)))
        .child(
            div().w(px(36.0)).flex_none().child(
                switch((id, index), visible)
                    .accessibility_label(SharedString::from(format!("Show {}", app.name)))
                    .on_change(move |on, _, window, cx| set(&on, window, cx)),
            ),
        )
}

fn detected_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let rows = this
        .model
        .browsers
        .detected
        .iter()
        .enumerate()
        .map(|(index, app)| app_row(List::Detected, index, app, cx))
        .collect();
    let rescan = request_button(
        "browsers-rescan",
        "Rescan",
        Kind::Default,
        true,
        SettingsRequest::RescanBrowsers,
        cx,
    );
    this.disclosure(
        "browsers-detected",
        heading("Applications", "Detected browsers").description(card_description(
            "Scanned at launch and on demand. Profiles are read from each browser\u{2019}s own \
             files.",
        )),
        Some(rescan.into_any_element()),
        rows,
        cx,
    )
}

fn other_apps_card(
    this: &SettingsWindow,
    cx: &mut Context<'_, SettingsWindow>,
) -> impl IntoElement {
    let rows: Vec<Div> = this
        .model
        .browsers
        .other_apps
        .iter()
        .enumerate()
        .map(|(index, app)| app_row(List::Other, index, app, cx))
        .collect();
    let description = field_sub_code(
        "Apps the rules name that the scan did not find: anything that accepts a URL, by name, \
         bundle id or path (Finicky\u{2019}s ",
        "appType: bundleId | path",
        "). Name one in a rule and it appears here.",
    );
    this.disclosure(
        "browsers-other",
        heading("By bundle id or path", "Other apps")
            .description(description.text_size(px(11.5)).line_height(px(18.4))),
        None,
        rows,
        cx,
    )
}
