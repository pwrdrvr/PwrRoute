//! Local agents: the MCP server the app serves, its approval window, and
//! the Settings › Local agents pane's producer.
//!
//! [`install`] starts [`pwrroute_mcp::McpHost`] on `<data dir>/mcp.sock`
//! (a named pipe on Windows) beside the router's socket, backed by the real
//! router handle ([`pwrroute_mcp::live`]), the browser catalog, `rules.json`
//! and an engine of its own that follows the same config and rules the
//! router routes by. `pwrroute mcp` ([`pwrroute_mcp::proxy`]) is what an
//! agent's MCP client launches; it pipes stdio to that socket.
//!
//! Approvals: the first connection under a new client name sends an
//! [`ApprovalRequest`] here, and [`ApprovalWindow`] asks the person for a
//! role. Nothing an agent sends can approve. [`Agents`] remembers the
//! answer in `mcp-agents.json` (names, roles, timestamps; never a URL).
//!
//! Nothing here writes a URL anywhere: the audit kept for the pane's
//! "Recent agent actions" is tool, client, time and outcome, and the pane
//! draws exactly that.

use std::{
    cell::RefCell,
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, Local};
use gpui_kit::base::Button;
use gpui_kit::{
    App, AppContext as _, BorrowAppContext as _, Bounds, Context, FocusHandle, Global,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions, div, point, px, size,
};
use pwrroute_engine::{
    ConfigChoice, Limits,
    discovery::{self, Environment},
};
use pwrroute_mcp::{
    Agents, ApprovalReply, ApprovalRequest, Approver, AuditRecord, AuditSink, Backends, Config,
    Endpoint, EngineTester, MemoryAudit, Outcome, Permission, Role, RulesTester,
    live::{LiveBrowsers, LiveRouter, LiveRules},
};
use pwrroute_router::RouterHandle;
use serde::{Deserialize, Serialize};

use crate::config_view::display_path;
use crate::settings::model::{
    AgentAction, AgentRole, AgentSession, Grant, Listener, LocalAgents, Permission as Shown,
    RoleId, SessionId, SettingsRequest, Status,
};
use crate::theme::{self, mono, text};
use crate::widgets::{ButtonKind, button};

/// The switch's state, in `<data dir>/mcp.json`. Sessions are kept in
/// `mcp-agents.json` whatever the switch says.
const PREFERENCES_FILE: &str = "mcp.json";

/// Tool calls the pane's "Recent agent actions" can show.
const RECENT_ACTIONS: usize = 20;

/// The approval window's size: the family's small dialog.
const WINDOW_SIZE: (f32, f32) = (480.0, 400.0);

/// Element ids the tests reach for.
pub const APPROVE_ID_PREFIX: &str = "mcp-approve";
pub const DENY_ID: &str = "mcp-deny";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Preferences {
    /// `false` keeps the listener off across launches.
    #[serde(default = "yes")]
    local_agents: bool,
}

fn yes() -> bool {
    true
}

impl Preferences {
    fn load(dir: Option<&Path>) -> Self {
        let Some(dir) = dir else {
            return Self { local_agents: true };
        };
        match fs::read_to_string(dir.join(PREFERENCES_FILE)) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Self { local_agents: true },
        }
    }

    fn save(&self, dir: Option<&Path>) -> io::Result<()> {
        let Some(dir) = dir else {
            return Ok(());
        };
        fs::create_dir_all(dir)?;
        fs::write(
            dir.join(PREFERENCES_FILE),
            serde_json::to_string_pretty(self).unwrap_or_default(),
        )
    }
}

/// An audit that keeps the last records in memory and tells the pane when
/// one lands. Tool, client, time and outcome: never arguments.
struct AppAudit {
    inner: MemoryAudit,
    changed: async_channel::Sender<()>,
}

impl AuditSink for AppAudit {
    fn record(&self, record: AuditRecord) {
        self.inner.record(record);
        let _ = self.changed.try_send(());
    }
}

/// Spawns the rules engine on first use, so a launch does not wait for a
/// QuickJS load it may never need. Backend calls run on blocking threads,
/// so the first `pwrroute_rules_test` pays for the load.
struct LazyTester {
    choice: ConfigChoice,
    rules_path: Option<PathBuf>,
    tester: Mutex<Option<EngineTester>>,
}

