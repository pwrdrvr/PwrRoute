//! What each Settings pane shows, as plain data.
//!
//! These are view-models, not the domain: the MCP server, config discovery
//! and the browser catalog each fill one later, in their own words, without
//! the window depending on their crates. Strings arrive ready to draw ("2 min
//! ago", "~/.finicky.ts"); formatting is the producer's job.
//!
//! What the user does in the window comes back out as a [`SettingsRequest`].

use gpui_kit::SharedString;

use crate::build_info::BuildInfo;
use crate::widgets::AppBadge;

pub use crate::default_browser::DefaultState;
pub use crate::settings_store::JournalLevel;
pub use crate::update_channel::Slot;

/// Everything the Settings window draws.
#[derive(Clone, Debug)]
pub struct SettingsModel {
    pub local_agents: LocalAgents,
    pub configuration: Configuration,
    pub sync: Sync,
    pub privacy: Privacy,
    pub browsers: Browsers,
    pub updates: Updates,
    pub experimental: Experimental,
    pub diagnostics: Diagnostics,
    pub about: About,
}

// ---- Local agents ---------------------------------------------------------

/// The MCP door: the listener, its sessions and the roles they hold.
#[derive(Clone, Debug)]
pub struct LocalAgents {
    /// The "Enable local-agent access" switch.
    pub enabled: bool,
    pub listener: Listener,
    /// Where the app listens: the per-user socket (a named pipe on
    /// Windows) that `pwrroute mcp` connects to.
    pub endpoint: SharedString,
    /// The command an MCP client launches: `pwrroute mcp`, with the
    /// binary's full path when it is not on the PATH.
    pub command: SharedString,
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

