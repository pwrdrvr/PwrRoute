#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../.."

if [[ "$(uname -s)" != Darwin ]]; then
  echo "The bundled debug Start action currently requires macOS." >&2
  exit 1
fi

# Build first, so a compiler error leaves the running router available.
cargo build --locked -p pwrroute-app
swift scripts/dev/quit-running.swift
cargo xtask bundle --debug --app-only

echo "Starting $PWD/target/dev/PwrRoute.app (requests default-browser ownership)."
exec "$PWD/target/dev/PwrRoute.app/Contents/MacOS/pwrroute" --make-default
