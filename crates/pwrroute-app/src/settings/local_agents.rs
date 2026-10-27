//! 1a Local agents: the MCP door, its sessions and their roles. Revoke asks
//! once more before it acts; a revoked row offers Allow again (with its
//! last role) and Forget (its next connection is asked about again).

use gpui_kit::base::actions::{SelectDown, SelectUp};
use gpui_kit::base::{Button, Select, box_shadow};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, Div, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    anchored, deferred, div, point, px, rgb,
};

use std::collections::HashMap;

use super::SettingsWindow;
use super::controls::{
    Kind, Tone, button, card_description, chip, column_head, command_box, field, field_help,
    field_sub, field_sub_code, heading, pane_head, status, switch,
};
use super::model::{AgentRole, AgentSession, Grant, Listener, SessionId, SettingsRequest};
use super::style::{self, para};
use crate::theme::{self, mono, text};

/// The pane's cards, for Collapse all.
const CARDS: [&str; 5] = [
    "agents-server",
    "agents-connect",
    "agents-sessions",
    "agents-roles",
    "agents-recent",
];

/// Element ids the tests reach for.
pub const ACCESS_SWITCH_ID: &str = "agents-access";
pub const COLLAPSE_ALL_ID: &str = "agents-collapse-all";
pub const EMPTY_SESSIONS_ID: &str = "agents-no-sessions";
pub const EMPTY_RECENT_ID: &str = "agents-no-recent";

/// A card's one row when it has nothing to list.
fn empty_row(id: &'static str, words: &'static str) -> Div {
    div()
        .py(px(14.0))
        .px(px(16.0))
        .child(
            div()
                .id(id)
                .child(para(words, 11.5, 400.0, 1.5, theme::text_muted())),
        )
}

/// The pane's own view state: which role menu is open, and which row's
/// Revoke waits to be confirmed.
pub struct State {
    menu: Option<Menu>,
    /// The session whose Revoke was clicked once; the second click acts.
    confirming: Option<SessionId>,
    /// Each session's role select trigger, by id, so a producer can add,
    /// drop or reorder sessions under an open window.
    triggers: HashMap<SessionId, FocusHandle>,
    /// The open menu's, which takes the arrow keys.
    menu_focus: FocusHandle,
}

#[derive(Clone)]
struct Menu {
    session: SessionId,
    highlighted: usize,
}

impl State {
    pub fn new(cx: &mut Context<'_, SettingsWindow>) -> Self {
        Self {
            menu: None,
            confirming: None,
            triggers: HashMap::new(),
            menu_focus: cx.focus_handle(),
        }
    }

    /// Closes the role menu and drops an unconfirmed Revoke.
    pub fn close_menu(&mut self) {
        self.menu = None;
        self.confirming = None;
    }

    /// Give every session a trigger handle and forget sessions that left;
    /// close the menu if its session is gone.
    fn sync(&mut self, sessions: &[AgentSession], cx: &mut Context<'_, SettingsWindow>) {
        self.triggers
            .retain(|id, _| sessions.iter().any(|session| &session.id == id));
        for session in sessions {
            self.triggers
                .entry(session.id.clone())
                .or_insert_with(|| cx.focus_handle());
        }
        if let Some(menu) = &self.menu
            && !self.triggers.contains_key(&menu.session)
        {
            self.menu = None;
        }
        if let Some(session) = &self.confirming
            && !self.triggers.contains_key(session)
        {
            self.confirming = None;
        }
    }

    /// The session whose role menu is open, for tests.
    #[cfg(test)]
    pub fn open_menu(&self) -> Option<SessionId> {
        self.menu.as_ref().map(|menu| menu.session.clone())
    }

    /// The session whose Revoke waits to be confirmed, for tests.
    #[cfg(test)]
    pub fn confirming(&self) -> Option<SessionId> {
        self.confirming.clone()
    }
}

impl SettingsWindow {
    fn set_agent_access(&mut self, on: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        let agents = &mut self.model.local_agents;
        agents.enabled = on;
        agents.listener = if on {
            Listener::Listening
        } else {
            Listener::Stopped
        };
        self.request(SettingsRequest::SetAgentAccess(on), window, cx);
        cx.notify();
    }

