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
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, Local};
use gpui_kit::base::Button;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, BorrowAppContext as _, Bounds, Context, Div, FocusHandle, Global,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, SharedString,
    Styled as _, TitlebarOptions, Window, WindowBounds, WindowHandle, WindowKind, WindowOptions,
    div, point, px, size,
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
use crate::theme::{self, mono, text, tracked};
use crate::widgets::{ButtonKind, Tone, button, chip, key_button, key_cap_in, toned_chip};

/// The switch's state, in `<data dir>/mcp.json`. Sessions are kept in
/// `mcp-agents.json` whatever the switch says.
const PREFERENCES_FILE: &str = "mcp.json";

/// Tool calls the pane's "Recent agent actions" can show.
const RECENT_ACTIONS: usize = 20;

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
/// one lands. Tool, client, time and outcome: never arguments. A
/// `pwrroute_rules_apply` that went through also tells the main window,
/// so the Rules screen reads the file the agent just wrote.
pub(crate) struct AppAudit {
    inner: MemoryAudit,
    changed: async_channel::Sender<()>,
    rules_written: async_channel::Sender<()>,
}

impl AppAudit {
    pub(crate) fn new(
        changed: async_channel::Sender<()>,
        rules_written: async_channel::Sender<()>,
    ) -> Self {
        Self {
            inner: MemoryAudit::default(),
            changed,
            rules_written,
        }
    }
}

impl AuditSink for AppAudit {
    fn record(&self, record: AuditRecord) {
        let wrote_rules = record.tool == "pwrroute_rules_apply" && record.outcome == Outcome::Ok;
        self.inner.record(record);
        let _ = self.changed.try_send(());
        if wrote_rules {
            let _ = self.rules_written.try_send(());
        }
    }
}

