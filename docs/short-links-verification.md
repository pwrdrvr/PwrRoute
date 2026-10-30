# Short-link verification — 2026-10-30

The router already expanded short links before resolving rules and kept the
clicked URL plus an `Expanded` step for Activity. The Test a link window did
not: it traced `Link::new(url)` directly, and its Received row displayed the
landing URL even when a trace supplied `original_url`.

Test a link now expands on a background executor, then resolves on its
QuickJS-owning UI thread. Received shows the clicked URL and a separate
Expanded short link step shows the destination before the rules. A pending
request cannot replace a newer run; changed configuration files trigger a
new run. Ordinary links and a disabled shortener list make no requests.

## Repeatable checks

- `crates/pwrroute-engine/tests/features.rs`: `.js` and `.ts` replacement
  lists, `[]`, `originalUrl` through expansion, and a fresh 750 ms budget for
  each method shared across its redirects.
- `crates/pwrroute-engine/tests/http_network.rs`: the real ureq client on a
  loopback server. HEAD/GET fallback, relative Location headers, three
  redirects per attempt (the third target is used without being requested),
  response bodies left unread, 503, slow headers, and connection refusal.
- Router tests cover rules.json alone, the launch destination, Activity's
  clicked/destination pair, its Test API, and privacy reduction.
- Window tests cover the displayed original/expanded URLs, rule selection,
  and a newer test winning over an old pending expansion.
- Diagnostic log tests cover ureq filtering even with PwrRoute trace logging,
  including a real log-file write that retains a safe marker and drops an
  HTTP request containing a contrived URL.

All these checks run without a public shortener. The app still permits URLs
on disk only through existing explicit features such as the optional
Activity journal or saving a test; expansion itself does not persist them.

## Manual public-network probe

Run `cargo run -p pwrroute-router --example shortlinks`, then provide a JSON
array on stdin. Each entry has `url` (a public short link) and `host` (the
expected destination hostname). For example, substitute a current public
test code for this placeholder:

```json
[{"url":"https://bit.ly/PUBLIC_TEST_CODE","host":"example.com"}]
```

The probe builds a rules.json-only engine with that host routed to Firefox
and Safari as the default. It uses the real router and HTTP client, checks
its dry-run launch, the stored Activity record and the router's Test API,
and exits nonzero if any check fails. It opens no browser, installs no
logger or journal, and prints only case numbers, booleans, methods, status
codes and timings. No short link or landing URL is printed or saved.

The probe intentionally does not ship live aliases as fixtures. Services
change their aliases and destinations; supply current public test links and
verify the expected hostname. An unchanged URL or a retained redirect target
can be the correct bounded fallback rather than a transport bug.

## Public-network observations

Ubuntu x64, Sandboxer `sb_mc6e4bssnh`, code commit `df377e44`. Each successful
probe case includes a dispatch and a separate Test API request. The times
below include both, not one expansion.

| Service | Observation | Combined time |
| --- | --- | --- |
| bit.ly | Passed routing, Activity and Test checks after correcting an outdated expected host. Its destination timed out on some attempts; the last redirect target still selected the expected rule. Another old alias returned 404. | 2,807 ms |
| t.co | Passed; HEAD returned 301 and the destination returned 200. | 354 ms |
| tinyurl.com | Passed; two HEAD redirects followed by 200. | 365 ms |
| aka.ms | Passed; both methods reached the three-redirect cap and routed its last target. This does not establish a final 200 beyond the cap. | 826 ms |
| Outlook Safe Links | A synthetic URL wrapping example.com returned HEAD 500 and GET 400; it stayed unchanged and did not match the destination rule. No valid tenant-issued URL was available, so successful Safe Links expansion remains unverified. | 363 ms |

The app does not decode a Safe Links query to bypass the service's response.
Offline and slow behavior were verified with deterministic fakes and real
loopback HTTP, rather than disconnecting the operator's machine.

Verification of that code commit: lint exit 0, workspace tests exit 0
(1,213 passed), Linux Desktop E2E exit 0 (15 passed). These runs do not verify
macOS or Windows native behavior. An isolated Linux window was also driven
through the E2E control socket against a loopback shortener using example.com
hostnames: the rendered trace showed Received, Expanded short link and the
Firefox handler result, with two real HTTP requests and no fixture URL in
the captured app log.

## rules.json controls

Recommendation: add optional `options.urlShorteners`. Absent inherits the
built-in list, `[]` disables expansion, and a list replaces it. An explicit
Finicky config option, including an empty list, should win independently of
handler precedence; otherwise rules.json should apply. Settings should name
the effective owner and offer Built-in, Custom and Off, preserving the
custom draft while the pane remains open.

This is designed in the PwrRoute Claude Design project and mirrored in
`design/PwrRoute Short Links.dc.html`. It is a proposal for a follow-up;
this verification fix does not add the schema or Settings controls.
