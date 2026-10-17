//! Registered clients and approved sessions: what Settings › Local agents
//! lists, and what every MCP request is checked against.
//!
//! A *session* is one approval: a name, a role, the scopes the client asked
//! for, and one bearer token. It is not the MCP transport's session (the
//! transport is stateless). Only a SHA-256 of each token is kept, in memory
//! and on disk; revoking a session drops even that, so its token stops
//! working on the next request.
//!
//! Two files under the state directory hold this, written atomically with
//! owner-only permissions. Neither holds a URL that passed through MCP:
//! clients carry their registered metadata (redirect URIs are the client's
//! own callback), sessions carry names, roles and timestamps.

use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio::sync::watch;

use crate::{
    backend::SessionRef,
    roles::{Permission, Permissions, Role},
};

/// Registered clients kept at once. At the cap, the oldest client with no
/// active session makes room.
pub const MAX_CLIENTS: usize = 256;
/// Sessions kept at once, revoked ones included. At the cap, the oldest
/// revoked session makes room.
pub const MAX_SESSIONS: usize = 256;
/// The longest Session Name, in characters.
pub const MAX_SESSION_NAME: usize = 200;
/// `lastUsedAt` is written to disk at most this often per session.
const LAST_USED_WRITE_INTERVAL: Duration = Duration::from_secs(60);

const TOKEN_PREFIX: &str = "prmcp_";
const CLIENTS_FILE: &str = "mcp-oauth-clients.json";
const SESSIONS_FILE: &str = "mcp-sessions.json";
const CLIENTS_FORMAT: &str = "pwrroute.mcp-oauth-clients/v1";
const SESSIONS_FORMAT: &str = "pwrroute.mcp-sessions/v1";

/// RFC 7591 client metadata kept from a registration. Anything else is
/// dropped, as the siblings' SDK schema strips it.
const CLIENT_FIELDS: &[&str] = &[
    "redirect_uris",
    "client_name",
    "client_uri",
    "logo_uri",
    "scope",
    "contacts",
    "tos_uri",
    "policy_uri",
    "jwks_uri",
    "jwks",
    "software_id",
    "software_version",
    "software_statement",
];

/// A registered public client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Client {
    pub client_id: String,
    pub client_id_issued_at: u64,
    pub redirect_uris: Vec<String>,
    /// The rest of the registered metadata, echoed back as registered.
    #[serde(flatten)]
    pub metadata: Map<String, Value>,
}

impl Client {
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.metadata
            .get("client_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|n| !n.is_empty())
    }

    /// The registration response: the metadata, with the fields every
    /// PwrRoute client gets forced to their only allowed values.
    #[must_use]
    pub fn registration(&self) -> Value {
        let mut out = self.metadata.clone();
        out.insert("client_id".into(), self.client_id.clone().into());
        out.insert(
            "client_id_issued_at".into(),
            self.client_id_issued_at.into(),
        );
        out.insert(
            "redirect_uris".into(),
            Value::from(self.redirect_uris.clone()),
        );
        out.insert("token_endpoint_auth_method".into(), "none".into());
        out.insert("grant_types".into(), Value::from(["authorization_code"]));
        out.insert("response_types".into(), Value::from(["code"]));
        Value::Object(out)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionRecord {
    id: String,
    name: String,
    role: Role,
    client_id: String,
    client_name: String,
    /// What the client asked for and the person consented to. The role's
    /// permissions are cut to these.
    scopes: Permissions,
    /// SHA-256 of the token, lowercase hex. `None` once revoked.
    token_hash: Option<String>,
    created_at: u64,
    updated_at: u64,
    last_used_at: Option<u64>,
    revoked_at: Option<u64>,
}

/// A session as Settings shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub client_name: String,
    pub role: Role,
    pub scopes: Permissions,
    /// What the session can do now: its role, within its scopes.
    pub permissions: Permissions,
    /// Seconds since the Unix epoch.
    pub created_at: u64,
    pub last_used_at: Option<u64>,
    pub revoked_at: Option<u64>,
    /// Tool calls since PwrRoute started. Not persisted.
    pub calls: u64,
}

/// A request's caller, once its token checks out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub session: SessionRef,
    pub role: Role,
    pub permissions: Permissions,
}

