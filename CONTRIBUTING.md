# Contributing to PwrRoute

Thanks for helping improve PwrRoute. The project is MIT-licensed; by
submitting a contribution, you agree that it may be distributed under that
license.

Use [GitHub Issues](https://github.com/pwrdrvr/PwrRoute/issues) for
reproducible bugs and focused feature proposals. Do not report vulnerabilities
publicly; follow [SECURITY.md](SECURITY.md) instead.

PwrRoute is pre-release, and the repository is a scaffold that app code is
being ported into. Read [AGENTS.md](AGENTS.md) before starting a change.

## Development setup

PwrRoute is a pnpm workspace. It requires Node.js 24 through nvm (the version
in `.nvmrc`) and uses the pnpm version pinned in the root `package.json`. A
fresh checkout or worktree needs its own install:

```bash
git clone https://github.com/pwrdrvr/PwrRoute.git
cd PwrRoute
source ~/.nvm/nvm.sh
nvm install
nvm use
corepack enable
pnpm install
pnpm dev
```

Do not assume `nvm` is initialized in a non-interactive shell. A root
`preinstall` guard refuses an install under the wrong Node.

## Repository map

- `apps/desktop` — Electron main, preload, renderer and packaging.
- `packages/shared` — the only shared contracts allowed across desktop process
  boundaries.
- `packages/mcp-server` — the local-agent MCP server.
- `scripts` — repository-wide license and release checks.
- `design` — the checked-in mirror of the Claude Design project; see
  `design/SOURCE.md`.

## How it's built

| Layer | Stack | Where it lives |
|---|---|---|
| Desktop shell | Electron + TypeScript + React + electron-vite | `apps/desktop/` |
| Shared contracts | Typed commands, events and domain types | `packages/shared/` |
| Local agent access | MCP over OAuth-protected loopback HTTP | `packages/mcp-server/` |

The resident router (Go, from the rough cut) and the rules engine are not
ported yet.

## Checks

Run the checks from the repository root:

```bash
pnpm lint
pnpm test
pnpm build
```

`pnpm lint` runs every check CI's Typecheck job runs, cheapest first: syntax
lint, third-party license checks and workspace typecheck. Add a new
repository-wide check to that chain in the root `package.json`, not as a
separate CI step.

For a documentation-only change, `pnpm lint` is the expected baseline. If you
skip a broader check, explain why in the pull request.

## Architecture and conventions

- Keep Electron main, preload, and renderer as separate bundles. Shared code
  belongs in `@pwrroute/shared`.
- Keep renderer windows sandboxed with context isolation enabled and Node
  integration disabled.
- Treat URLs as private data. Keep full URLs in memory; logs on disk carry
  timings, never URL contents.
- Never change the OS default browser except through the explicit action in
  the app.

## Releases

Releases are cut by pushing a `vX.Y.Z` tag that matches
`apps/desktop/package.json` and a `CHANGELOG.md` section; `pnpm release:check`
verifies both. CI, preview builds, and signing secrets are in
[.github/workflows/README.md](.github/workflows/README.md).

## Pull requests

- Keep each PR focused on one coherent change.
- Use a Conventional Commit-style title: `type(scope): description`.
- Prefer `desktop`, `router`, `rules`, `mcp`, `release`, `docs`, or `tests`
  when one of those scopes describes the change.
- Include tests for behavior changes, or state why the change is
  documentation-only.
- Record the commands and manual checks you ran.

If a dependency changes, regenerate and review the committed notices:

```bash
pnpm licenses:generate
pnpm licenses:check
```

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
