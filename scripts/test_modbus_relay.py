#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "websockets>=12.0",
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
"""Smoke-test OSSM Modbus WebSocket relay at /ws/modbus (binary RTU frames)."""

from __future__ import annotations

import argparse
import asyncio
import os
import struct
import sys
import time

import httpx
import websockets

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


def build_fc03(unit: int, start: int, count: int) -> bytes:
    pdu = bytes([unit, 0x03, (start >> 8) & 0xFF, start & 0xFF, (count >> 8) & 0xFF, count & 0xFF])
    crc = calc_crc16(pdu)
    return pdu + struct.pack("<H", crc)


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


async def run_ws_modbus_test(ip: str) -> None:
    url = f"ws://{ip}/ws/modbus"
    frame = build_fc03(1, 0, 26)
    print(f"Connecting to {url}...")
    async with websockets.connect(url, open_timeout=5, close_timeout=2) as ws:
        await ws.send(frame)
        resp = await asyncio.wait_for(ws.recv(), timeout=2.0)
        if isinstance(resp, str):
            resp = resp.encode("latin1")
        assert isinstance(resp, (bytes, bytearray)), f"expected binary, got {type(resp)}"
        assert len(resp) >= 5, f"short response: {resp!r}"
        assert resp[0] == 1 and resp[1] == 0x03, f"bad header: {resp[:4].hex()}"
        assert calc_crc16(resp[:-2]) == struct.unpack("<H", resp[-2:])[0], "CRC mismatch"
        byte_count = resp[2]
        assert byte_count == 52, f"expected 52 data bytes, got {byte_count}"
        print(f"✓ WS Modbus FC03 OK ({len(resp)} bytes)")


async def main() -> None:
    parser = argparse.ArgumentParser(description="OSSM Modbus WebSocket relay smoke test")
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

    # Servo WS path should still work for ping; modbus WS is under test.
    await run_ws_modbus_test(ip)

    # Motion mutate should be rejected
    try:
        async with httpx.AsyncClient(timeout=5.0) as client:
            r = await client.post(f"http://{ip}/config", json={"bpm": 40, "paused": True})
            assert r.status_code == 409, f"expected 409 for /config in relay, got {r.status_code}"
            print("✓ POST /config rejected in relay mode (409)")
    except AssertionError:
        raise

    if args.restore_servo:
        print(" -> Restoring operating_mode=servo via serial...")
        await serial.set_pin_config({"operating_mode": "servo"})
        await serial.restart_device()
        await asyncio.sleep(12)
        await wait_http(ip)
        pin2 = await DeviceBackend(mode="wifi", ip=ip).get_pin_config()
        assert pin2.get("operating_mode", "servo") == "servo"
        print("✓ Restored servo mode")

    print("✓ All Modbus WebSocket relay tests PASSED")


if __name__ == "__main__":
    asyncio.run(main())
