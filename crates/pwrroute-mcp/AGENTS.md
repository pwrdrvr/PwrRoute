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
- **Only a person denies.** A window has no time limit: it stays up
  while any connection under its name holds a claim on it (`Waiting`,
  kept in the connection), and closes when the last one goes away or
  the server stops. That is `Verdict::Closed`, not a denial: a waiting
  call gets `approval_closed`, and the next connection raises the window
  again. A tool call still gives up after `approval_wait` with
  `approval_pending`, leaving the window up. Deny, or the app dropping
  the reply, is `access_denied` for the rest of the connection;
  reconnecting asks again. Settings ›
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
persist owner-only and fail closed on a damaged file, at most 8 prompts are
open, and the audit holds no content.

## Different, on purpose

| Area | Siblings | PwrRoute | Why |
| --- | --- | --- | --- |
| Transport | loopback HTTP + OAuth 2.1 | stdio proxy + per-user socket/pipe | Claude Code refuses OAuth over `http://`; disktree's door. |
| What is approved | an OAuth session, one token each | a client name, remembered | No token on a same-user link; the name is what the client sends. |
| When it asks | `/authorize` | as the client connects; its first tool call waits up to 2 minutes | MCP clients connect at start-up, before any tool call. |
| Denial | per authorization | per connection; the next connection asks again; a window closed unanswered is not one | Not remembered, so a mis-click is not permanent. |
| Prompt lifetime | 5 minutes, the OAuth transaction's | while a connection under the name is open | The link is long-lived and its end is seen at once; an OAuth login has no such signal. |
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
- **Apply refuses, it does not warn.** A `precedence` other than
  `"pwrroute"`, `"config"` or `null` is `invalid_rules` (the file loader
  reads anything else as the config's order). `pwrroute_rules_apply` runs the
  saved examples and tests before and after the patch; a case that passed
  and now fails is a regression, and the write is refused with the list
  unless `force` is true. The audit's `note` says `refused` or `forced` in
  fixed words. Keep that: an agent treats a warning as a note.
- **A trace says who decided.** Each `TraceStep` names its list and
  number in `summary` ("rules.json rule 3 matched", "config handler 2
  matched") and in `source` (`rules` or `config`, absent when the record
  does not say), and a default names its owner as the app's views do:
  "PwrRoute's default" (`rules.json`'s `defaultBrowser`) or the config's by
  its shown path ("~/.finicky.ts's default"; "the config's default" when
  the caller cannot say which). `LiveRouter::with_config` asks the app for
  that path at each call; `EngineResolver` reads its engine's. A record
  that did not number its rule (`Matched::Unreported`) is never called a
  `rules.json` rule. `engine.rs` (`summary`, `default_config`) and
  `live.rs` (`trace_of`) share the default wording. A pause's or a
  send's step (`held_step`) names what it passed over the same way; a
  send says "the user sent this link (next link 1 of 3)". There is no
  tool to send links yet: it would need its own permission.
- **Not opened is said, not inferred.** A link a rule or default kept
  from opening (`rules.json`'s browser `"none"`, Don't open; a config's
  `browser: null`) or the user held is `outcome: "notOpened"`; its entry's
  `reason` (`rule`, `default`, `hold`) is the router's
  `not_opened_reason()`, and the deciding step's `summary` ends with
  `engine::NOT_OPENED` in both `trace_of`s. `filter: "notOpened"` lists
  them. `pwrroute_rules_apply` takes `"none"` as any browser.
- **Activity is already reduced.** Entries come from the router at their
  rule's level (`privacy`); a `none` entry has no trace and no `via`. The
  tool adds only the role's cut (`hosts_only`) and the filters. The journal
  is a separate `source` behind `activity.journal`, Editor only.

Not here yet: custom roles and the `status://router` resource.
