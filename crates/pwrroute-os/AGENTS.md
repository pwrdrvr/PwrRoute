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
- **macOS defaults to the link helper.** `set_default_browser` registers the
  app and its nested helper with LaunchServices (`LSRegisterURL` on each
  bundle) and asks for `LINK_HELPER_ID`; it refuses a bundle without the
  helper. `default_browser_status` counts either id as PwrRoute and sets
  `legacy` when the app's own id (what v0.1.0-rc.1 registered) is one of
  them; `macos_status` is the pure mapping, tested everywhere. Windows and
  Linux never set `legacy`.
- **`start_link_helper(replace_stale)` replaces a helper left from before an
  update.** Velopack restarts only the app, so a helper started earlier runs
  its deleted binary and still gets every link. `link_helper.rs` holds the
  pure decision: stale when its executable is gone, or when it is this app's
  helper and launched before the bundled executable was created (updates
  extract new files). Stale ones get AppKit's quit, then `forceTerminate`
  after 2 s, on a background thread, and the bundled helper starts as a new
  instance. The app passes `false` when a helper launched it with links;
  that helper exits by itself after its next job.
- Names (`BUNDLE_ID`, `LINK_HELPER_ID`, `LINK_HELPER_PATH`, `PROG_ID`,
  `DESKTOP_ID`) must match `packaging/` and xtask; a test reads those files
  to hold them together.
