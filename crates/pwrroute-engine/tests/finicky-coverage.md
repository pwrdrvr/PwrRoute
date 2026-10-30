# Finicky test coverage

Every test in Finicky's repository (`johnste/finicky`, `main`), and every
feature its documentation names, mapped to the PwrRoute test that covers it.
Expectations come from Finicky: its own assertions, or the PwrFinicky Go
host (goja) recorded by `oracle/record.py`. Update this map when a Finicky
test or a PwrRoute test it names changes.

Where the tests run:

- **unit**: `finicky_suite.rs::finickys_config_api_unit_tests_pass_in_the_engine`
  runs `configs/finicky-unit.js` in the engine's QuickJS context; each `it`
  there keeps Finicky's describe/it name. `record.py --eval-oracle` runs the
  same file in the Go host's goja VM, where all cases pass too.
- **suite**: a suite in `configs/expected.json`, recorded from the Go host and
  checked by `finicky_compat.rs::suites_match_the_go_host_and_their_traces`.
- **configs**: `configs/expected.json`'s `configs`, checked by
  `finicky_compat.rs::matches_the_go_host`.

## packages/config-api/src (vitest, 79 tests)

| Finicky test | PwrRoute |
| --- | --- |
| `config.test.ts` › openUrl › validation | unit; `finicky_suite.rs::an_invalid_default_browser_is_rejected_as_finicky_rejects_it` |
| `config.test.ts` › openUrl › handlers (7 URLs, null opener) | unit; suite "Finicky config.test.ts handlers" |
| `config.test.ts` › openUrl › rewrites (4) | unit; suite "Finicky config.test.ts rewrites" |
| `config.test.ts` › openUrl › wildcards (7) | unit; suite "Finicky config.test.ts wildcards" |
| `config.test.ts` › openUrl › edge cases (3) | unit; suite "Finicky config.test.ts edge cases" |
| `wildcard.test.ts` › matchWildcard (13) | unit, through `isMatch(pattern, { href })`, which is how the API calls it; real links: suite "wildcards" (`configs/wildcards.ts`) |
| `FinickyURL.test.ts` › constructor, urlString, url, opener, keys, URL inheritance (10) | unit; legacy `({ url })` / `({ opener })` in configs: `finicky_suite.rs::legacy_url_arguments_work_and_warn`, suites "Finicky wiki Configuration ideas" and "Finicky wiki Slack recipe" |
| `utils.test.ts` › isMatch (8) | unit |
| `utils.test.ts` › rewriteUrl (11) | unit. The two vitest mocks are replaced by the real functions: a legacy object without `username` rewrites to `https://undefinedlegacy.com/…` in Finicky, and in PwrRoute |
| `utils.test.ts` › autodetectAppStringType (5) | unit; `rules.rs::app_type_matches_finickys_autodetect` (the Rust copy `rules.json` uses) |
| `utils.test.ts` › resolveBrowser (9) | unit |
| PwrFinicky's `config.test.ts` › candidate validation (2; in the bundle's source) | unit |

## apps/finicky/src/resolver/resolver_test.go

| Finicky test | PwrRoute |
| --- | --- |
| `TestResolveURL_NoConfig` | `finicky_suite.rs::no_config_resolves_to_safari_by_bundle_id` |
| `TestResolveURL_JSConfig` | suite "Finicky resolver_test.go JS config" |
| `TestResolveURL_JSONRulesOnly` | suite "Finicky resolver_test.go JSON rules only" |
| `TestResolveURL_JSONRulesWithProfile` | suite "Finicky resolver_test.go JSON rules with a profile" |
| `TestResolveURL_MergedJSAndJSON` | suite "Finicky resolver_test.go JS config without rules" |
| `TestResolveURL_MergedJSAndJSON_WithCachedRules` | suite "Finicky resolver_test.go JS config with cached rules"; also "rules after a CommonJS config", "rules after a TS config with rewrites" |
| `TestResolveURL_JSConfigFunctionHandler` | suite "Finicky resolver_test.go function handler" |
| `TestResolveURL_RewriteRule` | suite "Finicky resolver_test.go rewrite" |
| `TestResolveURL_OpenInBackground` | engine half: `finicky_suite.rs::open_in_background_is_unset_unless_the_config_sets_it`. Finicky's host asks for the background when Finicky is not frontmost; PwrRoute's launcher does not (see Gaps) |

