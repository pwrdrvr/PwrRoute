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
# dmg-layout.py writes the image's .DS_Store directly, using PwrSuiteLab's
# unattended packaging approach. No Finder AppleEvents or Automation grant
# is needed, and background aliases contain no build-host paths.
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
trap 'exit 130' INT
trap 'exit 143' TERM

# A read-write image sized to the app plus headroom for the background,
# the volume icon, .DS_Store and HFS+ overhead.
app_mb=$(du -sm "$app" | awk '{print $1}')
size_mb=$((app_mb + app_mb / 5 + 20))
rw=$work/PwrRoute-rw.dmg
# -type UDIF makes a read-write image (UDRW); -format applies only to
# -srcfolder images.
hdiutil create -size "${size_mb}m" -fs HFS+ -type UDIF -volname "$volname" "$rw"

# Mount privately: concurrent packages with the same volume name cannot collide.
mkdir "$work/mount"
attach=$(hdiutil attach -readwrite -noverify -noautoopen -nobrowse -mountpoint "$work/mount" "$rw")
device=$(awk -F'\t' '/Apple_HFS/ {sub(/[[:space:]]+$/, "", $1); print $1; exit}' <<<"$attach")
mount=$work/mount
if [ -z "$device" ] || [ ! -d "$mount" ]; then
  echo "macos-dmg.sh: cannot find the attached volume in hdiutil's output:" >&2
  echo "$attach" >&2
  exit 1
fi

ditto "$app" "$mount/PwrRoute.app"
ln -s /Applications "$mount/Applications"
mkdir "$mount/.background"
cp "$background" "$mount/.background/background.png"

# Preserve the complete installer layout without a GUI or TCC prompt.
python3 "$here/dmg-layout.py" "$mount" "$volname"

# Mark the volume's custom icon after writing its layout.
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
