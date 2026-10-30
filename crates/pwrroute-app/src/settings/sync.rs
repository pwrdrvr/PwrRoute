//! Sync (design: `PwrRoute Synced Rules.dc.html`): the folders `rules.json`
//! syncs with, how each stands, a conflict's two sides, and what the
//! synced destinations and rule sets mean on this machine.
//!
//! The pane says before anything else that sync puts `rules.json`, with
//! the example and test URLs saved in it, in the cloud folder chosen.
//! Folders are only touched from a click (Add, Choose a folder…, Sync
//! now); the producer looks for them when the pane is built.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, Div, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
};

use super::SettingsWindow;
use super::controls::{
    Kind, Tone, button, card_description, chip, column_head, field, field_help, field_sub, heading,
    pane_head, row_actions, switch,
};
use super::model::{
    DestinationRow, RuleSetRow, SettingsRequest, SyncConflict, SyncFolder, SyncTone,
};
use super::style::{self, para};
use crate::destinations::{self, Density};
use crate::text_field::{self, Edit, Look, TextField};
use crate::theme::px;
use crate::theme::{self, mono, text};

pub const SYNC_NOW_ID: &str = "sync-now";
pub const CHOOSE_ID: &str = "sync-choose";
pub const NAME_ID: &str = "sync-machine-name";
pub const CONFIG_ID: &str = "sync-config";
pub const BACKUPS_ID: &str = "sync-backups";

/// What the pane keeps while the window is open.
pub struct State {
    name: TextField,
    /// The destination whose lanes are open (Change…).
    editing: Option<SharedString>,
}

impl State {
    pub fn new(sync: &super::model::Sync, cx: &mut Context<'_, SettingsWindow>) -> Self {
        Self {
            name: TextField::new(sync.machine_name.to_string(), cx),
            editing: None,
        }
    }

    pub fn close(&mut self) {
        self.editing = None;
    }
}

fn tone(tone: SyncTone) -> Tone {
    match tone {
        SyncTone::Neutral => Tone::Neutral,
        SyncTone::Ok => Tone::Ok,
        SyncTone::Warn => Tone::Warn,
        SyncTone::Bad => Tone::Bad,
    }
}

impl SettingsWindow {
    fn commit_machine_name(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let name = self.sync.name.text.trim().to_owned();
        if name != self.model.sync.machine_name.as_ref() {
            self.model.sync.machine_name = name.clone().into();
            self.request(SettingsRequest::SetMachineName(name.into()), window, cx);
        }
        cx.notify();
    }
}

pub fn pane(
    this: &SettingsWindow,
    window: &mut Window,
    cx: &mut Context<'_, SettingsWindow>,
) -> Div {
    let sync = &this.model.sync;
    div()
        .child(pane_head(
            "Across machines",
            "Sync",
            "Keep the same rules on every machine through a folder your cloud drive already \
             syncs. PwrRoute reads and writes only its own PwrRoute folder there.",
            None,
        ))
        .when_some(sync.problem.clone(), |pane, problem| {
            pane.child(div().max_w(px(super::CARD_MAX_WIDTH)).child(para(
                problem,
                11.5,
                500.0,
                1.5,
                theme::danger_text(),
            )))
        })
        .when(sync.folders.is_empty(), |pane| {
            pane.child(what_syncs(this, cx))
        })
        .child(folders_card(this, cx))
        .when(!sync.folders.is_empty(), |pane| {
            pane.child(what_syncs(this, cx)).child(files_card(this, cx))
        })
        .child(machine_card(this, window, cx))
}