    /// The commands the "Connect an agent" card offers: each registers
    /// `pwrroute mcp` as a stdio MCP server.
    pub fn connect_commands(&self) -> [ConnectCommand; 2] {
        let command = &self.command;
        [
            ConnectCommand {
                client: "Claude Code".into(),
                lines: vec![format!("claude mcp add --scope user pwrroute -- {command}").into()],
            },
            ConnectCommand {
                client: "Codex CLI".into(),
                lines: vec![format!("codex mcp add pwrroute -- {command}").into()],
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
    /// The tiles pick the source. Off, they only show which one is in
    /// charge: the router follows the files on disk and cannot be told
    /// otherwise yet.
    pub can_switch: bool,
    /// "rules.json · 3 rewrites · 7 handlers".
    pub visual_summary: SharedString,
    /// The Finicky config discovery found, if any.
    pub finicky: Option<FinickyConfig>,
    /// What the importer offers from that config.
    pub import: Option<ImportPlan>,
    /// The hosts whose links are followed to where they land before the
    /// rules run: the engine's built-in list, or the config's own
    /// `urlShorteners` when it sets one. Empty when the config turned
    /// expansion off.
    pub shorteners: Vec<SharedString>,
    /// Where `rules.json` lives, as the reader would type it.
    pub rules_path: SharedString,
    /// How the file stands: "Loaded · edited 5 min ago", "No rules.json
    /// yet", or why it did not load.
    pub rules_status: SharedString,
    /// Who takes the links nothing claims, while a Finicky config is in
    /// charge (Overview and Setup 5d). `None` while visual rules are:
    /// `rules.json` is then the whole config and its default takes them.
    pub everything_else: Option<EverythingElse>,
}

/// Whose default browser takes the links no handler or rule claims:
/// `rules.json`'s `precedence`, in the reader's words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultOwner {
    /// The config's own `defaultBrowser`, after its handlers and then the
    /// visual rules: Finicky's order, and `precedence`'s absence.
    Config,
    /// `rules.json`'s default, with the visual rules first, then the
    /// config's handlers.
    PwrRoute,
}

/// Settings › Configuration's Everything else field: the segmented control,
/// what each answer means, and `rules.json`'s default browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EverythingElse {
    pub owner: DefaultOwner,
    /// The config's answer: "~/.finicky.ts's default · Safari", or the
    /// first part alone when its default is a function or `null`.
    pub config_label: SharedString,
    /// What the config's answer means, under the control.
    pub config_detail: SharedString,
    /// What PwrRoute's answer means: "Your rules run first, then
    /// ~/.finicky.ts's handlers. Links nothing claims go here instead of
    /// Safari."
    pub pwrroute_detail: SharedString,
    /// What PwrRoute's default changes, from the config's description
    /// ("Its defaultBrowser function will no longer run."): shown only
    /// while PwrRoute's default is chosen.
    pub warnings: Vec<SharedString>,
    /// What the browser select offers: the scan less hidden apps, and the
    /// app `rules.json` names when the scan did not find it.
    pub browsers: Vec<DefaultChoice>,
    /// `rules.json`'s `defaultBrowser`, as an index into `browsers`;
    /// `None` when it names none.
    pub browser: Option<usize>,
    /// `rules.json`'s `defaultProfile`; `None` is the browser's own.
    pub profile: Option<SharedString>,
    /// Why the last change was not saved, or why none can be.
    pub problem: Option<SharedString>,
}

impl EverythingElse {
    /// The chosen browser, when there is one.
    pub fn chosen(&self) -> Option<&DefaultChoice> {
        self.browser.and_then(|index| self.browsers.get(index))
    }
}

/// One browser the Everything else select offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultChoice {
    /// What `rules.json`'s `defaultBrowser` gets: the bundle id on macOS,
    /// the name elsewhere, as first run writes it.
    pub value: SharedString,
    /// The name the reader knows, never a bundle id when one is known.
    pub name: SharedString,
    /// The `.bi` badge: the app's icon, or two letters.
    pub badge: AppBadge,
    /// Its profiles by the names the browser shows; empty: no profile
    /// select.
    pub profiles: Vec<SharedString>,
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
    /// Candidates that import exactly.
    pub fn importable(&self) -> usize {
        self.candidates
            .iter()
            .filter(|candidate| candidate.importable)
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

// ---- Privacy --------------------------------------------------------------

/// Settings › Privacy: what the router keeps of each link, where, and for
/// how long. The numbers and the level are the settings as saved; the
/// sentences are the router's own, rendered verbatim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Privacy {
    /// "Keep the last N links".
    pub keep_count: usize,
    /// "For at most H hours".
    pub keep_hours: u64,
    pub journal: JournalLevel,
    /// How many days of journal files are kept.
    pub journal_days: u32,
    /// `RouterHandle::retention().description`: where links are kept and
    /// for how long, as things stand. The `.disc` under the switch and the
    /// Activity footer show it verbatim.
    pub description: SharedString,
    /// The same sentence as it would read with the journal at full URLs:
    /// what the confirm sheet repeats before the switch moves.
    pub full_description: SharedString,
    /// The journal's files, as the reader would type the path:
    /// "~/Library/Application Support/PwrRoute/journal/activity-YYYY-MM-DD.jsonl".
    pub journal_path: SharedString,
    /// What is on disk now, or `None` while no journal file exists.
    pub journal_files: Option<JournalFiles>,
    /// Why the journal's last write failed, when it is on and did.
    pub journal_error: Option<SharedString>,
    /// The rules that keep less than full, for the confirm sheet: "Rule 9
    /// keeps host only and rule 10 keeps nothing". `None` when none.
    pub rules_keep_less: Option<SharedString>,
    /// The data directory exists: settings can be saved and the journal
    /// can be turned on.
    pub can_save: bool,
}

/// The journal's files on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalFiles {
    /// "1.2 MB · 8,114 lines · 6 days".
    pub summary: SharedString,
    /// The day of the oldest file: "2026-10-14".
    pub since: SharedString,
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultBrowser {
    pub state: DefaultState,
    /// Replaces the card's help line while there is more to say: "Waiting
    /// for macOS…" after a request, "Google Chrome is still the default."
    /// after a refusal, "Confirmed 2026-10-13 09:02." once it is done.
    pub note: Option<SharedString>,
}

/// One app PwrRoute can open links in.
#[derive(Clone, Debug)]
pub struct BrowserApp {
    /// The bundle id, ProgID or desktop file id: stable across scans.
    pub id: SharedString,
    pub name: SharedString,
    /// The `.bi` badge: the app's icon, or two letters.
    pub badge: AppBadge,
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

// ---- Updates --------------------------------------------------------------

/// Settings › Updates: this copy, the release channel it follows, and
/// what the updater last said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Updates {
    /// "0.9.0".
    pub version: SharedString,
    /// "Stable · Latest": the release channel slot this copy follows.
    pub channel: SharedString,
    /// Where this copy came from: "Installed by Velopack", "A development
    /// build", "Installed from a package".
    pub install: SharedString,
    pub state: UpdateState,
    /// "Today 08:13", or `None` while no check has finished.
    pub last_checked: Option<SharedString>,
    /// The release channel's four slots, in [`Slot::ALL`]'s order.
    pub slots: Vec<SlotTile>,
    /// The slot follows the installed build: nothing was picked yet.
    pub inferred: bool,
}

/// One tile of the release channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotTile {
    pub slot: Slot,
    /// "v0.1.0": the slot's release, when one is known.
    pub release: Option<SharedString>,
    /// What the slot is, or why it shows no release.
    pub note: SharedString,
    pub selected: bool,
    /// The slot's release is this copy's version.
    pub installed: bool,
}

