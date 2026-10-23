# PwrRoute design bundle — provenance

This directory is a checked-in copy of the **PwrRoute** project in
[Claude Design](https://claude.ai/design). It exists so that anyone without
Claude Design access can read the design from the repo. The project is the
source of truth; this copy is a mirror. If you change the design, change it in
the project and re-export here.

## Source

- Project: **PwrRoute** — <https://claude.ai/design/p/b9e61e6f-ed2f-4d41-b4a5-5bd0b2553897>
- Design system: PwrDrvr Design System `019debaf-c070-7afe-98db-4c9bbe10e72b`,
  with token values taken from PwrGit `apps/desktop/src/renderer/src/styles/tokens.css`
  (which tracks PwrAgnt `docs/UI-THEME.md`) on 2026-10-13. The shipped apps are
  the source of truth where they and the design system disagree.
- Exported: **2026-10-13**, the initial design pass. Copied verbatim into this
  repository on 2026-10-13 from the `design/` mirror in the PwrFinicky fork of
  Finicky (the rough cut), not re-exported from the project. Nothing here has
  been reconciled against code yet; the fork's Electron rough cut under
  `apps/pwrfinicky` predates the design and still carries the working name
  PwrFinicky.
- Updated **2026-10-13**: `pwrroute.css` re-exported after the strip gutter
  was narrowed to 68px, which puts the brand at x=96, the family spec. The
  GPUI app uses the same value.
- Updated **2026-10-18**: five canvases added for the workflows the first pass
  left out (rule editing, empty and error states, Windows and Linux chrome,
  updates and default-browser registration, the Finicky importer). Each was
  written to the project, then downloaded from its preview and compared byte
  for byte with the file here. The four earlier canvases, `pwrroute.css` and
  `support.js` are unchanged.
- Updated **2026-10-20**: one canvas added for link privacy and route tests
  (Settings › Privacy with retention and the journal, Activity's Unrouted and
  Private filters with Add a rule for this link, per-rule examples and Run
  tests on Rules, Save as test, and the MCP agent flow). Written to the
  project, then downloaded from its preview and compared byte for byte with
  the file here. The nine earlier canvases, `pwrroute.css` and `support.js`
  are unchanged.

- Updated **2026-10-22**: one canvas added for popovers and overflow (select
  lists as wide as their field, hints that end in an ellipsis, a browser's
  profiles as a count past two, lists that scroll past 8½ rows; the tray
  dropdown's window sized to its card, capped at the display's visible
  area, and the pause line naming the browser). Written to the project,
  then read back and compared by size with the file here. The ten earlier
  canvases, `pwrroute.css` and `support.js` are unchanged.
- Updated **2026-10-23**: one canvas added for the Overview home and setup
  (Overview with the route map, its hover states and the live pulse when a
  link routes, a Finicky config in charge with the Everything else callout
  and the override on with Undo in place, the hero states with the setup
  card including an unknown default and a default with no rules, the
  revised first run with no browsers found and a failed default request,
  the Rules banner switch with the overridden defaultBrowser row and its
  warnings, and the Settings › Configuration Everything else row). Revised
  the same day after review. Written to the project,
  then downloaded from its preview and compared byte for byte with the
  file here. The eleven earlier canvases, `pwrroute.css` and `support.js`
  are unchanged.
- Updated **2026-10-23**: the Overview canvas's route map redrawn as a
  graph from rules to profile nodes (one node per profile with its own
  port, every route always drawn and lit on hover or pin, fan-in into one
  node, a rule column that grows with the window, every installed browser
  shown with idle ones under Also installed); artboards 1a to 1d, 2a and
  2b updated, 1e (a node with eight routes in a wide window) and 1f
  (pinned) added. Written to the project, then downloaded from its preview
  and compared byte for byte with the file here. The other canvases,
  `pwrroute.css` and `support.js` are unchanged.
- Updated **2026-10-23**: Overview and Setup's artboard 3c (config didn't
  load) again shows the visual rules running without the config, with their
  counts, as the router now does; faded "not running" rows are only for
  nothing loading at all, and the hero copy note says so. Written to the
  project together with the route-map revision above, then downloaded and
  compared byte for byte. Nothing else changed.
- Updated **2026-10-23**: the Overview map gives spare width to the
  routes (the rule column stops at 720 and the destinations at 420, and
  the gutter takes the rest, shown in 1e at 1880 with a width table), and
  every browser card with a private mode ends with a private-window node
  in the browser's own words (Incognito, InPrivate, Private window), idle
  in most artboards and routed by rule 05 in 1a to 1g. 1g (adding a rule
  from an idle private node) and a note with the three callout copy
  variants added. Artboard 3c is unchanged here. The other canvases,
  `pwrroute.css` and `support.js` are unchanged.