    fn open_role_menu(&mut self, session: &SessionId, open: bool, cx: &mut Context<'_, Self>) {
        self.agents.menu = if open {
            let agents = &self.model.local_agents;
            let current = agents
                .sessions
                .iter()
                .find(|row| &row.id == session)
                .and_then(|row| agents.roles.iter().position(|role| role.id == row.role))
                .unwrap_or(0);
            Some(Menu {
                session: session.clone(),
                highlighted: current,
            })
        } else {
            None
        };
        cx.notify();
    }

    fn move_highlight(&mut self, step: isize, cx: &mut Context<'_, Self>) {
        let roles = self.model.local_agents.roles.len();
        if let Some(menu) = self.agents.menu.as_mut() {
            menu.highlighted = menu
                .highlighted
                .saturating_add_signed(step)
                .min(roles.saturating_sub(1));
            cx.notify();
        }
    }

    /// Give `session` the role at index `role`, and close its menu.
    pub(crate) fn set_role(
        &mut self,
        session: &SessionId,
        role: usize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.agents.menu = None;
        if let Some(trigger) = self.agents.triggers.get(session) {
            window.focus(trigger, cx);
        }
        let agents = &mut self.model.local_agents;
        let row = agents.sessions.iter_mut().find(|row| &row.id == session);
        let (Some(row), Some(role)) = (row, agents.roles.get(role)) else {
            return;
        };
        if row.role != role.id {
            row.role = role.id.clone();
            let request = SettingsRequest::SetSessionRole {
                session: row.id.clone(),
                role: role.id.clone(),
            };
            self.request(request, window, cx);
        }
        cx.notify();
    }

    /// The first click on Revoke: the row asks once more, and nothing
    /// changes until it is confirmed.
    fn ask_revoke(&mut self, session: &SessionId, cx: &mut Context<'_, Self>) {
        self.agents.menu = None;
        self.agents.confirming = Some(session.clone());
        cx.notify();
    }

    fn cancel_revoke(&mut self, cx: &mut Context<'_, Self>) {
        self.agents.confirming = None;
        cx.notify();
    }

    fn revoke(&mut self, session: &SessionId, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.agents.confirming = None;
        let sessions = &mut self.model.local_agents.sessions;
        let Some(row) = sessions.iter_mut().find(|row| &row.id == session) else {
            return;
        };
        // The producer reports the real date; until it does, say when.
        row.revoked = Some("just now".into());
        row.detail = "revoked just now".into();
        let id = row.id.clone();
        self.agents.menu = None;
        self.request(SettingsRequest::RevokeSession(id), window, cx);
        cx.notify();
    }

    /// Allow a revoked session again, with the role it last had.
    fn allow_again(
        &mut self,
        session: &SessionId,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let sessions = &mut self.model.local_agents.sessions;
        let Some(row) = sessions.iter_mut().find(|row| &row.id == session) else {
            return;
        };
        // The producer reports the real date; until it does, say when.
        row.revoked = None;
        row.detail = "approved just now".into();
        row.last_used = "never".into();
        let request = SettingsRequest::AllowSession {
            session: row.id.clone(),
            role: row.role.clone(),
        };
        self.request(request, window, cx);
        cx.notify();
    }

    /// Drop a revoked session; its next connection is asked about again.
    fn forget(&mut self, session: &SessionId, window: &mut Window, cx: &mut Context<'_, Self>) {
        let sessions = &mut self.model.local_agents.sessions;
        let before = sessions.len();
        sessions.retain(|row| &row.id != session);
        if sessions.len() == before {
            return;
        }
        self.request(SettingsRequest::ForgetSession(session.clone()), window, cx);
        cx.notify();
    }

    fn select_role_card(&mut self, role: usize, cx: &mut Context<'_, Self>) {
        let agents = &mut self.model.local_agents;
        agents.selected_role = agents.roles.get(role).map(|role| role.id.clone());
        cx.notify();
    }
}

pub fn pane(
    this: &mut SettingsWindow,
    window: &mut Window,
    cx: &mut Context<'_, SettingsWindow>,
) -> Div {
    this.agents.sync(&this.model.local_agents.sessions, cx);
    let collapse_label = if this.all_collapsed(&CARDS) {
        "Expand all"
    } else {
        "Collapse all"
    };
    let collapse = button(COLLAPSE_ALL_ID, collapse_label, Kind::Quiet, true)
        .on_click(cx.listener(|this, _, _, cx| this.collapse_all(&CARDS, cx)))
        .into_any_element();
    div()
        .child(pane_head(
            "Access control",
            "Local agents",
            "Authorize agents in PwrRoute\u{2019}s approval window, then manage their sessions \
             and roles here. Revocation applies to every tool and resource request.",
            Some(collapse),
        ))
        .child(server_card(this, cx))
        .child(connect_card(this, cx))
        .child(sessions_card(this, window, cx))
        .child(roles_card(this, cx))
        .child(recent_card(this, cx))
}

