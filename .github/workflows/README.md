# GitHub Actions

The workflows follow PwrGit's shape (required checks, label-gated previews,
signing environments) with disktree's Rust and GPUI build steps. Every build
step runs a `cargo xtask` task, so CI and a local run do the same thing.

| Workflow | Trigger | What it does |
|---|---|---|
| `ci.yml` | push to `main`, PRs | Typecheck (the whole `cargo xtask lint` chain), Build and Test on Linux; a macOS job that tests and bundles the app; a Windows job that runs the tests; Desktop E2E on Linux, macOS and Windows. Non-code-only PRs skip everything after Classify Changes (see below). |
| `preview-build.yml` | `build-preview` PR label, manual dispatch | An ad hoc signed macOS universal DMG and zip (macOS 26) and an unsigned Windows x64 zip, Velopack Setup.exe and MSI, uploaded as workflow artifacts. |
| `release.yml` | `v*` tag push, manual dispatch with a tag | Checks release metadata, lints and tests, builds macOS universal and Windows x64 in unprivileged jobs, packs and signs them with Velopack in credentialed jobs that never check out source, packages Linux x64 + arm64, then creates a GitHub Pre-release from the matching `CHANGELOG.md` section with every platform's assets and the Velopack update feeds. Signing and notarization run only when their credentials are configured (see below). |

Every job pins Rust 1.97, the same version `rust-toolchain.toml` pins. A newer
clippy's new lints are a change to make on purpose. Right after installing
it, every job runs `scripts/ci/prune-rust-toolchains.sh` to uninstall the
runner image's preinstalled toolchains: `Swatinem/rust-cache` hashes every
installed toolchain into its key, and the image's stable moves with each
rollout, so without the prune a PR can land on an image main has not cached
yet and compile cold. Only main saves the cache (`save-if`). Linux jobs install GPUI's
system libraries with `scripts/ci/install-linux-deps.sh`.

## ci.yml

Jobs and the names the "Protect main" ruleset requires:

| Job id | Check name | Runner | Runs |
|---|---|---|---|
| `changes` | Classify Changes | ubuntu-latest | Decides whether the PR is code-impacting. |
| `typecheck` | **Typecheck** (required) | ubuntu-24.04 | `cargo xtask lint`: rustfmt, CI classifier tests, clippy `-D warnings`, cargo-deny, THIRD_PARTY_LICENSES. |
| `build` | **Build** (required) | ubuntu-24.04 | `cargo build --release --locked --workspace`. |
| `test` | **Test** (required) | ubuntu-24.04 | `cargo xtask test`. |
| `macos` | macOS (test + bundle) | self-hosted `pwrdrvr-macos` | Tests, then `cargo xtask bundle --app-only` (ad hoc signed); same-repository PRs and pushes only. |
| `windows` | Windows (test) | windows-2025 | The tests, which compile every module of the app. |
| `e2e-linux` | Linux Desktop E2E | ubuntu-24.04 | `cargo xtask e2e` under Xvfb, with Mesa's software Vulkan. |
| `e2e-macos` | macOS Desktop E2E | self-hosted `pwrdrvr-macos` | `cargo xtask e2e` on a Tart guest; same-repository PRs and pushes only. |
| `e2e-windows` | Windows Desktop E2E | windows-2025 | `cargo xtask e2e`. |
| `desktop-e2e-result` | **Desktop E2E** (to require) | ubuntu-latest | Fails unless every platform's E2E job succeeded (macOS excused on fork PRs). |

The required check names are pinned. Renaming the `Typecheck`, `Build` or
`Test` job leaves that check permanently pending and blocks every PR.
`Typecheck` kept its name from the Electron scaffold; in Rust it is the lint
chain.

The self-hosted macOS job validates the app bundle, including its Info.plist
and ad hoc signature, without creating a DMG. DMG layout drives Finder through
AppleEvents, and Automation access is not guaranteed on the persistent Tart
guests. Preview and release workflows still create the full DMG on
GitHub-hosted macOS runners.

### Desktop E2E

`cargo xtask e2e` builds the app, wraps it in a dev `PwrRoute.app` on macOS,
and runs `e2e/` against it; `e2e/AGENTS.md` has the design. Each platform
uploads `target/e2e-artifacts/` (per-test app logs, state dumps on failure,
window-only screenshots of fixture data) for 30 days.

