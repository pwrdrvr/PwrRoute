#!/usr/bin/env bash
# Render packaging/icon/PwrRoute.svg into the committed icon files:
#
#   packaging/icon/PwrRoute.icon/    the Icon Composer package `cargo xtask
#                                    bundle` compiles with actool (macOS 26):
#     icon.json                      the SVG's ground gradient as the fill
#     Assets/glyph.png               the route mark alone, 1024px, transparent
#   packaging/macos/PwrRoute.icns    the ten sizes iconutil wants (16–512, @1x
#                                    and @2x): bundle's fallback without Xcode 26
#   packaging/windows/PwrRoute.ico   16, 24, 32, 48, 64, 128 and 256
#
# The glyph is the SVG minus its ground, with the lower branch at 55% opacity
# instead of baked into a darker orange: macOS draws the ground from icon.json
# and wants the mark on transparency. Rasterizes with rsvg-convert when
# installed (`brew install librsvg`), else with macOS's own renderer through
# render-svg.swift. iconutil is macOS only, so the .icns needs a Mac; the .ico
# and the package need python3 only. Every output is committed: CI has
# neither librsvg nor a reason to rebuild artwork, and `cargo xtask bundle`
# and `vpk pack` read them as files. Rerun this after editing the SVG and
# commit the result.
#
# usage: scripts/release/make-icns.sh
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
svg=$root/packaging/icon/PwrRoute.svg
icns=$root/packaging/macos/PwrRoute.icns
ico=$root/packaging/windows/PwrRoute.ico
package=$root/packaging/icon/PwrRoute.icon

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
sizes=(16 24 32 48 64 128 256 512 1024)

# The glyph-only SVG and icon.json both come from the one SVG, so the mark and
# the ground cannot drift apart. Each edit must match exactly once: a
# reshaped SVG fails here rather than leaving a ground in the glyph.
mkdir -p "$work/glyph" "$package/Assets"
python3 -I - "$svg" "$work/glyph.svg" "$package/icon.json" <<'PY'
import re, sys

svg, glyph_out, json_out = sys.argv[1:4]
src = open(svg, encoding="utf-8").read()
glyph, grounds = re.subn(r'\s*<rect[^>]*fill="url\(#ground\)"[^>]*/>', "", src)
glyph, branches = re.subn(r'stroke="#884726"', 'stroke="#e8743a" opacity="0.55"', glyph)
if (grounds, branches) != (1, 1):
    sys.exit(f"make-icns.sh: expected one ground rect and one #884726 branch, found {grounds} and {branches}")
open(glyph_out, "w", encoding="utf-8").write(glyph)

ground = re.search(r'<linearGradient id="ground".*?</linearGradient>', src, re.S)
stops = re.findall(r'stop-color="#([0-9a-fA-F]{6})"', ground.group(0)) if ground else []
if len(stops) != 2:
    sys.exit(f"make-icns.sh: expected two stops in the ground gradient, found {len(stops)}")

def srgb(hex_color):
    r, g, b = (int(hex_color[i:i + 2], 16) / 255 for i in (0, 2, 4))
    return f"srgb:{r:.5f},{g:.5f},{b:.5f},1.00000"

# Icon Composer's schema, as PwrSnap's generate-app-icon.swift writes it: the
# ground is the package fill, so macOS 26 shapes it and adds the glass edge;
# the one layer is the mark, `fill: automatic` so Dark, Clear and Tinted can
# recolor it, and `glass: false` so it stays a flat stroke.
top, bottom = (srgb(s) for s in stops)
open(json_out, "w", encoding="utf-8").write(f"""{{
  "fill" : {{
    "linear-gradient" : [
      "{top}",
      "{bottom}"
    ]
  }},
  "groups" : [
    {{
      "name" : "Mark",
      "layers" : [
        {{
          "name" : "glyph",
          "image-name" : "glyph.png",
          "fill" : "automatic",
          "glass" : false,
          "hidden" : false,
          "blend-mode" : "normal"
        }}
      ],
      "lighting" : "individual",
      "shadow" : {{
        "kind" : "neutral",
        "opacity" : 0.5
      }},
      "translucency" : {{
        "enabled" : false,
        "value" : 0.5
      }}
    }}
  ],
  "supported-platforms" : {{
    "circles" : [
      "watchOS"
    ],
    "squares" : "shared"
  }}
}}
""")
PY

if command -v rsvg-convert >/dev/null; then
  for size in "${sizes[@]}"; do
    rsvg-convert --width "$size" --height "$size" "$svg" --output "$work/$size.png"
  done
  rsvg-convert --width 1024 --height 1024 "$work/glyph.svg" --output "$work/glyph/1024.png"
elif [ "$(uname)" = Darwin ]; then
  swift "$root/scripts/release/render-svg.swift" "$svg" "$work" "${sizes[@]}"
  swift "$root/scripts/release/render-svg.swift" "$work/glyph.svg" "$work/glyph" 1024
else
  echo "make-icns.sh: needs rsvg-convert (librsvg) or macOS to rasterize the SVG" >&2
  exit 1
fi
cp "$work/glyph/1024.png" "$package/Assets/glyph.png"
echo "wrote $package"

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