/// Reloads the Rules screen after each in-process `pwrroute_rules_apply`
/// the audit reports as written, on the GPUI thread.
pub(crate) fn follow_rules_writes(rules_written: async_channel::Receiver<()>, cx: &mut App) {
    cx.spawn(async move |cx| {
        while rules_written.recv().await.is_ok() {
            cx.update(crate::app::sync_rules_windows);
        }
    })
    .detach();
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
                log::warn!("local agents: {e:#}");
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
    let (rules_tx, rules_rx) = async_channel::unbounded::<()>();
    let (prompt_tx, prompt_rx) = async_channel::unbounded::<Prompt>();
    let agents = match Agents::open(data_dir.as_deref()) {
        Ok(agents) => agents,
        Err(e) => {
            // Failing closed keeps a damaged file from forgetting a
            // revocation: no door at all until the file is looked at.
            log::warn!("local agents: {e:#}");
            let agents = Agents::open(None).expect("an in-memory store");
            let door = Door {
                data_dir: data_dir.clone(),
                agents: agents.clone(),
                audit: Arc::new(AppAudit::new(audit_tx, rules_tx)),
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
        audit: Arc::new(AppAudit::new(audit_tx, rules_tx)),
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
    // The Rules screen reloads after an agent writes rules.json.
    follow_rules_writes(rules_rx, cx);
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
    // Traces name the config in charge by its shown path, as the window
    // does: "~/.finicky.ts's default".
    let router = Arc::new(
        LiveRouter::new(wiring.handle.clone()).with_config(Arc::new(|| {
            crate::router::config_in_charge().map(|path| {
                crate::config_view::display_path(&path, std::env::home_dir().as_deref())
            })
        })),
    );
    Backends {
        resolver: router.clone(),
        router: router.clone(),
        browsers: Arc::new(LiveBrowsers::new(
            crate::browser_scan::environment(),
            || pwrroute_os::default_browser_status().ok(),
        )),
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
                    log::warn!("local agents: could not save the switch: {e}");
                }
            }
            refresh_settings(cx);
        }
        SettingsRequest::SetSessionRole { session, role } => {
            let agents = door.borrow().agents.clone();
            match role_of(role) {
                Some(role) => {
                    if let Err(e) = agents.set_role(&session.0, role) {
                        log::warn!("local agents: {e:#}");
                    }
                }
                None => log::warn!("local agents: no role named {}", role.0),
            }
            refresh_settings(cx);
        }
        SettingsRequest::RevokeSession(session) => {
            let agents = door.borrow().agents.clone();
            if let Err(e) = agents.revoke(&session.0) {
                log::warn!("local agents: {e:#}");
            }
            refresh_settings(cx);
        }
        SettingsRequest::AllowSession { session, role } => {
            let agents = door.borrow().agents.clone();
            match role_of(role) {
                Some(role) => {
                    if let Err(e) = agents.allow(&session.0, role) {
                        log::warn!("local agents: {e:#}");
                    }
                }
                None => log::warn!("local agents: no role named {}", role.0),
            }
            refresh_settings(cx);
        }
        SettingsRequest::ForgetSession(session) => {
            let agents = door.borrow().agents.clone();
            if let Err(e) = agents.forget(&session.0) {
                log::warn!("local agents: {e:#}");
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
            log::warn!("local agents: custom roles are not built yet");
        }
        _ => return false,
    }
    true
}

// ---- The approval window --------------------------------------------------
//
// Design: `PwrRoute Agent Approval` 1b-1f. The roles are a radio group with
// the least chosen; Allow and Deny decide, never a card. The window stays
// on the outcome, shrunk to it.

/// The approval window's kind. An agent connects from a terminal or an
/// editor, which is the active app then, and since macOS 14 PwrRoute can't
/// make itself active: a normal window would open behind that app, or on
/// another Space when it is full screen, and nobody would see it. A pop-up
/// panel shows over every app and on every Space (full-screen ones too) and
/// takes clicks and keys without activating PwrRoute. Elsewhere `PopUp` is a
/// window without a frame (Windows) or one the window manager ignores (X11),
/// so it stays a normal window.
#[cfg(target_os = "macos")]
const PROMPT_KIND: WindowKind = WindowKind::PopUp;
#[cfg(not(target_os = "macos"))]
const PROMPT_KIND: WindowKind = WindowKind::Normal;

/// The window's width, and its height at each stage (design 1b, 1d, 1e).
const PROMPT_WIDTH: f32 = 480.0;
const ASKING_HEIGHT: f32 = 600.0;
const ALLOWED_HEIGHT: f32 = 400.0;
const SHORT_HEIGHT: f32 = 280.0;

/// How long an allowed or denied window stays to say so.
const OUTCOME_FOR: Duration = Duration::from_secs(6);
/// The countdown turns amber below this.
const EXPIRY_WARNING: Duration = Duration::from_secs(30);
/// Keys do nothing this long after the window opens: it takes key focus
/// as it appears, and a Return meant for the terminal must not allow.
const KEY_GRACE: Duration = Duration::from_millis(800);
/// A cancel this close to the deadline is the prompt expiring; earlier, the
/// server stopped.
const EXPIRY_SLACK: Duration = Duration::from_secs(2);
const TICK: Duration = Duration::from_secs(1);

/// Element ids the tests reach for. A role card is `mcp-role-<role>`.
pub const ROLE_ID_PREFIX: &str = "mcp-role";
pub const ALLOW_ID: &str = "mcp-allow";
pub const DENY_ID: &str = "mcp-deny";
pub const DONE_ID: &str = "mcp-done";
pub const LOCAL_AGENTS_ID: &str = "mcp-local-agents";

/// The window's title: tells the panels apart (the resize finds one by
/// it) and from the tray dropdown, which is at the same level and has none.
fn prompt_title(request: &ApprovalRequest) -> String {
    format!("PwrRoute \u{2014} {}", request.client_name)
}

fn open_prompt(request: ApprovalRequest, reply: ApprovalReply, cx: &mut App) {
    // An outcome still on screen for this client gives way to its new
    // prompt, so two windows never share a title.
    let open: Vec<WindowHandle<ApprovalWindow>> = cx
        .try_global::<Prompts>()
        .map(|prompts| prompts.0.values().copied().collect())
        .unwrap_or_default();
    for handle in open {
        let _ = handle.update(cx, |this, window, _| {
            if this.request.client_name == request.client_name && this.stage != Stage::Asking {
                window.remove_window();
            }
        });
    }

    let id = request.id.clone();
    let client = request.client_name.clone();
    let title = prompt_title(&request);
    let display = crate::placement::display_for_new_window(cx);
    let bounds = Bounds::centered(display, size(px(PROMPT_WIDTH), px(ASKING_HEIGHT)), cx);
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            display_id: display,
            titlebar: Some(TitlebarOptions {
                title: Some(title.clone().into()),
                appears_transparent: true,
                traffic_light_position: Some(point(
                    px(crate::TRAFFIC_LIGHTS.0),
                    px(crate::TRAFFIC_LIGHTS.1),
                )),
            }),
            app_id: Some("pwrroute".to_owned()),
            window_min_size: Some(size(px(PROMPT_WIDTH), px(SHORT_HEIGHT))),
            kind: PROMPT_KIND,
            ..Default::default()
        },
        |window, cx| cx.new(|cx| ApprovalWindow::new(request, reply, window, cx)),
    );
    match opened {
        Ok(window) => {
            log::info!("approval window open for agent {client:?}");
            #[cfg(target_os = "macos")]
            crate::macos::keep_shown_while_inactive(&title);
            let _ = window.update(cx, |this, window, cx| window.focus(&this.focus, cx));
            // Off macOS the window is a normal one and needs the app in
            // front; on macOS the panel shows without it, and an activation
            // that went through would switch the person to PwrRoute's Space.
            if PROMPT_KIND == WindowKind::Normal {
                cx.activate(true);
            }
            if !cx.has_global::<Prompts>() {
                cx.set_global(Prompts::default());
            }
            cx.update_global::<Prompts, _>(|prompts, _| {
                prompts.0.insert(id, window);
            });
        }
        Err(e) => {
            // The reply is dropped with the window that never was: a denial.
            log::warn!("could not open the approval window: {e}");
        }
    }
}

