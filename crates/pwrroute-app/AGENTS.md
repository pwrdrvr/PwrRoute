# pwrroute-app

The GPUI shell: the main window, the tray, the first-run window and the
resident router's hand-off. `main.rs` only wires them; put new surfaces in
their own modules.

- **New windows open where the user is.** Every `open_window` passes the
  display from `placement::display_for_new_window` (PwrRoute's focused
  window's display, else the one under the pointer, else primary) both to
  `Bounds::centered` and as `WindowOptions::display_id`; GPUI opens on the
  primary display without it. `NSScreen.mainScreen` is not used: with no
  key window of its own (the tray, a link, an agent's request) the app gets
  the primary display from it, not the one the user is on. A window that is
  already open stays put. The tray dropdown uses its menu bar item's display.
- **Fixtures only for tests and the `screens` example.** The Rules screen
  reads and writes the real `rules.json` through `rules_store.rs` (tests
  write `fixtures/rules.json` to a scratch directory); Activity draws the
  router's real records; first run is built from `onboarding.rs` facts;
  Test a link starts empty, and its app picker offers the apps the rules
  name plus `fixtures::OPENERS` under an "Examples" heading (their order
  is fixed: Activity's re-test picks by position). The tray follows the
  feed, the OS and the router's pause (`tray/`, below). Settings is live:
  `settings_live.rs` builds every pane from
  its producer (`settings/*_host.rs`, `mcp.rs` for Local agents) and
  `settings::fixtures::model()` draws only the `screens` example and the
  window's tests (see `settings/AGENTS.md`). Never a real URL, Activity
  entry or trace, in code or in a screenshot: `fixtures::activity()` and
  `FirstRunModel::fixture` exist for tests.
- **Every engine that resolves a link gets a `NativeHost`.** The router
  (`router.rs`, injectable for tests), the window's config engine
  (`app.rs`), Test a link, the Rules screen's `try_it` runs and `pwrroute
  rules test` all answer `finicky.getModifierKeys()`, `isAppRunning()` and
  the rest from the OS, as the Go host did; `DefaultHost` stays only where
  a config is loaded to count or describe its rules. A run's `console`
  output comes back in the engine's trace: Test a link shows it as a
  "Config output" row, `rules test` prints it to stdout, and the router
  drops it with the trace. Never write those lines anywhere.
- **One browser scan (`browser_scan.rs`).** The machine's browsers and
  their profiles are read once, the first time a picker or Settings asks,
  and again only on Settings › Browsers' Rescan; `PwrRoute::browsers`
  (Activity's Add a rule sheet) and the Browsers pane read the same scan.
  `settings.json`'s `browsers.hidden` (bundle ids, or names where there is
  none) keeps apps out of the pickers; rules that name a hidden app still
  resolve. `browser_scan::install_fixture` gives a test a scan; first run
  (`onboarding.rs`) reads the same scan, and its "Look again" (no browsers
  found) is a rescan.
- **Activity is live (`activity.rs`).** `activity::install` mirrors the
  router's `DispatchRecord`s into a `Feed` entity and keeps it current from
  `Resident::dispatched`; the window's `Activity` and the tray observe it.
  Rows, today's counts and Clear all go through the feed, never around it
  (`Feed::reload` takes the router's buffer back after Settings changes the
  retention or clears the journal). A record is a row at `Full` or `Host`
  (the host-only chip) and a count at `None`; the filters are All,
  Everything else (`Filter::Unrouted`: a default, PwrRoute's or the
  config's, or the fallback took it), Failed, Rewritten and Private, each
  with its count. The Rule
  column says who routed a link: "rule 3" (`rules.json`'s, `Matched::Rule`),
  "cfg h 2" (the config's handler), "default" (PwrRoute's) or "config
  default" (`Matched::Default`'s owner; a record from before owners is
  plain "default"), "fallback"; the expanded row and the tray's recents
  ("config handler 2") say the same in full, and the expanded row names a
  default by its owner as Test a link and the Overview do ("PwrRoute's
  default", "~/.finicky.ts's default": the window passes the config's
  shown path through `Activity::set_config`). Add a rule's collision
  caption counts which caught links would change hands by that and the
  order rules run in (`RulesStore::precedence`, the config in charge, Top
  or Bottom); a `Matched::Unreported` record is `Took::Unknown`, never
  counted as moved unless the new rule runs before every handler. The
  retention sentence under the list and
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
  a journaled row's record back from the journal. The feed also keeps
  today's `Tally` (links per rule, handler and default owner, and per
  browser and profile), reset with the day, for the Overview's map.
- **Overview is where every window lands (`overview_model.rs`,
  `overview.rs`).** `Screen::Overview` is the default and the rail's first
  item; Rules opens as a list with nothing selected (a click or Down
  selects). `overview_model::build` is plain data over the edited
  `rules.json`, the config's `describe()`, the routing `Status` (config
  error > paused > not default > unknown > routing), the scan
  (`PwrRoute::installed`; a target is drawn missing only once the scan ran)
  and the feed's counts and `Tally`. The map is a graph from rows to
  profile nodes: one card per browser with a node per profile (routed
  ones by their first row, then the scan's others; a profile a rule names
  by its directory, Chromium's "Profile 1", is the scan's profile of that
  directory, `Installed::directories`), every installed browser
  `browsers.hidden` does not hide (never PwrRoute) under "Also installed"
  when no enabled rule or Everything else routes there, idle profiles
  folded into "+N more profiles" (`OverviewState::expanded`, by card key,
  for the session). A browser whose catalog entry can open a private
  window (`BrowserInfo::private_label`: "Incognito", "InPrivate", "Private
  window"; decided by `pwrroute_browsers::private_window`, the launcher's
  own lookup by the name as written and its `AppType`, never by the
  scan's bundle id) ends its card with one private-window node, after the fold,
  never folded or counted in it (`Card::visible`, `FocusKey::Private`):
  a `"private": true` rule, a handler or `defaultBrowser` whose `args`
  carry a private flag (`pwrroute_engine::is_private_flag`) routes there
  whatever profile it names, and today's private links count there
  (`Tally` keys on `Browser::private` where `private_window` says one
  opened); a browser without one keeps the route
  on its profile and the row says "not private", except that a config's
  private flag to a browser the catalog does not know goes to it as
  written, so that card gets a private node by the flag's word. The columns stop at
  their caps (`MapWidths::of_content`: destinations 266-420, rules up to
  720) and every pixel past them is the routes' gutter. What is hovered, pinned or keyed is kept as a
  `FocusKey` (a row's `RowSource`, a card's key and profile), never an
  index: the cards re-sort as links come in, so each frame
  `Map::resolve`s it, and a scroll or a reload of `rules.json` lets go of
  the pointer's. `Map::lit` is the pure trace model: for the focus (pin, else
  keyboard, else pointer) which rows, cards and nodes light, and every
  route with its `Trace` and whether it is running (dashed: config never
  loaded, not installed, paused, or a rule that is off, drawn only while
  traced, outside the fan), fanned into its node's port in row order;
  there is no cap on routes. `overview.rs` draws it: rows and each card's nodes record their
  bounds into `MapBounds` while they prepaint, and one canvas paints the
  dot grid and every route (rest first, lit on top) from those bounds, so
  a hover only notifies. A row click pins (Edit ›, a double-click or
  Return opens Rules); Esc, Clear or the empty ground lets go. A ƒ row
  reads `function_excerpt` of the function's source, a guard's condition
  when it returns a literal; a `finicky.matchHostnames` function reads as
  its hostnames (`FunctionSpec::hostnames`, from the engine). An idle
  node's "+ Add a rule…" (its click, or Return) is
  `PwrRoute::add_rule_for`, with a private window on for a private-window
  node; it writes the card's name, so a private node whose word came only
  from a route naming the browser another way does not offer it
  (`Node::addable`). The callout names both defaults as the map names a node (by
  profile when the browser is the same) and says so plainly when they
  are the same browser and profile. The setup card writes
  `defaultBrowser`/`defaultProfile`, and the callout writes `precedence`
  (`"pwrroute"`, with Undo), both through `PwrRoute::write_now` (as the
  Rules banner's switch does: sync, refuse as the store would, save now,
  take the change back when the write fails), so a change that was not
  saved never shows as made: the card keeps its state and says why
  (`OverviewState::refused`).
  `overview::status` reads the router, the tray's pause and the OS; a test
  or screenshot sets `OverviewState::status` instead. The strip chips and
  the rail foot use the same reading. A config the router never loaded
  (`RoutingSource::config_not_loaded`) is `ConfigTrouble::NotLoaded`: the
  router leaves it out and routes with `rules.json` alone, so its section
  says "not running" with one error row, the rules say "run without it"
  with their counts, and PwrRoute's default takes everything else. Only
  when nothing loaded at all (`RoutingSource::fallback` too) is it
  `NothingLoaded`: every link goes to the fallback browser, so the map
  marks both sections "not running" and fades their rows and the cards
  only they feed (`Row::idle`). A failed reload (`error` alone)
  is `ReloadFailed`, the warning tone: the last load still routes, config
  and `rules.json` together, so the headers say "as last loaded", no row
  or Rules list count is shown by number, the switches say a change
  applies once it reloads, and Save as example probes instead of trusting
  a record's rule number. Everything else's owner is the engine's
  (`RulesFile::owns_default`), never the precedence alone. Rules
  and Test a link reuse Overview's source chip. Leaving Overview
  (`OverviewState::leave`) drops its fades and the callout's hand-off.
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
  `activity_actions.rs`) says so in its copy. `try_it::TestRuns` runs them
  through an engine built from the config in charge and the edited rules
  (the same sources as the router's resolver, so indexes match the rows)
  at window open, after every debounced save and on Run tests; `pwrroute
  rules test` in `main.rs` is the CLI form.
  Activity records carry `Matched::Rule { index }` for a `rules.json`
  rule (older records `Matched::Unreported`).
  Save as example takes the rule from the record (`Matched::Rule`) only
  while `rules.json` is unchanged since the link was routed (the store's
  `Status` is `Saved`/`Reloaded` no later than `received_at`, after a
  `sync_with_disk`) and that rule is on and opening what the record
  opened; otherwise (a record from before rule numbers, a file saved,
  reloaded or edited since) it resolves the link again through `Probe` at
  click time, since an inserted or moved rule may now hold that number.
- **Who decides is the trace's to say (`try_it.rs`, `trace_view.rs`).**
  Test a link, Try it and the test runs resolve over the config in charge
  and `rules.json` together, in the order `rules.json`'s `precedence` sets
  (the window's config-only engine is reused only while `rules.json` is
  empty). Beside a config, Test a link's last step names the default's
  owner from the default step's source: "Everything else · PwrRoute's
  default" or "Everything else · ~/.finicky.ts's default", with the
  config's shown path, never a literal (the owner wraps to its own muted
  line, without the "·"); with `rules.json` alone both views say plain
  "Everything else". Try it reads the trace, not the browsers: under
  the Go order a config handler that wins took the link before the rule
  ran, so it never says "Move this rule above it" then; it offers letting
  PwrRoute's default take everything else, which runs the rules first. A
  handler that matched but whose browser failed "matched but its browser
  failed", never "threw". Add a rule's check (`activity_actions.rs`)
  blames the config's order only under the Go order; under `"pwrroute"` a
  config handler winning means the new rule's pattern missed, and the
  sheet's line about the config (`config_hint`) and the Added note say
  which runs first. One rule has one name on every surface: `rules.json`'s
  `rules[i]` is "rule N" (Rules' strip, failures and editor button, Try
  it, Test a link's steps, the Overview's tooltips, Activity, the tray,
  the importer, Settings › Privacy, `pwrroute rules test`, MCP), the
  config's `handlers[i]` "config handler N" ("Config handler 2 of
  ~/.finicky.ts" in the Add rule sheet; "cfg h N" only in Activity's
  narrow column).
- **The Finicky importer (`importer.rs`, `importer_view.rs`).** The model
  is plain data over `Engine::review_import` (verdicts, source lines,
  probed suggestions; recent links come from the feed's `Full` records, in
  memory); the window draws Review, Place and Choose source from it. Import
  goes through `PwrRoute::apply_import`: one `RulesStore` save over the
  file as it stands, the previous file kept for Undo import until the next
  edit; a save the store would refuse writes nothing and reopens the
  importer with the reason. Choose source is about the config in charge
  (`importer::Charge`, from `router::config_in_charge` at open), which
  need not be the file under review: Keep never changes `settings.json`'s
  `source`, and only Visual rules only turns the config in charge off
  (`ImportModel::source_after`: `visualOnly`, and the imported file in
  `dismissed`). The change goes through `router::choose_source`, which the
  dispatch thread applies on its next refresh; `spawn_router` reads the
  same setting. Window tests open the importer with `open_with`, naming
  the config in charge, since the router's source is process-wide. Entry
  points: the 1b banner, the 1a notice, Settings' Review import…, File ›
  Import Finicky Config… and first run's Import N rules. First run opens
  it with `open_from_first_run`: Import writes `rules.json` without
  opening the main window (through its store only when it is already
  open), and the importer answers through its
  `OnDone`, imported with the count or cancelled however it closed (its
  release counts as a cancel, and so does a window that fails to open).
  Choose another file keeps the first-run context: the file picked opens
  for first run the same way, and no file picked is a cancel. The config
  file is only ever read.
- **Rules under a Finicky config (`config_view.rs`).** The banner's switch
  "Let PwrRoute's default take everything else" is `rules.json`'s
  `precedence` (`RulesStore::set_precedence`), written at once like any
  edit through the store, never `settings.json`; a store that refuses
  the save leaves the switch where the router has it and says why. The
  cards follow `config_view::Order` in evaluation order: the visual rules
  after the config's handlers, or before them with the config's
  `defaultBrowser` struck through and PwrRoute's Everything else row last.
  With no usable `rules.json` default (empty, or PwrRoute) the override is
  inert, as in the router: the config's default stays unstruck and the
  banner asks for one. The warnings under the switch come from the
  config's description (`ConfigInCharge::override_warnings`: a function
  or `null` default only while it is overridden, a handler matching every
  link), never from running it. Default browsers are named through
  `default_browser::browser_name`, never as bundle ids. A config this
  window cannot read is described as the router runs it (`Order::trouble`,
  from `overview::status`): one that never loaded is left out, so the
  visual rules run without it and PwrRoute's default takes everything else
  until it loads (`nothing_runs`, when `rules.json` did not load either:
  every link opens there); after a failed reload (`stale`) the version that
  last loaded still routes.
  Stop using is `importer_view::set_source` with `visualOnly`
  (and the file dismissed); Open in editor is `open_with_system`, which the
  test platform does not implement, so tests never click it. The blank
  state's Choose browser… writes `defaultBrowser` from `PwrRoute::browsers`
  and its config row opens Settings › Configuration.
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
  (`model::host_of`), from the feed. Pause routing is
  `RouterHandle::set_paused` from `tray::toggle_pause`, with the target
  `tray::pause_target` names: `rules.json`'s `defaultBrowser` and
  `defaultProfile`, else the config's own `defaultBrowser` when it is not
  a function (evaluated once per change of the file, `ConfigDefault`),
  else `FALLBACK_BROWSER`. The dropdown re-reads it on each open; a pause
  keeps the target it began with until resumed. A paused record's
  `Step::Paused` is `RecentRoute::Paused` in the recents ("paused ·
  skipped rule 2") and "skipped rule 2" (or "skipped cfg h 4"; plain
  "paused" when the record names neither) in Activity's amber Rule chip,
  with "paused · skipped rule 2" in the expanded row; which list comes
  from the record's `Matched` step, else the `Paused` step's `skipped`
  (the config's handlers).
- **The dropdown's window is its card** (design: Popovers and Overflow
  2a). A transparent window taller than the card is not invisible: macOS
  draws its frame and shadow at the window's size. It opens at
  `fitted_height` and follows the card on every model change through
  `macos::set_dropdown_height`, which keeps the top edge (GPUI's
  `Window::resize` keeps the bottom). That call runs on the foreground
  executor, never inside an update or a `defer`: AppKit reports the resize
  synchronously and GPUI drops the report while the app is borrowed. The
  window stops at the display's visible area
  (`macos::dropdown_max_height`); past it the sections between the header
  and Quit scroll. Keep the metric constants in `dropdown.rs` matched to
  the layout; `tray/tests.rs` holds `card_height` to the laid-out card and
  the window to the card. Names come from the catalog or the installed
  app, never a bundle id: `BrowserLabel::name` is what the reader sees and
  `BrowserLabel::app` what the router is told.
- **Menus and select lists use `widgets::menu_card` and `menu_row`**
  (design: Popovers and Overflow 1a-1d). A select's list is as wide as its
  field (`.absolute().left_0().right_0()`), never narrower, and scrolls
  past `MENU_MAX_HEIGHT`; a row's hint gives way before its label and ends
  in an ellipsis; a browser row's hint is `profiles_hint` (two names, then
  a count). A field's value is `field_label`, never `theme::text`, which
  does not shrink.
- **Dock (`dock.rs`, `macos.rs`).** The Dock icon shows while the main,
  Settings or first-run window is open (`Regular`) and goes when the last closes
  (`Accessory`). objc2's AppKit calls here are safe; this crate stays
  `unsafe`-free.
- **First run never touches the OS.** `FirstRunHooks` decides what its
  buttons do; `onboarding.rs` builds the real ones and the model from
  `Facts` (tests write their own facts and count hook calls). `state.json`
  in the data directory says whether first run already ran. With a Finicky
  config in charge the window has three steps (the config, Everything
  else, the OS default), else two; the body scrolls, and finished steps
  fold under "Where links go" once routing starts. Everything else is
  preselected (the config's fixed `defaultBrowser`, the OS default unless
  it is PwrRoute, the first browser found, all less the apps
  `browsers.hidden` hides) and written at once to
  `rules.json`'s `defaultBrowser` and `defaultProfile` ("" for the
  browser's own default); for a config kept in charge, its "who owns
  everything else" control writes `rules.json`'s `precedence` (shown only
  while that config loads; a broken one kept says your rules run without
  it and the Everything else browser takes the rest until it loads, since
  the router leaves it out and routes with `rules.json` alone). Both are
  direct `RulesFile` writes; the main window's store follows the file.
  Import N rules shows "Importing…" until the importer answers; Don't use
  it turns the config off through `importer_view::set_source_and_show`
  (`visualOnly`, `router::choose_source`, and an open main window shown
  the change), which `onboarding::hooks_with`
  takes as a parameter so tests never switch the process-wide source.
  "Open PwrRoute" (every step done) and "Later" mark it done and
  open the main window on Overview, on the first-run window's display
  (`onboarding::main_window_display`); closing the window only marks it
  done. `default_browser.rs` is the only caller of
  `pwrroute_os::set_default_browser`, from a click (Settings, first run's
  last step, the tray's "Make default browser…" row); its `Os` struct takes
  fakes, so tests never reach the OS. After a request it re-reads the
  status every second for a minute (`POLL_EVERY`, `POLL_FOR`) and on
  window activation. LaunchServices can still name the old browser just
  after the dialog is approved, so an activation only settles a request
  as refused `ACTIVATION_GRACE` later. While first run is open and its last
  step is not done it re-reads every second regardless (`follow_while`);
  the step shows the wait and "Ask again", names an uncatalogued default
  by its bundle's name (`pwrroute_os::app_display_name`), and says why
  when the OS refuses the request with an error.
  It also re-reads every `RECHECK_EVERY` (a minute) for the life of the
  app and when the tray dropdown opens, so a router with no window
  notices losing the default; the tray chip and its Make default row
  follow the registrar. Never wire registration into a test or a launch
  path that runs without a click.
- **Updates (`updater.rs`).** Velopack's auto-apply at startup is off
  (`STARTUP`): a launch is usually for a link, and applying first would
  drop it. A downloaded package installs from Quit (`app_menu::quit`,
  which the tray's Quit also calls: `apply_pending_on_quit` hands it to
  Velopack's updater, no restart) or from Settings' Restart to update.
  `updater::install` checks again every `CHECK_EVERY` on a Velopack
  install only. Route every new Quit through `app_menu::quit`.
- **Manual checks on macOS:** `--dry-run --first-run` opens the first-run
  window; `PWRROUTE_TRAY_OPEN=1` opens the dropdown at launch (the item can
  sit behind the notch on a full menu bar); `PWRROUTE_TRAY_FIXTURE=routing`
  or `=paused` draws the design's tray fixture instead of the feed. A second
  router on the machine takes the launch: give the build its own `HOME`
  while another one runs.
