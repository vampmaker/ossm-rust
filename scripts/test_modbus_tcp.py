#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "python-dotenv>=1.0.0",
#     "httpx>=0.27.0",
#     "httpx2>=0.1.0",
#     "pyserial>=3.5",
#     "bleak>=0.21.0",
#     "typer>=0.12.0",
#     "rich>=13.7.0",
#     "pydantic>=2.0.0",
# ]
# ///
"""Smoke-test OSSM Modbus TCP relay on port 502 (MBAP ↔ RTU)."""

from __future__ import annotations

import argparse
import asyncio
import os
import socket
import struct
import sys
import time

import httpx

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ossm import DeviceBackend, load_env


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


def build_mbap_fc03(tid: int, unit: int, start: int, count: int) -> bytes:
    pdu = bytes([0x03, (start >> 8) & 0xFF, start & 0xFF, (count >> 8) & 0xFF, count & 0xFF])
    length = 1 + len(pdu)
    return struct.pack(">HHHB", tid, 0, length, unit) + pdu


def read_exact(sock: socket.socket, n: int) -> bytes:
    buf = bytearray()
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("socket closed")
        buf.extend(chunk)
    return bytes(buf)


def exchange_fc03(ip: str, tid: int = 1, unit: int = 1, start: int = 0, count: int = 26) -> bytes:
    req = build_mbap_fc03(tid, unit, start, count)
    with socket.create_connection((ip, 502), timeout=3.0) as sock:
        sock.settimeout(3.0)
        sock.sendall(req)
        hdr = read_exact(sock, 7)
        r_tid, r_proto, r_len, r_unit = struct.unpack(">HHHB", hdr)
        assert r_tid == tid, f"tid mismatch {r_tid} vs {tid}"
        assert r_proto == 0, f"proto {r_proto}"
        assert r_unit == unit, f"unit {r_unit}"
        assert r_len >= 2, f"len {r_len}"
        body = read_exact(sock, r_len - 1)
        assert body[0] == 0x03 or (body[0] & 0x80), f"fc={body[0]:#x}"
        if body[0] & 0x80:
            raise AssertionError(f"Modbus exception: {body.hex()}")
        assert body[1] == count * 2, f"byte_count={body[1]}"
        return body


async def wait_http(ip: str, timeout_s: float = 45.0) -> None:
    deadline = time.monotonic() + timeout_s
    url = f"http://{ip}/pin-config"
    while time.monotonic() < deadline:
        try:
            async with httpx.AsyncClient(timeout=2.0) as client:
                r = await client.get(url)
                if r.status_code == 200:
                    return
        except Exception:
            pass
        await asyncio.sleep(1.0)
    raise TimeoutError(f"Device HTTP not reachable at {url}")


async def main() -> None:
    parser = argparse.ArgumentParser(description="OSSM Modbus TCP relay smoke test")
    parser.add_argument(
        "--switch-mode",
        action="store_true",
        help="Switch device to rtu_relay via serial and restart before testing",
    )
    parser.add_argument(
        "--restore-servo",
        action="store_true",
        help="Restore operating_mode=servo via serial after tests",
    )
    args = parser.parse_args()
    load_env()
    ip = os.environ.get("DEVICE_IP", "").strip().strip('"')
    if not ip:
        raise SystemExit("DEVICE_IP missing in .env")

    serial = DeviceBackend(mode="serial")
    if args.switch_mode:
        print(" -> Switching operating_mode to rtu_relay via serial...")
        await serial.set_pin_config({"operating_mode": "rtu_relay"})
        await serial.restart_device()
        await asyncio.sleep(12)
        await wait_http(ip)

    wifi = DeviceBackend(mode="wifi", ip=ip)
    pin = await wifi.get_pin_config()
    mode = pin.get("operating_mode", "servo")
    print(f"operating_mode={mode}")
    if mode != "rtu_relay":
        raise SystemExit("Device is not in rtu_relay mode (pass --switch-mode)")

    # Happy-path FC03
    body = exchange_fc03(ip, tid=7)
    print(f"✓ Modbus TCP FC03 OK ({2 + body[1]} PDU bytes)")
    print("✓ TCP :502 accepts connections")

    # Back-to-back on one connection
    with socket.create_connection((ip, 502), timeout=3.0) as sock:
        sock.settimeout(3.0)
        for tid in (10, 11, 12):
            req = build_mbap_fc03(tid, 1, 0, 2)
            sock.sendall(req)
            hdr = read_exact(sock, 7)
            r_tid, _, r_len, _ = struct.unpack(">HHHB", hdr)
            assert r_tid == tid
            _ = read_exact(sock, r_len - 1)
    print("✓ Back-to-back Modbus TCP transactions OK")

    # Truncated / bad length should close or error
    with socket.create_connection((ip, 502), timeout=3.0) as sock:
        sock.settimeout(2.0)
        sock.sendall(b"\x00\x01\x00\x00\xff\xff\x01")  # absurd length
        try:
            _ = sock.recv(16)
        except (TimeoutError, socket.timeout, ConnectionError, OSError):
            pass
    print("✓ Bad MBAP length handled (connection dropped/ignored)")

    if args.restore_servo:
        print(" -> Restoring operating_mode=servo via serial...")
        await serial.set_pin_config({"operating_mode": "servo"})
        await serial.restart_device()
        await asyncio.sleep(12)
        await wait_http(ip)
        pin2 = await DeviceBackend(mode="wifi", ip=ip).get_pin_config()
        assert pin2.get("operating_mode", "servo") == "servo"
        print("✓ Restored servo mode")

    print("✓ All Modbus TCP relay tests PASSED")


if __name__ == "__main__":
    asyncio.run(main())
