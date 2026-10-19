//! The agents a person has approved, by MCP client name, and the role each
//! has: what Settings › Local agents lists.
//!
//! An agent is known by the `clientInfo.name` its client sends (e.g.
//! `claude-code`). That name is self-reported, and any process of this user
//! can reach the link, so a role is a guardrail against an agent doing more
//! than the person meant, not a wall against a hostile local process.
//!
//! Approvals persist in `mcp-agents.json` under the state directory, written
//! atomically and owner-only. It holds names, roles and timestamps; never a
//! URL.

use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::roles::{Permissions, Role};

/// Approved agents kept at once.
pub const MAX_AGENTS: usize = 256;
/// The longest client name kept; longer ones are cut.
pub const MAX_NAME: usize = 200;
/// `lastUsedAt` is written to disk at most this often per agent.
const LAST_USED_WRITE_INTERVAL: Duration = Duration::from_secs(60);

const FILE: &str = "mcp-agents.json";
const FORMAT: &str = "pwrroute.mcp-agents/v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    name: String,
    role: Role,
    /// Seconds since the Unix epoch.
    approved_at: u64,
    last_used_at: Option<u64>,
    revoked_at: Option<u64>,
}

/// An agent as Settings shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub name: String,
    pub role: Role,
    pub permissions: Permissions,
    pub approved_at: u64,
    pub last_used_at: Option<u64>,
    pub revoked_at: Option<u64>,
    /// Connections open now.
    pub connections: usize,
    /// Tool calls since PwrRoute started. Not persisted.
    pub calls: u64,
}

/// Where a client name stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Standing {
    Allowed(Role),
    /// Revoked in Settings: refused without asking again, until Settings
    /// allows it.
    Revoked,
    /// Never approved, or forgotten: ask.
    Unknown,
}

#[derive(Debug, Default)]
struct State {
    records: Vec<Record>,
    connections: HashMap<String, usize>,
    calls: HashMap<String, u64>,
    last_used_written: HashMap<String, Instant>,
}

/// The approval store. Cheap to clone; clones share state.
#[derive(Debug, Clone)]
pub struct Agents {
    state: Arc<Mutex<State>>,
    dir: Option<PathBuf>,
    changes: Arc<watch::Sender<u64>>,
}

impl Agents {
    /// Loads approvals from `dir`, or starts empty in memory when `dir` is
    /// `None`. A file that does not parse is an error, not an empty store:
    /// failing closed keeps a damaged file from forgetting a revocation.
    pub fn open(dir: Option<&Path>) -> Result<Self> {
        let mut state = State::default();
        if let Some(dir) = dir {
            let path = dir.join(FILE);
            match fs::read_to_string(&path) {
                Ok(text) => {
                    let file: Versioned =
                        serde_json::from_str(&text).with_context(|| path.display().to_string())?;
                    if file.format != FORMAT {
                        bail!("{}: expected format {FORMAT}", path.display());
                    }
                    if file.agents.len() > MAX_AGENTS {
                        bail!("{}: more than {MAX_AGENTS} agents", path.display());
                    }
                    state.records = file.agents;
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).with_context(|| path.display().to_string()),
            }
        }
        Ok(Self {
            state: Arc::new(Mutex::new(state)),
            dir: dir.map(Path::to_path_buf),
            changes: Arc::new(watch::channel(0).0),
        })
    }

    /// Ticks whenever an agent is approved, changed, revoked, connects or
    /// calls a tool, so Settings can redraw.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// Every known agent, most recently approved first.
    #[must_use]
    pub fn agents(&self) -> Vec<AgentInfo> {
        let state = self.lock();
        let mut out: Vec<AgentInfo> = state
            .records
            .iter()
            .map(|r| AgentInfo {
                name: r.name.clone(),
                role: r.role,
                permissions: r.role.permissions(),
                approved_at: r.approved_at,
                last_used_at: r.last_used_at,
                revoked_at: r.revoked_at,
                connections: state.connections.get(&r.name).copied().unwrap_or(0),
                calls: state.calls.get(&r.name).copied().unwrap_or(0),
            })
            .collect();
        out.sort_by_key(|a| std::cmp::Reverse(a.approved_at));
        out
    }

    /// Approves `name` with `role`, or allows a revoked agent again. The
    /// approval window and Settings both come through here.
    pub fn allow(&self, name: &str, role: Role) -> Result<()> {
        let name = clean_name(name);
        let mut state = self.lock();
        let now = now_secs();
        if let Some(record) = state.records.iter_mut().find(|r| r.name == name) {
            record.role = role;
            record.approved_at = now;
            record.revoked_at = None;
        } else {
            if state.records.len() >= MAX_AGENTS {
                bail!("{MAX_AGENTS} agents are approved already; forget one in Settings first");
            }
            state.records.push(Record {
                name,
                role,
                approved_at: now,
                last_used_at: None,
                revoked_at: None,
            });
        }
        self.save(&state)?;
        drop(state);
        self.changed();
        Ok(())
    }

    /// Gives an agent another role, from its next call on.
    pub fn set_role(&self, name: &str, role: Role) -> Result<bool> {
        self.update(name, |r| r.role = role)
    }

    /// Refuses the agent from its next call on, without asking again.
    pub fn revoke(&self, name: &str) -> Result<bool> {
        let now = now_secs();
        self.update(name, |r| {
            r.revoked_at.get_or_insert(now);
        })
    }