impl LazyTester {
    fn get(&self) -> anyhow::Result<EngineTester> {
        let mut slot = self.tester.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(tester) = slot.as_ref() {
            return Ok(tester.clone());
        }
        let tester = EngineTester::spawn(
            self.choice.clone(),
            self.rules_path.clone(),
            Limits::default(),
        )?;
        *slot = Some(tester.clone());
        Ok(tester)
    }
}

impl RulesTester for LazyTester {
    fn test(
        &self,
        proposal: Option<&pwrroute_engine::RulesFile>,
    ) -> anyhow::Result<pwrroute_engine::TestReport> {
        self.get()?.test(proposal)
    }

    fn collisions(&self, pattern: &str, urls: &[String]) -> anyhow::Result<Vec<String>> {
        self.get()?.collisions(pattern, urls)
    }
}

/// What a prompt asks of the window thread.
enum Prompt {
    Ask(ApprovalRequest, ApprovalReply),
    Cancel(String),
}

/// The server's approver: hands each prompt to GPUI, which opens an
/// [`ApprovalWindow`] for it.
struct GpuiApprover {
    prompts: async_channel::Sender<Prompt>,
}

impl Approver for GpuiApprover {
    fn request(&self, request: ApprovalRequest, reply: ApprovalReply) {
        // Unbounded: never blocks the server. Nobody listening (the app is
        // quitting) drops the reply, which is a denial.
        let _ = self.prompts.try_send(Prompt::Ask(request, reply));
    }

    fn cancel(&self, id: &str) {
        let _ = self.prompts.try_send(Prompt::Cancel(id.to_owned()));
    }
}

/// The door, as the app holds it.
struct Door {
    data_dir: Option<PathBuf>,
    agents: Agents,
    audit: Arc<AppAudit>,
    backends: Backends,
    approver: Arc<GpuiApprover>,
    endpoint: Endpoint,
    host: Option<pwrroute_mcp::McpHost>,
    /// Why the server is not listening although the switch is on.
    failure: Option<String>,
    enabled: bool,
}

impl Door {
    fn listener(&self) -> Listener {
        match (&self.host, &self.failure) {
            (Some(_), _) => Listener::Listening,
            (None, Some(why)) => Listener::Failed(why.clone().into()),
            (None, None) => Listener::Stopped,
        }
    }

    fn start(&mut self) {
        if self.host.is_some() {
            return;
        }
        let started = pwrroute_mcp::McpHost::start(
            Config {
                endpoint: self.endpoint.clone(),
                ..Config::default()
            },
            self.agents.clone(),
            self.backends.clone(),
            self.approver.clone(),
            self.audit.clone(),
        );
        match started {
            Ok(host) => {
                self.host = Some(host);
                self.failure = None;
            }
            Err(e) => {
                eprintln!("pwrroute: local agents: {e:#}");
                self.failure = Some(format!("{e:#}"));
            }
        }
    }

    fn stop(&mut self) {
        if let Some(host) = self.host.take() {
            host.stop();
        }
        self.failure = None;
    }
}

struct DoorGlobal(Rc<RefCell<Door>>);

impl Global for DoorGlobal {}

/// The approval windows open now, by prompt id.
#[derive(Default)]
struct Prompts(HashMap<String, WindowHandle<ApprovalWindow>>);

impl Global for Prompts {}

/// What the server needs from the router's launch: which config it loads
/// and where `rules.json` is, so the engine behind `pwrroute_rules_test`
/// follows the same files.
pub struct Wiring {
    pub handle: RouterHandle,
    pub config: Option<PathBuf>,
}