/// Why a token or session cannot act.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Denied {
    #[error("invalid_session: the session is unknown or was revoked")]
    InvalidSession,
    #[error("missing_permission: this session's role does not allow {tool} (needs {permission})")]
    MissingPermission {
        tool: &'static str,
        permission: Permission,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegisterError {
    #[error("{0}")]
    InvalidMetadata(String),
    #[error("OAuth client limit reached")]
    Full,
    #[error("could not store the client: {0}")]
    Store(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    #[error("an active session already uses the name {0}; approve again with another name")]
    NameTaken(String),
    #[error("the Session Name must be 1 to 200 characters")]
    BadName,
    #[error("the session limit is reached; revoke a session in Settings first")]
    Full,
    #[error("could not store the session: {0}")]
    Store(String),
}

#[derive(Debug, Default)]
struct State {
    /// Registration order; the front is evicted first.
    clients: Vec<Client>,
    sessions: Vec<SessionRecord>,
    calls: HashMap<String, u64>,
    last_used_written: HashMap<String, Instant>,
}

/// The client and session store. Cheap to clone; clones share state.
#[derive(Debug, Clone)]
pub struct AccessControl {
    state: Arc<Mutex<State>>,
    dir: Option<PathBuf>,
    changes: Arc<watch::Sender<u64>>,
}

impl AccessControl {
    /// Loads clients and sessions from `dir`, or starts empty in memory
    /// when `dir` is `None`. A file that does not parse is an error, not an
    /// empty store: failing closed keeps a damaged file from re-admitting a
    /// revoked session.
    pub fn open(dir: Option<&Path>) -> Result<Self> {
        let mut state = State::default();
        if let Some(dir) = dir {
            if let Some(file) = read_versioned::<Vec<Client>>(dir, CLIENTS_FILE, CLIENTS_FORMAT)? {
                if file.len() > MAX_CLIENTS {
                    bail!("{CLIENTS_FILE}: more than {MAX_CLIENTS} clients");
                }
                state.clients = file;
            }
            if let Some(file) =
                read_versioned::<Vec<SessionRecord>>(dir, SESSIONS_FILE, SESSIONS_FORMAT)?
            {
                if file.len() > MAX_SESSIONS {
                    bail!("{SESSIONS_FILE}: more than {MAX_SESSIONS} sessions");
                }
                state.sessions = file;
            }
        }
        Ok(Self {
            state: Arc::new(Mutex::new(state)),
            dir: dir.map(Path::to_path_buf),
            changes: Arc::new(watch::channel(0).0),
        })
    }

    /// Ticks whenever a session or client is added, changed or revoked, so
    /// Settings can redraw.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// Every session, newest first.
    #[must_use]
    pub fn sessions(&self) -> Vec<SessionInfo> {
        let state = self.lock();
        state
            .sessions
            .iter()
            .rev()
            .map(|s| SessionInfo {
                id: s.id.clone(),
                name: s.name.clone(),
                client_name: s.client_name.clone(),
                role: s.role,
                scopes: s.scopes.clone(),
                permissions: effective(s),
                created_at: s.created_at,
                last_used_at: s.last_used_at,
                revoked_at: s.revoked_at,
                calls: state.calls.get(&s.id).copied().unwrap_or(0),
            })
            .collect()
    }

    /// Revokes a session. Its token fails on the next request. Returns
    /// whether an active session was revoked.
    pub fn revoke(&self, session_id: &str) -> Result<bool> {
        let mut state = self.lock();
        let now = now_secs();
        let Some(session) = state
            .sessions
            .iter_mut()
            .find(|s| s.id == session_id && s.revoked_at.is_none())
        else {
            return Ok(false);
        };
        session.revoked_at = Some(now);
        session.updated_at = now;
        session.token_hash = None;
        self.save_sessions(&state)?;
        drop(state);
        self.changed();
        Ok(true)
    }

    /// Gives a session another role. What it can do stays within the scopes
    /// it was approved for.
    pub fn assign_role(&self, session_id: &str, role: Role) -> Result<bool> {
        let mut state = self.lock();
        let Some(session) = state
            .sessions
            .iter_mut()
            .find(|s| s.id == session_id && s.revoked_at.is_none())
        else {
            return Ok(false);
        };
        session.role = role;
        session.updated_at = now_secs();
        self.save_sessions(&state)?;
        drop(state);
        self.changed();
        Ok(true)
    }

    pub(crate) fn client(&self, client_id: &str) -> Option<Client> {
        self.lock()
            .clients
            .iter()
            .find(|c| c.client_id == client_id)
            .cloned()
    }

    /// Registers a public client from RFC 7591 metadata. Registration
    /// grants nothing; only an approval does.
    pub(crate) fn register(&self, metadata: &Map<String, Value>) -> Result<Client, RegisterError> {
        let client = validate_registration(metadata)?;
        let mut state = self.lock();
        if state.clients.len() >= MAX_CLIENTS {
            let bound = |id: &str| {
                state
                    .sessions
                    .iter()
                    .any(|s| s.revoked_at.is_none() && s.client_id == id)
            };
            let Some(evict) = state.clients.iter().position(|c| !bound(&c.client_id)) else {
                return Err(RegisterError::Full);
            };
            state.clients.remove(evict);
        }
        state.clients.push(client.clone());
        if let Err(e) = self.save_clients(&state) {
            state.clients.pop();
            return Err(RegisterError::Store(format!("{e:#}")));
        }
        drop(state);
        self.changed();
        Ok(client)
    }

    /// `base`, or `base 2`, `base 3`… so it differs from every active
    /// session's name.
    pub(crate) fn suggest_session_name(&self, base: &str) -> String {
        let state = self.lock();
        let base: String = base.trim().chars().take(MAX_SESSION_NAME - 4).collect();
        let base = if base.is_empty() {
            "Local agent".to_owned()
        } else {
            base
        };
        (1..)
            .map(|n| {
                if n == 1 {
                    base.clone()
                } else {
                    format!("{base} {n}")
                }
            })
            .find(|name| !name_taken(&state, name))
            .unwrap_or(base)
    }

    /// Creates a session for an approved code and returns its token, the
    /// only time the token exists in the clear.
    pub(crate) fn create_session(
        &self,
        name: &str,
        role: Role,
        client: &Client,
        scopes: Permissions,
    ) -> Result<(String, String), SessionError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > MAX_SESSION_NAME {
            return Err(SessionError::BadName);
        }
        let mut state = self.lock();
        if name_taken(&state, name) {
            return Err(SessionError::NameTaken(name.to_owned()));
        }
        if state.sessions.len() >= MAX_SESSIONS {
            let Some(oldest_revoked) = state.sessions.iter().position(|s| s.revoked_at.is_some())
            else {
                return Err(SessionError::Full);
            };
            let gone = state.sessions.remove(oldest_revoked);
            state.calls.remove(&gone.id);
        }
        let token = format!("{TOKEN_PREFIX}{}", random_b64(32));
        let now = now_secs();
        let session = SessionRecord {
            id: format!("ses_{}", random_hex(12)),
            name: name.to_owned(),
            role,
            client_id: client.client_id.clone(),
            client_name: client.name().unwrap_or("Local MCP client").to_owned(),
            scopes,
            token_hash: Some(sha256_hex(&token)),
            created_at: now,
            updated_at: now,
            last_used_at: None,
            revoked_at: None,
        };
        let id = session.id.clone();
        state.sessions.push(session);
        if let Err(e) = self.save_sessions(&state) {
            state.sessions.pop();
            return Err(SessionError::Store(format!("{e:#}")));
        }
        drop(state);
        self.changed();
        Ok((id, token))
    }

    /// The caller behind a bearer token, or `None` for an unknown or
    /// revoked one. Compares every stored hash in constant time.
    #[must_use]
    pub fn authenticate(&self, token: &str) -> Option<Principal> {
        let hash = sha256_hex(token);
        let state = self.lock();
        let mut found = None;
        for session in &state.sessions {
            if let Some(stored) = &session.token_hash
                && constant_time_eq(stored.as_bytes(), hash.as_bytes())
            {
                found = Some(session);
            }
        }
        found.filter(|s| s.revoked_at.is_none()).map(principal_of)
    }

    /// The caller as of now: a role change or revocation since the request
    /// was authenticated applies.
    pub(crate) fn principal(&self, session_id: &str) -> Result<Principal, Denied> {
        self.lock()
            .sessions
            .iter()
            .find(|s| s.id == session_id && s.revoked_at.is_none())
            .map(principal_of)
            .ok_or(Denied::InvalidSession)
    }

    /// Counts a tool call and moves `lastUsedAt`, writing it at most once a
    /// minute.
    pub(crate) fn record_use(&self, session_id: &str) {
        let mut state = self.lock();
        *state.calls.entry(session_id.to_owned()).or_default() += 1;
        let due = state
            .last_used_written
            .get(session_id)
            .is_none_or(|t| t.elapsed() >= LAST_USED_WRITE_INTERVAL);
        let now = now_secs();
        if let Some(s) = state.sessions.iter_mut().find(|s| s.id == session_id) {
            s.last_used_at = Some(now);
        }
        if due {
            state
                .last_used_written
                .insert(session_id.to_owned(), Instant::now());
            // Best effort: losing a timestamp must not fail the call.
            let _ = self.save_sessions(&state);
        }
        drop(state);
        self.changed();
    }

    /// RFC 7009 revocation: revokes the token's session only when `client`
    /// owns it. Unknown tokens are not an error.
    pub(crate) fn revoke_token(&self, client_id: &str, token: &str) -> Result<()> {
        let Some(principal) = self.authenticate(token) else {
            return Ok(());
        };
        let owned = self
            .lock()
            .sessions
            .iter()
            .any(|s| s.id == principal.session.id && s.client_id == client_id);
        if owned {
            self.revoke(&principal.session.id)?;
        }
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock leaves plain data behind; carry on.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn changed(&self) {
        self.changes.send_modify(|n| *n = n.wrapping_add(1));
    }

    fn save_clients(&self, state: &State) -> Result<()> {
        match &self.dir {
            Some(dir) => write_versioned(dir, CLIENTS_FILE, CLIENTS_FORMAT, &state.clients),
            None => Ok(()),
        }
    }

    fn save_sessions(&self, state: &State) -> Result<()> {
        match &self.dir {
            Some(dir) => write_versioned(dir, SESSIONS_FILE, SESSIONS_FORMAT, &state.sessions),
            None => Ok(()),
        }
    }
}

fn effective(session: &SessionRecord) -> Permissions {
    session
        .role
        .permissions()
        .intersection(&session.scopes)
        .copied()
        .collect()
}

fn principal_of(session: &SessionRecord) -> Principal {
    Principal {
        session: SessionRef {
            id: session.id.clone(),
            name: session.name.clone(),
        },
        role: session.role,
        permissions: effective(session),
    }
}

fn name_taken(state: &State, name: &str) -> bool {
    let name = name.trim().to_lowercase();
    state
        .sessions
        .iter()
        .any(|s| s.revoked_at.is_none() && s.name.to_lowercase() == name)
}

fn validate_registration(input: &Map<String, Value>) -> Result<Client, RegisterError> {
    let bad = |m: &str| RegisterError::InvalidMetadata(m.to_owned());
    let uris = input
        .get("redirect_uris")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or_else(|| bad("redirect_uris must be a non-empty list"))?;
    let mut redirect_uris = Vec::with_capacity(uris.len());
    for uri in uris {
        let uri = uri
            .as_str()
            .ok_or_else(|| bad("redirect_uris must be strings"))?;
        let parsed = url::Url::parse(uri).map_err(|_| bad("redirect_uris must be URLs"))?;
        if matches!(parsed.scheme(), "javascript" | "data" | "vbscript") {
            return Err(bad("redirect_uris cannot use that scheme"));
        }
        if parsed.fragment().is_some() {
            return Err(bad("redirect_uris cannot have a fragment"));
        }
        redirect_uris.push(uri.to_owned());
    }
    let mut metadata = Map::new();
    for field in CLIENT_FIELDS.iter().filter(|f| **f != "redirect_uris") {
        let Some(value) = input.get(*field) else {
            continue;
        };
        let ok = match *field {
            "contacts" => value
                .as_array()
                .is_some_and(|a| a.iter().all(Value::is_string)),
            "jwks" => value.is_object(),
            _ => value.is_string(),
        };
        if !ok {
            return Err(bad(&format!("{field} has the wrong type")));
        }
        metadata.insert((*field).to_owned(), value.clone());
    }
    Ok(Client {
        client_id: format!("prc_{}", random_hex(16)),
        client_id_issued_at: now_secs(),
        redirect_uris,
        metadata,
    })
}

#[derive(Serialize, Deserialize)]
struct Versioned<T> {
    format: String,
    entries: T,
}

fn read_versioned<T: for<'de> Deserialize<'de>>(
    dir: &Path,
    name: &str,
    format: &str,
) -> Result<Option<T>> {
    let path = dir.join(name);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| path.display().to_string()),
    };
    let file: Versioned<T> =
        serde_json::from_str(&text).with_context(|| path.display().to_string())?;
    if file.format != format {
        bail!("{}: expected format {format}", path.display());
    }
    Ok(Some(file.entries))
}

