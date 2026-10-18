# pwrroute-router

The resident router: every link goes normalize → resolve → launch → record
here. Pure Rust, no OS calls beyond files and a local socket, so every test
runs on every platform.

- **One dispatch thread owns the resolver.** The engine (QuickJS) is not
  `Send`; `Router::spawn` builds it on the thread from a `ResolverFactory`.
  Everything else talks through `RouterHandle`.
- **`Launcher` is the seam to pwrroute-browsers.** Keep it small. Tests use
  `DryRunLauncher`; nothing in this crate opens a browser.
- **URLs never touch disk.** `DispatchRecord` holds full URLs in memory only;
  `DispatchRecord::timing()` is the only view meant for logs, and a test
  checks it carries no link data. Error and step text can quote URLs, so it
  stays in memory too.
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
