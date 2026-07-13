#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "bleak>=0.21.0",
#     "python-dotenv>=1.0.0",
#     "typer>=0.12.0",
#     "rich>=13.7.0",
#     "websockets>=12.0",
#     "httpx2>=0.1.0",
#     "pyserial>=3.5",
#     "pydantic>=2.0.0",
# ]
# ///
"""
OSSM BLE Verification & Motor Test Script
Connects to the OSSM device via Bluetooth Low Energy (BLE),
verifies GATT characteristics, reads/writes configs, tests JSON-RPC commands,
and runs the motor. Uses DeviceBackend from ossm.py with persistent connection.
"""

import asyncio
import json
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ossm import DeviceBackend, BleChunkReassembler, load_env, CHAR_STATE, CHAR_RPC


async def run_tests():
    load_env()
    print("Initializing DeviceBackend in BLE mode...")
    backend = DeviceBackend(mode="ble")

    try:
        # 1. Read Pin Config
        print("\n--- 1. Testing Pin Configuration ---")
        pin_config = await backend.get_pin_config()
        print(f"Pin Config: {json.dumps(pin_config, indent=2)}")

        # 2. Read Motor Config & State
        print("\n--- 2. Testing Motor Controller Config & State ---")
        status = await backend.get_status()
        motor_config = status.get("config", {})
        motor_state = status.get("state", {})
        print(f"Motor Config: bpm={motor_config.get('bpm')}, depth={motor_config.get('depth')}, paused={motor_config.get('paused')}")
        print(f"Motor State: position={motor_state.get('position')}, ups={motor_state.get('ups')}, t={motor_state.get('t')}")
        assert "bpm" in motor_config, "Motor config missing bpm field!"
        assert "position" in motor_state, "Motor state missing position field (GATT state characteristic not populated)!"

        # 3. Test Paused Control Characteristic Write
        print("\n--- 3. Testing Paused Control Characteristic Write ---")
        res = await backend.set_paused(paused=True, position=0.0)
        print("Write to CHAR_PAUSED successful!")

        # 4. Test Motor Config Write
        print("\n--- 4. Testing Motor Config Write ---")
        motor_config["bpm"] = 25.0
        print("Writing updated config (bpm=25.0):")
        updated_config = await backend.set_config(motor_config)
        print(f"Verified new BPM: {updated_config.get('bpm')}")

        # 5. Test JSON-RPC over BLE
        print("\n--- 5. Testing JSON-RPC over BLE ---")
        print("Sending RPC ping...")
        res_ping = await backend.send_rpc("ping", rpc_id=1)
        print(f"Ping Response: {json.dumps(res_ping, indent=2)}")
        assert res_ping.get("result") == "pong", "BLE RPC ping failed!"

        print("Sending RPC get-state...")
        res_state = await backend.send_rpc("get-state", rpc_id=2)
        print(f"Get-State Response ID: {res_state.get('id')}")
        assert res_state.get("id") == 2, "BLE RPC get-state failed!"

        # 6. Run the motor via BLE!
        print("\n--- 6. Running Motor via BLE ---")
        print("Starting motor at 30 BPM, depth 0.5...")
        await backend.set_config({"bpm": 30.0, "depth": 0.5, "paused": False})

        async def _monitor_live_state():
            client = await backend._get_ble_client()
            notifications_received = []
            reassembler = BleChunkReassembler()

            def state_callback(sender, data):
                payload = reassembler.feed(bytes(data))
                if payload is None:
                    return
                try:
                    st = json.loads(payload.decode("utf-8", errors="ignore"))
                    notifications_received.append(st)
                    cfg = st.get("config", {})
                    print(f"  [Live State]: pos={st.get('position', 0):.3f}, speed={st.get('speed', 0):.2f}, paused={cfg.get('paused')}")
                except Exception as e:
                    msg = payload.decode("utf-8", errors="ignore")
                    print(f"  [Live State parse error]: {e} | {msg[:100]}...")

            print("Subscribing to state telemetry notifications (interval_ms=50) for 3 seconds...")
            await client.start_notify(CHAR_STATE, state_callback)
            await asyncio.sleep(0.2)
            sub_cmd = {"jsonrpc": "2.0", "method": "subscribe-state", "params": {"interval_ms": 50}, "id": 3}
            await client.write_gatt_char(CHAR_RPC, json.dumps(sub_cmd).encode("utf-8"), response=True)
            await asyncio.sleep(3.0)
            unsub_cmd = {"jsonrpc": "2.0", "method": "unsubscribe-state", "id": 4}
            await client.write_gatt_char(CHAR_RPC, json.dumps(unsub_cmd).encode("utf-8"), response=True)
            await asyncio.sleep(0.2)
            await client.stop_notify(CHAR_STATE)

            print(f"  Received {len(notifications_received)} state notifications in 3 seconds")
            assert len(notifications_received) >= 15, f"Expected >=15 fast state notifications, got {len(notifications_received)}"
            print(f"  OK: {len(notifications_received)} notifications received (avg rate: {len(notifications_received)/3.0:.1f} Hz)")
            first = notifications_received[0]
            assert "position" in first, f"Notification missing 'position': {list(first.keys())}"
            assert "config" in first, f"Notification missing 'config': {list(first.keys())}"
            print(f"  OK: Notification schema verified (has position, config)")

        await _monitor_live_state()

        # Stop motor
        print("\n--- 7. Pausing Motor ---")
        await backend.set_paused(paused=True, position=0.0)
        print("\n✓ BLE Verification & Motor Test Completed Successfully!")

    finally:
        await backend.disconnect()


def main():
    asyncio.run(run_tests())


if __name__ == "__main__":
    main()
