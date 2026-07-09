#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "bleak>=0.21.0",
#     "websockets>=12.0",
#     "requests>=2.31.0",
#     "pyserial>=3.5",
#     "typer>=0.12.0",
#     "rich>=13.7.0",
#     "python-dotenv>=1.0.0",
#     "pydantic>=2.0.0",
# ]
# ///
"""
OSSM Dual-Mode (WiFi / BLE / Serial) Control CLI Tool
A self-contained uv single-file script to monitor and control the Open Source Sex Machine.
"""

import asyncio
import json
import os
import sys
import time
from enum import StrEnum
from typing import Optional, Any
from dotenv import load_dotenv
from pydantic import BaseModel, Field, ConfigDict
import typer
from rich.console import Console
from rich.table import Table
from rich.panel import Panel
from rich.live import Live
import requests
import serial

def load_env():
    """Loads environment variables from .env file in workspace root."""
    env_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".env")
    if os.path.exists(env_path):
        load_dotenv(env_path)
    else:
        load_dotenv()

load_env()

app = typer.Typer(
    name="ossm-cli",
    help="Control and monitor OSSM device via WiFi (HTTP/WS), Bluetooth (BLE), or USB Serial.",
    no_args_is_help=True,
)
console = Console()

# BLE UUIDs
class BleUUID(StrEnum):
    SERVICE_UUID = "6e400001-b5a3-f393-e0a9-e50e24dcca9e"
    CHAR_CONFIG = "6e400002-b5a3-f393-e0a9-e50e24dcca9e"
    CHAR_STATE = "6e400003-b5a3-f393-e0a9-e50e24dcca9e"
    CHAR_PAUSED = "6e400004-b5a3-f393-e0a9-e50e24dcca9e"
    CHAR_PIN_CONFIG = "6e400005-b5a3-f393-e0a9-e50e24dcca9e"
    CHAR_RPC = "6e400006-b5a3-f393-e0a9-e50e24dcca9e"
    CHAR_NETWORK_CONFIG = "6e400007-b5a3-f393-e0a9-e50e24dcca9e"


SERVICE_UUID = BleUUID.SERVICE_UUID
CHAR_CONFIG = BleUUID.CHAR_CONFIG
CHAR_STATE = BleUUID.CHAR_STATE
CHAR_PAUSED = BleUUID.CHAR_PAUSED
CHAR_PIN_CONFIG = BleUUID.CHAR_PIN_CONFIG
CHAR_RPC = BleUUID.CHAR_RPC
CHAR_NETWORK_CONFIG = BleUUID.CHAR_NETWORK_CONFIG


# --- Pydantic Request / Response Schemas ---

class MotorControllerConfig(BaseModel):
    """Motor controller configuration request/response payload (/config or CHAR_CONFIG)."""
    model_config = ConfigDict(extra="allow", populate_by_name=True)
    bpm: float = 36.0
    depth: float = 1.0
    depth_top: bool = False
    reversed: bool = False
    wave_func: str = "sine"
    sharpness: float = 0.3
    spline_points: list[float] = Field(default_factory=lambda: [0.0, 1.0])
    paused: bool = False
    paused_position: float = 0.0
    streaming: bool = False


class StreamStatus(BaseModel):
    """Streaming buffer status sub-object."""
    model_config = ConfigDict(extra="allow", populate_by_name=True)
    buffered: int = 0
    stream_time: float = 0.0
    underrun: bool = False


class StreamWaypoint(BaseModel):
    """Individual motion trajectory waypoint."""
    model_config = ConfigDict(extra="allow", populate_by_name=True)
    ts: int
    pos: float
    vel: Optional[float] = None


class StateResponse(BaseModel):
    """Motor telemetry state response payload (/state or CHAR_STATE)."""
    model_config = ConfigDict(extra="allow", populate_by_name=True)
    config: MotorControllerConfig
    t: float = 0.0
    x: float = 0.0
    y: float = 0.0
    shaped_y: float = 0.0
    position: float = 0.0
    speed: float = 0.0
    stream: StreamStatus


class StatusResponse(BaseModel):
    """Combined status response returned by get_status()."""
    model_config = ConfigDict(extra="allow", populate_by_name=True)
    state: StateResponse
    config: MotorControllerConfig


class PinConfiguration(BaseModel):
    """RS-485 Modbus GPIO pin configuration payload (/pin-config or CHAR_PIN_CONFIG)."""
    model_config = ConfigDict(extra="allow", populate_by_name=True)
    modbus_tx: int = 18
    modbus_rx: int = 19
    modbus_de_re: int = 20
    modbus_timeout_ms: int = 0
    modbus_scan_delay_us: int = 0
    ble_enabled: bool = True


class NetworkConfiguration(BaseModel):
    """WiFi, DHCP, static IP, and mDNS configuration payload (/network-config or CHAR_NETWORK_CONFIG)."""
    model_config = ConfigDict(extra="allow", populate_by_name=True)
    hostname: str = "ossm"
    dhcp_enabled: bool = True
    static_ip: str = ""
    static_mask: str = ""
    static_gateway: str = ""
    static_dns: str = ""


class PausedControl(BaseModel):
    """Immediate pause and park control payload (/paused or CHAR_PAUSED)."""
    model_config = ConfigDict(extra="allow", populate_by_name=True)
    paused: Optional[bool] = None
    position: Optional[float] = None
    adjust: Optional[float] = None


