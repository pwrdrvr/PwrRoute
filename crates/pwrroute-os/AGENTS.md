# pwrroute-os

The OS edge: link receipt (macOS Apple Event sender, argv) and default-
browser registration on macOS, Windows and Linux.

- **Never change this machine's default browser.** No test, example or
  launch calls `register_browser`, `unregister_browser` or
  `set_default_browser`. Test the plans instead: `windows_registry_values`
  and `desktop_entry` are plain functions for that reason.
- **Two acts, kept apart.** `register_browser` / `unregister_browser` list or
  unlist PwrRoute as a browser (installer and updater hooks; keep the names).
  `set_default_browser` asks to become the default and runs only from an
  explicit user action; the OS always gets the last word.
- **`unsafe` lives in `src/macos/launch_services.rs` only**, for the
  LaunchServices C calls, each with a `SAFETY` comment. objc2's Foundation and
  AppKit bindings are safe; prefer them.
- Names (`BUNDLE_ID`, `PROG_ID`, `DESKTOP_ID`) must match `packaging/`; a
  test reads the packaging files to hold them together.
