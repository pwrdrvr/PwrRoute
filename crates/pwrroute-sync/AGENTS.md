# pwrroute-sync

Copies `rules.json` (and, if asked, the Finicky config) between PwrRoute's
data directory and `<cloud root>/PwrRoute/` folders that iCloud Drive,
Google Drive, Dropbox or any folder sync client keeps in step. `src/lib.rs`
maps the modules; the PR that added it has the design note.

- **Never lose an edit.** A pass writes only when one side is unchanged
  since the base (`decide.rs`). Both changed is a conflict that writes
  nothing until `Syncer::resolve`. Whatever a pull, a push or a resolution
  replaces is backed up first (`state.rs`, the newest `KEEP_BACKUPS`).
  Never add a "newest wins" shortcut.
- **Settle before applying.** Another machine's file is applied only once
  the manifest lists its hash, or it stayed the same across two passes, and
  it passes the item's `Check`. A half-synced file must never be applied.
- **Exact paths only.** Nothing lists or watches a directory, except
  `places::detect`, which lists `~/Library/CloudStorage` once a call to
  find Google Drive's account folders. On macOS 27.0.1 an ad-hoc-signed app
  launched through LaunchServices could stat, list, read and write inside
  iCloud Drive, Google Drive and Dropbox without a TCC prompt; still, the
  app only adds a folder (and writes the first file) from a click.
- **`runsAfter` rides with content.** A manifest entry's `runsAfter` (the
  config `rules.json` ran after on the machine that wrote it) is written
  only with that file's bytes, by a push or a "keep this machine's"
  resolution, never on its own and never by a pull. Otherwise two machines
  with different configs would rewrite `sync.json` back and forth forever.
- **A missing root is unavailable, never created.** `Folder::write` makes
  `PwrRoute/` inside an existing root only, so a client that is not running
  never gets a stray folder in its place.
- **Pure where it can be.** `decide` and `Schedule` take facts and a time;
  `Syncer::pass` takes `now`. Tests drive them with a fake clock and temp
  directories (`tests/machines.rs`); keep new behavior testable the same
  way. `Service` is only the thread.
- **No URLs in logs.** `rules.json` carries example and test URLs; log
  file names and folder labels, never contents.
