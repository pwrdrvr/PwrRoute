#!/usr/bin/env python3
"""Write the .DS_Store that gives a mounted DMG its installer window.

    dmg-layout.py <mounted-path> <volume-name>

Finder reads a volume's window geometry, icon-view options, background picture
and per-item icon positions out of .DS_Store. Nothing in the base system
writes one: `hdiutil` only makes the filesystem, and the usual way to get the
rest is to drive Finder over AppleScript.

This writes the file directly instead, for two reasons.

Finder's .DS_Store embeds the ABSOLUTE PATH OF THE DISK IMAGE on the machine
that styled it, inside the background picture's alias and in a companion
bookmark record. A DMG styled that way ships the build host's home directory
and staging paths to everyone who downloads it, and a captured copy could not
be committed here at all. The alias this module writes names only the volume
and the path inside it.

The second reason is that driving Finder needs an Apple Events grant for
whatever process runs the build. Packaging must work unattended on the
release runner, which has no one to answer a TCC prompt.

electron-builder reaches the same file through a downloaded `dmgbuild` binary,
which also writes a volume-relative alias. This is that step, for a repository
that does not download build tools. Format references: the Buddy allocator and
B-tree layout of .DS_Store, and Inside Macintosh's AliasRecord version 2.
"""

# Adapted from PwrSuiteLab lab-control/scripts/dmg-layout.py.

import os
import plistlib
import struct
import sys

# Window and icon layout — the suite's installer geometry, shared with
# generate-dmg-background.swift, which draws the arrow between these exact
# icon centres. Coordinates are the icon CENTRE, in points, from the window's
# top-left corner. Change one file, change the other.
WINDOW_SIZE = (660, 400)
WINDOW_ORIGIN = (200, 120)
ICON_SIZE = 112
TEXT_SIZE = 12
BACKGROUND = ".background/background.png"
# Two icons, as in the three reference installers. Anything else in the window
# crowds the drag-to-Applications line the artwork draws.
ITEM_POSITIONS = {
    "PwrRoute.app": (170, 230),
    "Applications": (500, 230),
}

# Unix epoch to the classic Mac epoch (1904-01-01), for alias date fields.
HFS_EPOCH_DELTA = 2082844800


def hfs_time(seconds):
    return max(0, int(seconds) + HFS_EPOCH_DELTA) & 0xFFFFFFFF


def pascal(text, size):
    """A Pascal string padded to a fixed field width, as AliasRecord uses.

    Over-long text is trimmed a CHARACTER at a time: slicing the encoded bytes
    would cut through a multi-byte sequence and leave invalid UTF-8 in the
    record.
    """
    raw = text.encode("utf-8")
    while len(raw) > size - 1:
        text = text[:-1]
        raw = text.encode("utf-8")
    return bytes([len(raw)]) + raw + b"\0" * (size - 1 - len(raw))


