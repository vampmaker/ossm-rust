#!/usr/bin/env -S uv run
"""Verify modbus_debug junk-skip + fault injection on a live device (RTT).

Enables `modbus_debug` (reboot) if needed, captures MODBUS_DBG over probe-rs RTT,
then disables debug and restarts on exit.

USB ACM is held open (DTR=0/RTS=0) and drained during inject so firmware LOG_LIVE
stays true — otherwise stalled USB silences the logger and RTT sees nothing.
"""

from __future__ import annotations

import os
import re
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path

import httpx
from dotenv import load_dotenv

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent
load_dotenv(REPO_ROOT / ".env")

sys.path.insert(0, str(SCRIPT_DIR))
from test_console import (  # noqa: E402
    BASE,
    CHIP,
    client,
    ensure_modbus_debug,
    open_acm,
    probe_cmd,
    recover_http_after_probe,
    wait_http,
    wait_motor,
    UPS_MIN_DEBUG,
)

RTT_OUT = Path(os.environ.get("RTT_OUT_MODBUS", "/tmp/ossm_rtt_modbus_debug.txt"))
DBG_RE = re.compile(
    r"MODBUS_DBG ok=(\w+) class=(\w+).*skip=(\d+) frame_len=(\d+)"
)


def set_inject(http: httpx.Client, mode: str, nbytes: int) -> dict:
    r = http.post(f"{BASE}/modbus-inject", json={"mode": mode, "nbytes": nbytes})
    r.raise_for_status()
    return r.json()


def drain_acm(ser, chunks: list[bytes], stop: threading.Event) -> None:
    while not stop.is_set():
        n = ser.in_waiting
        if n:
            chunks.append(ser.read(n))
        else:
            time.sleep(0.05)


def stop_probe(proc: subprocess.Popen | None) -> None:
    if proc is None or proc.poll() is not None:
        return
    proc.send_signal(signal.SIGINT)
    try:
        proc.wait(timeout=8)
    except subprocess.TimeoutExpired:
        proc.kill()


def restore_debug_off() -> None:
    try:
        wait_http(timeout_s=40)
        with client() as http:
            http.post(f"{BASE}/modbus-inject", json={"mode": "off", "nbytes": 0})
            http.post(f"{BASE}/paused", json={"paused": True, "position": 0.0})
        ensure_modbus_debug(False)
    except Exception as e:
        print(f"  (debug-off cleanup skipped: {e})")


def assert_dbg_hits(log: str) -> None:
    if "PANIC" in log:
        print(log[-800:])
        raise RuntimeError("capture contains PANIC")
    hits = DBG_RE.findall(log)
    if not hits:
        print(log[-1200:])
        raise RuntimeError("no MODBUS_DBG lines in RTT/ACM capture")

    trailing = [h for h in hits if h[1] == "long"]
    leading = [h for h in hits if h[1] == "long_resync"]
    for mode, nbytes, expect_class in (
        ("trailing", 3, "long"),
        ("leading", 3, "long_resync"),
    ):
        bucket = trailing if expect_class == "long" else leading
        if not bucket:
            print(log[-1200:])
            raise RuntimeError(f"{mode}: no class={expect_class} in capture")
        ok, cls, skip, flen = bucket[-1]
        print(f"  {mode} n={nbytes}: ok={ok} class={cls} skip={skip} frame_len={flen}")
        if ok != "true" or cls != expect_class:
            raise RuntimeError(f"{mode}: expected ok=true class={expect_class}")
        if expect_class == "long_resync" and skip != str(nbytes):
            raise RuntimeError(f"{mode}: expected skip={nbytes}")


def main() -> None:
    print(f"Device {BASE} chip={CHIP} (RTT MODBUS_DBG)")
    wait_http()
    proc: subprocess.Popen | None = None
    ser = None
    stop = threading.Event()
    acm_chunks: list[bytes] = []
    try:
        ensure_modbus_debug(True)
        with client() as http:
            wait_motor(http, UPS_MIN_DEBUG)

        RTT_OUT.unlink(missing_ok=True)
        cmd = probe_cmd(scan_memory=False)
        print(" ", " ".join(cmd))
        with RTT_OUT.open("wb") as rtt_file:
            proc = subprocess.Popen(cmd, stdout=rtt_file, stderr=subprocess.PIPE)
            time.sleep(2.5)
            if proc.poll() is not None:
                err = proc.stderr.read().decode(errors="replace") if proc.stderr else ""
                raise RuntimeError(f"probe-rs attach exited early: {err[:800]}")

            recover_http_after_probe(allow_reset=False)
            ser = open_acm(dtr=False, rts=False)
            reader = threading.Thread(
                target=drain_acm, args=(ser, acm_chunks, stop), daemon=True
            )
            reader.start()
            print("  ACM open (drain) so LOG_LIVE stays true")
            try:
                with client() as http:
                    wait_motor(http, UPS_MIN_DEBUG, timeout_s=50)
                    for mode, nbytes, _expect in (
                        ("trailing", 3, "long"),
                        ("leading", 3, "long_resync"),
                    ):
                        set_inject(http, "off", 0)
                        http.post(f"{BASE}/paused", json={"paused": True, "position": 0.0})
                        time.sleep(1.0)
                        set_inject(http, mode, nbytes)
                        time.sleep(0.3)
                        got = http.get(f"{BASE}/modbus-inject").json()
                        if got.get("mode") != mode or got.get("nbytes") != nbytes:
                            raise RuntimeError(f"{mode}: inject not applied ({got})")
                        http.post(f"{BASE}/paused", json={"paused": False, "position": 0.0})
                        time.sleep(8)
                        set_inject(http, "off", 0)
                        http.post(f"{BASE}/paused", json={"paused": True, "position": 0.0})
                        time.sleep(1.0)
                        print(f"  injected {mode} n={nbytes}")
            finally:
                stop.set()
                reader.join(timeout=2)
                if ser is not None:
                    ser.dtr = False
                    ser.rts = False
                    ser.close()
                    ser = None

            time.sleep(1.5)
            stop_probe(proc)
            proc = None
        recover_http_after_probe()

        rtt_log = RTT_OUT.read_text(encoding="utf-8", errors="replace")
        acm_log = b"".join(acm_chunks).decode("utf-8", errors="replace")
        log = rtt_log + "\n" + acm_log
        assert_dbg_hits(log)
        print("✓ modbus debug inject verification passed (RTT+ACM)")
    except Exception as e:
        print(f"FAIL: {e}")
        sys.exit(1)
    finally:
        stop.set()
        if ser is not None:
            try:
                ser.dtr = False
                ser.rts = False
                ser.close()
            except Exception:
                pass
        stop_probe(proc)
        recover_http_after_probe()
        restore_debug_off()


if __name__ == "__main__":
    main()