/// Starts serving local agents, unless the switch was left off. Call once,
/// after the router is up; Settings reads [`local_agents`] from here on.
pub fn install(wiring: Wiring, cx: &mut App) {
    let data_dir = pwrroute_os::data_dir();
    let preferences = Preferences::load(data_dir.as_deref());
    let (audit_tx, audit_rx) = async_channel::unbounded::<()>();
    let (prompt_tx, prompt_rx) = async_channel::unbounded::<Prompt>();
    let agents = match Agents::open(data_dir.as_deref()) {
        Ok(agents) => agents,
        Err(e) => {
            // Failing closed keeps a damaged file from forgetting a
            // revocation: no door at all until the file is looked at.
            eprintln!("pwrroute: local agents: {e:#}");
            let agents = Agents::open(None).expect("an in-memory store");
            let door = Door {
                data_dir: data_dir.clone(),
                agents: agents.clone(),
                audit: Arc::new(AppAudit {
                    inner: MemoryAudit::default(),
                    changed: audit_tx,
                }),
                backends: backends(&wiring),
                approver: Arc::new(GpuiApprover { prompts: prompt_tx }),
                endpoint: Endpoint::for_user(),
                host: None,
                failure: Some(format!("{e:#}")),
                enabled: preferences.local_agents,
            };
            cx.set_global(DoorGlobal(Rc::new(RefCell::new(door))));
            return;
        }
    };
    let mut door = Door {
        data_dir,
        agents: agents.clone(),
        audit: Arc::new(AppAudit {
            inner: MemoryAudit::default(),
            changed: audit_tx,
        }),
        backends: backends(&wiring),
        approver: Arc::new(GpuiApprover { prompts: prompt_tx }),
        endpoint: Endpoint::for_user(),
        host: None,
        failure: None,
        enabled: preferences.local_agents,
    };
    if door.enabled {
        door.start();
    }
    cx.set_global(DoorGlobal(Rc::new(RefCell::new(door))));
    cx.set_global(Prompts::default());

    // Prompts become windows on the GPUI thread.
    cx.spawn(async move |cx| {
        while let Ok(prompt) = prompt_rx.recv().await {
            cx.update(|cx| match prompt {
                Prompt::Ask(request, reply) => open_prompt(request, reply, cx),
                Prompt::Cancel(id) => close_prompt(&id, cx),
            });
        }
    })
    .detach();

    // Settings redraws when an agent connects, calls or is changed, and
    // when a tool call lands in the audit.
    let mut changes = agents.subscribe();
    cx.spawn(async move |cx| {
        while changes.changed().await.is_ok() {
            cx.update(refresh_settings);
        }
    })
    .detach();
    cx.spawn(async move |cx| {
        while audit_rx.recv().await.is_ok() {
            cx.update(refresh_settings);
        }
    })
    .detach();
}

/// The backends over the real thing: the router's handle, this machine's
/// browsers, `rules.json`, and an engine that follows the same files as the
/// router's resolver.
fn backends(wiring: &Wiring) -> Backends {
    let environment = Environment::current();
    let choice = match &wiring.config {
        Some(path) => ConfigChoice::Path(path.clone()),
        None => ConfigChoice::Discover(discovery::config_candidates(&environment)),
    };
    let rules_path = discovery::rules_path(&environment);
    let router = Arc::new(LiveRouter::new(wiring.handle.clone()));
    Backends {
        resolver: router.clone(),
        router: router.clone(),
        browsers: Arc::new(LiveBrowsers::new(pwrroute_browsers::Env::system(), || {
            pwrroute_os::default_browser_status().ok()
        })),
        rules: Arc::new(LiveRules::new(
            rules_path
                .clone()
                .unwrap_or_else(|| PathBuf::from("rules.json")),
            crate::router::config_in_charge,
        )),
        tester: Arc::new(LazyTester {
            choice,
            rules_path,
            tester: Mutex::new(None),
        }),
        activity: router,
    }
}

fn door(cx: &App) -> Option<Rc<RefCell<Door>>> {
    cx.try_global::<DoorGlobal>().map(|d| d.0.clone())
}

fn refresh_settings(cx: &mut App) {
    let model = local_agents(cx);
    crate::settings::update(cx, move |settings| settings.local_agents = model);
}

// ---- The Settings pane ----------------------------------------------------

/// The Settings › Local agents pane as things stand: the real listener,
/// sessions and audit. Contrived fixtures draw it when the door is not
/// installed (the window harness).
pub fn local_agents(cx: &App) -> LocalAgents {
    let Some(door) = door(cx) else {
        return crate::settings::fixtures::local_agents();
    };
    let door = door.borrow();
    model(
        &door.agents,
        &door.audit.inner.records(),
        door.enabled,
        door.listener(),
        &door.endpoint,
        SystemTime::now(),
    )
}

