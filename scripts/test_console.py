#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "httpx>=0.27",
#     "pyserial>=3.5",
#     "python-dotenv>=1.0",
# ]
# ///
"""Live USB console tests: late-attach CLI, motor ups, probe-rs RTT, fairness.

Usage:
  ./scripts/test_console.py
  ./scripts/test_console.py --only ups,rtt
"""

from __future__ import annotations

import argparse
import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

import httpx
import serial
from dotenv import load_dotenv

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent
load_dotenv(REPO_ROOT / ".env")

IP = os.environ.get("DEVICE_IP", "192.168.24.63")
PORT = os.environ.get("DEVICE_PORT", "/dev/ttyACM0")
CHIP = os.environ.get("DEVICE_MODEL", "esp32c6")
BASE = f"http://{IP}"

UPS_MIN = 300
UPS_MIN_DEBUG = 100
DT_MAX_MS = 4.5
LATE_IDLE_S = 12.0
ACK_TIMEOUT_S = 2.0
SET_CMD = "set pin.ble_enabled true"
ACK_NEEDLE = "pin.ble_enabled set to"

ELF = Path(
    os.environ.get(
        "OSSM_ELF",
        REPO_ROOT / "target/riscv32imac-unknown-none-elf/release/ossm-rust",
    )
)
PROBE_RS = Path(os.environ.get("PROBE_RS", shutil.which("probe-rs") or ""))
RTT_OUT = Path(os.environ.get("RTT_OUT", "/tmp/ossm_rtt_console.txt"))


def fail(msg: str) -> None:
    print(f"FAIL: {msg}")
    sys.exit(1)


def client() -> httpx.Client:
    return httpx.Client(timeout=10)


def get_state(http: httpx.Client) -> dict:
    return http.get(f"{BASE}/state").json()


def wait_http(timeout_s: float = 45.0) -> None:
    end = time.monotonic() + timeout_s
    last = None
    while time.monotonic() < end:
        try:
            with client() as http:
                http.get(f"{BASE}/pin-config").raise_for_status()
                return
        except Exception as e:
            last = e
            time.sleep(1)
    fail(f"device HTTP not reachable at {BASE} ({last})")


def wait_motor(http: httpx.Client, min_ups: int, timeout_s: float = 90.0) -> dict:
    end = time.monotonic() + timeout_s
    last: dict = {}
    while time.monotonic() < end:
        try:
            last = get_state(http)
            ups = last.get("ups") or 0
            if ups >= min_ups and last.get("motor_connected"):
                return last
        except (httpx.HTTPError, OSError, ValueError):
            pass
        time.sleep(1)
    fail(
        f"motor loop not ready (ups={last.get('ups')} connected={last.get('motor_connected')})"
    )
    return last


def sample_loop(
    http: httpx.Client, n: int, gap_s: float = 1.1, skip_first: bool = True
) -> list[dict]:
    rows: list[dict] = []
    for i in range(n):
        s = get_state(http)
        rec = {
            "ups": s.get("ups") or 0,
            "dt_max": float(s.get("dt_max_ms") or 0),
            "dt_avg": float(s.get("dt_avg_ms") or 0),
            "connected": bool(s.get("motor_connected")),
        }
        print(
            f"    [{i}] ups={rec['ups']} dt_max={rec['dt_max']:.3f} "
            f"dt_avg={rec['dt_avg']:.3f} connected={rec['connected']}"
        )
        if not (skip_first and i == 0):
            rows.append(rec)
        time.sleep(gap_s)
    return rows


def assert_healthy(rows: list[dict], *, min_ups: int, label: str) -> None:
    if not rows:
        fail(f"{label}: no samples")
    ups = [r["ups"] for r in rows]
    dt = [r["dt_max"] for r in rows]
    if min(ups) < min_ups:
        fail(f"{label}: ups {min(ups)} < {min_ups}")
    if max(dt) >= DT_MAX_MS:
        fail(f"{label}: dt_max {max(dt):.3f} >= {DT_MAX_MS} ms")
    print(
        f"  {label}: ups {min(ups)}..{max(ups)} dt_max {min(dt):.3f}..{max(dt):.3f}"
    )


def open_acm(*, dtr: bool = False, rts: bool = False) -> serial.Serial:
    """Set DTR/RTS *before* open so CDC ACM does not pulse chip reset."""
    ser = serial.Serial()
    ser.port = PORT
    ser.baudrate = 115200
    ser.timeout = 0.1
    ser.write_timeout = 2.0
    ser.dsrdtr = False
    ser.rtscts = False
    ser.dtr = dtr
    ser.rts = rts
    ser.open()
    ser.dtr = dtr
    ser.rts = rts
    return ser


def drain(ser: serial.Serial) -> bytes:
    n = ser.in_waiting
    return ser.read(n) if n else b""


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


