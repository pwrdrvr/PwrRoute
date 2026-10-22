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
- **Running browsers are asked, not detected.** `Launcher::with_running`
  takes a bundle-id check (the app passes `pwrroute_os::app_is_running`);
  this crate never calls AppKit. With it, a macOS `-n` launch of a running
  Chromium browser adds `-g` so the hand-off instance does not take focus.
  Tests pass a closure.
- **No `unsafe`.** Registry access goes through `windows-registry`'s safe
  API, confined to `src/registry.rs`. macOS detection reads XML
  `Info.plist`s rather than calling Launch Services.
