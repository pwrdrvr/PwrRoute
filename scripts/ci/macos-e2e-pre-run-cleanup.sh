#!/bin/bash
# Before Desktop E2E on a persistent pwrdrvr-macos Tart guest: stop PwrRoute
# copies an earlier job in this runner's workspace left running, and drop the
# app's saved window state, so a "reopen windows?" alert cannot block the
# launch. PwrGit's macos-e2e-pre-run-cleanup.sh, cut down to one app.
#
# Only processes whose executable lives under this runner's workspace are
# touched: the guest is shared with other repositories and an operator may be
# using it.
set -euo pipefail

scope="${RUNNER_WORKSPACE:-${GITHUB_WORKSPACE:-}}"
if [[ -z "$scope" ]]; then
  echo "::error::RUNNER_WORKSPACE is not set; refusing to guess a kill scope on a shared machine."
  exit 1
fi
scope="${scope%/}/"

leftovers() {
  # pid and executable path of every process whose argv[0] is a pwrroute
  # binary under the workspace.
  ps -Awwo pid=,comm= | awk -v scope="$scope" '
    { pid = $1; $1 = ""; sub(/^ /, ""); }
    index($0, scope) == 1 && $0 ~ /\/pwrroute$/ { print pid }'
}

for _ in 1 2; do
  pids=$(leftovers || true)
  [[ -z "$pids" ]] && break
  echo "::warning::Stopping PwrRoute left running by an earlier job: $pids"
  # shellcheck disable=SC2086 # one pid per word
  kill -TERM $pids 2>/dev/null || true
  sleep 2
  pids=$(leftovers || true)
  # shellcheck disable=SC2086
  [[ -n "$pids" ]] && kill -KILL $pids 2>/dev/null || true
  sleep 1
done
if [[ -n "$(leftovers || true)" ]]; then
  echo "::error title=Unkillable PwrRoute processes on macOS runner::recycle the guest"
  exit 1
fi

bundle_id="com.pwrdrvr.pwrroute"
rm -rf "$HOME/Library/Saved Application State/$bundle_id.savedState"
# Preferences only on CI: run by hand on a developer Mac, the script must not
# leave settings behind.
if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
  defaults write "$bundle_id" ApplePersistenceIgnoreState -bool true 2>/dev/null || true
  defaults write "$bundle_id" NSQuitAlwaysKeepsWindows -bool false 2>/dev/null || true
fi
echo "macOS E2E guest is clean."