class SubscribeParams(BaseModel):
    """Parameters for JSON-RPC subscribe-state method."""
    model_config = ConfigDict(extra="allow", populate_by_name=True)
    interval_ms: int = 300


class WaypointsInput(BaseModel):
    """Payload for set-waypoints JSON-RPC command."""
    model_config = ConfigDict(extra="allow", populate_by_name=True)
    waypoints: list[StreamWaypoint]
    reset_timestamp: bool = Field(default=False, alias="reset-timestamp")


class RpcRequest(BaseModel):
    """JSON-RPC 2.0 command request payload (/ws/command or CHAR_RPC)."""
    model_config = ConfigDict(extra="allow", populate_by_name=True)
    jsonrpc: str = "2.0"
    method: str
    params: Optional[Any] = None
    id: Optional[int | str] = 1


class RpcError(BaseModel):
    """JSON-RPC 2.0 error object."""
    model_config = ConfigDict(extra="allow", populate_by_name=True)
    code: int
    message: str


class RpcResponse(BaseModel):
    """JSON-RPC 2.0 command response payload."""
    model_config = ConfigDict(extra="allow", populate_by_name=True)
    jsonrpc: str = "2.0"
    id: Optional[int | str] = None
    result: Optional[Any] = None
    error: Optional[RpcError] = None


class WsNotification(BaseModel):
    """WebSocket / BLE push notification payload."""
    model_config = ConfigDict(extra="allow", populate_by_name=True)
    jsonrpc: str = "2.0"
    method: str
    cmd: str
    params: Optional[Any] = None
    state: Optional[Any] = None