/// The pane's model from the store, the audit and the listener's state.
/// `now` is for "2 min ago".
pub fn model(
    agents: &Agents,
    audit: &[AuditRecord],
    enabled: bool,
    listener: Listener,
    endpoint: &Endpoint,
    now: SystemTime,
) -> LocalAgents {
    let home = std::env::home_dir();
    let sessions = agents
        .agents()
        .into_iter()
        .map(|agent| {
            let calls = if agent.calls == 1 {
                "1 call".to_owned()
            } else {
                format!("{} calls", agent.calls)
            };
            let (detail, revoked) = match agent.revoked_at {
                Some(at) => (format!("revoked {}", day(at)), Some(day(at).into())),
                None => (
                    format!("approved {} \u{00b7} {calls}", day(agent.approved_at)),
                    None,
                ),
            };
            AgentSession {
                id: SessionId(agent.name.clone().into()),
                name: agent.name.into(),
                detail: detail.into(),
                role: role_id(agent.role),
                last_used: agent.last_used_at.map_or_else(
                    || "never".into(),
                    |at| ago(now, UNIX_EPOCH + Duration::from_secs(at)).into(),
                ),
                revoked,
            }
        })
        .collect();
    let recent = audit
        .iter()
        .rev()
        .take(RECENT_ACTIONS)
        .map(|record| {
            let call = match record.note {
                Some(note) => format!("{} \u{00b7} {note}", record.tool),
                None => record.tool.clone(),
            };
            let (outcome, status) = match record.outcome {
                Outcome::Ok => ("ok", Status::Ok),
                Outcome::Denied => ("denied", Status::Off),
                Outcome::Failed => ("failed", Status::Error),
            };
            AgentAction {
                time: clock(record.started).into(),
                client: record.agent.clone().into(),
                call: call.into(),
                outcome: outcome.into(),
                status,
            }
        })
        .collect();
    LocalAgents {
        enabled,
        listener,
        endpoint: display_path(endpoint.path(), home.as_deref()).into(),
        command: connect_command().into(),
        sessions,
        roles: Role::ALL.into_iter().map(role_model).collect(),
        selected_role: None,
        recent,
    }
}

/// `pwrroute mcp`, with the binary's path when it is not on the PATH: a
/// macOS bundle keeps it inside the `.app`.
fn connect_command() -> String {
    let exe = std::env::current_exe().ok();
    let on_path = exe
        .as_ref()
        .and_then(|p| p.file_name())
        .is_some_and(|name| {
            std::env::var_os("PATH")
                .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(name).exists()))
        });
    match exe {
        Some(exe) if !on_path => {
            let path = exe.display().to_string();
            if path.contains(' ') {
                format!("\"{path}\" mcp")
            } else {
                format!("{path} mcp")
            }
        }
        _ => "pwrroute mcp".to_owned(),
    }
}

fn role_id(role: Role) -> RoleId {
    RoleId(role.to_string().to_ascii_lowercase().into())
}

fn role_of(id: &RoleId) -> Option<Role> {
    id.0.parse().ok()
}

/// A built-in role as the Roles card draws it: the design's tool list,
/// with activity partial for a role that sees hosts only.
fn role_model(role: Role) -> AgentRole {
    let tools: [(&str, Permission, Option<Permission>); 7] = [
        ("resolve", Permission::Resolve, None),
        ("browsers", Permission::Browsers, None),
        ("rules_list", Permission::RulesRead, None),
        ("rules_test", Permission::RulesRead, None),
        (
            "activity",
            Permission::ActivityHosts,
            Some(Permission::ActivityUrls),
        ),
        ("open", Permission::Open, None),
        ("rules_apply", Permission::RulesWrite, None),
    ];
    AgentRole {
        id: role_id(role),
        name: role.to_string().into(),
        built_in: true,
        summary: role.description().into(),
        permissions: tools
            .into_iter()
            .map(|(tool, needs, full)| Shown {
                tool: tool.into(),
                grant: if !role.allows(needs) {
                    Grant::None
                } else if full.is_some_and(|full| !role.allows(full)) {
                    Grant::Partial("hosts only".into())
                } else {
                    Grant::Full
                },
            })
            .collect(),
    }
}

fn day(secs: u64) -> String {
    DateTime::<Local>::from(UNIX_EPOCH + Duration::from_secs(secs))
        .format("%Y-%m-%d")
        .to_string()
}

fn clock(at: SystemTime) -> String {
    DateTime::<Local>::from(at).format("%H:%M").to_string()
}

