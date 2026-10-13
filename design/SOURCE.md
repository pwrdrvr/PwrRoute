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

## Files

| File | What it holds |
| --- | --- |
| `PwrRoute - Name and Brief.dc.html` | The name decision (PwrRoute, with PwrOpen, PwrHop, PwrPick drawn and passed), the mark, tagline, assumptions, window map, Finicky v4 compatibility table, MCP tool and role matrix |
| `PwrRoute App Baseline.dc.html` | Main window: Rules (rewrites, handlers, default) with the rule editor rail; the Finicky-config-in-charge state; Test a link (trace); Activity |
| `PwrRoute Settings.dc.html` | Settings window: Local agents (MCP, sessions, roles), Configuration (source, importer, short links, files), Browsers (OS default, detected apps, other apps) |
| `PwrRoute Tray and First Run.dc.html` | macOS menu bar dropdown (normal and paused), Windows/Linux tray menu, first-run window |
| `pwrroute.css` | Shared tokens and chrome for the four canvases: family title strip, rail, cards, chips, controls |
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
