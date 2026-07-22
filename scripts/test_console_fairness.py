#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "httpx>=0.27",
#     "pyserial>=3.5",
#     "python-dotenv>=1.0",
# ]
# ///
"""Verify serial CLI stays responsive under modbus_debug MODBUS_DBG TX flood."""

from __future__ import annotations

import os
import sys
import time
from pathlib import Path

import httpx
import serial
from dotenv import load_dotenv

SCRIPT_DIR = Path(__file__).resolve().parent
load_dotenv(SCRIPT_DIR.parent / ".env")
IP = os.environ.get("DEVICE_IP", "192.168.24.63")
PORT = os.environ.get("DEVICE_PORT", "/dev/ttyACM0")
BASE = f"http://{IP}"


def serial_query(cmd: str, timeout_s: float = 2.0) -> str:
    ser = serial.Serial(PORT, 115200, timeout=0.1, write_timeout=2.0)
    ser.dtr = True
    ser.rts = False
    try:
        ser.reset_input_buffer()
        ser.write(f"{cmd}\r\n".encode())
        ser.flush()
        end = time.monotonic() + timeout_s
        chunks: list[str] = []
        while time.monotonic() < end:
            n = ser.in_waiting
            if n:
                chunks.append(ser.read(n).decode("utf-8", errors="replace"))
            else:
                time.sleep(0.02)
        return "".join(chunks)
    finally:
        ser.close()


def try_serial_ack(mode: str, nbytes: int, attempts: int = 12) -> str | None:
    needle = f"modbus_inject_junk: {mode} {nbytes}"
    for _ in range(attempts):
        try:
            log = serial_query("get-modbus-inject-junk", timeout_s=6.0)
        except serial.SerialTimeoutException:
            continue
        if needle in log:
            return log
        time.sleep(0.5)
    return None


def main() -> None:
    print(f"Device {BASE} serial {PORT}")
    with httpx.Client(timeout=10) as client:
        pins = client.get(f"{BASE}/pin-config").json()
        if not pins.get("modbus_debug"):
            print("FAIL: enable modbus_debug and reboot first")
            sys.exit(1)

        cases = [("trailing", 3), ("leading", 3)]
        for mode, nbytes in cases:
            client.post(f"{BASE}/modbus-inject", json={"mode": "off", "nbytes": 0})
            client.post(f"{BASE}/paused", json={"paused": True})
            time.sleep(0.5)
            client.post(f"{BASE}/modbus-inject", json={"mode": mode, "nbytes": nbytes})
            got = client.get(f"{BASE}/modbus-inject").json()
            if got.get("mode") != mode or got.get("nbytes") != nbytes:
                print(f"FAIL {mode}: HTTP inject not applied ({got})")
                sys.exit(1)

            client.post(f"{BASE}/paused", json={"paused": False})
            time.sleep(1.5)

            ups = 0
            for _ in range(40):
                state = client.get(f"{BASE}/state").json()
                ups = state.get("ups", 0) or 0
                if ups >= 100:
                    break
                time.sleep(0.5)
            if ups < 100:
                print(f"FAIL {mode}: motor loop not running (ups={ups})")
                sys.exit(1)

            log = try_serial_ack(mode, nbytes)
            if log is None:
                print(f"FAIL {mode}: serial CLI unresponsive under MODBUS_DBG flood")
                sys.exit(1)
            print(f"  {mode} n={nbytes}: serial ACK ok (ups={ups})")

            client.post(f"{BASE}/modbus-inject", json={"mode": "off", "nbytes": 0})
            client.post(f"{BASE}/paused", json={"paused": True})
            time.sleep(0.5)

        client.post(f"{BASE}/modbus-inject", json={"mode": "off", "nbytes": 0})
        client.post(f"{BASE}/pin-config", json={**pins, "modbus_debug": False})
        client.post(f"{BASE}/restart")

    print("✓ console fairness verification passed")


if __name__ == "__main__":
    main()
