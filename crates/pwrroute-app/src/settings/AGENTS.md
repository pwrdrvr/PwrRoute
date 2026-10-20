# settings

The Settings window: its own window, one at a time (`open`), as in PwrSnap
and PwrGit. Drawn from `design/PwrRoute Settings.dc.html` (1a Local agents,
1b Configuration, 1c Browsers); General, Updates, Diagnostics and About are
placeholders until the family's panes are ported.

- **The window draws a `SettingsModel` and nothing else.** `model.rs` is the
  contract for the MCP server, config discovery and the browser catalog:
  they fill it in their own words, with display-ready strings, and the
  window never depends on their crates. `fixtures.rs` is contrived data
  transcribed from the design; keep it that way.
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
  are on `Pane::nav_id`/`pane_id` and the `*_ID` constants in each pane.
