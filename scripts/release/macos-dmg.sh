#!/usr/bin/env bash
# Put PwrRoute.app in a drag-to-install DMG. With an identity, also sign the
# DMG, notarize it through the notarytool keychain profile NOTARY_PROFILE in
# KEYCHAIN, and staple the ticket.
#
# release.yml's macOS sign job runs this from the signing input, where there
# is no checkout and no cargo, after `vpk pack` has signed and notarized the
# app. `cargo xtask bundle` builds the same layout for local and preview
# builds.
#
# usage: macos-dmg.sh APP DMG [IDENTITY]
set -euo pipefail

app=$1
dmg=$2
identity=${3:-}

staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
ditto "$app" "$staging/PwrRoute.app"
ln -s /Applications "$staging/Applications"
rm -f "$dmg"
hdiutil create -volname PwrRoute -fs HFS+ -format UDZO -srcfolder "$staging" "$dmg"

if [ -n "$identity" ]; then
  codesign --force --timestamp --keychain "${KEYCHAIN:?}" --sign "$identity" "$dmg"
  xcrun notarytool submit "$dmg" --keychain-profile "${NOTARY_PROFILE:?}" --keychain "$KEYCHAIN" --wait
  xcrun stapler staple "$dmg"
fi
