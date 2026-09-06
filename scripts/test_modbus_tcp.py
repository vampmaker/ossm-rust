#!/usr/bin/env -S uv run
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


def connect_retry(
    ip: str,
    attempts: int = 10,
    timeout: float = 3.0,
) -> socket.socket:
    last: Exception | None = None
    for i in range(attempts):
        try:
            sock = socket.create_connection((ip, 502), timeout=timeout)
            sock.settimeout(timeout)
            return sock
        except (ConnectionRefusedError, TimeoutError, OSError, socket.timeout) as e:
            last = e
            print(f"   :502 connect attempt {i + 1}/{attempts} failed: {e}")
            time.sleep(0.5)
    raise AssertionError(f":502 connect failed after {attempts} attempts: {last}")


def exchange_fc03(ip: str, tid: int = 1, unit: int = 1, start: int = 0, count: int = 26) -> bytes:
    req = build_mbap_fc03(tid, unit, start, count)
    with connect_retry(ip) as sock:
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


def exchange_fc03_retry(ip: str, attempts: int = 10) -> bytes:
    last: Exception | None = None
    for i in range(attempts):
        try:
            return exchange_fc03(ip, tid=7 + i)
        except Exception as e:
            last = e
            print(f"   FC03 attempt {i + 1}/{attempts} failed: {e}")
            time.sleep(0.8)
    raise AssertionError(f"FC03 failed after {attempts} attempts: {last}")


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


async def main() -> None:
    parser = argparse.ArgumentParser(description="OSSM Modbus TCP relay smoke test")
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
        cur = await wifi.get_pin_config()
        if cur.get("operating_mode") == "rtu_relay":
            print(" -> bounce via servo to re-init UART1")
            await live_set_mode(wifi, ip, "servo")
            await asyncio.sleep(1.0)
        await live_set_mode(wifi, ip, "rtu_relay")

    pin = await wifi.get_pin_config()
    mode = pin.get("operating_mode", "servo")
    print(f"operating_mode={mode}")
    if mode != "rtu_relay":
        raise SystemExit("Device is not in rtu_relay mode (pass --switch-mode)")

    try:
        # Happy-path FC03 (UART owner may still be coming up after switch/reboot)
        body = exchange_fc03_retry(ip)
        print(f"✓ Modbus TCP FC03 OK ({2 + body[1]} PDU bytes)")
        print("✓ TCP :502 accepts connections")

        # Back-to-back on one connection
        with connect_retry(ip) as sock:
            for tid in (10, 11, 12):
                req = build_mbap_fc03(tid, 1, 0, 2)
                sock.sendall(req)
                hdr = read_exact(sock, 7)
                r_tid, _, r_len, _ = struct.unpack(">HHHB", hdr)
                assert r_tid == tid
                _ = read_exact(sock, r_len - 1)
        print("✓ Back-to-back Modbus TCP transactions OK")

        # Truncated / bad length should close or error without killing :502
        with connect_retry(ip, timeout=3.0) as sock:
            sock.settimeout(2.0)
            sock.sendall(b"\x00\x01\x00\x00\xff\xff\x01")  # absurd length
            try:
                _ = sock.recv(16)
            except (TimeoutError, socket.timeout, ConnectionError, OSError):
                pass
        print("✓ Bad MBAP length handled (connection dropped/ignored)")

        body = exchange_fc03_retry(ip)
        print(f"✓ Modbus TCP FC03 still OK after bad MBAP ({2 + body[1]} PDU bytes)")
    finally:
        if args.restore_servo:
            print(" -> Restoring operating_mode=servo (live HTTP)...")
            await live_set_mode(wifi, ip, "servo")
            pin2 = await wifi.get_pin_config()
            assert pin2.get("operating_mode", "servo") == "servo"
            print("✓ Restored servo mode")

    print("✓ All Modbus TCP relay tests PASSED")


if __name__ == "__main__":
    asyncio.run(main())
