# pwrroute-mcp

The local-agent door: MCP served by the running app over a per-user local
link, reached through `pwrroute mcp`, with PwrRoute's own approval window.
Tools are the brief's (`PwrRoute - Name and Brief` 2d); the pane is Settings
› Local agents (`PwrRoute Settings` 1a).

```sh
claude mcp add --scope user pwrroute -- pwrroute mcp
codex mcp add pwrroute -- pwrroute mcp
```

The shape follows disktree's `disktree mcp` (a stdio command an MCP client
launches, a same-user socket to the app), not PwrSnap's and PwrGit's
loopback HTTP with OAuth: Claude Code will not run OAuth against a plain
`http://` server, even on loopback. The design still describes the HTTP
door; this crate supersedes that part of it.

## Rules

- **The proxy is a byte pipe.** `proxy.rs` copies stdio to and from the link
  and nothing else; the app runs the one MCP server (`server.rs`, one rmcp
  session per connection). Do not give the proxy its own protocol or tools.
- **Only this user reaches the link.** Unix: a `0600` socket in
  `$XDG_RUNTIME_DIR` or `$TMPDIR`, and every connection's peer uid must match
  the socket's owner. Windows: a named pipe that refuses remote clients; the
  default pipe ACL gives other local users read access only, which cannot
  send a request. A second PwrRoute never takes over a live endpoint.
- **Only the app approves.** The first time a client name connects, the
  `Approver` shows the window; its answer is filed under that name in
  `Agents`. Nothing an agent sends can approve or pick a role. The name is
  self-reported, so a role is a guardrail against an agent overreaching,
  not a wall against a hostile local process: say so wherever roles are
  described.
- **Check the role at call time.** `tools.rs` reads the agent's standing on
  every call, so a role change, revocation or forget in Settings applies to
  the next call. Every tool stays listed; a refusal is an `isError` result
  naming the permission (`missing_permission`), or `access_denied`,
  `revoked`, `approval_pending`.
- **URLs stay in memory.** Arguments and results are never logged or
  written. `AuditRecord` has no field for them; do not add one. The only
  file is `mcp-agents.json` (names, roles, timestamps);
  `tests/protocol.rs` checks it holds no routed URL.
- **Backends are traits.** The router, launcher, rules store and activity
  log are other crates' work; `backend.rs` holds only the calls the tools
  need, `fake.rs` the in-memory stand-ins. Narrow a trait rather than widen
  it. Backend calls run on blocking threads.
- **Tests run a real server.** `tests/` start one on a private socket (a
  pipe on Windows) with the fakes, and drive it with rmcp clients over the
  link and through the `pwrroute-mcp-proxy` binary. CI runs them on macOS,
  Linux and Windows. A new tool or link behaviour gets a test there.

## Kept from PwrSnap and PwrGit

The approval is in-app only and per person, roles are named permission
sets, every tool is listed and a refusal is an `isError` result, approvals
persist owner-only and fail closed on a damaged file, a prompt expires after
5 minutes, at most 8 prompts are open, and the audit holds no content.

## Different, on purpose

| Area | Siblings | PwrRoute | Why |
| --- | --- | --- | --- |
| Transport | loopback HTTP + OAuth 2.1 | stdio proxy + per-user socket/pipe | Claude Code refuses OAuth over `http://`; disktree's door. |
| What is approved | an OAuth session, one token each | a client name, remembered | No token on a same-user link; the name is what the client sends. |
| When it asks | `/authorize` | as the client connects; its first tool call waits up to 2 minutes | MCP clients connect at start-up, before any tool call. |
| Denial | per authorization | per connection; the next connection asks again | Not remembered, so a mis-click is not permanent. |
| Revocation | per session | per name; refused without asking until Settings allows it again; *forget* asks again | Revoking must stick even though the name is self-reported. |
| Audit | PwrSnap: sensitive actions with subject ids | every call: tool, agent, connection, start, duration, outcome | No subject: here the subject is a URL. |
| Connections | per-request HTTP | 16 open at once | One cap. |

Not here yet: the app's `pwrroute mcp` subcommand (it calls
`proxy::main`), the approval window and Settings pane, custom roles, the
`status://router` resource, and the real router, launcher, rules store and
activity log behind the traits.
