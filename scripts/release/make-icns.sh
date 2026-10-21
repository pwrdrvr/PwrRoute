#!/usr/bin/env bash
# Render packaging/icon/PwrRoute.svg into the committed icon files:
#
#   packaging/macos/PwrRoute.icns    the ten sizes iconutil wants (16–512, @1x and @2x)
#   packaging/windows/PwrRoute.ico   16, 24, 32, 48, 64, 128 and 256
#
# Rasterizes with rsvg-convert when installed (`brew install librsvg`), else
# with macOS's own renderer through render-svg.swift. iconutil is macOS
# only, so the .icns needs a Mac; the .ico needs python3 only. Both outputs
# are committed: CI has neither librsvg nor a reason to rebuild artwork, and
# `cargo xtask bundle` and `vpk pack` read them as files. Rerun this after
# editing the SVG and commit the result.
#
# usage: scripts/release/make-icns.sh
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
svg=$root/packaging/icon/PwrRoute.svg
icns=$root/packaging/macos/PwrRoute.icns
ico=$root/packaging/windows/PwrRoute.ico

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
sizes=(16 24 32 48 64 128 256 512 1024)

if command -v rsvg-convert >/dev/null; then
  for size in "${sizes[@]}"; do
    rsvg-convert --width "$size" --height "$size" "$svg" --output "$work/$size.png"
  done
elif [ "$(uname)" = Darwin ]; then
  swift "$root/scripts/release/render-svg.swift" "$svg" "$work" "${sizes[@]}"
else
  echo "make-icns.sh: needs rsvg-convert (librsvg) or macOS to rasterize the SVG" >&2
  exit 1
fi

iconset=$work/PwrRoute.iconset
mkdir "$iconset"
for base in 16 32 128 256 512; do
  cp "$work/$base.png" "$iconset/icon_${base}x${base}.png"
  cp "$work/$((base * 2)).png" "$iconset/icon_${base}x${base}@2x.png"
done
if command -v iconutil >/dev/null; then
  iconutil --convert icns "$iconset" --output "$icns"
  echo "wrote $icns"
else
  echo "make-icns.sh: iconutil is macOS only; $icns not rebuilt" >&2
fi

# An .ico is a directory of entries followed by the images; since Windows
# Vista each image may be a PNG as is, which is what vpk and the Explorer
# expect for the 256 entry.
python3 -I - "$ico" "$work" 16 24 32 48 64 128 256 <<'PY'
import struct, sys

out, work, sizes = sys.argv[1], sys.argv[2], [int(s) for s in sys.argv[3:]]
images = []
for size in sizes:
    with open(f"{work}/{size}.png", "rb") as f:
        images.append(f.read())
header = struct.pack("<HHH", 0, 1, len(sizes))
offset = len(header) + 16 * len(sizes)
entries = b""
for size, png in zip(sizes, images):
    # Width and height are one byte, where 0 means 256.
    entries += struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(png), offset)
    offset += len(png)
with open(out, "wb") as f:
    f.write(header + entries + b"".join(images))
PY
echo "wrote $ico"
