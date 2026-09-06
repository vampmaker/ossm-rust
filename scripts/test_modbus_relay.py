#!/usr/bin/env -S uv run
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
    url = f"http://{ip}/shell-config"
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


async def wait_mode(ip: str, want: str, timeout_s: float = 20.0) -> None:
    deadline = time.monotonic() + timeout_s
    last = None
    while time.monotonic() < deadline:
        try:
            async with httpx.AsyncClient(timeout=2.0) as client:
                r = await client.get(f"http://{ip}/shell-config")
                if r.status_code == 200:
                    last = r.json().get("operating_mode")
                    if last == want:
                        return
        except Exception:
            pass
        await asyncio.sleep(0.4)
    raise TimeoutError(f"operating_mode did not become {want!r} (last={last!r})")


async def live_set_mode(wifi: DeviceBackend, ip: str, mode: str) -> None:
    pin = await wifi.get_pin_config()
    pin["operating_mode"] = mode
    await wifi.set_pin_config(pin)
    await wait_mode(ip, mode)


async def run_ws_modbus_test(ip: str) -> None:
    url = f"ws://{ip}/ws/modbus"
    frame = build_fc03(1, 0, 26)
    print(f"Connecting to {url}...")
    last: Exception | None = None
    for i in range(10):
        try:
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
                return
        except Exception as e:
            last = e
            print(f"   WS FC03 attempt {i + 1}/10 failed: {e}")
            await asyncio.sleep(0.8)
    raise AssertionError(f"WS Modbus FC03 failed after 10 attempts: {last}")


async def main() -> None:
    parser = argparse.ArgumentParser(description="OSSM Modbus WebSocket relay smoke test")
    parser.add_argument(
        "--switch-mode",
        action="store_true",
        help="Live-switch device to rtu_relay via HTTP /shell-config (no reboot)",
    )
    parser.add_argument(
        "--restore-servo",
        action="store_true",
        help="Live-switch operating_mode=servo via HTTP after tests",
    )
    args = parser.parse_args()
    load_env()
    ip = os.environ.get("DEVICE_IP", "").strip().strip('"')
    if not ip:
        raise SystemExit("DEVICE_IP missing in .env")

    await wait_http(ip)
    wifi = DeviceBackend(mode="wifi", ip=ip)
    if args.switch_mode:
        print(" -> Switching operating_mode to rtu_relay (live HTTP)...")
        await live_set_mode(wifi, ip, "rtu_relay")

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
        print(" -> Restoring operating_mode=servo (live HTTP)...")
        await live_set_mode(wifi, ip, "servo")
        pin2 = await wifi.get_pin_config()
        assert pin2.get("operating_mode", "servo") == "servo"
        print("✓ Restored servo mode")

    print("✓ All Modbus WebSocket relay tests PASSED")


if __name__ == "__main__":
    asyncio.run(main())
