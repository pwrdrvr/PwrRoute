#!/usr/bin/env python3
"""Record the `suites` in tests/configs/expected.json from the PwrFinicky Go host.

Each case runs in its own process with a fresh data directory, because the Go
host's native helpers time out when several run at once:

    PwrFinicky --data-dir DIR --headless --dry-run [--config CONFIG] --test URL

`rules.json`, when a suite has one, is copied into DIR, where the Go router
reads it. That CLI cannot pass an opener, so opener cases run
`openeroracle.go` instead: the same router `Resolve`, built from the fork with
this file dropped into `apps/finicky/src/cmd/openeroracle/`.

    record.py --oracle PATH/PwrFinicky --opener-oracle PATH/openeroracle \
        --eval-oracle PATH/evaloracle

To build the three from the fork without touching a checkout: `git archive`
the `pwrfinicky` branch's `apps/finicky/src` into a scratch directory, copy
this crate's `assets/finickyConfigAPI.js` to both `assets/finickyConfigAPI.js`
and `router/config-api.js` there (the build embeds them; they are not in
git), drop each harness in as `cmd/<name>/main.go`, and `go build ./cmd/...`.
A run that leaves every existing case unchanged confirms the build.

A case with a `goHost` note records where the Go host is wrong: it keeps its
expected values (the engine's, explained by the note) and is not overwritten.
Each one is listed on stderr with what Go answered, to review the note.

Every run gets an unreachable HTTP proxy, so the Go host's short-link
expansion (which ignores `options.urlShorteners`) fails and leaves each link
as given, as the engine's `resolve` does.

`--eval-oracle` also runs tests/configs/finicky-unit.js (Finicky's own unit
tests for its config API) in the Go host's goja VM with `evaloracle.go`
(built like `openeroracle`, from `apps/finicky/src/cmd/evaloracle/`) and
fails unless every case passes there; tests/finicky_suite.rs runs the same
file in the engine. It writes nothing.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

CONFIGS = Path(__file__).resolve().parent.parent / "configs"

SLACK = {"name": "Slack", "bundleId": "com.tinyspeck.slackmacgap", "path": "/Applications/Slack.app/Contents/MacOS/Slack"}
TERMINAL = {"name": "Terminal", "bundleId": "com.apple.Terminal", "path": "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal"}
OUTLOOK = {"name": "Microsoft Outlook", "bundleId": "com.microsoft.Outlook", "path": "/Applications/Microsoft Outlook.app/Contents/MacOS/Microsoft Outlook"}
MAIL = {"name": "Mail", "bundleId": "com.apple.mail", "path": "/System/Applications/Mail.app/Contents/MacOS/Mail"}
BLANK = {"name": "", "bundleId": "", "path": ""}
MAIL_INBOX = dict(MAIL, windowTitle="Inbox")
TESTAPP = {"name": "TestApp", "bundleId": "com.test.app", "path": "/Applications/TestApp.app"}
GO_TERMINAL = {"name": "Terminal", "bundleId": "com.apple.Terminal", "path": "/Applications/Utilities/Terminal.app"}
GO_FINDER = {"name": "Finder", "bundleId": "com.apple.finder", "path": "/System/Library/CoreServices/Finder.app"}
TELEGRAM = {"name": "Telegram", "bundleId": "ru.keepcoder.Telegram", "path": "/Applications/Telegram.app"}

# The links Finicky's testdata/config.js and testdata/rules.json route.
TESTDATA_URLS = [
    "https://example.com/a",
    "https://github.com/johnste/finicky",
    "https://linear.app/team/issue/1",
    "https://other.example/",
]

RULES_URLS = [
    "https://github.com/pwrdrvr/PwrRoute",
    "https://acme.atlassian.net/browse/X-1",
    "https://linear.app/pwrdrvr/issue/1",
    "https://acme.zoom.us/j/123456789",
    "https://www.example.org/?utm_source=slack&keep=1",
    "https://example.org/page",
    "http://example.com/insecure",
    "https://other.example/",
]

SUITES = [
    {
        "name": "functions",
        "config": "functions.js",
        "urls": [
            "https://chain.example/start",
            "https://legacy.example/old",
            "https://boom.example/x",
            "https://static.example/anything",
            "https://case.example/x",
            "https://bundle.example/",
            "https://path.example/",
            "https://undefined.example/",
            "https://invalid.example/",
            "https://args.example/?q=hello",
            "https://api.service.example/v1",
            "https://exact.example/",
            "https://second.example/page",
            "https://handler-boom.example/",
            "https://site.dev/",
            "https://other.example/",
        ],
    },
    {
        "name": "wildcards",
        "config": "wildcards.ts",
        "urls": [
            "https://exact.example/",
            "https://exact.example",
            "https://a.sub.example/x",
            "http://a.sub.example/x",
            "https://sub.example/x",
            "HTTPS://UPPER.SUB.EXAMPLE/Path",
            "https://user:pw@a.sub.example:8443/p?q=1#frag",
            "https://scheme.example/a",
            "http://scheme.example/a",
            "ftp://scheme.example/a",
            "https://path.example/docs/intro",
            "https://path.example/blog",
            "https://any.example/p?share=1",
            "https://port.example:8443/x",
            "https://port.example/x",
            "https://literal.example/a*b",
            "https://literal.example/aXb",
            "https://bücher.example/",
            "mailto:dev@team.example",
            "https://x.example/case/1",
        ],
    },
    {"name": "rules only", "rules": "rules/go-schema.json", "urls": RULES_URLS},
    {"name": "rules after a CommonJS config", "config": "legacy-commonjs.js", "rules": "rules/go-schema.json", "urls": RULES_URLS},
    {"name": "rules after a TS config with rewrites", "config": "stress.ts", "rules": "rules/go-schema.json", "urls": RULES_URLS},
    {
        "name": "rules with a default profile",
        "rules": "rules/default-profile.json",
        "urls": ["https://printer.local/status", "https://example.com/"],
    },
    {
        "name": "openers",
        "config": "opener.js",
        "cases": [
            ("https://example.com/a", SLACK),
            ("https://example.com/a", None),
            ("http://localhost:3000/", TERMINAL),
            ("https://example.com/", TERMINAL),
            ("https://example.com/", OUTLOOK),
            ("https://direct.example/", None),
            ("https://direct.example/", SLACK),
            ("https://direct.example/", BLANK),
            ("https://example.com/", MAIL_INBOX),
        ],
    },
    {
        "name": "opener in the Finicky example",
        "config": "finicky-example.ts",
        "cases": [
            ("https://example.org/", SLACK),
            ("https://example.org/", MAIL),
            ("https://workplace.example/", SLACK),
        ],
    },
    {
        "name": "opener with rules",
        "config": "finicky-example.ts",
        "rules": "rules/go-schema.json",
        "cases": [("https://github.com/x", SLACK), ("https://github.com/x", None)],
    },
    # Finicky's own tests, through its configs and rules files: config.test.ts
    # (packages/config-api), resolver_test.go and rules_test.go
    # (apps/finicky/src), and its testdata/ pair.
    {
        "name": "Finicky config.test.ts handlers",
        "config": "finicky/config-test-handlers.js",
        "cases": [
            ("https://example.com", TESTAPP),
            ("https://browser.spotify.com/track/123", TESTAPP),
            ("https://github.com/some-repo", TESTAPP),
            ("https://amazon.com/product", TESTAPP),
            ("https://figma.com/file/123", TESTAPP),
            ("https://sketch.com/dashboard", TESTAPP),
            ("https://example.com/docs", TESTAPP),
            ("https://example.com", None),
        ],
    },
    {
        "name": "Finicky config.test.ts rewrites",
        "config": "finicky/config-test-rewrites.js",
        "cases": [
            ("https://slack-redir.net/link?url=https://example.com", TESTAPP),
            ("https://t.co/abc123", TESTAPP),
            ("https://my.shortened.url/123", TESTAPP),
            ("https://www.use-url-object.com", TESTAPP),
        ],
    },
    {
        "name": "Finicky config.test.ts wildcards",
        "config": "finicky/config-test-wildcards.js",
        "cases": [
            (u, TESTAPP)
            for u in [
                "https://sub1.example.com",
                "https://sub2.example.com/path",
                "https://mail.google.com",
                "https://docs.google.co.uk",
                "https://mail.yahoo.com",
                "https://example.com",
                "https://google.com",
            ]
        ],
    },
    {
        "name": "Finicky config.test.ts edge cases",
        "config": "finicky/config-test-edge-cases.js",
        "cases": [
            (u, TESTAPP)
            for u in ["http://example.com", "https://example.com", "http://example.com/?param=value&other=123"]
        ],
    },
    {
        "name": "Finicky resolver_test.go JS config",
        "config": "finicky/resolver-test-js.js",
        "urls": [
            "https://github.com/johnste/finicky",
            "https://gist.github.com/foo",
            "https://linear.app/team/issue/123",
            "https://example.com",
        ],
    },
    {
        "name": "Finicky resolver_test.go JSON rules only",
        "rules": "finicky/resolver-test-rules-only.json",
        "urls": ["https://github.com/johnste/finicky", "https://linear.app/team/issue/123", "https://example.com"],
    },
    {
        "name": "Finicky resolver_test.go JSON rules with a profile",
        "rules": "finicky/resolver-test-rules-profile.json",
        "urls": ["https://github.com/foo"],
    },
    {
        "name": "Finicky resolver_test.go JS config without rules",
        "config": "finicky/resolver-test-merged.js",
        "urls": ["https://github.com/foo", "https://example.com"],
    },
    {
        "name": "Finicky resolver_test.go JS config with cached rules",
        "config": "finicky/resolver-test-merged.js",
        "rules": "finicky/resolver-test-cached-rules.json",
        "urls": ["https://github.com/foo", "https://linear.app/team/issue/1", "https://example.com"],
    },
    {
        "name": "Finicky resolver_test.go function handler",
        "config": "finicky/resolver-test-function-handler.js",
        "cases": [("https://example.com", GO_TERMINAL), ("https://example.com", GO_FINDER)],
    },
    {
        "name": "Finicky resolver_test.go rewrite",
        "config": "finicky/resolver-test-rewrite.js",
        "urls": ["https://www.youtube.com/watch?v=dQw4w9WgXcQ"],
    },
    {
        "name": "Finicky rules_test.go handlers",
        "rules": "finicky/rules-test-handlers.json",
        "urls": [
            "https://github.com/johnste/finicky",
            "https://linear.app/team/issue/1",
            "https://example.com/",
        ],
    },
    {"name": "Finicky testdata rules.json", "rules": "finicky/testdata-rules.json", "urls": TESTDATA_URLS},
    {"name": "Finicky testdata config.js", "config": "finicky/testdata-config.js", "urls": TESTDATA_URLS},
    {
        "name": "Finicky testdata config.js with rules.json",
        "config": "finicky/testdata-config.js",
        "rules": "finicky/testdata-rules.json",
        "urls": TESTDATA_URLS,
    },
    # Finicky's wiki: the "Configuration (v4)" page and "Configuration ideas".
    {
        "name": "Finicky wiki Configuration (v4)",
        "config": "finicky/wiki-v4.js",
        "urls": [
            "https://example.org/page",
            "https://string.example.org/x",
            "https://instance.example.org/",
            "https://dynamic.example.org/x",
            "https://old.example.com/path?q=1",
            "https://old.example.net/a",
            "https://redirect.example.com/p?q=1",
            "https://apple.com/mac",
            "http://match.example.com/x",
            "https://a.sub.example.com/x",
            "https://regex.example.com/x",
            "https://x.dev.example.com",
            "https://fn.example.com/",
            "https://other.example/api/v1",
            "https://combined.example.com",
            "http://combined.org/x",
            "https://simple.example.org/",
            "https://byname.example/",
            "https://bybundle.example/",
            "https://bypath.example/",
            "https://profile-string.example/",
            "https://profile-object.example/",
            "https://advanced.example/",
            "https://dynamic.example/",
            "https://www.facebook.com/",
            "https://work.example.com/",
            "https://personal.example.com/",
            "https://safari.example.com/",
            "https://site.dev/",
        ],
    },
    {
        "name": "Finicky wiki Configuration ideas",
        "config": "finicky/wiki-ideas.js",
        "cases": [
            ("http://plain.example/x", TERMINAL),
            ("https://example.com/?utm_source=x&fbclid=y&keep=1&source=share", TERMINAL),
            ("https://google.com/search?q=1", TERMINAL),
            ("https://acme.zoom.us/j/123456789?pwd=abc", TERMINAL),
            ("https://vk.com/away.php?to=https%253A%252F%252Fexample.org%252F", TERMINAL),
            ("https://app.clickup.com/t/86c1au25p", TERMINAL),
            ("https://example.com/", MAIL),
            ("https://example.com/", OUTLOOK),
            ("https://drive.google.com/x", SLACK),
            ("https://drive.google.com/x", TERMINAL),
            ("https://www.example.com/Finicky-docs", TERMINAL),
            ("https://trello.com/b/1", TERMINAL),
            ("https://open.spotify.com/track/1", TERMINAL),
            ("https://teams.microsoft.com/l/1", TERMINAL),
            ("https://www.figma.com/file/abc", TERMINAL),
            ("https://discord.com/channels/1", TERMINAL),
            ("https://meet.jit.si/room", TERMINAL),
            ("https://example.org/", TELEGRAM),
            ("https://meet.google.com/abc-defg-hij", TERMINAL),
            ("https://www.office.com/", TERMINAL),
            ("https://music.apple.com/us/album/1", TERMINAL),
            ("https://unrouted.example/", TERMINAL),
        ],
    },
    {
        "name": "Finicky wiki Slack recipe",
        "config": "finicky/wiki-slack.js",
        "urls": [
            "https://acme.slack.com/archives/C0123/p1700000000123456",
            "https://app.slack.com/client/T999/C888",
            "https://acme.enterprise.slack.com/files/U1/F123",
            "https://other.slack.com/archives/C1",
            "https://acme.slack.com/",
            "https://example.com/",
        ],
    },
]


# An unreachable proxy: the Go host's short-link requests fail fast. Go reads
# each variable in upper then lower case and takes the first that is set, so
# both spellings are overridden (a caller's `no_proxy=*` would skip the proxy).
DEAD_PROXY = "http://127.0.0.1:9"
OFFLINE = dict(
    os.environ,
    HTTP_PROXY=DEAD_PROXY,
    http_proxy=DEAD_PROXY,
    HTTPS_PROXY=DEAD_PROXY,
    https_proxy=DEAD_PROXY,
    NO_PROXY="",
    no_proxy="",
)


def check_unit(evaloracle):
    """Runs finicky-unit.js in the Go host's VM; every case must pass."""
    api = CONFIGS.parent.parent / "assets" / "finickyConfigAPI.js"
    cmd = [evaloracle, "--api", str(api), "--script", str(CONFIGS / "finicky-unit.js"), "--expr", "finickyUnit.run()"]
    out = subprocess.run(cmd, capture_output=True, text=True, timeout=60, env=OFFLINE)
    if out.returncode != 0:
        sys.exit(f"evaloracle: exit {out.returncode}\n{out.stderr[-2000:]}")
    results = json.loads(out.stdout)
    failed = [r for r in results if not r["ok"]]
    for r in failed:
        print(f"finicky-unit.js under Go: {r['file']} {r['name']}: {r['error']}", file=sys.stderr)
    if failed:
        sys.exit(f"{len(failed)} of {len(results)} finicky-unit.js cases fail under the Go host")
    print(f"finicky-unit.js: {len(results)} cases pass under the Go host", file=sys.stderr)