/// What leaves this machine. First and open before any folder is added;
/// after that, one line under the folders.
fn what_syncs(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    if this.model.sync.folders.is_empty() {
        let copied = style::rich(
            &[
                (
                    "Sync copies rules.json into the folder you choose, ",
                    style::sans(400.0),
                    theme::text_secondary(),
                ),
                (
                    "including the example and test URLs saved with your rules",
                    style::sans(600.0),
                    theme::text_primary(),
                ),
                (
                    ". Anyone who can open that folder, and the company that stores it, can \
                     read them. Activity, the journal and this machine\u{2019}s own settings \
                     never sync.",
                    style::sans(400.0),
                    theme::text_secondary(),
                ),
            ],
            11.5,
            1.6,
        );
        this.disclosure(
            "sync-what",
            heading("Before you add a folder", "What syncs"),
            Some(chip("Off", Tone::Neutral).into_any_element()),
            vec![field(
                "What is copied",
                Some(field_sub(
                    "Said once here, before the first folder is added.",
                )),
                [copied.into_any_element()],
            )],
            cx,
        )
    } else {
        this.disclosure(
            "sync-what",
            heading("What syncs", "rules.json, with its example and test URLs"),
            None,
            vec![field(
                "Stays here",
                None,
                [field_help(
                    "Activity, the journal and this machine\u{2019}s settings never sync. The \
                     folder\u{2019}s cloud service can read what syncs.",
                )
                .into_any_element()],
            )],
            cx,
        )
    }
}

fn folders_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let sync = &this.model.sync;
    let mut rows: Vec<Div> = sync
        .folders
        .iter()
        .enumerate()
        .map(|(i, folder)| folder_row(i, folder, cx))
        .collect();
    if !sync.offered.is_empty() {
        let found = if sync.folders.is_empty() {
            "Found on "
        } else {
            "Also on "
        };
        rows.push(
            div()
                .pt(px(10.0))
                .pb(px(2.0))
                .px(px(16.0))
                .child(column_head(&format!("{found}{}", super::sync_host::here()))),
        );
    }
    for (i, place) in sync.offered.iter().enumerate() {
        let root = place.root.clone();
        rows.push(field(
            place.label.clone(),
            Some(path_line(place.shown.clone())),
            [button(("sync-add", i), "Add", Kind::Default, true)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.request(SettingsRequest::AddSyncFolder(root.clone()), window, cx);
                }))
                .into_any_element()],
        ));
    }
    let other = if cfg!(target_os = "linux") {
        "An rclone, Insync or GNOME Online Accounts mount, or a network share."
    } else if cfg!(windows) {
        "Another drive letter, a OneDrive folder, a network share."
    } else {
        "A folder another app keeps in sync: an rclone or Insync folder, a network share."
    };
    rows.push(field(
        "Choose a folder\u{2026}",
        Some(field_sub(other)),
        [button(CHOOSE_ID, "Choose\u{2026}", Kind::Default, true)
            .on_click(cx.listener(|this, _, window, cx| {
                this.request(SettingsRequest::ChooseSyncFolder, window, cx);
            }))
            .into_any_element()],
    ));
    if !sync.folders.is_empty() {
        let backups = (sync.backups > 0).then(|| {
            button(BACKUPS_ID, "Show backups", Kind::Quiet, true)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.request(SettingsRequest::RevealSyncBackups, window, cx);
                }))
                .into_any_element()
        });
        rows.push(
            div()
                .flex()
                .items_center()
                .gap(px(10.0))
                .py(px(10.0))
                .px(px(16.0))
                .child(field_help(format!(
                    "Last synced {} \u{00b7} Checks every minute, when PwrRoute comes to the \
                     front and after every save.",
                    sync.last_synced
                )))
                .children(backups),
        );
    }
    let trailing = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(chip(sync.summary.clone(), tone(sync.summary_tone)))
        .when(!sync.folders.is_empty(), |head| {
            head.child(
                button(SYNC_NOW_ID, "Sync now", Kind::Default, true).on_click(cx.listener(
                    |this, _, window, cx| {
                        this.request(SettingsRequest::SyncNow, window, cx);
                    },
                )),
            )
        });
    this.disclosure(
        "sync-folders",
        heading("Cloud folders", "Sync to").description(card_description(
            "Add one or more. PwrRoute makes a PwrRoute folder inside and keeps rules.json there.",
        )),
        Some(trailing.into_any_element()),
        rows,
        cx,
    )
}