/// The server closed the prompt: it expired, or the server stopped. The
/// server has already filed it as expired, which is not a denial.
fn close_prompt(id: &str, cx: &mut App) {
    let window = cx
        .try_global::<Prompts>()
        .and_then(|prompts| prompts.0.get(id).copied());
    if let Some(window) = window {
        let _ = window.update(cx, |this, window, cx| this.closed_by_server(window, cx));
    }
}

/// Where the window is in its one exchange.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Waiting for Allow or Deny (1b, 1c).
    Asking,
    /// Allowed with this role (1d).
    Allowed(Role),
    /// Denied (1e).
    Denied,
    /// Nobody answered in time (1f). Not a denial.
    Expired,
}

/// "Allow <client> to use PwrRoute?": one window per prompt. The cards
/// choose a role; Allow (Return) approves with it and Deny (Esc), or
/// closing the window while it asks, denies. Then it says what happened.
pub struct ApprovalWindow {
    pub request: ApprovalRequest,
    reply: Option<ApprovalReply>,
    pub focus: FocusHandle,
    title: String,
    chosen: Role,
    stage: Stage,
    opened_at: Instant,
    deadline: Instant,
    /// When an outcome closes itself.
    closes_at: Option<Instant>,
}

impl ApprovalWindow {
    pub fn new(
        request: ApprovalRequest,
        reply: ApprovalReply,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let id = request.id.clone();
        cx.on_release(move |_, cx| {
            if cx.has_global::<Prompts>() {
                cx.update_global::<Prompts, _>(|prompts, _| {
                    prompts.0.remove(&id);
                });
            }
        })
        .detach();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this
                    .update_in(cx, |this, window, cx| this.tick(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let now = Instant::now();
        Self {
            title: prompt_title(&request),
            chosen: request.roles.first().copied().unwrap_or(Role::Reader),
            stage: Stage::Asking,
            opened_at: now,
            deadline: now + request.expires_in,
            closes_at: None,
            request,
            reply: Some(reply),
            focus: cx.focus_handle(),
        }
    }

    /// The name the window calls the client by: its title when it sent
    /// one, else its name.
    fn client(&self) -> String {
        self.request
            .client_title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| self.request.client_name.clone())
    }

    fn choose(&mut self, role: Role, cx: &mut Context<'_, Self>) {
        if self.stage == Stage::Asking && self.chosen != role {
            self.chosen = role;
            cx.notify();
        }
    }

    /// Up and Down: the role before or after the chosen one, stopping at
    /// the ends.
    fn step(&mut self, forward: bool, cx: &mut Context<'_, Self>) {
        let roles = &self.request.roles;
        let Some(at) = roles.iter().position(|r| *r == self.chosen) else {
            return;
        };
        let next = if forward {
            (at + 1).min(roles.len() - 1)
        } else {
            at.saturating_sub(1)
        };
        self.choose(roles[next], cx);
    }

    fn answer(&mut self, role: Option<Role>, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(reply) = self.reply.take() else {
            return;
        };
        let (stage, height) = match role {
            Some(role) => {
                reply.approve(role);
                (Stage::Allowed(role), ALLOWED_HEIGHT)
            }
            None => {
                reply.deny();
                (Stage::Denied, SHORT_HEIGHT)
            }
        };
        self.stage = stage;
        self.closes_at = Some(Instant::now() + OUTCOME_FOR);
        self.resize(height, window, cx);
        cx.notify();
    }

    fn closed_by_server(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.stage != Stage::Asking {
            return;
        }
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left > EXPIRY_SLACK {
            // The server stopped (the switch, or quitting): nothing to
            // ask again for.
            log::info!(
                "approval window for agent {:?} closed: the server stopped",
                self.request.client_name
            );
            window.remove_window();
            return;
        }
        log::info!(
            "approval window for agent {:?}: expired unanswered",
            self.request.client_name
        );
        // Dropping the reply now changes nothing: the server has moved on.
        self.reply = None;
        self.stage = Stage::Expired;
        self.resize(SHORT_HEIGHT, window, cx);
        cx.notify();
    }

    /// Shrink to the outcome. macOS keeps a resized window's bottom edge,
    /// so there AppKit does it, keeping the top, and outside any update.
    fn resize(&self, height: f32, window: &mut Window, cx: &mut Context<'_, Self>) {
        #[cfg(target_os = "macos")]
        {
            let _ = window;
            let title = self.title.clone();
            cx.spawn(async move |_, _| {
                if !crate::macos::set_prompt_height(&title, height) {
                    log::debug!("the approval window was not resized: no open panel");
                }
            })
            .detach();
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = cx;
            window.resize(size(px(PROMPT_WIDTH), px(height)));
        }
    }

    fn tick(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.closes_at.is_some_and(|at| Instant::now() >= at) {
            window.remove_window();
            return;
        }
        if self.stage == Stage::Asking || self.closes_at.is_some() {
            cx.notify();
        }
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if modifiers.platform || modifiers.control || modifiers.alt || modifiers.function {
            return;
        }
        if self.opened_at.elapsed() < KEY_GRACE {
            return;
        }
        match (self.stage, keystroke.key.as_str()) {
            (Stage::Asking, "enter") => self.answer(Some(self.chosen), window, cx),
            (Stage::Asking, "escape") => self.answer(None, window, cx),
            (Stage::Asking, "up") => self.step(false, cx),
            (Stage::Asking, "down") => self.step(true, cx),
            (Stage::Asking, digit @ ("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")) => {
                let index = digit.parse::<usize>().unwrap_or(0).saturating_sub(1);
                match self.request.roles.get(index) {
                    Some(&role) => self.choose(role, cx),
                    None => return,
                }
            }
            (_, "enter" | "escape") => window.remove_window(),
            _ => return,
        }
        cx.stop_propagation();
    }

    /// The prompt's time left, as the footer shows it.
    fn time_left(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    /// Where the window is, for tests.
    #[cfg(test)]
    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// The role Allow would approve with, for tests.
    #[cfg(test)]
    pub fn chosen(&self) -> Role {
        self.chosen
    }

    /// Whether the prompt still waits for an answer, for tests.
    #[cfg(test)]
    pub fn pending(&self) -> bool {
        self.reply.as_ref().is_some_and(ApprovalReply::is_pending)
    }

    /// Lets keys through at once, for tests.
    #[cfg(test)]
    pub fn skip_key_grace(&mut self) {
        self.opened_at = Instant::now() - KEY_GRACE;
    }

    // ---- Drawing -------------------------------------------------------------

    fn asking(&self, cx: &mut Context<'_, Self>) -> Div {
        let roles = self
            .request
            .roles
            .iter()
            .enumerate()
            .map(|(index, &role)| role_card(index, role, role == self.chosen, cx));
        let expires = self.time_left();
        let countdown = mono(
            format!("Expires in {}", minutes_seconds(expires)),
            10.5,
            500.0,
            if expires < EXPIRY_WARNING {
                theme::warn_text()
            } else {
                theme::text_subtle()
            },
        );
        let chosen = self.chosen;
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap(px(14.0))
                    .pt(px(4.0))
                    .px(px(24.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.0))
                            .child(tracked("Local agent", 9.5, 600.0, 0.14, theme::accent()))
                            .child(text(
                                format!("Allow {} to use PwrRoute?", self.client()),
                                17.0,
                                700.0,
                                theme::text_primary(),
                            ))
                            .child(who(&self.request, Some(chip("name sent by the client")))),
                    )
                    .child(crate::widgets::paragraph(
                        "An MCP client on this computer wants to connect. PwrRoute can't confirm \
                         who is calling, so the role is a limit on what it may do. Links it asks \
                         about stay in memory.",
                        11.5,
                        17.8,
                        theme::text_secondary(),
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.0))
                            .child(tracked("Role", 9.5, 600.0, 0.14, theme::text_subtle()))
                            .child(div().flex().flex_col().gap(px(2.0)).children(roles)),
                    )
                    .child(permissions(chosen)),
            )
            .child(
                footer()
                    .child(countdown)
                    .child(div().flex_1())
                    .child(
                        Button::new(DENY_ID)
                            .accessibility_label("Deny")
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.answer(None, window, cx);
                            }))
                            .child(key_button("Deny", "esc", ButtonKind::Default)),
                    )
                    .child(
                        Button::new(ALLOW_ID)
                            .accessibility_label(format!("Allow as {chosen}"))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| {
                                let role = this.chosen;
                                this.answer(Some(role), window, cx);
                            }))
                            .child(key_button(
                                format!("Allow as {chosen}"),
                                "\u{21a9}",
                                ButtonKind::Primary,
                            )),
                    ),
            )
    }

    fn outcome(&self, cx: &mut Context<'_, Self>) -> Div {
        let client = self.client();
        let (glyph, fill, ink, title, role_chip, lead): (_, _, _, String, _, &str) =
            match self.stage {
                Stage::Allowed(role) => (
                    "\u{2713}",
                    theme::success_soft(),
                    theme::success_text(),
                    format!("{client} can use PwrRoute"),
                    Some(toned_chip(role.to_string(), Tone::Ok)),
                    "Its request goes ahead now. Change the role or revoke it any time in \
                     Settings \u{203a} Local agents.",
                ),
                Stage::Denied => (
                    "\u{2715}",
                    theme::danger_soft(),
                    theme::danger_text(),
                    format!("{client} was denied"),
                    None,
                    "Its calls are refused until it reconnects. To be asked again, reconnect \
                     PwrRoute's MCP server in the agent, or restart it.",
                ),
                Stage::Expired | Stage::Asking => (
                    "!",
                    theme::warn_soft(),
                    theme::warn_text(),
                    "This request expired".to_owned(),
                    None,
                    "",
                ),
            };
        let lead = if self.stage == Stage::Expired {
            format!(
                "{client} waited {} without an answer, so nothing was allowed or denied. \
                 Its next call to PwrRoute opens this window again.",
                spoken(self.request.expires_in)
            )
        } else {
            lead.to_owned()
        };
        let closes = self.closes_at.map(|at| {
            let left = at
                .saturating_duration_since(Instant::now())
                .as_secs_f32()
                .ceil();
            mono(
                format!("Closes in {left}s"),
                10.5,
                500.0,
                theme::text_subtle(),
            )
        });
        let done_label = if self.stage == Stage::Expired {
            "Close"
        } else {
            "Done"
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap(px(14.0))
                    .pt(px(4.0))
                    .px(px(24.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(36.0))
                            .rounded_full()
                            .bg(fill)
                            .child(text(glyph, 16.0, 600.0, ink)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.0))
                            .child(text(title, 17.0, 700.0, theme::text_primary()))
                            .child(who(&self.request, role_chip)),
                    )
                    .child(crate::widgets::paragraph(
                        lead,
                        11.5,
                        17.8,
                        theme::text_secondary(),
                    ))
                    .when_some(
                        match self.stage {
                            Stage::Allowed(role) => Some(role),
                            _ => None,
                        },
                        |body, role| body.child(permissions(role)),
                    ),
            )
            .child(
                footer()
                    .children(closes)
                    .child(div().flex_1())
                    .when(matches!(self.stage, Stage::Allowed(_)), |row| {
                        row.child(
                            Button::new(LOCAL_AGENTS_ID)
                                .accessibility_label("Open Settings, Local agents")
                                .cursor_pointer()
                                .on_click(cx.listener(|_, _, window, cx| {
                                    cx.defer(|cx| {
                                        crate::settings::open(
                                            cx,
                                            Some(crate::settings::Pane::LocalAgents),
                                        );
                                    });
                                    window.remove_window();
                                }))
                                .child(button("Local agents\u{2026}", ButtonKind::Default, false)),
                        )
                    })
                    .child(
                        Button::new(DONE_ID)
                            .accessibility_label(done_label)
                            .cursor_pointer()
                            .on_click(cx.listener(|_, _, window, _| window.remove_window()))
                            .child(key_button(done_label, "\u{21a9}", ButtonKind::Primary)),
                    ),
            )
    }
}