class DeviceBackend:
    """Helper class managing communication across WiFi, BLE, and Serial modes."""
    def __init__(self, mode: str, ip: str = "192.168.24.63", port: str = "/dev/ttyACM0", baud: int = 115200, ble_addr: str = ""):
        self.mode = mode.lower()
        self.ip = ip
        self.port = port
        self.baud = baud
        self.ble_addr = ble_addr

    # --- Serial Helpers ---
    def _get_serial(self):
        if not hasattr(self, "_s") or self._s is None or not self._s.is_open:
            self._s = serial.Serial(self.port, self.baud, timeout=0.1)
        return self._s

    def _serial_command(self, cmd: str, wait_response: bool = True) -> list[str]:
        try:
            s = self._get_serial()
            s.reset_input_buffer()
            s.write(f"{cmd}\r\n".encode("utf-8"))
            s.flush()
            lines = []
            if wait_response:
                start_time = time.time()
                while time.time() - start_time < 1.2:
                    while s.in_waiting:
                        line = s.readline().decode("utf-8", errors="ignore").strip()
                        if line and not (line in cmd or cmd in line):
                            lines.append(line)
                    text = "\n".join(lines)
                    if "ok" in text or ("{" in text and "}" in text):
                        break
                    time.sleep(0.01)
            return lines
        except Exception as e:
            console.print(f"[bold red]Serial Error:[/bold red] Could not send command '{cmd}' to {self.port}: {e}")
            sys.exit(1)

    # --- BLE Helpers ---
    async def _get_ble_client(self):
        from bleak import BleakScanner, BleakClient
        if self.ble_addr:
            client = BleakClient(self.ble_addr)
            await client.connect()
            return client
        console.print("[dim]Scanning for advertising OSSM BLE device...[/dim]")
        device = await BleakScanner.find_device_by_filter(
            lambda d, adv: d.name and "OSSM" in d.name,
            timeout=8.0
        )
        if not device:
            console.print("[bold red]BLE Error:[/bold red] No advertising OSSM device found.")
            sys.exit(1)
        console.print(f"[dim]Found OSSM device: [bold green]{device.name}[/bold green] [{device.address}][/dim]")
        self.ble_addr = device.address
        client = BleakClient(device.address)
        await client.connect()
        return client

    # --- Public Methods ---
    def get_status(self) -> dict:
        if self.mode == "wifi":
            try:
                state = requests.get(f"http://{self.ip}/state", timeout=3.0).json()
                config = requests.get(f"http://{self.ip}/config", timeout=3.0).json()
                return {"state": state, "config": config}
            except Exception as e:
                console.print(f"[bold red]WiFi Error:[/bold red] Failed to reach http://{self.ip}: {e}")
                sys.exit(1)
        elif self.mode == "ble":
            async def _ble_get():
                client = await self._get_ble_client()
                try:
                    state_raw = await client.read_gatt_char(BleUUID.CHAR_STATE)
                    config_raw = await client.read_gatt_char(BleUUID.CHAR_CONFIG)
                    return {
                        "state": json.loads(state_raw.decode("utf-8")),
                        "config": json.loads(config_raw.decode("utf-8"))
                    }
                finally:
                    await client.disconnect()
            return asyncio.run(_ble_get())
        elif self.mode == "serial":
            lines = self._serial_command("get-status")
            text = "\n".join(lines)
            if "{" in text and "}" in text:
                try:
                    return json.loads(text[text.find("{"):text.rfind("}")+1])
                except Exception:
                    pass
            lines = self._serial_command("get-motor-config")
            config = {}
            text = "\n".join(lines)
            if "{" in text and "}" in text:
                try:
                    config = json.loads(text[text.find("{"):text.rfind("}")+1])
                except Exception:
                    pass
            return {"config": config, "state": {"position": 0.0, "speed": 0.0, "config": config}}
        else:
            console.print(f"[bold red]Error:[/bold red] Unsupported mode '{self.mode}'")
            sys.exit(1)

    def set_config(self, config_dict: dict) -> dict:
        try:
            full_config = self.get_status().get("config", {})
        except Exception:
            full_config = {}
        full_config.update(config_dict)
        if self.mode == "wifi":
            try:
                res = requests.post(f"http://{self.ip}/config", json=full_config, timeout=3.0)
                return res.json()
            except Exception as e:
                console.print(f"[bold red]WiFi Error:[/bold red] Failed to post config to http://{self.ip}: {e}")
                sys.exit(1)
        elif self.mode == "ble":
            async def _ble_set():
                client = await self._get_ble_client()
                try:
                    payload = json.dumps(full_config).encode("utf-8")
                    await client.write_gatt_char(BleUUID.CHAR_CONFIG, payload, response=True)
                    res_raw = await client.read_gatt_char(BleUUID.CHAR_CONFIG)
                    return json.loads(res_raw.decode("utf-8"))
                finally:
                    await client.disconnect()
            return asyncio.run(_ble_set())
        elif self.mode == "serial":
            payload = json.dumps(full_config, separators=(',', ':'))
            self._serial_command(f"set-motor-config {payload}", wait_response=False)
            return full_config
        return {}

    def set_paused(self, paused: bool, position: Optional[float] = None) -> dict:
        payload = {"paused": paused}
        if position is not None:
            payload["paused_position"] = position
        if self.mode == "wifi":
            try:
                return requests.post(f"http://{self.ip}/paused", json=payload, timeout=3.0).json()
            except Exception as e:
                console.print(f"[bold red]WiFi Error:[/bold red] Failed to post paused state: {e}")
                sys.exit(1)
        elif self.mode == "ble":
            async def _ble_pause():
                client = await self._get_ble_client()
                try:
                    await client.write_gatt_char(BleUUID.CHAR_PAUSED, json.dumps(payload).encode("utf-8"), response=True)
                    res_raw = await client.read_gatt_char(BleUUID.CHAR_CONFIG)
                    return json.loads(res_raw.decode("utf-8"))
                finally:
                    await client.disconnect()
            return asyncio.run(_ble_pause())
        elif self.mode == "serial":
            curr = self.get_status().get("config", {})
            curr.update(payload)
            return self.set_config(curr)
        return {}

    def send_waypoints(self, waypoints: list[dict], reset_timestamp: bool = False) -> dict:
        if reset_timestamp:
            return self.send_rpc("set-waypoints", {"waypoints": waypoints, "reset-timestamp": True})
        else:
            return self.send_rpc("append-waypoints", waypoints)

    def get_pin_config(self) -> dict:
        if self.mode == "wifi":
            return requests.get(f"http://{self.ip}/pin-config", timeout=3.0).json()
        elif self.mode == "ble":
            async def _ble_get_pins():
                client = await self._get_ble_client()
                try:
                    raw = await client.read_gatt_char(BleUUID.CHAR_PIN_CONFIG)
                    return json.loads(raw.decode("utf-8"))
                finally:
                    await client.disconnect()
            return asyncio.run(_ble_get_pins())
        elif self.mode == "serial":
            lines = self._serial_command("get-pin-configuration")
            text = "\n".join(lines)
            if "{" in text and "}" in text:
                try:
                    return json.loads(text[text.find("{"):text.rfind("}")+1])
                except Exception:
                    pass
            return {}
        return {}

    def set_pin_config(self, pin_dict: dict) -> dict:
        if self.mode == "wifi":
            return requests.post(f"http://{self.ip}/pin-config", json=pin_dict, timeout=3.0).json()
        elif self.mode == "ble":
            async def _ble_set_pins():
                client = await self._get_ble_client()
                try:
                    await client.write_gatt_char(BleUUID.CHAR_PIN_CONFIG, json.dumps(pin_dict).encode("utf-8"), response=True)
                    raw = await client.read_gatt_char(BleUUID.CHAR_PIN_CONFIG)
                    return json.loads(raw.decode("utf-8"))
                finally:
                    await client.disconnect()
            return asyncio.run(_ble_set_pins())
        elif self.mode == "serial":
            if "modbus_tx" in pin_dict:
                self._serial_command(f"set-pin-modbus-tx {pin_dict['modbus_tx']}", wait_response=False)
            if "modbus_rx" in pin_dict:
                self._serial_command(f"set-pin-modbus-rx {pin_dict['modbus_rx']}", wait_response=False)
            if "modbus_de_re" in pin_dict:
                self._serial_command(f"set-pin-modbus-de-re {pin_dict['modbus_de_re']}", wait_response=False)
            if "modbus_timeout_ms" in pin_dict:
                self._serial_command(f"set-modbus-timeout-ms {pin_dict['modbus_timeout_ms']}", wait_response=False)
            if "modbus_scan_delay_us" in pin_dict:
                self._serial_command(f"set-modbus-scan-delay-us {pin_dict['modbus_scan_delay_us']}", wait_response=False)
            if "ble_enabled" in pin_dict:
                val = "true" if pin_dict["ble_enabled"] else "false"
                self._serial_command(f"set-ble-enabled {val}", wait_response=False)
            return self.get_pin_config()
        return {}

    def get_network_config(self) -> dict:
        if self.mode == "wifi":
            return requests.get(f"http://{self.ip}/network-config", timeout=3.0).json()
        elif self.mode == "ble":
            async def _ble_get_net():
                client = await self._get_ble_client()
                try:
                    raw = await client.read_gatt_char(BleUUID.CHAR_NETWORK_CONFIG)
                    return json.loads(raw.decode("utf-8"))
                finally:
                    await client.disconnect()
            return asyncio.run(_ble_get_net())
        elif self.mode == "serial":
            lines = self._serial_command("get-network-config")
            text = "\n".join(lines)
            if "{" in text and "}" in text:
                try:
                    return json.loads(text[text.find("{"):text.rfind("}")+1])
                except Exception:
                    pass
            return {}
        return {}

    def set_network_config(self, net_dict: dict) -> dict:
        if self.mode == "wifi":
            return requests.post(f"http://{self.ip}/network-config", json=net_dict, timeout=3.0).json()
        elif self.mode == "ble":
            async def _ble_set_net():
                client = await self._get_ble_client()
                try:
                    await client.write_gatt_char(BleUUID.CHAR_NETWORK_CONFIG, json.dumps(net_dict).encode("utf-8"), response=True)
                    raw = await client.read_gatt_char(BleUUID.CHAR_NETWORK_CONFIG)
                    return json.loads(raw.decode("utf-8"))
                finally:
                    await client.disconnect()
            return asyncio.run(_ble_set_net())
        elif self.mode == "serial":
            if "hostname" in net_dict:
                self._serial_command(f"set-hostname {net_dict['hostname']}", wait_response=False)
            if "dhcp_enabled" in net_dict:
                val = "true" if net_dict["dhcp_enabled"] else "false"
                self._serial_command(f"set-dhcp-enabled {val}", wait_response=False)
            if "static_ip" in net_dict:
                self._serial_command(f"set-static-ip {net_dict['static_ip']}", wait_response=False)
            if "static_mask" in net_dict:
                self._serial_command(f"set-static-mask {net_dict['static_mask']}", wait_response=False)
            if "static_gateway" in net_dict:
                self._serial_command(f"set-static-gateway {net_dict['static_gateway']}", wait_response=False)
            if "static_dns" in net_dict:
                self._serial_command(f"set-static-dns {net_dict['static_dns']}", wait_response=False)
            return self.get_network_config()
        return {}

    def restart_device(self):
        if self.mode == "wifi":
            requests.post(f"http://{self.ip}/restart", timeout=2.0)
        elif self.mode == "ble":
            async def _ble_restart():
                client = await self._get_ble_client()
                try:
                    cmd = {"jsonrpc": "2.0", "method": "restart", "id": 1}
                    await client.write_gatt_char(BleUUID.CHAR_RPC, json.dumps(cmd).encode("utf-8"), response=True)
                finally:
                    await client.disconnect()
            asyncio.run(_ble_restart())
        elif self.mode == "serial":
            self._serial_command("reset", wait_response=False)

    def ws_connect(self):
        import websockets.sync.client as ws_client
        return ws_client.connect(f"ws://{self.ip}/ws/command")

    def send_rpc(self, method: str, params: Optional[Any] = None, rpc_id: int = 1) -> dict:
        cmd = {"jsonrpc": "2.0", "method": method, "id": rpc_id}
        if params is not None:
            cmd["params"] = params
        if self.mode == "wifi":
            with self.ws_connect() as ws:
                ws.send(json.dumps(cmd))
                return json.loads(ws.recv())
        elif self.mode == "ble":
            if method in ("get-state", "get_state"):
                st = self.get_status()
                return {"jsonrpc": "2.0", "id": rpc_id, "result": st["state"]}
            if method == "status":
                st = self.get_status()
                return {"jsonrpc": "2.0", "id": rpc_id, "result": st}
            async def _ble_rpc():
                client = await self._get_ble_client()
                try:
                    res_data = []
                    def _cb(sender, data):
                        try:
                            msg = json.loads(data.decode("utf-8", errors="ignore"))
                            if msg.get("id") == rpc_id:
                                res_data.append(msg)
                        except Exception:
                            pass
                    await client.start_notify(BleUUID.CHAR_RPC, _cb)
                    await client.write_gatt_char(BleUUID.CHAR_RPC, json.dumps(cmd).encode("utf-8"), response=True)
                    for _ in range(25):
                        if res_data:
                            break
                        await asyncio.sleep(0.1)
                    await client.stop_notify(BleUUID.CHAR_RPC)
                    return res_data[0] if res_data else {}
                finally:
                    await client.disconnect()
            return asyncio.run(_ble_rpc())
        elif self.mode == "serial":
            if method in ("set-waypoints", "append-waypoints"):
                payload_str = json.dumps(params, separators=(',', ':'))
                self._serial_command(f"{method} {payload_str}", wait_response=True)
                return {"result": "ok"}
            elif method in ("get-state", "get_state"):
                lines = self._serial_command("get-state")
                text = "\n".join(lines)
                if "{" in text and "}" in text:
                    try:
                        return {"result": json.loads(text[text.find("{"):text.rfind("}")+1])}
                    except Exception:
                        pass
                return {}
            elif method == "status":
                return {"result": self.get_status()}
            elif method == "reset-timestamp":
                lines = self._serial_command("reset-timestamp")
                return {"result": "ok"}
            else:
                return {}
        return {}

    def wait_for_wifi_ip(self, timeout: int = 25) -> Optional[str]:
        if self.mode != "serial":
            return None
        import re
        start_time = time.time()
        buffer = ""
        try:
            with serial.Serial(self.port, self.baud, timeout=0.1) as s:
                while time.time() - start_time < timeout:
                    if s.in_waiting:
                        chunk = s.read(s.in_waiting).decode("utf-8", errors="replace")
                        buffer += chunk
                        while "\n" in buffer:
                            line, buffer = buffer.split("\n", 1)
                            line = line.strip()
                            if line:
                                print(f"  {line}")
                                ip_match = re.search(r"(?:sta ip:|got ip:|ip:\s*)([0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3})", line, re.IGNORECASE)
                                if ip_match:
                                    return ip_match.group(1)
                    else:
                        time.sleep(0.05)
        except Exception as e:
            console.print(f"[bold red]Serial Error:[/bold red] Could not monitor for IP: {e}")
        return None

    def monitor_serial(self):
        if self.mode != "serial":
            return
        try:
            with serial.Serial(self.port, self.baud, timeout=0.1) as s:
                while True:
                    if s.in_waiting:
                        chunk = s.read(s.in_waiting).decode("utf-8", errors="replace")
                        print(chunk, end="", flush=True)
                    time.sleep(0.05)
        except KeyboardInterrupt:
            print("\nExiting monitor mode.")
        except Exception as e:
            console.print(f"[bold red]Serial Error:[/bold red] Monitor failed: {e}")

