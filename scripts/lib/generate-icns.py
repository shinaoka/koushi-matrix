#!/usr/bin/env python3
"""Pack PNG images into an Apple ICNS container.

ICNS files are a sequence of:
    4-byte type code
    4-byte big-endian size (including the 8-byte header)
    payload (a PNG file for every code used here)

Usage: generate-icns.py <out.icns> <code>=<image.png> [<code>=<image.png> ...]

The type codes below are the ones Apple's own `iconutil` emits for PNG payloads.
The legacy `icp4`/`icp5`/`icp6` codes are also valid PNG types, but macOS decodes
them as garbage (observed on macOS 26): every small icon macOS drew (System
Settings > Notifications) came out as colour noise while the payloads themselves
were correct renders (#1942). Apple covers the 16x16 and 32x32 @1x slots with raw
`ic04`/`ic05` bitmaps instead; this packer leaves those slots out and lets macOS
downscale `ic11`, which is what most modern app bundles ship.
"""

import struct
import sys
from pathlib import Path

# Type code -> PNG pixel width. Retina slots are keyed by pixel size, not points.
SLOT_PIXELS = {
    "ic11": 32,    # 16x16 @2x
    "ic12": 64,    # 32x32 @2x
    "ic07": 128,   # 128x128 @1x
    "ic08": 256,   # 256x256 @1x
    "ic13": 256,   # 128x128 @2x
    "ic14": 512,   # 256x256 @2x
    "ic09": 512,   # 512x512 @1x
    "ic10": 1024,  # 512x512 @2x
}


def png_size(path: Path) -> tuple[int, int]:
    """Return the pixel width and height of a PNG file by reading the IHDR chunk."""
    with path.open("rb") as f:
        header = f.read(24)
    if header[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path} is not a PNG file")
    return struct.unpack(">II", header[16:24])


def main() -> int:
    if len(sys.argv) < 3:
        print(
            f"Usage: {sys.argv[0]} <out.icns> <code>=<image.png> [<code>=<image.png> ...]",
            file=sys.stderr,
        )
        return 1

    out_path = Path(sys.argv[1])
    entries = []
    for arg in sys.argv[2:]:
        code, _, raw_path = arg.partition("=")
        if not raw_path:
            print(f"Expected <code>=<image.png>, got {arg}", file=sys.stderr)
            return 1
        if code not in SLOT_PIXELS:
            print(f"Unknown ICNS type code {code!r}", file=sys.stderr)
            return 1
        path = Path(raw_path)
        width, height = png_size(path)
        expected = SLOT_PIXELS[code]
        if (width, height) != (expected, expected):
            print(
                f"{code} needs a {expected}x{expected} image, {path} is {width}x{height}",
                file=sys.stderr,
            )
            return 1
        data = path.read_bytes()
        entries.append(code.encode("ascii") + struct.pack(">I", 8 + len(data)) + data)

    missing = sorted(set(SLOT_PIXELS) - {arg.split("=", 1)[0] for arg in sys.argv[2:]})
    if missing:
        print(f"Missing ICNS slots: {' '.join(missing)}", file=sys.stderr)
        return 1

    body = b"".join(entries)
    # Overall ICNS container: 'icns' + size + body
    total_size = 8 + len(body)
    out_path.write_bytes(b"icns" + struct.pack(">I", total_size) + body)
    print(f"Wrote {len(entries)} images to {out_path} ({total_size} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
