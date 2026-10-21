//! Diagnostics: where PwrRoute's files are, whether the router and the
//! MCP door listen, and what routes links. Paths and states only; nothing
//! here can show a link.

use gpui_kit::{Context, Div, IntoElement, ParentElement as _, div};

use super::controls::{
    Kind, Tone, button_when, chip, command_box, field, field_help, field_sub, heading, pane_head,
    row_actions,
};
use super::model::SettingsRequest;
use super::{SettingsWindow, request_button};

pub const OPEN_DATA_ID: &str = "diagnostics-open-data";

/// The Copy button's slot in the window's copied state, past the connect
/// commands' indexes; its element id follows from it.
const COPY_INDEX: usize = 100;
#[cfg(test)]
pub const COPY_ID: &str = "settings-copy-100";

pub fn pane(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> Div {
    let copy = this.copy_button(COPY_INDEX, this.model.diagnostics.report(), cx);
    div()
        .child(pane_head(
            "Support",
            "Diagnostics",
            "What this copy runs with. Copy carries these lines and nothing else: no link ever \
             appears here.",
            Some(copy),
        ))
        .child(this_copy_card(this, cx))
        .child(router_card(this, cx))
}

fn this_copy_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let diagnostics = &this.model.diagnostics;
    let rows = vec![
        field(
            "Version",
            None,
            [field_help(diagnostics.version.clone()).into_any_element()],
        ),
        field(
            "Operating system",
            None,
            [field_help(diagnostics.os.clone()).into_any_element()],
        ),
        field(
            "Data folder",
            Some(field_sub(
                "rules.json, settings.json, the journal and the agent sessions.",
            )),
            [
                command_box(std::slice::from_ref(&diagnostics.data_dir), None).into_any_element(),
                row_actions()
                    .child(
                        button_when(
                            OPEN_DATA_ID,
                            "Open data folder",
                            Kind::Default,
                            true,
                            diagnostics.data_dir_exists,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.request(SettingsRequest::OpenDataFolder, window, cx);
                        })),
                    )
                    .into_any_element(),
            ],
        ),
        field(
            "Log",
            None,
            [field_help(diagnostics.log.clone()).into_any_element()],
        ),
    ];
    this.disclosure(
        "diagnostics-copy-card",
        heading("This copy", "PwrRoute"),
        None,
        rows,
        cx,
    )
}

fn router_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let diagnostics = &this.model.diagnostics;
    let state = if diagnostics.router_listening {
        chip("Routing", Tone::Ok)
    } else {
        chip("Not listening", Tone::Warn)
    };
    let mut rows = vec![
        field(
            "Router",
            Some(field_sub(
                "The process that receives links from the OS and from later launches.",
            )),
            [field_help(diagnostics.router.clone()).into_any_element()],
        ),
        field(
            "Routing with",
            Some(field_sub("What decides where each link goes right now.")),
            [
                field_help(diagnostics.routing.clone()).into_any_element(),
                row_actions()
                    .child(request_button(
                        "diagnostics-reload",
                        "Reload now",
                        Kind::Default,
                        true,
                        SettingsRequest::ReloadRules,
                        cx,
                    ))
                    .into_any_element(),
            ],
        ),
        field(
            "Local agents",
            None,
            [field_help(diagnostics.agents.clone()).into_any_element()],
        ),
    ];
    if diagnostics.dry_run {
        rows.push(field(
            "Dry run",
            None,
            [
                field_help("Links are resolved and recorded, never opened (--dry-run).")
                    .into_any_element(),
            ],
        ));
    }
    this.disclosure(
        "diagnostics-router",
        heading("Resident", "Router"),
        Some(state.into_any_element()),
        rows,
        cx,
    )
}