/// What the updater last said, for the state row and its chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateState {
    /// No check has run since launch.
    NeverChecked,
    Checking,
    /// The checked version is the newest on the channel.
    UpToDate,
    /// `version` is downloaded; a restart installs it.
    Ready(SharedString),
    /// `version`, older than this copy, is downloaded for the slot picked;
    /// Restart to switch installs it.
    SwitchReady(SharedString),
    /// The slot has no release published yet.
    NothingPublished,
    /// Why the last check failed; routing is unaffected.
    Failed(SharedString),
    /// This copy cannot update itself, and the text says who does.
    NotUpdatable(SharedString),
}

// ---- Experimental ---------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Experimental {
    /// macOS only; absent on platforms without this experiment.
    pub yield_browser_focus: Option<bool>,
}

// ---- Diagnostics ----------------------------------------------------------

/// Settings › Diagnostics: where things are and whether they run. Paths
/// and states, never a link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostics {
    /// "0.9.0 · release · arm64".
    pub version: SharedString,
    /// "macOS 15.6 (arm64)".
    pub os: SharedString,
    /// The data directory, as the reader would type it.
    pub data_dir: SharedString,
    /// The directory exists on disk, so it can be opened.
    pub data_dir_exists: bool,
    /// The router's socket and whether this process listens on it.
    pub router: SharedString,
    pub router_listening: bool,
    /// What routes links right now: the config in charge, `rules.json`, or
    /// why everything goes to the fallback.
    pub routing: SharedString,
    /// `--dry-run`: links are resolved and recorded, never opened.
    pub dry_run: bool,
    /// The MCP endpoint and whether it listens.
    pub agents: SharedString,
    /// Where the diagnostic log is, or why there is none.
    pub log: SharedString,
    /// There is a log file, so Help › Logs has something to show.
    pub log_exists: bool,
    /// Which build this is: the report's first lines.
    pub build: BuildInfo,
    /// macOS only; absent where cooperative focus yielding is unavailable.
    pub yield_browser_focus: Option<bool>,
}

impl Diagnostics {
    /// The report as plain lines, for the Copy button.
    pub fn report(&self) -> String {
        let dry_run = if self.dry_run {
            "\nDry run: links are resolved and recorded, not opened"
        } else {
            ""
        };
        let experimental = self
            .yield_browser_focus
            .map_or_else(String::new, |enabled| {
                format!(
                    "Experimental yield browser focus: {}\n",
                    if enabled { "on" } else { "off" }
                )
            });
        format!(
            "{}OS: {}\nData folder: {}\nRouter: {}\nRouting: {}{dry_run}\nLocal agents: {}\nLog: {}\n{experimental}",
            self.build.report(),
            self.os,
            self.data_dir,
            self.router,
            self.routing,
            self.agents,
            self.log
        )
    }
}