## apps/finicky/src/rules/rules_test.go

| Finicky test | PwrRoute |
| --- | --- |
| `TestToJSHandlers_Empty` | `finicky_suite.rs::an_empty_rules_file_falls_back_to_safari`; `rules.rs::plan_follows_go_to_js_handlers` |
| `TestToJSHandlers_SkipsIncompleteRules` | `finicky_suite.rs::incomplete_rules_are_skipped_and_the_rest_route`; `rules.rs::plan_follows_go_to_js_handlers`. Not a suite: the PwrFinicky router refuses such a file, upstream skips the rule |
| `TestToJSHandlers_StringBrowser`, `_WithProfile`, `_MultipleRules` | `finicky_suite.rs::rules_keep_their_browser_profile_and_order`; suite "Finicky rules_test.go handlers" |
| `TestToJSConfigScript_DefaultBrowserFallback` | `finicky_suite.rs::an_empty_rules_file_falls_back_to_safari`; `rules.rs::base_config_follows_go_to_js_config_script` |
| `TestToJSConfigScript_ExplicitDefaultBrowser` | `finicky_suite.rs::rules_keep_their_browser_profile_and_order`; suite "Finicky resolver_test.go JSON rules only" |
| `TestToJSConfigScript_NamespaceIsUsed` | not applicable: the namespace is internal (`finickyConfig`), not part of the file |
| `TestLoadSave_RoundTrip` | `finicky_suite.rs::a_go_rules_file_round_trips_byte_for_byte_in_shape`; `rules.rs::writes_what_the_go_host_writes_when_no_extension_is_used` |
| `TestLoad_MissingFile`, `TestLoad_EmptyRulesNotNil` | `finicky_suite.rs::a_missing_file_or_absent_rules_load_empty`; `rules.rs::null_rules_are_empty` |

## testdata/ and example-config/

| Finicky file | PwrRoute |
| --- | --- |
| `testdata/config.js` | `configs/finicky/testdata-config.js` (verbatim); suites "Finicky testdata config.js" and "… with rules.json". Same file as `configs/legacy-commonjs.js` |
| `testdata/rules.json` | `configs/finicky/testdata-rules.json` (verbatim); suites "Finicky testdata rules.json" and "… with rules.json"; `finicky_suite.rs::finickys_testdata_rules_json_loads_unchanged` |
| `example-config/example.js`, `example.ts` | `configs/finicky-example.js`, `.ts` (verbatim); configs, suites "opener in the Finicky example", "opener with rules" |

## Documented API (wiki "Configuration (v4)", "Configuration ideas", `finicky.d.ts`)

