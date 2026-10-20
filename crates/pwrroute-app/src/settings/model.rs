//! What each Settings pane shows, as plain data.
//!
//! These are view-models, not the domain: the MCP server, config discovery
//! and the browser catalog each fill one later, in their own words, without
//! the window depending on their crates. Strings arrive ready to draw ("2 min
//! ago", "~/.finicky.ts"); formatting is the producer's job.
//!
//! What the user does in the window comes back out as a [`SettingsRequest`].

use gpui_kit::SharedString;

/// Everything the Settings window draws.
#[derive(Clone, Debug)]
pub struct SettingsModel {
    pub local_agents: LocalAgents,
    pub configuration: Configuration,
    pub browsers: Browsers,
}

// ---- Local agents ---------------------------------------------------------

/// The MCP door: the listener, its sessions and the roles they hold.
#[derive(Clone, Debug)]
pub struct LocalAgents {
    /// The "Enable local-agent access" switch.
    pub enabled: bool,
    pub listener: Listener,
    /// The loopback URL agents connect to, e.g. `http://127.0.0.1:51731/mcp`.
    pub endpoint: SharedString,
    pub sessions: Vec<AgentSession>,
    pub roles: Vec<AgentRole>,
    /// The role card drawn selected.
    pub selected_role: Option<RoleId>,
    /// Newest first.
    pub recent: Vec<AgentAction>,
}

impl LocalAgents {
    /// Sessions that are not revoked: the nav count and the "N active" chip.
    pub fn active_sessions(&self) -> usize {
        self.sessions
            .iter()
            .filter(|session| session.revoked.is_none())
            .count()
    }

    pub fn role(&self, id: &RoleId) -> Option<&AgentRole> {
        self.roles.iter().find(|role| &role.id == id)
    }

    /// The commands the "Connect an agent" card offers, for this endpoint.
    pub fn connect_commands(&self) -> [ConnectCommand; 2] {
        let url = &self.endpoint;
        [
            ConnectCommand {
                client: "Claude Code".into(),
                lines: vec![
                    format!("claude mcp add --scope user --transport http pwrroute {url}").into(),
                    "claude mcp login pwrroute".into(),
                ],
            },
            ConnectCommand {
                client: "Codex CLI".into(),
                lines: vec![
                    format!("codex mcp add pwrroute --url {url} --oauth-client-registration dcr")
                        .into(),
                ],
            },
        ]
    }
}

/// The MCP listener's state, for the card header's chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Listener {
    Listening,
    Stopped,
    /// It could not bind or serve; the text says why.
    #[allow(
        dead_code,
        reason = "the MCP server reports it; the fixtures never fail"
    )]
    Failed(SharedString),
}

/// A command a reader copies into a terminal.
#[derive(Clone, Debug)]
pub struct ConnectCommand {
    pub client: SharedString,
    /// One line per shell command.
    pub lines: Vec<SharedString>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionId(pub SharedString);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RoleId(pub SharedString);

/// One approved agent.
#[derive(Clone, Debug)]
pub struct AgentSession {
    pub id: SessionId,
    /// "Claude Code · MacBook Pro".
    pub name: SharedString,
    /// "approved 2026-10-02 · 14 calls today".
    pub detail: SharedString,
    pub role: RoleId,
    /// "2 min ago".
    pub last_used: SharedString,
    /// When it was revoked ("2026-10-11"), or `None` while it is live.
    pub revoked: Option<SharedString>,
}

/// A set of tool permissions a session can hold.
#[derive(Clone, Debug)]
pub struct AgentRole {
    pub id: RoleId,
    pub name: SharedString,
    pub built_in: bool,
    pub summary: SharedString,
    pub permissions: Vec<Permission>,
}

/// One tool and how much of it a role gets.
#[derive(Clone, Debug)]
pub struct Permission {
    /// "resolve", "rules_apply".
    pub tool: SharedString,
    pub grant: Grant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Grant {
    Full,
    /// Allowed with a limit, which the note names ("hosts only").
    Partial(SharedString),
    None,
}

/// One tool call an agent made.
#[derive(Clone, Debug)]
pub struct AgentAction {
    /// "09:40".
    pub time: SharedString,
    pub client: SharedString,
    /// "pwrroute_open · https://…".
    pub call: SharedString,
    pub outcome: SharedString,
    pub status: Status,
}

/// The `.st` dot's color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    #[allow(
        dead_code,
        reason = "for producers; the design's fixtures draw no warning"
    )]
    Warning,
    Error,
    Off,
}

// ---- Configuration --------------------------------------------------------

/// Where the router reads its rules from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RulesSource {
    /// `rules.json`, edited in the Rules view.
    Visual,
    /// A Finicky v4 `.js` or `.ts` file.
    Finicky,
}