## Files

| File | What it holds |
| --- | --- |
| `PwrRoute - Name and Brief.dc.html` | The name decision (PwrRoute, with PwrOpen, PwrHop, PwrPick drawn and passed), the mark, tagline, assumptions, window map, Finicky v4 compatibility table, MCP tool and role matrix |
| `PwrRoute App Baseline.dc.html` | Main window: Rules (rewrites, handlers, default) with the rule editor rail; the Finicky-config-in-charge state; Test a link (trace); Activity |
| `PwrRoute Settings.dc.html` | Settings window: Local agents (MCP, sessions, roles), Configuration (source, importer, short links, files), Browsers (OS default, detected apps, other apps) |
| `PwrRoute Tray and First Run.dc.html` | macOS menu bar dropdown (normal and paused), Windows/Linux tray menu, first-run window |
| `PwrRoute Rule Editing.dc.html` | Add a handler (rail or sheet), the rewrite editor and a new rewrite, reorder by drag and by keyboard, delete with undo (toast or tombstone), validation problems and warnings |
| `PwrRoute Empty and Error States.dc.html` | No rules; no configuration; a config that fails to load while the last good one routes (window and menu bar); a matcher that timed out; a browser that is not installed |
| `PwrRoute Windows and Linux Chrome.dc.html` | Main-window strip on Windows 11 and GNOME: brand at x=16, menu bar, caption buttons, and the rejected keep-the-gutter option |
| `PwrRoute Updates and Default Browser.dc.html` | Velopack update states and Restart to update; becoming the default browser on macOS, Windows 11 and Linux |
| `PwrRoute Finicky Importer.dc.html` | The importer end to end: discovery and entry points, Review with verdicts and suggestions, Place, Choose source, result with Undo import, a config that does not load |
| `PwrRoute Activity Privacy and Route Tests.dc.html` | Settings › Privacy (retention count and hours, the journal switch Off / Hosts only / Full URLs with its disclosure sentence, file, Reveal, Clear now, Export, the per-rule Keep explainer); Activity with Unrouted and Private filters, the row menu, the Add a rule sheet with pattern suggestions and collisions, and the needs-a-JS-handler state; per-rule example chips, the Run tests strip with a failure row, Add example, Save as test; the MCP storyboard (activity, rules_test, rules_apply refused) |
| `PwrRoute Popovers and Overflow.dc.html` | Select lists and menus: width follows the field, hints and long names end in an ellipsis, a browser's profiles named up to two and counted past that, lists that scroll past 8½ rows, the row menu's Save hint; the tray dropdown sized to its card, on a short display, and its pause line naming the browser |
| `PwrRoute Overview and Setup.dc.html` | Overview, the window's new home: hero sentence and state, Today tiles, the route map (rules in evaluation order on the left in a column that widens with the window, every installed browser on the right as a card of profile nodes, idle ones under Also installed; every route drawn dim at rest and traced bright on hover or pin, the live pulse when a link routes); a Finicky config in charge with the Everything else callout and the override on with Undo in place; paused, not the default browser with the setup card, config didn't load, nothing set up yet, default unknown, and a default with no rules while a config is found; the revised first run (config found and its variants, broken config, no config in two steps, the Everything else segmented control, done, no browsers found, a failed default request); the Rules banner switch with the overridden defaultBrowser row and its warnings; Settings › Configuration's Everything else row |
| `pwrroute.css` | Shared tokens and chrome for all twelve canvases: family title strip, rail, cards, chips, controls |
| `support.js` | Claude Design's Design Components runtime, as written by the project. Not ours; do not edit |

Open a `.dc.html` next to `support.js` and `pwrroute.css` in a browser to view it.

## Reaching the project from Claude Code or Claude Desktop

Address the project by id with the `claude-design` MCP server
(`mcp__claude-design__list_files`, `read_file`, `finalize_plan`, `write_files`,
`render_preview`) or with the `DesignSync` tool. `list_projects` does list this
project. See PwrGit's `design/SOURCE.md` for the same workflow in more detail.

## How to re-export

1. `list_files` the project at depth -1 and `read_file` each file. The body
   comes back HTML-entity-escaped (`&amp;`, `&lt;`, `&gt;` only); decode those
   three and nothing else.
2. Write each file here under its project name. Check sizes against the
   listing.
3. Update the "Exported" date above and commit.
