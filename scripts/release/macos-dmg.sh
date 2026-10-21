#!/usr/bin/env bash
# Put PwrRoute.app in a drag-to-install DMG laid out like the Pwr family's:
# a 660x400 window over packaging/dmg/background.png, the app at (170,230),
# an Applications link at (500,230), 112 pt icons, the app icon as the volume
# icon, and a volume named "PwrRoute <version>-<arch>". With an identity, also
# sign the DMG, notarize it through the notarytool keychain profile
# NOTARY_PROFILE in KEYCHAIN, and staple the ticket.
#
# release.yml's macOS sign job runs this from the signing input, where there
# is no checkout and no cargo, after `vpk pack` has signed and notarized the
# app. `cargo xtask bundle` builds the same layout for local and preview
# builds.
#
# The layout lives in the image's .DS_Store, which only Finder writes here, so
# this drives Finder through osascript: it needs a logged-in session (a
# GitHub macOS runner has one) and Automation access to Finder.
#
# usage: macos-dmg.sh APP DMG [IDENTITY]
#
#   DMG_VERSION      version in the volume name; defaults to the app's
#                    CFBundleShortVersionString
#   DMG_BACKGROUND   background PNG; defaults to background.png beside this
#                    script, then packaging/dmg/background.png in a checkout
#   DMG_VOLUME_ICON  volume icon; defaults to PwrRoute.icns beside this
#                    script, then packaging/macos/PwrRoute.icns in a checkout
set -euo pipefail

