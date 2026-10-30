<div align="center">

<h1>PwrRoute</h1>

<strong>Every link to the right browser.</strong>

<p>PwrRoute becomes your default browser and sends each link, after any URL
rewrites, to the browser and profile your rules pick.<br>
macOS, Windows and Linux. Runs your Finicky config.</p>

<sub>Status: pre-release · MIT · by <a href="https://pwrdrvr.com">PwrDrvr LLC</a></sub>

</div>

## Why PwrRoute

Work links belong in the work profile, personal links in the personal one,
and the client's links in the client's.
[Finicky](https://github.com/johnste/finicky) has done this well on macOS for
over ten years, and PwrRoute started as a fork of it. We wanted three more
things:

- **No wait.** On macOS, Finicky opens a Chromium profile with `open -n`,
  which starts a second copy of the browser just to hand the link to the
  copy that's already running. When the browser hasn't been used for a
  while, that can take 2–3 seconds. For Chrome, Chromium and Edge (every
  channel), PwrRoute writes the link straight to the running browser over
  the socket the browser uses for itself, so the tab opens right away. More
  browsers follow as each one's protocol is verified.
- **Every desktop.** The same rules on macOS, Windows and Linux.
- **Rules you can see.** A visual editor, a trace that says why each link
  went where it did, and a history of recent links so you can write a rule
  for a link you never saw.

PwrRoute runs your Finicky v4 `.js` or `.ts` config as it is: same file
locations, same API. You can try PwrRoute and go back to Finicky at any time.

## Install

Download the latest build from
[GitHub Releases](https://github.com/pwrdrvr/PwrRoute/releases). Releases
are pre-releases until 1.0.

| OS | Download | Then |
| --- | --- | --- |
| macOS 11+ (Intel and Apple silicon) | `PwrRoute-<version>-universal.dmg` | Drag PwrRoute.app to Applications and open it once. The first-run window offers to make it the default browser, and macOS asks you to confirm. |
| Windows 10/11 | `PwrRoute-win-Setup.exe` (or `PwrRoute-win.msi`) | Run the installer, then choose PwrRoute as the default browser in the first-run window or in Settings › Default apps. |
| Linux (x64, arm64) | `.deb`, or `.tar.gz` for other distributions | Install it, then make it the default from the first-run window (or `xdg-settings set default-web-browser pwrroute.desktop`). |

PwrRoute then stays in the menu bar (the system tray on Windows and Linux)
with its windows closed. It updates itself on macOS and Windows; on Linux,
install the newer package.

## What it does

- **Routes every link.** Registered as the OS default browser, PwrRoute
  receives each link you click in another app and opens it in the browser and
  profile the rules choose. A resident router does the dispatch and keeps
  working with the window closed. If that browser can't be opened, the link
  goes to the fallback browser instead of disappearing.
- **Rewrites, then handlers, then a default.** Every matching rewrite runs in
  order, the first matching handler picks the destination, and anything
  unmatched falls through to the default browser. Rules can match on URL
  wildcards, regular expressions and the app that sent the link. Short links
  are expanded first, so rules see where a link really goes.
- **Visual rules and code together.** Rules you edit in the app are stored as
  JSON. A Finicky v4 `.js` or `.ts` config in Finicky's usual places runs
  alongside them, for function matchers and URL transformers; you choose
  which goes first. The Finicky importer moves what it can from your config
  into visual rules and tells you which rules have to stay in code.
- **Rules that test themselves.** Save example links with a rule, and
  PwrRoute checks them on every change, in the window, from the command line
  and before an agent's edit is accepted.
- **A trace for every link.** Test a link shows each step that fired. Activity
  keeps the same trace for recent dispatches, so you can make a rule from a
  link after you've clicked it. Full URLs stay in memory unless you turn on
  the activity journal; disk logs carry timings, never URL contents.
- **Per-rule privacy and private windows.** A rule can open its links in a
  private window, and can tell PwrRoute to keep only the host, or nothing, in
  Activity.
- **An MCP server for agents.** The running app serves MCP on a socket only
  your user can reach, with an approval window and per-agent roles. A local
  agent can resolve a URL to its trace, open a link through the router, list
  browsers, read Activity, find the links no rule routed, propose a narrow
  rule, test it against the examples saved with your rules, and apply it.
  See [Local agents](#local-agents).

The design lives in [design/](design/); start with
[PwrRoute - Name and Brief](design/PwrRoute%20-%20Name%20and%20Brief.dc.html).

## Coming from Finicky

PwrRoute looks for your config where Finicky does, in the same order:
`~/.finicky.js` or `.ts`, `~/.config/finicky.js` or `.ts`, then
`~/.config/finicky/finicky.js` or `.ts` (plus `$XDG_CONFIG_HOME` on Linux and
`%APPDATA%\finicky\` on Windows). It runs Finicky's own configuration API,
unchanged, so routing decisions are Finicky's. Its compatibility tests
compare PwrRoute's results with those recorded from Finicky's Go host, link
by link.

PwrRoute never writes your config. Visual rules go to PwrRoute's own
`rules.json`, which uses the schema of Finicky 4.4's `rules.json` plus
optional fields, so Finicky's rules file loads as it is.

| | PwrRoute | Finicky |
| --- | --- | --- |
| Platforms | macOS, Windows, Linux | macOS |
| Built with | Rust: a native GPUI window, QuickJS for configs | Go and Objective-C, with a Svelte UI in a web view |
| Opening a Chrome or Edge profile on macOS | Hands the link to the running browser over its own socket | `open -n` starts a second browser process to hand it off |
| Finicky v4 `.js` / `.ts` configs | Yes, same locations, same API | Yes |
| Visual rules | Yes, alongside a config, with a choice of which goes first | In 4.4.0-alpha (not yet in a stable release) |
| Import config rules into visual rules | Yes | No |
| Saved example links checked on every change | Yes | No |
| History of routed links, with the trace | Activity, in memory by default | Optional request log on disk (`logRequests`) |
| MCP server for agents | Yes | No |
| Match on the sending window's title (`opener.windowTitle`) | Not yet | In 4.4.0-alpha |
| Browser extension ("Open with…") | Not yet | Yes |

Comparison as of Finicky v4.2.2 (latest stable) and v4.4.0-alpha.

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

PwrRoute exists because of [Finicky](https://github.com/johnste/finicky), by
[John Sterling](https://github.com/johnste) and
[Finicky's contributors](https://github.com/johnste/finicky/graphs/contributors).
Finicky's configuration API is the heart of PwrRoute's rules engine, its
browser list is the start of PwrRoute's, and its config format is one we're
glad to keep running. If you're happy on macOS with Finicky, keep using it,
and consider [supporting its author](https://github.com/sponsors/johnste).

PwrRoute is an independent project by PwrDrvr LLC. It is not affiliated with
or endorsed by Finicky or its authors. Please report PwrRoute problems here,
not to Finicky.

## Contributing

Issues and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md),
[SUPPORT.md](SUPPORT.md) for questions, and [SECURITY.md](SECURITY.md) for
reporting a vulnerability privately.

## License

MIT. Copyright (c) 2026 PwrDrvr LLC. Code derived from Finicky is used under
Finicky's MIT license (Copyright (c) 2015-2025 John Sterling), reproduced in
full in [LICENSE](LICENSE). Third-party notices for shipped dependencies are
in [THIRD_PARTY_LICENSES](THIRD_PARTY_LICENSES).
