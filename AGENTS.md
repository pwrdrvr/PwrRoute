# AGENTS.md

Guidance for coding agents (and humans) in PwrRoute — a cross-platform
Electron app in the Pwr family (siblings: PwrSnap, PwrGit, PwrAgnt) that
becomes the OS default browser and routes each link to a browser and profile
by rules. Keep this file short; see **Docs & conventions** below.

## Status: scaffold

The workspace, CI, release pipeline and license checks are in place; the app
is not. `apps/desktop` shows the family title strip and nothing else, and
`packages/shared` and `packages/mcp-server` are empty. App code is ported in
from the PwrFinicky rough cut (a fork of Finicky) in later changes. Until then:

- Port, don't invent. The design in `design/` and the rough cut decide the
  shape; read `design/PwrRoute - Name and Brief.dc.html` first.
- Code derived from Finicky (the Go URL resolver and browser launcher,
  config-api's wildcard matcher and schema, `browsers.json`) is MIT, Copyright
  (c) 2015-2025 John Sterling. Its notice is reproduced in `LICENSE`; keep it
  there, and keep any per-file copyright headers the source carries.
- Finicky's name and icon are not PwrRoute branding. The icon is by @uetchy
  and is not ours to reuse.

## Set up a new checkout or worktree

Enter the repo through nvm, then install. **Do not assume `nvm` is
initialized** — it is a shell function, and a non-interactive agent shell
usually does not have it, so `node` is whatever the machine defaults to:

```bash
source ~/.nvm/nvm.sh   # agent shells: nvm is undefined until you source it
nvm install            # first time on a machine
nvm use                # reads .nvmrc (Node 24)
pnpm i                 # every fresh checkout/worktree needs its own
```

A root `preinstall` guard ([scripts/check-node-version.mjs](scripts/check-node-version.mjs))
fails the install under a Node that does not match `.nvmrc`, or one that is not
nvm's on a machine that has nvm.

## Common commands

```bash
pnpm dev        # run the app (electron-vite dev)
pnpm build      # production build
pnpm test       # vitest across the workspace
pnpm typecheck  # TypeScript 7 native compiler across packages
pnpm lint       # every check CI's Typecheck job runs, cheapest-first
```

`pnpm lint` chains `lint:syntax` → `licenses:check` → `typecheck`, ordered so a
fast failure doesn't wait on the slow one. CI's Typecheck job runs exactly this
one command, so **add new repo-wide checks to the chain in the root
`package.json`**, not as another CI step.

`licenses:check` chains three scripts copied from PwrGit:
`check-package-license-policy.mjs` (our own package.json files and `LICENSE`),
`check-third-party-license-allowlist.mjs` (judges every shipped dependency's
license against an explicit allowlist) and
`generate-third-party-licenses.mjs --check` (the committed
`THIRD_PARTY_LICENSES` matches the tree). After a dependency change run
`pnpm licenses:generate` and commit the result. **Adding a dependency under an
unfamiliar license, or an id to the allowlist, is a legal decision — do not do
it to make CI green.**

`typecheck` selects the exact-pinned `typescript-native` alias through
`scripts/typecheck.mjs`. Both `typescript` and `typescript-native` publish
`tsc`, so do not rely on whichever binary wins `node_modules/.bin/tsc`. For a
comparison, run `PWRROUTE_TYPECHECK_COMPILER=typescript pnpm typecheck`.
`tsconfig.base.json` sets `noUnusedLocals` and `noUnusedParameters`, so dead
imports and locals fail typecheck.

`pnpm-workspace.yaml` sets a seven-day `minimumReleaseAge`, matching the
Dependabot cooldown. Exempting a package from it is a supply-chain decision.

## Launch the dev app

Before launching, check whether a window titled `PwrRoute` already exists and
reuse it when suitable. For Computer Use, never target the generic `Electron`
display name or the shared `com.github.Electron` bundle ID; resolve this
checkout's Electron path instead.

Agent shells hosted by another Electron Pwr app can inherit that parent's
`ELECTRON_EXEC_PATH`, `ELECTRON_CLI_ARGS`, or `ELECTRON_MAJOR_VER`. Clear them:

```bash
node --version # must match .nvmrc
corepack enable
env -u ELECTRON_EXEC_PATH -u ELECTRON_CLI_ARGS -u ELECTRON_MAJOR_VER pnpm dev
```

PwrRoute will register as the OS default browser. Registration is an explicit
action in the app; never change the default browser on a developer machine as
a side effect of a test or launch.

## Packaging & releases

`apps/desktop/scripts/package.mjs` wraps electron-builder and decides signed or
unsigned from the environment (`pnpm --filter @pwrroute/desktop package:mac`
for a local ad hoc signed build). Releases are cut by pushing a `vX.Y.Z` tag
that matches `apps/desktop/package.json` and a `CHANGELOG.md` section
(`pnpm release:check` verifies). CI, preview builds, and signing secrets are
documented in `.github/workflows/README.md`.

## Pull Requests

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
