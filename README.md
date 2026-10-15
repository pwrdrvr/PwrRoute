<div align="center">

<h1>PwrRoute</h1>

<strong>Every link to the right browser.</strong>

<p>PwrRoute becomes your default browser and sends each link, after any URL
rewrites, to the browser and profile your rules pick.<br>
macOS first, then Windows and Linux.</p>

<sub>Status: pre-release. Nothing to download yet. · MIT</sub>

</div>

## Status

PwrRoute is an early port. It is a Rust workspace with a native window built
on [GPUI](https://www.gpui.rs) and a rules engine that runs Finicky v4 configs
in QuickJS. The window draws the Rules screen from sample data. The engine
resolves URLs the way Finicky does and is tested against it. Neither is wired
to the other yet, and PwrRoute does not receive links from the OS. There are
no releases, and the behavior described below is the design, not the current
build.

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
  Finicky v4 `.js` or `.ts` config can be selected instead, for function
  matchers and URL transformers.
- **A trace for every link.** Test a link shows each step that fired. Activity
  keeps the same trace for recent dispatches. Full URLs stay in memory; disk
  logs carry timings, never URL contents.
- **An MCP server for agents.** A loopback MCP server, authorized with OAuth
  and per-session roles as in PwrSnap and PwrGit, lets a local agent resolve a
  URL to its trace, open a link through the router, list browsers, and read or
  change rules.

The design lives in [design/](design/); start with
[PwrRoute - Name and Brief](design/PwrRoute%20-%20Name%20and%20Brief.dc.html).

## Development

PwrRoute is a Cargo workspace on Rust 1.97 (`rust-toolchain.toml`). See
[CONTRIBUTING.md](CONTRIBUTING.md) for setup.

```bash
cargo run -p pwrroute-app   # the window
cargo xtask lint            # rustfmt, clippy, cargo-deny, license notices
cargo xtask test
cargo xtask bundle          # macOS app, zip and DMG
```

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
