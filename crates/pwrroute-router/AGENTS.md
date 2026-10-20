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
  only, `None`: counted only) on the dispatch thread, before the buffer, the
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
- **`Resolved::privacy` is the seam to the engine.** `impl Resolver for
  Engine` sets it; a test resolver sets it directly. Until the engine's
  `Browser` carries the rule's level it is `Privacy::Full` there.
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