/// "just now", "2 min ago", "3 hours ago", "4 days ago".
fn ago(now: SystemTime, then: SystemTime) -> String {
    let secs = now.duration_since(then).map_or(0, |d| d.as_secs());
    match secs {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => {
            let h = secs / 3600;
            format!("{h} {} ago", if h == 1 { "hour" } else { "hours" })
        }
        _ => {
            let d = secs / 86_400;
            format!("{d} {} ago", if d == 1 { "day" } else { "days" })
        }
    }
}

/// The Settings requests this module answers. `false` for any other.
pub fn handle_settings_request(request: &SettingsRequest, cx: &mut App) -> bool {
    let Some(door) = door(cx) else {
        return false;
    };
    match request {
        SettingsRequest::SetAgentAccess(on) => {
            {
                let mut door = door.borrow_mut();
                door.enabled = *on;
                if *on {
                    door.start();
                } else {
                    door.stop();
                }
                let saved = Preferences { local_agents: *on }.save(door.data_dir.as_deref());
                if let Err(e) = saved {
                    eprintln!("pwrroute: local agents: could not save the switch: {e}");
                }
            }
            refresh_settings(cx);
        }
        SettingsRequest::SetSessionRole { session, role } => {
            let agents = door.borrow().agents.clone();
            match role_of(role) {
                Some(role) => {
                    if let Err(e) = agents.set_role(&session.0, role) {
                        eprintln!("pwrroute: local agents: {e:#}");
                    }
                }
                None => eprintln!("pwrroute: local agents: no role named {}", role.0),
            }
            refresh_settings(cx);
        }
        SettingsRequest::RevokeSession(session) => {
            let agents = door.borrow().agents.clone();
            if let Err(e) = agents.revoke(&session.0) {
                eprintln!("pwrroute: local agents: {e:#}");
            }
            refresh_settings(cx);
        }
        SettingsRequest::OpenActivity => {
            crate::show_main_window(cx);
            let main = cx
                .windows()
                .into_iter()
                .find_map(|window| window.downcast::<crate::app::PwrRoute>());
            if let Some(window) = main {
                let _ = window.update(cx, |view, window, cx| {
                    view.navigate(crate::app::Screen::Activity, window, cx);
                });
            }
        }
        SettingsRequest::NewCustomRole => {
            eprintln!("pwrroute: local agents: custom roles are not built yet");
        }
        _ => return false,
    }
    true
}

// ---- The approval window --------------------------------------------------

fn open_prompt(request: ApprovalRequest, reply: ApprovalReply, cx: &mut App) {
    let id = request.id.clone();
    let bounds = Bounds::centered(None, size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)), cx);
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("PwrRoute".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(
                    px(crate::TRAFFIC_LIGHTS.0),
                    px(crate::TRAFFIC_LIGHTS.1),
                )),
            }),
            app_id: Some("pwrroute".to_owned()),
            window_min_size: Some(size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1))),
            ..Default::default()
        },
        |_, cx| cx.new(|cx| ApprovalWindow::new(request, reply, cx)),
    );
    match opened {
        Ok(window) => {
            let _ = window.update(cx, |this, window, cx| window.focus(&this.focus, cx));
            cx.activate(true);
            cx.update_global::<Prompts, _>(|prompts, _| {
                prompts.0.insert(id, window);
            });
        }
        Err(e) => {
            // The reply is dropped with the window that never was: a denial.
            eprintln!("pwrroute: could not open the approval window: {e}");
        }
    }
}

/// The prompt expired or the server stopped: close its window. Dropping
/// the view drops its reply, which the server already counts as a denial.
fn close_prompt(id: &str, cx: &mut App) {
    let window = cx.update_global::<Prompts, _>(|prompts, _| prompts.0.remove(id));
    if let Some(window) = window {
        let _ = window.update(cx, |_, window, _| window.remove_window());
    }
}

/// "Allow <client> to use PwrRoute?": one window per prompt. A role button
/// approves; Deny, or closing the window, denies.
pub struct ApprovalWindow {
    pub request: ApprovalRequest,
    reply: Option<ApprovalReply>,
    pub focus: FocusHandle,
}

impl ApprovalWindow {
    pub fn new(request: ApprovalRequest, reply: ApprovalReply, cx: &mut Context<'_, Self>) -> Self {
        Self {
            request,
            reply: Some(reply),
            focus: cx.focus_handle(),
        }
    }

