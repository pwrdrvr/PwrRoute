#!/usr/bin/env python3
"""Record the `suites` in tests/configs/expected.json from the PwrFinicky Go host.

Each case runs in its own process with a fresh data directory, because the Go
host's native helpers time out when several run at once:

    PwrFinicky --data-dir DIR --headless --dry-run [--config CONFIG] --test URL

`rules.json`, when a suite has one, is copied into DIR, where the Go router
reads it. That CLI cannot pass an opener, so opener cases run
`openeroracle.go` instead: the same router `Resolve`, built from the fork with
this file dropped into `apps/finicky/src/cmd/openeroracle/`.

    record.py --oracle PATH/PwrFinicky --opener-oracle PATH/openeroracle

A case with a `goHost` note records where the Go host is wrong: it keeps its
expected values (the engine's, explained by the note) and is not overwritten.
Each one is listed on stderr with what Go answered, to review the note.
"""

import argparse
import json
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
]


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
            cmd += ["--opener", "|".join([opener["name"], opener["bundleId"], opener["path"]])]
        if suite.get("config"):
            cmd += ["--config", str(CONFIGS / suite["config"])]
        cmd += ["--test", url]
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
        if out.returncode != 0:
            sys.exit(f"{suite['name']} {url}: exit {out.returncode}\n{out.stderr[-2000:]}")
        line = [l for l in out.stdout.splitlines() if l.strip().startswith("{")][-1]
        return json.loads(line)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--oracle", required=True)
    parser.add_argument("--opener-oracle")
    args = parser.parse_args()

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
