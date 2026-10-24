# pwrroute-link-helper

`pwrroute-links`, the macOS link helper: a windowless (`LSUIElement`) app
nested at `PwrRoute.app/Contents/Helpers/PwrRoute.app` (bundle id
`com.pwrdrvr.pwrroute.links`) that is the `http`/`https` handler once
PwrRoute is the default. LaunchServices activates whichever app it hands a
link to; the GPUI app is a regular app while a window is open, so as the
handler it brought its window over the reader's app on every link. The
helper takes that activation instead and forwards the link to the running
router.

- **Small on purpose.** No GPUI, no engine: `forward.rs` is plain Rust over
  two traits (`Router`, `Apps`) and is tested on every OS; `macos.rs` is the
  `NSApplication`, its delegate and NSWorkspace. The router and OS crates
  are dependencies for the socket client, the wire types, `current_opener`
  and `apple_event_launch_args`; the linker drops the rest (the release
  binary is about 650 KB). Elsewhere `main` is a stub, so the workspace
  builds on Windows and Linux.
- **The same hand-off as a second launch.** Each Apple Event's links go to
  the router as `Request::Open` with `Source::AppleEvent` and the sender
  from `pwrroute_os::current_opener()`, through `instance::Client` at
  `pwrroute_os::router_dir()`, on one worker thread so they keep their order.
  The helper stays resident; a later link skips the process launch.
- **It exits once its executable is gone** (`replaced.rs`). An update swaps
  the whole `PwrRoute.app` (Velopack renames the new bundle over the old one
  and deletes it) and restarts only the app; the Trash moves it. The helper
  notes its executable's dev and inode at start and looks again after each
  job and every 30 s while idle (after a move to the Trash macOS can no
  longer find the bundle id, so no link would come): once the path names
  another file or none, it forwards what is queued and exits, so the next
  link starts the helper installed now. The app also
  replaces a stale helper when it starts (`pwrroute_os::start_link_helper`).
- **A link is never lost.** No router answering means PwrRoute is not
  running or is stuck: the helper launches `PwrRoute.app` (found from its
  own bundle path) with NSWorkspace, `activates = false` and a new instance,
  and the links, `--source=apple-event` and `--opener=<JSON>` as arguments.
  That copy routes them without opening a window (`router::start`). Links
  arriving while it starts wait for its socket (`STARTUP_WAIT`). If macOS
  cannot launch the app, each link opens in Safari, named by bundle id so it
  can never come back here.
- **Never log a link.** `eprintln!` takes fixed words only; no URL, opener
  or error text that could quote one.
- **Unsafe is confined to `macos.rs`'s delegate class** (`define_class!`
  and its `super` init, plus reading an AppKit constant), each with a
  `SAFETY` comment. Prefer objc2's safe bindings for anything new.
- **Opened by hand** (a default launch, or a reopen not beside a link) it
  asks the router for the window, as a launch of the app with no link does.
  Not when the app started it (`--started-by-app`,
  `pwrroute_os::start_link_helper`, so the first link finds it up): that
  launch is for neither a link nor the window.
- Packaging (`xtask/src/macos.rs`): built with the app, lipo'd for
  `--universal`, its Info.plist from `packaging/macos/LinkHelper.Info.plist.in`,
  the app's icon files, signed before the app (never `codesign --deep`).