def serial_cmd(ser: serial.Serial, cmd: str, needle: str, timeout_s: float = 6.0) -> str:
    ser.reset_input_buffer()
    ser.write(f"{cmd}\r\n".encode())
    ser.flush()
    return wait_ack(ser, needle, timeout_s)


def test_ups_closed() -> None:
    print("=== ups (ACM closed) ===")
    wait_http()
    with client() as http:
        wait_motor(http, UPS_MIN)
        rows = sample_loop(http, 5)
        assert_healthy(rows, min_ups=UPS_MIN, label="ACM-closed")
    print("✓ ups ACM-closed ok")


def test_ups_acm_open() -> None:
    print("=== ups (ACM held open, DTR=0 RTS=0) ===")
    wait_http()
    ser = open_acm(dtr=False, rts=False)
    try:
        with client() as http:
            reset_on_open = False
            try:
                s = get_state(http)
                print(f"  HTTP survived open ups={s.get('ups')}")
            except Exception:
                reset_on_open = True
                print(
                    "  ACM open reset USB-Serial-JTAG (kernel DTR); "
                    "keeping port open through boot"
                )
            if reset_on_open:
                wait_http(timeout_s=50)
            wait_motor(http, UPS_MIN, timeout_s=90)
            nread = 0
            rows: list[dict] = []
            for i in range(6):
                nread += len(drain(ser))
                s = get_state(http)
                rec = {
                    "ups": s.get("ups") or 0,
                    "dt_max": float(s.get("dt_max_ms") or 0),
                    "dt_avg": float(s.get("dt_avg_ms") or 0),
                    "connected": bool(s.get("motor_connected")),
                }
                print(
                    f"    [{i}] ups={rec['ups']} dt_max={rec['dt_max']:.3f} usb={nread}"
                )
                if i > 0:
                    rows.append(rec)
                time.sleep(1.1)
            assert_healthy(rows, min_ups=UPS_MIN, label="ACM-open")
            log = serial_cmd(ser, "get pin.modbus_tx", "modbus_tx", 2.0)
            if "modbus_tx" not in log:
                fail(f"CLI silent while ACM open: {log[:200]!r}")
            print("  CLI get pin.modbus_tx ok")
    finally:
        ser.dtr = False
        ser.rts = False
        ser.close()
    print("✓ ups ACM-open ok")


def test_late_attach() -> None:
    print(f"=== late-attach CLI (ACM closed {LATE_IDLE_S:.0f}s) ===")
    try:
        ser = open_acm(dtr=False, rts=False)
        try:
            ser.write(b"reset\r\n")
            ser.flush()
            time.sleep(0.15)
        finally:
            ser.dtr = False
            ser.rts = False
            ser.close()
    except serial.SerialException as e:
        print(f"  (soft reset skipped: {e})")

    print(f"  ACM closed {LATE_IDLE_S:.0f}s...")
    time.sleep(LATE_IDLE_S)

    try:
        ser = open_acm(dtr=False, rts=False)
    except serial.SerialException as e:
        fail(f"could not open {PORT}: {e}")

    try:
        ser.reset_input_buffer()
        ser.write(f"{SET_CMD}\r\n".encode())
        ser.flush()
        log = wait_ack(ser, ACK_NEEDLE, ACK_TIMEOUT_S)
    except serial.SerialTimeoutException:
        fail("serial write timeout (device not reading USB OUT)")
    finally:
        ser.dtr = False
        ser.rts = False
        ser.close()

    if ACK_NEEDLE not in log:
        fail(
            f"no CLI ACK {ACK_NEEDLE!r} within {ACK_TIMEOUT_S:.0f}s "
            f"({len(log)} B: {log[:500]!r})"
        )
    print(f"✓ late-attach CLI ACK ok ({ACK_NEEDLE})")


def probe_cmd(*, scan_memory: bool = False) -> list[str]:
    if not PROBE_RS:
        fail("probe-rs not found (install probe-rs or set PROBE_RS)")
    if not ELF.is_file():
        fail(f"ELF not found at {ELF} (cargo b-c6 --release first)")
    cmd = [
        str(PROBE_RS),
        "attach",
        "--chip",
        CHIP,
        "--no-catch-reset",
    ]
    if scan_memory:
        cmd.append("--rtt-scan-memory")
    cmd.append(str(ELF))
    if os.geteuid() != 0 and shutil.which("sudo"):
        return ["sudo", "-n", *cmd]
    return cmd


