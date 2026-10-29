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
- Updated **2026-10-23**: the private-window mask glyph's eye slits are
  now filled almond eye-holes, so at 11-13px it no longer reads as a
  chain link; the Private windows note says so. Nothing else changed.
- Updated **2026-10-23**: Overview and Setup split into three canvases
  because of its size, each small enough to write to the project in one
  go: `PwrRoute Overview and Setup.dc.html` keeps the name and turn 1 (the
  route map, 1a to 1g) with a note pointing to the other two;
  `PwrRoute Overview States.dc.html` holds turns 2 and 3 (a Finicky config
  in charge, and the hero states); `PwrRoute First Run and Everything
  Else.dc.html` holds turns 4 and 5 (first run, and the Everything else
  override). Each carries the same head and styles. No artboard changed.
  All three were written to the project, then downloaded from its preview
  and compared byte for byte with the files here. The other canvases,
  `pwrroute.css` and `support.js` are unchanged.

- Updated **2026-10-24**: one canvas added for creating and editing a rule
  (`PwrRoute Rule Editor Pickers.dc.html`): Rules keeps its Add rule head
  while a Finicky config is in charge, Overview cards get a browser-level
  "+ Rule" in the header and profile rows say "+ Rule for <profile>", the
  rule editor's Open in and Profile become pickers of the scan's browsers
  and profiles with free text as the last row (and their empty, loading
  and error states), the rest of the rail is tidied (Activity keeps, the
  private switch in the browser's word, an Examples empty state, the
  unwired background and arguments rows gone), and Test a link gains Add
  a rule. Written to the project, then compared by size with the file
  here. The other canvases, `pwrroute.css` and `support.js` are unchanged.

- Updated **2026-10-26**: one canvas added for starting a rule from a
  link (`PwrRoute Rule From a Link.dc.html`): the rail's Start from picker
  over the recent links Activity holds, prefilled with the last link, its
  suggested patterns as chips, Clear, and the no-links states; the
  Overview's first-rule prompt naming the last link with Add a rule for
  this link and Blank rule; the defaults by entry point. Written to the
  project, then compared by size with the file here. The other canvases,
  `pwrroute.css` and `support.js` are unchanged.

- Updated **2026-10-26**: one canvas added for macOS 27's app data
  protection (`PwrRoute First Run Profile Access.dc.html`, turn 6): first
  run's new first step when the scan finds a browser whose profile list is
  there but may not be read, with a copy of the Files & Folders switches
  to turn on, Open Files & Folders, Check again and Skip, its partial,
  still-blocked and skipped states, and the profile select appearing once
  it is done. Written to the project, then read back and compared by size
  with the file here. The other canvases, `pwrroute.css` and `support.js`
  are unchanged.

- Updated **2026-10-27**: one canvas added for adding a rule from an
  Activity row in one click (`PwrRoute Activity Quick Add Rule.dc.html`):
  "+ Rule" in the Took column on the row under the pointer, the places
  tried for it, at rest on every row under Everything else, the expanded
  row with the link on its own lines and Add a rule… first as the primary
  button, ⌘N, and the host-only row that offers nothing. Written to the
  project, then compared by size with the file here. The other canvases,
  `pwrroute.css` and `support.js` are unchanged.