- **Linux** runs under `xvfb-run` with `mesa-vulkan-drivers` (lavapipe),
  since wgpu finds no GPU on the runner, plus `x11-utils` and ImageMagick
  for the OS-side window checks and screenshots, and `xdg-utils` for the
  default-browser scenarios.
- **macOS** runs on the self-hosted Tart pool shared with PwrGit and
  PwrAgnt, selected by `[self-hosted, macOS, ARM64, pwrdrvr-macos]`. Add
  `pwrdrvr/PwrRoute` to that organization runner group's selected
  repositories, or the job stays queued. Guests need a logged-in desktop.
  Fork PRs skip it so untrusted code never reaches the persistent guests.
  `scripts/ci/macos-e2e-pre-run-cleanup.sh` first stops PwrRoute copies an
  earlier job in this workspace left running and clears the app's saved
  window state. The job then wakes the display, holds it awake with
  `caffeinate` for the run, and `scripts/ci/macos-e2e-require-desktop.sh`
  fails it at once if the runner's session is locked or off the console:
  GPUI draws no window the platform does not present, so behind the lock
  screen every test that needs a redraw would time out. A locked guest needs
  its screen lock and screensaver off, which is the lab's to set.
- **Windows** needs nothing extra: the hosted image has a desktop session.

Hosted runners set `PWRROUTE_E2E_DISPOSABLE=1`; the Tart guests persist, so
they do not. Only a disposable machine runs the scenarios in which the app
registers itself with the OS and a link arrives through the OS's handler for
web links (`e2e/CONTRACT.md`, "Registration"): on Linux through `xdg-open`,
on Windows through the registered ProgID command. Elsewhere they skip.

The check to require is `Desktop E2E`, the `desktop-e2e-result` aggregate,
not any platform job: a skipped required check counts as satisfied, so a
fork-guarded job would gate nothing, and naming per-platform jobs binds the
ruleset to the job list. Adding a platform is a change to the aggregate's
`needs`. `Desktop E2E` is not in the "Protect main" ruleset yet; add it once
it is green on `main`.

### Non-code-only PRs

`ci.yml` retains the paginated, path-based classifier used by PwrAgnt and
extends PwrRoute's allowlist. Expensive CI skips only when **every** changed
path, including `previous_filename` on a rename, matches one of:

- `docs/**` or a Markdown file (`.md`, case-insensitive), except
  `CHANGELOG.md`, which the app embeds at compile time.
- `.agents/skills/**`, including `SKILL.md`, `agents/openai.yaml`, references,
  scripts and other skill assets. These files are agent resources, not app
  or CI inputs. If a build or workflow starts consuming them, update the
  classifier before introducing that dependency.
- `.github/ISSUE_TEMPLATE/**`, `.github/PULL_REQUEST_TEMPLATE/**`, or exactly
  `.github/dependabot.yml`.

For example, PR #93's release skill has three paths: `SKILL.md`,
`agents/openai.yaml` and `references/publishing.md`, all under
`.agents/skills/release/`. All three qualify, individually or together.
Adding a Rust source, dependency manifest, build script, packaging asset or
workflow change to that PR selects full CI. Unknown paths also run full CI.
A rename qualifies only if both paths qualify; missing rename metadata runs
full CI.