def test_rtt() -> None:
    print("=== probe-rs RTT ===")
    wait_http()
    with client() as http:
        wait_motor(http, UPS_MIN)

    RTT_OUT.unlink(missing_ok=True)
    cmd = probe_cmd(scan_memory=False)
    print(" ", " ".join(cmd))
    with RTT_OUT.open("wb") as rtt_file:
        proc = subprocess.Popen(cmd, stdout=rtt_file, stderr=subprocess.PIPE)
        try:
            time.sleep(2.5)
            if proc.poll() is not None:
                err = proc.stderr.read().decode(errors="replace") if proc.stderr else ""
                fail(f"probe-rs attach exited early: {err[:800]}")
            with client() as http:
                time.sleep(2.0)
                rows = sample_loop(http, 5, skip_first=True)
                assert_healthy(rows, min_ups=UPS_MIN, label="during-RTT")
        finally:
            if proc.poll() is None:
                proc.send_signal(signal.SIGINT)
                try:
                    proc.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    proc.kill()

    log = RTT_OUT.read_text(encoding="utf-8", errors="replace")
    if "PANIC" in log:
        print(log[-800:])
        fail("RTT capture contains PANIC")
    if "ossm_rust" not in log and "Motor loop" not in log and "[console] ready" not in log:
        print(log[-800:])
        fail("RTT capture has no firmware logs")
    print(f"  RTT {len(log)} B")
    print("✓ probe-rs RTT ok")


def ensure_modbus_debug(enabled: bool) -> dict:
    wait_http()
    with client() as http:
        pins = http.get(f"{BASE}/pin-config").json()
        if bool(pins.get("modbus_debug")) == enabled:
            return pins
        print(f"  setting modbus_debug={enabled} and restarting...")
        http.post(f"{BASE}/pin-config", json={**pins, "modbus_debug": enabled})
        http.post(f"{BASE}/restart")
    time.sleep(5)
    wait_http(timeout_s=50)
    with client() as http:
        pins = http.get(f"{BASE}/pin-config").json()
        if bool(pins.get("modbus_debug")) != enabled:
            fail(f"modbus_debug did not become {enabled}")
        return pins


def test_fairness() -> None:
    print("=== CLI fairness under MODBUS_DBG ===")
    ensure_modbus_debug(True)
    ser = open_acm(dtr=False, rts=False)
    try:
        wait_http(timeout_s=50)
        with client() as http:
            wait_motor(http, UPS_MIN_DEBUG)
            cases = [("trailing", 3), ("leading", 3)]
            for mode, nbytes in cases:
                http.post(f"{BASE}/modbus-inject", json={"mode": "off", "nbytes": 0})
                http.post(f"{BASE}/paused", json={"paused": True})
                time.sleep(0.4)
                http.post(
                    f"{BASE}/modbus-inject", json={"mode": mode, "nbytes": nbytes}
                )
                got = http.get(f"{BASE}/modbus-inject").json()
                if got.get("mode") != mode or got.get("nbytes") != nbytes:
                    fail(f"{mode}: HTTP inject not applied ({got})")
                http.post(f"{BASE}/paused", json={"paused": False})
                time.sleep(1.2)
                wait_motor(http, UPS_MIN_DEBUG, timeout_s=25)
                needle = f"inject: {mode} {nbytes}"
                log = ""
                for _ in range(8):
                    if ser.in_waiting:
                        drain(ser)
                    log = serial_cmd(ser, "get inject", needle, timeout_s=6.0)
                    if needle in log:
                        break
                    time.sleep(0.4)
                if needle not in log:
                    fail(f"{mode}: serial CLI unresponsive under MODBUS_DBG flood")
                ups = get_state(http).get("ups") or 0
                if ups < UPS_MIN_DEBUG:
                    fail(f"{mode}: ups={ups} < {UPS_MIN_DEBUG} during flood")
                print(f"  {mode} n={nbytes}: serial ACK ok (ups={ups})")
                http.post(f"{BASE}/modbus-inject", json={"mode": "off", "nbytes": 0})
                http.post(f"{BASE}/paused", json={"paused": True})
                time.sleep(0.3)
    finally:
        ser.dtr = False
        ser.rts = False
        ser.close()
        try:
            wait_http(timeout_s=40)
            with client() as http:
                http.post(f"{BASE}/modbus-inject", json={"mode": "off", "nbytes": 0})
            ensure_modbus_debug(False)
        except Exception as e:
            print(f"  (cleanup skipped: {e})")
    print("✓ console fairness ok")


ALL = ("ups", "acm-open", "rtt", "late-attach", "fairness")


def main() -> None:
    parser = argparse.ArgumentParser(description="OSSM USB console live tests")
    parser.add_argument(
        "--only",
        default="",
        help="comma-separated subset: " + ",".join(ALL),
    )
    args = parser.parse_args()
    selected = [s.strip() for s in args.only.split(",") if s.strip()] or list(ALL)
    unknown = [s for s in selected if s not in ALL]
    if unknown:
        fail(f"unknown tests {unknown}; choose from {ALL}")

    print(f"Device {BASE} serial {PORT} tests={selected}")
    runners = {
        "ups": test_ups_closed,
        "acm-open": test_ups_acm_open,
        "rtt": test_rtt,
        "late-attach": test_late_attach,
        "fairness": test_fairness,
    }
    for name in selected:
        runners[name]()
    print("✓ test_console.py passed")


if __name__ == "__main__":
    main()
