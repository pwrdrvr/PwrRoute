# pwrroute-browsers

The browser catalog, installed-browser detection, profile listing, and
launching for the engine's `Browser` result. The router wraps `Launcher`
behind its own trait; keep the public API in `src/lib.rs` small.

- **The catalog is data.** `data/browsers.json` holds every browser; code
  never special-cases one by name. Add a browser there, with `sources` that
  support each fact (vendor docs, Chromium/Mozilla source, Flathub or distro
  packaging). `src/catalog/tests.rs` validates the file: run the tests after
  every edit.
- **Finicky's entries stay Finicky's.** Entries with `"finicky": true` come
  from Finicky's `browsers.json` (MIT, John Sterling);
  `tests/fixtures/finicky-browsers.json` is that file unchanged, and a test
  holds their macOS bundle id, app name, engine and profile directory to it.
  Only Finicky's entries carry the flag.
- **Never touch the real machine in tests.** Everything takes an `Env`;
  tests use `Env::fixture` over a temp dir with the fixture `Local State` and
  `profiles.ini`. Contrived names only. No test launches a browser:
  `tests/launch.rs` spawns the test binary itself as a stand-in, and macOS
  `open` is only ever planned.
- **Never open through the default browser.** On Windows and Linux the plan
  runs the browser's own executable or desktop entry. `start`, `xdg-open`
  and `open URL` route to PwrRoute itself once it is the default.
- **URLs are private.** Nothing here logs. `Plan` holds the URL;
  `LaunchError`'s `Display` and `Note`s never do.
- **Private windows are a catalog fact.** `private_flag` defaults from the
  engine (`--incognito`, `--private-window`, none for WebKit); an entry
  overrides it only with a source (Edge's `--inprivate`) or to say the
  browser has no command-line way (Arc, Dia). The launcher adds the flag
  once, after the profile flag and inside `open --args` on macOS, when the
  engine's `Browser` asks for it (`pwrroute_engine::Browser::private`, set
  from `rules[].private` or a known private flag in a JS config's args); a
  browser without one gets `Note::PrivateUnsupported` and still opens.
  `private_window` says what such a request opens, by the launcher's own
  lookup (`catalog_entry`); the app's Overview and its Activity counts
  use it rather than reading the catalog themselves. `private_label` names the window as the browser's menu does (the flag's
  word unless an entry, with a source, says otherwise: Brave, Vivaldi and
  Opera say "Private window"); the Overview draws it.
- **Running browsers are asked, not detected.** `Launcher::with_running`
  takes a bundle-id check (the app passes `pwrroute_os::app_is_running`);
  this crate never calls AppKit. With it, a macOS `-n` launch of a running
  Chromium browser adds `-g` so the hand-off instance does not take focus,
  and `Plan::hand_off_to` names the running copy unless the link (or the
  config's default) asked for the background: the app yields to that copy
  so it may raise its own window. Tests pass a closure.
- **A running Chromium browser gets the link over its singleton socket**
  (`src/singleton.rs`, Chromium's `process_singleton_posix.cc` protocol),
  not by starting a second copy with `open -n`, which costs seconds cold.
  `Plan::socket` is the planned hand-off (`Some(Err)` says why not, so the
  dry run says which way a link goes); `Launcher::run` tries it within
  `SOCKET_BUDGET` and runs the `open` plan on any failure, and `Launched`'s
  `Delivery` says which way it went. Only the user data dir from
  `profile_dir`, a socket and directory owned by this uid under `$TMPDIR`,
  and matching cookies pass. A config's `open` options (before its own
  `--args`) or `--user-data-dir` keep `open`. Tests serve a fake socket
  (`singleton::fake`); a test that `run`s a plan swaps its program for the
  fake `open` first, so a fallback never starts anything.
- **No `unsafe`.** Registry access goes through `windows-registry`'s safe
  API, confined to `src/registry.rs`. macOS detection reads XML
  `Info.plist`s rather than calling Launch Services.
