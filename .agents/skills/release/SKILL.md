---
name: release
description: Prepare and verify a PwrRoute release, commit and push it directly to main, create its release tag, and monitor publication. Use for cutting PwrRoute releases; ordinary feature and fix PRs do not use this workflow.
---

# PwrRoute release

Prepare and verify releases for `pwrdrvr/PwrRoute`. A request to make a release
authorizes the agent to commit and push its preparation directly to `main`,
then create and push the new release tag. Do not open a release-prep PR.
The user takes over at `apple-signing` approval and owns environment policy
changes, moving existing tags, recovery reruns and promotion to Latest.

For preparation, follow this file. After pushing main, or when monitoring an
existing tag, read [Publishing and recovery](references/publishing.md).
Resume at the current stage; reuse verified preparation and inspect existing
commits and tags before creating anything.

## Establish the release scope

Read the applicable `AGENTS.md` files, `.github/workflows/release.yml` and
its adjacent `README.md`. The checked-in workflow and xtask implementation
are authoritative for build behavior. Older release PRs are historical
examples of metadata and changelog coverage, not the current delivery process.

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

## Prepare the release commit

Use a clean, up-to-date `main` checkout. An isolated worktree is useful when
the main checkout has unrelated work; it does not require a preparation branch
or PR. Preserve unrelated changes. For a normal version-only release, the
expected diff is exactly these six files:

| File | Change |
| --- | --- |
| `Cargo.toml` | Set `[workspace.package].version` to `X.Y.Z`. |
| `Cargo.lock` | Refresh with `cargo metadata --format-version 1` (discard or redirect its JSON output); only workspace package versions should change. |
| `THIRD_PARTY_LICENSES` | Regenerate with `cargo xtask licenses`; it embeds workspace crate versions. |
| `crates/pwrroute-app/src/settings/model.rs` | Update the current-version doc examples, including the Updates, Diagnostics and About examples. |
| `crates/pwrroute-app/src/settings/system_host.rs` | Update the `"X.Y.Z · release · arm64"` doc-comment example. |
| `CHANGELOG.md` | Add the release section described below. |

Do not add `--no-deps` to the metadata command: it skips refreshing the
lockfile. Inspect the diff rather than globally replacing version strings.
Leave fixed test values in `settings/fixtures.rs`, `updater.rs` tests and other fixtures
alone. Unexpected dependency or file changes need investigation before pushing;
do not fold unrelated fixes into the release bump. Never widen license
allow-lists to get a release through verification.

Only release preparation writes `CHANGELOG.md`; feature and fix PRs leave it alone. Add
`## vX.Y.Z - YYYY-MM-DD` directly beneath `## Unreleased`, leaving Unreleased
empty. Use the actual preparation date. Write one bullet per user-facing
change as `- Name - what changed, in user terms.` Name affected screens and
settings, describe what the user sees, and include measured numbers when
supported by evidence. Omit PR numbers and private browsing URLs. This section
becomes the GitHub release notes. If a feature PR left an Unreleased bullet,
reconcile it into this section without duplicating it.

## Verify and push main

Verify the release metadata and run the lint and test commands below. When
Sandboxer is available, follow the repository's
[Sandboxer verification skill](../sandboxer-verification/SKILL.md) for heavy
checks and notice generation: verify a committed tree in the VM, then bring
back generated notices and verify the final commit.

```bash
RELEASE_TAG=vX.Y.Z cargo xtask release-check
cargo xtask lint
cargo xtask test
```

The first command must report `release metadata agrees for vX.Y.Z`. Lint runs
rustfmt, clippy with `-D warnings`, cargo-deny and the license freshness check.
Record actual results and test counts; the 865 tests in v0.2.2 are historical,
not a target. Report failures or unavailable checks without claiming success.

Inspect the final diff and commit as `chore(release): vX.Y.Z`, without
attribution lines. Refresh `origin/main` before pushing. If it advanced,
reconcile the added commits with the changelog and repeat affected checks;
do not force-push or silently release unreviewed additions.

Apply already verified preparation to clean, current main with a fast-forward
or cherry-pick as appropriate. Confirm the release files still match the
verified tree, then `git push origin main`. No PR, merge, or PR CI watch is
part of this process. If a repository policy rejects the push, report the
rejection and stop rather than changing policy or opening a PR automatically.

Report the release commit, included changes and verification results. Stop
the verification sandbox after checks; because this release has no PR, clean
up this thread's sandbox explicitly when no longer needed. Continue to
[Publishing and recovery](references/publishing.md) to tag the exact pushed
commit and hand off signing approval.
