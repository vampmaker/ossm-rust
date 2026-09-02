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
"""Live-switch `operating_mode=rs485` (no reboot) and exercise USB + /ws/rs485 via ossm-std."""

from __future__ import annotations

import asyncio
import os
import socket
import struct
import subprocess
import sys
import time
from pathlib import Path

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


def exchange_fc03(host: str, port: int, tid: int = 1, unit: int = 1, count: int = 26) -> bytes:
    req = build_mbap_fc03(tid, unit, 0, count)
    with socket.create_connection((host, port), timeout=5.0) as sock:
        sock.settimeout(5.0)
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


async def wait_mode(ip: str, want: str, timeout_s: float = 15.0) -> None:
    deadline = time.monotonic() + timeout_s
    url = f"http://{ip}/pin-config"
    last = None
    while time.monotonic() < deadline:
        try:
            async with httpx.AsyncClient(timeout=2.0) as client:
                r = await client.get(url)
                if r.status_code == 200:
                    last = r.json().get("operating_mode")
                    if last == want:
                        return
        except Exception:
            pass
        await asyncio.sleep(0.4)
    raise TimeoutError(f"operating_mode did not become {want!r} (last={last!r})")


async def wait_tcp(host: str, port: int, timeout_s: float = 20.0) -> None:
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        try:
            with socket.create_connection((host, port), timeout=1.0):
                return
        except OSError:
            await asyncio.sleep(0.3)
    raise TimeoutError(f"TCP {host}:{port} not accepting")


def start_std(extra: list[str], log_name: str) -> subprocess.Popen[str]:
    repo = Path(__file__).resolve().parents[1]
    log_path = repo / "target" / log_name
    log_path.parent.mkdir(parents=True, exist_ok=True)
    cmd = [
        "cargo",
        "+stable",
        "run",
        "-p",
        "ossm-std",
        "--offline",
        "--",
        "--mode",
        "rtu-relay",
        "--no-repl",
        "--config",
        str(repo / "target" / "ossm-std-rs485-e2e.json"),
        *extra,
    ]
    print(" $", " ".join(cmd))
    logf = open(log_path, "w", encoding="utf-8")
    proc = subprocess.Popen(
        cmd,
        cwd=repo,
        stdout=logf,
        stderr=subprocess.STDOUT,
        text=True,
    )
    proc._ossm_log = logf  # type: ignore[attr-defined]
    proc._ossm_log_path = log_path  # type: ignore[attr-defined]
    return proc


def stop_std(proc: subprocess.Popen[str]) -> None:
    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=8)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=3)
    logf = getattr(proc, "_ossm_log", None)
    if logf is not None:
        logf.close()


def dump_std_log(proc: subprocess.Popen[str]) -> None:
    path = getattr(proc, "_ossm_log_path", None)
    if path is None:
        return
    try:
        text = Path(path).read_text(encoding="utf-8", errors="replace")
    except OSError:
        return
    tail = text[-4000:] if len(text) > 4000 else text
    if tail.strip():
        print("--- ossm-std log ---")
        print(tail)


async def wait_ws_rs485(ip: str, timeout_s: float = 15.0) -> None:
    deadline = time.monotonic() + timeout_s
    url = f"ws://{ip}/ws/rs485"
    last_err: Exception | None = None
    while time.monotonic() < deadline:
        try:
            async with websockets.connect(url, open_timeout=2, close_timeout=1):
                return
        except Exception as e:
            last_err = e
            await asyncio.sleep(0.4)
    raise TimeoutError(f"/ws/rs485 not accepting: {last_err}")


def exchange_fc03_retry(host: str, port: int, attempts: int = 8) -> bytes:
    last: Exception | None = None
    for i in range(attempts):
        try:
            return exchange_fc03(host, port)
        except Exception as e:
            last = e
            print(f"   FC03 attempt {i + 1}/{attempts} failed: {e}")
            time.sleep(0.6)
    raise AssertionError(f"FC03 failed after {attempts} attempts: {last}")


UPS_MIN = 300
DT_MAX_MS = 4.5


