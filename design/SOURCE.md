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
| `pwrroute.css` | Shared tokens and chrome for all ten canvases: family title strip, rail, cards, chips, controls |
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
