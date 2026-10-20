#!/usr/bin/env bash
# Install Velopack's vpk CLI from a SHA-512-verified package, and put it on
# PATH for the steps after this one.
#
# The release sign jobs run vpk next to the signing credentials, so it is
# installed only from the bytes pinned here, never from whatever the feed
# serves. VPK_VERSION must equal the velopack crate pin in
# crates/pwrroute-app/Cargo.toml; upgrade both together and take the new
# hash from NuGet's catalog (packageHash, base64) or `shasum -a 512`, then
# rerun velopack-notices.sh for the new binaries' license notices.
#
# Needs .NET 8 (actions/setup-dotnet), curl, and GITHUB_PATH / RUNNER_TEMP
# from Actions. Runs under bash on macOS and on Windows (Git Bash).
#
# usage: install-vpk.sh
set -euo pipefail

VPK_VERSION=1.2.161
VPK_SHA512=359f48f056f0973e1b1cf971bd67862b1dd35c21147f801d3c78541e2769a6033708a56dd5b1801c7add0630a36344b5ba892997ad2f03cf3dc00ccedbf8b19b

feed="${RUNNER_TEMP:?}/vpk-feed"
tools="$RUNNER_TEMP/vpk"
mkdir -p "$feed"
package="$feed/vpk.$VPK_VERSION.nupkg"
curl -fsSL --retry 3 -o "$package" \
  "https://api.nuget.org/v3-flatcontainer/vpk/$VPK_VERSION/vpk.$VPK_VERSION.nupkg"

# From stdin: given a Windows path, GNU sha512sum escapes its backslashes and
# prefixes the hash with "\".
if command -v sha512sum >/dev/null; then
  actual=$(sha512sum < "$package" | awk '{print $1}')
else
  actual=$(shasum -a 512 < "$package" | awk '{print $1}')
fi
if [ "$actual" != "$VPK_SHA512" ]; then
  echo "::error::vpk $VPK_VERSION package SHA-512 mismatch: expected $VPK_SHA512, got $actual" >&2
  exit 1
fi

# A NuGet config whose only source is the verified package.
cat > "$feed/nuget.config" <<CONFIG
<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <clear />
    <add key="verified" value="$feed" />
  </packageSources>
</configuration>
CONFIG

dotnet tool install vpk --version "$VPK_VERSION" --tool-path "$tools" --configfile "$feed/nuget.config"

if command -v cygpath >/dev/null; then
  tools=$(cygpath -w "$tools")
fi
echo "$tools" >> "${GITHUB_PATH:?}"