// ---- Sync -----------------------------------------------------------------

/// Settings › Sync (design: Synced Rules): the folders `rules.json` syncs
/// with, how each stands, this machine's name, and what the synced
/// destinations and rule sets mean here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sync {
    /// This machine's name in sync status and manifests.
    pub machine_name: SharedString,
    /// The computer's own name: what an empty name falls back to.
    pub computer_name: SharedString,
    pub folders: Vec<SyncFolder>,
    /// Cloud folders found on this machine and not synced yet.
    pub offered: Vec<SyncPlace>,
    /// The Finicky config in charge, as shown ("~/.finicky.ts"), when one
    /// is; and whether it syncs too.
    pub config: Option<SharedString>,
    pub config_on: bool,
    /// "Synced 09:41", "Not synced yet", or the worst folder's state, for
    /// the pane head's chip.
    pub summary: SharedString,
    pub summary_tone: SyncTone,
    /// When every folder last agreed: "09:41", "never".
    pub last_synced: SharedString,
    /// How many earlier versions are kept.
    pub backups: usize,
    pub destinations: Vec<DestinationRow>,
    pub sets: Vec<RuleSetRow>,
    /// Why the last change was not made.
    pub problem: Option<SharedString>,
}

impl Sync {
    /// Whether a folder is in conflict (or joining), which needs the reader.
    pub fn has_conflict(&self) -> bool {
        self.folders.iter().any(|f| f.conflict.is_some())
    }
}

/// A chip's tone, as the producer judges it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyncTone {
    #[default]
    Neutral,
    Ok,
    Warn,
    Bad,
}

/// A cloud folder the Sync to card offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncPlace {
    /// "Google Drive · jo@example.com".
    pub label: SharedString,
    /// The folder sync would use, shown: "~/Library/CloudStorage/Dropbox/PwrRoute".
    pub shown: SharedString,
    pub root: std::path::PathBuf,
}

/// One folder rules sync with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncFolder {
    pub label: SharedString,
    pub shown: SharedString,
    pub root: std::path::PathBuf,
    /// "in sync", "waiting", "unavailable", "conflict".
    pub chip: SharedString,
    pub tone: SyncTone,
    /// "In sync · 09:41 · saved by MacBook Air".
    pub line: SharedString,
    pub conflict: Option<SyncConflict>,
}

/// Two versions that both changed, until the reader picks one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncConflict {
    /// `rules.json` or the config's name.
    pub file: SharedString,
    /// "Both changed since the last sync", or the join's sentence.
    pub headline: SharedString,
    /// "14 rules · this Mac".
    pub mine: SharedString,
    /// "15 rules · saved 09:20".
    pub theirs: SharedString,
    /// Who saved the folder's version: "MacBook Air", else the folder.
    pub theirs_name: SharedString,
    /// What differs, a line each: "+ rule 15 *.example.org/* → Work 2".
    pub differences: Vec<SharedString>,
    /// The folder's version as shown, so a choice applies to it only.
    pub remote_sha: SharedString,
    /// This machine had never synced with the folder.
    pub join: bool,
}

/// One named destination and what it means on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DestinationRow {
    pub name: SharedString,
    /// "Google Chrome · Work".
    pub here: SharedString,
    /// Bound on this machine (`machine.json`) rather than synced.
    pub this_machine: bool,
    /// Rules (and Everything else) that open in it.
    pub uses: usize,
    /// "Arc isn't installed on this Mac", when the binding cannot open.
    pub warning: Option<SharedString>,
    /// The pickers' lanes, the current binding lit.
    pub lanes: Vec<crate::destinations::Lane>,
}

/// One rule set and whether it is on here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleSetRow {
    pub name: SharedString,
    /// Rules and rewrites in it.
    pub entries: usize,
    pub on: bool,
    /// The synced default, and whether this machine overrides it.
    pub synced_on: bool,
    pub overridden: bool,
}

// ---- About ----------------------------------------------------------------