    /// Drops the agent; its next connection is asked about again.
    pub fn forget(&self, name: &str) -> Result<bool> {
        let name = clean_name(name);
        let mut state = self.lock();
        let before = state.records.len();
        state.records.retain(|r| r.name != name);
        if state.records.len() == before {
            return Ok(false);
        }
        self.save(&state)?;
        drop(state);
        self.changed();
        Ok(true)
    }

    pub(crate) fn standing(&self, name: &str) -> Standing {
        let name = clean_name(name);
        match self.lock().records.iter().find(|r| r.name == name) {
            Some(r) if r.revoked_at.is_some() => Standing::Revoked,
            Some(r) => Standing::Allowed(r.role),
            None => Standing::Unknown,
        }
    }

    /// Counts a tool call and moves `lastUsedAt`, writing it at most once a
    /// minute.
    pub(crate) fn record_use(&self, name: &str) {
        let name = clean_name(name);
        let mut state = self.lock();
        // Only approved agents: a name nobody approved is not kept.
        let Some(record) = state.records.iter_mut().find(|r| r.name == name) else {
            return;
        };
        record.last_used_at = Some(now_secs());
        *state.calls.entry(name.clone()).or_default() += 1;
        let due = state
            .last_used_written
            .get(&name)
            .is_none_or(|t| t.elapsed() >= LAST_USED_WRITE_INTERVAL);
        if due {
            state.last_used_written.insert(name, Instant::now());
            // Best effort: losing a timestamp must not fail the call.
            let _ = self.save(&state);
        }
        drop(state);
        self.changed();
    }

    pub(crate) fn connected(&self, name: &str) {
        *self.lock().connections.entry(clean_name(name)).or_default() += 1;
        self.changed();
    }

    pub(crate) fn disconnected(&self, name: &str) {
        let name = clean_name(name);
        let mut state = self.lock();
        if let Some(n) = state.connections.get_mut(&name) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                state.connections.remove(&name);
            }
        }
        drop(state);
        self.changed();
    }

    fn update(&self, name: &str, change: impl FnOnce(&mut Record)) -> Result<bool> {
        let name = clean_name(name);
        let mut state = self.lock();
        let Some(record) = state.records.iter_mut().find(|r| r.name == name) else {
            return Ok(false);
        };
        change(record);
        self.save(&state)?;
        drop(state);
        self.changed();
        Ok(true)
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn changed(&self) {
        self.changes.send_modify(|n| *n = n.wrapping_add(1));
    }

    fn save(&self, state: &State) -> Result<()> {
        let Some(dir) = &self.dir else {
            return Ok(());
        };
        create_private_dir(dir)?;
        let path = dir.join(FILE);
        let temporary = dir.join(format!("{FILE}.{}.tmp", std::process::id()));
        let json = serde_json::to_vec_pretty(&Versioned {
            format: FORMAT.to_owned(),
            agents: state.records.clone(),
        })?;
        write_private(&temporary, &json)?;
        fs::rename(&temporary, &path).with_context(|| path.display().to_string())?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct Versioned {
    format: String,
    agents: Vec<Record>,
}

/// The name an agent is filed under: trimmed, cut to [`MAX_NAME`], and
/// never empty.
pub(crate) fn clean_name(name: &str) -> String {
    let name: String = name.trim().chars().take(MAX_NAME).collect();
    if name.is_empty() {
        "Unnamed MCP client".to_owned()
    } else {
        name
    }
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| dir.display().to_string())
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| dir.display().to_string())
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::{io::Write as _, os::unix::fs::OpenOptionsExt as _};
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| path.display().to_string())?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    // The per-user data folder's ACL is what protects it on Windows.
    fs::write(path, bytes).with_context(|| path.display().to_string())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("pwrroute-mcp-{tag}-{}-{n}", std::process::id()))
    }

    #[test]
    fn approvals_persist_and_revocation_sticks() {
        let dir = temp_dir("agents");
        let agents = Agents::open(Some(&dir)).unwrap();
        assert_eq!(agents.standing("claude-code"), Standing::Unknown);
        agents.allow("claude-code", Role::Opener).unwrap();
        assert!(agents.revoke("codex").is_ok_and(|changed| !changed));

        let reopened = Agents::open(Some(&dir)).unwrap();
        assert_eq!(
            reopened.standing("claude-code"),
            Standing::Allowed(Role::Opener)
        );
        reopened.revoke("claude-code").unwrap();
        let reopened = Agents::open(Some(&dir)).unwrap();
        assert_eq!(reopened.standing("claude-code"), Standing::Revoked);
        reopened.allow("claude-code", Role::Reader).unwrap();
        assert_eq!(
            reopened.standing("claude-code"),
            Standing::Allowed(Role::Reader)
        );
        reopened.forget("claude-code").unwrap();
        assert_eq!(reopened.standing("claude-code"), Standing::Unknown);
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = temp_dir("mode");
        Agents::open(Some(&dir))
            .unwrap()
            .allow("a", Role::Reader)
            .unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join(FILE)), 0o600);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_damaged_file_fails_closed() {
        let dir = temp_dir("bad");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(FILE), "{").unwrap();
        assert!(Agents::open(Some(&dir)).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn names_are_trimmed_and_never_empty() {
        assert_eq!(clean_name("  claude-code "), "claude-code");
        assert_eq!(clean_name(""), "Unnamed MCP client");
        assert_eq!(clean_name(&"x".repeat(500)).len(), MAX_NAME);
    }
}
