# Contributing to PwrRoute

Thanks for helping improve PwrRoute. The project is MIT-licensed; by
submitting a contribution, you agree that it may be distributed under that
license.

Use [GitHub Issues](https://github.com/pwrdrvr/PwrRoute/issues) for
reproducible bugs and focused feature proposals. Do not report vulnerabilities
publicly; follow [SECURITY.md](SECURITY.md) instead.

PwrRoute is pre-release, and app code is still being ported in. Read
[AGENTS.md](AGENTS.md) before starting a change.

## Development setup

PwrRoute is a Cargo workspace. `rust-toolchain.toml` pins Rust 1.97, and rustup
installs it on first use. The lint chain needs cargo-deny, cargo-about and
Node.js 22 or newer for the dependency-free CI classifier tests.

```bash
git clone https://github.com/pwrdrvr/PwrRoute.git
cd PwrRoute
cargo install --locked cargo-deny cargo-about
cargo run -p pwrroute-app
```

On Linux, install GPUI's system libraries first with
`scripts/ci/install-linux-deps.sh` (Debian and Ubuntu).

On macOS, PwrAgent's **Start** action (from
`.codex/environments/environment.toml`) runs `bash scripts/dev/start.sh`.
It builds the debug app, quits the running PwrRoute copy, then bundles and
launches this checkout's app with `--make-default`. macOS may ask you to
approve the default-browser change. It uses your usual rules and settings.

The app stays at `<worktree>/target/dev/PwrRoute.app` across rebuilds, even
when `CARGO_TARGET_DIR` points to a shared build cache. Each worktree has its
own app path; switching worktrees may require approving another default
change. To build that bundle without launching or requesting ownership, run
`cargo xtask bundle --debug --app-only`. A normal launch never requests
ownership automatically, and `--dry-run` ignores `--make-default`.

## Repository map

- `crates/pwrroute-app` — the GPUI window, binary `pwrroute`.
- `crates/pwrroute-engine` — the rules engine: Finicky v4 configs in QuickJS.
- `xtask` — lint, test, license notices, release checks and packaging.
- `packaging` — the macOS `Info.plist` template and the Linux desktop entry.
- `licenses`, `about.toml`, `deny.toml` — license gate and notice generation.
- `design` — the checked-in mirror of the Claude Design project; see
  `design/SOURCE.md`.

## How it's built

| Layer | Stack | Where it lives |
|---|---|---|
| Desktop app | Rust + GPUI (gpui-kit) | `crates/pwrroute-app/` |
| Rules engine | Rust + QuickJS (rquickjs) + oxc, running Finicky's config API | `crates/pwrroute-engine/` |
| Resident router | Rust: dispatch, trace buffer, single instance, IPC | `crates/pwrroute-router/` |
| OS edge | Rust + objc2 (macOS), registry (Windows), xdg (Linux) | `crates/pwrroute-os/` |
| Local agent access | MCP over OAuth-protected loopback HTTP | not ported yet |

The browser launcher and the MCP server are not ported yet.

## Checks

Run the checks from the repository root:

```bash
cargo xtask lint
cargo xtask test
```

`cargo xtask lint` runs every check CI's Typecheck job runs, cheapest first:
rustfmt, CI classifier tests, clippy with warnings denied, cargo-deny and the
THIRD_PARTY_LICENSES check. Add a new repository-wide check to that chain in
`xtask/src/main.rs`,
not as a separate CI step.

For a documentation-only change, `cargo xtask lint` is the expected baseline.
If you skip a broader check, explain why in the pull request.

## Architecture and conventions

- Routing decisions come from Finicky's configuration API, run unchanged in
  the engine. Change routing behavior there, not in a Rust reimplementation.
- No `unsafe` outside a module that exists for it and says why.
- Treat URLs as private data. Keep full URLs in memory; logs on disk carry
  timings, never URL contents.
- Change the OS default browser only through an explicit action in the app
  or a launch with `--make-default` (including PwrAgent's Start action).

## Releases

Releases are cut by pushing a `vX.Y.Z` tag that matches the workspace version
in `Cargo.toml` and a `CHANGELOG.md` section; `cargo xtask release-check --tag
vX.Y.Z` verifies both. CI, preview builds, and signing secrets are in
[.github/workflows/README.md](.github/workflows/README.md).

## Pull requests

- Keep each PR focused on one coherent change.
- Use a Conventional Commit-style title: `type(scope): description`.
- Prefer `app`, `engine`, `router`, `rules`, `mcp`, `release`, `docs`, or `tests`
  when one of those scopes describes the change.
- Include tests for behavior changes, or state why the change is
  documentation-only.
- Record the commands and manual checks you ran.

If a dependency changes, regenerate and review the committed notices:

```bash
cargo xtask licenses
cargo xtask licenses --check
```

A new license id in `deny.toml` is a legal decision; make it in its own
commit and say why.

## Code derived from Finicky

Parts of PwrRoute are ported from [Finicky](https://github.com/johnste/finicky)
(MIT, Copyright (c) 2015-2025 John Sterling). Keep the Finicky notice in
`LICENSE` intact, keep any copyright header a ported file carries, and do not
use Finicky's name or icon as PwrRoute branding.

## Conduct

Participation is governed by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## Security

Do not report vulnerabilities in public issues or pull requests. Use GitHub
private vulnerability reporting, or email security@pwrdrvr.com. The full
policy, including what to put in a report, is in [SECURITY.md](SECURITY.md).