async def restore_servo(wifi: DeviceBackend, ip: str) -> None:
    print(" -> set pin.operating_mode servo (no restart)")
    pin = await wifi.get_pin_config()
    pin["operating_mode"] = "servo"
    await wifi.set_pin_config(pin)
    await wait_mode(ip, "servo")

    deadline = time.monotonic() + 45.0
    last: dict | None = None
    while time.monotonic() < deadline:
        try:
            async with httpx.AsyncClient(timeout=2.0) as client:
                r = await client.get(f"http://{ip}/state")
                if r.status_code == 200:
                    last = r.json()
                    ups = last.get("ups")
                    dt_max = last.get("dt_max_ms")
                    pos_min = last.get("pos_min")
                    pos_max = last.get("pos_max")
                    homed = (
                        isinstance(pos_min, (int, float))
                        and isinstance(pos_max, (int, float))
                        and pos_max > pos_min
                    )
                    if (
                        isinstance(ups, (int, float))
                        and ups >= UPS_MIN
                        and isinstance(dt_max, (int, float))
                        and dt_max < DT_MAX_MS
                        and homed
                    ):
                        print(
                            f"servo /state ups={ups} dt_max_ms={dt_max:.3f} "
                            f"dt_avg_ms={last.get('dt_avg_ms')} "
                            f"pos_min={pos_min} pos_max={pos_max}"
                        )
                        print("✓ live switch back to servo (motor update rate recovered, homed)")
                        return
        except Exception:
            pass
        await asyncio.sleep(0.5)
    print(f"servo /state last={last}")
    raise AssertionError(
        f"motor loop did not recover to ups>={UPS_MIN}, dt_max_ms<{DT_MAX_MS}, "
        f"and pos_max>pos_min"
    )


async def main() -> None:
    load_env()
    ip = os.environ.get("DEVICE_IP", "").strip().strip('"')
    port = os.environ.get("DEVICE_PORT", "").strip().strip('"')
    if not ip:
        raise SystemExit("DEVICE_IP missing in .env")
    if not port:
        raise SystemExit("DEVICE_PORT missing in .env")

    wifi = DeviceBackend(mode="wifi", ip=ip)
    pin = await wifi.get_pin_config()
    print(f"boot operating_mode={pin.get('operating_mode')}")

    print(" -> set pin.operating_mode rs485 (no restart)")
    pin["operating_mode"] = "rs485"
    await wifi.set_pin_config(pin)
    await wait_mode(ip, "rs485")
    await wait_ws_rs485(ip)
    print("✓ live switch to rs485")

    try:
        usb_proc = start_std(
            [
                "--serial",
                port,
                "--baud",
                "115200",
                "--modbus-bind",
                "127.0.0.1:1502",
                "--bind",
                "127.0.0.1:18082",
            ],
            "ossm-std-rs485-usb.log",
        )
        try:
            await wait_tcp("127.0.0.1", 1502)
            # ACM open may still pulse DTR and reboot; wait until rs485 is back.
            await wait_mode(ip, "rs485", timeout_s=20.0)
            await wait_ws_rs485(ip, timeout_s=20.0)
            body = exchange_fc03_retry("127.0.0.1", 1502)
            print(f"✓ USB rs485 path FC03 OK ({2 + body[1]} PDU bytes)")
        except Exception:
            dump_std_log(usb_proc)
            raise
        finally:
            stop_std(usb_proc)

        await wait_mode(ip, "rs485", timeout_s=20.0)
        await wait_ws_rs485(ip, timeout_s=20.0)

        ws_proc = start_std(
            [
                "--rs485-ws",
                f"ws://{ip}/ws/rs485",
                "--modbus-bind",
                "127.0.0.1:1503",
                "--bind",
                "127.0.0.1:18083",
            ],
            "ossm-std-rs485-ws.log",
        )
        try:
            await wait_tcp("127.0.0.1", 1503)
            body = exchange_fc03_retry("127.0.0.1", 1503)
            print(f"✓ WS /ws/rs485 path FC03 OK ({2 + body[1]} PDU bytes)")
        except Exception:
            dump_std_log(ws_proc)
            raise
        finally:
            stop_std(ws_proc)
    finally:
        await restore_servo(wifi, ip)

    print("✓ All RS-485 transceiver tests PASSED")


if __name__ == "__main__":
    asyncio.run(main())
