# pwrroute-app

The GPUI shell: the main window, the tray, the first-run window and the
resident router's hand-off. `main.rs` only wires them; put new surfaces in
their own modules.

- **Fixtures only.** Every window and the tray draw contrived data
  (`fixtures.rs`, `TrayModel::fixture`, `FirstRunModel::fixture`). Never a
  real URL, Activity entry or trace, in code or in a screenshot.
- **Tray (`tray/`).** `tray-icon` owns the OS item and its native menu
  (design 1c). On macOS a left click opens `tray/dropdown.rs`, a GPUI
  `PopUp` window (design 1a/1b); recents there are hosts only
  (`model::host_of`). The router fills `tray::model(cx)`; pausing updates
  that entity and whoever routes observes it.
- **The dropdown cannot move or grow from the top.** GPUI has no window
  move and macOS resizes from the bottom edge, so the window opens at
  `window_height` (the tallest the card can be) with a transparent
  background. Keep the metric constants in `dropdown.rs` matched to the
  layout; `tray/tests.rs` holds `card_height` to the laid-out card.
- **Dock (`dock.rs`, `macos.rs`).** The Dock icon shows while the main or
  first-run window is open (`Regular`) and goes when the last closes
  (`Accessory`). objc2's AppKit calls here are safe; this crate stays
  `unsafe`-free.
- **First run never touches the OS.** `FirstRunHooks` decides what its
  buttons do; development and tests use `FirstRunHooks::logging` or
  counters. Never wire `pwrroute_os::set_default_browser` into a test or a
  launch path that runs without a click.
- **Manual checks on macOS:** `--dry-run --first-run` opens the first-run
  window; `PWRROUTE_TRAY_OPEN=1` opens the dropdown at launch (the item can
  sit behind the notch on a full menu bar); `PWRROUTE_TRAY_FIXTURE=paused`
  starts paused. A second router on the machine takes the launch: give the
  build its own `HOME` while another one runs.