impl Render for ApprovalWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let body = match self.stage {
            Stage::Asking => self.asking(cx),
            _ => self.outcome(cx),
        };
        div()
            .id("mcp-approval")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.key(event, window, cx);
            }))
            .size_full()
            .flex()
            .flex_col()
            .pt(px(crate::chrome::STRIP_HEIGHT))
            .bg(theme::bg_app())
            .text_color(theme::text_primary())
            .font_family(theme::SANS)
            .child(body)
    }
}

/// The client as it named itself: name and version, mono, and a chip.
fn who(request: &ApprovalRequest, chip: Option<Div>) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(8.0))
        .child(mono(
            format!(
                "{} \u{00b7} {}",
                request.client_name, request.client_version
            ),
            11.0,
            500.0,
            theme::text_subtle(),
        ))
        .children(chip)
}

/// One role: a radio, its name, its key and what it may do. The chosen one
/// is lit; a click chooses, never approves.
fn role_card(index: usize, role: Role, chosen: bool, cx: &mut Context<'_, ApprovalWindow>) -> Div {
    let name = role.to_string();
    let id = gpui_kit::ElementId::Name(SharedString::from(format!(
        "{ROLE_ID_PREFIX}-{}",
        name.to_ascii_lowercase()
    )));
    let key = ["1", "2", "3", "4", "5", "6", "7", "8", "9"]
        .get(index)
        .copied();
    let radio = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .mt(px(1.0))
        .size(px(14.0))
        .rounded_full()
        .border_1()
        .border_color(if chosen {
            theme::accent()
        } else {
            theme::border_strong()
        })
        .bg(theme::bg_input())
        .when(chosen, |radio| {
            radio.child(div().size(px(6.0)).rounded_full().bg(theme::accent()))
        });
    let card = Button::new(id)
        .selected(chosen)
        .accessibility_label(format!("{name} role"))
        .w_full()
        .flex()
        .items_start()
        .gap(px(10.0))
        .py(px(9.0))
        .px(px(12.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(if chosen {
            theme::accent_border()
        } else {
            theme::border_subtle()
        })
        .bg(if chosen {
            theme::bg_row_active()
        } else {
            theme::bg_elevated()
        })
        .cursor_pointer()
        .when(!chosen, |card| {
            card.hover(|style| {
                style
                    .border_color(theme::border_default())
                    .bg(theme::bg_hover())
            })
        })
        .on_click(cx.listener(move |this, _, _, cx| this.choose(role, cx)))
        .child(radio)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(text(
                            name.clone(),
                            12.5,
                            600.0,
                            if chosen {
                                theme::accent_bright()
                            } else {
                                theme::text_primary()
                            },
                        ))
                        .child(div().flex_1())
                        .children(key.map(|key| {
                            key_cap_in(key, 15.0, 9.0, theme::border_subtle(), theme::text_subtle())
                        })),
                )
                .child(crate::widgets::paragraph(
                    role.description(),
                    10.5,
                    15.2,
                    theme::text_muted(),
                )),
        );
    // The focus ring: an accent edge 1px outside the chosen card.
    div()
        .p(px(1.0))
        .border_2()
        .rounded(px(11.0))
        .border_color(if chosen {
            theme::accent()
        } else {
            theme::transparent()
        })
        .child(card)
}

