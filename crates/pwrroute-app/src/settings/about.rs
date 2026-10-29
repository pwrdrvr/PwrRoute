//! About: the version, where this binary was built from, the license and
//! where the project lives. Its buttons open Help's windows (Changelog,
//! License, Third-Party Notices); the addresses are words to copy, which
//! Help's rows open. The Source row and Copy build info follow the design's
//! Build Identity 2a and 2b.

use gpui_kit::{AnyElement, Context, Div, IntoElement, ParentElement as _, Styled as _, div, px};

use super::SettingsWindow;
use super::controls::{
    Kind, Tone, button_when, chip, command_box, field, field_help, field_sub, heading, pane_head,
    row_actions,
};
use super::model::SettingsRequest;
use crate::theme::{self, mono, text};

pub const NOTICES_ID: &str = "about-third-party";
pub const LICENSE_ID: &str = "about-license";
pub const CHANGELOG_ID: &str = "about-changelog";

/// The copy buttons' slots in the window's copied state, past
/// Diagnostics'; their element ids follow from them.
const COPY_ALL: usize = 200;
const COPY_CHECKOUT: usize = 201;
const COPY_BRANCH: usize = 202;
const COPY_COMMIT: usize = 203;
#[cfg(test)]
pub const COPY_ALL_ID: &str = "settings-copy-200";
#[cfg(test)]
pub const COPY_COMMIT_ID: &str = "settings-copy-203";

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
                field_help(format!("PwrRoute {}", about.source.version)).into_any_element(),
                field_help(about.source.build_line()).into_any_element(),
            ],
        ),
        field(
            "Source",
            Some(field_sub(
                "Where this binary was built from, read by git at build time.",
            )),
            source_rows(this, cx),
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
    let source = &about.source;
    let trailing = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .children((!source.is_release()).then(|| chip("Dev build", Tone::Warn)))
        .child(this.copy_button_labeled(COPY_ALL, "Copy build info", source.report(), cx));
    this.disclosure(
        "about-version",
        heading("This copy", "PwrRoute"),
        Some(trailing.into_any_element()),
        rows,
        cx,
    )
}

/// The checkout, the branch (a release's tag) and the commit, each in a
/// command box with its own Copy; then the commit's subject and date, and
/// a warning when the build had uncommitted changes.
fn source_rows(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> Vec<AnyElement> {
    let source = &this.model.about.source;
    let boxed = |index: usize, value: &str, cx: &mut Context<'_, SettingsWindow>| {
        command_box(
            &[value.to_owned().into()],
            Some(this.copy_button(index, value.to_owned(), cx)),
        )
        .into_any_element()
    };
    let mut rows = Vec::new();
    if let Some(checkout) = &source.checkout {
        rows.push(boxed(COPY_CHECKOUT, checkout, cx));
    }
    match (source.is_release(), &source.tag, &source.branch) {
        (true, Some(tag), _) => rows.push(boxed(COPY_BRANCH, tag, cx)),
        (_, _, Some(branch)) => rows.push(boxed(COPY_BRANCH, branch, cx)),
        _ => rows.push(field_help("Detached HEAD: the commit names it.").into_any_element()),
    }
    match &source.commit {
        Some(commit) => rows.push(boxed(COPY_COMMIT, commit, cx)),
        None => rows.push(
            field_help("No commit recorded: this copy was built outside a git checkout.")
                .into_any_element(),
        ),
    }
    if let Some(line) = source.commit_line() {
        rows.push(field_help(line).into_any_element());
    }
    if let Some(line) = source.changes_line() {
        rows.push(
            field_help(line)
                .text_color(crate::theme::warn_text())
                .into_any_element(),
        );
    }
    rows
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
