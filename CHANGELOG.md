# Changelog

## Unreleased

- Router - Added the resident router: macOS Apple Events and Windows and Linux command lines reach one router per user, a second launch forwards its links over a local socket or named pipe, and each dispatch is resolved by the engine and kept with its trace in memory. The app declares `http` and `https`, and can register as a browser and ask to become the default on request. Opening browsers waits for the launcher.
- Installer and updates - Windows installs per user from a Velopack Setup.exe or MSI; macOS ships a DMG of the Velopack-packed app. Both check GitHub Releases at startup and from Check for Updates… (macOS menu), download a newer release and apply it on the next launch. Linux keeps the .deb and tar.gz.
- Rust port - Replaced the Electron and pnpm scaffold with a Cargo workspace: a GPUI window that draws the Rules screen, a rules engine that runs Finicky v4 `.js` and `.ts` configs in QuickJS and is tested against Finicky, and `cargo xtask` tasks for lint, license notices and packaging on macOS, Windows and Linux.
- Scaffold - Added the workspace, CI, release pipeline, license checks, design mirror and a desktop stub that shows the PwrRoute title strip. No app code yet.
