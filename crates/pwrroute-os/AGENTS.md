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
- **`unsafe` lives in `src/macos/launch_services.rs` and
  `src/macos/app_icon.rs` only**: the LaunchServices C calls, and the two
  AppKit calls that render an icon (`CGImageForProposedRect`, the PNG
  export), each with a `SAFETY` comment. objc2's other Foundation and AppKit
  bindings are safe; prefer them.
- **App icons (`app_bundle`, `app_icon_png`) read the disk.** Callers run
  them off the UI thread and cache the result; the app's `app_icons.rs`
  does. Windows and Linux return `None` until they have an implementation.
- Names (`BUNDLE_ID`, `PROG_ID`, `DESKTOP_ID`) must match `packaging/`; a
  test reads the packaging files to hold them together.
