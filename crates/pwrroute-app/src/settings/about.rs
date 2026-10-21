//! About: the version, the license and where the project lives, as plain
//! text. The window opens no link: addresses are words to copy.

use gpui_kit::{Context, Div, IntoElement, ParentElement as _, Styled as _, div, px};

use super::SettingsWindow;
use super::controls::{
    Kind, button_when, field, field_help, field_sub, heading, pane_head, row_actions,
};
use super::model::SettingsRequest;
use crate::theme::{self, mono, text};

pub const NOTICES_ID: &str = "about-third-party";
pub const NOTES_ID: &str = "about-release-notes";

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
                    .child(
                        button_when(
                            NOTICES_ID,
                            "Third-party notices",
                            Kind::Default,
                            true,
                            about.has_third_party_notices,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.request(SettingsRequest::RevealThirdPartyNotices, window, cx);
                        })),
                    )
                    .child(
                        button_when(
                            NOTES_ID,
                            "Release notes",
                            Kind::Default,
                            true,
                            about.has_release_notes,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.request(SettingsRequest::OpenReleaseNotes, window, cx);
                        })),
                    )
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
            "Addresses to type or copy; PwrRoute opens no page itself.",
        )),
        None,
        rows,
        cx,
    )
}
