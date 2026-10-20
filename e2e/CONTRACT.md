# Desktop E2E contract

What the app promises the harness in `e2e/`, and what the router must
report for the routing scenarios. The app half is
`crates/pwrroute-app/src/e2e.rs`; the harness half is `e2e/src/lib.rs`. A
section marked **pending** describes behavior a test already asserts under
`#[ignore]`; meeting it means removing that `ignore`.

## The hook

Every build carries the hook, and it stays inert unless the environment
turns it on when the app starts:

| Variable | Effect |
|---|---|
| `PWRROUTE_E2E_SOCKET` | Serve the control socket here: a file path on Unix, a named pipe name on Windows. Also skips the startup update check, so a run makes no network call, and opens the first-run window only for `--first-run`, never because onboarding is pending. |
| `PWRROUTE_E2E_ROUTER_DIR` | Keep the router's lock and IPC socket in this directory instead of the per-user one, so a test run neither forwards to nor blocks the developer's own PwrRoute. |

The harness also passes `--dry-run` on every launch, so nothing opens, and
points `HOME`, `XDG_CONFIG_HOME` and `XDG_DATA_HOME` (on Windows
`USERPROFILE`, `APPDATA` and `LOCALAPPDATA`) of every isolated launch into an
empty directory, so the developer's own Finicky config and `rules.json` are
never found. The
registration scenarios' shared launch keeps the real home, as the copy the
OS starts does, and always passes a config. The window tests run with no
config, and the harness puts `e2e/fixtures/rules.json` where the app looks for
`rules.json` in the empty home, so the window shows those rules, editable.
The routing tests add `--config e2e/fixtures/routing.js` and no `rules.json`,
which puts that config alone in charge and shows it read-only.

### Wire format

One request per connection, one line of JSON each way. Every answer carries
`ok` (bool), `v` (protocol version, now `1`) and `pid`; a failed request has
`error`. Bump the version on an incompatible change.

| Request | Answer |
|---|---|
| `{"cmd":"ping"}` | `{}` |
| `{"cmd":"state"}` | See below. |
| `{"cmd":"key","keys":"down cmd-q"}` | Keystrokes in GPUI's syntax, dispatched to the main window with `Window::dispatch_keystroke`, so key bindings and the focused view's handlers run as for a real key press. |
| `{"cmd":"click","region":"rail.activity"}` | A left mouse down and up at the center of a painted region, through `Window::dispatch_event`: hit testing and click handlers run as for a real click. |
| `{"cmd":"action","name":"quit"}` | `quit` dispatches the app's `Quit` action (the menu bar's Quit, ⌘Q, Ctrl+Q). `closeWindow` closes the main window as its close button does. `showWindow` opens or raises it. |
| `{"cmd":"dispatches"}` | `{"dispatches":[…]}`: the router's recent `DispatchRecord`s, newest first, serialized as in `pwrroute-router` (camelCase). |
| `{"cmd":"os"}` | Which app handles web links now: `pwrroute_os::default_browser_status()`, read-only, as `{"defaultBrowser": bool, "http": …, "https": …}`. A request of its own because on Linux it starts `xdg-mime`. |

`ping`, `dispatches`, `os` and the registration actions are answered off the
UI thread; the rest run on it and time out after 5 s. `key` and `quit` answer
first and act once the answer is written, so a quit never ends the process
before the client has its answer; requests still apply in order. A connection that sends
nothing for 10 s is dropped (not on Windows, whose named pipes have no
timeouts).

`state`:

```json
{
  "windows": [{ "title": "PwrRoute", "active": false, "main": true,
                "bounds": {"x":…,"y":…,"width":1240,"height":820},
                "viewport": {"width":1240,"height":820}, "scaleFactor": 2 }],
  "main": { "frames": 12, "focused": true,
            "regions": { "titleStrip": {"x":0,"y":0,"width":1240,"height":41}, … },
            "section": "rules", "configInCharge": false, "editing": true,
            "rules": { "rewrites": 3, "handlers": 7,
                       "selection": {"kind":"handler","index":2},
                       "selectionEnabled": true } },
  "router": { "dryRun": true, "configError": null }
}
```

`title` is the platform's title on macOS and `null` elsewhere: gpui-pre
reads a title back only from AppKit. The harness checks the title through the
OS's own window list on every platform (`e2e/src/os.rs`). `main` is `null`
while the main window is closed. `frames` counts the main window's
prepaints, so a positive number means GPUI really laid out and drew a frame.
`frames` and `regions` belong to the window that painted them: a reopened
window starts again from zero, never with the closed one's. `section` is the
screen shown (`rules`, `test`, `activity`); `configInCharge` says a Finicky
config is routing and shown read-only, and `editing` that the visual rules
and their editor are shown instead. `selection` is the selected rule, a
rewrite or a handler by index (`null` with no rules), and `selectionEnabled`
its switch.

### Regions

A region is the painted bounds of one element, recorded in window
coordinates during prepaint by `e2e::regions`, which a parent div hands to
`on_children_prepainted` with one name per child, in order. A region is
present only once the current main window has painted it, and is dropped
when its parent paints without that child (the editor off the Rules screen,
a strip with one chip).

