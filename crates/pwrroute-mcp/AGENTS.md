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
  Before the first byte it waits for the app to listen (`DEFAULT_CONNECT_WAIT`,
  20 s at 250 ms; `PWRROUTE_MCP_CONNECT_WAIT` in seconds overrides it, and
  the tests set `0`), so an MCP client launched before PwrRoute still gets
  its tools.
- **Only this user reaches the link.** Unix: a `0600` `mcp.sock` beside the
  router's socket (`pwrroute_os::router_dir()`, the user's own directory),
  and every connection's peer uid must match the socket's owner. Never put
  it in a shared directory under a name without the user. Windows: a named pipe that refuses remote clients; the
  default pipe ACL gives other local users read access only, which cannot
  send a request. A second PwrRoute never takes over a live endpoint.
- **Only the app approves.** The first time a client name connects, the
  `Approver` shows the window; its answer is filed under that name in
  `Agents`. Nothing an agent sends can approve or pick a role. The name is
  self-reported, so a role is a guardrail against an agent overreaching,
  not a wall against a hostile local process: say so wherever roles are
  described.
- **Only a person denies.** A window that closes unanswered (its
  `Config::prompt_ttl`, 5 minutes, or the server stopping) is
  `Verdict::Expired`, not a denial: the waiting call gets
  `approval_expired`, and the next call on that connection raises the
  window again. Deny, or the app dropping the reply, is `access_denied`
  for the rest of the connection; reconnecting asks again. Settings ›
  Local agents offers Allow again and Forget on a revoked row, so the
  `revoked` message can send an agent there.
- **Check the role at call time.** `tools.rs` reads the agent's standing on
  every call, so a role change, revocation or forget in Settings applies to
  the next call. Every tool stays listed; a refusal is an `isError` result
  naming the permission (`missing_permission`), or `access_denied`,
  `revoked`, `approval_pending`.
- **URLs stay in memory.** Arguments and results are never logged or
  written. `AuditRecord` has no field for them; do not add one. The only
  file is `mcp-agents.json` (names, roles, timestamps);
  `tests/protocol.rs` checks it holds no routed URL. A config's `console`
  lines (`Trace::console`, `rules_test`'s `console`) are results like any
  other: returned to the agent, cleared by `hosts_only`, never kept.
- **The engine thread resolves with a `NativeHost`** (`engine.rs`), so a
  config's `finicky.getModifierKeys()` and `isAppRunning()` answer from
  the OS as they do in the router; a config's load-time console output
  has no result to ride in and is dropped.
- **Backends are traits.** The router, launcher, rules store and activity
  log are other crates' work; `backend.rs` holds only the calls the tools
  need, `fake.rs` the in-memory stand-ins. Narrow a trait rather than widen
  it. Backend calls run on blocking threads. `LiveRules` holds one lock
  from the revision check to the write (and across a read), so two agents
  writing at once cannot both pass the check; the loser gets a conflict.
- **Tests run a real server.** `tests/` start one on a private socket (a
  pipe on Windows) with the fakes, and drive it with rmcp clients over the
  link and through the `pwrroute-mcp-proxy` binary. CI runs them on macOS,
  Linux and Windows. A new tool or link behaviour gets a test there.

## Kept from PwrSnap and PwrGit

The approval is in-app only and per person, roles are named permission
sets, every tool is listed and a refusal is an `isError` result, approvals
persist owner-only and fail closed on a damaged file, a prompt expires after
5 minutes (here without counting as a denial), at most 8 prompts are open,
and the audit holds no content.

## Different, on purpose

| Area | Siblings | PwrRoute | Why |
| --- | --- | --- | --- |
| Transport | loopback HTTP + OAuth 2.1 | stdio proxy + per-user socket/pipe | Claude Code refuses OAuth over `http://`; disktree's door. |
| What is approved | an OAuth session, one token each | a client name, remembered | No token on a same-user link; the name is what the client sends. |
| When it asks | `/authorize` | as the client connects; its first tool call waits up to 2 minutes | MCP clients connect at start-up, before any tool call. |
| Denial | per authorization | per connection; the next connection asks again; an expired window is not one | Not remembered, so a mis-click is not permanent. |
| Revocation | per session | per name; refused without asking until Settings allows it again; *forget* asks again | Revoking must stick even though the name is self-reported. |
| Audit | PwrSnap: sensitive actions with subject ids | every call: tool, agent, connection, start, duration, outcome | No subject: here the subject is a URL. |
| Connections | per-request HTTP | 16 open at once | One cap. |

- **Backends over the real thing live in `live.rs`.** `LiveRouter` is the
  router's handle (resolve is its Test a link, open is a dispatch, activity
  is its buffer and journal), `LiveBrowsers` the catalog over an `Env`,
  `LiveRules` `rules.json` on disk with a content-hash revision.
  `EngineTester` runs the saved examples and tests on an engine of its own
  that follows the same config choice and `rules.json` as the router's
  resolver. The app wires them in `crates/pwrroute-app/src/mcp.rs`.
- **Apply refuses, it does not warn.** `pwrroute_rules_apply` runs the
  saved examples and tests before and after the patch; a case that passed
  and now fails is a regression, and the write is refused with the list
  unless `force` is true. The audit's `note` says `refused` or `forced` in
  fixed words. Keep that: an agent treats a warning as a note.
- **Activity is already reduced.** Entries come from the router at their
  rule's level (`privacy`); a `none` entry has no trace and no `via`. The
  tool adds only the role's cut (`hosts_only`) and the filters. The journal
  is a separate `source` behind `activity.journal`, Editor only.

Not here yet: custom roles and the `status://router` resource.
