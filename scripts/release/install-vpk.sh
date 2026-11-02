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

VPK_VERSION=1.3.39-g8d2844f
VPK_SHA512=a4f35a2109eb88564f1bc3a8a0ddc385ef18ccf6c760959384084bcb10529bc647041f1051f9422fd143705670e24cfbfbdf65230250d13fd8334d663218dccc

temp_root=${VPK_TEMP_ROOT:-${RUNNER_TEMP:?}}
feed="$temp_root/vpk-feed"
tools="$temp_root/vpk"
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