def resolve_backend(ctx: typer.Context, mode_override: Optional[str]) -> DeviceBackend:
    backend: DeviceBackend = ctx.obj
    if mode_override is not None:
        return DeviceBackend(
            mode=mode_override,
            ip=backend.ip,
            port=backend.port,
            baud=backend.baud,
            ble_addr=backend.ble_addr,
        )
    return backend


# --- Global CLI Callback & Context ---
@app.callback()
def main(
    ctx: typer.Context,
    mode: str = typer.Option("wifi", "--mode", "-m", help="Communication mode: wifi, ble, or serial"),
    ip: str = typer.Option(os.environ.get("DEVICE_IP", "192.168.24.63"), "--ip", "-i", help="Target IP address for WiFi mode"),
    port: str = typer.Option(os.environ.get("DEVICE_PORT", "/dev/ttyACM0"), "--port", "-p", help="Serial port path for USB serial mode"),
    baud: int = typer.Option(int(os.environ.get("DEVICE_BAUD", "115200")), "--baud", "-b", help="Baud rate for serial mode"),
    ble_addr: str = typer.Option(os.environ.get("DEVICE_BLE_ADDR", ""), "--ble-addr", "-a", help="BLE device MAC/address (auto-scans if empty)"),
):
    ctx.obj = DeviceBackend(mode=mode, ip=ip, port=port, baud=baud, ble_addr=ble_addr)


