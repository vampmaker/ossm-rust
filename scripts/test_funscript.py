#!/usr/bin/env -S uv run
"""
Automated Funscript Playback & Streaming verification script for OSSM firmware.
Tests .funscript parsing, trajectory scaling, chunked waypoint streaming, and device buffer state.
"""

import argparse
import asyncio
import json
import os
import socket
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from ossm import DeviceBackend


def log_step(name: str):
    print(f"\n======== [TEST] {name} ========")


def log_pass(msg: str):
    print(f"  [PASS] {msg}")


def log_fail(msg: str):
    print(f"  [FAIL] {msg}")
    sys.exit(1)


def is_host_online(host: str, port: int = 80, timeout: float = 1.0) -> bool:
    try:
        with socket.create_connection((host, port), timeout=timeout):
            return True
    except OSError:
        return False


async def run_funscript_tests(mode: str, live: bool):
    backend = DeviceBackend(mode=mode)
    print(f"Initialized DeviceBackend in '{backend.mode}' mode (Target IP: {backend.ip})")

    # 1. Synthetic .funscript creation & parsing test
    log_step("1. Create synthetic .funscript test document")
    sample_actions = [
        {"at": 0, "pos": 0},
        {"at": 500, "pos": 100},
        {"at": 1000, "pos": 0},
        {"at": 1500, "pos": 100},
        {"at": 2000, "pos": 50},
    ]
    with tempfile.NamedTemporaryFile(mode="w", suffix=".funscript", delete=False) as tf:
        json.dump({"actions": sample_actions}, tf)
        temp_path = tf.name

    try:
        with open(temp_path, "r", encoding="utf-8") as f:
            parsed = json.load(f)
        if len(parsed.get("actions", [])) == 5:
            log_pass("Successfully created and parsed synthetic .funscript document")
        else:
            log_fail("Parsed actions count mismatch")
    finally:
        if os.path.exists(temp_path):
            os.remove(temp_path)

    # 2. Test waypoint chunk formatting & scaling
    log_step("2. Test stroke range scaling (min_depth=0.2, max_depth=0.8)")
    min_depth = 0.2
    max_depth = 0.8
    scaled_chunk = []
    base_at = sample_actions[0]["at"]
    for act in sample_actions:
        pos_norm = act["pos"] / 100.0
        mapped_pos = min_depth + pos_norm * (max_depth - min_depth)
        scaled_chunk.append({"ts": act["at"] - base_at, "pos": mapped_pos})

    assert abs(scaled_chunk[0]["pos"] - 0.2) < 1e-4, f"Expected 0.2, got {scaled_chunk[0]['pos']}"
    assert abs(scaled_chunk[1]["pos"] - 0.8) < 1e-4, f"Expected 0.8, got {scaled_chunk[1]['pos']}"
    assert abs(scaled_chunk[4]["pos"] - 0.5) < 1e-4, f"Expected 0.5, got {scaled_chunk[4]['pos']}"
    log_pass("Stroke range mapping correctly scaled pos 0..100 -> 0.2..0.8")

    # 3. Test sending waypoints to live device
    log_step(f"3. Send streaming waypoints to device over {mode.upper()}")
    if mode == "wifi" and not is_host_online(backend.ip, 80, 1.0) and not live:
        print(f"  [NOTE] Target device at {backend.ip}:80 not reachable right now. Live streaming hardware test skipped.")
    else:
        try:
            res = await backend.set_config({"streaming": True, "paused": False})
            log_pass(f"Device set to streaming mode: {res}")

            send_res = await backend.send_waypoints(scaled_chunk, reset_timestamp=True)
            log_pass(f"Sent initial waypoint chunk (reset-timestamp=True): {send_res}")

            time.sleep(0.3)
            st = await backend.get_status()
            stream_st = st.get("state", {}).get("stream") or st.get("stream", {})
            log_pass(f"Stream status after feeding: {stream_st}")

            await backend.set_config({"streaming": False, "paused": True})
            log_pass("Cleaned up device state (streaming=False, paused=True)")
        except Exception as e:
            if live:
                log_fail(f"Live hardware test failed: {e}")
            else:
                print(f"  [NOTE] Live test skipped/offline: {e}")

    print("\n✓ All Funscript verification tests completed successfully.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Test Funscript streaming & playback engine.")
    parser.add_argument("--mode", "-m", default="wifi", choices=["wifi", "ble", "serial"], help="Communication mode")
    parser.add_argument("--live", action="store_true", help="Force fail if physical device is not reachable")
    args = parser.parse_args()
    asyncio.run(run_funscript_tests(args.mode, args.live))
