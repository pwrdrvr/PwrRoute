---
name: sandboxer-verification
description: >-
  Run PwrRoute's lint chain (rustfmt, clippy, cargo-deny, cargo-about),
  workspace tests, release build, Linux Desktop E2E and the
  THIRD_PARTY_LICENSES regeneration in a Sandboxer cloud sandbox instead of
  on the local machine. Use when the thread has the Sandboxer MCP
  (create_sandbox and exec tools) and a change needs `cargo xtask lint`,
  `cargo xtask test`, `cargo xtask e2e`, `cargo xtask licenses`, clippy or a
  workspace build. Sandboxer is Ubuntu only: it does not verify macOS or
  Windows behavior.
---

# Verify on Sandboxer

Sandboxer gives this thread an Ubuntu 24.04 x64 VM. The checks here mirror
CI's `Typecheck`, `Test`, `Build` and `Linux Desktop E2E` jobs. Run them in
the VM so the laptop's fans stay quiet: a GPUI build and cargo-about each
take every core they can get.

[`scripts/sandboxer/verify.sh`](../../../scripts/sandboxer/verify.sh) runs in
the VM. Its `setup` installs GPUI's libraries, the E2E desktop, the pinned
Rust toolchain and the cargo-deny and cargo-about releases CI uses, then
each step writes a log and a status line.
[`scripts/sandboxer/transfer.sh`](../../../scripts/sandboxer/transfer.sh)
runs on the laptop and moves files through
[`transfer.py`](../../../scripts/sandboxer/transfer.py) in the VM.

## Measured on a `large` sandbox (2026-10-29)

`large` is 8 vCPUs and 15 GB; `xlarge` needs a higher Sandboxer plan.

| Step | Command | Cold `target/` | From the snapshot, one crate edited |
| --- | --- | --- | --- |
| Setup | `verify.sh setup` | 58 s | 1 s |
| Typecheck job | `verify.sh lint` | 112 s | 47 s |
| Test job | `verify.sh test` | 176 s | 32 s |
| Linux Desktop E2E | `verify.sh e2e` | 95 s | 15 s |

"Cold" compiled every crate. The base snapshot holds a `target/` built at
its commit, so a run there recompiles only what the change touches. The last
column edited `pwrroute-engine`, which most crates depend on. The tests are
1,045 workspace tests and 15 E2E tests, all passing at both points.

## 1. Get a sandbox

Use one sandbox per thread. Name it `pwrroute-<branch>`, with each `/`
replaced by `-`, so `list_sandboxes` shows which thread owns it.

Call `list_sandboxes` first. If this thread's sandbox exists, use its ID;
`resume_sandbox` brings back a stopped one.

Before creating, check that the `create_sandbox` schema has `noEnv`. It
keeps the account's environment and secret files, such as `GH_TOKEN`, out
of the VM. Without it, do not create a sandbox: report the blocker.

Call `create_sandbox` with `noEnv: true`, `from: "pwrroute-base-us-east"`,
`region: "us-east"`, `type: "large"`, the name, `labels: {repo: pwrroute}`
and `ttlSeconds: 3600`. Always pass `region`: without it, Sandboxer picks
the one nearest the caller. The TTL is a wall-clock stop timer, not an idle
timer. Pass the `sandboxId` on every later call: the MCP default is the
last sandbox this server used, which can be another task's.

If the snapshot is missing, build it first (section 7).

## 2. Open the transfer port

PwrRoute is private and the VM has no GitHub token, so commits travel as git
bundles over a port the MCP exposes. Start the server in the VM with a
detached `exec`:

```sh
python3 ~/pwrroute/bin/transfer.py
```

Then call `host_port` with `port: 8722` (not `public`). It returns
`https://<host>?_token=<token>`; call that URL `URL` below. The token is a
credential: never write it into a commit, a PR, a log or a file in the
repository. A resumed sandbox may not have kept the process: if
`curl -s -o /dev/null -w '%{http_code}' localhost:8722/x` in the VM does not
print `404`, start it again.

## 3. Send a commit

Verify a commit, not a dirty tree. Commit first; amend freely between runs.
On the laptop:

```sh
scripts/sandboxer/transfer.sh send "$URL"
```

It bundles `HEAD` (a few MB), uploads it as `<sha>.bundle` and prints the
SHA. In the VM, check it out in the one checkout, `~/pwrroute/repo`.
Reusing that path keeps `target/` warm: cargo rebuilds only what changed.

```sh
sha=<sha>
cd ~/pwrroute/repo
git fetch -q "../files/$sha.bundle" HEAD
git checkout -q --detach "$sha"
rm "../files/$sha.bundle"
```

Never check out while a check runs. `git checkout` does not take
`verify.sh`'s lock, so wait for the run's last status line first: `all `
for `all`, or the step's own line for a single step. Earlier step lines
appear while `all` is still running.

## 4. Run the checks

One MCP `exec` call can run for 600 seconds at most. Start the run in the
background and poll with calls of five minutes or less:

```sh
cd ~/pwrroute/repo
nohup scripts/sandboxer/verify.sh all > /dev/null 2>&1 &
```

