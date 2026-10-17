# GitHub Actions

The workflows follow PwrGit's shape (required checks, label-gated previews,
signing environments) with disktree's Rust and GPUI build steps. Every build
step runs a `cargo xtask` task, so CI and a local run do the same thing.

| Workflow | Trigger | What it does |
|---|---|---|
| `ci.yml` | push to `main`, PRs | Typecheck (the whole `cargo xtask lint` chain), Build and Test on Linux; a macOS job that tests and bundles the app; a Windows job that checks the notice and runs the tests. Docs/Dependabot-only PRs skip everything after Classify Changes (see below). |
| `preview-build.yml` | `build-preview` PR label, manual dispatch | An ad hoc signed macOS universal DMG and zip (macOS 26) and an unsigned Windows x64 zip, uploaded as workflow artifacts. |
| `release.yml` | `v*` tag push, manual dispatch with a tag | Checks release metadata, lints and tests, packages macOS universal, Windows x64 and Linux x64 + arm64, then creates a GitHub Pre-release from the matching `CHANGELOG.md` section with every platform's assets. Signing and notarization run only when their credentials are configured (see below). |

Every job pins Rust 1.97, the same version `rust-toolchain.toml` pins. A newer
clippy's new lints are a change to make on purpose. Linux jobs install GPUI's
system libraries with `scripts/ci/install-linux-deps.sh`.

## ci.yml

Jobs and the names the "Protect main" ruleset requires:

| Job id | Check name | Runner | Runs |
|---|---|---|---|
| `changes` | Classify Changes | ubuntu-latest | Decides whether the PR is code-impacting. |
| `typecheck` | **Typecheck** (required) | ubuntu-24.04 | `cargo xtask lint`: rustfmt, clippy `-D warnings`, cargo-deny, THIRD_PARTY_LICENSES. |
| `build` | **Build** (required) | ubuntu-24.04 | `cargo build --release --locked --workspace`. |
| `test` | **Test** (required) | ubuntu-24.04 | `cargo xtask test`. |
| `macos` | macOS (test + bundle) | macos-26 | Tests, then `cargo xtask bundle` (ad hoc signed). |
| `windows` | Windows (test) | windows-2025 | `cargo xtask licenses --check`, then the tests, which compile every module of the app. |

The required check names are pinned. Renaming the `Typecheck`, `Build` or
`Test` job leaves that check permanently pending and blocks every PR.
`Typecheck` kept its name from the Electron scaffold; in Rust it is the lint
chain.

### Docs/Dependabot-only PRs

`ci.yml` classifies each pull request before the expensive jobs. If every
changed path (and `previous_filename` on a rename) is under `docs/` or ends
with `.md` (case-insensitive), or is exactly `.github/dependabot.yml`,
Classify Changes sets `code_impacting=false` and every other job skips. Push
events, API errors and incomplete file lists still run full CI.

Do not add `paths` or `paths-ignore` to the `pull_request` trigger. Required
status checks must still exist on every PR: skipped jobs satisfy them, missing
jobs leave them pending.

## preview-build.yml

Same label gate as PwrGit: applying `build-preview` starts a run, and later
pushes to a labeled PR re-run it. `workflow_dispatch` packages a branch
without a PR. PR builds are versioned `<base>-pr.<number>`, dispatched builds
`<base>-dev.<run number>`. The version reaches xtask through
`PWRROUTE_VERSION`; `Cargo.toml` is never edited, so `--locked` holds.

No secrets reach this workflow. macOS previews are ad hoc signed and not
notarized, so a downloaded copy needs "Open Anyway" once. The Windows zip is
unsigned.

## release.yml

Cutting a release: bump `version` in `[workspace.package]` of the root
`Cargo.toml`, add a matching `## vX.Y.Z` section to `CHANGELOG.md`, then push
the `vX.Y.Z` tag. `cargo xtask release-check --tag vX.Y.Z` verifies the tag,
version and changelog section before anything is built. No release is created
until macOS, Windows and both Linux jobs succeed. Releases are born as GitHub
Pre-releases; promote to Latest manually after validation. Release tags are
protected by the "Protect v* release tags" ruleset (signed tags only).

| Platform | Built on | Assets |
|---|---|---|
| macOS | macos-26, both architectures, joined with `lipo` | `PwrRoute-<v>-universal.dmg`, `PwrRoute-<v>-universal-mac.zip` |
| Windows | windows-2025, static C runtime | `PwrRoute-<v>-windows-x64.zip` and its `.sha256` |
| Linux | ubuntu-22.04 and ubuntu-22.04-arm, for the oldest glibc floor | `PwrRoute-<v>-linux-<arch>.tar.gz`, `.deb`, `-SHA256SUMS` |

### Signing setup (secrets and variables)

Two GitHub Environments, as in PwrGit. The secret names did not change in the
Rust port. Add required reviewers and a `v*` tag deployment policy before
adding secrets.

- **apple-signing** — `CSC_LINK` (Developer ID .p12, base64),
  `CSC_KEY_PASSWORD`, `APPLE_API_KEY_BASE64` (App Store Connect .p8, base64),
  `APPLE_API_KEY_ID`, `APPLE_API_ISSUER`. With all of them present the macOS
  job imports the certificate into a throwaway keychain, picks the
  `Developer ID Application` identity for team `T44CNHC4UH`, signs with the
  hardened runtime, notarizes and staples the app and the DMG, then verifies
  with `codesign --verify`, `xcrun stapler validate` and `spctl --assess`.
- **windows-signing** — environment variables
  `WIN_AZURE_SIGN_PUBLISHER_NAME`, `WIN_AZURE_SIGN_ENDPOINT`,
  `WIN_AZURE_SIGN_ACCOUNT`, `WIN_AZURE_SIGN_PROFILE`; environment secrets
  `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`. With all of them
  present the Windows job installs TrustedSigning, signs `pwrroute.exe`
  through Azure Artifact Signing and verifies its Authenticode signer.

Optional repository secret `RELEASES_PAT` overrides `github.token` for creating
the release.

### Not in release.yml yet

- **A Windows installer.** The Electron scaffold built NSIS; Windows ships a
  zip of the signed exe until an installer (MSI or NSIS) is chosen.
- **Windows arm64.** disktree builds it on `windows-11-arm`; add it when there
  is a user for it.
- **An app icon.** The design draws placeholder tiles only.
- **The signing-input hand-off** between an unprivileged build job and a
  credentialed signing job that never checks out source, as in PwrGit. Worth
  restoring before the first signed release.
- **Updater manifests, Homebrew and Winget.**

## PR labels

Keep label names namespaced when they start, skip, or narrow CI work.

| Label | Workflow | Effect |
|---|---|---|
| `build-preview` | `preview-build.yml` | Builds the macOS DMG and Windows zip for the PR. Applied label triggers a run; later pushes to a labeled PR re-run it. |

If you add another label-influenced workflow path, document it here in the
same change as the workflow update.
