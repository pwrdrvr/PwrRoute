//! PwrRoute's local-agent door: MCP served by the running app over a
//! per-user local link, reached through `pwrroute mcp`.
//!
//! - `pwrroute mcp` ([`proxy`]) is what an MCP client launches. It copies
//!   stdio to and from the app's [`link`]: a Unix socket on macOS and Linux,
//!   a named pipe on Windows. It is a byte pipe; the app is the server.
//! - [`McpServer`] accepts link connections and serves MCP (`rmcp`) on each.
//! - [`Approver`] is the app's approval window. The first time a client
//!   name connects, the person picks a role or denies; [`Agents`] remembers
//!   the answer, and Settings changes, revokes or forgets it there.
//! - [`Backends`] are what the tools act on. [`live`] implements them over
//!   the router's handle, the browser catalog and `rules.json`;
//!   [`EngineTester`] runs the saved examples and tests on an engine of its
//!   own; [`fake`] has in-memory stand-ins for tests.
//!
//! URLs that pass through MCP stay in memory. The only files written are
//! the approval store and, through `pwrroute_rules_apply`, `rules.json`
//! itself, with the examples and tests an agent writes into it;
//! [`AuditRecord`]s carry the tool, agent and timing.

mod agents;
mod approval;
mod audit;
mod backend;
mod engine;
pub mod fake;
pub mod link;
pub mod live;
pub mod proxy;
mod roles;
mod server;
mod tools;

pub use agents::{AgentInfo, Agents, MAX_AGENTS, MAX_NAME};
pub use approval::{ApprovalDecision, ApprovalReply, ApprovalRequest, Approver};
pub use audit::{AuditRecord, AuditSink, MemoryAudit, NoAudit, Outcome};
pub use backend::{
    ActivityEntry, ActivityLog, ActivityOutcome, ActivitySource, AgentRef, Backends,
    BrowserCatalog, BrowserList, DetectedBrowser, Resolver, Router, RulesError, RulesSnapshot,
    RulesSource, RulesStore, RulesTester, StepKind, Trace, TraceStep,
};
pub use engine::{EngineResolver, EngineTester, EngineThread, Loader};
pub use link::Endpoint;
pub use roles::{Permission, Permissions, Role};
pub use server::{Config, McpServer};
