# pwrroute-router

The resident router: every link goes normalize → resolve → launch → record
here. Pure Rust, no OS calls beyond files and a local socket, so every test
runs on every platform.

- **One dispatch thread owns the resolver.** The engine (QuickJS) is not
  `Send`; `Router::spawn` builds it on the thread from a `ResolverFactory`.
  Everything else talks through `RouterHandle`.
- **`Launcher` is the seam to pwrroute-browsers.** Keep it small. Tests use
  `DryRunLauncher`; nothing in this crate opens a browser.
  `launch_observed` optionally reports fixed labels, counts and timings
  during an attempt. The router logs each at Info with the dispatch id and
  attempt number, gated by `diagnostic_log` and retention as dispatch lines
  are. These messages must carry no URL, command argument, browser/profile
  name or arbitrary error text, even for a counted-only link. A boxed
  launcher must forward the observer.
- **A link is never dropped for want of a browser.** `Worker::launch` tries
  the fallback browser once when the chosen browser fails to open, traced
  as `LaunchFailed`, `Fallback`, `Launched`, keeping the chosen browser's
  `private`; the record's `browser` is the one that opened the link.
  `Outcome::Failed` means both failed, or the chosen browser was the
  fallback already.
- **The fallback is asked for, not stored.** `Options::fallback` is a
  `FallbackSource` the router calls each time a link falls back (the app
  reads `rules.json`'s `defaultBrowser` and `defaultProfile`), so a choice
  saved while the router runs applies to the next link. `FallbackSource::get`
  turns an empty answer, or one naming PwrRoute, into `FALLBACK_BROWSER`;
  callers do not repeat that check.
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
  and `DispatchRecord::log_line()` are the only other views meant for logs;
  tests check they carry no link data beyond their level. With
  `Options::diagnostic_log` set, the dispatch thread logs `log_line()` for
  each reduced record to the `log` facade (target `pwrroute::link`, a
  warning past `SLOW_DISPATCH`), but only while links are kept in memory or
  the journal. `Options::on_received` fires on the handing-over thread with
  each link's id and receipt instant (the app times the hand-off to the
  browser from it). `Options::retention_description()` is the sentence the
  UI and MCP show about all this, the log included; keep it true.
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
- **Pause is a dispatch-time swap, not a resolver.**
  `RouterHandle::set_paused(Some(PauseTarget))` is read once per link in
  `Worker::process`, after `resolve`: the rules have run, so the rewrites,
  the rule steps and the record's privacy level stand; `Step::Paused {
  skipped }` names the config handler that matched (a `rules.json` rule
  passed over is the record's `Matched::Rule`, with `skipped: None`), and
  the target replaces the browser, a `browser: null` or `private` handler
  included.
  Test a link ignores the pause. Keep it out of `launch` and `fallback()`.
- **Send links shares the pause's slot.** `RouterHandle::set_send_to(Some(
  SendTo))` sends links to a user's `Destination` (browser, profile,
  private) for `SendScope::Next { left, of }` or `All { until }`, applied
  where a pause is and traced as `Step::Sent { sent, skipped }`. One
  `Hold` slot holds a pause or a send: setting one ends the other, and
  clearing one (`None`) leaves the other alone. A `Next` is counted in
  `Worker::take_hold`, under the slot's lock, once per dispatched link
  (forwarded and agent links included, a rejected link or a test not),
  and the slot clears when it reaches zero; an `All` past `until` clears
  without taking the link, and `send_to()` never returns one. A private
  destination keeps the record at most `Host`. `retarget_send` and
  `add_to_next` change a send in force under the same lock. Memory only:
  nothing about a send is saved.
- **Short links expand on the dispatch thread, before the resolver.**
  `Worker::expand` reads `Resolver::short_links()` (the engine's list, as
  the config in charge set it), then follows a short link through
  `ShortLinks::expand` with `Options::network` (`HttpNetwork` built on the
  thread the first time it is needed; tests pass a fake). That request to
  the shortener's host is the only place a link leaves the machine before
  a rule sees it, as under Finicky. The resolver then gets the expanded
  `pwrroute_engine::Link`, so a config still sees `originalUrl`, and the
  record carries `Step::Expanded` with where it landed, reduced like every
  other URL in it. A resolver that borrows an engine answers
  `short_links()` with a clone and releases the borrow first; never expand
  inside it.
- **The trace is the product.** Activity, Test a link and the MCP tool all
  read `DispatchRecord`. The engine names the rules that fired through
  `Engine::trace`, which becomes `Resolved::rule_steps`; report more that way
  rather than by changing `Step`. Who decided comes from the trace step's
  source, never from comparing browsers: `Matched::Handler` is a config
  handler, `Matched::Rule` a `rules.json` rule (file position), and
  `Matched::Default`'s owner is `Config` or `PwrRoute`, whichever order
  `rules.json`'s `precedence` ran them in (`dispatch::rule_steps`).
- **IPC:** one JSON line each way per connection (`ipc.rs`). Bump
  `PROTOCOL_VERSION` on any incompatible change; a mismatched launch routes
  its own links rather than lose them.
- **Single instance:** the file lock decides who is primary, never the
  socket. Tests use their own temp endpoint; never the real `router_dir()`.
- `tests/forwarding.rs` re-runs its own test binary as the second process.
