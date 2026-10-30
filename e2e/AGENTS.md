# e2e

Desktop E2E: launches the built PwrRoute and drives it through the
env-gated control socket in `crates/pwrroute-app/src/e2e.rs`. `CONTRACT.md`
is the protocol and what the router must report; read it before changing
either side.

## Run

```bash
cargo xtask e2e                     # build, then every live test
cargo xtask e2e -- quits            # a name filter, as for cargo test
cargo xtask e2e --pending           # also the #[ignore]d scenarios
cargo xtask e2e --screenshots       # window-only PNGs into the artifacts
```

Artifacts (each test's `app.log`, which has the app's own log, its
`startup.json`, state dumps on failure, screenshots) land in
`target/e2e-artifacts/<test>/`. On macOS the binary is wrapped in an ad hoc
signed `target/e2e/PwrRoute.app`. Both follow `CARGO_TARGET_DIR` when it is
set. Without `cargo xtask e2e` (plain
`cargo test`, `cargo xtask test`) every test here returns at once.

## Why a hook, not accessibility or screenshots

GPUI has no DOM. It does build an AccessKit tree on all three OSes, but only
for elements with an id and a role (PwrRoute sets none yet), and only after a
screen reader activates it. Driving it would mean three client stacks (AX
with a TCC grant, UI Automation, AT-SPI on a D-Bus session). Screenshots
carry no semantics and differ by font and DPI. The hook reads GPUI's own
state and the router's in-memory dispatch records, and injects input through
`Window::dispatch_event`, the path platform events take. What only the OS
can show is checked from outside: the process lifecycle, the OS's own window
list (`os.rs`), real link hand-offs (`open -a`, a second launch).

## Rules

- **Never change this machine's default browser.** Scenarios that register
  PwrRoute or make it the default skip unless `PWRROUTE_E2E_DISPOSABLE=1`,
  which only GitHub-hosted runners set, and the app refuses the hook's
  registration actions without it too. Never set it locally.
- **Isolated by default.** Each launch has its own router directory
  (`PWRROUTE_E2E_ROUTER_DIR`), socket and empty home, and passes
  `--dry-run`, so it never finds the developer's config. A shared launch
  (`isolated: false`, disposable runners only) keeps the real home, so the
  copy the OS starts finds the same router and registration; it must pass a
  config. Window tests launch
  with no config and `fixtures/rules.json` seeded in the empty home (the
  editable rules); routing tests use `Launch::routing()` for
  `fixtures/routing.js` alone.
- **Contrived URLs only** (`example.com`, `.org`, `.net`). Screenshots are
  off unless asked for, and capture the PwrRoute window only.
- **Pending means `#[ignore = "pending: …"]`** with a pointer to the
  `CONTRACT.md` section. When the app meets it, remove the `ignore` in the
  same change.
- **Regions are positional.** `e2e::regions` names a div's children in
  order; keep the lists in `app.rs`, `chrome.rs` and `editor.rs` in step
  with the children.
- Tests run one at a time (`--test-threads=1`): they open real windows.
