# pwrroute-router

The resident router: every link goes normalize → resolve → launch → record
here. Pure Rust, no OS calls beyond files and a local socket, so every test
runs on every platform.

- **One dispatch thread owns the resolver.** The engine (QuickJS) is not
  `Send`; `Router::spawn` builds it on the thread from a `ResolverFactory`.
  Everything else talks through `RouterHandle`.
- **`Launcher` is the seam to pwrroute-browsers.** Keep it small. Tests use
  `DryRunLauncher`; nothing in this crate opens a browser.
- **URLs never touch disk unless the journal is on, and then only at the
  record's privacy level.** Each `DispatchRecord` is reduced to the
  `Privacy` of the rule that routed it (`Full`, `Host`: scheme and host
  only, no error text and no opener window title, `None`: counted only)
  on the dispatch thread, before the buffer, the
  journal, `on_dispatch` or a reply sees it; a link no rule routed stays
  `Full`. The buffer (`recent.rs`) is memory only, bounded by
  `Options::retention` (count and age, swept on push and read). The journal
  (`journal.rs`) is off unless `Options::journal` is set; on, it appends one
  JSON line per dispatch to `<dir>/activity-YYYY-MM-DD.jsonl` (UTC days,
  `0600`), capped at the lesser of the record's level and `JournalMode`, and
  never with error text, which can quote a URL. `DispatchRecord::timing()`
  is the only other view meant for logs; tests check both carry no link data
  beyond their level. `Options::retention_description()` is the sentence the
  UI and MCP show about all this; keep it true.
- **`Privacy` is the engine's type.** `trace.rs` re-exports
  `pwrroute_engine::Privacy` (serialized `full`/`host`/`none`) and adds what
  the router needs through `PrivacyExt` (`at_most`, `label`), written in
  terms of the engine's `tighter()` so the two crates cannot disagree on
  which level is tighter. Do not define a second level type here.
- **`Resolved::privacy` is the seam to the engine.** `impl Resolver for
  Engine` copies `trace.resolution.browser.privacy`, the tightest level of
  the `rules.json` rewrites that matched and the handler that won (a private
  rule with no level is `Host`); a test resolver sets it directly.
  `dispatch/tests.rs` proves the path with a `rules.json`-only engine and
  the `DryRunLauncher`; keep those tests when the seam changes.
- **The trace is the product.** Activity, Test a link and the MCP tool all
  read `DispatchRecord`. The engine names the rules that fired through
  `Engine::trace`, which becomes `Resolved::rule_steps`; report more that way
  rather than by changing `Step`.
- **IPC:** one JSON line each way per connection (`ipc.rs`). Bump
  `PROTOCOL_VERSION` on any incompatible change; a mismatched launch routes
  its own links rather than lose them.
- **Single instance:** the file lock decides who is primary, never the
  socket. Tests use their own temp endpoint; never the real `router_dir()`.
- `tests/forwarding.rs` re-runs its own test binary as the second process.