fn write_versioned<T: Serialize>(dir: &Path, name: &str, format: &str, entries: &T) -> Result<()> {
    create_private_dir(dir)?;
    let path = dir.join(name);
    let temporary = dir.join(format!("{name}.{}.tmp", std::process::id()));
    let json = serde_json::to_vec_pretty(&Versioned {
        format: format.to_owned(),
        entries,
    })?;
    write_private(&temporary, &json)?;
    fs::rename(&temporary, &path).with_context(|| path.display().to_string())?;
    Ok(())
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

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).expect("the OS random source failed");
    bytes
}

/// `n` random bytes as unpadded base64url.
pub(crate) fn random_b64(n: usize) -> String {
    let bytes: Vec<u8> = (0..n.div_ceil(32))
        .flat_map(|_| random_bytes::<32>())
        .take(n)
        .collect();
    URL_SAFE_NO_PAD.encode(bytes)
}

fn random_hex(n: usize) -> String {
    let bytes: Vec<u8> = (0..n.div_ceil(32))
        .flat_map(|_| random_bytes::<32>())
        .take(n)
        .collect();
    hex(&bytes)
}

pub(crate) fn sha256_hex(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn registration() -> Map<String, Value> {
        json!({
            "redirect_uris": ["http://127.0.0.1:4567/callback"],
            "client_name": "Test agent",
            "client_secret": "dropped",
            "grant_types": ["client_credentials"]
        })
        .as_object()
        .unwrap()
        .clone()
    }

    #[test]
    fn registration_forces_a_public_code_client() {
        let access = AccessControl::open(None).unwrap();
        let client = access.register(&registration()).unwrap();
        let out = client.registration();
        assert_eq!(out["token_endpoint_auth_method"], "none");
        assert_eq!(out["grant_types"], json!(["authorization_code"]));
        assert!(out.get("client_secret").is_none());
        assert!(client.client_id.starts_with("prc_"));
    }

    #[test]
    fn registration_refuses_script_redirects() {
        let access = AccessControl::open(None).unwrap();
        let mut bad = registration();
        bad.insert("redirect_uris".into(), json!(["javascript:alert(1)"]));
        assert!(matches!(
            access.register(&bad),
            Err(RegisterError::InvalidMetadata(_))
        ));
    }

    #[test]
    fn tokens_are_stored_hashed_and_revocation_drops_them() {
        let dir = std::env::temp_dir().join(format!("pwrroute-mcp-access-{}", random_hex(6)));
        let access = AccessControl::open(Some(&dir)).unwrap();
        let client = access.register(&registration()).unwrap();
        let (id, token) = access
            .create_session("Agent", Role::Reader, &client, Role::Reader.permissions())
            .unwrap();
        let on_disk = fs::read_to_string(dir.join(SESSIONS_FILE)).unwrap();
        assert!(!on_disk.contains(&token));
        assert!(on_disk.contains(&sha256_hex(&token)));

        // Survives a restart.
        let reopened = AccessControl::open(Some(&dir)).unwrap();
        assert_eq!(reopened.authenticate(&token).unwrap().session.id, id);

        assert!(reopened.revoke(&id).unwrap());
        assert!(reopened.authenticate(&token).is_none());
        let on_disk = fs::read_to_string(dir.join(SESSIONS_FILE)).unwrap();
        assert!(!on_disk.contains(&sha256_hex(&token)));
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("pwrroute-mcp-mode-{}", random_hex(6)));
        let access = AccessControl::open(Some(&dir)).unwrap();
        access.register(&registration()).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join(CLIENTS_FILE)), 0o600);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_damaged_file_fails_closed() {
        let dir = std::env::temp_dir().join(format!("pwrroute-mcp-bad-{}", random_hex(6)));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(SESSIONS_FILE), "{").unwrap();
        assert!(AccessControl::open(Some(&dir)).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn role_changes_stay_within_consent() {
        let access = AccessControl::open(None).unwrap();
        let client = access.register(&registration()).unwrap();
        let (id, token) = access
            .create_session("Agent", Role::Reader, &client, Role::Reader.permissions())
            .unwrap();
        assert!(access.assign_role(&id, Role::Editor).unwrap());
        let principal = access.authenticate(&token).unwrap();
        assert_eq!(principal.role, Role::Editor);
        assert!(!principal.permissions.contains(&Permission::RulesWrite));
        assert!(!principal.permissions.contains(&Permission::Open));
    }

    #[test]
    fn session_names_are_unique_among_active_sessions() {
        let access = AccessControl::open(None).unwrap();
        let client = access.register(&registration()).unwrap();
        let scopes = Role::Reader.permissions();
        access
            .create_session("Agent", Role::Reader, &client, scopes.clone())
            .unwrap();
        assert_eq!(access.suggest_session_name("Agent"), "Agent 2");
        assert_eq!(
            access.create_session("agent", Role::Reader, &client, scopes),
            Err(SessionError::NameTaken("agent".into()))
        );
    }
}