# --- Commands ---
@app.command()
def status(
    ctx: typer.Context,
    mode: Optional[str] = typer.Option(None, "--mode", "-m", help="Override mode: wifi, ble, or serial"),
):
    """View current device status, motor coordinates, and configuration."""
    backend = resolve_backend(ctx, mode)
    with console.status(f"[bold cyan]Fetching device status via {backend.mode.upper()}..."):
        data = backend.get_status()

    config = data.get("config", {})
    state = data.get("state", {})

    table = Table(title=f"OSSM Device Status ({backend.mode.upper()})", show_header=True, header_style="bold magenta")
    table.add_column("Parameter", style="cyan", justify="right")
    table.add_column("Value", style="green")

    table.add_row("Operating State", "[bold red]PAUSED[/bold red]" if config.get("paused", True) else "[bold green]RUNNING[/bold green]")
    table.add_row("BPM (Speed)", f"{config.get('bpm', 0.0):.1f}")
    table.add_row("Stroke Depth", f"{config.get('depth', 0.0) * 100.0:.1f}%")
    table.add_row("Wave Function", f"{config.get('wave_func', 'sine').upper()}")
    table.add_row("Current Position", f"{state.get('position', 0.0):.3f}")
    table.add_row("Current Speed", f"{state.get('speed', 0.0):.2f}")
    table.add_row("Top Anchored", f"{config.get('depth_top', False)}")
    table.add_row("Reversed Stroke", f"{config.get('reversed', False)}")

    console.print(table)


@app.command()
def run(
    ctx: typer.Context,
    bpm: float = typer.Option(30.0, "--bpm", "-s", help="Speed in Beats Per Minute"),
    depth: float = typer.Option(0.8, "--depth", "-d", help="Stroke depth ratio (0.0 to 1.0)"),
    wave: str = typer.Option("sine", "--wave", "-w", help="Wave function: sine, triangle, square, spline"),
    reversed_stroke: bool = typer.Option(False, "--reversed", "-r", help="Reverse stroke trajectory"),
    mode: Optional[str] = typer.Option(None, "--mode", "-m", help="Override mode: wifi, ble, or serial"),
):
    """Start the motor with specified motion parameters."""
    backend = resolve_backend(ctx, mode)
    config_update = {
        "paused": False,
        "bpm": bpm,
        "depth": max(0.0, min(1.0, depth)),
        "wave_func": wave.lower(),
        "reversed": reversed_stroke,
    }
    with console.status(f"[bold green]Starting motor at {bpm} BPM via {backend.mode.upper()}..."):
        res = backend.set_config(config_update)
    console.print(Panel(f"[bold green]Motor Running![/bold green]\nBPM: [cyan]{res.get('bpm', bpm)}[/cyan] | Depth: [cyan]{res.get('depth', depth)*100:.1f}%[/cyan] | Wave: [cyan]{res.get('wave_func', wave)}[/cyan]", title="Action"))


