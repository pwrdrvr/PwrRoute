# Changelog

## Unreleased

- Rust port - Replaced the Electron and pnpm scaffold with a Cargo workspace: a GPUI window that draws the Rules screen, a rules engine that runs Finicky v4 `.js` and `.ts` configs in QuickJS and is tested against Finicky, and `cargo xtask` tasks for lint, license notices and packaging on macOS, Windows and Linux.
- Scaffold - Added the workspace, CI, release pipeline, license checks, design mirror and a desktop stub that shows the PwrRoute title strip. No app code yet.
