# Publishing and recovery

Read after the verified release preparation is pushed directly to main, or
when monitoring an existing release. The agent creates the initial release
tag and monitors the run. The user approves signing, changes environment
policies, moves existing tags, executes recovery reruns and promotes.

## Create the release tag

Confirm preparation checks passed and the release commit is present on
`origin/main`. Pull clean main with `git pull --ff-only origin main`, then
verify its exact commit, workspace version and nonempty changelog section.
If main advanced beyond the verified release commit, reconcile the additional
changes and verify them before releasing. If the tag already exists, inspect
its target and existing run; never move or repush it automatically.

Create an annotated tag at the verified full release commit SHA, then push
only that tag. Substitute concrete values; do not tag an unverified moving ref:

```bash
git tag -a vX.Y.Z RELEASE_COMMIT_SHA -m "vX.Y.Z"
git push origin refs/tags/vX.Y.Z
```

The authenticated operator's push may print `Bypassed rule violations` about
restricted tag creation. This has accompanied successful releases; verify the
remote tag instead of treating the notice alone as a failure. If policy or
permission review rejects the push, report it and stop. Do not change rules
or retry through another mechanism. The annotated-tag command preserves the
operator's established release process; do not silently change signing policy.

Verify the remote tag's peeled commit against the exact release commit;
inspect its subject and embedded workspace version:

```bash
git fetch origin --tags
git log --oneline -1 vX.Y.Z
git show vX.Y.Z:Cargo.toml
git ls-remote origin 'refs/tags/vX.Y.Z*'
```

Expect `chore(release): vX.Y.Z` and workspace version `X.Y.Z`. A mismatch
stops publication; do not force-fetch, retag or repush to conceal it.

## Monitor the release run

A version tag push triggers `.github/workflows/release.yml`, named
`Release (macOS + Windows + Linux)`. Find the run for this tag and commit,
not merely the newest run:

```bash
gh run list -R pwrdrvr/PwrRoute --workflow release.yml --limit 10
gh run watch RUN_ID -R pwrdrvr/PwrRoute --exit-status
```

Run the watch in the background or a job monitor and keep the user informed.
Report each job's result when it ends. The current jobs inspect Winget/Homebrew
before publication, check release metadata and lint, test, build macOS
universal, build Windows x64, package Linux x64 and arm64, pack/sign/notarize
macOS, pack/sign Windows, publish assets, then inspect Winget/Homebrew again.
The Winget/Homebrew jobs are currently audit stubs, not package publication.

`macos-sign` uses `apple-signing`; `windows-sign` uses `windows-signing`.
The handoff records user approval and `v*` tag-only refs for `apple-signing`.
When waiting for approval, give the user the run link and pending environment.
Do not approve, change environment policies, or dispatch on main to evade the
gate: a `workflow_dispatch` on main is refused even if its tag input names a
release tag. The user owns any tag move and repush needed for a fresh run.

Release lint and tests now reuse main's Rust caches when available. Cold
caches remain possible; their 60-minute limits allow for that. Do not assume
a slow build is stuck solely because v0.2.2 compiled cold.

On failure, inspect the job and failed log, report the cause and stop release
progression. Reruns and fresh release dispatches are user-executed too: an
instruction to retry means diagnose and hand off the recovery action, not run
it. Do not rerun publication, dispatch another run, move a tag or retry a
release mutation. Prepare a targeted fix PR if requested; leave recovery
execution to the user.

## Verify publication and hand off promotion

Inspect the published prerelease, notes and assets:

```bash
gh release view vX.Y.Z -R pwrdrvr/PwrRoute --json url,isDraft,isPrerelease,assets,body
```

Check the current workflow and its README's asset table for the expected
platform files: universal DMG, Windows Setup and MSI, both Linux architectures'
deb/tar archives, checksums, portable archives and Velopack update feeds and
packages. Recent complete releases had 23 assets; optional delta packages and
workflow changes can alter the count, so also verify names and platform
coverage. Confirm signing jobs actually signed/notarized as intended: a
successful unsigned lane with missing credentials is not evidence of signing.
Report any missing assets, signing warnings or untested installation behavior
before recommending promotion.

The workflow creates a prerelease. Copies installed on 0.1.0 use an older
updater that skips prereleases, so they see a stable release only after
promotion. Later versions support channel selection; do not describe the old
updater restriction as applying to every version. For a validated suffix-free
stable tag, give the user:

```bash
gh release edit vX.Y.Z -R pwrdrvr/PwrRoute --latest --prerelease=false
```

Do not execute it. Suffixed tags remain prereleases. After the user promotes,
confirm the intended release is Latest:

```bash
gh release list -R pwrdrvr/PwrRoute --limit 2
```

Superseded releases may stay prereleases; do not promote or alter them as
cleanup. While the repository is private, the app's unauthenticated GitHub
release checks return HTTP 404. The user has accepted this limitation; mention
it when relevant to updater verification, without changing repository
visibility or adding authentication as part of a routine release.