@app.command()
def pause(
    ctx: typer.Context,
    position: Optional[float] = typer.Option(None, "--pos", "-pos", help="Target park position ratio (0.0=bottom, 1.0=top)"),
    mode: Optional[str] = typer.Option(None, "--mode", "-m", help="Override mode: wifi, ble, or serial"),
):
    """Pause the motor immediately or park at a fixed position."""
    backend = resolve_backend(ctx, mode)
    with console.status(f"[bold yellow]Pausing motor via {backend.mode.upper()}..."):
        res = backend.set_paused(paused=True, position=position)
    pos_str = f" at ratio {position}" if position is not None else ""
    console.print(f"[bold yellow]✓ Motor Paused{pos_str}.[/bold yellow]")


@app.command()
def resume(
    ctx: typer.Context,
    mode: Optional[str] = typer.Option(None, "--mode", "-m", help="Override mode: wifi, ble, or serial"),
):
    """Resume motor movement with current settings."""
    backend = resolve_backend(ctx, mode)
    with console.status(f"[bold green]Resuming motor via {backend.mode.upper()}..."):
        backend.set_paused(paused=False)
    console.print("[bold green]✓ Motor Resumed.[/bold green]")


@app.command()
def config(
    ctx: typer.Context,
    bpm: Optional[float] = typer.Option(None, "--bpm", help="Update BPM speed"),
    depth: Optional[float] = typer.Option(None, "--depth", help="Update stroke depth (0.0 to 1.0)"),
    wave: Optional[str] = typer.Option(None, "--wave", help="Update wave function (sine, triangle, square, spline)"),
    sharpness: Optional[float] = typer.Option(None, "--sharpness", help="Update curve sharpness"),
    reversed_stroke: Optional[bool] = typer.Option(None, "--reversed/--no-reversed", help="Toggle stroke reversal"),
    mode: Optional[str] = typer.Option(None, "--mode", "-m", help="Override mode: wifi, ble, or serial"),
):
    """View or update motor controller configuration."""
    backend = resolve_backend(ctx, mode)
    with console.status(f"[bold cyan]Reading configuration via {backend.mode.upper()}..."):
        curr_data = backend.get_status()
    curr_config = curr_data.get("config", {})

    changes = {}
    if bpm is not None: changes["bpm"] = bpm
    if depth is not None: changes["depth"] = depth
    if wave is not None: changes["wave_func"] = wave.lower()
    if sharpness is not None: changes["sharpness"] = sharpness
    if reversed_stroke is not None: changes["reversed"] = reversed_stroke

    if changes:
        with console.status(f"[bold cyan]Applying configuration changes..."):
            curr_config.update(changes)
            curr_config = backend.set_config(curr_config)
        console.print("[bold green]✓ Configuration Updated Successfully:[/bold green]")

    console.print_json(data=curr_config)


@app.command()
def pins(
    ctx: typer.Context,
    tx: Optional[int] = typer.Option(None, "--tx", help="Modbus UART TX GPIO Pin"),
    rx: Optional[int] = typer.Option(None, "--rx", help="Modbus UART RX GPIO Pin"),
    de_re: Optional[int] = typer.Option(None, "--de-re", help="Modbus UART DE/RE GPIO Pin"),
    ble_enabled: Optional[bool] = typer.Option(None, "--ble/--no-ble", help="Enable or disable Bluetooth Low Energy"),
    mode: Optional[str] = typer.Option(None, "--mode", "-m", help="Override mode: wifi, ble, or serial"),
):
    """View or update RS-485 Modbus pin configuration and BLE enable flag."""
    backend = resolve_backend(ctx, mode)
    with console.status(f"[bold cyan]Reading pin configuration via {backend.mode.upper()}..."):
        pin_cfg = backend.get_pin_config()

    changes = {}
    if tx is not None: changes["modbus_tx"] = tx
    if rx is not None: changes["modbus_rx"] = rx
    if de_re is not None: changes["modbus_de_re"] = de_re
    if ble_enabled is not None: changes["ble_enabled"] = ble_enabled

    if changes:
        with console.status(f"[bold cyan]Applying pin configuration updates..."):
            pin_cfg.update(changes)
            pin_cfg = backend.set_pin_config(pin_cfg)
        console.print("[bold green]✓ Pin Configuration Updated (Reboot required to apply hardware changes):[/bold green]")

    console.print_json(data=pin_cfg)