def utf16_field(text):
    """An alias `extra` payload: character count then UTF-16BE, no BOM."""
    encoded = text.encode("utf-16-be")
    return struct.pack(">H", len(encoded) // 2) + encoded


def background_alias(volume, volume_name, relative_path):
    """An AliasRecord version 2 for a file on the mounted volume.

    `volume` is where the image is mounted on THIS machine, and is used only to
    stat the real files. Every string the record carries is built from
    `volume_name` and `relative_path` instead, so the alias describes the
    volume as the person who opens the DMG will see it — mounted at
    /Volumes/<name> — and says nothing about the build host, wherever it
    happened to mount the scratch image. Finder resolves the record by
    catalogue node id first and falls back to those path `extra` entries.
    """
    mount_point = f"/Volumes/{volume_name}"
    parts = relative_path.split("/")
    parent_relative, filename = "/".join(parts[:-1]), parts[-1]
    target = os.path.join(volume, relative_path)
    parent = os.path.dirname(target)

    target_stat, parent_stat, volume_stat = os.stat(target), os.stat(parent), os.stat(volume)
    # Alias records carry 32-bit HFS+ catalogue node ids. macos-dmg.sh builds the
    # scratch image with `-fs HFS+` so these always fit; say so plainly rather
    # than letting struct.pack raise "argument out of range".
    for label, node in (("target", target_stat.st_ino), ("parent", parent_stat.st_ino)):
        if node > 0xFFFFFFFF:
            raise SystemExit(
                f"dmg-layout: the {label} inode {node} does not fit a 32-bit alias field — "
                "the volume must be HFS+"
            )

    record = b"".join([
        struct.pack(">I", 0),                       # application-specific user type
        struct.pack(">H", 0),                       # record size, patched below
        struct.pack(">HH", 2, 0),                   # version 2, kind 0 (file)
        pascal(volume_name, 28),
        struct.pack(">I", hfs_time(volume_stat.st_birthtime)),
        b"H+",                                      # volume signature (HFS+)
        struct.pack(">H", 4),                       # volume type (disk image)
        struct.pack(">I", parent_stat.st_ino),
        pascal(filename, 64),
        struct.pack(">I", target_stat.st_ino),
        struct.pack(">I", hfs_time(target_stat.st_birthtime)),
        b"\0\0\0\0",                                # file type
        b"\0\0\0\0",                                # file creator
        struct.pack(">HH", 0xFFFF, 0xFFFF),         # levels from / to: unknown
        struct.pack(">I", 0),                       # volume attributes
        struct.pack(">H", 0),                       # volume filesystem id
        b"\0" * 10,                                 # reserved
    ])
    assert len(record) == 150, len(record)

    extras = [
        (0, os.path.basename(parent_relative).encode("utf-8")),  # parent folder name
        (1, struct.pack(">I", parent_stat.st_ino)),             # parent folder cnid
        # Carbon path: colon-separated, and the parent segment drops out when
        # the target sits at the volume root.
        (2, ":".join(filter(None, [volume_name, parent_relative, filename])).encode("utf-8")),
        (14, utf16_field(filename)),                            # unicode filename
        (15, utf16_field(volume_name)),                         # unicode volume name
        (18, f"/{relative_path}".encode("utf-8")),              # path within the volume
        (19, mount_point.encode("utf-8")),                      # volume mount point
    ]
    for kind, payload in extras:
        record += struct.pack(">hH", kind, len(payload)) + payload
        if len(payload) % 2:
            record += b"\0"
    record += struct.pack(">hH", -1, 0)                         # end of extras

    return record[:4] + struct.pack(">H", len(record)) + record[6:]


def window_settings():
    (x, y), (width, height) = WINDOW_ORIGIN, WINDOW_SIZE
    return {
        "ContainerShowSidebar": False,
        "ShowSidebar": False,
        "ShowStatusBar": False,
        "ShowTabView": False,
        "ShowToolbar": False,
        # The path bar is a per-viewer Finder preference, and it eats 26pt off
        # the bottom of the window — enough to hide the background's own
        # "Drag to Applications" line. Pin it off so every viewer sees the
        # same artwork. Finder omits this key when the preference is already
        # off, which is why a captured .DS_Store does not carry it.
        "ShowPathbar": False,
        "WindowBounds": "{{%d, %d}, {%d, %d}}" % (x, y, width, height),
    }


def icon_view_settings(alias):
    return {
        "arrangeBy": "none",
        # backgroundType 2 is "picture"; the colour keys are the white Finder
        # falls back to and are written even when a picture is set.
        "backgroundType": 2,
        "backgroundColorRed": 1.0,
        "backgroundColorGreen": 1.0,
        "backgroundColorBlue": 1.0,
        "backgroundImageAlias": alias,
        "gridOffsetX": 0.0,
        "gridOffsetY": 0.0,
        "gridSpacing": 100.0,
        "iconSize": float(ICON_SIZE),
        "labelOnBottom": True,
        "showIconPreview": False,
        "showItemInfo": False,
        "textSize": float(TEXT_SIZE),
        "viewOptionsVersion": 1,
    }


def icon_location(x, y):
    """An `Iloc` blob: the icon centre, then Finder's unset-index padding."""
    return struct.pack(">II", x, y) + b"\xff\xff\xff\xff\xff\xff\x00\x00"


def encode_records(records):
    """The B-tree leaf node holding every record, sorted the way Finder sorts.

    Keys order by filename case-insensitively, then by the four-character
    structure id. A reader that binary-searches a mis-sorted node silently
    misses records, so this is not cosmetic.
    """
    node = struct.pack(">II", 0, len(records))  # 0 = leaf node
    for name, struct_id, value in sorted(records, key=lambda r: (r[0].lower(), r[1])):
        encoded = name.encode("utf-16-be")
        node += struct.pack(">I", len(encoded) // 2) + encoded
        node += struct_id.encode("ascii")
        if isinstance(value, bytes):
            node += b"blob" + struct.pack(">I", len(value)) + value
        elif isinstance(value, int):
            node += b"long" + struct.pack(">I", value)
        elif isinstance(value, str):
            code = value.encode("ascii")
            if len(code) != 4:
                raise ValueError(f"{name}/{struct_id}: type values must be four ASCII bytes")
            node += b"type" + code
        else:
            raise TypeError(f"unsupported value for {name}/{struct_id}: {type(value)}")
    return node


def encode_ds_store(node, record_count):
    """Wrap the leaf node in the Buddy allocator file .DS_Store actually is.

    Three blocks, at the fixed addresses Finder itself uses, so the result is
    byte-for-byte conventional: block 0 is the allocator's own root block,
    block 1 the B-tree header, block 2 the single leaf page. Allocator
    addresses are relative to file offset 4, and an address's low five bits
    carry log2 of the block size rather than part of the offset.
    """
    page = 0x1000
    if len(node) > page:
        raise SystemExit(f"dmg-layout: the leaf node is {len(node)} bytes, over the {page}-byte page")

    blocks = [(0x2000, 11), (0x40, 5), (0x1000, 12)]  # (address, log2 size)
    root = struct.pack(">II", len(blocks), 0)
    root += b"".join(struct.pack(">I", address | shift) for address, shift in blocks)
    root += b"\0" * (4 * (256 - len(blocks)))         # the table is padded to 256 slots
    root += struct.pack(">I", 1) + bytes([4]) + b"DSDB" + struct.pack(">I", 1)
    # Buddy free lists, one per power of two. Everything the three blocks do
    # not occupy below 2^31 is free, which is the state Finder leaves behind.
    free = {5: [0x20, 0x60], 7: [0x80], 8: [0x100], 9: [0x200], 10: [0x400],
            11: [0x800, 0x2800], 12: [0x3000]}
    free.update({width: [1 << width] for width in range(14, 31)})
    for width in range(32):
        addresses = free.get(width, [])
        root += struct.pack(">I", len(addresses))
        root += b"".join(struct.pack(">I", address) for address in addresses)

    root_size = 0x800
    if len(root) > root_size:
        raise SystemExit(f"dmg-layout: the allocator root is {len(root)} bytes, over {root_size}")
    root = root.ljust(root_size, b"\0")

    header = struct.pack(">I4sIII", 1, b"Bud1", 0x2000, root_size, 0x2000) + bytes.fromhex(
        "0000100c" + "00" * 12
    )
    image = bytearray(header.ljust(4 + 0x2000 + root_size, b"\0"))
    # DSDB header: root node block, tree levels, total records, node count, page size.
    image[4 + 0x40 : 4 + 0x40 + 20] = struct.pack(">IIIII", 2, 0, record_count, 1, page)
    image[4 + 0x1000 : 4 + 0x1000 + len(node)] = node
    image[4 + 0x2000 : 4 + 0x2000 + root_size] = root
    return bytes(image)


def main():
    if len(sys.argv) not in (2, 3):
        raise SystemExit("usage: dmg-layout.py <mounted-path> [volume-name]")
    volume = sys.argv[1]
    # The volume name the DMG will carry, which need not be the directory the
    # image is mounted on while it is being built.
    volume_name = sys.argv[2] if len(sys.argv) == 3 else os.path.basename(volume.rstrip("/"))
    if not os.path.isdir(volume):
        raise SystemExit(f"dmg-layout: {volume} is not a mounted volume")
    for name in ITEM_POSITIONS:
        if not os.path.lexists(os.path.join(volume, name)):
            raise SystemExit(f"dmg-layout: {volume} has no item named {name!r}")

    alias = background_alias(volume, volume_name, BACKGROUND)
    records = [(".", "bwsp", plistlib.dumps(window_settings(), fmt=plistlib.FMT_BINARY)),
               (".", "icvp", plistlib.dumps(icon_view_settings(alias), fmt=plistlib.FMT_BINARY)),
               (".", "vSrn", 1),
               (".", "vstl", "icnv")]
    records += [(name, "Iloc", icon_location(*xy)) for name, xy in ITEM_POSITIONS.items()]

    destination = os.path.join(volume, ".DS_Store")
    with open(destination, "wb") as handle:
        handle.write(encode_ds_store(encode_records(records), len(records)))
    print(f"  wrote {destination} ({len(alias)}-byte background alias, {len(records)} records)")


if __name__ == "__main__":
    main()
