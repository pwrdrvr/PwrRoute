---
name: release
description: Prepare a PwrRoute release PR with version metadata, changelog and verification, or monitor a tagged release and hand off publication steps to the user. Use for cutting PwrRoute releases; ordinary feature and fix PRs do not use this workflow.
---

# PwrRoute release

Prepare and verify releases for `pwrdrvr/PwrRoute`. The agent opens the
release-prep PR and monitors CI. The user merges, creates/pushes/moves release
tags, approves signing, changes environment policies and promotes releases.
Do not perform those user-owned actions or treat a request to prepare a
release as authorization for them. Hand off exact commands when ready.

For preparation, follow this file. After merge, or when asked to monitor an
existing tag, read [Publishing and recovery](references/publishing.md).
Resume at the current stage; do not recreate an existing release PR or tag.

## Establish the release scope

Read the applicable `AGENTS.md` files, `.github/PULL_REQUEST_TEMPLATE.md`,
`~/.codex/pr-style.md`, `.github/workflows/release.yml` and its adjacent
`README.md`. The checked-in workflow and xtask implementation are authoritative
for build behavior; the user-owned steps above remain the operating boundary.
Reference release PRs: [#83](https://github.com/pwrdrvr/PwrRoute/pull/83)
and [#86](https://github.com/pwrdrvr/PwrRoute/pull/86).

Fetch main and tags, identify the previous release tag on the intended release
line, and review the squash-merged PRs since it. Do not choose the highest tag
blindly when stable and prerelease lines coexist. Substitute concrete values
for `vPREV` and `X.Y.Z` in the commands below.

```bash
git fetch origin --tags
git log --oneline vPREV..origin/main
```

Read included PR descriptions and relevant diffs to establish user-visible
changes and measured results. Exclude changes already covered by the previous
release. Confirm the target version with the user if the request and context
do not establish it; continue inspecting scope while awaiting that answer.

## Prepare an isolated release PR

Create a worktree from fresh `origin/main` on `chore/release-vX.Y.Z`. Preserve
unrelated work in existing checkouts. For a normal version-only release, the
expected diff is exactly these six files:

| File | Change |
| --- | --- |
| `Cargo.toml` | Set `[workspace.package].version` to `X.Y.Z`. |
| `Cargo.lock` | Refresh with `cargo metadata --format-version 1` (discard or redirect its JSON output); only workspace package versions should change. |
| `THIRD_PARTY_LICENSES` | Regenerate with `cargo xtask licenses`; it embeds workspace crate versions. |
| `crates/pwrroute-app/src/settings/model.rs` | Update the two doc-comment examples: `"X.Y.Z"` and `"X.Y.Z · release · arm64"`. |
| `crates/pwrroute-app/src/settings/system_host.rs` | Update the `"X.Y.Z · release · arm64"` doc-comment example. |
| `CHANGELOG.md` | Add the release section described below. |

Do not add `--no-deps` to the metadata command: it skips refreshing the
lockfile. Inspect the diff rather than globally replacing version strings.
Leave fixed test values in `settings/fixtures.rs`, `updater.rs` tests and other fixtures
alone. Unexpected dependency or file changes need investigation before the PR;
do not fold unrelated fixes into the release bump. Never widen license
allow-lists to get a release through verification.

Only the release PR writes `CHANGELOG.md`. Add
`## vX.Y.Z - YYYY-MM-DD` directly beneath `## Unreleased`, leaving Unreleased
empty. Use the actual preparation date. Write one bullet per user-facing
change as `- Name - what changed, in user terms.` Name affected screens and
settings, describe what the user sees, and include measured numbers when
supported by evidence. Omit PR numbers and private browsing URLs. This section
becomes the GitHub release notes. If a feature PR left an Unreleased bullet,
reconcile it into this section without duplicating it.

## Verify and open the PR

Run in the release worktree:

```bash
RELEASE_TAG=vX.Y.Z cargo xtask release-check
cargo xtask lint
cargo xtask test
```

The first command must report `release metadata agrees for vX.Y.Z`. Lint runs
rustfmt, clippy with `-D warnings`, cargo-deny and the license freshness check.
Record actual results and test counts; the 865 tests in v0.2.2 are historical,
not a target. Report failures or unavailable checks without claiming success.

Inspect the final diff, commit without attribution lines, push the preparation
branch and open the PR against main:

- Title: `chore(release): vX.Y.Z`.
- **Summary:** name the version bump and metadata files; describe the changelog
  coverage with one sub-bullet per included PR, its `#number` and a one-line
  description. PR numbers belong here even though they do not belong in the
  changelog.
- **Verification:** list the three commands and observed results.
- **Notes:** record untested behavior or remaining manual checks and include
  the concrete user-run promotion command from the publishing reference.

Follow the repo PR template and user's PR style. Monitor required checks
(currently Typecheck, Build and Test under Protect main) and report failures
with the failing job and log. Do not merge. Hand the user the PR link and
verification status. Continue to the publishing reference only once the user
has merged or when explicitly monitoring an existing release.
