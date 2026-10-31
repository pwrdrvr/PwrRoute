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
  (the rule editor's lanes, Activity's Add a rule sheet) and the
  Browsers pane read the same scan.
  `settings.json`'s `browsers.hidden` (bundle ids, or names where there is
  none) keeps apps out of the pickers; rules that name a hidden app still
  resolve. `browser_scan::environment` opts the scan, router launcher and
  MCP browser listings into `<data dir>/profiles.json`. Successful reads
  refresh its parsed metadata; blocked reads may populate profiles from it
  while keeping `profiles_blocked` true. Settings labels those as cached,
  and launch diagnostics identify cache use without profile names. Unit-test environments
  never opt into the real data directory's cache.
  `browser_scan::install_fixture` gives a test a scan; first run
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
  config's, or the fallback took it), Not opened (`Filter::NotOpened`:
  Don't open, a config's `browser: null`, or held), Failed, Rewritten and
  Private, each with its count. The Rule
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
  a journaled row's record back from the journal. Add a rule is one click
  from a row (design: Activity Quick Add Rule): `activity::quick_add`
  decides when "+ Rule" takes the Took cell's place (the row under the
  pointer, `Activity::hovered`, set from `on_hover` as the Overview's
  `idle_hover` is; every row at rest under Everything else, whose Took
  heading goes; never a row kept as host only, nor the expanded row). It
  keeps the cell's width and the tag's height, so nothing moves, and its
  click opens the sheet without toggling the row. The expanded row draws
  the link on its own two lines (`line_clamp`, never `mono`, which does not
  wrap and pushed the buttons off the edge) and then "Add a rule…" first as
  the primary button. ⌘N (Ctrl+N off macOS, `add_rule_shortcut`) is
  `PwrRoute::activity_key`: the row whose "⋯" menu is open, else the
  expanded row, as the menu's first item does (a host-only row gets the
  sheet's disabled state), and only while that row is listed under the
  filter and search; a row click, "⋯" and the sheet's Cancel focus the
  window's list so the key reaches it. Every feed change and a scroll
  forget the hovered row (GPUI reports hover only on a move), as the
  Overview's `forget_pointer` does. The menu item, the button's tooltip
  (`overview::tip_with_key`) and the primary button show the key. The
  sheet's head cuts the link at two lines (Show full link past
  `LINK_FITS` characters), and a record that names its app offers "Any
  link from <app>" above the patterns (design: Add Rule From an App): the
  pattern `*` with Sent from on, which `AddRule::toggle_only_from` never
  leaves on its own (off goes back to the narrowest pattern). With Sent
  from on, Also catches lists only that app's links, compared as
  `rules[].opener` is. No pattern carries the query or fragment
  (`suggest_rule`: a one-segment page with a query is `host/page*`), and
  a Match row's pattern ends in an ellipsis beside an explanation that
  keeps `WHY_MIN`, so a long one never pushes the row past the sheet.
  The feed also keeps today's `Tally` (links per rule, handler and default owner, and per
  browser and profile), reset with the day, for the Overview's map.
