#!/usr/bin/env -S uv run
"""
Post-flashing setup script for OSSM ESP32-C6 / ESP32-S3 firmware.
Configures WiFi credentials, Modbus GPIO pins, and initial motor parameters over USB serial.
Uses DeviceBackend from ossm.py for UART communication and environment loading.
"""

import argparse
import asyncio
import os
import sys
import time

# Ensure scripts/ directory is in path to import ossm
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ossm import DeviceBackend, load_env


def update_env_ip(ip_address):
    """Updates DEVICE_IP in .env file when a new IP is assigned."""
    env_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".env")
    if not os.path.exists(env_path):
        return
    lines = []
    updated = False
    with open(env_path, "r", encoding="utf-8") as f:
        for line in f:
            if line.strip().startswith("DEVICE_IP="):
                lines.append(f'DEVICE_IP="{ip_address}"\n')
                updated = True
            else:
                lines.append(line)
    if not updated:
        lines.append(f'\nDEVICE_IP="{ip_address}"\n')
    with open(env_path, "w", encoding="utf-8") as f:
        f.writelines(lines)
    print(f"\033[36mUpdated DEVICE_IP=\"{ip_address}\" in .env\033[0m")


async def main_async():
    load_env()

    parser = argparse.ArgumentParser(description="OSSM Post-Flashing Setup Tool")
    parser.add_argument(
        "--port",
        "-p",
        default=os.environ.get("DEVICE_PORT", "/dev/ttyACM0"),
        help="Serial port (default from .env or /dev/ttyACM0)",
    )
    parser.add_argument(
        "--baud",
        "-b",
        type=int,
        default=int(os.environ.get("DEVICE_BAUD", "115200")),
        help="Baud rate (default from .env or 115200)",
    )
    parser.add_argument(
        "--ssid",
        default=os.environ.get("WIFI_SSID", "LoudNet-2.4G"),
        help="WiFi SSID (default from .env)",
    )
    parser.add_argument(
        "--password",
        default=os.environ.get("WIFI_PASSWORD", "password"),
        help="WiFi Password (default from .env)",
    )
    parser.add_argument(
        "--tx", type=int, default=int(os.environ.get("MODBUS_TX", "2")), help="Modbus TX pin (default from .env)"
    )
    parser.add_argument(
        "--rx", type=int, default=int(os.environ.get("MODBUS_RX", "1")), help="Modbus RX pin (default from .env)"
    )
    parser.add_argument(
        "--dere", type=int, default=int(os.environ.get("MODBUS_DERE", "0")), help="Modbus DE/RE pin (default from .env)"
    )
    parser.add_argument(
        "--model",
        "-m",
        default=os.environ.get("DEVICE_MODEL", "esp32c6"),
        help="Target device chip model (default from .env or esp32c6)",
    )
    parser.add_argument(
        "--flash",
        action="store_true",
        help="Flash the release binary for the selected device model before configuring",
    )
    parser.add_argument(
        "--no-reset",
        action="store_true",
        help="Do not reset the device after sending configuration",
    )
    parser.add_argument(
        "--monitor",
        action="store_true",
        help="Stay in monitor mode after setup completes",
    )

    args = parser.parse_args()

    if args.flash:
        import subprocess
        bin_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "release", f"ossm-{args.model}.bin")
        print(f"\033[33m--- Flashing {args.model} firmware ({bin_path}) to {args.port} ---\033[0m")
        cmd = ["espflash", "write-bin", "--port", args.port, "--chip", args.model, "0x0", bin_path]
        res = subprocess.run(cmd)
        if res.returncode != 0:
            print(f"\033[31mError:\033[0m Flashing failed with exit code {res.returncode}", file=sys.stderr)
            sys.exit(1)
        time.sleep(2)  # Wait for device boot after flashing

    print(f"Connecting to {args.model} device on {args.port} at {args.baud} baud using DeviceBackend...")
    backend = DeviceBackend(mode="serial", ip="", port=args.port, baud=args.baud, ble_addr="")

    print(f"\n\033[33m--- Sending Device Configurations (SSID: {args.ssid}) ---\033[0m")
    commands = [
        f"set net.ssid {args.ssid}",
        f"set net.password {args.password}",
        f"set pin.modbus_tx {args.tx}",
        f"set pin.modbus_rx {args.rx}",
        f"set pin.modbus_de_re {args.dere}",
        "set pin.modbus_timeout_ms 0",
        "set pin.modbus_scan_delay_us 0",
        'set motor {"bpm":36.0,"depth":1.0,"depth_top":false,"reversed":false,"wave_func":"sine","sharpness":0.3,"spline_points":[0.0,1.0],"paused":false,"paused_position":0.0,"streaming":false}',
        "get pin",
    ]

    for cmd in commands:
        print(f"\033[36m>\033[0m {cmd}")
        lines = await backend._serial_command(cmd, wait_response=True)
        for line in lines:
            print(f"  {line}")

    if not args.no_reset:
        print("\n\033[32mConfiguration sent.\033[0m Resetting device to apply changes...")
        await backend.restart_device()
        await asyncio.sleep(0.5)

        print("\n\033[33m--- Waiting for Reboot & WiFi Connection ---\033[0m")
        ip_assigned = await backend.wait_for_wifi_ip(timeout=25)

        if ip_assigned:
            print(f"\n\033[32m========================================================\033[0m")
            print(f"\033[32m✓ Successfully configured and connected to WiFi!\033[0m")
            print(f"\033[32m  IP Address:    {ip_assigned}\033[0m")
            print(f"\033[32m  HTTP Endpoint: http://{ip_assigned}\033[0m")
            print(f"\033[32m  WS Endpoint:   ws://{ip_assigned}/ws/command\033[0m")
            print(f"\033[32m========================================================\033[0m")
            update_env_ip(ip_assigned)
        else:
            print("\n\033[33mNote: Did not detect an IP address assignment within 25 seconds.\033[0m")
            print("Check if the WiFi SSID and password are correct and within range.")

        if args.monitor:
            print("\n\033[36mEntering monitor mode (press Ctrl+C to exit)...\033[0m")
            await backend.monitor_serial()

    print("\nDone!")


def main():
    asyncio.run(main_async())


if __name__ == "__main__":
    main()