fn server_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let agents = &this.model.local_agents;
    // The chip is one word; a failure's reason wraps under the switch.
    let (listener, failure) = match &agents.listener {
        Listener::Listening => (chip("Listening", Tone::Ok), None),
        Listener::Stopped => (chip("Stopped", Tone::Neutral), None),
        Listener::Failed(why) => (
            chip("Not listening", Tone::Bad),
            Some(field_help(why.clone()).into_any_element()),
        ),
    };
    let endpoint = agents.endpoint.clone();
    let rows = vec![
        field(
            "Enable local-agent access",
            Some(field_sub(
                "Serves MCP on a socket only this user can reach. Off stops the listener and \
                 keeps sessions for next time.",
            )),
            [switch(ACCESS_SWITCH_ID, agents.enabled)
                .accessibility_label("Enable local-agent access")
                .on_change({
                    let set = cx.listener(|this, on: &bool, window, cx| {
                        this.set_agent_access(*on, window, cx);
                    });
                    move |on, _, window, cx| set(&on, window, cx)
                })
                .into_any_element()]
            .into_iter()
            .chain(failure),
        ),
        field(
            "Endpoint",
            Some(field_sub(
                "A per-user socket beside the router\u{2019}s. `pwrroute mcp` pipes an agent\u{2019}s \
                 stdio to it.",
            )),
            [
                command_box(
                    std::slice::from_ref(&endpoint),
                    Some(this.copy_button(0, endpoint.to_string(), cx)),
                )
                .into_any_element(),
                field_help(
                    "The first connection under a new client name opens the approval window; \
                     the name is what the client sends, so a role is a guardrail, not proof of \
                     who is calling. Links that pass through these tools stay in memory.",
                )
                .into_any_element(),
            ],
        ),
    ];
    this.disclosure(
        "agents-server",
        heading("Local agent access", "MCP server"),
        Some(listener.into_any_element()),
        rows,
        cx,
    )
}

fn connect_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let [claude, codex] = this.model.local_agents.connect_commands();
    let command = |index: usize, lines: &[SharedString], cx: &mut Context<'_, SettingsWindow>| {
        let all = lines
            .iter()
            .map(SharedString::as_ref)
            .collect::<Vec<_>>()
            .join("\n");
        command_box(lines, Some(this.copy_button(index, all, cx))).into_any_element()
    };
    let rows = vec![
        field(
            claude.client.clone(),
            Some(field_sub_code(
                "",
                "--scope user",
                " registers it for every project.",
            )),
            [command(1, &claude.lines, cx)],
        ),
        field(
            codex.client.clone(),
            Some(field_sub(
                "No OAuth: the approval happens in PwrRoute\u{2019}s window.",
            )),
            [command(2, &codex.lines, cx)],
        ),
    ];
    this.disclosure(
        "agents-connect",
        heading("Connect an agent", "Claude Code and Codex").description(card_description(
            "Run one of these. The approval window opens in PwrRoute the first time the agent \
             connects; its first call waits for the answer.",
        )),
        None,
        rows,
        cx,
    )
}

/// `.sess`: the sessions table's row box. Columns: name, 120, 110, 70.
fn session_row() -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.0))
        .py(px(10.0))
        .px(px(16.0))
}

fn sessions_card(
    this: &SettingsWindow,
    window: &mut Window,
    cx: &mut Context<'_, SettingsWindow>,
) -> impl IntoElement {
    let agents = &this.model.local_agents;
    let active = agents.active_sessions();
    let head = session_row()
        .py(px(7.0))
        .bg(theme::bg_elevated())
        .child(div().flex_1().min_w_0().child(column_head("Session")))
        .child(div().w(px(120.0)).flex_none().child(column_head("Role")))
        .child(
            div()
                .w(px(110.0))
                .flex_none()
                .child(column_head("Last used")),
        )
        .child(div().w(px(ACTIONS_WIDTH)).flex_none());
    let mut rows = vec![head];
    for (index, session) in agents.sessions.iter().enumerate() {
        rows.push(session_line(this, index, session, window, cx));
    }
    if agents.sessions.is_empty() {
        rows.push(empty_row(
            EMPTY_SESSIONS_ID,
            "No agent has connected yet. Add PwrRoute with a command above, then start a new \
             session in that agent: it connects as the session starts, and the approval window \
             opens over your other windows.",
        ));
    }
    this.disclosure(
        "agents-sessions",
        heading("Authorization", "Sessions"),
        Some(chip(format!("{active} active"), Tone::Ok).into_any_element()),
        rows,
        cx,
    )
}

