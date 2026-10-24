# pwrroute-os

The OS edge: link receipt (macOS Apple Event sender, argv), default-
browser registration on macOS, Windows and Linux, and apps' icons (macOS).

- **Never change this machine's default browser.** No test, example or
  launch calls `register_browser`, `unregister_browser` or
  `set_default_browser`. Test the plans instead: `windows_registry_values`
  and `desktop_entry` are plain functions for that reason. The exception is
  the app's E2E hook, which calls them only on a throwaway runner
  (`PWRROUTE_E2E_DISPOSABLE=1`, see `e2e/CONTRACT.md`).
- **Two acts, kept apart.** `register_browser` / `unregister_browser` list or
  unlist PwrRoute as a browser (installer and updater hooks; keep the names).
  `set_default_browser` asks to become the default and runs only from an
  explicit user action; the OS always gets the last word.
- **`unsafe` lives in `src/macos/launch_services.rs`,
  `src/macos/app_icon.rs` and `src/macos/window_order.rs` only**: the
  LaunchServices C calls, the two AppKit calls that render an icon
  (`CGImageForProposedRect`, the PNG export), and the window server's list
  (a typed cast and three CoreGraphics key constants), each with a `SAFETY`
  comment. objc2's other Foundation and AppKit bindings are safe; prefer
  them.
- **`top_window_owner` reads the window server, not AppKit**, so it is
  current from any thread even while the app's main thread is busy (which
  is what the app's hand-off log measures). Owner pids and layers need no
  Screen Recording permission; never read window titles there.
- **`pass_front_to` is for a hand-off launch only** (`open -g -n`, a
  Chromium profile): macOS 14's cooperative activation lets the running
  browser bring itself forward only when the active app gives way. It
  activates the instance with the earliest launch date, never the
  short-lived `-n` one, and does nothing unless PwrRoute is the active app.
  It takes a bundle id: the launcher's `Plan::hand_off_to` names the copy,
  and says when the link is not meant for the background.
- **App icons (`app_bundle`, `app_icon_png`) read the disk.** Callers run
  them off the UI thread and cache the result; the app's `app_icons.rs`
  does. Windows and Linux return `None` until they have an implementation.
- Names (`BUNDLE_ID`, `PROG_ID`, `DESKTOP_ID`) must match `packaging/`; a
  test reads the packaging files to hold them together.
