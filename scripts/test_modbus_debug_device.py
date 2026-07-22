#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "httpx>=0.27",
#     "python-dotenv>=1.0",
# ]
# ///
"""Verify modbus_debug junk-skip + fault injection on a live device via RTT."""

from __future__ import annotations

import os
import re
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

import httpx
from dotenv import load_dotenv

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent
load_dotenv(REPO_ROOT / ".env")

IP = os.environ.get("DEVICE_IP", "192.168.24.63")
BASE = f"http://{IP}"
CHIP = os.environ.get("DEVICE_MODEL", "esp32c6")
ELF = Path(
    os.environ.get(
        "OSSM_ELF",
        REPO_ROOT / f"target/riscv32imac-unknown-none-elf/release/ossm-rust",
    )
)
PROBE_RS = Path(os.environ.get("PROBE_RS", shutil.which("probe-rs") or ""))
RTT_OUT = Path(os.environ.get("RTT_OUT", "/tmp/ossm_rtt_modbus_dbg.txt"))


def probe_cmd() -> list[str]:
    if not PROBE_RS:
        print("FAIL: probe-rs not found (install probe-rs or set PROBE_RS)")
        sys.exit(1)
    if not ELF.is_file():
        print(f"FAIL: ELF not found at {ELF} (build firmware first)")
        sys.exit(1)
    cmd = [
        str(PROBE_RS),
        "attach",
        "--chip",
        CHIP,
        "--rtt-scan-memory",
        "--no-catch-reset",
        str(ELF),
    ]
    # ESP USB-JTAG often needs elevated permissions without udev rules.
    if os.geteuid() != 0 and shutil.which("sudo"):
        return ["sudo", *cmd]
    return cmd


def wait_device_ready(timeout_s: float = 90.0) -> None:
    end = time.monotonic() + timeout_s
    while time.monotonic() < end:
        try:
            with httpx.Client(timeout=3) as client:
                pins = client.get(f"{BASE}/pin-config").json()
                if not pins.get("modbus_debug"):
                    print("FAIL: enable modbus_debug and reboot first")
                    sys.exit(1)
                state = client.get(f"{BASE}/state").json()
                ups = state.get("ups", 0) or 0
                # Homing + WiFi can take ~15s after probe attach/reset.
                if ups > 50:
                    return
        except (httpx.HTTPError, OSError, ValueError):
            pass
        time.sleep(1)
    print("FAIL: device not reachable / motor loop not running")
    sys.exit(1)


def set_inject(client: httpx.Client, mode: str, nbytes: int) -> dict:
    r = client.post(f"{BASE}/modbus-inject", json={"mode": mode, "nbytes": nbytes})
    r.raise_for_status()
    return r.json()


def main() -> None:
    print(f"Device {BASE} RTT via {PROBE_RS or 'probe-rs'} chip={CHIP}")

    RTT_OUT.unlink(missing_ok=True)
    with RTT_OUT.open("wb") as rtt_file:
        proc = subprocess.Popen(
            probe_cmd(),
            stdout=rtt_file,
            stderr=subprocess.PIPE,
        )
        try:
            wait_device_ready()
            if proc.poll() is not None:
                err = proc.stderr.read().decode(errors="replace") if proc.stderr else ""
                print("FAIL: probe-rs attach exited early")
                if err:
                    print(err[:600])
                sys.exit(1)
            with httpx.Client(timeout=10) as client:
                pins = client.get(f"{BASE}/pin-config").json()
                cases = [
                    ("trailing", 3, "long"),
                    ("leading", 3, "long_resync"),
                ]
                for mode, nbytes, expect_class in cases:
                    set_inject(client, "off", 0)
                    client.post(f"{BASE}/paused", json={"paused": True})
                    time.sleep(1.0)
                    set_inject(client, mode, nbytes)
                    time.sleep(0.3)
                    got = client.get(f"{BASE}/modbus-inject").json()
                    if got.get("mode") != mode or got.get("nbytes") != nbytes:
                        print(f"FAIL {mode}: inject not applied ({got})")
                        sys.exit(1)
                    client.post(f"{BASE}/paused", json={"paused": False})
                    time.sleep(8)
                    set_inject(client, "off", 0)
                    client.post(f"{BASE}/paused", json={"paused": True})
                    time.sleep(1.0)

                set_inject(client, "off", 0)
                client.post(
                    f"{BASE}/pin-config",
                    json={**pins, "modbus_debug": False},
                )
                client.post(f"{BASE}/restart")
        finally:
            time.sleep(1.0)
            if proc.poll() is None:
                proc.send_signal(signal.SIGINT)
                try:
                    proc.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    proc.kill()

    log = RTT_OUT.read_text(encoding="utf-8", errors="replace")
    hits = re.findall(
        r"MODBUS_DBG ok=(\w+) class=(\w+).*skip=(\d+) frame_len=(\d+)",
        log,
    )
    if not hits:
        print("FAIL: no MODBUS_DBG lines in RTT capture")
        print(log[-1200:])
        sys.exit(1)

    trailing = [h for h in hits if h[1] == "long"]
    leading = [h for h in hits if h[1] == "long_resync"]
    for mode, nbytes, expect_class in cases:
        bucket = trailing if expect_class == "long" else leading
        if not bucket:
            print(f"FAIL {mode}: no class={expect_class} in RTT log")
            sys.exit(1)
        ok, cls, skip, flen = bucket[-1]
        print(f"  {mode} n={nbytes}: ok={ok} class={cls} skip={skip} frame_len={flen}")
        if ok != "true" or cls != expect_class:
            print(f"FAIL {mode}: expected ok=true class={expect_class}")
            sys.exit(1)
        if expect_class == "long_resync" and skip != str(nbytes):
            print(f"FAIL {mode}: expected skip={nbytes}")
            sys.exit(1)

    print("✓ modbus debug inject verification passed (RTT)")


if __name__ == "__main__":
    main()