/// The last column: Revoke, then Confirm and Cancel; or Allow again and
/// Forget on a revoked row.
const ACTIONS_WIDTH: f32 = 150.0;

fn session_line(
    this: &SettingsWindow,
    index: usize,
    session: &AgentSession,
    window: &mut Window,
    cx: &mut Context<'_, SettingsWindow>,
) -> Div {
    let revoked = session.revoked.is_some();
    let confirming = this.agents.confirming.as_ref() == Some(&session.id);
    let name_color = if revoked {
        theme::text_muted()
    } else {
        theme::text_primary()
    };
    let name = para(session.name.clone(), 12.0, 600.0, 1.3, name_color)
        .when(revoked, |name| name.line_through());
    let id = session.id.clone();
    session_row()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .child(name)
                .child(
                    mono(session.detail.clone(), 10.0, 500.0, theme::text_subtle())
                        .line_height(px(13.0)),
                ),
        )
        .child(
            div()
                .w(px(120.0))
                .flex_none()
                .child(role_select(this, index, session, window, cx)),
        )
        .child(div().w(px(110.0)).flex_none().child(if revoked {
            chip("Revoked", Tone::Neutral).into_any_element()
        } else {
            text(
                session.last_used.clone(),
                11.5,
                400.0,
                theme::text_secondary(),
            )
            .into_any_element()
        }))
        .child(
            div()
                .w(px(ACTIONS_WIDTH))
                .flex_none()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(4.0))
                .map(|cell| {
                    if revoked {
                        let allow = id.clone();
                        let forget = id.clone();
                        cell.child(
                            button(("agents-allow", index), "Allow again", Kind::Default, true)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.allow_again(&allow, window, cx);
                                })),
                        )
                        .child(
                            button(("agents-forget", index), "Forget", Kind::Quiet, true).on_click(
                                cx.listener(move |this, _, window, cx| {
                                    this.forget(&forget, window, cx);
                                }),
                            ),
                        )
                    } else if confirming {
                        let confirm = id.clone();
                        cell.child(
                            button(
                                ("agents-revoke-confirm", index),
                                "Revoke",
                                Kind::Danger,
                                true,
                            )
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.revoke(&confirm, window, cx);
                                },
                            )),
                        )
                        .child(
                            button(("agents-revoke-cancel", index), "Keep", Kind::Quiet, true)
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_revoke(cx))),
                        )
                    } else {
                        cell.child(
                            button(("agents-revoke", index), "Revoke", Kind::Danger, true)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.ask_revoke(&id, cx);
                                })),
                        )
                    }
                }),
        )
}

/// The role column: gpui-kit's `Select` with an `.inp` trigger and a menu
/// of the roles.
fn role_select(
    this: &SettingsWindow,
    index: usize,
    session: &AgentSession,
    _window: &mut Window,
    cx: &mut Context<'_, SettingsWindow>,
) -> AnyElement {
    let agents = &this.model.local_agents;
    let disabled = session.revoked.is_some();
    let menu = this
        .agents
        .menu
        .clone()
        .filter(|menu| menu.session == session.id);
    let current = agents
        .role(&session.role)
        .map_or_else(|| session.role.0.clone(), |role| role.name.clone());
    let id = session.id.clone();
    let open_change = cx.listener(move |this, open: &bool, _, cx| {
        this.open_role_menu(&id, *open, cx);
    });
    let confirm = cx.listener(move |this, _: &(), window, cx| {
        if let Some(menu) = this.agents.menu.clone() {
            this.set_role(&menu.session, menu.highlighted, window, cx);
        }
    });
    let trigger = div()
        .id(("agents-role-trigger", index))
        .flex()
        .items_center()
        .gap(px(8.0))
        .min_h(px(26.0))
        .px(px(10.0))
        .bg(theme::bg_input())
        .border_1()
        .border_color(if menu.is_some() {
            theme::border_strong()
        } else {
            theme::border_default()
        })
        .rounded(px(7.0))
        .overflow_hidden()
        .child(crate::widgets::one_line(
            current.clone(),
            theme::SANS,
            400.0,
            11.5,
            11.5,
            theme::text_primary(),
        ))
        .child(
            div()
                .ml_auto()
                .child(text("\u{25be}", 10.0, 400.0, theme::text_subtle())),
        )
        .map(|trigger| {
            if disabled {
                trigger.opacity(0.5)
            } else {
                let open = menu.is_some();
                let id = session.id.clone();
                trigger
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_role_menu(&id, !open, cx);
                    }))
            }
        });
    let mut select = Select::new(("agents-role", index))
        .open(menu.is_some())
        .disabled(disabled)
        .accessibility_label("Role")
        .accessibility_value(current)
        .on_open_change(move |open, window, cx| open_change(&open, window, cx))
        .on_confirm(move |window, cx| confirm(&(), window, cx))
        .w_full()
        .child(trigger);
    if let Some(trigger) = this.agents.triggers.get(&session.id) {
        select = select
            .focus_handle(trigger)
            .content_focus_handle(&this.agents.menu_focus);
    }
    select
        .when_some(menu, |select, menu| {
            select.child(deferred(
                anchored()
                    .offset(point(px(0.0), px(4.0)))
                    .snap_to_window_with_margin(px(8.0))
                    .child(role_menu(&agents.roles, menu, &this.agents.menu_focus, cx)),
            ))
        })
        .into_any_element()
}