/// What `role` may do, in two columns: what every role has on the left,
/// what the larger roles add on the right.
fn permissions(role: Role) -> Div {
    let (every, more) = Permission::ALL.split_at(4);
    let column = |list: &[Permission]| {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .children(list.iter().map(|&permission| {
                let allowed = role.allows(permission);
                let (glyph, fill, mark, color) = if allowed {
                    (
                        "\u{2713}",
                        theme::success_soft(),
                        theme::success_text(),
                        theme::text_secondary(),
                    )
                } else {
                    (
                        "\u{2014}",
                        theme::bg_elevated(),
                        theme::text_subtle(),
                        theme::text_subtle(),
                    )
                };
                div()
                    .flex()
                    .items_start()
                    .gap(px(7.0))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .mt(px(1.0))
                            .size(px(12.0))
                            .rounded(px(3.0))
                            .bg(fill)
                            .child(text(glyph, 8.0, 400.0, mark)),
                    )
                    .child(div().flex_1().min_w_0().child(crate::widgets::paragraph(
                        permission.label(),
                        10.5,
                        14.2,
                        color,
                    )))
            }))
    };
    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .py(px(10.0))
        .px(px(12.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(theme::border_subtle())
        .bg(theme::bg_panel())
        .child(text(
            format!("{role} can"),
            11.0,
            600.0,
            theme::text_secondary(),
        ))
        .child(
            div()
                .flex()
                .gap(px(14.0))
                .child(column(every))
                .child(column(more)),
        )
}

fn footer() -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .pt(px(14.0))
        .px(px(24.0))
        .pb(px(18.0))
}