/// What goes in the folder: rules.json always, the Finicky config when
/// this machine says so.
fn files_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let sync = &this.model.sync;
    let mut rows = vec![field(
        "rules.json",
        Some(field_sub(
            "Your visual rules, with their examples and tests.",
        )),
        [chip("Always", Tone::Neutral).into_any_element()],
    )];
    if let Some(config) = sync.config.clone() {
        let set = cx.listener(|this, on: &bool, window, cx| {
            this.model.sync.config_on = *on;
            this.request(SettingsRequest::SetSyncConfig(*on), window, cx);
        });
        let saved_as = if config.ends_with(".js") {
            "finicky.js"
        } else {
            "finicky.ts"
        };
        rows.push(field(
            format!("Also sync {config}"),
            Some(para(
                format!("On this machine only. Saved in the folder as {saved_as}."),
                11.0,
                400.0,
                1.5,
                theme::text_muted(),
            )),
            [
                switch(CONFIG_ID, sync.config_on)
                    .accessibility_label(SharedString::from(format!("Also sync {config}")))
                    .on_change(move |on, _, window, cx| set(&on, window, cx))
                    .into_any_element(),
                field_help(format!(
                    "Only this file syncs; files it imports do not. The router keeps reading \
                     {config}, never the cloud folder."
                ))
                .into_any_element(),
            ],
        ));
    }
    this.disclosure(
        "sync-files",
        heading("Files", "What goes in the folder"),
        None,
        rows,
        cx,
    )
}

fn path_line(shown: SharedString) -> Div {
    mono(shown, 10.0, 500.0, theme::text_subtle())
        .line_height(px(13.0))
        .min_w_0()
        .text_ellipsis()
}

fn folder_row(i: usize, folder: &SyncFolder, cx: &mut Context<'_, SettingsWindow>) -> Div {
    let (reveal, remove) = (folder.root.clone(), folder.root.clone());
    let actions = row_actions()
        .child(
            button(("sync-reveal", i), "Reveal", Kind::Quiet, true).on_click(cx.listener(
                move |this, _, window, cx| {
                    this.request(
                        SettingsRequest::RevealSyncFolder(reveal.clone()),
                        window,
                        cx,
                    );
                },
            )),
        )
        .child(
            button(("sync-remove", i), "Remove", Kind::Danger, true).on_click(cx.listener(
                move |this, _, window, cx| {
                    this.request(
                        SettingsRequest::RemoveSyncFolder(remove.clone()),
                        window,
                        cx,
                    );
                },
            )),
        );
    let state = div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .child(chip(folder.chip.clone(), tone(folder.tone)))
        .child(field_help(folder.line.clone()));
    let mut content: Vec<AnyElement> = vec![state.into_any_element()];
    if let Some(conflict) = &folder.conflict {
        content.push(conflict_block(i, folder, conflict, cx).into_any_element());
    }
    content.push(actions.into_any_element());
    field(
        folder.label.clone(),
        Some(path_line(folder.shown.clone())),
        content,
    )
}

fn side(title: SharedString, detail: SharedString) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .py(px(8.0))
        .px(px(10.0))
        .rounded(px(7.0))
        .border_1()
        .border_color(theme::border_subtle())
        .bg(theme::bg_input())
        .child(text(title, 11.5, 600.0, theme::text_primary()))
        .child(mono(detail, 10.5, 500.0, theme::text_muted()))
}

