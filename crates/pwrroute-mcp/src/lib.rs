//! PwrRoute's local-agent door: an MCP server on loopback, behind OAuth 2.1
//! and PwrRoute's own approval window, as PwrSnap and PwrGit have.
//!
//! - [`McpServer`] listens on `127.0.0.1` only and serves the MCP endpoint
//!   (stateless streamable HTTP, via `rmcp`) and the OAuth endpoints a
//!   client discovers from its first 401.
//! - [`Approver`] is the app's half of approval: it shows the window and
//!   answers. Nothing a browser sends can approve.
//! - [`AccessControl`] holds clients and sessions; Settings lists, re-roles
//!   and revokes sessions through it.
//! - [`Backends`] are what the tools act on. [`EngineResolver`] adapts
//!   today's engine; [`fake`] has in-memory stand-ins for the rest.
//!
//! URLs that pass through MCP stay in memory. The only files written are the
//! client and session stores, and [`AuditRecord`]s carry the tool, session
//! and timing only.

mod access;
mod approval;
mod audit;
mod backend;
mod engine;
pub mod fake;
mod oauth;
mod roles;
mod server;
mod tools;

pub use access::{
    AccessControl, Client, MAX_CLIENTS, MAX_SESSION_NAME, MAX_SESSIONS, Principal, SessionInfo,
};
pub use approval::{ApprovalDecision, ApprovalReply, ApprovalRequest, Approver};
pub use audit::{AuditRecord, AuditSink, MemoryAudit, NoAudit, Outcome};
pub use backend::{
    ActivityEntry, ActivityLog, Backends, BrowserCatalog, BrowserList, DetectedBrowser, Resolver,
    Router, RulesError, RulesSnapshot, RulesSource, RulesStore, SessionRef, StepKind, Trace,
    TraceStep,
};
pub use engine::EngineResolver;
pub use roles::{Permission, Permissions, Role};
pub use server::{Config, DEFAULT_PORT, McpServer};