/// `4:58`.
fn minutes_seconds(left: Duration) -> String {
    let secs = left.as_secs_f32().ceil() as u64;
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// "five minutes", "90 seconds": the prompt's lifetime in words.
fn spoken(span: Duration) -> String {
    let secs = span.as_secs();
    match (secs / 60, secs % 60) {
        (1, 0) => "a minute".to_owned(),
        (5, 0) => "five minutes".to_owned(),
        (minutes, 0) => format!("{minutes} minutes"),
        _ => format!("{secs} seconds"),
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
                        SettingsRequest::AllowSession { session, role } => {
                            agents.allow(&session.0, role_of(role).unwrap()).unwrap();
                        }
                        SettingsRequest::ForgetSession(session) => {
                            agents.forget(&session.0).unwrap();
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

    /// The row drawn for the agent named `name`: its index and its model.
    /// Rows sort by `approved_at`, which has one-second resolution, so a
    /// test never assumes the producer's order.
    fn row(
        view: &Entity<SettingsWindow>,
        cx: &mut VisualTestContext,
        name: &str,
    ) -> (usize, crate::settings::model::AgentSession) {
        view.read_with(cx, |view, _| {
            view.model
                .local_agents
                .sessions
                .iter()
                .enumerate()
                .find(|(_, s)| s.name.as_ref() == name)
                .map(|(i, s)| (i, s.clone()))
                .unwrap_or_else(|| panic!("no row named {name}"))
        })
    }

    #[gpui_kit::test]
    fn the_pane_draws_the_producer_and_revokes_through_the_store(cx: &mut TestAppContext) {
        let dir = temp_dir("pane");
        let (agents, audit) = producer(&dir);
        let endpoint = Endpoint::new(dir.join("mcp.sock"));
        let (view, cx, requests) = settings(cx, agents.clone(), audit, endpoint);
        click(cx, Pane::LocalAgents.nav_id());
        // The live sessions have a Revoke button; the revoked one offers
        // Allow again and Forget instead.
        let (first_at, first) = row(&view, cx, "claude-code");
        let (second_at, _) = row(&view, cx, "pwragnt");
        let (revoked_at, _) = row(&view, cx, "codex-mcp-client");
        assert!(visible(cx, ("agents-revoke", first_at)));
        assert!(visible(cx, ("agents-revoke", second_at)));
        assert!(!visible(cx, ("agents-revoke", revoked_at)));
        assert!(visible(cx, ("agents-allow", revoked_at)));
        assert!(visible(cx, ("agents-forget", revoked_at)));
        assert!(first.revoked.is_none());

        // Revoke asks once more; Keep leaves the row alone.
        click(cx, ("agents-revoke", first_at));
        assert_eq!(
            view.read_with(cx, |view, _| view.agents.confirming()),
            Some(first.id.clone())
        );
        assert!(requests.borrow().is_empty(), "nothing until confirmed");
        assert!(visible(cx, ("agents-revoke-confirm", first_at)));
        click(cx, ("agents-revoke-cancel", first_at));
        assert_eq!(view.read_with(cx, |view, _| view.agents.confirming()), None);
        assert!(requests.borrow().is_empty());
        assert!(visible(cx, ("agents-revoke", first_at)));

        click(cx, ("agents-revoke", first_at));
        click(cx, ("agents-revoke-confirm", first_at));
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
        let (first_at, _) = row(&view, cx, "claude-code");
        assert!(visible(cx, ("agents-allow", first_at)));
        fs::remove_dir_all(dir).unwrap();
    }

    #[gpui_kit::test]
    fn the_pane_allows_a_revoked_agent_again_with_its_last_role(cx: &mut TestAppContext) {
        let dir = temp_dir("allow");
        let (agents, audit) = producer(&dir);
        let endpoint = Endpoint::new(dir.join("mcp.sock"));
        let (view, cx, requests) = settings(cx, agents.clone(), audit, endpoint);
        click(cx, Pane::LocalAgents.nav_id());
        let (at, revoked) = row(&view, cx, "codex-mcp-client");
        assert!(revoked.revoked.is_some());
        assert_eq!(revoked.role, RoleId("reader".into()));

        click(cx, ("agents-allow", at));
        assert_eq!(
            *requests.borrow(),
            [SettingsRequest::AllowSession {
                session: revoked.id.clone(),
                role: RoleId("reader".into()),
            }]
        );
        // The store took it: the agent is allowed again, as a Reader.
        let stored = agents
            .agents()
            .into_iter()
            .find(|a| a.name == "codex-mcp-client")
            .unwrap();
        assert!(stored.revoked_at.is_none());
        assert_eq!(stored.role, Role::Reader);
        draw(cx);
        let (at, shown) = row(&view, cx, "codex-mcp-client");
        assert!(
            shown.revoked.is_none(),
            "the row was rebuilt from the store"
        );
        assert!(shown.detail.starts_with("approved "), "{}", shown.detail);
        assert!(visible(cx, ("agents-revoke", at)));
        assert!(!visible(cx, ("agents-allow", at)));
        assert_eq!(
            view.read_with(cx, |view, _| view.model.local_agents.active_sessions()),
            3
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[gpui_kit::test]
    fn the_pane_forgets_a_revoked_agent_through_the_store(cx: &mut TestAppContext) {
        let dir = temp_dir("forget");
        let (agents, audit) = producer(&dir);
        let endpoint = Endpoint::new(dir.join("mcp.sock"));
        let (view, cx, requests) = settings(cx, agents.clone(), audit, endpoint);
        click(cx, Pane::LocalAgents.nav_id());
        let (at, revoked) = row(&view, cx, "codex-mcp-client");

        click(cx, ("agents-forget", at));
        assert_eq!(
            *requests.borrow(),
            [SettingsRequest::ForgetSession(revoked.id.clone())]
        );
        assert!(
            agents.agents().iter().all(|a| a.name != "codex-mcp-client"),
            "the store dropped it"
        );
        draw(cx);
        let names: Vec<String> = view.read_with(cx, |view, _| {
            view.model
                .local_agents
                .sessions
                .iter()
                .map(|s| s.name.to_string())
                .collect()
        });
        assert_eq!(names.len(), 2);
        assert!(!names.iter().any(|n| n == "codex-mcp-client"));
        assert_eq!(
            view.read_with(cx, |view, _| view.model.local_agents.active_sessions()),
            2
        );
        fs::remove_dir_all(dir).unwrap();
    }

    /// An approval window over a fresh prompt, drawn.
    fn approval_window<'a>(
        cx: &'a mut TestAppContext,
        name: &str,
        expires_in: Duration,
    ) -> (
        Entity<ApprovalWindow>,
        &'a mut VisualTestContext,
        pwrroute_mcp::ApprovalAnswer,
    ) {
        cx.update(|cx| {
            crate::assets::load_fonts(cx);
            cx.set_global(Prompts::default());
        });
        let (mut request, reply, answer) = prompt(name);
        request.expires_in = expires_in;
        let (view, cx) =
            cx.add_window_view(move |window, cx| ApprovalWindow::new(request, reply, window, cx));
        draw(cx);
        (view, cx, answer)
    }

    fn role_id(role: &str) -> SharedString {
        SharedString::from(format!("{ROLE_ID_PREFIX}-{role}"))
    }

    #[gpui_kit::test]
    fn a_role_card_chooses_and_allow_decides(cx: &mut TestAppContext) {
        let (view, cx, mut answer) = approval_window(cx, "claude-code", Duration::from_secs(300));
        // Reader, the least role, is chosen at open.
        assert_eq!(view.read_with(cx, |view, _| view.chosen()), Role::Reader);
        assert!(visible(cx, ALLOW_ID));

        // A card only chooses.
        click(cx, role_id("editor"));
        draw(cx);
        assert_eq!(view.read_with(cx, |view, _| view.chosen()), Role::Editor);
        assert!(view.read_with(cx, |view, _| view.pending()));
        assert_eq!(answer.get(), None);

        // Allow approves with the chosen role, once, and the window stays
        // to say so.
        click(cx, ALLOW_ID);
        draw(cx);
        assert_eq!(
            answer.get(),
            Some(pwrroute_mcp::ApprovalDecision::Approve { role: Role::Editor })
        );
        assert_eq!(
            view.read_with(cx, |view, _| view.stage()),
            Stage::Allowed(Role::Editor)
        );
        assert!(visible(cx, DONE_ID));
        assert!(visible(cx, LOCAL_AGENTS_ID));
        assert!(!visible(cx, ALLOW_ID));
    }

    #[gpui_kit::test]
    fn deny_denies_and_says_so(cx: &mut TestAppContext) {
        let (view, cx, mut answer) = approval_window(cx, "codex", Duration::from_secs(300));
        click(cx, DENY_ID);
        draw(cx);
        assert_eq!(answer.get(), Some(pwrroute_mcp::ApprovalDecision::Deny));
        assert_eq!(view.read_with(cx, |view, _| view.stage()), Stage::Denied);
        assert!(visible(cx, DONE_ID));
        assert!(!visible(cx, LOCAL_AGENTS_ID));
    }

    #[gpui_kit::test]
    fn keys_choose_and_decide_after_the_grace(cx: &mut TestAppContext) {
        let (view, cx, mut answer) = approval_window(cx, "claude-code", Duration::from_secs(300));
        view.update_in(cx, |view, window, cx| window.focus(&view.focus, cx));

        // A Return meant for the terminal the window opened over.
        cx.simulate_keystrokes("enter");
        assert!(view.read_with(cx, |view, _| view.pending()));

        view.update(cx, |view, _| view.skip_key_grace());
        cx.simulate_keystrokes("2");
        assert_eq!(view.read_with(cx, |view, _| view.chosen()), Role::Opener);
        cx.simulate_keystrokes("down down up");
        assert_eq!(view.read_with(cx, |view, _| view.chosen()), Role::Opener);
        cx.simulate_keystrokes("3 enter");
        assert_eq!(
            answer.get(),
            Some(pwrroute_mcp::ApprovalDecision::Approve { role: Role::Editor })
        );

        let (view, cx, mut answer) = approval_window(cx, "codex", Duration::from_secs(300));
        view.update_in(cx, |view, window, cx| {
            window.focus(&view.focus, cx);
            view.skip_key_grace();
        });
        cx.simulate_keystrokes("escape");
        assert_eq!(answer.get(), Some(pwrroute_mcp::ApprovalDecision::Deny));
        assert_eq!(view.read_with(cx, |view, _| view.stage()), Stage::Denied);
    }

    #[gpui_kit::test]
    fn an_expired_prompt_says_so_and_is_not_a_denial(cx: &mut TestAppContext) {
        let (view, cx, mut answer) = approval_window(cx, "claude-code", Duration::ZERO);
        view.update_in(cx, |view, window, cx| view.closed_by_server(window, cx));
        draw(cx);
        assert_eq!(view.read_with(cx, |view, _| view.stage()), Stage::Expired);
        assert!(!view.read_with(cx, |view, _| view.pending()));
        // The reply was dropped after the server moved on: no decision.
        assert_eq!(answer.get(), None);
        assert!(visible(cx, DONE_ID));
    }

    #[test]
    fn the_countdown_and_lifetime_read_as_time() {
        assert_eq!(minutes_seconds(Duration::from_secs(300)), "5:00");
        assert_eq!(minutes_seconds(Duration::from_millis(298_200)), "4:59");
        assert_eq!(minutes_seconds(Duration::from_secs(24)), "0:24");
        assert_eq!(minutes_seconds(Duration::ZERO), "0:00");
        assert_eq!(spoken(Duration::from_secs(300)), "five minutes");
        assert_eq!(spoken(Duration::from_secs(120)), "2 minutes");
        assert_eq!(spoken(Duration::from_secs(90)), "90 seconds");
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
                expires_in: Duration::from_secs(300),
            },
            reply,
            rx,
        )
    }
}