fn role_menu(
    roles: &[AgentRole],
    menu: Menu,
    focus: &FocusHandle,
    cx: &mut Context<'_, SettingsWindow>,
) -> impl IntoElement {
    roles.iter().enumerate().fold(
        div()
            .id("agents-role-menu")
            .track_focus(focus)
            .w(px(160.0))
            .max_h(px(crate::widgets::MENU_MAX_HEIGHT))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .p(px(4.0))
            .bg(theme::bg_elevated())
            .border_1()
            .border_color(theme::border_default())
            .rounded(px(7.0))
            .shadow(vec![box_shadow(0.0, 18.0, 48.0, 0.0, {
                let mut shadow: gpui_kit::Hsla = rgb(0x000000).into();
                shadow.a = 0.42;
                shadow
            })])
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.agents.close_menu();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SelectUp, _, cx| this.move_highlight(-1, cx)))
            .on_action(cx.listener(|this, _: &SelectDown, _, cx| this.move_highlight(1, cx))),
        |list, (index, role)| {
            let highlighted = index == menu.highlighted;
            let session = menu.session.clone();
            list.child(
                Button::new(("agents-role-option", index))
                    .role(gpui_kit::Role::ListBoxOption)
                    .selected(highlighted)
                    .focusable(false)
                    .justify_start()
                    .gap(px(8.0))
                    .py(px(6.0))
                    .px(px(8.0))
                    .rounded(px(5.0))
                    .cursor_pointer()
                    .when(highlighted, |option| option.bg(theme::bg_hover_strong()))
                    .on_mouse_move(cx.listener(move |this, _, _, cx| {
                        if let Some(menu) = this.agents.menu.as_mut()
                            && menu.highlighted != index
                        {
                            menu.highlighted = index;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_role(&session, index, window, cx);
                    }))
                    .child(crate::widgets::one_line(
                        role.name.clone(),
                        theme::SANS,
                        500.0,
                        11.5,
                        11.5,
                        theme::text_primary(),
                    ))
                    .when(!role.built_in, |option| {
                        option.child(mono("custom", 9.5, 500.0, theme::text_subtle()))
                    }),
            )
        },
    )
}

fn roles_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let agents = &this.model.local_agents;
    let tiles = agents.roles.iter().enumerate().fold(
        div().flex().gap(px(10.0)).py(px(14.0)).px(px(16.0)),
        |tiles, (index, role)| {
            let selected = agents.selected_role.as_ref() == Some(&role.id);
            tiles.child(role_tile(index, role, selected, cx))
        },
    );
    let new_role = button("agents-new-role", "New custom role", Kind::Default, true).on_click(
        cx.listener(|this, _, window, cx| {
            this.request(SettingsRequest::NewCustomRole, window, cx);
        }),
    );
    this.disclosure(
        "agents-roles",
        heading("Permissions", "Roles").description(card_description(
            "Built-in roles cover the three things an agent can want. Permissions are additive; \
             a custom role starts from a copy.",
        )),
        Some(new_role.into_any_element()),
        vec![tiles],
        cx,
    )
}

