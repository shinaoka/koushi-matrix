#!/usr/bin/env python3
"""Pack PNG images into a minimal Apple ICNS container.

ICNS files are a sequence of:
    4-byte type (e.g. icp4, icp5, ic11, ic10)
    4-byte big-endian size (including header)
    PNG data

Only the sizes required for a modern desktop app icon are supported here.
"""

import struct
import sys
from pathlib import Path

# Map PNG width/height to ICNS type codes.
# See Apple icon family type codes; values are PNG pixels, not display points.
# Retina slots map the @2x pixel size to the corresponding type code.
TYPE_FOR_SIZE = {
    16: b"icp4",    # 16x16 @1x
    32: b"icp5",    # 32x32 @1x (also 16x16 @2x)
    64: b"icp6",    # 64x64 @1x (also 32x32 @2x)
    128: b"ic07",   # 128x128 @1x
    256: b"ic13",   # 128x128 @2x
    512: b"ic09",   # 512x512 @1x (also 256x256 @2x)
    1024: b"ic10",  # 512x512 @2x
}


def png_size(path: Path) -> int:
    """Return the width of a PNG file by reading the IHDR chunk."""
    with path.open("rb") as f:
        header = f.read(24)
    if header[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path} is not a PNG file")
    width, _ = struct.unpack(">II", header[16:24])
    return width


def main() -> int:
    if len(sys.argv) < 3:
        print(f"Usage: {sys.argv[0]} <out.icns> <image1.png> [image2.png ...]", file=sys.stderr)
        return 1

    out_path = Path(sys.argv[1])
    entries = []
    for arg in sys.argv[2:]:
        path = Path(arg)
        size = png_size(path)
        if size not in TYPE_FOR_SIZE:
            print(f"Skipping unsupported size {size}px: {path}", file=sys.stderr)
            continue
        data = path.read_bytes()
        icns_type = TYPE_FOR_SIZE[size]
        entry = icns_type + struct.pack(">I", 8 + len(data)) + data
        entries.append(entry)

    if not entries:
        print("No usable PNG images provided.", file=sys.stderr)
        return 1

    body = b"".join(entries)
    # Overall ICNS container: 'icns' + size + body
    total_size = 8 + len(body)
    container = b"icns" + struct.pack(">I", total_size) + body
    out_path.write_bytes(container)
    print(f"Wrote {len(entries)} images to {out_path} ({total_size} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