| Feature | PwrRoute |
| --- | --- |
| Every example on "Configuration (v4)" | suite "Finicky wiki Configuration (v4)" (`configs/finicky/wiki-v4.js`) |
| "Configuration ideas" recipes that validate under Finicky 4 | suites "Finicky wiki Configuration ideas", "Finicky wiki Slack recipe"; the `finicky://open?url=` bookmark and a recipe run without an opener: `finicky_suite.rs::the_bookmark_recipe_…`, `::a_recipe_that_assumes_an_opener_…` |
| Finicky 3 recipes Finicky 4 rejects (partial `url` objects), `finicky.log` | `finicky_suite.rs::v3_partial_url_objects_fail_validation_as_in_finicky_4`, `::finicky_log_from_v3_is_reported_as_an_error` |
| Matchers: wildcard, regex, function, array | unit; suites "wildcards", "functions", wiki suites |
| Rewrites: string, URL, function, legacy object, chained | unit; suites "functions", "rules after a TS config with rewrites", wiki suites |
| Browser: name, bundle id, path, `name:profile`, object (`appType`, `profile`, `args`, `openInBackground`), function, `null` | unit; configs (`stress.ts`), suite "functions"; `finicky_compat.rs::null_browser_means_do_not_open` |
| `options.urlShorteners` | `features.rs` (`a_configs_url_shorteners_replace_…`, `an_empty_url_shorteners_turns_…`) |
| `options.logRequests`, `checkForUpdates`, `keepRunning`, `hideIcon` | validated: `finicky_suite.rs::every_documented_option_loads`; not acted on (see Gaps) |
| `opener.name`, `bundleId`, `path`, `null` | suites "openers", "Finicky resolver_test.go function handler", "Finicky wiki Configuration ideas" |
| `opener.windowTitle` | engine: suite "openers" (`MAIL_INBOX`); not captured by the macOS host (see Gaps) |
| `options.originalUrl` | suite "functions" (rewrites); `features.rs::a_config_sees_the_short_link_as_original_url` |
| `finicky.matchHostnames`, `matchDomains` | `finicky_suite.rs::the_finicky_global_answers_every_documented_utility`; configs (`stress.ts`) |
| `finicky.getModifierKeys`, `getSystemInfo`, `getPowerInfo`, `isAppRunning` | `finicky_suite.rs::the_finicky_global_answers_every_documented_utility`; `finicky_compat.rs::host_functions_and_console_reach_the_host`; per-OS answers: `src/native/mod.rs` |
| `finicky.notify`, `finicky.getBattery` (API stubs) | `finicky_suite.rs::the_finicky_global_answers_every_documented_utility` |
| `console.log`, `warn`, `error` | `finicky_suite.rs::console_levels_and_the_commonjs_warning_reach_the_host`; `features.rs::console_output_reaches_the_native_hosts_log` |
| `module.exports` (CommonJS) with its deprecation warning | suites "rules after a CommonJS config", "Finicky testdata config.js", "Finicky wiki Slack recipe"; `finicky_suite.rs::console_levels_and_the_commonjs_warning_reach_the_host` |
| `.ts` configs, relative imports | configs (`finicky-example.ts`, `stress.ts` + `helpers.ts`) |
| Config locations (`~/.finicky.js` …) | `discovery.rs::finicky_order_comes_first` |
| JSON rules (Finicky 4.4-alpha `rules.json`) merged after a config's handlers | suites "rules …", "Finicky resolver_test.go JS config with cached rules", "Finicky testdata config.js with rules.json" |

## Gaps

What PwrRoute's host does not do as Finicky's does. Configs load and route
the same; these are host features around them.

- `opener.windowTitle`: passed to configs when known, but the macOS host does
  not read the focused window's title (Finicky 4.4-alpha does, with
  Accessibility permission), so it is absent.
- `finicky://open/<base64>` links (Finicky's browser extension, since 4.1.0):
  PwrRoute does not register or decode the `finicky` scheme.
- Background opening when Finicky is not frontmost: Finicky's host asks for
  `open -g` unless the config sets `openInBackground`; PwrRoute brings the
  browser forward unless the config says otherwise.
- `options.logRequests`, `checkForUpdates`, `keepRunning`, `hideIcon`
  validate and are ignored: PwrRoute stays resident, records Activity in
  memory, and has its own update and icon settings.
- Finicky's `rules.json` (`~/Library/Application Support/Finicky/`) loads and
  routes unchanged, but PwrRoute reads only its own file and does not offer
  to import Finicky's.

## Where the Go host is wrong

Cases with a `goHost` note in `expected.json` hold the engine's answer:
goja leaves RegExp named groups undefined, so `stress.ts`'s Zoom rewrite and
the wiki's Slack recipe throw under the Go host (and under Finicky 4, which
also runs configs in goja). QuickJS supports them.
