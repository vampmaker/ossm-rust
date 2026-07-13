#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "websockets>=12.0",
#     "python-dotenv>=1.0.0",
#     "httpx2>=0.1.0",
#     "pyserial>=3.5",
#     "bleak>=0.21.0",
#     "typer>=0.12.0",
#     "rich>=13.7.0",
#     "pydantic>=2.0.0",
# ]
# ///
"""
Automated WebSocket JSON-RPC test script for OSSM firmware.
Tests ping, status, get-state, set-config, subscribe-state, unsubscribe-state, append-waypoints, and set-waypoints.
Uses DeviceBackend from ossm.py for environment loading and WebSocket connection management.
"""

import argparse
import asyncio
import json
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ossm import DeviceBackend, load_env


async def run_tests(backend: DeviceBackend):
    url = f"ws://{backend.ip}/ws/command"
    print(f"Connecting to {url} using DeviceBackend...")

    try:
        async with backend.ws_connect() as ws:
            print("\033[32mConnected to WebSocket!\033[0m")

            # 1. Test ping
            req = {"jsonrpc": "2.0", "method": "ping", "id": 1}
            print("\n\033[33m--- 1. Testing ping ---\033[0m")
            print("Send:", json.dumps(req))
            await ws.send(json.dumps(req))
            res = json.loads(await ws.recv())
            print("Recv:", json.dumps(res, indent=2))
            assert res.get("id") == 1 and res.get("result") == "pong", "Ping failed!"

            # 2. Test status
            req = {"jsonrpc": "2.0", "method": "status", "id": 2}
            print("\n\033[33m--- 2. Testing status ---\033[0m")
            print("Send:", json.dumps(req))
            await ws.send(json.dumps(req))
            res = json.loads(await ws.recv())
            print("Recv:", json.dumps(res, indent=2))
            assert res.get("id") == 2 and "buffered" in res.get("result", {}), "Status failed!"

            # 3. Test get-state
            req = {"jsonrpc": "2.0", "method": "get-state", "id": 3}
            print("\n\033[33m--- 3. Testing get-state ---\033[0m")
            print("Send:", json.dumps(req))
            await ws.send(json.dumps(req))
            res = json.loads(await ws.recv())
            print("Recv:", json.dumps(res, indent=2))
            assert res.get("id") == 3, "Get-state failed!"
            if "error" in res:
                print("Note: get-state returned error:", res["error"])
            else:
                assert "config" in res.get("result", {}), "Get-state missing config!"

            # 4. Test set-config
            req = {
                "jsonrpc": "2.0",
                "method": "set-config",
                "params": {
                    "bpm": 40.0,
                    "depth": 0.8,
                    "depth_top": False,
                    "reversed": False,
                    "wave_func": "sine",
                    "sharpness": 0.3,
                    "spline_points": [0.0, 1.0],
                    "paused": False,
                    "paused_position": 0.0,
                    "streaming": False
                },
                "id": 4
            }
            print("\n\033[33m--- 4. Testing set-config ---\033[0m")
            print("Send:", json.dumps(req))
            await ws.send(json.dumps(req))
            res = json.loads(await ws.recv())
            print("Recv:", json.dumps(res, indent=2))
            assert res.get("id") == 4, "Set-config failed!"

            # 5. Test subscribe-state
            req = {"jsonrpc": "2.0", "method": "subscribe-state", "params": {"interval_ms": 300}, "id": 5}
            print("\n\033[33m--- 5. Testing subscribe-state (300ms interval) ---\033[0m")
            print("Send:", json.dumps(req))
            await ws.send(json.dumps(req))
            res = json.loads(await ws.recv())
            print("Recv:", json.dumps(res, indent=2))
            result = res.get("result", {})
            assert res.get("id") == 5, "Subscribe-state failed!"
            assert result.get("subscribed") is True, f"Expected subscribed=true, got {result}"
            assert result.get("interval_ms") == 300, f"Expected interval_ms=300, got {result.get('interval_ms')}"

            print("Waiting for 2 pushed state notifications...")
            for i in range(2):
                push = json.loads(await ws.recv())
                print(f"Push {i+1}:", json.dumps(push, indent=2))
                assert push.get("method") == "state", f"Expected state notification, got {push}"
                assert "config" in push.get("params", {}), "Push state missing config in params!"

            # 6. Test unsubscribe-state
            req = {"jsonrpc": "2.0", "method": "unsubscribe-state", "id": 6}
            print("\n\033[33m--- 6. Testing unsubscribe-state ---\033[0m")
            print("Send:", json.dumps(req))
            await ws.send(json.dumps(req))
            res = json.loads(await ws.recv())
            while res.get("id") != 6:
                print("Received while waiting for ack:", res)
                res = json.loads(await ws.recv())
            print("Recv:", json.dumps(res, indent=2))
            assert res.get("id") == 6 and res.get("result") == "unsubscribed", "Unsubscribe failed!"

            # 7. Test append-waypoints
            req = {
                "jsonrpc": "2.0",
                "method": "append-waypoints",
                "params": [
                    {"ts": 1000, "pos": 0.5, "vel": 0.1},
                    {"ts": 1100, "pos": 0.8}
                ],
                "id": 7
            }
            print("\n\033[33m--- 7. Testing append-waypoints ---\033[0m")
            print("Send:", json.dumps(req))
            await ws.send(json.dumps(req))
            res = json.loads(await ws.recv())
            while res.get("id") != 7:
                print("Received while waiting for ack:", res)
                res = json.loads(await ws.recv())
            print("Recv:", json.dumps(res, indent=2))
            assert res.get("id") == 7 and res.get("result") == "ok", "append-waypoints failed!"

            # 8. Test set-waypoints (list style)
            req = {
                "jsonrpc": "2.0",
                "method": "set-waypoints",
                "params": [
                    {"ts": 2000, "pos": 0.2, "vel": 0.0}
                ],
                "id": 8
            }
            print("\n\033[33m--- 8. Testing set-waypoints (list style) ---\033[0m")
            print("Send:", json.dumps(req))
            await ws.send(json.dumps(req))
            res = json.loads(await ws.recv())
            while res.get("id") != 8:
                print("Received while waiting for ack:", res)
                res = json.loads(await ws.recv())
            print("Recv:", json.dumps(res, indent=2))
            assert res.get("id") == 8 and res.get("result") == "ok", "set-waypoints failed!"

            # 9. Test set-waypoints with reset-timestamp
            req = {
                "jsonrpc": "2.0",
                "method": "set-waypoints",
                "params": {
                    "waypoints": [
                        {"ts": 3000, "pos": 0.5, "vel": 0.1}
                    ],
                    "reset-timestamp": True
                },
                "id": 9
            }
            print("\n\033[33m--- 9. Testing set-waypoints with reset-timestamp ---\033[0m")
            print("Send:", json.dumps(req))
            await ws.send(json.dumps(req))
            res = json.loads(await ws.recv())
            while res.get("id") != 9:
                print("Received while waiting for ack:", res)
                res = json.loads(await ws.recv())
            print("Recv:", json.dumps(res, indent=2))
            assert res.get("id") == 9 and res.get("result") == "ok", "set-waypoints with reset-timestamp failed!"

        print("\n\033[32m==========================================\033[0m")
        print("\033[32m✓ All WebSocket JSON-RPC tests PASSED!\033[0m")
        print("\033[32m==========================================\033[0m")
    except Exception as e:
        print(f"\n\033[31mTest Failed:\033[0m {e}", file=sys.stderr)
        sys.exit(1)
    finally:
        await backend.disconnect()


def main():
    load_env()
    default_ip = os.environ.get("DEVICE_IP", "192.168.24.63")

    parser = argparse.ArgumentParser(description="OSSM WebSocket JSON-RPC Test Tool")
    parser.add_argument(
        "--ip",
        "-i",
        default=default_ip,
        help=f"Target device IP address (default from .env: {default_ip})",
    )
    args = parser.parse_args()

    backend = DeviceBackend(mode="wifi", ip=args.ip)
    asyncio.run(run_tests(backend))


if __name__ == "__main__":
    main()