Push events always run full CI. API errors, empty or incomplete pagination
(including GitHub's 3,000-file limit), invalid counts, malformed file entries
and duplicate results also run full CI. A script failure or missing output
cannot opt out: the job output defaults to full CI unless the classifier
successfully emits `false`. Downstream jobs also run if the classification
job itself fails, while retaining the macOS self-hosted fork guard.

Do not add `paths` or `paths-ignore` to the `pull_request` trigger. Required
status checks must still exist on every PR: skipped jobs satisfy them, missing
jobs leave them pending. The effective main ruleset requires `Typecheck`,
`Build` and `Test`; their names and the `Desktop E2E` aggregate are preserved.
No override or ruleset change is needed for non-code PRs.

`node --test scripts/ci/classify-changes.test.cjs` runs fixtures against the
actual inline classifier and job conditions, without GitHub credentials or
npm dependencies. `cargo xtask lint` includes this check; Node.js 22 or newer
must be on `PATH` (the GitHub runners already provide it).

## preview-build.yml

Same label gate as PwrGit: applying `build-preview` starts a run, and later
pushes to a labeled PR re-run it. `workflow_dispatch` packages a branch
without a PR. PR builds are versioned `<base>-pr.<number>`, dispatched builds
`<base>-dev.<run number>`. The version reaches xtask through
`PWRROUTE_VERSION`; `Cargo.toml` is never edited, so `--locked` holds.

No secrets reach this workflow. macOS previews are ad hoc signed and not
notarized, so a downloaded copy needs "Open Anyway" once. They are
`cargo xtask bundle` output, without Velopack's updater. The Windows zip,
`PwrRoute-win-Setup.exe` and `PwrRoute-win.msi` are unsigned, so SmartScreen
warns; Setup.exe comes from the same `vpk pack` arguments as the release,
minus signing, release notes and deltas.

## release.yml

Cutting a release: bump `version` in `[workspace.package]` of the root
`Cargo.toml`, add a matching `## vX.Y.Z` section to `CHANGELOG.md`, then push
the `vX.Y.Z` tag. `cargo xtask release-check --tag vX.Y.Z` verifies the tag,
version and changelog section before anything is built. No release is created
until macOS, Windows and both Linux jobs succeed. Releases are born as GitHub
Pre-releases; promote to Latest manually after validation
(`gh release edit vX.Y.Z --latest --prerelease=false`, suffix-free tags only).
Release tags are protected by the "Protect v* release tags" ruleset (signed
tags only).

The tag decides which release channel slot a release lands in (the app's
Settings › Updates; `crates/pwrroute-app/src/update_channel.rs`), as in
PwrSnap, PwrGit and PwrAgnt:

| Slot | Tag | |
|---|---|---|
| Stable · Latest | `v0.1.0`, promoted | The default. Keeps the previous promoted release until the next one is promoted. |
| Stable · Prerelease | `v0.1.1` not yet promoted, `v0.1.1-rc.1`, `v0.1.1-prerelease.1` | Candidates for the stable line. |
| Beta · Latest | `v0.2.0-beta.3` | Beta builds off main. |
| Beta · Prerelease | `v0.2.0-alpha.7` | Newest alpha off main. |

Every suffixed tag stays a Pre-release. A beta or alpha counts for the Beta
train only while its version is ahead of the promoted release; once that
version is promoted, Beta falls back to it.

| Platform | Built on | Assets |
|---|---|---|
| macOS | macos-26, both architectures, joined with `lipo`; packed by `vpk` | `PwrRoute-<v>-universal.dmg`, `PwrRoute-osx-Portable.zip`, `PwrRoute-<v>-osx-full.nupkg` (and `-delta`), `releases.osx.json`, `assets.osx.json`, `RELEASES-osx`, `PwrRoute-<v>-macos-SHA256SUMS` |
| Windows | windows-2025, static C runtime; packed by `vpk` | `PwrRoute-win-Setup.exe`, `PwrRoute-win.msi`, `PwrRoute-win-Portable.zip`, `PwrRoute-<v>-full.nupkg` (and `-delta`), `releases.win.json`, `assets.win.json`, `RELEASES`, `PwrRoute-<v>-windows-SHA256SUMS` |
| Linux | ubuntu-22.04 and ubuntu-22.04-arm, for the oldest glibc floor | `PwrRoute-<v>-linux-<arch>.tar.gz`, `.deb`, `-SHA256SUMS` |

Velopack's installer and portable names carry no version, so
`releases/latest/download/PwrRoute-win-Setup.exe` is a stable link once a
release is promoted to Latest. The publish job downloads each platform's
artifact into its own directory and fails if two platforms produced the same
file name, rather than letting one overwrite the other.

### Jobs

Signed platforms follow PwrGit's hand-off: a prepare job with the checkout
and the toolchain builds and uploads a signing-input archive, and passes its
SHA-256 as a job output; the sign job, the only one in the signing
environment, downloads it, checks both the job output and the archive's
`.sha256`, and works from the archive alone. It never checks out source,
runs cargo or installs project dependencies.

| Job | Environment | Does |
|---|---|---|
| `distribution-audit` | — | Stub (see below). |
| `typecheck` | — | `cargo xtask release-check` and `lint` on Linux. |
| `test` | — | `cargo xtask test` on Linux, beside `typecheck`. Both restore main's CI cache, so their job ids, env and toolchain steps match `ci.yml`'s. |
| `macos-prepare` | — | `cargo xtask bundle --universal --app-only --require-icon-catalog`; archives `PwrRoute.app`, `PwrRoute.entitlements`, `PwrRoute.icns`, the DMG's `background.png`, `macos-dmg.sh`, `install-vpk.sh` and the release notes. |
| `macos-sign` | apple-signing | `vpk pack`, then the DMG from the packed app; verifies. |
| `windows-prepare` | — | `cargo build --release`; archives `pwrroute.exe`, README, LICENSE, THIRD_PARTY_LICENSES, `install-vpk.sh`, `vpk-pack-windows.ps1`, `PwrRoute.ico` and the release notes. |
| `windows-sign` | windows-signing | `vpk pack` with Setup.exe and MSI; verifies. |
| `linux` | — | `cargo xtask package-linux` on x64 and arm64. |
| `publish` | — (`contents: write`) | Creates the Pre-release with every platform's assets. |
| `distribution-audit-after` | — | Stub (see below). |

### Velopack

PwrRoute installs and updates through [Velopack](https://docs.velopack.io).
The app runs Velopack's startup hooks first in `main` and checks the GitHub
Releases feed (`crates/pwrroute-app/src/updater.rs`). The `vpk` CLI is a
.NET tool. The sign jobs run vpk beside the signing credentials, so
`scripts/release/install-vpk.sh` installs it only from a nupkg whose
SHA-512 matches the one pinned in the script (NuGet's catalog hash), after
`actions/setup-dotnet` installs .NET 8. The script's `VPK_VERSION` must
equal the `velopack` crate's `=` pin; upgrade both, and the hash, together,
then rerun `scripts/release/velopack-notices.sh` for the license notices.
`scripts/release/vpk-pack-windows.ps1` holds the Windows `vpk pack` flags,
shared by `windows-sign` and the preview.

- **Feed.** Each release carries `releases.<channel>.json` (`win`, `osx`)
  and its own packages. The app reads the release list from GitHub's API
  (`github_releases.rs`; paging until it has seen GitHub's latest release),
  picks the release of the slot in force, and points Velopack's HTTP source
  at that release's download directory, so it installs exactly that version.
  Before packing, each sign job fetches the newest published full package
  (`vpk download github --pre`) so `vpk` can write a delta against it; a
  delta applies when the install is that previous version, and otherwise
  Velopack downloads the full package. The first release packs full only.
- **Release channel.** An install follows the slot its version implies (no
  suffix: Stable · Latest; `-rc`: Stable · Prerelease; `-beta`: Beta ·
  Latest; `-alpha`: Beta · Prerelease) until a slot is picked in Settings ›
  Updates (`settings.json`'s `updates`). Automatic checks only move forward.
  Check now may download an older release for the slot picked; that one
  installs only from Restart to switch, never on quit. A download is
  recorded with its slot (`updates.json` in the data directory) and is
  neither shown nor installed after another slot is picked.
- **Private repository.** The app reads releases without a token, so updates
  need the repository (or a release mirror) to be public. Until then a check
  fails with HTTP 404.
- **Windows.** Installs per user into `%LocalAppData%\PwrRoute` with a Start
  menu shortcut. The MSI is `--instLocation PerUser`: browser registration is
  HKCU, and vpk's default dual-scope MSI preselects per-machine. Install and
  update hooks call `register_browser()`, uninstall `unregister_browser()`;
  registering never changes the default browser.
- **macOS.** `vpk` adds `UpdateMac` and `sq.version` to the app, so only the
  packed app can update; the DMG is cut from it. `macos-dmg.sh` lays the DMG
  window out like PwrSnap's (background, icon positions, volume icon, volume
  `PwrRoute <v>-universal`) by driving Finder through `osascript`, so the
  job needs the runner's GUI session; it retries Finder five times and fails
  with a message rather than shipping an unstyled DMG. vpk signs with
  `packaging/macos/PwrRoute.entitlements` (none): vpk's default allows JIT,
  unsigned executable memory and `DYLD_*` variables and turns off library
  validation. No `.pkg` installer (`--noInst`): it needs a Developer ID
  Installer certificate, which apple-signing does not hold.
- **Linux.** Not used. `vpk`'s only Linux output is an AppImage, and its
  updater only runs from one. There is no apt repository either, so `.deb`
  and tar.gz installs update only by downloading the next release.

### Signing setup (secrets and variables)

Two GitHub Environments, as in PwrGit. The secret names did not change in the
Rust port. Add required reviewers and a `v*` tag deployment policy before
adding secrets. Each sign job detects its credentials first; with any missing
it packs anyway, unsigned (ad hoc on macOS), and says so in a warning.

- **apple-signing** — `CSC_LINK` (Developer ID .p12, base64),
  `CSC_KEY_PASSWORD`, `APPLE_API_KEY_BASE64` (App Store Connect .p8, base64),
  `APPLE_API_KEY_ID`, `APPLE_API_ISSUER`. With all of them present
  `macos-sign` imports the certificate into a throwaway keychain, picks the
  `Developer ID Application` identity for team `T44CNHC4UH` and stores the
  API key there as the notarytool profile `pwrroute-notary`. `vpk pack` signs
  the app with the hardened runtime, notarizes and staples it;
  `macos-dmg.sh` signs, notarizes and staples the DMG. The job then checks
  `codesign --verify`, the team identifier, `xcrun stapler validate` and
  `spctl --assess` for both.
  Upload them from 1Password instead of pasting: with the `op` and `gh` CLIs
  signed in, `OP_ACCOUNT=<user id from op account list> scripts/release/upload-csc-link-from-1password.sh`
  reads the "Apple Signing - PwrDrvr" item in the "PwrDrvr LLC" vault (the
  .p12 and AuthKey .p8 attachments, the password, key id and issuer) and sets
  the five secrets on this repository's `apple-signing` environment, which
  must exist first. The script is PwrGit's with the repository name changed.
- **windows-signing** — environment variables
  `WIN_AZURE_SIGN_PUBLISHER_NAME`, `WIN_AZURE_SIGN_ENDPOINT`,
  `WIN_AZURE_SIGN_ACCOUNT`, `WIN_AZURE_SIGN_PROFILE`; environment secrets
  `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`. With all of them
  present `windows-sign` writes the Artifact Signing `metadata.json` and
  passes it to `vpk pack --azureTrustedSignFile`; vpk ships signtool and the
  Azure dlib, which reads the service principal from `AZURE_*`. It signs
  `pwrroute.exe`, `Update.exe` and the launcher stub before packing, then
  `PwrRoute-win-Setup.exe` and `PwrRoute-win.msi`. The job checks each one's
  Authenticode status and signer.

Optional repository secret `RELEASES_PAT` overrides `github.token` for creating
the release. The sign jobs read the previous release with `github.token`.

### Not in release.yml yet

- **Winget and Homebrew.** `distribution-audit` and
  `distribution-audit-after` are stubs that print a notice. PwrGit's
  `package-distribution.yml` compares both package managers with the release
  before and after publication; port it when PwrRoute has a manifest in
  either.
- **PR signing smoke runs.** PwrGit runs its Windows sign job on PRs labeled
  `ci:windows-signing`. Not ported; dispatch `release.yml` with a test tag
  instead.
- **Windows arm64.** disktree builds it on `windows-11-arm`; add it when there
  is a user for it. vpk would pack it as its own channel.
- **Icons for pwrroute.exe and the Linux .deb.** `vpk pack --icon` covers
  Setup.exe, the MSI and the shortcut; the executable has no icon resource,
  and the desktop entry no `Icon=`.
- **Update UI.** A log line and the macOS "Check for Updates…" menu item
  until the update design lands.

## PR labels

Keep label names namespaced when they start, skip, or narrow CI work.

| Label | Workflow | Effect |
|---|---|---|
| `build-preview` | `preview-build.yml` | Builds the macOS DMG and the Windows zip, Setup.exe and MSI for the PR. Applied label triggers a run; later pushes to a labeled PR re-run it. |

If you add another label-influenced workflow path, document it here in the
same change as the workflow update.
