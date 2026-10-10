#!/usr/bin/env python3
"""Pack PNGs into a Windows .ico (PNG-compressed entries, Vista and later).

    python3 assets/make-ico.py out.ico 16.png 24.png 32.png ... 256.png
"""
import struct, sys

out, files = sys.argv[1], sys.argv[2:]
blobs = []
for f in files:
    data = open(f, "rb").read()
    w, h = struct.unpack(">II", data[16:24])  # from the PNG's IHDR
    blobs.append((w, h, data))
header = struct.pack("<HHH", 0, 1, len(blobs))
offset = 6 + 16 * len(blobs)
entries = b""
for w, h, data in blobs:
    entries += struct.pack("<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(data), offset)
    offset += len(data)
with open(out, "wb") as fh:
    fh.write(header + entries + b"".join(d for _, _, d in blobs))
print(f"wrote {out}: {', '.join(f'{w}px' for w, _, _ in blobs)}")
