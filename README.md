<div align="center">

<h1>PwrRoute</h1>

<strong>Every link to the right browser.</strong>

<p>PwrRoute becomes your default browser and sends each link, after any URL
rewrites, to the browser and profile your rules pick.<br>
macOS first, then Windows and Linux.</p>

<sub>Status: pre-release · MIT</sub>

</div>

## Status

PwrRoute is an early port. It is a Rust workspace with a native window built
on [GPUI](https://www.gpui.rs) and a rules engine that runs Finicky v4 configs
in QuickJS. The engine resolves URLs the way Finicky does and is tested
against it. A resident router receives links from the OS, resolves each one
with the engine and opens the browser and profile the rules pick; when that
browser cannot be opened, the link goes to the fallback browser instead. The
window's Rules screen edits your `rules.json`, Test a link and Activity run
on the real engine and router, and Settings shows and changes the running
app. Not done yet: importing a Finicky config into `rules.json`, and
expanding short links in Test a link. Releases are pre-releases.

## Install (macOS)

Download the DMG (`PwrRoute-<version>-universal.dmg`) from the
[GitHub release](https://github.com/pwrdrvr/PwrRoute/releases), drag
PwrRoute.app to Applications and open it once. Its first-run window offers
to make PwrRoute the default browser; macOS asks you to confirm. After that
PwrRoute stays in the menu bar with its windows closed.

While this repository is private, the in-app updater cannot see its
releases: it polls GitHub Releases without a token. Update by hand for now:
quit PwrRoute, download the newer DMG and replace the app in Applications.

## What it does

- **Routes every link.** Registered as the OS default browser, PwrRoute
  receives each link you click in another app and opens it in the browser and
  profile the rules choose. A resident router does the dispatch and keeps
  working with the window closed.
- **Rewrites, then handlers, then a default.** Every matching rewrite runs in
  order, the first matching handler picks the destination, and anything
  unmatched falls through to the default browser. Rules can match on URL
  wildcards, regular expressions and the app that sent the link.
- **Visual rules or code.** Rules you edit in the app are stored as JSON. A
  Finicky v4 `.js` or `.ts` config in Finicky's usual places runs first, for
  function matchers and URL transformers, with the JSON rules after it.
- **A trace for every link.** Test a link shows each step that fired. Activity
  keeps the same trace for recent dispatches. Full URLs stay in memory
  unless you turn on the activity journal; disk logs carry timings, never
  URL contents.
- **An MCP server for agents.** The running app serves MCP on a socket only
  your user can reach, with an approval window and per-agent roles. A local
  agent can resolve a URL to its trace, open a link through the router, list
  browsers, read Activity, find the links no rule routed, propose a narrow
  rule, test it against the examples saved with your rules, and apply it.
  See [Local agents](#local-agents).

The design lives in [design/](design/); start with
[PwrRoute - Name and Brief](design/PwrRoute%20-%20Name%20and%20Brief.dc.html).

## Local agents

The app serves MCP while it runs. `pwrroute mcp` is the command an MCP
client launches: it pipes the client's stdio to the app's per-user socket
(`mcp.sock` beside the router's, in the data directory; a named pipe on
Windows). Register it once:

```bash
claude mcp add --scope user pwrroute -- pwrroute mcp
codex mcp add pwrroute -- pwrroute mcp
```

Settings › Local agents shows the exact command, with the binary's path
when it is not on the `PATH`. The first time a client name connects, PwrRoute
opens an approval window and you pick a role: **Reader** (resolve links, list
browsers, read rules, see Activity as hosts only), **Opener** (plus opening
links, with full URLs in Activity) or **Editor** (plus writing `rules.json`
and reading the activity journal). The name is what the client sends, so a
role is a guardrail against an agent overreaching, not proof of who is
calling. Change, revoke or forget an agent in the same pane.

The tools:

| Tool | Role | What it does |
| --- | --- | --- |
| `pwrroute_resolve` | Reader | Where a link would go and why. Opens nothing. |
| `pwrroute_browsers` | Reader | Installed browsers, their profiles, and whether PwrRoute is the OS default. |
| `pwrroute_rules_list` | Reader | `rules.json` as written, with the revision a write must name. |
| `pwrroute_rules_test` | Reader | Runs the examples and tests saved in `rules.json`, as they are or against a proposal: a whole file, or one extra handler with the links it must route. Writes nothing. |
| `pwrroute_suggest_rule` | Reader | Wildcard patterns that would route a link, narrowest first, whether a precise rule needs a JS config, and which recent links each pattern would also catch. |
| `pwrroute_activity` | Reader | Recent dispatches, newest first. `filter` finds the `unrouted`, `failed`, `rewritten` or `private` ones; `retention` says where they are kept and for how long; each entry's `privacy` says how much of the link it keeps. Readers see hosts only. `source: "journal"` reads the on-disk journal, when it is on, for Editors. |
| `pwrroute_open` | Opener | Dispatches a link through the router, as a click would, recorded in Activity against the agent. |
| `pwrroute_rules_apply` | Editor | Writes a JSON Merge Patch to `rules.json`, naming the revision read. It runs the saved examples and tests first and refuses, with the failures, a change that makes one fail, unless `force` is true. |

Links that pass through these tools stay in memory. The only things written
to disk are the approvals (`mcp-agents.json`: names, roles, timestamps) and,
through `pwrroute_rules_apply`, `rules.json` itself, including the examples
and tests an agent writes into it. The router's activity journal is a
separate, opt-in setting.

## Development

PwrRoute is a Cargo workspace on Rust 1.97 (`rust-toolchain.toml`). See
[CONTRIBUTING.md](CONTRIBUTING.md) for setup.

```bash
cargo run -p pwrroute-app   # the window
cargo xtask lint            # rustfmt, clippy, cargo-deny, license notices
cargo xtask test
cargo xtask bundle          # macOS app, zip and DMG
```

### Route tests from the command line

The examples and tests saved in `rules.json` (each rule's `examples`, each
rewrite's `url` / `becomes` pairs, and the top-level `tests`) run through the
same engine as the window's "Run tests":

```bash
pwrroute rules test                       # the rules.json and config the router uses
pwrroute rules test --rules path/to/rules.json [--config path/to/finicky.js]
```

With `--rules`, the file runs on its own unless `--config` names the config
to pair it with.

It prints one block per failure (input, expected, actual, winner) and a
`N passed, M failed, T total` line, and exits 0 when everything passed, 1
when something failed and 2 when the rules could not be loaded or run. It
never launches a browser and writes nothing.

## Thanks, Finicky

PwrRoute started from [Finicky](https://github.com/johnste/finicky), by
[John Sterling](https://github.com/johnste) and
[Finicky's contributors](https://github.com/johnste/finicky/graphs/contributors).
It still reads Finicky v4 `.js` and `.ts` configs, and is tested against them.

PwrRoute is going in a slightly different direction: a visual rules editor
that stores rules as JSON alongside JS/TS configs, support for Windows and
Linux as well as macOS, a trace for every link, and an MCP server for agents.

Thank you for Finicky.

PwrRoute is an independent project. It is not affiliated with or endorsed by
Finicky or its authors.

## License

MIT. Copyright (c) 2026 PwrDrvr LLC. Code derived from Finicky is used under
Finicky's MIT license, reproduced in full in [LICENSE](LICENSE). Third-party
notices for shipped dependencies are in
[THIRD_PARTY_LICENSES](THIRD_PARTY_LICENSES).
