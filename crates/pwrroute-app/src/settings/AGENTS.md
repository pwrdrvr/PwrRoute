# settings

The Settings window: its own window, one at a time (`open`), as in PwrSnap
and PwrGit. Drawn from `design/PwrRoute Settings.dc.html` (1a Local agents,
1b Configuration, 1c Browsers) and `design/PwrRoute Activity Privacy and
Route Tests.dc.html` turn 1 (Privacy); General, Updates, Diagnostics and
About are placeholders until the family's panes are ported.

- **The window draws a `SettingsModel` and nothing else.** `model.rs` is the
  contract for the MCP server, config discovery and the browser catalog:
  they fill it in their own words, with display-ready strings, and the
  window never depends on their crates (`crate::mcp` fills Local agents).
  `fixtures.rs` is contrived data transcribed from the design; keep it that
  way. Privacy is real: `main.rs` replaces the fixture's `privacy` with
  `privacy_host::model`.
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
