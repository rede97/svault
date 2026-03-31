#!/usr/bin/env python3
"""Inspect raw IFD structure of a JPEG to diagnose 'Unexpected next IFD'.

Usage:
    tests/.venv/bin/python3 tests/scratch/inspect_exif_ifd.py <file.jpg>
"""
import struct, sys
from pathlib import Path

path = Path(sys.argv[1] if len(sys.argv) > 1 else "tests/fixtures/source/apple_with_exif.jpg")
data = path.read_bytes()

idx = data.find(b'\xff\xe1')
if idx < 0:
    print("No APP1 marker found")
    sys.exit(1)

app1_len = struct.unpack_from('>H', data, idx+2)[0]
print(f"APP1 at offset {idx}, length {app1_len}")

# EXIF data starts after FF E1 <len> Exif\x00\x00
exif_base = idx + 10
tiff = data[exif_base:]

order = tiff[:2]
big = order == b'MM'
fmt = '>' if big else '<'
print(f"Byte order: {'big-endian (MM)' if big else 'little-endian (II)'}")

def u16(off): return struct.unpack_from(f'{fmt}H', tiff, off)[0]
def u32(off): return struct.unpack_from(f'{fmt}I', tiff, off)[0]

ifd0_off = u32(4)
print(f"IFD0 offset: {ifd0_off}")

count = u16(ifd0_off)
print(f"IFD0 entry count: {count}")

for i in range(count):
    entry_off = ifd0_off + 2 + i * 12
    tag   = u16(entry_off)
    typ   = u16(entry_off + 2)
    cnt   = u32(entry_off + 4)
    val   = u32(entry_off + 8)
    print(f"  tag=0x{tag:04X} type={typ} count={cnt} value/offset={val}")

next_ifd = u32(ifd0_off + 2 + count * 12)
print(f"IFD0 next IFD pointer: {next_ifd}")
if next_ifd != 0:
    print("  *** Non-zero next IFD — this is what causes 'Unexpected next IFD' in the exif crate ***")
    print("  Fix: patch the bytes at that offset to 0x00000000")
    patch_offset = exif_base + ifd0_off + 2 + count * 12
    print(f"  Absolute file offset to patch: {patch_offset}")