- **Overview is where every window lands (`overview_model.rs`,
  `overview.rs`).** `Screen::Overview` is the default and the rail's first
  item; Rules opens as a list with nothing selected (a click or Down
  selects). `overview_model::build` is plain data over the edited
  `rules.json`, the config's `describe()`, the routing `Status` (config
  error > paused > holding > sending > not default > unknown > routing), the scan
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
  a hover only notifies. A port sits on a border and must paint after it:
  GPUI paints a div's border after its children, unlike CSS, so a row's
  or one-node card's port is the frame's sibling, and a browser card's
  fill, border and shadow are an underlay under its nodes. A row click
  pins (Edit ›, a double-click or Return opens Rules); Esc, Clear or the
  empty ground lets go. A ƒ row
  reads `function_excerpt` of the function's source, a guard's condition
  when it returns a literal; a `finicky.matchHostnames` function reads as
  its hostnames (`FunctionSpec::hostnames`, from the engine). A browser
  card's header "+ Rule" is `PwrRoute::add_rule_for` with no profile (the
  browser's own default); an idle node's "+ Rule for Travel" (its click,
  or Return) is the same with that profile, and a private window on for a
  private-window node. Both name what they add for, so a profile's button
  never reads as the card's. The node's writes the card's name, so a private node whose word came only
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
- **Synced rules (`sync.rs`; design: Synced Rules, Synced Rules
  Machine).** `sync::install` runs `pwrroute_sync::Service` over
  `rules.json` (and the Finicky config in charge unless `settings.json`'s
  `sync.config` is `false`: absent means on, because a config's handlers
  run before the rules and a folder without it is half the routing) and
  the folders in `sync.targets`; `rules.json`'s item names the config it
  runs after here (`runsAfter`), so another machine can tell what it is
  missing (Settings › Cloud Sync, `settings/AGENTS.md`); it wakes on
  window activation and after an agent's `pwrroute_rules_apply`, and a
  pull is followed by `app::sync_rules_windows`, so every writer goes
  through the same path. `sync::machine()` is this machine's
  `machine.json`; every engine that resolves a link takes it as
  `Sources::machine`, and `edit_machine` is the one writer (it refuses a
  `machine.json` that does not parse rather than save over it);
  `reload_machine` follows hand edits on window activation. The files
  that sync are re-read on every wake and pass (`follow_items`), since
  the router publishes the config in charge after launch.
  `sync::overall` is the one reading of how sync stands: `line_of` makes
  the rail foot's line and the strip's chip (`chrome.rs`) from it, and
  `tray_row_of` the tray's row (`TrayModel::sync`: the dropdown's row
  under Incoming links, and the native menu's first row, enabled only for
  a conflict); each opens Settings › Cloud Sync. The editor offers named
  destinations above the lanes, Name it (Return makes a `destinations[]`
  entry from the rule's browser, `RulesStore::name_destination`) and a
  Rule set field. The Rules list heads each run of rules in one set
  (`set_heading`) and draws a named destination by its name and what it
  opens here; a rule whose set is off here is dimmed and reads "off on
  this Mac" (`Matcher::OffHere`). Folders are added only from a click.
- **Settings persist in `settings.json` (`settings_store.rs`).** The data
  directory's `settings.json` holds Activity's retention and the journal
  (`activity.keep`, `activity.journal`, `activity.journalDays`), the
  hidden browsers (`browsers.hidden`) and the Logs window's view
  (`logs`, below); an absent file is the defaults, and
  a bad one is reported and ignored. `router.rs`
  reads it before the router spawns, so `Options::retention` and
  `Options::journal` (`<data dir>/journal`) are in force from the first
  link; `settings_store::install` then makes the same store and the
  router's handle the globals Settings › Privacy edits, saving first and
  applying through `set_retention` / `set_journal`. The file carries
  numbers, switches and a level, never a link or a log line.
  Settings › Experimental's macOS-only "Yield focus to browsers" saves
  `experimental.yieldBrowserFocus`, off by default. The store shares an
  atomic switch with the dispatch thread, so it applies to the next link
  without a restart. It gates only `yield_front_to`; the launcher's running
  browser detection and direct singleton delivery stay enabled.
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
- **Menus follow PwrSuite menus v1 (`app_menu.rs`, `help.rs`).** The
  standard is the PwrSuite Claude Design project's "Menu Standard - Suite
  Review" and its "Views Slot" addendum: [PwrRoute] · File · Edit · View ·
  Window · Help, with About and Check for Updates… atop the macOS app
  menu, View opening with the screens, and Help in its fixed order (Settings
  in File and About last in Help off macOS). `app_menu::menus` is a pure
  template per `Platform` and its tests hold both to the order. macOS
  installs it with `cx.set_menus`; Windows and Linux draw the same
  template in the title strip (below). No Website row
  (PwrRoute has none) and no Reload row. View ends in Actual Size, Zoom
  In and Zoom Out (`zoom.rs`, below).
  Edit's rows replay ⌘X/⌘C/⌘V/⌘A into the window in front: the text
  fields read raw keys, so those bindings live in `MENU_ONLY`, a context no
  element has, and never steal a key. A global action that touches a
  window goes through `in_front_window`, or `cx.defer`s whatever reads,
  raises or navigates a window (View's rows, About, Settings…, Help's
  windows): GPUI still holds the window an action was dispatched in, so
  the update fails without a word, or a second window opens. About (menu and tray) is Settings ›
  About. Help's links go through the router as an `Api` link (else
  `open_url`), so the rules pick the browser. Changelog, View License and
  Third-Party Notices are `doc_window.rs`, from files compiled in with
  `include_str!` (they work offline, on every platform); Logs is
  `logs_window.rs` over `log_view.rs`, which reads `main.old.log` and
  `main.log`, keeps the newest 5,000 lines and polls for appends and
  rotation. Help's windows share `aux_window.rs` (one of each, "Help ›"
  crumb) and keep the Dock icon (`dock.rs`).
