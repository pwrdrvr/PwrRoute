# pwrroute-mcp

The local-agent door: MCP on loopback behind OAuth 2.1 and PwrRoute's own
approval window (Settings › Local agents, design `PwrRoute Settings` 1a;
tools from `PwrRoute - Name and Brief` 2d). The model is PwrSnap's and
PwrGit's (`apps/desktop/src/main/local-agents/` and
`apps/desktop/src/main/agent-access/`); match their protocol behaviour, and
record any deliberate difference in the table below.

## Rules

- **Loopback only.** Bind `127.0.0.1`; the guard in `server.rs` checks the
  peer is loopback, `Host` is exactly `127.0.0.1:<port>` (DNS rebinding), and
  any `Origin` is an http(s) loopback page. It runs before every route. Do
  not add a listener on another address or relax the Host check.
- **Only the app approves.** `/authorize` asks the `Approver` and serves a
  page with no form or script; it refuses decision-like parameters. Never add
  a way for an HTTP request to approve, pick a role, or name a session.
- **Tokens are hashed.** Only a SHA-256 of a token is stored; revoking drops
  the hash. The clear token exists once, in the `/token` response.
- **Check permissions at call time.** `tools.rs` re-reads the session for
  every call, so a role change or revocation applies to the next one. Every
  tool stays listed; a refusal is an `isError` result naming the permission.
- **URLs stay in memory.** Arguments and results are never logged or
  written. `AuditRecord` has no field for them; do not add one. The only
  files are `mcp-oauth-clients.json` and `mcp-sessions.json`;
  `tests/protocol.rs` checks neither holds a routed URL.
- **Backends are traits.** The router, launcher, rules store and activity
  log are other crates' work; `backend.rs` holds only the calls the tools
  need, `fake.rs` the in-memory stand-ins. Narrow a trait rather than widen
  it. Backend calls run on blocking threads.
- **Tests run a real server.** `tests/` start one on port 0 with the fakes and
  drive it with reqwest and an rmcp client. A new endpoint or tool gets a test
  there, not only a unit test.

## Differences from PwrSnap and PwrGit

| Area | PwrSnap | PwrGit | PwrRoute | Why |
| --- | --- | --- | --- | --- |
| Port | 51729 | 51731 | 51733 | The design's 51731 is a placeholder and is PwrGit's port. |
| Scopes | capture permissions | repository permissions | `links.resolve`, `links.open`, `browsers.read`, `rules.read`, `rules.write`, `activity.hosts`, `activity.urls` | The brief's role matrix, one scope per permission, as both siblings do. |
| Client store | memory, 64, kept on a grant | file, 256 | file, 256, evicts the oldest client with no active session | PwrGit's: a registration survives a restart. |
| Token storage | hash in keychain-encrypted store | hash in a 0600 JSON file | hash in a 0600 JSON file | A hash of a 256-bit token reveals nothing; no keychain dependency here. |
| Token format | `<grantId>:pws_local_…` | `pgmcp_…` | `prmcp_…` (32 random bytes) | PwrGit's single opaque token. |
| `/revoke` | grant id and client | full token and client | full token and client | PwrSnap's accepts a guessed grant id without the secret. |
| Redirect URI at `/authorize` | required | defaults to the only registered one | required | PwrSnap's: one fewer implicit rule. |
| PKCE verifier | not length-checked | not length-checked | 43–128 unreserved characters | RFC 7636. |
| Scopes no role fits | prompt with nothing to pick | prompt with nothing to pick | `invalid_scope` before any prompt | Saves an empty window. |
| Session cap | none | 256, revoked count | 256, the oldest revoked one makes room | Revoked sessions should not lock out new ones. |
| Usage | `lastUsedAt` every 60 s; SQLite budgets | none | `lastUsedAt` every 60 s; call count in memory | Settings shows both; budgets have no meaning for routing. |
| Audit | sensitive actions with subject ids, 500 kept | none | every tool call: tool, session, start, duration, outcome; `AuditSink` | The task's privacy rule: no subject, because the subject is a URL. |
| MCP body | 1 MiB | 64 KiB | 1 MiB | A rules patch can be large. |
| In-flight MCP requests | unlimited | 16 per session, 16 sessions | 32 | One cap; requests are short. |

Unchanged from both: issuer and resource URLs, metadata documents (path-
suffixed protected-resource metadata only), the 401 challenge, public clients
only, S256 only, `resource` required and exact, 5-minute approvals and codes,
64 pending of each, codes not burnt by a failed check, no refresh tokens or
expiry, stateless JSON transport with no `Mcp-Session-Id`, `Cache-Control:
no-store`, and the SDK's rate limits (global, as they are in effect on
loopback). PwrGit's cap of 8 open approval windows is kept too.

Not here yet: custom roles, the `status://router` resource, and a real
router, launcher, rules store and activity log behind the traits.
