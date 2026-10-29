#!/usr/bin/env python3
"""Moves files between this machine and a Sandboxer sandbox.

Runs in the sandbox. Serves one directory over HTTP: PUT /<name> stores a
file, GET /<name> returns one. The Sandboxer MCP's host_port puts it behind
an HTTPS URL that carries an access token, so the laptop can `curl` a git
bundle in and a generated file (THIRD_PARTY_LICENSES) out without either
passing through the agent's context. The procedure is in
.agents/skills/sandboxer-verification/SKILL.md.

    python3 transfer.py [DIR] [PORT]     default ~/pwrroute/files, 8722
"""

import http.server
import os
import sys

ROOT = os.path.realpath(sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser("~/pwrroute/files"))
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 8722


def target(path):
    """The file a request names, or None for anything outside ROOT."""
    name = path.split("?", 1)[0].lstrip("/")
    if not name or "/" in name or name.startswith("."):
        return None
    return os.path.join(ROOT, name)


class Handler(http.server.BaseHTTPRequestHandler):
    def do_PUT(self):
        path = target(self.path)
        length = int(self.headers.get("Content-Length", "-1"))
        if path is None or length < 0:
            self.send_error(400)
            return
        partial = path + ".partial"
        with open(partial, "wb") as out:
            remaining = length
            while remaining > 0:
                chunk = self.rfile.read(min(remaining, 1 << 20))
                if not chunk:
                    break
                out.write(chunk)
                remaining -= len(chunk)
        if remaining:
            os.remove(partial)
            self.send_error(400, "short body")
            return
        os.replace(partial, path)
        self.send_response(201)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_GET(self):
        path = target(self.path)
        if path is None or not os.path.isfile(path):
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Length", str(os.path.getsize(path)))
        self.end_headers()
        with open(path, "rb") as src:
            while chunk := src.read(1 << 20):
                self.wfile.write(chunk)


os.makedirs(ROOT, exist_ok=True)
http.server.ThreadingHTTPServer(("0.0.0.0", PORT), Handler).serve_forever()