- **The Logs window (`logs_window.rs`, `log_view.rs`; design: PwrAgent's
  "Log Viewer Workflows UX Review").** `LogView::rows` is the model: the
  lines the levels let through, and with a query in Filter mode only the
  matches, `context` lines around each and a `Row::Gap` per hidden run
  (a click opens it); Highlight keeps every line; the mark is a
  `Row::Mark` after the line it was dropped on, in the view only. The
  view is a GPUI `list`, not `uniform_list`: `sync_list` diffs
  `RowKey`s (by line number, which outlives lines dropped from the
  front) and splices only what changed, so measured rows keep their
  height at 5,000 lines. GPUI text has no hanging indent, so Wrap cuts a
  line into rows itself (`LogsWindow::segments`, GPUI's `LineWrapper`)
  and draws continuations 2ch in; with Wrap off each row is one line and
  the text scrolls sideways under a fixed gutter (`h_offset`). The
  current match (`LogView::active`, a line number) moves with Enter /
  Shift-Enter and the field's chevrons; Esc returns to it among all the
  lines. Select since mark, or a Shift-click that reaches the newest
  line, grows with new lines (`follows_tail`); a plain click or ⌘A stays
  put. Paused, `sync_list` keeps the line at the top in place whatever
  changed around it.
  Wrap, Filter/Highlight, Context, Include diagnostics and the Error,
  Warning and Info levels are `settings.json`'s `logs`
  (`settings_store::LogsSettings`); Debug is never stored, nor the
  query. Below `NARROW_BELOW` Wrap and Mark keep icons and tooltips;
  `FRAME.min` holds the toolbar's one row. Window tests drive it
  through `debug_selector`s; the `screens` example draws `help-logs*`.
- **The strip is the menu bar off macOS (`chrome.rs`, `menu_bar.rs`;
  design: Windows and Linux Chrome 1a/1b).** Every window with a strip
  (main, Settings, the importer, first run, Help's) owns a `MenuBar` and
  draws `chrome::strip`, never its own copy. `chrome::Chrome` decides:
  macOS keeps the brand at x=96 after the traffic lights; Windows and Linux
  start it at x=16 with the bar after it, built from
  `app_menu::menus(Platform::Other)` (no second list) plus Window's list of
  the windows with a bar. Rows are `widgets::menu_card`/`menu_row`, each with
  the keymap's shortcut ("Ctrl+W"), and dispatch to the window's focused
  element. Keys reach the bar through `cx.intercept_keystrokes`, before any
  binding or field, so focus never moves to it and Edit's replay lands in
  the field: F10 or a lone Alt (a binding, `FocusMenuBar`) takes the
  keyboard, Alt+letter opens a menu, arrows/Enter/Esc walk it, and a
  shortcut lets go and runs. The keyboard model is `menu_bar::State`, plain
  data with its own tests. **Bring a window forward with
  `menu_bar::bring_forward`, never `activate_window` directly:** on Windows
  GPUI taps Alt itself (`SendInput`) to take the foreground, and the bar
  ignores a lone Alt for `FAKE_ALT_WINDOW` after `bring_forward`, and after
  a window becomes active (Alt+Tab back in re-reports a held Alt), so neither
  lights File; nor does releasing an Alt that clicked. On Windows GPUI hides the OS title bar, so the
  strip is the drag area and draws minimize, maximize/restore and close as
  `WindowControlArea`s (Windows presses them; titles and buttons `occlude`
  the drag area, and so must anything clickable a window adds to its strip,
  or Windows takes the press for a drag). Linux keeps the desktop's server-side title bar and its
  buttons; the design's frameless GNOME strip (1b) is not built yet.
  `app_menu::install_for(Chrome::Windows | Linux)` draws that strip and
  installs the Ctrl keymap on a Mac, for the window tests and `screens`.
- **Log with the `log` macros, never `eprintln!` (`diag_log.rs`).**
  `diag_log::install` (top of `main`) sends PwrRoute's lines, Velopack's and
  GPUI's warnings to `main.log` in `pwrroute_os::log_dir()`
  (`~/Library/Logs/PwrRoute`), rotated to `main.old.log` past 1 MB, and
  echoes to stderr only when stderr is a terminal or the E2E harness
  launched it (`e2e::requested`: stderr is the test's `app.log`). A
  launched app has no terminal, so a message on stderr is lost. `PWRROUTE_LOG=debug` raises
  PwrRoute's level. `eprintln!` stays only for the `pwrroute rules` CLI and
  `pwrroute mcp`, whose stderr is someone's terminal or MCP client.
  **Never log a URL, a host, a window title or error text that can quote
  one.** Per-link lines come from the router (`DispatchRecord::log_line`,
  after the record is reduced to its rule's privacy) and from `handoff.rs`
  (who owns the top window from receipt until the browser does), and both
  are written only while links are kept at all. `router.rs`'s
  `BrowserLauncher::launch_observed` reports planning, URL-free delivery
  decisions, profile/private requests, fixed note labels, yield and each
  socket/command attempt through the router's observer. The router logs
  them at Info with the link id and attempt number, only while diagnostic
  logging and retention are enabled. Never log the command line or stderr.
  Command return and socket ACK do not establish tab readiness. The window
  watcher logs its first sample's delay and largest sampling gap, and warns
  when PwrRoute stays on top through its timeout. The retention text in
  Activity, Settings and MCP names the log while it is.
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
- **A Rules row's "⋯" is real (`rule_menu.rs`; design: Rule Editing 3a,
  3b, 4b).** `rules_view::rule_row` draws only the `more` it is given
  (`rule_menu::more_cell`), never a bare "⋯": a rule's menu moves it
  (top, up, down, bottom; also by dragging the row and ⌥⌘↑/⌥⌘↓),
  duplicates it (⌘D, without its examples), turns it off, tests a link
  with it and deletes it (⌫). Moves go through `RulesStore::move_to`,
  which keeps each entry's id, so a rule's saved version moves with it;
  the selection, the rail and the draft notes follow by
  `rule_menu::shifted`. A move's count re-resolves the feed's newest
  `IMPACT_OF` full-URL records against the written file before and after
  (`RowActions::impact`), and says nothing below `IMPACT_MIN`. Delete
  leaves a tombstone with Undo and ⌘Z (`RulesStore::remove`/`restore`)
  until the next `refresh`; a draft is discarded without one. Everything
  else's menu writes `defaultBrowser`/`defaultProfile` through
  `write_now`; a config's read-only row offers its file and the importer.
  Previews (the importer's) pass no `more`. The rule menu's rows are an
  exception to `menu_row` (a move shows a count and a key, and an item
  can be dim or danger); their labels still end in an ellipsis. The keys
  act on the row whose menu is open, else the selected row. A drop lands
  only on the row its line is drawn on.
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
  and its config row opens Settings › Config Files.
- **Local agents (`mcp.rs`).** `mcp::install` serves MCP from
  `pwrroute_mcp::McpHost` on `<data dir>/mcp.sock`, over the router's
  handle and an engine that follows the same files; `pwrroute mcp` in
  `main.rs` is the stdio proxy and runs before anything else. Prompts
  become `ApprovalWindow`s on the GPUI thread (design: Agent Approval):
  the role cards are a radio group with the request's first (least) role
  chosen, and only Allow as <role> (Return) or Deny (Esc) answers; a card
  never does. No card or button takes focus (`focusable(false)`): keys
  belong to the window, so Return means Allow whatever was clicked, and
  an answer refocuses it. Closing the window while it asks denies. On
  macOS the panel
  opens without taking the keyboard (`focus: false`; a click makes it
  key): an agent asks while the person types in its terminal, and a
  Return there must not allow. Elsewhere the window comes to the front,
  so keys wait `KEY_GRACE`. There is no time limit: the server closes the
  prompt when every connection waiting on it has gone, and the window
  just closes (`closed_by_server`). After an answer the window stays on
  the outcome (allowed or denied) for `OUTCOME_FOR` and shrinks to it
  through `macos::set_prompt_height`, which finds the panel by its title,
  `PwrRoute — <client name>`. Each stage's height fits the design's
  content; the body scrolls rather than run under the buttons, and while
  it overflows (a client title wraps, a wider font) `ApprovalWindow::fit`
  grows the window by the overflow, up to `MAX_HEIGHT`. The title wraps,
  cut at `MAX_TITLE`; the name and version line ellipsizes: the client
  chooses them. A new prompt closes any window for the same name. The
  pane's "Recent agent actions" is the audit: tool, agent, time, outcome,
  never an argument. The switch is `mcp.json`; approvals are
  `mcp-agents.json`.
- **Tray (`tray/`).** `tray-icon` owns the OS item and its native menu
  (design 1c). On macOS a left click opens `tray/dropdown.rs`, a GPUI
  `PopUp` window (design 1a/1b) under the item
  (`macos::status_item_anchor`); its header (`HEADER_ID`) opens the main
  window, as the native menu's Open PwrRoute does. macOS 27 draws status items itself, and
  the item's own window can poke 1pt above the display or be 0x0 at the
  primary screen's corner, so the item counts by its center, an empty one
  not at all, and a pointer pressing the menu bar away from it wins
  (`macos::anchor_on`). Recents there are hosts only
  (`model::host_of`), from the feed. Pause is the switch's fourth
  segment (`tray::set_mode(Mode::Pause)`, `Override::Paused`), with the
  target
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
- **Incoming links (`tray/mod.rs`, design: Incoming Links, which
  supersedes Send Links turn 1).** One switch says what the next links do:
  Rules, Send, Hold, Pause (`model::Mode`), drawn the same in the
  dropdown and the Overview's hero, in its tone (Send accent, Hold info
  blue, Pause warning). `tray::set_mode` writes the router's one
  `Override` slot first (`set_override`), then `TrayModel::routing`
  (`Routing::Sending` / `Holding` / `Paused`). Send and Hold take the For
  row (`ForKind`: Links with the stepper's count, Time from `TIME_STEPS`,
  Until off), the last chosen this session; `set_for` and `step` apply to
  a running one from the next link (`rescope`, `step_next`: a count keeps
  what was used, a time restarts from now). Send adds the To row: the
  editor's lanes in place (`destinations::draw` at `Density::Compact`,
  over `TrayModel::choices`: the scan less PwrRoute, every other link
  router and hidden apps; a pill is the destination, `destination_of`);
  `choose_destination` retargets a send in force. At rest a hint says
  where Send goes and what Hold does; while on, the live line says what
  is left and has the one-click way back (Back to rules, or Resume). The
  router counts, and the tray reads its slot back after every dispatch and
  every second while a time runs (`sync_override`, `follow_clock`). Hold
  is refused while Activity keeps nothing in memory (`can_hold`, read
  again on every feed change and before Hold starts; a hold on then
  ends). Held links come from the feed (`HeldLink::from_record`: a hold's
  record not yet released, at least host level, that the router's
  `pending()` still holds) and lead the dropdown with their host
  and path, where they came from and where the rules would open them:
  Open (`tray::open_held`, `OpenAs::Rules`), "···" for Open in… (the
  lanes, `OpenAs::In`), Make a rule… (Activity's sheet) and Copy link, and
  Open all; they are not in Recent, and Quit says how many will be lost.
  Nothing about it is saved. The dropdown is keyboard-operable: Tab walks
  the card in draw order (its rows, then the switch as one stop, To,
  For, the stepper and the live button),
  Left/Right move the switch and For, Up/Down step, and the focused stop
  wears `.foc`'s ring only while the keyboard drives
  (`last_input_was_keyboard`). A key the focused stop does not take goes
  to the menu's own Up/Down/Home/End (`accessibility::move_focus`), and
  Esc folds what is open before it closes. The switch and For are radio
  groups to a screen reader, one tab stop each. The native menu has check rows (Route by
  rules, Send next link → destination, Hold next link, Hold for 1 hour,
  Pause routing), Choose… (the Overview) and Held links (N)… (Activity).
  The Overview reads it as `Status::{paused, holding, sending}` (hero
  `State::Holding` after paused and before sending, in `Tone::Info`, with
  Back to rules; the strip chip "holding · 1 of 3 left", the rail foot and
  the rail's Activity count "2 held"; the Held tile in Failed's place while
  links wait and none failed, opening Not opened; All links or every link
  held dims the rule column only, under "When rules decide again"). The
  hero's switch, For row and stepper are tab stops, reached by the
  window's Tab (`app_menu::FocusNext`) in draw order; `hero_key` gives
  them their arrows (`OverviewState::hero_stops`). It marks the node
  `Sending::is_node` names with a pill and offers Next link here on a
  hovered node (`tray::send_next_to`, always one link). Activity's Rule
  column says "next link" / "all links" in an accent chip with the arrow,
  and "held 2 of 3" in an info chip for a held link, whose expanded row
  leads with Open in <the rules' browser>; a sent link is unrouted and
  counts for its browser only, as a paused one does. Window tests install
  the tray with `tray::install_for_test`.
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
- **Apps are drawn by `widgets::badge` (`app_icons.rs`).** `badge_for`
  makes an `AppBadge`: the letters and the app (a name, bundle id or path).
  `badge` (and `icon_or` for the larger boxes: Settings' 20px, the
  Overview card's 28px, Test a link's 44px) draws the app's icon as Finder
  shows it, 4px larger than the box and centered so the layout is the
  same, else the letters; a plain string is a fixed label that never
  becomes one (`NOT_OPENED_BADGE`), and a target drawn as not installed
  uses `without_icon`. Models carry the `AppBadge`, never its letters
  alone. Icons load through GPUI's asset cache on the background executor
  (`pwrroute_os::app_bundle`, `app_icon_png`), at the window's pixel size,
  and only once `app_icons::install` ran (`main`): tests draw letters, and
  the `screens` example too unless given `--app-icons`.
  `browser_scan::rescan` drops them (`app_icons::forget`). Windows finds
  an app only by the `.exe` the scan placed it at (`location_of`), and
  draws the icon Explorer shows; Linux has no icon yet. PwrRoute's own
  window and taskbar icon on Windows is `packaging/windows/PwrRoute.ico`,
  embedded by `build.rs` as icon resource 1, which GPUI loads for every
  window.
- **Menus and select lists use `widgets::menu_card` and `menu_row`**
  (design: Popovers and Overflow 1a-1d). A select's list is as wide as its
  field (`.absolute().left_0().right_0()`), never narrower, and scrolls
  past `MENU_MAX_HEIGHT`; a row's hint gives way before its label and ends
  in an ellipsis; a browser row's hint is `profiles_hint` (two names, then
  a count). A field's value is `field_label`, never `theme::text`, which
  does not shrink.
- **Every editable field is a `text_field::TextField`.** It owns the caret
  and selection (arrows, ⌥/⌘ moves, Shift, click, drag, double-click) and
  draws through `text_field::field`, or `text_field::text` inside a
  caller's own `.inp` box made a `text_field::hit_area` (the rule editor,
  Activity's sheet). Never draw a field's text as a plain div: it loses
  the caret. Write the text only through `set`, so the caret follows.
- **Colors, dimming and zoom come from `theme` (design: Contrast and
  Scale).** Four palettes: dark and light, each with Increase Contrast,
  which strengthens ink and edges, never tints. Light draws only as a
  preview (`PWRROUTE_APPEARANCE=light`); `PWRROUTE_INCREASE_CONTRAST=1`
  previews the other. The theme is thread-local: set it on the GPUI
  thread, then `cx.refresh_windows()`. Text never fades by `.opacity()`:
  wrap what is off or missed in `theme::dimmed` / `dimmed_if`, which step
  the neutral text ramp down one level (muted is the floor), and say
  "off" another way (a chip, a strike, the dashed frame). Opacity is for
  what is not text (`DIM_DECORATIVE`) and for passing states (`DISABLED`,
  `LIFTED`, `SPOTLIGHT`). Every text token clears 4.5:1 and every control
  edge 3:1 on every surface, also dimmed. `theme`'s tests check that and
  compare `src/contrast.snap` (rewrite it with `UPDATE_CONTRAST_SNAP=1`
  and review the diff). `design/pwrroute.css` holds the same values.
  **Views size with `theme::px`, never `gpui_kit::px`:** it is the
  design's pixels times View › Zoom (`zoom.rs`: 100 to 200%, saved as
  `settings.json`'s `view.zoom`). A measured length goes back through
  `theme::css` before it meets a constant. Screen geometry (`macos.rs`)
  and tests keep `gpui_kit::px`. A window opens at `zoom::window_size`,
  its design size zoomed and capped to the display; its minimum is
  `zoom::min_size`, never zoomed (GPUI fixes it at open). Layouts reflow rather
  than clip (`app::Layout`): below 960 design px the rail is icons
  (full while its foot offers Reload and Overwrite); Rules
  shows the list or the editor (with "‹ All rules") when both do not fit;
  Activity's table scrolls sideways past `column::TABLE_MIN`; Today's
  tiles go in pairs. The `screens` example takes `--light`,
  `--high-contrast`, `--zoom=1.5` and `--display=WxH`.
- **Never clip text at its line box.** `theme::text` and most one-line
  helpers set the line height to the font size, and Geist's glyphs are
  taller than that (1.14em, Geist Mono 1.3em), so `overflow_hidden` on
  such a box cuts off g, p and y, and GPUI's `overflow_x_hidden` clips
  the top and bottom too. An ellipsized line is `widgets::one_line`; any
  other box that clips text sideways is `widgets::clip_x`, or a box that
  is taller than the glyphs (stretched to its row).
- **Every way to start a rule ends in one editor (`editor.rs`; design:
  Rule Editor Pickers).** Rules' Add rule (also
  under a config, where it opens the folded visual rules), the Overview's
  "+ Rule" and "+ Rule for …", and Test a link's "Add a rule…" (its pattern
  suggested from the URL the rules saw, after rewrites: `TestLink::rule_start`)
  all add a draft handler and select it; `PwrRoute::draft_from` gives the draft its
  note (where it came from, and `config_hint`'s line beside a config).
  Activity's sheet is the one other form, with the same rule: no browser
  is chosen for the person, and Add waits for one.
  A draft starts from a link (`start_from.rs`; design: Rule From a Link):
  the last link the feed holds with its full URL (`start_from::recent`,
  newest first, one per URL), or Test a link's, puts its narrowest
  `suggest_rule` pattern in the first field, selected, and the link in Try
  it; `PwrRoute::draft_link` keeps which link, by the draft's index like
  `draft_from` (`drafts_after_delete` / `drafts_after_insert` move both).
  A `StartLink` is the URL as opened, for Try it, and the URL the
  handlers saw after the rewrites (the record's `final_url`, Test a link's
  `rules_url`), which the patterns come from: a rule written for the
  pre-rewrite host would never match. The chips show only while the rule
  is a draft. Start from's rows are the one exception to `menu_row`: the
  URL is the label and gives way before its time and destination. The
  Overview's "+ Rule" offers the links but fills in nothing: its browser
  is set, so a pattern would save the rule unasked. The rail's Start from
  picker (drafts only) lists the recent links, the wider suggestions are
  chips under the pattern, and Clear goes
  back to a blank rule. The Overview's first-rule prompt names the last
  link, with "Blank rule" for `add_blank_handler`. The link stays in
  memory: Try it is never written.
- **Don't open (design: Not Opened).** `rules.json`'s browser `"none"`
  (`pwrroute_engine::is_dont_open`) is no app: `default_browser::
  browser_name` names it `widgets::DONT_OPEN` and `badge_for` gives it
  `NOT_OPENED_BADGE`, so a row, a tooltip or a label never shows an app
  called "none", and no picker ever calls it missing. It is the last lane
  of Open in where a rule is picked (`destinations::lanes(.., dont_open:
  true)`: the editor and Activity's sheet; never the tray's Send,
  which is Hold's business) and the last row of Everything else's menu,
  not the setup card or first run. The Overview draws it on its Not
  opened card (`overview_model::resolve`), and `Tally` counts the links a
  rule or default kept from opening there under the word `"none"` (a held
  link counts for no card). Never a target to launch: the
  tray's pause target passes over it, and the router's `FallbackSource`
  refuses it. In Activity a not-opened row has its filter
  (`Filter::NotOpened`), a trace line naming who decided
  (`not_opened_line`), and, while `RouterHandle::pending` holds it
  (`ActivityRow::waiting`), Open in… first: compact lanes under the row
  (`activity_actions::open_in_lanes`) whose pill calls
  `PwrRoute::open_not_opened(key, OpenAs::In(..))` off the GPUI thread.
  The later record's `released_from` marks the old row "opened later in
  X" (`ActivityRow::opened_later`); a journaled or swept one says why it
  can't be opened (`not_opened_why`). Hold's rows use the same action.
- **Open in is lanes, Sent from is chips (`destinations.rs`; design:
  Destination and App Pickers).** The editor, Activity's sheet and the
  tray's Send destination and held links' Open in… draw the same `destinations::lanes`
  (the tray at `Density::Compact`; it sizes its card before drawing, so
  `destinations::height` measures the wrapped pills and `tray/tests.rs`
  holds it to the laid-out lanes): a lane per browser in the scan, in its order
  (the sheet leads with where the link opened), and as pills its Default,
  each profile and its private window in the browser's word
  (`overview_model::private_term`; none for a browser without one). One
  pill is on across every lane, and a pill is the whole choice: browser,
  profile and private together. A value the scan cannot match is never
  rewritten: the rule's browser gets a lane of its own, first (missing once
  the scan ran, or "hidden in Settings"), a private flag on a browser with
  no private window from a link still shows as a lit "Private window"
  pill, and a profile the scan lacks gets a pill of its own, with
  `profile-warning` when the browser's profiles were read. The editor's way out, "Other app or profile by name…", is the typed
  browser and profile fields and the private switch; `Rail::browser_typed`
  keeps it across rebuilds of the same rule's rail (`rebuild_rail_kept`,
  only after adding or removing a field; a delete, insert or reload can
  put another rule at the index, so those rebuild fresh). Sent from is
  never typed: each app is a chip by the name it goes by
  (`default_browser::browser_name`) while `rules[].opener` keeps the bundle
  id, and "+ app" offers the apps that sent links recently
  (`start_from::recent_senders`), the apps other rules name, and any app
  in Applications through the file panel (`pwrroute_os::app_bundle_id`).
