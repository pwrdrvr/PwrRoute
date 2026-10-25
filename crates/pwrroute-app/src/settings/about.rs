//! About: the version, the license and where the project lives. Its
//! buttons open Help's windows (Changelog, License, Third-Party Notices);
//! the addresses are words to copy, which Help's rows open.

use gpui_kit::{Context, Div, IntoElement, ParentElement as _, Styled as _, div, px};

use super::SettingsWindow;
use super::controls::{
    Kind, button_when, field, field_help, field_sub, heading, pane_head, row_actions,
};
use super::model::SettingsRequest;
use crate::theme::{self, mono, text};

pub const NOTICES_ID: &str = "about-third-party";
pub const LICENSE_ID: &str = "about-license";
pub const CHANGELOG_ID: &str = "about-changelog";

pub fn pane(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> Div {
    div()
        .child(pane_head(
            "PwrRoute",
            "About",
            "Becomes the default browser and routes each link to a browser and profile by \
             rules, in the Pwr family with PwrSnap, PwrGit and PwrAgnt.",
            None,
        ))
        .child(version_card(this, cx))
        .child(project_card(this, cx))
}

fn version_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let about = &this.model.about;
    let rows = vec![
        field(
            "Version",
            None,
            [
                field_help(format!("PwrRoute {}", about.version)).into_any_element(),
                field_help(about.build.clone()).into_any_element(),
            ],
        ),
        field(
            "License",
            Some(field_sub(
                "Third-party notices list every crate and bundled asset this copy ships.",
            )),
            [
                field_help(about.license.clone()).into_any_element(),
                row_actions()
                    .child(about_button(
                        LICENSE_ID,
                        "View license",
                        SettingsRequest::OpenLicense,
                        cx,
                    ))
                    .child(about_button(
                        NOTICES_ID,
                        "Third-party notices",
                        SettingsRequest::OpenThirdPartyNotices,
                        cx,
                    ))
                    .child(about_button(
                        CHANGELOG_ID,
                        "Changelog",
                        SettingsRequest::OpenChangelog,
                        cx,
                    ))
                    .into_any_element(),
            ],
        ),
    ];
    this.disclosure(
        "about-version",
        heading("This copy", "PwrRoute"),
        None,
        rows,
        cx,
    )
}

fn about_button(
    id: &'static str,
    label: &'static str,
    request: SettingsRequest,
    cx: &mut Context<'_, SettingsWindow>,
) -> impl IntoElement {
    button_when(id, label, Kind::Default, true, true).on_click(cx.listener(
        move |this, _, window, cx| {
            this.request(request.clone(), window, cx);
        },
    ))
}

fn project_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let rows = this
        .model
        .about
        .links
        .iter()
        .map(|(label, address)| {
            div()
                .flex()
                .items_center()
                .gap(px(20.0))
                .py(px(12.0))
                .px(px(16.0))
                .child(div().w(px(200.0)).flex_none().child(text(
                    label.clone(),
                    12.0,
                    600.0,
                    theme::text_primary(),
                )))
                .child(mono(address.clone(), 11.0, 500.0, theme::text_secondary()))
        })
        .collect();
    this.disclosure(
        "about-project",
        heading("On GitHub", "Project").description(super::controls::card_description(
            "Addresses to copy. Help \u{203a} View Source and Report an Issue\u{2026} open them in the \
             browser your rules pick.",
        )),
        None,
        rows,
        cx,
    )
}