fn conflict_block(
    i: usize,
    folder: &SyncFolder,
    conflict: &SyncConflict,
    cx: &mut Context<'_, SettingsWindow>,
) -> Div {
    let request = |keep_local: bool| SettingsRequest::ResolveSync {
        root: folder.root.clone(),
        file: conflict.file.clone(),
        remote_sha: conflict.remote_sha.clone(),
        keep_local,
    };
    let (keep, take) = (request(true), request(false));
    let (keep_label, take_label): (SharedString, SharedString) = if conflict.join {
        (
            format!("Replace with {}\u{2019}s", super::sync_host::here()).into(),
            "Use the folder\u{2019}s rules".into(),
        )
    } else {
        (
            format!("Keep {}\u{2019}s", super::sync_host::here()).into(),
            format!("Use {}\u{2019}s", conflict.theirs_name).into(),
        )
    };
    let differences = conflict
        .differences
        .iter()
        .fold(div().flex().flex_col().gap(px(2.0)), |list, line| {
            list.child(mono(line.clone(), 10.5, 500.0, theme::text_secondary()))
        });
    let mine_title: SharedString = {
        let here = super::sync_host::here();
        let mut title = here.to_owned();
        if let Some(first) = title.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        title.into()
    };
    let take_kind = if conflict.join {
        Kind::Primary
    } else {
        Kind::Default
    };
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .w_full()
                .flex()
                .gap(px(8.0))
                .child(side(mine_title, conflict.mine.clone()))
                .child(side(conflict.theirs_name.clone(), conflict.theirs.clone())),
        )
        .child(column_head(&format!(
            "What {}\u{2019}s has, against {}\u{2019}s",
            conflict.theirs_name,
            super::sync_host::here()
        )))
        .child(differences)
        .child(
            row_actions()
                .child(
                    button(("sync-keep", i), keep_label, Kind::Default, true).on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.request(keep.clone(), window, cx);
                        }),
                    ),
                )
                .child(
                    button(("sync-take", i), take_label, take_kind, true).on_click(cx.listener(
                        move |this, _, window, cx| {
                            this.request(take.clone(), window, cx);
                        },
                    )),
                ),
        )
        .child(field_help(if conflict.join {
            format!(
                "{} rules are saved to backups first, whichever you choose.",
                conflict.mine
            )
        } else {
            "The one you don\u{2019}t keep is saved to backups.".to_owned()
        }))
        .child(field_help(conflict.headline.clone()))
}

fn machine_card(
    this: &SettingsWindow,
    window: &mut Window,
    cx: &mut Context<'_, SettingsWindow>,
) -> impl IntoElement {
    let sync = &this.model.sync;
    let input = text_field::field(
        NAME_ID,
        &this.sync.name,
        Look {
            mono: false,
            placeholder: "",
        },
        None,
        window,
    )
    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
        match this.sync.name.key(&event.keystroke, cx) {
            Edit::Ignored => return,
            Edit::Submit => this.commit_machine_name(window, cx),
            Edit::Changed | Edit::Handled => {}
        }
        cx.stop_propagation();
        cx.notify();
    }))
    .on_click(cx.listener(|this, _, window, cx| {
        let focus = this.sync.name.focus.clone();
        window.focus(&focus, cx);
        cx.notify();
    }));
    let help = if sync.machine_name.is_empty() {
        format!("Starts as the computer name, {}.", sync.computer_name)
    } else {
        "Return saves it.".to_owned()
    };
    let mut rows = vec![field(
        "Name",
        Some(field_sub(
            "How your other machines see this one in sync status.",
        )),
        [
            div().w(px(260.0)).flex().child(input).into_any_element(),
            field_help(help).into_any_element(),
        ],
    )];
    if sync.destinations.is_empty() {
        rows.push(field(
            "Destinations",
            Some(field_sub("Named places a synced rule opens in.")),
            [field_help(
                "None yet. A rule that opens in a named destination (\u{201c}Work\u{201d}, \
                 \u{201c}Personal\u{201d}) lets each machine say what that name means here.",
            )
            .into_any_element()],
        ));
    }
    for (i, row) in sync.destinations.iter().enumerate() {
        rows.push(destination_row(this, i, row, cx));
    }
    for (i, set) in sync.sets.iter().enumerate() {
        rows.push(set_row(i, set, cx));
    }
    this.disclosure(
        "sync-machine",
        heading("Never syncs \u{00b7} machine.json", "This machine"),
        None,
        rows,
        cx,
    )
}

