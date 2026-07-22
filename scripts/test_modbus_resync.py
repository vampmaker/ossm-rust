#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# ///
"""Host-side mirror of modbus_rtu CRC + find_modbus_response (see src/modbus_rtu.rs)."""

from __future__ import annotations


def calc_crc16(data: bytes) -> int:
    crc = 0xFFFF
    for b in data:
        crc ^= b
        for _ in range(8):
            if crc & 1:
                crc = (crc >> 1) ^ 0xA001
            else:
                crc >>= 1
    return crc & 0xFFFF


def verify_rtu_crc(frame: bytes) -> bool:
    if len(frame) < 4:
        return False
    expected = frame[-2] | (frame[-1] << 8)
    return calc_crc16(frame[:-2]) == expected


def guess_response_frame_len(hdr: bytes) -> int:
    if len(hdr) < 2:
        raise ValueError("short header")
    func = hdr[1]
    if func >= 0x80:
        return 5
    if func in (1, 2, 3, 4):
        if len(hdr) < 3:
            raise ValueError("need byte count")
        return hdr[2] + 3 + 2
    if func in (5, 6, 15, 16):
        return 8
    raise ValueError(f"unknown func {func:#x}")


def find_modbus_response(rx: bytes, slave: int, expected: int) -> tuple[int, int] | None:
    if len(rx) < 4:
        return None
    for offset in range(0, len(rx) - 3):
        if rx[offset] != slave:
            continue
        hdr_len = min(3, len(rx) - offset)
        try:
            frame_len = guess_response_frame_len(rx[offset : offset + hdr_len])
        except ValueError:
            continue
        if frame_len < 4 or offset + frame_len > len(rx):
            continue
        if frame_len != expected and frame_len != 5:
            continue
        frame = rx[offset : offset + frame_len]
        if verify_rtu_crc(frame):
            return offset, frame_len
    return None


def classify_recovered(rx_len: int, offset: int, frame_len: int) -> str:
    if offset > 0:
        return "long_resync"
    if rx_len > frame_len:
        return "long"
    return "exact"


def _frame(payload: bytes) -> bytes:
  crc = calc_crc16(payload)
  return payload + bytes((crc & 0xFF, crc >> 8))


def test_exact() -> None:
    raw = _frame(bytes([0x01, 0x03, 0x04, 0x00, 0x10, 0x00, 0x20]))
    assert find_modbus_response(raw, 1, 9) == (0, 9)
    assert classify_recovered(len(raw), 0, 9) == "exact"


def test_trailing_junk() -> None:
    raw = _frame(bytes([0x01, 0x03, 0x04, 0x00, 0x10, 0x00, 0x20])) + bytes([0xFF, 0xFE])
    assert find_modbus_response(raw, 1, 9) == (0, 9)
    assert classify_recovered(len(raw), 0, 9) == "long"


def test_leading_resync() -> None:
    raw = bytes([0xAA, 0xBB, 0xCC]) + _frame(bytes([0x01, 0x03, 0x04, 0x00, 0x10, 0x00, 0x20]))
    assert find_modbus_response(raw, 1, 9) == (3, 9)
    assert classify_recovered(len(raw), 3, 9) == "long_resync"


def test_bad_crc() -> None:
    raw = bytes([0x01, 0x03, 0x04, 0x00, 0x10, 0x00, 0x20, 0x00, 0x00])
    assert find_modbus_response(raw, 1, 9) is None


def main() -> None:
    test_exact()
    test_trailing_junk()
    test_leading_resync()
    test_bad_crc()
    print("✓ modbus resync host tests passed")


if __name__ == "__main__":
    main()