/// `.role`: one role's card, `.sel` when it is the one shown selected.
fn role_tile(
    index: usize,
    role: &AgentRole,
    selected: bool,
    cx: &mut Context<'_, SettingsWindow>,
) -> Button {
    let permissions = role.permissions.iter().map(|permission| {
        let (glyph, fill, mark, label, label_color) = match &permission.grant {
            Grant::Full => (
                "\u{2713}",
                style::success_soft(),
                style::success_text(),
                permission.tool.clone(),
                theme::text_secondary(),
            ),
            Grant::Partial(limit) => (
                "~",
                style::warn_soft(),
                style::warn_text(),
                SharedString::from(format!("{} \u{00b7} {limit}", permission.tool)),
                theme::text_secondary(),
            ),
            Grant::None => (
                "\u{2014}",
                theme::bg_elevated(),
                theme::text_subtle(),
                permission.tool.clone(),
                theme::text_subtle(),
            ),
        };
        div()
            .flex()
            .items_center()
            .gap(px(7.0))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(12.0))
                    .rounded(px(3.0))
                    .bg(fill)
                    .child(text(glyph, 8.0, 400.0, mark)),
            )
            .child(mono(label, 10.5, 500.0, label_color).line_height(px(13.65)))
    });
    Button::new(("agents-role-card", index))
        .selected(selected)
        .accessibility_label(role.name.clone())
        .flex_1()
        .min_w_0()
        .flex_col()
        .items_stretch()
        .justify_start()
        .gap(px(8.0))
        .py(px(10.0))
        .px(px(12.0))
        .border_1()
        .rounded(px(8.0))
        .map(|tile| {
            if selected {
                tile.bg(theme::bg_row_active())
                    .border_color(style::accent_border())
            } else {
                tile.bg(theme::bg_app())
                    .border_color(theme::border_subtle())
                    .cursor_pointer()
                    .hover(|style| style.border_color(theme::border_default()))
            }
        })
        .on_click(cx.listener(move |this, _, _, cx| this.select_role_card(index, cx)))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(
                    text(role.name.clone(), 12.5, 600.0, theme::text_primary())
                        .line_height(px(15.0)),
                )
                .child(mono(
                    if role.built_in { "built-in" } else { "custom" },
                    9.5,
                    500.0,
                    theme::text_subtle(),
                )),
        )
        .child(para(
            role.summary.clone(),
            10.5,
            400.0,
            1.5,
            theme::text_muted(),
        ))
        .children(permissions)
}

fn recent_card(this: &SettingsWindow, cx: &mut Context<'_, SettingsWindow>) -> impl IntoElement {
    let mut rows: Vec<Div> = this
        .model
        .local_agents
        .recent
        .iter()
        .map(|action| {
            session_row()
                .child(
                    div().w(px(64.0)).flex_none().child(
                        div()
                            .flex()
                            .flex_none()
                            .px(px(9.0))
                            .py(px(4.0))
                            .rounded_full()
                            .border_1()
                            .border_color(theme::border_subtle())
                            .bg(theme::bg_elevated())
                            // The design's inline `font:` resets the line height
                            // to `normal`, about 1.25 for Geist Mono.
                            .child(
                                mono(action.time.clone(), 10.0, 500.0, theme::text_muted())
                                    .line_height(px(12.5)),
                            ),
                    ),
                )
                .child(div().w(px(150.0)).flex_none().child(text(
                    action.client.clone(),
                    11.5,
                    400.0,
                    theme::text_secondary(),
                )))
                .child(div().flex_1().min_w_0().child(crate::widgets::one_line(
                    action.call.clone(),
                    theme::mono_family(500.0),
                    500.0,
                    11.0,
                    11.0,
                    theme::text_primary(),
                )))
                .child(
                    div()
                        .w(px(160.0))
                        .flex_none()
                        .child(status(action.outcome.clone(), action.status)),
                )
        })
        .collect();
    if rows.is_empty() {
        rows.push(empty_row(
            EMPTY_RECENT_ID,
            "No tool calls since PwrRoute started. Calls are listed here by tool, agent and \
             outcome; the links they carried are not kept.",
        ));
    }
    let open_activity = button("agents-open-activity", "Open Activity", Kind::Quiet, true)
        .on_click(cx.listener(|this, _, window, cx| {
            this.request(SettingsRequest::OpenActivity, window, cx);
        }));
    this.disclosure(
        "agents-recent",
        heading("Activity", "Recent agent actions"),
        Some(open_activity.into_any_element()),
        rows,
        cx,
    )
}
