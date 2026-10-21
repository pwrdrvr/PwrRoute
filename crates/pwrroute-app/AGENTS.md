# pwrroute-app

The GPUI shell: the main window, the tray, the first-run window and the
resident router's hand-off. `main.rs` only wires them; put new surfaces in
their own modules.

- **Fixtures only for tests and the `screens` example.** The Rules screen
  reads and writes the real `rules.json` through `rules_store.rs` (tests
  write `fixtures/rules.json` to a scratch directory); Activity draws the
  router's real records; first run is built from `onboarding.rs` facts;
  Test a link starts empty, and its app picker offers the apps the rules
  name plus `fixtures::OPENERS` under an "Examples" heading (their order
  is fixed: Activity's re-test picks by position). The tray follows the
  feed and the OS; its pause target is `rules.json`'s fallback, read once
  at launch. Settings is live: `settings_live.rs` builds every pane from
  its producer (`settings/*_host.rs`, `mcp.rs` for Local agents) and
  `settings::fixtures::model()` draws only the `screens` example and the
  window's tests (see `settings/AGENTS.md`). Never a real URL, Activity
  entry or trace, in code or in a screenshot: `fixtures::activity()` and
  `FirstRunModel::fixture` exist for tests.
- **One browser scan (`browser_scan.rs`).** The machine's browsers and
  their profiles are read once, the first time a picker or Settings asks,
  and again only on Settings › Browsers' Rescan; `PwrRoute::browsers`
  (Activity's Add a rule sheet) and the Browsers pane read the same scan.
  `settings.json`'s `browsers.hidden` (bundle ids, or names where there is
  none) keeps apps out of the pickers; rules that name a hidden app still
  resolve. `browser_scan::install_fixture` gives a test a scan; first run
  (`onboarding.rs`) still scans for itself, since it runs before the window.
- **Activity is live (`activity.rs`).** `activity::install` mirrors the
  router's `DispatchRecord`s into a `Feed` entity and keeps it current from
  `Resident::dispatched`; the window's `Activity` and the tray observe it.
  Rows, today's counts and Clear all go through the feed, never around it
  (`Feed::reload` takes the router's buffer back after Settings changes the
  retention or clears the journal). A record is a row at `Full` or `Host`
  (the host-only chip) and a count at `None`; the filters are All,
  Unrouted (the default or the fallback took it), Failed, Rewritten and
  Private, each with its count. The retention sentence under the list and
  the toolbar's note come from `RouterHandle::retention()`, never from a
  literal here. With the journal on, journaled records no longer in memory
  are listed below a "From the journal" divider (`Feed::journal`, at most
  `JOURNAL_ROWS`, read from disk only when the files' `JournalStamp`
  changed since the last read: `JournalCache`). Export… goes through
  `settings::privacy_host::export`, the OS save dialog and nothing else.
  Full URLs stay in that entity and the router's buffer; nothing in the app
  writes one to disk or a log. The row's "⋯" opens `activity::row_menu`
  and the expanded row's "Add a rule…" calls `PwrRoute::add_rule`, both in
  `activity_actions.rs`; a `RowKey` names the row, and `Feed::record` reads
  a journaled row's record back from the journal.
- **Settings persist in `settings.json` (`settings_store.rs`).** The data
  directory's `settings.json` holds Activity's retention and the journal
  (`activity.keep`, `activity.journal`, `activity.journalDays`) and the
  hidden browsers (`browsers.hidden`); an absent file is the defaults, and
  a bad one is reported and ignored. `router.rs`
  reads it before the router spawns, so `Options::retention` and
  `Options::journal` (`<data dir>/journal`) are in force from the first
  link; `settings_store::install` then makes the same store and the
  router's handle the globals Settings › Privacy edits, saving first and
  applying through `set_retention` / `set_journal`. The file carries
  numbers and a level, never a link.
- **The Rules screen follows `rules.json`'s other writers.** The store
  (`rules_store.rs`) stamps the bytes it last read or wrote, and
  `RulesStore::check_disk` compares the file to them: changed with nothing
  unsaved, it reads the file again (`Status::Reloaded`, "reloaded 09:41");
  changed under unsaved edits, it keeps them, refuses the next save
  (`Status::Changed`) and the rail foot offers Reload (drop the edits) and
  Overwrite (write them anyway). Its own saves restamp, so they are never
  a change. `PwrRoute::sync_with_disk` runs the check when the window
  comes to the front (`watch_activation`), on every dispatch, before the
  Add a rule sheet and Test a link read the store, at the top of `refresh`,
  and from `mcp::follow_rules_writes` right after an in-process
  `pwrroute_rules_apply` the audit reports as written. Never write the
  file around the store from the app; agents and editors go through the
  check instead.
- **Examples and tests are the only URLs that reach disk, and only by
  hand.** `rules_store.rs` writes a rule's `examples`, a rewrite's
  `url`/`becomes` pairs, `rules[].private` / `rules[].privacy` and the
  top-level `tests[]` with the rest of `rules.json`; every control that adds
  one (the editor's Add example, Test a link's Save as test, Activity's Add a
  rule / Save as example / Save as test in the row's "⋯" menu,
  `activity_actions.rs`) says so in its copy. `try_it::TestRuns` runs them through an engine built from the
  config in charge and the edited rules (the same sources as the router's
  resolver, so indexes match the rows) at window open, after every debounced
  save and on Run tests; `pwrroute rules test` in `main.rs` is the CLI form.
  Activity records report a `rules.json` handler as `Matched::Unreported`,
  so Save as example resolves the link again through `Probe` at click time.
- **Local agents (`mcp.rs`).** `mcp::install` serves MCP from
  `pwrroute_mcp::McpHost` on `<data dir>/mcp.sock`, over the router's
  handle and an engine that follows the same files; `pwrroute mcp` in
  `main.rs` is the stdio proxy and runs before anything else. Prompts
  become `ApprovalWindow`s on the GPUI thread; closing one denies. The
  pane's "Recent agent actions" is the audit: tool, agent, time, outcome,
  never an argument. The switch is `mcp.json`; approvals are
  `mcp-agents.json`.
- **Tray (`tray/`).** `tray-icon` owns the OS item and its native menu
  (design 1c). On macOS a left click opens `tray/dropdown.rs`, a GPUI
  `PopUp` window (design 1a/1b); recents there are hosts only
  (`model::host_of`), from the feed. Pausing updates `tray::model(cx)` and
  whoever routes observes it.
- **The dropdown cannot move or grow from the top.** GPUI has no window
  move and macOS resizes from the bottom edge, so the window opens at
  `window_height` (the tallest the card can be) with a transparent
  background. Keep the metric constants in `dropdown.rs` matched to the
  layout; `tray/tests.rs` holds `card_height` to the laid-out card.
- **Dock (`dock.rs`, `macos.rs`).** The Dock icon shows while the main,
  Settings or first-run window is open (`Regular`) and goes when the last closes
  (`Accessory`). objc2's AppKit calls here are safe; this crate stays
  `unsafe`-free.
- **First run never touches the OS.** `FirstRunHooks` decides what its
  buttons do; `onboarding.rs` builds the real ones and the model from
  `Facts` (tests write their own facts and count hook calls). `state.json`
  in the data directory says whether first run already ran. "Open
  PwrRoute" (every step done) and "Later" mark it done and open the main
  window on Rules; closing the window only marks it done.
  `default_browser.rs` is the only caller of
  `pwrroute_os::set_default_browser`, from a click; its `Os` struct takes
  fakes, so tests never reach the OS. After a request it re-reads the
  status every second for a minute (`POLL_EVERY`, `POLL_FOR`) and on
  window activation; first run's step 3 shows the wait and "Ask again".
  Never wire registration into a test or a launch path that runs without
  a click.
- **Manual checks on macOS:** `--dry-run --first-run` opens the first-run
  window; `PWRROUTE_TRAY_OPEN=1` opens the dropdown at launch (the item can
  sit behind the notch on a full menu bar); `PWRROUTE_TRAY_FIXTURE=routing`
  or `=paused` draws the design's tray fixture instead of the feed. A second
  router on the machine takes the launch: give the build its own `HOME`
  while another one runs.