if [ $# -lt 2 ] || [ $# -gt 3 ]; then
  echo "usage: macos-dmg.sh APP DMG [IDENTITY]" >&2
  exit 2
fi
app=$1
dmg=$2
identity=${3:-}

# Window and icon layout: keep in sync with generate-dmg-background.swift,
# which paints the arrow between the two icon positions (xtask's tests
# compare the two).
window_width=660
window_height=400
icon_size=112
text_size=12
app_x=170
applications_x=500
icon_y=230

here=$(cd "$(dirname "$0")" && pwd)

# $1 if set, else the first of the other arguments that is a file.
asset() {
  local name=$1 explicit=$2 candidate
  shift 2
  if [ -n "$explicit" ]; then
    [ -f "$explicit" ] && { printf '%s\n' "$explicit"; return 0; }
    echo "macos-dmg.sh: $explicit does not exist" >&2
    return 1
  fi
  for candidate in "$@"; do
    [ -f "$candidate" ] && { printf '%s\n' "$candidate"; return 0; }
  done
  echo "macos-dmg.sh: no $name beside the script or in the checkout's packaging/" >&2
  return 1
}
background=$(asset background.png "${DMG_BACKGROUND:-}" \
  "$here/background.png" "$here/../../packaging/dmg/background.png")
volume_icon=$(asset PwrRoute.icns "${DMG_VOLUME_ICON:-}" \
  "$here/PwrRoute.icns" "$here/../../packaging/macos/PwrRoute.icns")

plist=$app/Contents/Info.plist
version=${DMG_VERSION:-$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$plist")}
executable=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$plist")
read -ra archs <<<"$(lipo -archs "$app/Contents/MacOS/$executable")"
if [[ " ${archs[*]} " == *" arm64 "* && " ${archs[*]} " == *" x86_64 "* ]]; then
  arch=universal
else
  arch=$(IFS=-; echo "${archs[*]}")
fi
volname="PwrRoute $version-$arch"

work=$(mktemp -d)
device=
cleanup() {
  if [ -n "$device" ]; then
    hdiutil detach "$device" -quiet || hdiutil detach "$device" -force -quiet || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT

# A read-write image sized to the app plus headroom for the background,
# the volume icon, .DS_Store and HFS+ overhead.
app_mb=$(du -sm "$app" | awk '{print $1}')
size_mb=$((app_mb + app_mb / 5 + 20))
rw=$work/PwrRoute-rw.dmg
# -type UDIF makes a read-write image (UDRW); -format applies only to
# -srcfolder images.
hdiutil create -size "${size_mb}m" -fs HFS+ -type UDIF -volname "$volname" "$rw"

# Not -nobrowse: Finder has to see the volume to lay out its window. A
# mounted volume of the same name gets " 1" appended to the new one's mount
# point, so take the device and mount point from hdiutil's output.
attach=$(hdiutil attach -readwrite -noverify -noautoopen "$rw")
device=$(awk -F'\t' '/Apple_HFS/ {sub(/[[:space:]]+$/, "", $1); print $1; exit}' <<<"$attach")
mount=$(awk -F'\t' '/Apple_HFS/ {print $NF; exit}' <<<"$attach")
if [ -z "$device" ] || [ ! -d "$mount" ]; then
  echo "macos-dmg.sh: cannot find the attached volume in hdiutil's output:" >&2
  echo "$attach" >&2
  exit 1
fi

ditto "$app" "$mount/PwrRoute.app"
ln -s /Applications "$mount/Applications"
mkdir "$mount/.background"
cp "$background" "$mount/.background/background.png"

# Finder sometimes needs a moment to see a freshly attached volume, and an
# AppleEvent can time out on a busy machine, so try a few times.
layout() {
  osascript - "$mount" <<EOF
on run argv
  set mountPath to item 1 of argv
  tell application "Finder"
    set theDisk to (POSIX file mountPath as alias)
    open theDisk
    set theWindow to container window of theDisk
    set current view of theWindow to icon view
    set toolbar visible of theWindow to false
    set statusbar visible of theWindow to false
    set pathbar visible of theWindow to false
    -- bounds are the whole window, title bar included: add its height so
    -- the window's content area is the background's size.
    set the bounds of theWindow to {200, 120, 200 + $window_width, 120 + $window_height + 28}
    set theOptions to the icon view options of theWindow
    set arrangement of theOptions to not arranged
    set icon size of theOptions to $icon_size
    set text size of theOptions to $text_size
    set label position of theOptions to bottom
    set shows item info of theOptions to false
    set shows icon preview of theOptions to false
    set background picture of theOptions to file ".background:background.png" of theDisk
    set position of item "PwrRoute.app" of theWindow to {$app_x, $icon_y}
    set position of item "Applications" of theWindow to {$applications_x, $icon_y}
    update theDisk without registering applications
    delay 1
    close theWindow
  end tell
end run
EOF
}

attempt=1
until layout; do
  if [ "$attempt" -ge 5 ]; then
    echo "macos-dmg.sh: Finder could not lay out the DMG window after $attempt attempts." >&2
    echo "It needs a logged-in GUI session and Automation access to Finder for this" >&2
    echo "process (System Settings > Privacy & Security > Automation)." >&2
    exit 1
  fi
  echo "macos-dmg.sh: Finder layout attempt $attempt failed; retrying" >&2
  attempt=$((attempt + 1))
  sleep 2
done

# Finder writes .DS_Store when the window closes, sometimes a moment later.
for _ in $(seq 1 20); do
  [ -f "$mount/.DS_Store" ] && break
  sleep 0.5
done
if [ ! -f "$mount/.DS_Store" ]; then
  echo "macos-dmg.sh: Finder did not write .DS_Store; the DMG window would have no layout" >&2
  exit 1
fi

# The volume icon goes in after the layout: on macOS 26, Finder deleted a
# .VolumeIcon.icns copied in before it and cleared the volume's custom-icon
# flag. That flag (kHasCustomIcon) on the volume root is what makes Finder
# use the file. SetFile comes with the Xcode command line tools; without it,
# set the same bit (0x0400 in the flags at byte 8) in com.apple.FinderInfo.
cp "$volume_icon" "$mount/.VolumeIcon.icns"
if command -v SetFile >/dev/null; then
  SetFile -a C "$mount"
else
  xattr -wx com.apple.FinderInfo \
    "0000000000000000040000000000000000000000000000000000000000000000" "$mount"
fi

rm -rf "$mount/.fseventsd" "$mount/.Trashes"
sync
hdiutil detach "$device" -quiet || hdiutil detach "$device" -force -quiet
device=

rm -f "$dmg"
hdiutil convert "$rw" -format UDZO -imagekey zlib-level=9 -o "$dmg"

if [ -n "$identity" ]; then
  codesign --force --timestamp --keychain "${KEYCHAIN:?}" --sign "$identity" "$dmg"
  xcrun notarytool submit "$dmg" --keychain-profile "${NOTARY_PROFILE:?}" --keychain "$KEYCHAIN" --wait
  xcrun stapler staple "$dmg"
fi
