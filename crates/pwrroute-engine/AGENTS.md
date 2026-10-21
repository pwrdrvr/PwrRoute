# pwrroute-engine

Loads a Finicky v4 config and `rules.json` and resolves URLs. Compatibility
with Finicky is the contract, so most rules here protect it. `src/lib.rs`'s
crate docs map the modules.

- **Routing logic is Finicky's.** `assets/finickyConfigAPI.js` is Finicky's
  configuration API, evaluated unchanged. Do not edit it, and do not
  reimplement matching or rewriting in Rust. `assets/README.md` records where
  the bundle came from; replace it only with a rebuild and update that table
  and `licenses/bundled.txt` in the same change.
- **The Rust side is the host.** It provides what the Go host did: `self`,
  `console`, the `finicky` global (the API's utilities plus the `Host`
  trait's native functions), module loading with oxc, and the `url`,
  `originalUrl`, `opener`, `finalConfig` globals `openUrl` reads.
  Wherever a real or simulated link is resolved, the host is a
  `NativeHost`; `DefaultHost` (all keys up, no apps running, no output) is
  only for loads that count or describe a config's rules.
- **`src/js/host.js` is PwrRoute's glue, not Finicky's.** It turns
  `rules.json` into Finicky objects, appends them after a config's own (as the
  Go resolver appended JSON handlers), and traces by wrapping matchers around
  the API's exported `isMatch`, `rewriteUrl` and browser handling while
  `openUrl` still decides. It also describes a config's own rules for the
  window (`Engine::describe`), spelling fixed browsers out with the API's
  `createBrowserConfig`. Keep it to the API's exports; if a feature needs
  the bundle changed, stop and say so.
- **`rules.json` is Go's schema plus optional fields.** Every extension
  (`rewrites`, `opener`, `enabled`) defaults to what the Go host did, so a Go
  file loads unchanged. A file using an extension behaves differently under
  Go, which ignores unknown fields; `src/rules.rs` has the table.
- **`tests/finicky_compat.rs` is the gate.** `tests/configs/expected.json`
  was recorded from the PwrFinicky Go host (goja), one URL per process.
  `configs` holds the original cases; `suites` (config, `rules.json`,
  openers) is written by `tests/oracle/record.py`. A case carrying a `goHost`
  note is one where that host is wrong and the engine is right; say why in the
  note. A case with `error` compares only the error, because Go replaced the
  result with `com.apple.Safari`. Never edit an expectation to match new
  engine output without re-running the Go host or explaining the difference
  there. Behavior Go never had (extensions, trace, short links with a fake
  network, reload) is tested in `tests/features.rs` and `tests/reload.rs`.
- **Limits stay on.** Every load and resolve runs under a deadline and a heap
  limit (`Limits`), so a looping or runaway config cannot hang the router.
  Short-link expansion has its own per-attempt deadline and redirect limit.
- **Short links expand before the rules run, and the config can say which.**
  `Engine::short_links()` is the built-in list unless the config sets
  `options.urlShorteners`: a list replaces it, and `[]` turns expansion off
  (the Go host ignored the option). `host.js`'s `shorteners()` returns
  `null` for an absent option so the two can be told apart; keep that. The
  engine never expands on its own: the router and the MCP engine thread
  call `ShortLinks::expand` with a `Network`, then `trace` the expanded
  `Link`, so `originalUrl` reaches the config.
- **URLs are private.** `Host::log` receives config `console` output, which
  often contains URLs; each run's lines also come back in `Trace::console`
  and `TestReport::console` (the last `ConsoleLog::CAPACITY`), for a screen
  or a tool result, never for a file. Nothing in this crate writes a URL the
  user opened to disk, or logs one: `RulesFile::save` writes only the rules it is given, and
  a `Network` must not record what it fetches. ureq logs requests through the
  `log` crate at debug level, so a host that writes logs to disk must filter
  `ureq` out. The examples and tests in `rules.json` are URLs the user chose
  to save; they are the one exception, and `run_tests` only reads them.
- **Privacy comes from the trace.** `resolve` is `trace` without the steps:
  one `openUrl` run, whose `rules.json` steps say which rewrites matched and
  which handler won, and those set `Browser::privacy` and `Browser::private`
  (`Engine::apply_privacy`). A JS config's result is `Full` and private only
  when its `args` carry a private flag. The engine never adds a flag; the
  launcher maps `private` to one.
- **Examples and tests run through the live engine.** `Engine::test_rules`
  (and `RulesFile::run_tests`) resolve every example and test through the
  JS config and `rules.json` together, so a change to either is caught; a
  proposed file is tested on a temporary engine built from the same config.
  `suggest_rule` only shapes candidate patterns; whether one matches is
  always the API's `isMatch`, through `Engine::collisions`.
- **The importer reads what the config produced (`src/import.rs`).**
  `Engine::review_import` classifies `describe()`'s rules: wildcard
  matchers, fixed browsers (their `appType` must be the one `rules.json`
  would detect) and string rewrites import; regexes (`rules.json` holds
  none), functions and arguments stay JS. A suggestion is found by
  probing, never by reading JS: literals make candidate patterns, and
  `host.js`'s `probe` runs the real matchers inside `openUrl` beside
  `isMatch` on each pattern. Probe links are built from literals plus the
  caller's recent links, in memory only.
- **Unsafe is confined.** The crate denies `unsafe_code`; only
  `src/native/macos.rs` and `src/native/windows.rs` allow it, for FFI, with a
  safety comment on each block. `src/native/mod.rs` documents what each OS
  can answer.
- **Never watch a directory.** Discovery and reload check exact paths only;
  on macOS a directory watch can open protected folders and prompt the user.
- oxc's enum transform needs `SemanticBuilder::with_enum_eval(true)`; without
  it string enums get wrong reverse mappings, and only a debug assertion says
  so.
