#!/bin/bash
# Before Desktop E2E on a pwrdrvr-macos Tart guest: require that this user's
# session owns the console and its screen is not locked. GPUI draws nothing
# for a window the platform does not present, and behind the lock screen no
# window is presented: each test's window paints its first frames and stops,
# and the app cannot come to the front. Fail here, with the reason, instead
# of after every wait in the suite has timed out.
#
# Read-only: this reports the session and display state and changes nothing.
# Unlocking a guest, or turning its screen lock off, is for the lab operator.
set -euo pipefail

user="$(/usr/bin/id -un)"
sessions="$(/usr/sbin/ioreg -n Root -d1 -a | /usr/bin/plutil -extract IOConsoleUsers json -o - - 2>/dev/null || true)"
if [[ -z "$sessions" ]]; then
  echo "::warning::Cannot read the console session; running Desktop E2E anyway."
  exit 0
fi

state="$(/usr/bin/python3 -c '
import json, sys
user = sys.argv[1]
mine = [s for s in json.loads(sys.stdin.read()) if s.get("kCGSSessionUserNameKey") == user]
if not mine:
    print("absent")
elif not any(s.get("kCGSSessionOnConsoleKey") for s in mine):
    print("off-console")
elif any(s.get("CGSSessionScreenIsLocked") for s in mine):
    print("locked")
else:
    print("ready")
' "$user" <<<"$sessions")"

echo "Console session for $user: $state"
/usr/bin/pmset -g | /usr/bin/grep -E '^ *(displaysleep|sleep) ' || true

case "$state" in
  ready)
    echo "macOS E2E guest presents an unlocked desktop."
    ;;
  locked)
    echo "::error title=macOS runner screen is locked::$user's session on this guest is behind the lock screen, so no window is presented and GPUI draws none. Unlock the guest, or have the lab disable its screen lock and screensaver."
    exit 1
    ;;
  *)
    echo "::error title=macOS runner has no desktop::$user's session is $state, so no window is presented. Desktop E2E needs this user logged in on the console (automatic login)."
    exit 1
    ;;
esac
