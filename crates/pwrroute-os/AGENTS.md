# pwrroute-os

The OS edge: link receipt (macOS Apple Event sender, argv), default-
browser registration on macOS, Windows and Linux, and apps' icons (macOS
and Windows).

- **Default-browser changes require explicit intent.** No test, example or
  ordinary launch calls `register_browser`, `unregister_browser` or
  `set_default_browser`. An explicit `--make-default` launch may request
  ownership (PwrAgent's Start action); it is ignored with `--dry-run` and
  under the E2E hook. Test the plans instead: `windows_registry_values`
  and `desktop_entry` are plain functions for that reason. The exception is
  the app's E2E hook, which calls them only on a throwaway runner
  (`PWRROUTE_E2E_DISPOSABLE=1`, see `e2e/CONTRACT.md`).
- **Two acts, kept apart.** `register_browser` / `unregister_browser` list or
  unlist PwrRoute as a browser (installer and updater hooks; keep the names).
  `set_default_browser` asks to become the default and runs only from an
  explicit user action; the OS always gets the last word.
- **`unsafe` lives in `src/macos/launch_services.rs`,
  `src/macos/app_icon.rs`, `src/macos/window_order.rs`,
  `src/win/app_icon.rs` and `src/accessibility.rs` only**: the
  LaunchServices C calls, the two AppKit calls that render an icon
  (`CGImageForProposedRect`, the PNG export), the window server's list
  (a typed cast and three CoreGraphics key constants), and on Windows
  the Shell's `IShellItemImageFactory`, COM's per-thread init, GDI's
  `GetObjectW`/`GetDIBits` and the typed high-contrast query, each with a
  `SAFETY` comment. objc2's other Foundation and AppKit bindings are safe;
  prefer them.
- **`top_window_owner` reads the window server, not AppKit**, so it is
  current from any thread even while the app's main thread is busy (which
  is what the app's hand-off log measures). Owner pids and layers need no
  Screen Recording permission; never read window titles there.
- **`yield_front_to` is for a hand-off launch only** (a Chromium profile
  over the singleton socket or `open -g -n`): macOS 14's cooperative
  activation lets the running browser bring itself forward only when the
  active app yields. It yields (`NSApplication.yieldActivation`), never
  activates: the browser raises the window it opened the tab in, where
  activating it would raise whichever window was last in front. It does
  nothing unless PwrRoute is the active app. `NSApplication` is
  main-thread only, so off the main thread it queues the call on the main
  queue and waits a bounded time for it (the caller passes the budget;
  the yield must land before the browser hears of the link). It takes a
  bundle id: the launcher's `Plan::hand_off_to` names the copy, and is
  `None` when the link is meant for the background.
- **`system_uptime` is `NSProcessInfo.systemUptime`**: time awake since
  boot, the clock Chromium's `TimeTicks` reads on macOS. Edge's singleton
  message carries it; the app passes it to the launcher's `with_uptime`.
- **App icons (`app_bundle`, `app_icon_png`) read the disk.** Callers run
  them off the UI thread and cache the result; the app's `app_icons.rs`
  does. Windows has no bundle ids or app names: `app_bundle` answers only a
  path to a file (the app's scan names each `.exe`), and `app_icon_png`
  asks the Shell for that file's icon as PwrSnap's `--extract-app-icon`
  does (`SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK`, premultiplied BGRA made
  straight RGBA in `icon_pixels.rs`, which is plain code tested on every
  platform). Linux returns `None` until it has an implementation.
- Names (`BUNDLE_ID`, `PROG_ID`, `DESKTOP_ID`) must match `packaging/`; a
  test reads the packaging files to hold them together.
- macOS requests target the running `.app` by URL through NSWorkspace
  (legacy bundle-id fallback on macOS 11). Status also compares the selected
  handlers' paths, so an installed copy cannot count as the worktree copy,
  and names that other copy (`DefaultStatus::other_copy`).
- **LaunchServices keeps the default browser as a bundle id**, and opens the
  highest-versioned copy with it. A newer dev build, or the DMG, takes the
  links, and macOS shows no prompt for an id that is already the default.
  So `set_default` first takes every other copy out of LaunchServices
  (`lsregister -u`; the files stay, and a copy comes back when it is
  launched, rebuilt or rescanned), then asks; a takeover from another copy
  needs no prompt. Only that explicit request does this.