#[derive(Clone, Debug)]
pub struct Configuration {
    pub active: RulesSource,
    /// "rules.json · 3 rewrites · 7 handlers".
    pub visual_summary: SharedString,
    /// The Finicky config discovery found, if any.
    pub finicky: Option<FinickyConfig>,
    /// What the importer offers from that config.
    pub import: Option<ImportPlan>,
    pub expand_short_links: bool,
    pub shorteners: Vec<SharedString>,
    /// Where `rules.json` lives, as the reader would type it.
    pub rules_path: SharedString,
}

#[derive(Clone, Debug)]
pub struct FinickyConfig {
    /// "~/.finicky.ts".
    pub path: SharedString,
    /// "~/.finicky.ts · 2 rewrites · 6 handlers".
    pub summary: SharedString,
    /// How it was found: "Found in the home folder. …".
    pub found: SharedString,
}

/// The importer's verdict on each rule of a Finicky config.
#[derive(Clone, Debug)]
pub struct ImportPlan {
    pub candidates: Vec<ImportCandidate>,
}

impl ImportPlan {
    /// Candidates that will import as things stand.
    pub fn checked(&self) -> usize {
        self.candidates
            .iter()
            .filter(|candidate| candidate.importable && candidate.checked)
            .count()
    }

    /// Candidates that cannot become visual rules.
    pub fn stays_js(&self) -> usize {
        self.candidates
            .iter()
            .filter(|candidate| !candidate.importable)
            .count()
    }
}

#[derive(Clone, Debug)]
pub struct ImportCandidate {
    /// The rule, as a run of words and tags.
    pub summary: Vec<Piece>,
    /// Whether visual rules can hold it. A function matcher usually cannot.
    pub importable: bool,
    /// Whether the reader left it checked. Ignored when not importable.
    pub checked: bool,
}

/// One run of an import row's summary.
#[derive(Clone, Debug)]
pub enum Piece {
    /// Plain words: "rewrite ·", "→ Firefox".
    Text(SharedString),
    /// A pattern in a `.tag`.
    Tag(SharedString),
    /// A rewrite's action in an accent `.tag`.
    Action(SharedString),
    /// A function matcher's source, in mono, after an `fn` tag.
    Source(SharedString),
    /// A dim aside: "proved equal to".
    Aside(SharedString),
    /// Source quoted in an aside, in mono.
    Code(SharedString),
}

// ---- Browsers -------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Browsers {
    pub default: DefaultBrowser,
    pub detected: Vec<BrowserApp>,
    /// Apps added by bundle id or path.
    pub other_apps: Vec<BrowserApp>,
}

/// Whether the OS hands PwrRoute its links.
#[derive(Clone, Debug)]
pub struct DefaultBrowser {
    pub is_default: bool,
    /// "Confirmed 2026-10-13 09:02." once the OS has said so.
    pub confirmed: Option<SharedString>,
}

/// One app PwrRoute can open links in.
#[derive(Clone, Debug)]
pub struct BrowserApp {
    /// The bundle id, ProgID or desktop file id: stable across scans.
    pub id: SharedString,
    pub name: SharedString,
    /// Two letters for the `.bi` badge.
    pub badge: SharedString,
    /// "com.google.Chrome · /Applications/Google Chrome.app".
    pub location: SharedString,
    pub profiles: Vec<SharedString>,
    /// Shown instead of profiles when there are none: "no profiles".
    pub note: Option<SharedString>,
    pub usage: Usage,
    /// Off hides it from pickers; rules that name it still resolve.
    pub visible: bool,
}

/// How rules use an app, for its status column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Usage {
    /// Named by `count` rules; `all_off` when every one is switched off.
    Rules {
        count: usize,
        all_off: bool,
    },
    Unused,
    NotInstalled,
}

// ---- Requests -------------------------------------------------------------

/// What the window asks of the rest of the app.
///
/// The window updates its own model where the answer is obvious (a switch
/// flips), and sends every request to the handler given to it. Nothing in
/// the window acts on the OS itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsRequest {
    SetAgentAccess(bool),
    SetSessionRole {
        session: SessionId,
        role: RoleId,
    },
    RevokeSession(SessionId),
    NewCustomRole,
    OpenActivity,
    SetRulesSource(RulesSource),
    ChooseConfigFile,
    /// Import the checked candidates, by index into the plan.
    Import {
        candidates: Vec<usize>,
        keep_config: bool,
    },
    SetExpandShortLinks(bool),
    AddShortener,
    RevealRules,
    OpenDataFolder,
    ReloadRules,
    /// The explicit "make PwrRoute the default browser" action.
    RegisterDefaultBrowser,
    /// Open the OS's own default-apps settings.
    OpenDefaultAppsSettings,
    RescanBrowsers,
    SetAppVisible {
        app: SharedString,
        visible: bool,
    },
    AddApp,
}
