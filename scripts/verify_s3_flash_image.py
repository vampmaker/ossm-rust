#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Fail if an ESP32-S3 merged image has multiple DROM segments (bootloader error)."""

from __future__ import annotations

import struct
import sys
from pathlib import Path


def drom_segments(bin_path: Path, app_offset: int = 0x10000) -> list[tuple[int, int]]:
    data = bin_path.read_bytes()
    if data[app_offset] != 0xE9:
        raise SystemExit(f"{bin_path}: no ESP image magic at offset 0x{app_offset:x}")

    seg_count = data[app_offset + 1]
    pos = app_offset + 24
    drom: list[tuple[int, int]] = []
    for _ in range(seg_count):
        addr, length = struct.unpack_from("<II", data, pos)
        pos += 8
        if 0x3C000000 <= addr < 0x3E000000:
            drom.append((addr, length))
        pos += (length + 3) & ~3
    return drom


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} <ossm-esp32s3.bin>")

    path = Path(sys.argv[1])
    drom = drom_segments(path)
    if len(drom) != 1:
        details = ", ".join(f"0x{addr:08x}+0x{length:x}" for addr, length in drom)
        raise SystemExit(
            f"{path}: expected 1 DROM segment, found {len(drom)} ({details}). "
            "Rebuild with ld/esp32s3 linker scripts (see build.rs)."
        )

    addr, length = drom[0]
    if length < 0x120:
        raise SystemExit(
            f"{path}: DROM segment too small (0x{length:x}); "
            "appdesc/rodata merge likely missing."
        )

    print(f"OK: single DROM segment 0x{addr:08x} size=0x{length:x} ({length} bytes)")


if __name__ == "__main__":
    main()