fn destination_row(
    this: &SettingsWindow,
    i: usize,
    row: &DestinationRow,
    cx: &mut Context<'_, SettingsWindow>,
) -> Div {
    let open = this.sync.editing.as_ref() == Some(&row.name);
    let name = row.name.clone();
    let toggle = cx.listener(move |this, _, _, cx| {
        this.sync.editing = if this.sync.editing.as_ref() == Some(&name) {
            None
        } else {
            Some(name.clone())
        };
        cx.notify();
    });
    let source = if row.this_machine {
        chip(super::sync_host::here(), Tone::Ok)
    } else {
        chip("synced", Tone::Neutral)
    };
    let mut content: Vec<AnyElement> = vec![
        div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .child(text(row.here.clone(), 12.0, 600.0, theme::text_primary()))
            .child(source)
            .into_any_element(),
    ];
    if let Some(warning) = &row.warning {
        content
            .push(para(warning.clone(), 11.0, 500.0, 1.5, style::warn_text()).into_any_element());
    }
    let mut actions = row_actions().child(
        button(
            ("sync-change", i),
            if open { "Done" } else { "Change\u{2026}" },
            Kind::Default,
            true,
        )
        .on_click(toggle),
    );
    if row.this_machine {
        let name = row.name.clone();
        actions = actions.child(
            button(("sync-use-synced", i), "Use synced", Kind::Quiet, true).on_click(cx.listener(
                move |this, _, window, cx| {
                    this.request(SettingsRequest::UseSyncedBinding(name.clone()), window, cx);
                },
            )),
        );
    }
    content.push(actions.into_any_element());
    if open {
        let name = row.name.clone();
        content.push(
            div()
                .w_full()
                .child(destinations::draw(
                    "sync-lanes",
                    &row.lanes,
                    Density::Regular,
                    move |this: &mut SettingsWindow, choice, window, cx| {
                        this.sync.editing = None;
                        this.request(
                            SettingsRequest::BindDestination {
                                name: name.clone(),
                                browser: choice.browser.into(),
                                profile: choice.profile.into(),
                                private: choice.private,
                            },
                            window,
                            cx,
                        );
                    },
                    cx,
                ))
                .into_any_element(),
        );
    }
    let uses = match row.uses {
        0 => "No rule opens in it yet.".to_owned(),
        1 => "1 rule opens in it.".to_owned(),
        n => format!("{n} rules open in it."),
    };
    let sub = para(uses, 11.0, 400.0, 1.5, theme::text_muted());
    field(row.name.clone(), Some(sub), content)
}

fn set_row(i: usize, set: &RuleSetRow, cx: &mut Context<'_, SettingsWindow>) -> Div {
    let name = set.name.clone();
    let change = cx.listener(move |this, on: &bool, window, cx| {
        this.request(
            SettingsRequest::SetRuleSetHere {
                name: name.clone(),
                on: *on,
            },
            window,
            cx,
        );
    });
    let entries = if set.entries == 1 {
        "1 rule".to_owned()
    } else {
        format!("{} rules", set.entries)
    };
    let default = if set.synced_on { "on" } else { "off" };
    let help = if set.overridden {
        format!("Synced default: {default}. This machine overrides it.")
    } else {
        format!("Synced default: {default}.")
    };
    field(
        format!("Rule set \u{00b7} {}", set.name),
        Some(para(entries, 11.0, 400.0, 1.5, theme::text_muted())),
        [
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    switch(("sync-set", i), set.on)
                        .accessibility_label(SharedString::from(format!("{} on here", set.name)))
                        .on_change(move |on, _, window, cx| change(&on, window, cx)),
                )
                .child(text(
                    if set.on { "On here" } else { "Off here" },
                    11.5,
                    500.0,
                    theme::text_secondary(),
                ))
                .into_any_element(),
            field_help(help).into_any_element(),
        ],
    )
}
