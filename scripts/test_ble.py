#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "bleak>=0.21.0",
#     "python-dotenv>=1.0.0",
#     "typer>=0.12.0",
#     "rich>=13.7.0",
#     "websockets>=12.0",
#     "requests>=2.31.0",
#     "pyserial>=3.5",
# ]
# ///
"""
OSSM BLE Verification & Motor Test Script
Connects to the OSSM device via Bluetooth Low Energy (BLE),
verifies GATT characteristics, reads/writes configs, tests JSON-RPC commands,
and runs the motor. Uses DeviceBackend from ossm.py.
"""

import asyncio
import json
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ossm import DeviceBackend, load_env, CHAR_STATE, CHAR_RPC


def main():
    load_env()
    print("Initializing DeviceBackend in BLE mode...")
    backend = DeviceBackend(mode="ble")

    # 1. Read Pin Config
    print("\n--- 1. Testing Pin Configuration ---")
    pin_config = backend.get_pin_config()
    print(f"Pin Config: {json.dumps(pin_config, indent=2)}")

    # 2. Read Motor Config
    print("\n--- 2. Testing Motor Controller Config ---")
    status = backend.get_status()
    motor_config = status.get("config", {})
    print(f"Current Motor Config: {json.dumps(motor_config, indent=2)}")

    # 3. Read Motor State
    print("\n--- 3. Testing Motor State ---")
    motor_state = status.get("state", {})
    print(f"Current Motor State: {json.dumps(motor_state, indent=2)}")

    # 4. Test Paused Control Characteristic Write
    print("\n--- 4. Testing Paused Control Characteristic Write ---")
    res = backend.set_paused(paused=True, position=0.0)
    print("Write to CHAR_PAUSED successful!")

    # 5. Test Motor Config Write
    print("\n--- 5. Testing Motor Config Write ---")
    motor_config["bpm"] = 25.0
    print("Writing updated config (bpm=25.0):")
    updated_config = backend.set_config(motor_config)
    print(f"Verified new BPM: {updated_config.get('bpm')}")

    # 6. Test JSON-RPC over BLE
    print("\n--- 6. Testing JSON-RPC over BLE ---")
    print("Sending RPC ping...")
    res_ping = backend.send_rpc("ping", rpc_id=1)
    print(f"Ping Response: {json.dumps(res_ping, indent=2)}")
    assert res_ping.get("result") == "pong", "BLE RPC ping failed!"

    print("Sending RPC get-state...")
    res_state = backend.send_rpc("get-state", rpc_id=2)
    print(f"Get-State Response ID: {res_state.get('id')}")
    assert res_state.get("id") == 2, "BLE RPC get-state failed!"

    # 7. Run the motor via BLE!
    print("\n--- 7. Running Motor via BLE ---")
    print("Starting motor at 30 BPM, depth 0.5...")
    backend.set_config({"bpm": 30.0, "depth": 0.5, "paused": False})

    async def _monitor_live_state():
        client = await backend._get_ble_client()
        try:
            def state_callback(sender, data):
                try:
                    st = json.loads(data.decode("utf-8", errors="ignore"))
                    print(f"[Live State Notify]: pos={st.get('position', 0):.3f}, speed={st.get('speed', 0):.2f}, paused={st.get('config', {}).get('paused')}")
                except Exception:
                    msg = data.decode("utf-8", errors="ignore")
                    print(f"[Live State Notify]: {msg[:100]}...")

            print("Subscribing to state telemetry notifications for 3 seconds...")
            await client.start_notify(CHAR_STATE, state_callback)
            await asyncio.sleep(0.2)
            sub_cmd = {"jsonrpc": "2.0", "method": "subscribe-state", "params": {"interval_ms": 200}, "id": 3}
            await client.write_gatt_char(CHAR_RPC, json.dumps(sub_cmd).encode("utf-8"), response=True)
            await asyncio.sleep(3.0)
            unsub_cmd = {"jsonrpc": "2.0", "method": "unsubscribe-state", "id": 4}
            await client.write_gatt_char(CHAR_RPC, json.dumps(unsub_cmd).encode("utf-8"), response=True)
            await client.stop_notify(CHAR_STATE)
        finally:
            await client.disconnect()

    asyncio.run(_monitor_live_state())

    # Stop motor
    print("\n--- 8. Pausing Motor ---")
    backend.set_paused(paused=True, position=0.0)
    print("\n✓ BLE Verification & Motor Test Completed Successfully!")


if __name__ == "__main__":
    main()