def run(args, suite, url, opener):
    with tempfile.TemporaryDirectory(prefix="pwrroute-oracle-") as data:
        if suite.get("rules"):
            shutil.copy(CONFIGS / suite["rules"], Path(data) / "rules.json")
        binary = args.opener_oracle if opener is not None else args.oracle
        if binary is None:
            sys.exit("opener cases need --opener-oracle")
        cmd = [binary, "--data-dir", data]
        if opener is None:
            cmd += ["--headless", "--dry-run"]
        else:
            fields = [opener["name"], opener["bundleId"], opener["path"]]
            if opener.get("windowTitle"):
                fields.append(opener["windowTitle"])
            cmd += ["--opener", "|".join(fields)]
        if suite.get("config"):
            cmd += ["--config", str(CONFIGS / suite["config"])]
        cmd += ["--test", url]
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=60, env=OFFLINE)
        if out.returncode != 0:
            sys.exit(f"{suite['name']} {url}: exit {out.returncode}\n{out.stderr[-2000:]}")
        line = [l for l in out.stdout.splitlines() if l.strip().startswith("{")][-1]
        return json.loads(line)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--oracle", required=True)
    parser.add_argument("--opener-oracle")
    parser.add_argument("--eval-oracle")
    args = parser.parse_args()
    if args.eval_oracle:
        check_unit(args.eval_oracle)

    path = CONFIGS / "expected.json"
    expected = json.loads(path.read_text())
    notes = {}
    for old in expected.get("suites", []):
        for case in old["cases"]:
            if "goHost" in case:
                key = (old["name"], case["input"], json.dumps(case.get("opener")))
                notes[key] = case

    suites = []
    for suite in SUITES:
        pairs = suite.get("cases") or [(u, None) for u in suite["urls"]]
        cases = []
        for url, opener in pairs:
            got = run(args, suite, url, opener)
            old = notes.get((suite["name"], url, json.dumps(opener)))
            if old:
                cases.append(old)
                print(f"goHost note kept: {suite['name']} {url}: Go says {got['browser']} {got['url']} {got.get('error', '')[:120]!r}", file=sys.stderr)
                continue
            case = {"input": url}
            if opener is not None:
                case["opener"] = opener
            for key in ("url", "browser", "profile", "args", "openInBackground"):
                case[key] = got.get(key)
            if got.get("error"):
                case["error"] = got["error"]
            cases.append(case)
            print(f"{suite['name']}: {url} -> {case['browser']} {case['url']}", file=sys.stderr)
        entry = {"name": suite["name"]}
        for key in ("config", "rules"):
            if suite.get(key):
                entry[key] = suite[key]
        entry["cases"] = cases
        suites.append(entry)

    expected["suites"] = suites
    path.write_text(json.dumps(expected, indent=2, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
