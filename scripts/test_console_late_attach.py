#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "pyserial>=3.5",
#     "python-dotenv>=1.0",
# ]
# ///
"""Verify USB CLI ACKs after ACM has been closed (configurator late-attach).

Reproduces: device runs for a while with nobody reading USB Serial/JTAG, then
the flasher/configurator opens the port and expects `{path} set to`.
"""

from __future__ import annotations

import os
import sys
import time
from pathlib import Path

import serial
from dotenv import load_dotenv

SCRIPT_DIR = Path(__file__).resolve().parent
load_dotenv(SCRIPT_DIR.parent / ".env")
PORT = os.environ.get("DEVICE_PORT", "/dev/ttyACM0")

IDLE_S = 12.0
ACK_TIMEOUT_S = 2.0
SET_CMD = "set pin.ble_enabled true"
ACK_NEEDLE = "pin.ble_enabled set to"


def open_acm(*, dtr: bool) -> serial.Serial:
    # dsrdtr=False: do not let pyserial pulse DTR on open (native USB-Serial-JTAG
    # reset would hide a wedged IN endpoint from unread boot logs).
    ser = serial.Serial(
        PORT,
        115200,
        timeout=0.1,
        write_timeout=2.0,
        dsrdtr=False,
        rtscts=False,
    )
    ser.dtr = dtr
    ser.rts = False
    return ser


def try_soft_reset() -> None:
    try:
        ser = open_acm(dtr=True)
        try:
            ser.reset_input_buffer()
            ser.write(b"reset\r\n")
            ser.flush()
            time.sleep(0.15)
        finally:
            ser.close()
    except serial.SerialException as e:
        print(f"  (soft reset skipped: {e})")


def wait_ack(ser: serial.Serial, needle: str, timeout_s: float) -> str:
    end = time.monotonic() + timeout_s
    chunks: list[str] = []
    while time.monotonic() < end:
        n = ser.in_waiting
        if n:
            chunks.append(ser.read(n).decode("utf-8", errors="replace"))
            if needle in "".join(chunks):
                break
        else:
            time.sleep(0.02)
    return "".join(chunks)


def main() -> None:
    print(f"Serial {PORT}: reset, idle {IDLE_S:.0f}s with ACM closed, then CLI set")
    try_soft_reset()
    print(f"  ACM closed for {IDLE_S:.0f}s (motor/WiFi logs run unread)...")
    time.sleep(IDLE_S)

    try:
        ser = open_acm(dtr=False)
    except serial.SerialException as e:
        print(f"FAIL: could not open {PORT}: {e}")
        sys.exit(1)

    try:
        ser.reset_input_buffer()
        ser.write(f"{SET_CMD}\r\n".encode())
        ser.flush()
        log = wait_ack(ser, ACK_NEEDLE, ACK_TIMEOUT_S)
    except serial.SerialTimeoutException:
        print("FAIL: serial write timeout (device not reading USB OUT)")
        sys.exit(1)
    finally:
        ser.close()

    if ACK_NEEDLE not in log:
        print(f"FAIL: no CLI ACK {ACK_NEEDLE!r} within {ACK_TIMEOUT_S:.0f}s after late attach")
        print(f"  received {len(log)} bytes: {log[:500]!r}")
        sys.exit(1)

    print(f"✓ late-attach CLI ACK ok ({ACK_NEEDLE})")


if __name__ == "__main__":
    main()
