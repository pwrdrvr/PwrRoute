# settings

The Settings window: its own window, one at a time (`open`), as in PwrSnap
and PwrGit. Drawn from `design/PwrRoute Settings.dc.html` (1a Local agents,
1b Configuration, 1c Browsers), `design/PwrRoute Activity Privacy and
Route Tests.dc.html` turn 1 (Privacy) and `design/PwrRoute Updates and
Default Browser.dc.html` 1a/1b (Updates), `design/PwrRoute Synced
Rules.dc.html` and its Machine file (Sync); Diagnostics and About follow
the family's panes in the same controls. There is no General pane until
there is something to put in it; About leads the nav, Browsers opens first.

- **The window draws a `SettingsModel` and nothing else.** `model.rs` is the
  contract for the MCP server, config discovery and the browser catalog:
  they fill it in their own words, with display-ready strings, and the
  window never depends on their crates (`crate::mcp` fills Local agents).
  `fixtures.rs` is contrived data transcribed from the design; keep it that
  way: it draws the `screens` example and `tests.rs`, never the shipped
  window, which `settings_live.rs` fills from the producers below.
- **Producers (`*_host.rs`).** Each builds its pane from the real thing and
  answers its requests; each has a pure `build` over plain facts, which the
  tests feed from tempdirs, never from the user's files. `browsers_host`:
  the one scan (`crate::browser_scan`), `rules.json`'s handlers for "in N
  rules" (a handler names an app by bundle id, name or path, ignoring
  case) and the apps the rules name that the scan did not find under
  Other apps; a switch saves `browsers.hidden` and refreshes the main
  window's pickers. `configuration_host`: `router::routing_source` says
  which file is in charge; the tiles only show it (`can_switch` is off;
  the importer's Choose source picks the source), the importer card is a
  read-only summary of `Engine::review_import` whose Review import… opens
  the importer (`importer_view::open`),
  Short links lists the engine's built-in hosts, and Reveal, Open data
  folder and Reload now act. While a config is in charge (never with
  visual rules alone) the Source card adds Everything else (design:
  Overview and Setup 5d): a segmented control, "{config}'s default ·
  {browser}" | "PwrRoute's default", that is `rules.json`'s `precedence`,
  the browser and profile selects that are its `defaultBrowser` /
  `defaultProfile`, and, only while PwrRoute's default is chosen, the
  Rules banner's warnings from the config's `describe()` (a function or
  `null` default, a handler that matches every link: `catch_all`, whose
  `matches_everything` the banner and Overview use too) and, with no
  default PwrRoute can open, the banner's `NEEDS_DEFAULT`.
  `configuration_host::edit_rules` writes them through the main window's
  `RulesStore` when it is open (sync, refuse as the store would, edit,
  save now), else through a store over the file; never around a store.
  A request runs inside the Settings window's own update, so the read
  back after a change (`settle`) is deferred; a change that was not
  saved comes back as the `.errl` line "Not saved: {reason}." and the
  control shows what the file still says.
  `follow` reads the pane again when the window comes to the front and
  the router's source or either file's time changed. `sync_host`: the
  sync service's `Status` (`crate::sync::feed`), the places
  `pwrroute_sync::detect` finds, `machine.json` and the synced
  destinations and sets; a folder is added only from Add or Choose…, a
  conflict resolves only from its buttons (naming the hash it showed), and
  binding a destination or switching a set writes `machine.json` through
  `sync::edit_machine`. Its copy says plainly what goes to the cloud.
  `updates_host`:
  `updater::installation` and
  `updater::state`; a dev build reads "not updatable", no check reads
  "never checked", and Check now is the menu's check (`updater::install`
  redraws the pane when it finishes). `system_host`: Diagnostics (version,
  OS from its files, data folder, the router's socket and `listening()`,
  what routes links, the MCP door from the Local agents model) and About
  (version, the build's Source and Copy build info from
  `crate::build_info`, license, addresses as text; its License, notices
  and Changelog buttons and Diagnostics' Open Logs ask for Help's
  windows). Diagnostics' Version carries the short commit ("+" for
  uncommitted changes) and its Copy starts with `BuildInfo::report`.
  A producer that reports back asks `settings::is_open` first, so a
  closed window reads no file.
- **Privacy (`privacy.rs`, `privacy_host.rs`).** The view keeps the two
  number fields (applied on Return, clamped to `COUNT_RANGE` /
  `HOURS_RANGE`) and the open sheet; Full URLs and the journal's Clear now
  go through the sheet (`Sheet::WriteFull`, `Sheet::ClearJournal`), and
  Escape, Cancel or a pane switch leave the switch where it was. The
  producer builds the model from `settings_store`, the router
  (`RouterHandle::retention().description` is the sentence under the
  switch, verbatim; `full_description` is the same sentence for the sheet)
  and the journal's files, and answers the requests: save, then
  `set_retention` / `set_journal`, then `settings::update`. Reveal uses
  GPUI's `reveal_path` on the newest journal file; Clear now closes the
  journal, deletes `activity-*.jsonl`, reopens it and clears the feed;
  Export writes the router's buffer, each record at its level, to the path
  the OS save dialog returns, owner-readable, and nowhere else. Nothing in
  the pane or its producer writes a URL anywhere but that export.
- **Local agents (`local_agents.rs`).** Revoke is two clicks: the first
  turns the cell into Revoke / Keep (`agents.confirming`, dropped on a
  pane switch or when the row goes), the second sends `RevokeSession`. A
  revoked row offers Allow again (`AllowSession` with the row's last role)
  and Forget (`ForgetSession`); `crate::mcp` answers them through
  `Agents::allow` and `Agents::forget` and rebuilds the pane.
- **Everything the reader asks for leaves as a `SettingsRequest`** through
  the `SettingsHost` handler, deferred until the window's update is done.
  The window flips its own model where the answer is obvious (a switch);
  anything the OS decides (above all `RegisterDefaultBrowser`) waits for the
  producer to report back. Nothing here calls `pwrroute_os`; never make it.
- **Controls are gpui-kit's unstyled `base` primitives** (`Button`,
  `Switch`, `Checkbox`, `Radio`, `Select`) dressed in the tokens by
  `controls.rs`, so focus, keyboard and accessibility come with them. Use
  them rather than a clickable `div`; `install` calls `base::init` for their
  key bindings.
- **Cards are disclosures** (`SettingsWindow::disclosure`): the heading
  folds the card, the trailing chip or button does not. Fold state is a
  reading position and lives with the window, never in settings.
- **Fidelity:** compare against headless Chrome at the design's size (each
  option's `.dv-card` is 1060px wide). Geist's `ch` is 0.663em
  (`style::ch`); an inline `font:` in the design resets line height to
  `normal`. `cargo run -p pwrroute-app --example screens -- OUT_DIR`
  renders each pane at its option's size (`settings-1a.png` …).
- Tests (`tests.rs`) drive the real window through gpui-kit's harness: ids
  are on `Pane::nav_id`/`pane_id` and the `*_ID` constants in each pane. An
  element the harness must find needs `.test_support()`; gpui-kit's
  controls have it, a plain `div().id(..)` does not.