- Updated **2026-10-27**: one canvas added for Activity's Add a rule
  sheet (`PwrRoute Add Rule From an App.dc.html`): the link in the head
  cut at two lines with Show full link and its query count, and "Any link
  from <app>" as the first Match choice whenever the record names the app
  that sent the link (pattern `*`, Sent from on, Also catches only that
  app's links; Sent from off goes back to the narrowest pattern). Written
  to the project, then compared by size with the file here. The other
  canvases, `pwrroute.css` and `support.js` are unchanged.

- Updated **2026-10-27**: one canvas added for picking where a rule sends
  links and which app it listens to (`PwrRoute Destination and App
  Pickers.dc.html`): Open in as a lane per browser with its profiles and
  private window as pills, in Activity's Add a rule sheet and the rule
  editor, a browser the rule names but this Mac lacks; Sent from as app
  chips by name, the "+ app" menu (recent senders, apps the rules name,
  any app in Applications) and an app not on this Mac. Written to the
  project, then compared by size with the file here. The other canvases,
  `pwrroute.css` and `support.js` are unchanged.

- Updated **2026-10-27**: one canvas added for sending links without a
  rule (`PwrRoute Send Links.dc.html`): the menu bar dropdown's Send links
  section (Rules, Next link with its count, All links until switched back
  or for an hour, the destination row and its list in place, sharing the
  router's slot with Pause routing), the Windows and Linux menu rows, the
  Overview's hero, strip chip, rail foot and marked node, a node's Next
  link here, Activity's next link and all links chips, and the behaviour
  table (which links count, memory only, private windows, hero order).
  Written to the project, then compared by size with the file here. The
  other canvases, `pwrroute.css` and `support.js` are unchanged.

- Updated **2026-10-27**: `PwrRoute Send Links.dc.html` 1b redrawn: the
  dropdown's destination list is now the lanes from Destination and App
  Pickers, compact (a lane per browser, its profiles and private window as
  pills, the destination's pill lit), so PwrRoute has one way to pick a
  destination; the behaviour table's Destination row says so. Written over
  the project's copy (its etag unchanged since the last write), then
  compared by size with the file here.

- Updated **2026-10-28**: one canvas added for telling builds apart
  (`PwrRoute Build Identity.dc.html`): the main window's rail foot with a
  dev build's checkout, branch and commit, each a click to copy, and a
  copy-all button; a detached HEAD with uncommitted changes; hover,
  tooltip and Copied; a release's one line; Settings › About's Source row
  and Copy build info for a dev build and a release, and what each copy
  writes. Written to the project, then compared by size with the file
  here. The other canvases, `pwrroute.css` and `support.js` are
  unchanged.

- Updated **2026-10-28**: one canvas added for approving a local agent
  (`PwrRoute Agent Approval.dc.html`): the approval window as it was, then
  the roles as a radio group with Reader chosen, the chosen role's
  permissions listed, Deny and Allow as <role> (Esc and Return, 1 2 3 to
  pick), and the window kept open on the outcome (allowed, denied),
  shrunk to it. PwrSnap's and PwrGit's consent windows were read first;
  both pick then confirm. Revised the same day after review: no time
  limit, so the countdown and the expired outcome are gone (the window
  stays while the agent waits and closes when it goes), and the Editor
  role's description is corrected. Written to the project, then compared
  by size with the file here. The other canvases, `pwrroute.css` and
  `support.js` are unchanged.

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
| `PwrRoute Rule Editor Pickers.dc.html` | Creating and editing a rule: Add rule on Rules with and without a Finicky config, browser-level and profile-level Add rule on Overview cards, the Open in and Profile pickers with their empty, loading and error states, the rest of the rail, and every entry point's defaults |
| `PwrRoute Rule From a Link.dc.html` | Starting a rule from a link you opened: the rail's Start from picker (prefilled with the last link, the recent links list, Clear, no links), suggested-pattern chips, the Overview's first-rule prompt naming the last link, and the defaults by entry point |
| `PwrRoute Add Rule From an App.dc.html` | Activity's Add a rule sheet: the link in the head at two lines with Show full link, Any link from <app> above the patterns and what it does to Sent from and Also catches, and the sheet with no app on record |
| `PwrRoute Destination and App Pickers.dc.html` | Open in as lanes: a browser per lane, its Default, profiles and private window as pills, in the Add a rule sheet and the rule editor, and a browser the rule names that is not installed; Sent from as app chips by name, the "+ app" menu and an app not on this Mac |
| `PwrRoute Overview and Setup.dc.html` | Overview, the window's new home (turn 1): hero sentence and state, Today tiles, the route map (rules in evaluation order on the left in a column that widens with the window up to a cap, every installed browser on the right as a card of profile nodes, idle ones under Also installed, private-window nodes; every route drawn dim at rest and traced bright on hover or pin, the live pulse when a link routes, a pinned node, adding a private rule), with the measurements, width table and private-window notes |
| `PwrRoute Overview States.dc.html` | Overview states (turns 2 and 3): a Finicky config in charge with the Everything else callout and the override on with Undo in place; paused, not the default browser with the setup card, config didn't load, nothing set up yet, default unknown, and a default with no rules while a config is found |
| `PwrRoute First Run and Everything Else.dc.html` | The revised first run (turn 4: config found and its variants, broken config, no config in two steps, the Everything else segmented control, done, no browsers found, a failed default request) and the Everything else override (turn 5: the Rules banner switch with the overridden defaultBrowser row and its warnings; Settings › Configuration's Everything else row) |
| `PwrRoute First Run Profile Access.dc.html` | First run on macOS 27 when a browser's profile list is kept from PwrRoute (turn 6): why, the Files &amp; Folders switches as the user will find them, Open Files &amp; Folders, Check again, Skip, and the profile select once access is on |
| `PwrRoute Activity Quick Add Rule.dc.html` | Adding a rule from an Activity row in one click: "+ Rule" where Took was on hover (and at rest under Everything else), where else it was tried, the expanded row's primary Add a rule… under a long link, ⌘N, and host-only rows |
| `PwrRoute Send Links.dc.html` | Sending the next link, the next few, or all links for a while to a browser and profile without a rule: the dropdown's Send links section and its destination lanes, the native menu's rows, the Overview's hero, chips, rail foot and marked node, Next link here on a node, Activity's chips, and how it behaves (one slot with Pause, counted per link in the router, memory only) |
| `PwrRoute Build Identity.dc.html` | Which build is running: the rail foot's build block (checkout, branch, commit, each click-to-copy, and copy all) for a dev build, a detached HEAD with changes and a release, and Settings › About's Source row with Copy build info |
| `PwrRoute Agent Approval.dc.html` | The local-agent approval window: before, a role chosen (Reader at open) with its permissions and Deny / Allow as <role>, keys, and the allowed and denied outcomes; no time limit |
| `pwrroute.css` | Shared tokens and chrome for all twenty-two canvases: family title strip, rail, cards, chips, controls |
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