/// Settings › About: the build, the license and where to find the
/// project, as plain text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct About {
    /// The license line: "MIT. Includes code from Finicky, …".
    pub license: SharedString,
    /// Where to find the project: each a label and the address as text.
    /// Help's rows open them; here they are words to copy.
    pub links: Vec<(SharedString, SharedString)>,
    /// This copy: the Version row ("PwrRoute 0.9.0", "debug build for
    /// aarch64"), the Source row and Copy build info (design: Build
    /// Identity 2a, 2b).
    pub source: BuildInfo,
}

// ---- Requests -------------------------------------------------------------

/// What the window asks of the rest of the app.
///
/// The window updates its own model where the answer is obvious (a switch
/// flips), and sends every request to the handler given to it. Nothing in
/// the window acts on the OS itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsRequest {
    SetYieldBrowserFocus(bool),
    SetAgentAccess(bool),
    SetSessionRole {
        session: SessionId,
        role: RoleId,
    },
    /// Arrives only after the row's Revoke was confirmed.
    RevokeSession(SessionId),
    /// Allow a revoked session again, with `role` (the row's last one).
    AllowSession {
        session: SessionId,
        role: RoleId,
    },
    /// Drop a revoked session; its next connection is asked about again.
    ForgetSession(SessionId),
    NewCustomRole,
    OpenActivity,
    SetRulesSource(RulesSource),
    /// Configuration › Everything else: whose default takes the links
    /// nothing claims, written as `rules.json`'s `precedence`.
    SetDefaultOwner(DefaultOwner),
    /// Configuration › Everything else: `rules.json`'s `defaultBrowser`
    /// and `defaultProfile` ("" for the browser's own profile).
    SetDefaultBrowser {
        browser: SharedString,
        profile: SharedString,
    },
    /// Open the importer on the Finicky config (Finicky Importer 1a).
    ReviewImport,
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
    /// Updates: ask GitHub Releases now.
    CheckForUpdates,
    /// Updates: install the downloaded release and start again.
    RestartToUpdate,
    /// Updates: follow this release channel slot from now on.
    SetUpdateSlot(Slot),
    /// About: Help's Changelog window.
    OpenChangelog,
    /// About: Help's License window.
    OpenLicense,
    /// About: Help's Third-Party Notices window.
    OpenThirdPartyNotices,
    /// Diagnostics: Help's Logs window.
    OpenLogs,
    /// Privacy: keep the last `count` links for at most `hours`.
    SetRetention {
        count: usize,
        hours: u64,
    },
    /// Privacy: the journal switch. `Full` arrives only after the sheet
    /// confirmed it.
    SetJournal(JournalLevel),
    /// Privacy: show the journal's directory in the file manager.
    RevealJournal,
    /// Privacy: delete every journal file and empty the links in memory.
    /// Arrives only after the sheet confirmed it.
    ClearJournal,
    /// Privacy: empty the links in memory, here, in Activity and the tray.
    ClearRecent,
    /// Privacy: write the links in memory, each at its level, to a file the
    /// reader picks.
    ExportActivity,
    /// Sync: run a pass now.
    SyncNow,
    /// Sync: start syncing with this cloud folder.
    AddSyncFolder(std::path::PathBuf),
    /// Sync: pick any folder with the OS's folder panel.
    ChooseSyncFolder,
    RemoveSyncFolder(std::path::PathBuf),
    RevealSyncFolder(std::path::PathBuf),
    /// Sync: settle a conflict; `keep_local` keeps this machine's version.
    ResolveSync {
        root: std::path::PathBuf,
        file: SharedString,
        remote_sha: SharedString,
        keep_local: bool,
    },
    SetMachineName(SharedString),
    /// Sync: "Also sync" the Finicky config in charge.
    SetSyncConfig(bool),
    RevealSyncBackups,
    /// Sync: bind a named destination on this machine.
    BindDestination {
        name: SharedString,
        browser: SharedString,
        profile: SharedString,
        private: bool,
    },
    /// Sync: drop this machine's binding; the synced one applies.
    UseSyncedBinding(SharedString),
    /// Sync: turn a rule set on or off on this machine.
    SetRuleSetHere {
        name: SharedString,
        on: bool,
    },
}