@app.command()
def net(
    ctx: typer.Context,
    hostname: Optional[str] = typer.Option(None, "--hostname", "-h", help="mDNS hostname"),
    dhcp: Optional[bool] = typer.Option(None, "--dhcp/--no-dhcp", help="Enable or disable DHCP"),
    ip: Optional[str] = typer.Option(None, "--ip", help="Static IP address"),
    mask: Optional[str] = typer.Option(None, "--mask", help="Subnet mask"),
    gateway: Optional[str] = typer.Option(None, "--gateway", help="Gateway address"),
    dns: Optional[str] = typer.Option(None, "--dns", help="DNS server address"),
    mode: Optional[str] = typer.Option(None, "--mode", "-m", help="Override mode: wifi, ble, or serial"),
):
    """View or configure Network and mDNS settings."""
    backend = resolve_backend(ctx, mode)
    with console.status(f"[bold cyan]Fetching current network configuration ({backend.mode.upper()})..."):
        net_cfg = backend.get_network_config()

    changes = {}
    if hostname is not None: changes["hostname"] = hostname
    if dhcp is not None: changes["dhcp_enabled"] = dhcp
    if ip is not None: changes["static_ip"] = ip
    if mask is not None: changes["static_mask"] = mask
    if gateway is not None: changes["static_gateway"] = gateway
    if dns is not None: changes["static_dns"] = dns

    if changes:
        with console.status(f"[bold cyan]Applying network configuration updates..."):
            net_cfg.update(changes)
            net_cfg = backend.set_network_config(net_cfg)
        console.print("[bold green]✓ Network Configuration Updated (Reboot required to apply):[/bold green]")

    console.print_json(data=net_cfg)


@app.command()
def wifi(
    ctx: typer.Context,
    ssid: str = typer.Option(..., "--ssid", help="WiFi network SSID"),
    password: str = typer.Option(..., "--password", help="WiFi network Password"),
    mode: Optional[str] = typer.Option(None, "--mode", "-m", help="Override mode: wifi, ble, or serial"),
):
    """Configure WiFi credentials (USB Serial mode recommended)."""
    backend = resolve_backend(ctx, mode)
    if backend.mode != "serial":
        console.print("[bold yellow]Warning:[/bold yellow] Configuring WiFi over WiFi/BLE may cause immediate disconnection.")
    console.print(f"[cyan]Setting WiFi SSID to [bold]{ssid}[/bold]...[/cyan]")
    if backend.mode == "serial":
        backend._serial_command(f"set-wifi-ssid {ssid}", wait_response=False)
        backend._serial_command(f"set-wifi-password {password}", wait_response=False)
        console.print("[bold green]✓ WiFi credentials saved to NVS over USB Serial![/bold green]")
        console.print("Run [cyan]ossm.py restart --mode serial[/cyan] to connect.")
    else:
        console.print("[bold red]Error:[/bold red] WiFi configuration currently supported via --mode serial.")


@app.command()
def monitor(
    ctx: typer.Context,
    duration: int = typer.Option(30, "--duration", "-t", help="Duration in seconds to monitor live telemetry"),
    mode: Optional[str] = typer.Option(None, "--mode", "-m", help="Override mode: wifi, ble, or serial"),
):
    """Monitor live motor coordinates and speed in real-time over WebSocket or BLE."""
    backend = resolve_backend(ctx, mode)
    console.print(f"[bold cyan]Starting real-time telemetry monitoring ({backend.mode.upper()}) for {duration}s...[/bold cyan]")
    console.print("Press [bold red]Ctrl+C[/bold red] to exit early.\n")

    if backend.mode == "wifi":
        import websockets
        async def _ws_monitor():
            uri = f"ws://{backend.ip}/ws/command"
            async with websockets.connect(uri) as ws:
                sub_cmd = {"jsonrpc": "2.0", "method": "subscribe-state", "params": {"interval_ms": 150}, "id": 1}
                await ws.send(json.dumps(sub_cmd))
                start_t = time.time()
                with Live(refresh_per_second=8) as live:
                    while time.time() - start_t < duration:
                        try:
                            msg = await asyncio.wait_for(ws.recv(), timeout=1.0)
                            data = json.loads(msg)
                            if "state" in data or "result" in data:
                                st = data.get("state") or (data.get("result") if isinstance(data.get("result"), dict) else {})
                                if "position" in st:
                                    tbl = Table(title="Live Motor Telemetry (WiFi)", show_header=True, header_style="bold green")
                                    tbl.add_column("Metric", style="cyan")
                                    tbl.add_column("Value", style="magenta")
                                    tbl.add_row("Position (mm)", f"{st.get('position', 0.0):.3f}")
                                    tbl.add_row("Speed (mm/s)", f"{st.get('speed', 0.0):.2f}")
                                    tbl.add_row("Trajectory y", f"{st.get('y', 0.0):.3f}")
                                    live.update(tbl)
                        except asyncio.TimeoutError:
                            pass
                unsub_cmd = {"jsonrpc": "2.0", "method": "unsubscribe-state", "id": 2}
                await ws.send(json.dumps(unsub_cmd))
        asyncio.run(_ws_monitor())

    elif backend.mode == "ble":
        async def _ble_monitor():
            client = await backend._get_ble_client()
            try:
                state_data = {"pos": 0.0, "speed": 0.0, "y": 0.0}
                def _cb(sender, data):
                    try:
                        st = json.loads(data.decode("utf-8", errors="ignore"))
                        state_data["pos"] = st.get("position", 0.0)
                        state_data["speed"] = st.get("speed", 0.0)
                        state_data["y"] = st.get("y", 0.0)
                    except Exception:
                        pass

                await client.start_notify(BleUUID.CHAR_STATE, _cb)
                sub_cmd = {"jsonrpc": "2.0", "method": "subscribe-state", "params": {"interval_ms": 150}, "id": 1}
                await client.write_gatt_char(BleUUID.CHAR_RPC, json.dumps(sub_cmd).encode("utf-8"), response=True)

                start_t = time.time()
                with Live(refresh_per_second=8) as live:
                    while time.time() - start_t < duration:
                        tbl = Table(title="Live Motor Telemetry (BLE)", show_header=True, header_style="bold green")
                        tbl.add_column("Metric", style="cyan")
                        tbl.add_column("Value", style="magenta")
                        tbl.add_row("Position (mm)", f"{state_data['pos']:.3f}")
                        tbl.add_row("Speed (mm/s)", f"{state_data['speed']:.2f}")
                        tbl.add_row("Trajectory y", f"{state_data['y']:.3f}")
                        live.update(tbl)
                        await asyncio.sleep(0.12)

                unsub_cmd = {"jsonrpc": "2.0", "method": "unsubscribe-state", "id": 2}
                await client.write_gatt_char(BleUUID.CHAR_RPC, json.dumps(unsub_cmd).encode("utf-8"), response=True)
                await client.stop_notify(BleUUID.CHAR_STATE)
            finally:
                await client.disconnect()
        asyncio.run(_ble_monitor())
    else:
        console.print("[bold red]Error:[/bold red] Monitor mode is supported in wifi and ble modes.")