    fn answer(&mut self, role: Option<Role>, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(reply) = self.reply.take() {
            match role {
                Some(role) => reply.approve(role),
                None => reply.deny(),
            }
        }
        let id = self.request.id.clone();
        if let Some(prompts) = cx.try_global::<Prompts>()
            && prompts.0.contains_key(&id)
        {
            cx.update_global::<Prompts, _>(|prompts, _| {
                prompts.0.remove(&id);
            });
        }
        window.remove_window();
    }

    /// Whether the prompt still waits for an answer, for tests.
    #[cfg(test)]
    pub fn pending(&self) -> bool {
        self.reply.as_ref().is_some_and(ApprovalReply::is_pending)
    }
}

impl Render for ApprovalWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let request = &self.request;
        let client = request
            .client_title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| request.client_name.clone());
        let roles = request.roles.clone();
        div()
            .id("mcp-approval")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .gap(px(14.0))
            .pt(px(crate::chrome::STRIP_HEIGHT + 12.0))
            .px(px(24.0))
            .pb(px(20.0))
            .bg(theme::bg_app())
            .text_color(theme::text_primary())
            .font_family(theme::SANS)
            .child(text(
                format!("Allow {client} to use PwrRoute?"),
                16.0,
                600.0,
                theme::text_primary(),
            ))
            .child(div().flex().flex_col().gap(px(4.0)).child(mono(
                format!(
                    "{} \u{00b7} {}",
                    request.client_name, request.client_version
                ),
                11.0,
                500.0,
                theme::text_subtle(),
            )))
            .child(crate::widgets::paragraph(
                "An MCP client on this computer is connecting. The name is what it sent, so a \
                 role limits what it may do; it does not prove who is calling. Links it asks \
                 about stay in memory. Change or revoke the role later in Settings \u{203a} \
                 Local agents.",
                11.5,
                17.0,
                theme::text_secondary(),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .children(roles.into_iter().map(|role| {
                        let id = gpui_kit::ElementId::Name(SharedString::from(format!(
                            "{APPROVE_ID_PREFIX}-{}",
                            role.to_string().to_ascii_lowercase()
                        )));
                        Button::new(id)
                            .accessibility_label(format!("Allow as {role}"))
                            .w_full()
                            .flex_col()
                            .items_start()
                            .gap(px(4.0))
                            .py(px(10.0))
                            .px(px(12.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(theme::border_subtle())
                            .bg(theme::bg_elevated())
                            .cursor_pointer()
                            .hover(|style| style.border_color(theme::border_default()))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.answer(Some(role), window, cx);
                            }))
                            .child(text(role.to_string(), 12.5, 600.0, theme::text_primary()))
                            .child(crate::widgets::paragraph(
                                role.description(),
                                10.5,
                                15.0,
                                theme::text_muted(),
                            ))
                    })),
            )
            .child(
                div().flex().justify_end().mt_auto().child(
                    Button::new(DENY_ID)
                        .accessibility_label("Deny")
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.answer(None, window, cx);
                        }))
                        .child(button("Deny", ButtonKind::Default, false)),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{Entity, TestAppContext, VisualTestContext};
    use pwrroute_mcp::Outcome;

    use super::*;
    use crate::settings::{Pane, SettingsWindow};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("pwrroute-app-mcp-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn record(
        tool: &str,
        agent: &str,
        outcome: Outcome,
        note: Option<&'static str>,
    ) -> AuditRecord {
        AuditRecord {
            tool: tool.into(),
            agent: agent.into(),
            connection: 1,
            started: SystemTime::now(),
            duration: Duration::from_millis(3),
            outcome,
            note,
        }
    }

    /// A store with two approved agents and one revoked, as the design's
    /// sessions card shows, and an audit with a refused apply.
    fn producer(dir: &Path) -> (Agents, Vec<AuditRecord>) {
        let agents = Agents::open(Some(dir)).unwrap();
        agents.allow("claude-code", Role::Opener).unwrap();
        agents.allow("pwragnt", Role::Editor).unwrap();
        agents.allow("codex-mcp-client", Role::Reader).unwrap();
        agents.revoke("codex-mcp-client").unwrap();
        let audit = vec![
            record("pwrroute_open", "claude-code", Outcome::Ok, None),
            record(
                "pwrroute_rules_apply",
                "pwragnt",
                Outcome::Failed,
                Some("refused: the change fails saved examples or tests"),
            ),
            record(
                "pwrroute_resolve",
                "codex-mcp-client",
                Outcome::Denied,
                None,
            ),
        ];
        (agents, audit)
    }

    #[test]
    fn the_model_names_sessions_roles_and_actions_without_urls() {
        let dir = temp_dir("model");
        let (agents, audit) = producer(&dir);
        let model = super::model(
            &agents,
            &audit,
            true,
            Listener::Listening,
            &Endpoint::new(dir.join("mcp.sock")),
            SystemTime::now() + Duration::from_secs(120),
        );
        assert!(model.enabled);
        assert_eq!(model.active_sessions(), 2);
        let names: Vec<&str> = model.sessions.iter().map(|s| s.name.as_ref()).collect();
        assert_eq!(names.len(), 3);
        let revoked = model
            .sessions
            .iter()
            .find(|s| s.name.as_ref() == "codex-mcp-client")
            .unwrap();
        assert!(revoked.revoked.is_some());
        assert!(revoked.detail.starts_with("revoked "));
        let live = model
            .sessions
            .iter()
            .find(|s| s.name.as_ref() == "claude-code")
            .unwrap();
        assert_eq!(live.role, RoleId("opener".into()));
        assert!(live.detail.starts_with("approved "), "{}", live.detail);
        assert!(live.detail.ends_with("0 calls"), "{}", live.detail);
        assert_eq!(live.last_used.as_ref(), "never");

        assert_eq!(model.roles.len(), 3);
        let reader = &model.roles[0];
        assert_eq!(reader.name.as_ref(), "Reader");
        let activity = reader
            .permissions
            .iter()
            .find(|p| p.tool.as_ref() == "activity")
            .unwrap();
        assert_eq!(activity.grant, Grant::Partial("hosts only".into()));
        let open = reader
            .permissions
            .iter()
            .find(|p| p.tool.as_ref() == "open")
            .unwrap();
        assert_eq!(open.grant, Grant::None);
        assert!(
            model.roles[2]
                .permissions
                .iter()
                .all(|p| p.grant == Grant::Full)
        );

        // Newest first; the note rides along, never an argument.
        assert_eq!(model.recent.len(), 3);
        assert_eq!(model.recent[0].client.as_ref(), "codex-mcp-client");
        assert_eq!(model.recent[0].status, Status::Off);
        assert_eq!(
            model.recent[1].call.as_ref(),
            "pwrroute_rules_apply \u{00b7} refused: the change fails saved examples or tests"
        );
        assert_eq!(model.recent[1].status, Status::Error);
        assert_eq!(model.recent[2].outcome.as_ref(), "ok");
        assert!(model.endpoint.ends_with("mcp.sock"));
        assert!(model.command.ends_with(" mcp"));
        assert!(
            model.connect_commands()[0].lines[0]
                .starts_with("claude mcp add --scope user pwrroute -- ")
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn relative_times_read_as_the_design_writes_them() {
        let now = SystemTime::now();
        assert_eq!(ago(now, now), "just now");
        assert_eq!(ago(now, now - Duration::from_secs(120)), "2 min ago");
        assert_eq!(ago(now, now - Duration::from_secs(3600)), "1 hour ago");
        assert_eq!(
            ago(now, now - Duration::from_secs(3 * 86_400)),
            "3 days ago"
        );
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn click(cx: &mut VisualTestContext, id: impl Into<gpui_kit::ElementId>) {
        let id = id.into();
        cx.update(|window, cx| window.click(id, cx));
    }

    fn visible(cx: &mut VisualTestContext, id: impl Into<gpui_kit::ElementId>) -> bool {
        let id = id.into();
        cx.update(|window, _| window.try_find(id).is_some_and(|found| found.visible()))
    }

    /// The Settings window over the producer's model; requests go back
    /// through the store, as the app's handler does, and the model is
    /// rebuilt from it.
    fn settings(
        cx: &mut TestAppContext,
        agents: Agents,
        audit: Vec<AuditRecord>,
        endpoint: Endpoint,
    ) -> (
        Entity<SettingsWindow>,
        &mut VisualTestContext,
        Rc<RefCell<Vec<SettingsRequest>>>,
    ) {
        let requests = Rc::new(RefCell::new(Vec::new()));
        let recorded = requests.clone();
        cx.update(|cx| {
            crate::assets::load_fonts(cx);
            crate::settings::install(cx);
        });
        let build = move |agents: &Agents| {
            let mut model = crate::settings::fixtures::model();
            model.local_agents = super::model(
                agents,
                &audit,
                true,
                Listener::Listening,
                &endpoint,
                SystemTime::now(),
            );
            model
        };
        let model = build(&agents);
        let (view, cx) = cx.add_window_view(move |_, cx| {
            SettingsWindow::new(
                model,
                Rc::new(move |request, _, cx| {
                    recorded.borrow_mut().push(request.clone());
                    match request {
                        SettingsRequest::RevokeSession(session) => {
                            agents.revoke(&session.0).unwrap();
                        }
                        SettingsRequest::SetSessionRole { session, role } => {
                            agents.set_role(&session.0, role_of(role).unwrap()).unwrap();
                        }
                        _ => {}
                    }
                    let rebuilt = build(&agents).local_agents;
                    crate::settings::update(cx, move |model| model.local_agents = rebuilt);
                }),
                cx,
            )
        });
        draw(cx);
        (view, cx, requests)
    }

    #[gpui_kit::test]
    fn the_pane_draws_the_producer_and_revokes_through_the_store(cx: &mut TestAppContext) {
        let dir = temp_dir("pane");
        let (agents, audit) = producer(&dir);
        let endpoint = Endpoint::new(dir.join("mcp.sock"));
        let (view, cx, requests) = settings(cx, agents.clone(), audit, endpoint);
        click(cx, Pane::LocalAgents.nav_id());
        // Two live sessions have a Revoke button; the revoked one does not.
        assert!(visible(cx, ("agents-revoke", 0_usize)));
        assert!(visible(cx, ("agents-revoke", 1_usize)));
        assert!(!visible(cx, ("agents-revoke", 2_usize)));
        let first = view.read_with(cx, |view, _| view.model.local_agents.sessions[0].clone());
        assert!(first.revoked.is_none());

        click(cx, ("agents-revoke", 0_usize));
        assert_eq!(
            *requests.borrow(),
            [SettingsRequest::RevokeSession(first.id.clone())]
        );
        // The store took it, and the pane was rebuilt from the store.
        let name = first.id.0.to_string();
        let stored = agents
            .agents()
            .into_iter()
            .find(|a| a.name == name)
            .unwrap();
        assert!(stored.revoked_at.is_some());
        let shown = view.read_with(cx, |view, _| {
            view.model
                .local_agents
                .sessions
                .iter()
                .find(|s| s.id == first.id)
                .map(|s| s.revoked.clone())
        });
        assert!(shown.flatten().is_some(), "the row shows the store's date");
        draw(cx);
        assert_eq!(
            view.read_with(cx, |view, _| view.model.local_agents.active_sessions()),
            1
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[gpui_kit::test]
    fn the_approval_window_answers_the_prompt_once(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::assets::load_fonts(cx);
            cx.set_global(Prompts::default());
        });
        let (request, reply, mut answer) = prompt("claude-code");
        let (view, cx) = cx.add_window_view(move |_, cx| ApprovalWindow::new(request, reply, cx));
        draw(cx);
        assert!(view.read_with(cx, |view, _| view.pending()));
        let opener = SharedString::from(format!("{APPROVE_ID_PREFIX}-opener"));
        assert!(visible(cx, opener.clone()));
        click(cx, opener);
        assert_eq!(
            answer.get(),
            Some(pwrroute_mcp::ApprovalDecision::Approve { role: Role::Opener })
        );
        assert!(!view.read_with(cx, |view, _| view.pending()));

        let (request, reply, mut answer) = prompt("codex");
        let (_view, cx) = cx.add_window_view(move |_, cx| ApprovalWindow::new(request, reply, cx));
        draw(cx);
        click(cx, DENY_ID);
        assert_eq!(answer.get(), Some(pwrroute_mcp::ApprovalDecision::Deny));
    }

    /// A prompt and the answer its reply lands on.
    fn prompt(name: &str) -> (ApprovalRequest, ApprovalReply, pwrroute_mcp::ApprovalAnswer) {
        let (reply, rx) = ApprovalReply::pair();
        (
            ApprovalRequest {
                id: format!("approval-{name}"),
                client_name: name.to_owned(),
                client_title: None,
                client_version: "1.0.0".to_owned(),
                roles: Role::ALL.to_vec(),
            },
            reply,
            rx,
        )
    }
}