```sh
cd ~/pwrroute/repo
for i in $(seq 1 55); do grep -q '^all ' .local/sandboxer/status 2>/dev/null && break; sleep 5; done
cat .local/sandboxer/status
```

`all` runs setup, then lint, test and E2E, and it continues after a failed
check. Each invocation of `verify.sh` starts a new `.local/sandboxer/status`.
Each step appends `<step> exit=<code> secs=<seconds>`, and `all` ends with
`all exit=<0|1>`. Poll a single step the same way, for its `^<step> ` line.

Run one step when that is all the change needs. Arguments pass through:

```sh
scripts/sandboxer/verify.sh lint
scripts/sandboxer/verify.sh test -p pwrroute-engine
scripts/sandboxer/verify.sh build
scripts/sandboxer/verify.sh e2e -- quits
scripts/sandboxer/verify.sh licenses
```

`build` is CI's release `Build` job. It is not part of `all`: its profile
shares nothing with the others, so a first run compiles everything again.

## 5. Read the results

Read logs with bounded output, never whole:

```sh
tail -n 40 .local/sandboxer/lint.log
grep -nE '^test result|FAILED|panicked' .local/sandboxer/test.log | tail -n 20
```

E2E artifacts (each test's `app.log`, `startup.json`, state dumps) are in
`target/e2e-artifacts/<test>/`. Read the text ones with `read_file`.

Report the sandbox ID, the commit, and each status line. Name every failure.

## 6. Bring back THIRD_PARTY_LICENSES

After a dependency change, regenerate the notices in the VM, not on the
laptop:

```sh
cd ~/pwrroute/repo
scripts/sandboxer/verify.sh licenses && cp THIRD_PARTY_LICENSES ../files/
```

Then on the laptop download it over the file in the checkout and commit it:

```sh
scripts/sandboxer/transfer.sh get "$URL" THIRD_PARTY_LICENSES THIRD_PARTY_LICENSES
```

The file is over 1 MB: never pass it through `read_file`. Bring back any
other generated file the same way.

## 7. Build or refresh the base snapshot

Refresh `pwrroute-base-us-east` when `Cargo.lock`, `rust-toolchain.toml`,
the CI tool pins in `verify.sh`, or `verify.sh setup` changes, or when its
commit is far behind `origin/main`.

Create a `large` sandbox in `us-east` with `noEnv: true`, named
`pwrroute-base-builder`: from the current snapshot to refresh, or with no
`from` to build anew. Open the transfer port (section 2). The first time,
`transfer.py` is not in the VM yet: write
`scripts/sandboxer/transfer.py` to `/home/user/pwrroute/bin/transfer.py`
with `write_file` (a few KB). On the laptop, at the commit to snapshot
(normally `origin/main`):

```sh
git switch --detach origin/main
scripts/sandboxer/transfer.sh send "$URL"
```

In the VM, clone it the first time, or check it out (section 3):

```sh
cd ~/pwrroute
git clone -q "files/<sha>.bundle" repo && rm "files/<sha>.bundle"
cd repo
git remote set-url origin https://github.com/pwrdrvr/PwrRoute.git
```

Then run `verify.sh all` (section 4) to warm `target/`. A failed check
still warms it, but note the failure. Then clean the VM:

```sh
cd ~/pwrroute/repo
rm -rf .local/sandboxer target/e2e-artifacts ~/pwrroute/files/*
git status --short
```

`git status --short` must print nothing. Every new sandbox is a copy of the
base: never leave a log, a bundle or a credential in it. Call
`save_snapshot` with the name `pwrroute-base-us-east` and the labels
`repo: pwrroute`, `role: base`, `region: us-east` and
`commit: <short sha>`. Saving under an existing name replaces it. Then
delete the builder; named snapshots survive it.

## 8. Known differences from CI

- `verify.sh` turns off debug info (`CARGO_PROFILE_DEV_DEBUG=0`,
  `CARGO_PROFILE_TEST_DEBUG=0`) as CI does, so its `target/` is not
  interchangeable with a local one.
- `PWRROUTE_E2E_DISPOSABLE` stays unset, so the E2E scenarios that make
  PwrRoute the default browser skip. CI's hosted runners still run them.
- Linux only: macOS and Windows code under `cfg` is not compiled, let
  alone tested. `cargo xtask bundle`, the `screens` example and macOS E2E
  still need the laptop, CI or the PwrSuiteLab skills.

## 9. Lifetime and cleanup

- Call `stop_sandbox` after each verification run. It keeps the disk,
  including `target/`, and stops compute billing.
- Keep the stopped sandbox for the thread's next run while its PR is open.
- PwrAgent releases the thread's sandboxes once every PR attached to the
  thread has merged or closed, if the thread created them with its own
  `create_sandbox`. Attach the PR to the thread (check with
  `check_thread_pull_request_status`; `attach_thread_pull_request` when
  branch discovery missed it), then call `watch_thread_pull_request` with
  `notifyOn: ["merged", "closed"]`. A thread with no attached PR is never
  cleaned up. Report a cleanup that failed or left a deletion pending.
- Do not stop or delete a sandbox that another thread owns.
- Use only the Sandboxer MCP tools. Do not run the `sbx` CLI.
