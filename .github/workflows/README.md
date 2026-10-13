# GitHub Actions

The workflows are adapted from PwrGit's and reduced to what a scaffold can
run. Each section below says what was kept and what returns with the app port.

| Workflow | Trigger | What it does |
|---|---|---|
| `ci.yml` | push to `main`, PRs | Typecheck (the whole `pnpm lint` chain), Build and Test on Linux, plus one Windows job that runs license checks, typecheck, build and test. Docs/Dependabot-only PRs skip the expensive jobs after Classify Changes (see below). |
| `preview-build.yml` | `build-preview` PR label, manual dispatch | Unsigned macOS universal + arm64 DMGs and ZIPs (macOS 26) and an unsigned Windows NSIS installer, uploaded as workflow artifacts. |
| `release.yml` | `v*` tag push, manual dispatch with a tag | Checks release metadata, lints and tests, packages macOS, Windows and Linux (x64 + arm64), then creates a GitHub Pre-release from the matching `CHANGELOG.md` section with every platform's assets. Signing and notarization run only when their credentials are configured (see below). |

## ci.yml

Jobs and the names the "Protect main" ruleset requires:

| Job id | Check name | Runner | Runs |
|---|---|---|---|
| `changes` | Classify Changes | ubuntu-latest | Decides whether the PR is code-impacting. |
| `install-deps` | Install Dependencies | ubuntu-latest | Primes the dependency cache (`lookup-only`). |
| `windows-install-deps` | Install Dependencies (Windows) | windows-latest | Same, for the Windows cache key. |
| `typecheck` | **Typecheck** (required) | ubuntu-latest | `pnpm lint`: `lint:syntax` → `licenses:check` → `typecheck`. |
| `build` | **Build** (required) | ubuntu-latest | `pnpm run build` (electron-vite). |
| `test` | **Test** (required) | ubuntu-latest | `pnpm run test` (vitest). |
| `windows` | Windows (typecheck + build + test) | windows-latest | `licenses:check`, typecheck, build, test. |

The required check names are pinned. Renaming the `Typecheck`, `Build` or
`Test` job leaves that check permanently pending and blocks every PR.

PwrGit runs Typecheck, Build and Test on Linux and repeats them in one Windows
job; macOS appears in its CI only as Desktop E2E on self-hosted Tart guests.
This repo matches that. Desktop E2E (Linux, macOS and Windows shards plus the
`Desktop E2E` aggregate check) is left out until there is UI worth driving;
when it returns, add `Desktop E2E` to the ruleset in the same change.

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
pushes to a labeled PR re-run it. `workflow_dispatch` is an addition so
packaging can be exercised from a branch before there are PRs; dispatched
builds are versioned `<base>-dev.<run number>`, PR builds `<base>-pr.<number>`.

No secrets reach this workflow. macOS previews are ad hoc signed (identity
`-`) with hardened runtime off and are not notarized; the Windows installer is
unsigned.

## release.yml

Cutting a release: bump `apps/desktop/package.json` version, add a matching
`## vX.Y.Z` section to `CHANGELOG.md`, then push the `vX.Y.Z` tag.
`pnpm release:check` (scripts/check-desktop-release-metadata.mjs) verifies the
tag, package version, `CHANGELOG.md` section, `electronVersion` pin and
prerelease publish type. No release is created until macOS, Windows and both
Linux jobs succeed. Releases are born as GitHub Pre-releases; promote to Latest
manually after validation. Release tags are protected by the "Protect v*
release tags" ruleset (signed tags only).

Packaging goes through `apps/desktop/scripts/package.mjs`, which chooses
signed or unsigned from the environment. Each signing step in the workflow is
gated on a detection step, and an unsigned lane emits a warning annotation, so
adding the secrets below is all it takes to turn signing on. The signed steps
pass `--require-signing`, so partial configuration fails instead of shipping
an unsigned build.

### Signing setup (secrets and variables)

Two GitHub Environments, as in PwrGit. Add required reviewers and a `v*` tag
deployment policy before adding secrets.

- **apple-signing** — `CSC_LINK` (Developer ID .p12, base64),
  `CSC_KEY_PASSWORD`, `APPLE_API_KEY_BASE64` (App Store Connect .p8, base64),
  `APPLE_API_KEY_ID`, `APPLE_API_ISSUER`. With all of them present the macOS
  job signs, notarizes, then verifies with `codesign --verify` and
  `xcrun stapler validate`. The team id is `T44CNHC4UH`.
- **windows-signing** — environment variables
  `WIN_AZURE_SIGN_PUBLISHER_NAME`, `WIN_AZURE_SIGN_ENDPOINT`,
  `WIN_AZURE_SIGN_ACCOUNT`, `WIN_AZURE_SIGN_PROFILE`; environment secrets
  `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`. With all of them
  present the Windows job installs TrustedSigning, signs through Azure
  Artifact Signing and verifies Authenticode on the app and installer.

Optional repository secret `RELEASES_PAT` overrides `github.token` for creating
the release.

### Left out of release.yml until the port

PwrGit's release pipeline does more, and each piece returns when the thing it
protects exists:

- The signing-input archive and SHA-256 hand-off between an unprivileged
  prepare job and a credentialed job that never checks out source. Worth
  restoring before the first signed release.
- The pnpm-deploy release stage, asar content audits, native-module staging
  and the macOS executable UUID hook (`release.mjs`). Nothing native or
  unbundled ships yet.
- The Icon Composer app icon and `select-xcode-for-actool`. There is no app
  icon yet; the design draws placeholder tiles.
- Linux RPM and pacman packages, the installed-DEB smoke test and updater
  manifests. The stub ships DEB and tar.gz with a SHA-256 manifest.
- Stable-name download aliases, resumable draft publication and the Homebrew
  and Winget distribution audits.
- PR label lanes (`ci:windows-signing`, `ci:linux-packages`).

## PR labels

Keep label names namespaced when they start, skip, or narrow CI work.

| Label | Workflow | Effect |
|---|---|---|
| `build-preview` | `preview-build.yml` | Builds the unsigned macOS DMGs and Windows installer for the PR. Applied label triggers a run; later pushes to a labeled PR re-run it. |

If you add another label-influenced workflow path, document it here in the
same change as the workflow update.
