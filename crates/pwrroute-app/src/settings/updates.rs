//! Updates: this copy, what the updater last said, and Check now. Drawn
//! from `design/PwrRoute Updates and Default Browser.dc.html` (1a, 1b),
//! less the automatic-check switch and the download progress the updater
//! does not report yet.

use gpui_kit::{Context, Div, IntoElement, ParentElement as _, SharedString, div};

use super::controls::{
    Kind, Tone, button_when, chip, field, field_help, field_sub, heading, pane_head, row_actions,
};
use super::model::{SettingsRequest, UpdateState};
use super::{SettingsWindow, request_button};

pub const CHECK_ID: &str = "updates-check";
pub const RESTART_ID: &str = "updates-restart";

pub fn pane(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> Div {
    div()
        .child(pane_head(
            "Software",
            "Updates",
            "PwrRoute updates itself from GitHub Releases. Rules, rules.json and agent sessions \
             are kept.",
            None,
        ))
        .child(install_card(this, cx))
        .child(checking_card(this, cx))
}

/// The chip in the card's header.
fn state_chip(state: &UpdateState) -> Div {
    match state {
        UpdateState::NeverChecked => chip("not checked yet", Tone::Neutral),
        UpdateState::Checking => chip("checking", Tone::Neutral),
        UpdateState::UpToDate => chip("up to date", Tone::Ok),
        UpdateState::Ready(version) => chip(format!("{version} downloaded"), Tone::Ok),
        UpdateState::Failed(_) => chip("check failed", Tone::Warn),
        UpdateState::NotUpdatable(_) => chip("not updatable here", Tone::Neutral),
    }
}

/// The state row's sentence.
fn state_text(this: &SettingsWindow) -> SharedString {
    let updates = &this.model.updates;
    match &updates.state {
        UpdateState::NeverChecked => "No check has run since PwrRoute started.".into(),
        UpdateState::Checking => "Checking github.com/pwrdrvr/PwrRoute\u{2026}".into(),
        UpdateState::UpToDate => format!(
            "{} is the newest {} release.",
            updates.version, updates.channel
        )
        .into(),
        UpdateState::Ready(version) => format!(
            "{version} is downloaded. Routing pauses for about two seconds while PwrRoute \
             restarts."
        )
        .into(),
        UpdateState::Failed(why) => format!("{why} Routing is unaffected.").into(),
        UpdateState::NotUpdatable(why) => why.clone(),
    }
}

fn install_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let updates = &this.model.updates;
    let version = field(
        "Version",
        Some(field_sub("This copy, and the channel it follows.")),
        [
            field_help(format!(
                "{} \u{00b7} {} channel",
                updates.version, updates.channel
            ))
            .into_any_element(),
            field_help(updates.install.clone()).into_any_element(),
        ],
    );
    let checking = updates.state == UpdateState::Checking;
    let actions = match &updates.state {
        UpdateState::Ready(_) => row_actions()
            .child(request_button(
                RESTART_ID,
                "Restart to update",
                Kind::Primary,
                false,
                SettingsRequest::RestartToUpdate,
                cx,
            ))
            .child(field_help(
                "Later keeps it downloaded; it installs the next time PwrRoute starts for any \
                 reason.",
            )),
        UpdateState::NotUpdatable(_) => row_actions(),
        UpdateState::Failed(_) => row_actions().child(request_button(
            CHECK_ID,
            "Try again",
            Kind::Default,
            true,
            SettingsRequest::CheckForUpdates,
            cx,
        )),
        _ => row_actions().child(
            button_when(CHECK_ID, "Check now", Kind::Default, true, !checking).on_click(
                cx.listener(|this, _, window, cx| {
                    this.request(SettingsRequest::CheckForUpdates, window, cx);
                }),
            ),
        ),
    };
    let state = field(
        "Install",
        Some(field_sub("What the last check found.")),
        [
            field_help(state_text(this)).into_any_element(),
            actions.into_any_element(),
        ],
    );
    this.disclosure(
        "updates-install",
        heading("This copy", "PwrRoute"),
        Some(state_chip(&updates.state).into_any_element()),
        vec![version, state],
        cx,
    )
}

fn checking_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let updates = &this.model.updates;
    let when = updates.last_checked.clone().map_or_else(
        || SharedString::from("Never checked since launch."),
        |at| format!("{at} \u{00b7} github.com/pwrdrvr/PwrRoute").into(),
    );
    let rows = vec![
        field(
            "Checks",
            Some(field_sub(
                "At startup, every 6 hours and from Check for Updates\u{2026} in the app menu. \
                 A download installs when you quit, or from Restart to update.",
            )),
            [field_help(
                "Nothing is downloaded without a newer release on this copy\u{2019}s channel.",
            )
            .into_any_element()],
        ),
        field("Last checked", None, [field_help(when).into_any_element()]),
    ];
    this.disclosure(
        "updates-checking",
        heading("Checking", "Automatic updates"),
        None,
        rows,
        cx,
    )
}
