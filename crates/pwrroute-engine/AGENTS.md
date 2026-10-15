# pwrroute-engine

Loads a Finicky v4 config and resolves URLs. Compatibility with Finicky is the
contract, so most rules here protect it.

- **Routing logic is Finicky's.** `assets/finickyConfigAPI.js` is Finicky's
  configuration API, evaluated unchanged. Do not edit it, and do not
  reimplement matching or rewriting in Rust. `assets/README.md` records where
  the bundle came from; replace it only with a rebuild and update that table
  and `licenses/bundled.txt` in the same change.
- **The Rust side is the host.** It provides what the Go host did: `self`,
  `console`, the `finicky` global (the API's utilities plus the `Host`
  trait's native functions), module loading with oxc, and the `url`,
  `originalUrl`, `opener`, `finalConfig` globals `openUrl` reads.
- **`tests/finicky_compat.rs` is the gate.** `tests/configs/expected.json`
  was recorded from the PwrFinicky Go host (goja), one URL per process. A case
  carrying a `goHost` note is one where that host is wrong and the engine is
  right; say why in the note. Never edit an expectation to match new engine
  output without re-running the Go host or explaining the difference there.
- **Limits stay on.** Every load and resolve runs under a deadline and a heap
  limit (`Limits`), so a looping or runaway config cannot hang the router.
- **URLs are private.** `Host::log` receives config `console` output, which
  often contains URLs. Nothing in this crate writes to disk.
- oxc's enum transform needs `SemanticBuilder::with_enum_eval(true)`; without
  it string enums get wrong reverse mappings, and only a debug assertion says
  so.
