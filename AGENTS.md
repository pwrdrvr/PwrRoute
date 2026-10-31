# AGENTS.md

Guidance for coding agents (and humans) in PwrRoute — a cross-platform Rust
app in the Pwr family (siblings: PwrSnap, PwrGit, PwrAgnt) that becomes the
OS default browser and routes each link to a browser and profile by rules.
Keep this file short; see **Docs & conventions** below.

## Workspace

A Cargo workspace with a GPUI window, a Finicky-compatible rules engine, a
resident router that receives links from the OS, and a browser launcher. The
router resolves every link, opens it through `pwrroute-browsers`' launcher
(or the fallback browser, when that fails) and records it;
`--dry-run` swaps in a launcher that opens nothing.

- `crates/pwrroute-engine` — loads Finicky v4 `.js`/`.ts` configs in QuickJS
  (rquickjs) with oxc for TypeScript, and resolves URLs. See its `AGENTS.md`.
- `crates/pwrroute-router` — the dispatch pipeline, the in-memory trace of
  recent dispatches, single instance and IPC. See its `AGENTS.md`.
- `crates/pwrroute-browsers` — the browser catalog (`data/browsers.json`,
  from Finicky's `browsers.json` plus Windows, Linux and more browsers),
  detection, Chromium/Firefox profiles, and launch commands with a dry run.
  See its `AGENTS.md`.
- `crates/pwrroute-os` — link receipt and default-browser registration per
  OS; the one crate with `unsafe`. See its `AGENTS.md`.
- `crates/pwrroute-mcp` — the MCP server for local agents, served by the
  running app and reached through `pwrroute mcp`. See its `AGENTS.md`.
- `crates/pwrroute-sync` — copies `rules.json` (and, if asked, the Finicky
  config) to and from cloud-drive folders, with three-way conflict
  detection and backups. See its `AGENTS.md`.
- `crates/pwrroute-app` — the GPUI shell (binary `pwrroute`). It hosts the
  router and the MCP server and draws the design's screens over live data:
  Rules edits the real `rules.json`, Test a link runs the real engine,
  Activity and the tray show the router's dispatches, Settings and the
  first-run window read and change the running app. Contrived fixture data
  (`fixtures/`) is for tests and the `screens` example only. See its
  `AGENTS.md`.
- `e2e` — Desktop E2E: launches the built app and drives it through an
  env-gated control socket (`crates/pwrroute-app/src/e2e.rs`). See its
  `AGENTS.md` and `CONTRACT.md`.
- `xtask` — lint, test, license notices, release checks, packaging and E2E.

Port, don't invent: the design in `design/` decides the shape. Read
`design/PwrRoute - Name and Brief.dc.html` first. The PwrFinicky rough cut (a
fork of Finicky, Go + Electron) is reference only; Go and Electron are gone.

Code derived from Finicky (its configuration API in
`crates/pwrroute-engine/assets`, its `browsers.json` and launcher in
`crates/pwrroute-browsers`, and anything later ported from its resolver) is
MIT, Copyright (c) 2015-2025 John Sterling. Its notice is in `LICENSE`; keep
it there, and keep any per-file copyright headers. Finicky's name and icon are
not PwrRoute branding; the icon is by @uetchy and is not ours to reuse.

## Set up a checkout

`rust-toolchain.toml` pins Rust 1.97, and rustup installs it on first use.
Linux also needs GPUI's system libraries: `scripts/ci/install-linux-deps.sh`.
The lint chain needs Node.js 22 or newer for its dependency-free CI classifier
tests, Python 3.9+ for mocked macOS packaging/signing checks, plus two cargo tools:

```bash
# The versions CI installs; another cargo-about writes different notices.
cargo install --locked cargo-deny@0.20.2
cargo install --locked --features cli cargo-about@0.9.2
```

## Common commands

```bash
cargo run -p pwrroute-app        # the window
cargo xtask lint                 # everything CI's Typecheck job runs
cargo xtask test                 # every workspace test
cargo xtask licenses             # regenerate THIRD_PARTY_LICENSES
cargo xtask e2e                  # build the app, run Desktop E2E against it
cargo xtask bundle               # macOS: target/bundle/PwrRoute.app, zip, DMG ($CARGO_TARGET_DIR if set)
bash scripts/dev/start.sh        # macOS: target/dev/PwrRoute.app; requests default ownership
cargo run -p pwrroute-engine --example resolve -- --config FILE URL...
cargo run -p pwrroute-app --example screens -- DIR   # macOS: each screen as a PNG
```

`cargo xtask lint` chains rustfmt → CI classifier and packaging tests → clippy
`-D warnings` → `cargo deny check`
→ the THIRD_PARTY_LICENSES check, cheapest first. CI's Typecheck job runs
exactly this one command, so **add new repo-wide checks to the chain in
`xtask/src/main.rs`**, not as another CI step.

Workspace lints (root `Cargo.toml`) deny `unsafe_code` and `clippy::all`.
clippy::pedantic is not on yet; turning it on is its own change, with its
allow-list written down.

### Verification on Sandboxer

- When the thread has the Sandboxer MCP (`create_sandbox` and `exec`
  tools), do not run these on the local machine:
  - `cargo xtask lint` or any part of it: clippy, `cargo deny`, the
    THIRD_PARTY_LICENSES check.
  - `cargo xtask licenses` or `cargo about`: cargo-about alone takes every
    core for minutes.
  - `cargo xtask test`, `cargo test`, `cargo build --release`.
  - Linux Desktop E2E (`cargo xtask e2e`).
- Run them in a sandbox with the
  [Sandboxer verification skill](.agents/skills/sandboxer-verification/SKILL.md).
  This rule applies wherever repository guidance says to run one of them.
- Sandboxer is Ubuntu x64. It verifies Linux behavior only. macOS work
  (`cargo xtask bundle`, `scripts/dev/start.sh`, the `screens` example,
  macOS E2E) still runs on the laptop, in CI or through PwrSuiteLab.
- Verify a commit, not a dirty tree. Use one sandbox per thread, started
  from the `pwrroute-base-us-east` snapshot in the `us-east` region with
  `noEnv: true`. Stop it after each run.
- Attach the thread's PR and watch it with `notifyOn: ["merged", "closed"]`.
  PwrAgent releases the thread's sandboxes when its PRs merge or close.
- Use only the Sandboxer MCP tools. Do not run the `sbx` CLI.
- Report the sandbox ID, the commit, and each check's exit status.
- Without the Sandboxer MCP, run the checks locally as this file describes.

## Licenses

`deny.toml` is the gate: its `[licenses] allow` list judges every crate that
ships on a release target. **Adding an id there, or a crate exception, is a
legal decision — do not do it to make CI green.** The first block is the Pwr
family's shared list; ids the Rust port needed follow, each with its reason.

`THIRD_PARTY_LICENSES` is generated by `cargo xtask licenses` from cargo-about
(`about.toml`, `licenses/about.hbs`) plus `licenses/bundled.txt`, which lists
what ships but is not a crate: the npm packages inside Finicky's config API
bundle, the Geist fonts and Velopack's native binaries. After a dependency
change run `cargo xtask licenses` (in a sandbox when the thread has
Sandboxer; see above) and commit the result; when a bundled asset
changes, edit `licenses/bundled.txt` by hand. Keep `about.toml`'s `accepted`
list in step with `deny.toml`.

The crates inside Velopack's binaries come from Velopack's own Cargo.lock, so
`cargo deny` never sees them. `scripts/release/velopack-notices.sh` writes
them to `licenses/velopack.txt` and prints their license ids; rerun it, and
check those ids against `deny.toml`, whenever the vpk version changes.

Dependabot proposes a version only after it is seven days old. `gpui-kit` is
pinned with `=` and upgraded by hand, with a look at the window.

## Launch the app

Before launching, check whether a window titled `PwrRoute` already exists and
reuse it when suitable. The main window's rail foot (and Settings › About)
says which build is running: a dev build's checkout, branch and commit,
each click-to-copy. A dev build runs as a bare binary (`target/*/pwrroute`);
`cargo xtask bundle` makes a real `.app` with bundle id
`com.pwrdrvr.pwrroute`.

The first launch becomes the router and stays running with its windows
closed; a later launch forwards its links (or asks for the window) and exits.
Quit from the app menu, or kill the process, to stop it.

PwrRoute declares `http` and `https` (`packaging/macos/Info.plist.in`,
`packaging/linux/pwrroute.desktop`), so the OS can offer it as a browser;
macOS also needs the `public.html` and `public.xhtml` document types, or
System Settings leaves it out of "Default web browser".
Becoming the default requires an explicit app action or `--make-default`
(PwrAgent's Start action passes it). That flag requests ownership of the
running bundle's path, so a worktree build can replace `/Applications`.
It is ignored with `--dry-run` and under the E2E hook. Never request ownership
from an ordinary launch or test. The E2E registration scenarios run only on
throwaway CI runners (`e2e/AGENTS.md`).

## Packaging & releases

`cargo xtask bundle [--universal] [--sign IDENTITY] [--notarize]` builds the
macOS app, zip and DMG, ad hoc signed unless an identity is given.
`cargo xtask package-linux` builds the tar.gz and .deb. The app icon is
`packaging/icon/PwrRoute.svg`, the design mark on the Pwr family's tile;
`scripts/release/make-icns.sh` renders it into the committed Icon Composer
package `packaging/icon/PwrRoute.icon`, `packaging/macos/PwrRoute.icns` and
`packaging/windows/PwrRoute.ico`, so rerun it and commit all three after
editing the SVG. On macOS 26 the package is the icon: `bundle` compiles it
with Xcode 26's `actool` into `Assets.car` plus a derived `Icon.icns`
(`CFBundleIconName` and `CFBundleIconFile`), because macOS 26 redraws a bare
`.icns` its own way, differently across point releases (PwrSnap's
`docs/solutions/2026-09-05-macos-26-legacy-icon-light-plate.md`). Without
actool 26 `bundle` falls back to `PwrRoute.icns` with a warning; CI and
release builds pass `--require-icon-catalog`, so they fail instead. The DMG
window's background, `packaging/dmg/background.png`, is painted by
`scripts/release/generate-dmg-background.swift` in the vendored Geist Bold
(`packaging/dmg/fonts`) and committed the same way; `macos-dmg.sh` holds the
window layout, and xtask's tests keep the two in step. Windows and macOS
releases install and update through Velopack: release.yml packs them with
`vpk`, installed by `scripts/release/install-vpk.sh` from a pinned version
and SHA-512; that version must equal the `velopack` crate's `=` pin, and
`licenses/velopack.txt` must be regenerated for it (see Licenses). A dev
build is not a Velopack install, so its update check only logs that.
Releases are cut by pushing a `vX.Y.Z` tag that
matches the workspace version in `Cargo.toml` and a `CHANGELOG.md` section
(`cargo xtask release-check --tag vX.Y.Z` verifies). CI, preview builds, and
signing secrets are documented in `.github/workflows/README.md`.

## Pull Requests

- **Don't edit `CHANGELOG.md` in feature or fix PRs.** Every open PR that
  adds a bullet conflicts with every other one. Put the user-facing summary in
  the PR description; the release-prep PR (`chore(release): vX.Y.Z`) writes
  the version's section from the PRs merged since the last tag.
- Prefer before/after screenshots on PRs that change visible UI, with 100%
  contrived fixture data. URLs are private data: never put real browsing
  history, Activity entries or traces in a screenshot.
- Attach screenshots and videos with GitHub CLI 2.99+ `gh pr create/edit/comment
  --attach`. Use byte-identical paths in the Markdown body and `--attach`; for
  later edits, fetch and edit the live body (`gh pr view <n> --json body -q
  .body`) so uploaded `user-attachments` URLs survive. PwrGit's `AGENTS.md`
  has the full procedure.

## Docs & conventions

- **Agent guidance lives in `AGENTS.md`** — not in READMEs or human-facing code
  comments.
- **Progressive disclosure — short and close.** Put guidance in the smallest
  scope that needs it: a nested `AGENTS.md` beside the code it governs, not one
  giant root file.
- **Always symlink `CLAUDE.md → AGENTS.md`.** Whenever you add an `AGENTS.md`,
  create a sibling `CLAUDE.md` symlink to it (`ln -s AGENTS.md CLAUDE.md`) so
  both toolchains read one source of truth.
- **Design mirror.** `design/` is a checked-in copy of the Claude Design
  project; `design/SOURCE.md` says how to re-export it. Do not edit
  `design/support.js`.