@app.command()
def restart(
    ctx: typer.Context,
    mode: Optional[str] = typer.Option(None, "--mode", "-m", help="Override mode: wifi, ble, or serial"),
):
    """Reboot the target microcontroller."""
    backend = resolve_backend(ctx, mode)
    console.print(f"[bold red]Rebooting OSSM device via {backend.mode.upper()}...[/bold red]")
    backend.restart_device()
    console.print("[bold green]✓ Restart command issued.[/bold green]")


@app.command()
def play(
    ctx: typer.Context,
    script_path: str = typer.Argument(..., help="Path to .funscript JSON file"),
    speed: float = typer.Option(1.0, "--speed", "-s", help="Playback speed multiplier (e.g. 1.0)"),
    min_depth: float = typer.Option(0.0, "--min-depth", help="Minimum stroke depth limit (0.0 to 1.0)"),
    max_depth: float = typer.Option(1.0, "--max-depth", help="Maximum stroke depth limit (0.0 to 1.0)"),
    mode: Optional[str] = typer.Option(None, "--mode", "-m", help="Override mode: wifi, ble, or serial"),
):
    """Play a .funscript file with chunked waypoint streaming."""
    if not os.path.exists(script_path):
        console.print(f"[bold red]Error:[/bold red] File not found: {script_path}")
        raise typer.Exit(code=1)

    with open(script_path, "r", encoding="utf-8") as f:
        doc = json.load(f)

    actions = doc.get("actions", [])
    if not actions:
        console.print("[bold red]Error:[/bold red] Invalid or empty .funscript file (no actions found).")
        raise typer.Exit(code=1)

    actions.sort(key=lambda a: a.get("at", 0))
    backend = resolve_backend(ctx, mode)

    console.print(f"[bold green]Starting funscript playback ({len(actions)} actions) via {backend.mode.upper()}...[/bold green]")
    backend.set_config({
        "streaming": True,
        "paused": False,
        "bpm": 30.0,
        "depth": 1.0,
    })

    try:
        idx = 0
        total_actions = len(actions)
        t0 = time.time()
        base_at = actions[0].get("at", 0)

        # Send initial chunk
        chunk = []
        for _ in range(min(20, total_actions - idx)):
            act = actions[idx]
            idx += 1
            rel_ms = (act.get("at", 0) - base_at) / speed
            pos_norm = max(0.0, min(100.0, act.get("pos", 0))) / 100.0
            mapped_pos = min_depth + pos_norm * (max_depth - min_depth)
            chunk.append({"ts": int(rel_ms), "pos": mapped_pos})

        backend.send_waypoints(chunk, reset_timestamp=True)

        total_duration_sec = (actions[-1].get("at", 0) - base_at) / (1000.0 * speed)
        with Live(console=console, refresh_per_second=4) as live:
            while True:
                elapsed = time.time() - t0
                if elapsed >= total_duration_sec + 0.5 and idx >= total_actions:
                    break

                st = backend.get_status()
                stream_st = st.get("state", {}).get("stream", {})
                buffered = stream_st.get("buffered", 0)

                if buffered < 50 and idx < total_actions:
                    chunk = []
                    for _ in range(min(20, total_actions - idx)):
                        act = actions[idx]
                        idx += 1
                        rel_ms = (act.get("at", 0) - base_at) / speed
                        pos_norm = max(0.0, min(100.0, act.get("pos", 0))) / 100.0
                        mapped_pos = min_depth + pos_norm * (max_depth - min_depth)
                        chunk.append({"ts": int(rel_ms), "pos": mapped_pos})
                    backend.send_waypoints(chunk, reset_timestamp=False)

                table = Table(title=f"Funscript Playback ({os.path.basename(script_path)})")
                table.add_column("Metric", style="cyan", no_wrap=True)
                table.add_column("Value", style="bold green")
                table.add_row("Progress", f"{elapsed:.1f}s / {total_duration_sec:.1f}s")
                table.add_row("Sent Actions", f"{idx} / {total_actions}")
                table.add_row("Device Buffered", str(buffered))
                table.add_row("Speed Multiplier", f"{speed:.2f}x")
                live.update(table)
                time.sleep(0.2)

        console.print("[bold green]✓ Funscript playback completed cleanly.[/bold green]")
    except KeyboardInterrupt:
        console.print("\n[bold yellow]Playback interrupted by user.[/bold yellow]")
    finally:
        backend.set_config({"streaming": False, "paused": True})


if __name__ == "__main__":
    app()