- **Which build this is (`build.rs`, `build_info.rs`; design: Build
  Identity).** `build.rs` reads the checkout, branch, commit (with its
  subject and time), the tag on HEAD and uncommitted changes from `git`
  once per build, into `PWRROUTE_BUILD_*` env vars; nothing reads a
  checkout at run time. It reruns when HEAD, a branch, a tag or anything
  under `crates/` changes, never on the index alone, and `git status`
  runs with `--no-optional-locks` so a build never rewrites what it
  watches. CI leaves the checkout out (a runner's path) and takes the
  branch and tag from GitHub's env when the checkout is detached or
  shallow. A release is the release profile at exactly `v{version}` with
  no changes (`BuildInfo::is_release`); anything else is a dev build. The
  rail's foot draws it (`chrome::build_block`): a release's one line, or
  a dev build's block of checkout, branch and commit rows, each a click
  to copy its full value (`PwrRoute::copy_build`, a check for
  `COPIED_FOR`), and a copy-all button. Settings › About's Source row and
  Copy build info, and the top of Diagnostics' Copy, write the same
  `BuildInfo::report`. Tests and the `screens` example set
  `PwrRoute::build` to `BuildInfo::fixture()`, never this checkout.
- **Dock (`dock.rs`, `macos.rs`).** The Dock icon shows while the main,
  Settings, first-run or one of Help's windows (`DocWindow`, `LogsWindow`)
  is open (`Regular`) and goes when the last closes
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
  On macOS a browser whose profile list is there but may not be read
  (`DetectedApp::profiles_blocked`, from `pwrroute_browsers::read_profiles`:
  macOS 27 keeps other apps' data until Files & Folders allows it) puts
  an access step first (design: First Run Profile Access): why, a copy
  of the switches to turn on, Open Files & Folders
  (`pwrroute_os::open_files_and_folders_settings`), Check again and Skip.
  It re-reads through `browser_scan::reread` (icons kept) on Check again
  and when the window is activated after losing focus, never on a timer:
  each blocked read can post macOS's notice again.
  "Open PwrRoute" (every step done) and "Later" mark it done and
  open the main window on Overview, on the first-run window's display
  (`onboarding::main_window_display`); closing the window only marks it
  done. `default_browser.rs` is the only caller of
  `pwrroute_os::set_default_browser`, from a click (Settings, first run's
  last step, the tray's "Make default browser…" row) or an explicit
  `--make-default` launch (PwrAgent's Start action); its `Os` struct takes
  fakes, so tests never reach the OS. After a request it re-reads the
  status every second for a minute (`POLL_EVERY`, `POLL_FOR`) and on
  window activation. LaunchServices can still name the old browser just
  after the dialog is approved, so an activation only settles a request
  as refused `ACTIVATION_GRACE` later. While first run is open and its last
  step is not done it re-reads every second regardless (`follow_while`);
  the step shows the wait and "Ask again", names an uncatalogued default
  by its bundle's name (`pwrroute_os::app_display_name`), and says why
  when the OS refuses the request with an error.
  `DefaultState::OtherCopy` is another copy of PwrRoute holding the
  default (its path, from `DefaultStatus::other_copy`); a request takes
  it back with no prompt (`DefaultRequest::TookOverFromCopy`).
  It also re-reads every `RECHECK_EVERY` (a minute) for the life of the
  app and when the tray dropdown opens, so a router with no window
  notices losing the default; the tray chip and its Make default row
  follow the registrar. Never wire registration into a test or an ordinary
  launch. `--make-default` is ignored with `--dry-run` or the E2E hook.
- **Updates (`updater.rs`).** Velopack's auto-apply at startup is off
  (`STARTUP`): a launch is usually for a link, and applying first would
  drop it. A downloaded package installs from Quit (`app_menu::quit`,
  which the tray's Quit also calls: `apply_pending_on_quit` hands it to
  Velopack's updater, no restart) or from Settings' Restart to update.
  `updater::install` checks again every `CHECK_EVERY` on a Velopack
  install only. Route every new Quit through `app_menu::quit`. Which
  release is the release channel's (`update_channel.rs`, the Pwr family's
  Stable/Beta × Latest/Prerelease): keep its selection logic pure and
  tested there, the network in `github_releases.rs`, and the check's
  decision in `updater::plan`. Only a manual check (`Trigger::is_manual`)
  may switch back to an older release, and a switch back never installs
  on quit.
- **Manual checks on macOS:** `--dry-run --first-run` opens the first-run
  window; `PWRROUTE_TRAY_OPEN=1` opens the dropdown at launch (the item can
  sit behind the notch on a full menu bar); `PWRROUTE_TRAY_FIXTURE=routing`,
  `=paused` or `=holding` draws the design's tray fixture instead of the
  feed. A second
  router on the machine takes the launch: give the build its own `HOME`
  while another one runs.
