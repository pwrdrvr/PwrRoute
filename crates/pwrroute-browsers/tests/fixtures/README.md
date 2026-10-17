# Test fixtures

Contrived data only: never copy a real `Local State` or `profiles.ini` here.

| File | What it is |
| --- | --- |
| `finicky-browsers.json` | Finicky's `apps/finicky/src/browser/browsers.json`, unchanged, at upstream commit `4a38875` (johnste/finicky, MIT, Copyright (c) 2015-2025 John Sterling). `catalog::tests::covers_finickys_browsers_json` checks the catalog keeps every entry. |
| `chromium-local-state.json` | A Chromium `Local State` with three profiles. |
| `firefox-profiles.ini` | A Firefox 67+ `profiles.ini` (CRLF, an `[Install…]` default) with two profiles. |