| Region | Parent |
|---|---|
| `titleStrip`, `body` | the window root (`app.rs`) |
| `rail`, `pane`, `editor` | the body (`app.rs`); `pane` is the screen's own content |
| `titleStrip.brand`, `.chevron`, `.breadcrumb`, `.spacer`, `.chip1`, `.chip2` | `chrome::title_strip`; the chips depend on the screen |
| `rail.rules`, `.test`, `.activity`, `.settingsHeading`, `.openSettings`, `.foot` | `chrome::nav_rail` |
| `editor.title`, `editor.enabled` | `editor::header` |

When you add, remove or reorder children of one of those divs, update its
name list in the same change; the E2E layout tests catch a mismatch.

## Router: dispatch decisions

What every dispatch must report through `dispatches`, whatever path the link
took. The routing tests assert these against `fixtures/routing.js`.

| # | Requirement | Status |
|---|---|---|
| R1 | `--dry-run` resolves and records every link and opens nothing; the record's `outcome` is `dryRun` and its steps include `{"kind":"launched","dryRun":true}`. `status` reports `dryRun: true`. | met |
| R2 | Each record carries `link.url` as received, `link.source` (`commandLine`, `appleEvent`, `api`), `forwarded`, `finalUrl` after every rewrite, `browser.name` and `browser.profile`, and `steps` naming the rules that fired: `rewrite` with its `rule` index, then `matched` with `by.kind` `handler` (and `index`) or `default`. | met |
| R3 | `browser: null` records `outcome: notOpened`. | met |
| R4 | A second launch with links forwards them to the running router (`forwarded: true`), exits 0, and opens no window. | met |
| R5 | A launch whose command line carries links routes them and shows no window. | met |
| R6 | macOS: a `kAEGetURL` Apple Event to the running app records `source: appleEvent`. | met |
| R7 | URLs stay in memory: the hook's socket is the only way out, as with the router's own IPC. Nothing writes a URL to disk or a log, the hook included. | met |
| R8 | A router the OS starts for a link honors `PWRROUTE_E2E_SOCKET`, `PWRROUTE_E2E_DRY_RUN=1` and `PWRROUTE_E2E_CONFIG=<path>` from its environment, since the OS passes no arguments of ours. The two flags count only while the hook is on. | met |

When the launcher lands, R1 must keep holding: `--dry-run` never reaches it.

## Rail navigation

`rail_navigation_switches_sections` asserts:

- `state.main.section` names the shown section: `rules`, `test`,
  `activity`.
- A click on `rail.rules`, `rail.test` or `rail.activity` shows that
  section, and the editor region is painted on Rules only.

## Registration (disposable machines only)

The scenarios that hand a link over through the OS's handler for web links
change machine-wide state, so they run only where `PWRROUTE_E2E_DISPOSABLE=1`
marks a throwaway runner, and skip anywhere else. GitHub's hosted runners
qualify. The self-hosted `pwrdrvr-macos` Tart guests do not: they persist
between jobs and are shared with other repositories. Never set it on a
developer machine.

The hook provides:

| Request | Effect |
|---|---|
| `{"cmd":"action","name":"registerBrowser"}` | `pwrroute_os::register_browser()`: lists PwrRoute as a browser for this executable. Never changes the default. |
| `{"cmd":"action","name":"setDefaultBrowser"}` | `pwrroute_os::set_default_browser()`; answers `{"request":"Set"|"Prompted"|"OpenedSettings"}`. |
| `{"cmd":"action","name":"unregisterBrowser"}` | `pwrroute_os::unregister_browser()`. |

The app refuses all three unless its own environment has
`PWRROUTE_E2E_DISPOSABLE=1`, whoever asks. The registration is the app's own
(its `current_exe`), never a registry or desktop-file write from the test.
These run off the UI thread, since they start OS tools, and can be slow:
xdg-utils' generic `xdg-settings` sleeps about 12 s while it sets the default
browser. The harness allows them 60 s.

`{"cmd":"os"}` reports `pwrroute_os::default_browser_status()`, read-only.

Per OS, the scenarios hand the link over like this:

| OS | Becoming the handler | Hand-off | Status |
|---|---|---|---|
| Linux | `setDefaultBrowser` (`xdg-settings`, else `xdg-mime`); no prompt | `xdg-open URL`, which runs the desktop entry's `Exec` | runs on the hosted runner |
| Windows | `registerBrowser` only: an app cannot make itself the default (`UserChoice` is hashed), and `setDefaultBrowser` opens Settings | the command the `PwrRouteURL` ProgID registered, run as the shell would once the user picks PwrRoute | runs on the hosted runner |
| macOS | `setDefaultBrowser` shows LaunchServices' consent prompt | `open URL` | **pending**: the harness cannot confirm the prompt yet, and there is no disposable macOS runner. Confirming it through System Events needs Accessibility for the runner. |

The cold-start scenario hands the OS-started copy `PWRROUTE_E2E_SOCKET` (a
fixed per-user session socket), `PWRROUTE_E2E_DRY_RUN=1` and
`PWRROUTE_E2E_CONFIG` through the environment of the tool that opens the
link (R8), then attaches to that socket.
