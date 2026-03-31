#!/usr/bin/env python3
"""Patch the ExifIFD next-IFD pointer to 0 so the `exif` Rust crate accepts the JPEG.

piexif writes a garbage non-zero next-IFD pointer in the ExifIFD chain,
causing the Rust `exif` crate to bail with 'Unexpected next IFD'.
This script zeroes that pointer in-place.

Usage:
    tests/.venv/bin/python3 tests/scratch/fix_exif_ifd.py <file.jpg>
    # or call patch_jpeg_exif_ifd(path) from gen_fixtures.py
"""
import struct
import sys
from pathlib import Path


def patch_jpeg_exif_ifd(path: Path) -> None:
    """Zero out any non-zero next-IFD pointers in IFD0 and ExifSubIFD."""
    data = bytearray(path.read_bytes())

    # Find APP1
    idx = bytes(data).find(b'\xff\xe1')
    if idx < 0:
        return  # no APP1, nothing to do

    exif_base = idx + 10  # skip FFE1 + len(2) + 'Exif\x00\x00'(6)
    tiff = data  # we'll index absolute positions

    order = bytes(data[exif_base:exif_base+2])
    big = order == b'MM'
    fmt = '>' if big else '<'

    def u16(off): return struct.unpack_from(f'{fmt}H', data, exif_base + off)[0]
    def u32(off): return struct.unpack_from(f'{fmt}I', data, exif_base + off)[0]
    def w32(off, val): struct.pack_into(f'{fmt}I', data, exif_base + off, val)

    ifd0_off = u32(4)
    count0 = u16(ifd0_off)

    # Patch IFD0 next pointer
    next0_abs = ifd0_off + 2 + count0 * 12
    next0 = u32(next0_abs)
    if next0 != 0:
        print(f"  Patching IFD0 next pointer {next0} → 0")
        w32(next0_abs, 0)

    # Find ExifIFD (tag 0x8769) and patch its next pointer too
    for i in range(count0):
        entry_off = ifd0_off + 2 + i * 12
        tag = u16(entry_off)
        if tag == 0x8769:
            subifd_off = u32(entry_off + 8)
            sub_count = u16(subifd_off)
            next_sub_abs = subifd_off + 2 + sub_count * 12
            next_sub = u32(next_sub_abs)
            if next_sub != 0:
                print(f"  Patching ExifIFD next pointer {next_sub} → 0")
                w32(next_sub_abs, 0)
            break

    path.write_bytes(bytes(data))


if __name__ == "__main__":
    for arg in sys.argv[1:]:
        p = Path(arg)
        print(f"Patching {p}")
        patch_jpeg_exif_ifd(p)
        print("  Done")
