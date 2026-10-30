//! Updates: this copy, what the updater last said, and Check now. Drawn
//! from `design/PwrRoute Updates and Default Browser.dc.html` (1a, 1b),
//! less the automatic-check switch and the download progress the updater
//! does not report yet. The Release channel card is the Pwr family's (as in
//! PwrSnap, PwrGit and PwrAgnt): four slots, Stable or Beta by Latest or
//! Prerelease, each with the release it would install.

use crate::theme::px;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, Div, IntoElement, ParentElement as _, SharedString, Styled as _, div,
};

use super::controls::{
    Kind, Tone, button_when, chip, column_head, field, field_help, field_sub, heading, pane_head,
    radio_dot, row_actions, slot,
};
use super::model::{SettingsRequest, Slot, SlotTile, UpdateState};
use super::style::para;
use super::{SettingsWindow, request_button};
use crate::theme::{self, text};
use crate::update_channel::{Track, Train};

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
        .child(channel_card(this, cx))
        .child(checking_card(this, cx))
}

/// A slot tile's id: "updates-slot-stable-latest".
pub fn slot_id(slot: Slot) -> &'static str {
    match (slot.train, slot.track) {
        (Train::Stable, Track::Latest) => "updates-slot-stable-latest",
        (Train::Stable, Track::Prerelease) => "updates-slot-stable-prerelease",
        (Train::Beta, Track::Latest) => "updates-slot-beta-latest",
        (Train::Beta, Track::Prerelease) => "updates-slot-beta-prerelease",
    }
}

/// `.slot`: one release channel tile, with the release it would install.
fn slot_tile(tile: &SlotTile, cx: &mut Context<'_, SettingsWindow>) -> AnyElement {
    let picked = tile.slot;
    let set = cx.listener(move |this, _: &bool, window, cx| {
        this.request(SettingsRequest::SetUpdateSlot(picked), window, cx);
    });
    let (release, color) = match &tile.release {
        Some(release) => (release.clone(), theme::text_primary()),
        None => (SharedString::from("\u{2014}"), theme::text_subtle()),
    };
    let color = if tile.selected {
        theme::accent_bright()
    } else {
        color
    };
    slot(slot_id(tile.slot), tile.selected)
        .accessibility_label(SharedString::from(format!(
            "{} \u{2014} {}",
            tile.slot,
            tile.release.as_deref().unwrap_or("none")
        )))
        .on_change(move |on, _, window, cx| set(&on, window, cx))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(
                    text(release, 12.0, 600.0, color)
                        .font_family(theme::mono_family(600.0))
                        .line_height(px(14.4)),
                )
                .child(radio_dot(tile.selected)),
        )
        .child(para(
            tile.note.clone(),
            10.5,
            400.0,
            1.45,
            theme::text_subtle(),
        ))
        .when(tile.installed, |tile| {
            tile.child(div().flex().child(chip("installed", Tone::Neutral)))
        })
        .into_any_element()
}

/// Release channel: the 2×2 grid of slots, trains down, tracks across.
fn channel_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let updates = &this.model.updates;
    // The row stretches its tiles to the taller one's height, so the label
    // centers itself in the stretched cell.
    let label = |name: &str| {
        div()
            .w(px(52.0))
            .flex_none()
            .flex()
            .items_center()
            .child(column_head(name))
    };
    let head = div()
        .flex()
        .gap(px(8.0))
        .child(div().w(px(52.0)).flex_none())
        .child(div().flex_1().child(column_head("Latest")))
        .child(div().flex_1().child(column_head("Prerelease")));
    let mut grid = div().w_full().flex().flex_col().gap(px(8.0)).child(head);
    for (train, name) in [(Train::Stable, "Stable"), (Train::Beta, "Beta")] {
        let row = updates
            .slots
            .iter()
            .filter(|tile| tile.slot.train == train)
            .fold(
                div().flex().items_stretch().gap(px(8.0)).child(label(name)),
                |row, tile| row.child(slot_tile(tile, cx)),
            );
        grid = grid.child(row);
    }
    let following = if updates.inferred {
        "Following the build you installed. Pick a slot to pin it."
    } else {
        "Picked here. A slot\u{2019}s release installs when it is newer than this copy; an \
         older one only from Check now."
    };
    let slots = field(
        "Follow this build",
        Some(field_sub(
            "Two trains, two tracks. Stable is the smoke-checked feed; Beta follows main. \
             Latest is smoke-checked within its train; Prerelease is newer and may not \
             install.",
        )),
        [
            grid.into_any_element(),
            field_help(following).into_any_element(),
        ],
    );
    let tone = if updates
        .slots
        .iter()
        .any(|tile| tile.selected && tile.slot.train == Train::Beta)
    {
        Tone::Warn
    } else {
        Tone::Neutral
    };
    this.disclosure(
        "updates-channel",
        heading("Release channel", "Which releases"),
        Some(chip(updates.channel.clone(), tone).into_any_element()),
        vec![slots],
        cx,
    )
}

/// The chip in the card's header.
fn state_chip(state: &UpdateState) -> Div {
    match state {
        UpdateState::NeverChecked => chip("not checked yet", Tone::Neutral),
        UpdateState::Checking => chip("checking", Tone::Neutral),
        UpdateState::UpToDate => chip("up to date", Tone::Ok),
        UpdateState::Ready(version) => chip(format!("{version} downloaded"), Tone::Ok),
        UpdateState::SwitchReady(version) => chip(format!("{version} downloaded"), Tone::Warn),
        UpdateState::NothingPublished => chip("nothing published", Tone::Neutral),
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
        UpdateState::SwitchReady(version) => format!(
            "{version} is downloaded: older than this copy, as the slot picked has it. \
             Restart to switch installs it; quitting does not."
        )
        .into(),
        UpdateState::NothingPublished => {
            format!("Nothing is published on {} yet.", updates.channel).into()
        }
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
                "{} \u{00b7} follows {}",
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
        UpdateState::SwitchReady(_) => row_actions().child(request_button(
            RESTART_ID,
            "Restart to switch",
            Kind::Primary,
            false,
            SettingsRequest::RestartToUpdate,
            cx,
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
                "Nothing is downloaded without a newer release in the slot picked. A release \
                 older than this copy installs only when you ask for it with Check now.",
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
