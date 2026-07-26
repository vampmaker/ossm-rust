#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "bleak>=0.21.0",
#     "websockets>=12.0",
#     "httpx2>=0.1.0",
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
import httpx2
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


class BleChunkReassembler:
    """Reassembles multi-packet BLE notifications using the [index, total, ...payload] protocol.

    Each BLE notification carries a 2-byte header:
      byte 0 = chunk index (0-based)
      byte 1 = total number of chunks
    Remaining bytes are the payload fragment. Fragments are concatenated
    in index order once all chunks arrive.
    """

    def __init__(self):
        self._chunks: dict[int, bytes] = {}
        self._total: int = 0

    def feed(self, data: bytes) -> bytes | None:
        """Feed a raw notification. Returns the reassembled payload when complete, else None."""
        if len(data) < 2:
            return data
        index = data[0]
        total = data[1]
        payload = data[2:]

        if total == 0:
            return data

        if total == 1 and index == 0:
            self._chunks.clear()
            self._total = 0
            return payload

        if total != self._total:
            self._chunks.clear()
            self._total = total

        self._chunks[index] = payload

        if len(self._chunks) >= self._total:
            result = b"".join(
                self._chunks[i] for i in range(self._total) if i in self._chunks
            )
            self._chunks.clear()
            self._total = 0
            return result

        return None


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
    motor_connected: bool = True
    modbus_stats: Optional[dict] = None


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
    modbus_rx_timeout_us: int = 0
    modbus_scan_delay_us: int = 0
    modbus_inter_frame_delay_us: int = 0
    ble_enabled: bool = True
    modbus_debug: bool = False
    operating_mode: str = "servo"


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
    """Helper class managing communication across WiFi, BLE, and Serial modes asynchronously."""

    def __init__(
        self,
        mode: str,
        ip: str = "192.168.24.63",
        port: str = "/dev/ttyACM0",
        baud: int = 115200,
        ble_addr: str = "",
    ):
        self.mode = mode.lower()
        self.ip = ip
        self.port = port
        self.baud = baud
        self.ble_addr = ble_addr
        self._ble_client = None
        self._http_client: Optional[httpx2.AsyncClient] = None
        self._config_version: int = 0

    def _track_version(self, config: dict):
        if isinstance(config, dict) and "version" in config:
            try:
                val = int(config["version"])
                if val > self._config_version:
                    self._config_version = val
            except (ValueError, TypeError):
                pass

    async def _get_http_client(self) -> httpx2.AsyncClient:
        if self._http_client is None or self._http_client.is_closed:
            self._http_client = httpx2.AsyncClient(timeout=4.0)
        return self._http_client

    # --- Serial Helpers ---
    DEFAULT_SERIAL_COMMAND_TIMEOUT = 10

    def _get_serial(self):
        if not hasattr(self, "_s") or self._s is None or not self._s.is_open:
            self._s = serial.Serial(
                self.port, self.baud, timeout=0.2, write_timeout=1.0
            )
            self._s.dtr = True
            self._s.rts = False
        return self._s

    async def _serial_command(
        self,
        cmd: str,
        wait_response: bool = True,
        timeout: float = DEFAULT_SERIAL_COMMAND_TIMEOUT,
    ) -> list[str]:
        for attempt in range(2):
            start_time = time.time()
            try:
                s = self._get_serial()
                s.write_timeout = timeout
                if time.time() - start_time >= timeout:
                    raise TimeoutError(f"Serial command timed out before send: '{cmd}'")
                s.reset_input_buffer()
                s.write(f"{cmd}\r\n".encode("utf-8"))
                s.flush()
                lines = []
                if wait_response:
                    while time.time() - start_time < timeout:
                        while s.in_waiting:
                            line = s.readline().decode("utf-8", errors="ignore").strip()
                            if line and not (line in cmd or cmd in line):
                                lines.append(line)
                        text = "\n".join(lines)
                        if any(
                            kw in text.lower()
                            for kw in (
                                "ok",
                                "restart to apply",
                                "saved",
                                "enabled",
                                "disabled",
                                "set to",
                                "resetting",
                                "hostname set",
                                "updated",
                                "saving",
                                "configuration",
                                "got ip",
                            )
                        ) or ("{" in text and "}" in text):
                            break
                        await asyncio.sleep(0.01)
                return lines
            except Exception as e:
                if hasattr(self, "_s") and self._s is not None:
                    try:
                        self._s.close()
                    except Exception:
                        pass
                    self._s = None
                if attempt == 0:
                    await asyncio.sleep(0.3)
                    continue
                console.print(
                    f"[bold red]Serial Error:[/bold red] Could not send command '{cmd}' to {self.port}: {e}"
                )
                sys.exit(1)

    # --- BLE Helpers ---
    async def _get_ble_client(self):
        from bleak import BleakScanner, BleakClient

        if (
            hasattr(self, "_ble_client")
            and self._ble_client
            and self._ble_client.is_connected
        ):
            return self._ble_client
        if (
            not self.ble_addr
            or not hasattr(self, "_ble_device")
            or not self._ble_device
        ):
            console.print("[dim]Scanning for advertising OSSM BLE device...[/dim]")
            device = await BleakScanner.find_device_by_filter(
                lambda d, adv: "OSSM" in (d.name or adv.local_name or ""), timeout=12.0
            )
            if not device:
                console.print(
                    "[bold red]BLE Error:[/bold red] No advertising OSSM device found."
                )
                sys.exit(1)
            console.print(
                f"[dim]Found OSSM device: [bold green]{device.name or 'OSSM'}[/bold green] [{device.address}][/dim]"
            )
            self.ble_addr = device.address
            self._ble_device = device

        for attempt in range(1, 5):
            try:
                # Always re-scan after a failed attempt — BlueZ device paths
                # go stale when the peripheral drops mid-connect.
                if attempt > 1 or not getattr(self, "_ble_device", None):
                    console.print(
                        "[dim]Re-scanning to refresh BLE device handle...[/dim]"
                    )
                    self._ble_device = None
                    self.ble_addr = None
                    device = await BleakScanner.find_device_by_filter(
                        lambda d, adv: "OSSM" in (d.name or adv.local_name or ""),
                        timeout=10.0,
                    )
                    if not device:
                        raise RuntimeError("OSSM BLE advertisement not found")
                    self._ble_device = device
                    self.ble_addr = device.address
                target = self._ble_device
                client = BleakClient(target, timeout=20.0)
                await client.connect()
                # Tiny settle so GATT discovery finishes before first read.
                await asyncio.sleep(0.25)
                self._ble_client = client
                return client
            except Exception as e:
                self._ble_client = None
                self._ble_device = None
                self.ble_addr = None
                if attempt == 4:
                    raise
                console.print(
                    f"[dim yellow]BLE connect attempt {attempt} failed ({e}), retrying...[/dim yellow]"
                )
                await asyncio.sleep(2.0 + attempt)

    async def _disconnect_ble(self):
        if self._ble_client:
            try:
                if self._ble_client.is_connected:
                    await self._ble_client.disconnect()
            except Exception:
                pass
            finally:
                self._ble_client = None
                # Give the peripheral time to return to advertising before the
                # next connect (BlueZ device path goes stale otherwise).
                await asyncio.sleep(1.0)
                self._ble_device = None
                self.ble_addr = None

    async def disconnect(self):
        """Disconnect any persistent BLE or HTTP connections."""
        if self.mode == "ble" and self._ble_client:
            try:
                await self._disconnect_ble()
            except Exception:
                pass
        if self._http_client and not self._http_client.is_closed:
            try:
                await self._http_client.aclose()
            except Exception:
                pass
        self._http_client = None

    # --- Public Methods ---
    async def get_status(self) -> dict:
        if self.mode == "wifi":
            try:
                client = await self._get_http_client()
                res = await client.get(f"http://{self.ip}/state")
                state = res.json()
                await asyncio.sleep(0.05)
                config = state.get("config")
                if not config:
                    res_cfg = await client.get(f"http://{self.ip}/config")
                    config = res_cfg.json()
                self._track_version(config)
                return {"state": state, "config": config}
            except Exception as e:
                console.print(
                    f"[bold red]WiFi Error:[/bold red] Failed to reach http://{self.ip}: {e}"
                )
                sys.exit(1)
        elif self.mode == "ble":
            client = await self._get_ble_client()
            state_raw = await client.read_gatt_char(BleUUID.CHAR_STATE)
            config_raw = await client.read_gatt_char(BleUUID.CHAR_CONFIG)
            config = json.loads(config_raw.decode("utf-8"))
            self._track_version(config)
            return {
                "state": json.loads(state_raw.decode("utf-8")),
                "config": config,
            }
        elif self.mode == "serial":
            lines = await self._serial_command("get-status")
            text = "\n".join(lines)
            if "{" in text and "}" in text:
                try:
                    res = json.loads(text[text.find("{") : text.rfind("}") + 1])
                    self._track_version(res.get("config"))
                    return res
                except Exception:
                    pass
            lines = await self._serial_command("get motor")
            config = {}
            text = "\n".join(lines)
            if "{" in text and "}" in text:
                try:
                    config = json.loads(text[text.find("{") : text.rfind("}") + 1])
                except Exception:
                    pass
            self._track_version(config)
            return {
                "config": config,
                "state": {"position": 0.0, "speed": 0.0, "config": config},
            }
        else:
            console.print(f"[bold red]Error:[/bold red] Unsupported mode '{self.mode}'")
            sys.exit(1)

    async def set_config(self, config_dict: dict) -> dict:
        try:
            full_config = (await self.get_status()).get("config", {})
        except Exception:
            full_config = {}
        full_config.update(config_dict)
        return await self.post_full_config(full_config)

    async def post_full_config(self, full_config: dict) -> dict:
        """Post a complete motor config without a prior get_status round-trip."""
        if isinstance(full_config, dict):
            try:
                curr_ver = int(full_config.get("version", 0))
            except (ValueError, TypeError):
                curr_ver = 0
            if curr_ver <= self._config_version:
                if self._config_version > 0 or "version" in full_config:
                    self._config_version += 1
                    full_config["version"] = self._config_version
            else:
                self._config_version = curr_ver

        if self.mode == "wifi":
            try:
                client = await self._get_http_client()
                res = await client.post(f"http://{self.ip}/config", json=full_config)
                ret = res.json()
                self._track_version(ret)
                return ret
            except Exception as e:
                console.print(
                    f"[bold red]WiFi Error:[/bold red] Failed to post config to http://{self.ip}: {e}"
                )
                sys.exit(1)
        elif self.mode == "ble":
            client = await self._get_ble_client()
            payload = json.dumps(full_config).encode("utf-8")
            await client.write_gatt_char(BleUUID.CHAR_CONFIG, payload, response=True)
            res_raw = await client.read_gatt_char(BleUUID.CHAR_CONFIG)
            ret = json.loads(res_raw.decode("utf-8"))
            self._track_version(ret)
            return ret
        elif self.mode == "serial":
            payload = json.dumps(full_config, separators=(",", ":"))
            await self._serial_command(
                f"set motor {payload}", wait_response=False
            )
            self._track_version(full_config)
            return full_config
        return {}

    async def set_paused(self, paused: bool, position: Optional[float] = None) -> dict:
        payload = {"paused": paused}
        if position is not None:
            payload["position"] = position
        if self.mode == "wifi":
            try:
                client = await self._get_http_client()
                res = await client.post(f"http://{self.ip}/paused", json=payload)
                return res.json()
            except Exception as e:
                console.print(
                    f"[bold red]WiFi Error:[/bold red] Failed to post paused state: {e}"
                )
                sys.exit(1)
        elif self.mode == "ble":
            client = await self._get_ble_client()
            # Canonical BLE/HTTP paused payload uses "position", not the
            # motor-config field name "paused_position".
            await client.write_gatt_char(
                BleUUID.CHAR_PAUSED, json.dumps(payload).encode("utf-8"), response=True
            )
            await asyncio.sleep(0.15)
            res_raw = await client.read_gatt_char(BleUUID.CHAR_CONFIG)
            if not res_raw:
                await asyncio.sleep(0.2)
                res_raw = await client.read_gatt_char(BleUUID.CHAR_CONFIG)
            return json.loads(res_raw.decode("utf-8"))
        elif self.mode == "serial":
            curr = (await self.get_status()).get("config", {})
            curr.update(payload)
            return await self.set_config(curr)
        return {}

    async def send_waypoints(
        self, waypoints: list[dict], reset_timestamp: bool = False
    ) -> dict:
        if reset_timestamp:
            return await self.send_rpc(
                "set-waypoints", {"waypoints": waypoints, "reset-timestamp": True}
            )
        else:
            return await self.send_rpc("append-waypoints", waypoints)

    async def get_pin_config(self) -> dict:
        if self.mode == "wifi":
            client = await self._get_http_client()
            res = await client.get(f"http://{self.ip}/pin-config")
            return res.json()
        elif self.mode == "ble":
            client = await self._get_ble_client()
            raw = await client.read_gatt_char(BleUUID.CHAR_PIN_CONFIG)
            return json.loads(raw.decode("utf-8"))
        elif self.mode == "serial":
            lines = await self._serial_command("get pin")
            text = "\n".join(lines)
            if "{" in text and "}" in text:
                try:
                    return json.loads(text[text.find("{") : text.rfind("}") + 1])
                except Exception:
                    pass
            return {}
        return {}

    async def set_pin_config(self, pin_dict: dict) -> dict:
        if self.mode == "wifi":
            client = await self._get_http_client()
            res = await client.post(f"http://{self.ip}/pin-config", json=pin_dict)
            return res.json()
        elif self.mode == "ble":
            client = await self._get_ble_client()
            await client.write_gatt_char(
                BleUUID.CHAR_PIN_CONFIG,
                json.dumps(pin_dict).encode("utf-8"),
                response=True,
            )
            raw = await client.read_gatt_char(BleUUID.CHAR_PIN_CONFIG)
            return json.loads(raw.decode("utf-8"))
        elif self.mode == "serial":
            pin_keys = {
                "modbus_tx": "pin.modbus_tx",
                "modbus_rx": "pin.modbus_rx",
                "modbus_de_re": "pin.modbus_de_re",
                "modbus_timeout_ms": "pin.modbus_timeout_ms",
                "modbus_rx_timeout_us": "pin.modbus_rx_timeout_us",
                "modbus_scan_delay_us": "pin.modbus_scan_delay_us",
                "modbus_inter_frame_delay_us": "pin.modbus_inter_frame_delay_us",
            }
            for field, path in pin_keys.items():
                if field in pin_dict:
                    await self._serial_command(
                        f"set {path} {pin_dict[field]}", wait_response=False
                    )
            if "ble_enabled" in pin_dict:
                val = "true" if pin_dict["ble_enabled"] else "false"
                await self._serial_command(
                    f"set pin.ble_enabled {val}", wait_response=False
                )
            if "modbus_debug" in pin_dict:
                val = "true" if pin_dict["modbus_debug"] else "false"
                await self._serial_command(
                    f"set pin.modbus_debug {val}", wait_response=False
                )
            if "operating_mode" in pin_dict:
                await self._serial_command(
                    f"set pin.operating_mode {pin_dict['operating_mode']}",
                    wait_response=False,
                )
            return await self.get_pin_config()
        return {}

    async def get_network_config(self) -> dict:
        if self.mode == "wifi":
            client = await self._get_http_client()
            res = await client.get(f"http://{self.ip}/network-config")
            return res.json()
        elif self.mode == "ble":
            client = await self._get_ble_client()
            raw = await client.read_gatt_char(BleUUID.CHAR_NETWORK_CONFIG)
            return json.loads(raw.decode("utf-8"))
        elif self.mode == "serial":
            lines = await self._serial_command("get net")
            text = "\n".join(lines)
            if "{" in text and "}" in text:
                try:
                    return json.loads(text[text.find("{") : text.rfind("}") + 1])
                except Exception:
                    pass
            return {}
        return {}

    async def set_network_config(self, net_dict: dict) -> dict:
        if self.mode == "wifi":
            client = await self._get_http_client()
            res = await client.post(f"http://{self.ip}/network-config", json=net_dict)
            return res.json()
        elif self.mode == "ble":
            client = await self._get_ble_client()
            await client.write_gatt_char(
                BleUUID.CHAR_NETWORK_CONFIG,
                json.dumps(net_dict).encode("utf-8"),
                response=True,
            )
            raw = await client.read_gatt_char(BleUUID.CHAR_NETWORK_CONFIG)
            return json.loads(raw.decode("utf-8"))
        elif self.mode == "serial":
            net_keys = {
                "hostname": "net.hostname",
                "static_ip": "net.static_ip",
                "static_mask": "net.static_mask",
                "static_gateway": "net.static_gateway",
                "static_dns": "net.static_dns",
                "ssid": "net.ssid",
                "password": "net.password",
            }
            for field, path in net_keys.items():
                if field in net_dict:
                    await self._serial_command(
                        f"set {path} {net_dict[field]}", wait_response=False
                    )
            if "dhcp_enabled" in net_dict:
                val = "true" if net_dict["dhcp_enabled"] else "false"
                await self._serial_command(
                    f"set net.dhcp_enabled {val}", wait_response=False
                )
            if "wifi_enabled" in net_dict:
                val = "true" if net_dict["wifi_enabled"] else "false"
                await self._serial_command(
                    f"set net.wifi_enabled {val}", wait_response=False
                )
            return await self.get_network_config()
        return {}

    async def restart_device(self):
        if self.mode == "wifi":
            try:
                client = await self._get_http_client()
                await client.post(f"http://{self.ip}/restart")
            except Exception:
                pass
        elif self.mode == "ble":
            client = await self._get_ble_client()
            cmd = {"jsonrpc": "2.0", "method": "restart", "id": 1}
            await client.write_gatt_char(
                BleUUID.CHAR_RPC, json.dumps(cmd).encode("utf-8"), response=True
            )
            self._ble_client = None
        elif self.mode == "serial":
            await self._serial_command("reset", wait_response=False)

    def ws_connect(self):
        import websockets

        return websockets.connect(f"ws://{self.ip}/ws/command", ping_interval=None)

    async def send_rpc(
        self, method: str, params: Optional[Any] = None, rpc_id: int = 1
    ) -> dict:
        cmd = {"jsonrpc": "2.0", "method": method, "id": rpc_id}
        if params is not None:
            cmd["params"] = params
        if self.mode == "wifi":
            import websockets

            async with websockets.connect(
                f"ws://{self.ip}/ws/command", ping_interval=None
            ) as ws:
                await ws.send(json.dumps(cmd))
                msg = await ws.recv()
                return json.loads(msg)
        elif self.mode == "ble":
            if method in ("get-state", "get_state"):
                st = await self.get_status()
                return {"jsonrpc": "2.0", "id": rpc_id, "result": st["state"]}
            if method == "status":
                st = await self.get_status()
                return {"jsonrpc": "2.0", "id": rpc_id, "result": st}
            if method in ("set-waypoints", "append-waypoints", "reset-timestamp"):
                client = await self._get_ble_client()
                await client.write_gatt_char(
                    BleUUID.CHAR_RPC, json.dumps(cmd).encode("utf-8"), response=True
                )
                return {"jsonrpc": "2.0", "id": rpc_id, "result": "ok"}
            client = await self._get_ble_client()
            res_data = []
            reassembler = BleChunkReassembler()

            def _cb(sender, data):
                payload = reassembler.feed(bytes(data))
                if payload is None:
                    return
                try:
                    msg = json.loads(payload.decode("utf-8", errors="ignore"))
                    if msg.get("id") == rpc_id:
                        res_data.append(msg)
                except Exception:
                    pass

            await client.start_notify(BleUUID.CHAR_RPC, _cb)
            await client.write_gatt_char(
                BleUUID.CHAR_RPC, json.dumps(cmd).encode("utf-8"), response=True
            )
            for _ in range(25):
                if res_data:
                    break
                await asyncio.sleep(0.1)
            await client.stop_notify(BleUUID.CHAR_RPC)
            return res_data[0] if res_data else {}
        elif self.mode == "serial":
            if method in ("set-waypoints", "append-waypoints"):
                payload_str = json.dumps(params, separators=(",", ":"))
                await self._serial_command(
                    f"{method} {payload_str}", wait_response=True
                )
                return {"result": "ok"}
            elif method in ("get-state", "get_state"):
                lines = await self._serial_command("get-state")
                text = "\n".join(lines)
                if "{" in text and "}" in text:
                    try:
                        return {
                            "result": json.loads(
                                text[text.find("{") : text.rfind("}") + 1]
                            )
                        }
                    except Exception:
                        pass
                return {}
            elif method == "status":
                return {"result": await self.get_status()}
            elif method == "reset-timestamp":
                lines = await self._serial_command("reset-timestamp")
                return {"result": "ok"}
            else:
                return {}
        return {}

    async def wait_for_wifi_ip(self, timeout: int = 25) -> Optional[str]:
        if self.mode != "serial":
            return None
        import re

        start_time = time.time()
        buffer = ""
        try:
            with serial.Serial(
                self.port, self.baud, timeout=0.1, write_timeout=1.0
            ) as s:
                while time.time() - start_time < timeout:
                    if s.in_waiting:
                        chunk = s.read(s.in_waiting).decode("utf-8", errors="replace")
                        buffer += chunk
                        while "\n" in buffer:
                            line, buffer = buffer.split("\n", 1)
                            line = line.strip()
                            if line:
                                print(f"  {line}")
                                ip_match = re.search(
                                    r"(?:sta ip:|got ip:|ip:\s*)([0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3})",
                                    line,
                                    re.IGNORECASE,
                                )
                                if ip_match:
                                    return ip_match.group(1)
                    else:
                        await asyncio.sleep(0.05)
        except Exception as e:
            console.print(
                f"[bold red]Serial Error:[/bold red] Could not monitor for IP: {e}"
            )
        return None

    async def monitor_serial(self, timeout: Optional[float] = None):
        if self.mode != "serial":
            return
        try:
            with serial.Serial(
                self.port, self.baud, timeout=0.1, write_timeout=1.0
            ) as s:
                start_time = time.time()
                while timeout is None or time.time() - start_time < timeout:
                    if s.in_waiting:
                        chunk = s.read(s.in_waiting).decode("utf-8", errors="replace")
                        print(chunk, end="", flush=True)
                    await asyncio.sleep(0.05)
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
    mode: str = typer.Option(
        "wifi", "--mode", "-m", help="Communication mode: wifi, ble, or serial"
    ),
    ip: str = typer.Option(
        os.environ.get("DEVICE_IP", "192.168.24.63"),
        "--ip",
        "-i",
        help="Target IP address for WiFi mode",
    ),
    port: str = typer.Option(
        os.environ.get("DEVICE_PORT", "/dev/ttyACM0"),
        "--port",
        "-p",
        help="Serial port path for USB serial mode",
    ),
    baud: int = typer.Option(
        int(os.environ.get("DEVICE_BAUD", "115200")),
        "--baud",
        "-b",
        help="Baud rate for serial mode",
    ),
    ble_addr: str = typer.Option(
        os.environ.get("DEVICE_BLE_ADDR", ""),
        "--ble-addr",
        "-a",
        help="BLE device MAC/address (auto-scans if empty)",
    ),
):
    ctx.obj = DeviceBackend(mode=mode, ip=ip, port=port, baud=baud, ble_addr=ble_addr)


# --- Commands ---
@app.command()
def status(
    ctx: typer.Context,
    mode: Optional[str] = typer.Option(
        None, "--mode", "-m", help="Override mode: wifi, ble, or serial"
    ),
):
    """View current device status, motor coordinates, and configuration."""
    backend = resolve_backend(ctx, mode)
    with console.status(
        f"[bold cyan]Fetching device status via {backend.mode.upper()}..."
    ):
        data = asyncio.run(backend.get_status())

    config = data.get("config", {})
    state = data.get("state", {})

    table = Table(
        title=f"OSSM Device Status ({backend.mode.upper()})",
        show_header=True,
        header_style="bold magenta",
    )
    table.add_column("Parameter", style="cyan", justify="right")
    table.add_column("Value", style="green")

    table.add_row(
        "Operating State",
        (
            "[bold red]PAUSED[/bold red]"
            if config.get("paused", True)
            else "[bold green]RUNNING[/bold green]"
        ),
    )
    table.add_row(
        "Motor Connected",
        (
            "[bold green]YES[/bold green]"
            if state.get("motor_connected", True)
            else "[bold red]NO[/bold red]"
        ),
    )
    if "modbus_stats" in state and state["modbus_stats"]:
        mst = state["modbus_stats"]
        sr = mst.get("success_rate", 0.0)
        table.add_row(
            "Modbus Success Rate",
            f"{sr:.1f}% ({mst.get('successful_requests', 0)} ok / {mst.get('failed_requests', 0)} fail)",
        )
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
    depth: float = typer.Option(
        0.8, "--depth", "-d", help="Stroke depth ratio (0.0 to 1.0)"
    ),
    wave: str = typer.Option(
        "sine", "--wave", "-w", help="Wave function: sine, triangle, square, spline"
    ),
    reversed_stroke: bool = typer.Option(
        False, "--reversed", "-r", help="Reverse stroke trajectory"
    ),
    mode: Optional[str] = typer.Option(
        None, "--mode", "-m", help="Override mode: wifi, ble, or serial"
    ),
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
    with console.status(
        f"[bold green]Starting motor at {bpm} BPM via {backend.mode.upper()}..."
    ):
        res = asyncio.run(backend.set_config(config_update))
    console.print(
        Panel(
            f"[bold green]Motor Running![/bold green]\nBPM: [cyan]{res.get('bpm', bpm)}[/cyan] | Depth: [cyan]{res.get('depth', depth)*100:.1f}%[/cyan] | Wave: [cyan]{res.get('wave_func', wave)}[/cyan]",
            title="Action",
        )
    )


@app.command()
def pause(
    ctx: typer.Context,
    position: Optional[float] = typer.Option(
        None, "--pos", "-pos", help="Target park position ratio (0.0=bottom, 1.0=top)"
    ),
    mode: Optional[str] = typer.Option(
        None, "--mode", "-m", help="Override mode: wifi, ble, or serial"
    ),
):
    """Pause the motor immediately or park at a fixed position."""
    backend = resolve_backend(ctx, mode)
    with console.status(f"[bold yellow]Pausing motor via {backend.mode.upper()}..."):
        res = asyncio.run(backend.set_paused(paused=True, position=position))
    pos_str = f" at ratio {position}" if position is not None else ""
    console.print(f"[bold yellow]✓ Motor Paused{pos_str}.[/bold yellow]")


@app.command()
def resume(
    ctx: typer.Context,
    mode: Optional[str] = typer.Option(
        None, "--mode", "-m", help="Override mode: wifi, ble, or serial"
    ),
):
    """Resume motor movement with current settings."""
    backend = resolve_backend(ctx, mode)
    with console.status(f"[bold green]Resuming motor via {backend.mode.upper()}..."):
        asyncio.run(backend.set_paused(paused=False))
    console.print("[bold green]✓ Motor Resumed.[/bold green]")


@app.command()
def config(
    ctx: typer.Context,
    bpm: Optional[float] = typer.Option(None, "--bpm", help="Update BPM speed"),
    depth: Optional[float] = typer.Option(
        None, "--depth", help="Update stroke depth (0.0 to 1.0)"
    ),
    wave: Optional[str] = typer.Option(
        None, "--wave", help="Update wave function (sine, triangle, square, spline)"
    ),
    sharpness: Optional[float] = typer.Option(
        None, "--sharpness", help="Update curve sharpness"
    ),
    reversed_stroke: Optional[bool] = typer.Option(
        None, "--reversed/--no-reversed", help="Toggle stroke reversal"
    ),
    mode: Optional[str] = typer.Option(
        None, "--mode", "-m", help="Override mode: wifi, ble, or serial"
    ),
):
    """View or update motor controller configuration."""
    backend = resolve_backend(ctx, mode)
    with console.status(
        f"[bold cyan]Reading configuration via {backend.mode.upper()}..."
    ):
        curr_data = asyncio.run(backend.get_status())
    curr_config = curr_data.get("config", {})

    changes = {}
    if bpm is not None:
        changes["bpm"] = bpm
    if depth is not None:
        changes["depth"] = depth
    if wave is not None:
        changes["wave_func"] = wave.lower()
    if sharpness is not None:
        changes["sharpness"] = sharpness
    if reversed_stroke is not None:
        changes["reversed"] = reversed_stroke

    if changes:
        with console.status(f"[bold cyan]Applying configuration changes..."):
            curr_config.update(changes)
            curr_config = asyncio.run(backend.set_config(curr_config))
        console.print("[bold green]✓ Configuration Updated Successfully:[/bold green]")

    console.print_json(data=curr_config)


@app.command()
def pins(
    ctx: typer.Context,
    tx: Optional[int] = typer.Option(None, "--tx", help="Modbus UART TX GPIO Pin"),
    rx: Optional[int] = typer.Option(None, "--rx", help="Modbus UART RX GPIO Pin"),
    de_re: Optional[int] = typer.Option(
        None, "--de-re", help="Modbus UART DE/RE GPIO Pin"
    ),
    ble_enabled: Optional[bool] = typer.Option(
        None, "--ble/--no-ble", help="Enable or disable Bluetooth Low Energy"
    ),
    modbus_debug: Optional[bool] = typer.Option(
        None,
        "--modbus-debug/--no-modbus-debug",
        help="Modbus RX debug mode (5ms deadline + MODBUS_DBG class/base64; 256B DMA; reboot required)",
    ),
    operating_mode: Optional[str] = typer.Option(
        None,
        "--operating-mode",
        help="Device operating mode: servo or rtu_relay (restart required)",
    ),
    mode: Optional[str] = typer.Option(
        None, "--mode", "-m", help="Override mode: wifi, ble, or serial"
    ),
):
    """View or update RS-485 Modbus pin configuration and BLE enable flag."""
    backend = resolve_backend(ctx, mode)
    with console.status(
        f"[bold cyan]Reading pin configuration via {backend.mode.upper()}..."
    ):
        pin_cfg = asyncio.run(backend.get_pin_config())

    changes = {}
    if tx is not None:
        changes["modbus_tx"] = tx
    if rx is not None:
        changes["modbus_rx"] = rx
    if de_re is not None:
        changes["modbus_de_re"] = de_re
    if ble_enabled is not None:
        changes["ble_enabled"] = ble_enabled
    if modbus_debug is not None:
        changes["modbus_debug"] = modbus_debug
    if operating_mode is not None:
        if operating_mode not in ("servo", "rtu_relay"):
            console.print(
                "[bold red]operating_mode must be 'servo' or 'rtu_relay'[/bold red]"
            )
            raise typer.Exit(1)
        changes["operating_mode"] = operating_mode

    if changes:
        with console.status(f"[bold cyan]Applying pin configuration updates..."):
            pin_cfg.update(changes)
            pin_cfg = asyncio.run(backend.set_pin_config(pin_cfg))
        console.print(
            "[bold green]✓ Pin Configuration Updated (Reboot required to apply hardware changes):[/bold green]"
        )

    console.print_json(data=pin_cfg)


@app.command()
def net(
    ctx: typer.Context,
    hostname: Optional[str] = typer.Option(
        None, "--hostname", "-h", help="mDNS hostname"
    ),
    dhcp: Optional[bool] = typer.Option(
        None, "--dhcp/--no-dhcp", help="Enable or disable DHCP"
    ),
    ip: Optional[str] = typer.Option(None, "--ip", help="Static IP address"),
    mask: Optional[str] = typer.Option(None, "--mask", help="Subnet mask"),
    gateway: Optional[str] = typer.Option(None, "--gateway", help="Gateway address"),
    dns: Optional[str] = typer.Option(None, "--dns", help="DNS server address"),
    mode: Optional[str] = typer.Option(
        None, "--mode", "-m", help="Override mode: wifi, ble, or serial"
    ),
):
    """View or configure Network and mDNS settings."""
    backend = resolve_backend(ctx, mode)
    with console.status(
        f"[bold cyan]Fetching current network configuration ({backend.mode.upper()})..."
    ):
        net_cfg = asyncio.run(backend.get_network_config())

    changes = {}
    if hostname is not None:
        changes["hostname"] = hostname
    if dhcp is not None:
        changes["dhcp_enabled"] = dhcp
    if ip is not None:
        changes["static_ip"] = ip
    if mask is not None:
        changes["static_mask"] = mask
    if gateway is not None:
        changes["static_gateway"] = gateway
    if dns is not None:
        changes["static_dns"] = dns

    if changes:
        with console.status(f"[bold cyan]Applying network configuration updates..."):
            net_cfg.update(changes)
            net_cfg = asyncio.run(backend.set_network_config(net_cfg))
        console.print(
            "[bold green]✓ Network Configuration Updated (Reboot required to apply):[/bold green]"
        )

    console.print_json(data=net_cfg)


@app.command()
def wifi(
    ctx: typer.Context,
    ssid: str = typer.Option(..., "--ssid", help="WiFi network SSID"),
    password: str = typer.Option(..., "--password", help="WiFi network Password"),
    mode: Optional[str] = typer.Option(
        None, "--mode", "-m", help="Override mode: wifi, ble, or serial"
    ),
):
    """Configure WiFi credentials (USB Serial mode recommended)."""
    backend = resolve_backend(ctx, mode)
    if backend.mode != "serial":
        console.print(
            "[bold yellow]Warning:[/bold yellow] Configuring WiFi over WiFi/BLE may cause immediate disconnection."
        )
    console.print(f"[cyan]Setting WiFi SSID to [bold]{ssid}[/bold]...[/cyan]")
    if backend.mode == "serial":
        asyncio.run(
            backend._serial_command(f"set net.ssid {ssid}", wait_response=False)
        )
        asyncio.run(
            backend._serial_command(
                f"set net.password {password}", wait_response=False
            )
        )
        console.print(
            "[bold green]✓ WiFi credentials saved to NVS over USB Serial![/bold green]"
        )
        console.print("Run [cyan]ossm.py restart --mode serial[/cyan] to connect.")
    else:
        console.print(
            "[bold red]Error:[/bold red] WiFi configuration currently supported via --mode serial."
        )


@app.command()
def monitor(
    ctx: typer.Context,
    duration: int = typer.Option(
        30, "--duration", "-t", help="Duration in seconds to monitor live telemetry"
    ),
    mode: Optional[str] = typer.Option(
        None, "--mode", "-m", help="Override mode: wifi, ble, or serial"
    ),
):
    """Monitor live motor coordinates and speed in real-time over WebSocket or BLE."""
    backend = resolve_backend(ctx, mode)
    console.print(
        f"[bold cyan]Starting real-time telemetry monitoring ({backend.mode.upper()}) for {duration}s...[/bold cyan]"
    )
    console.print("Press [bold red]Ctrl+C[/bold red] to exit early.\n")

    if backend.mode == "wifi":
        import websockets

        async def _ws_monitor():
            uri = f"ws://{backend.ip}/ws/command"
            async with websockets.connect(uri, ping_interval=None) as ws:
                sub_cmd = {
                    "jsonrpc": "2.0",
                    "method": "subscribe-state",
                    "params": {"interval_ms": 150},
                    "id": 1,
                }
                await ws.send(json.dumps(sub_cmd))
                start_t = time.time()
                with Live(refresh_per_second=8) as live:
                    while time.time() - start_t < duration:
                        try:
                            msg = await asyncio.wait_for(ws.recv(), timeout=1.0)
                            data = json.loads(msg)
                            if "state" in data or "result" in data:
                                st = data.get("state") or (
                                    data.get("result")
                                    if isinstance(data.get("result"), dict)
                                    else {}
                                )
                                if "position" in st:
                                    tbl = Table(
                                        title="Live Motor Telemetry (WiFi)",
                                        show_header=True,
                                        header_style="bold green",
                                    )
                                    tbl.add_column("Metric", style="cyan")
                                    tbl.add_column("Value", style="magenta")
                                    tbl.add_row(
                                        "Position (mm)",
                                        f"{st.get('position', 0.0):.3f}",
                                    )
                                    tbl.add_row(
                                        "Speed (mm/s)", f"{st.get('speed', 0.0):.2f}"
                                    )
                                    tbl.add_row(
                                        "Trajectory y", f"{st.get('y', 0.0):.3f}"
                                    )
                                    live.update(tbl)
                        except asyncio.TimeoutError:
                            pass
                unsub_cmd = {"jsonrpc": "2.0", "method": "unsubscribe-state", "id": 2}
                await ws.send(json.dumps(unsub_cmd))

        asyncio.run(_ws_monitor())

    elif backend.mode == "ble":

        async def _ble_monitor():
            client = await backend._get_ble_client()
            state_data = {"pos": 0.0, "speed": 0.0, "y": 0.0}
            reassembler = BleChunkReassembler()

            def _cb(sender, data):
                payload = reassembler.feed(bytes(data))
                if payload is None:
                    return
                try:
                    st = json.loads(payload.decode("utf-8", errors="ignore"))
                    state_data["pos"] = st.get("position", 0.0)
                    state_data["speed"] = st.get("speed", 0.0)
                    state_data["y"] = st.get("y", 0.0)
                except Exception:
                    pass

            await client.start_notify(BleUUID.CHAR_STATE, _cb)
            sub_cmd = {
                "jsonrpc": "2.0",
                "method": "subscribe-state",
                "params": {"interval_ms": 150},
                "id": 1,
            }
            await client.write_gatt_char(
                BleUUID.CHAR_RPC, json.dumps(sub_cmd).encode("utf-8"), response=True
            )

            start_t = time.time()
            with Live(refresh_per_second=8) as live:
                while time.time() - start_t < duration:
                    tbl = Table(
                        title="Live Motor Telemetry (BLE)",
                        show_header=True,
                        header_style="bold green",
                    )
                    tbl.add_column("Metric", style="cyan")
                    tbl.add_column("Value", style="magenta")
                    tbl.add_row("Position (mm)", f"{state_data['pos']:.3f}")
                    tbl.add_row("Speed (mm/s)", f"{state_data['speed']:.2f}")
                    tbl.add_row("Trajectory y", f"{state_data['y']:.3f}")
                    live.update(tbl)
                    await asyncio.sleep(0.12)

            unsub_cmd = {"jsonrpc": "2.0", "method": "unsubscribe-state", "id": 2}
            await client.write_gatt_char(
                BleUUID.CHAR_RPC, json.dumps(unsub_cmd).encode("utf-8"), response=True
            )
            await client.stop_notify(BleUUID.CHAR_STATE)

        asyncio.run(_ble_monitor())
    else:
        console.print(
            "[bold red]Error:[/bold red] Monitor mode is supported in wifi and ble modes."
        )


@app.command()
def restart(
    ctx: typer.Context,
    mode: Optional[str] = typer.Option(
        None, "--mode", "-m", help="Override mode: wifi, ble, or serial"
    ),
):
    """Reboot the target microcontroller."""
    backend = resolve_backend(ctx, mode)
    console.print(
        f"[bold red]Rebooting OSSM device via {backend.mode.upper()}...[/bold red]"
    )
    asyncio.run(backend.restart_device())
    console.print("[bold green]✓ Restart command issued.[/bold green]")


@app.command()
def play(
    ctx: typer.Context,
    script_path: str = typer.Argument(..., help="Path to .funscript JSON file"),
    speed: float = typer.Option(
        1.0, "--speed", "-s", help="Playback speed multiplier (e.g. 1.0)"
    ),
    min_depth: float = typer.Option(
        0.0, "--min-depth", help="Minimum stroke depth limit (0.0 to 1.0)"
    ),
    max_depth: float = typer.Option(
        1.0, "--max-depth", help="Maximum stroke depth limit (0.0 to 1.0)"
    ),
    mode: Optional[str] = typer.Option(
        None, "--mode", "-m", help="Override mode: wifi, ble, or serial"
    ),
):
    """Play a .funscript file with chunked waypoint streaming."""
    if not os.path.exists(script_path):
        console.print(f"[bold red]Error:[/bold red] File not found: {script_path}")
        raise typer.Exit(code=1)

    with open(script_path, "r", encoding="utf-8") as f:
        doc = json.load(f)

    actions = doc.get("actions", [])
    if not actions:
        console.print(
            "[bold red]Error:[/bold red] Invalid or empty .funscript file (no actions found)."
        )
        raise typer.Exit(code=1)

    actions.sort(key=lambda a: a.get("at", 0))
    backend = resolve_backend(ctx, mode)

    async def _run_play():
        console.print(
            f"[bold green]Starting funscript playback ({len(actions)} actions) via {backend.mode.upper()}...[/bold green]"
        )
        await backend.set_config(
            {
                "streaming": True,
                "paused": False,
                "bpm": 30.0,
                "depth": 1.0,
            }
        )

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

            await backend.send_waypoints(chunk, reset_timestamp=True)

            total_duration_sec = (actions[-1].get("at", 0) - base_at) / (1000.0 * speed)
            with Live(console=console, refresh_per_second=4) as live:
                while True:
                    elapsed = time.time() - t0
                    if elapsed >= total_duration_sec + 0.5 and idx >= total_actions:
                        break

                    st = await backend.get_status()
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
                        await backend.send_waypoints(chunk, reset_timestamp=False)

                    table = Table(
                        title=f"Funscript Playback ({os.path.basename(script_path)})"
                    )
                    table.add_column("Metric", style="cyan", no_wrap=True)
                    table.add_column("Value", style="bold green")
                    table.add_row(
                        "Progress", f"{elapsed:.1f}s / {total_duration_sec:.1f}s"
                    )
                    table.add_row("Sent Actions", f"{idx} / {total_actions}")
                    table.add_row("Device Buffered", str(buffered))
                    table.add_row("Speed Multiplier", f"{speed:.2f}x")
                    live.update(table)
                    await asyncio.sleep(0.2)

            console.print(
                "[bold green]✓ Funscript playback completed cleanly.[/bold green]"
            )
        except KeyboardInterrupt:
            console.print("\n[bold yellow]Playback interrupted by user.[/bold yellow]")
        finally:
            await backend.set_config({"streaming": False, "paused": True})

    asyncio.run(_run_play())


EXAMPLE_MACRO_JSON = {
    "version": 1,
    "name": "Warm up",
    "loop": False,
    "instructions": [
        {
            "at": 0,
            "action": "set",
            "params": {"bpm": 40, "depth": 0.6, "wave_func": "sine"},
        },
        {"at": 0, "action": "start"},
        {
            "at": 15000,
            "action": "set",
            "params": {"bpm": 90, "sharpness": 0.2, "wave_func": "thrust"},
        },
        {
            "at": 30000,
            "action": "set",
            "params": {
                "wave_func": "spline",
                "spline_points": [0, 1, 0.3, 0.8],
            },
        },
        {"at": 60000, "action": "stop", "params": {"position": 0.0}},
    ],
}


def _normalize_macro(doc: dict) -> tuple[dict, list[str]]:
    """Validate and normalize a macro document. Returns (doc, errors)."""
    errors: list[str] = []
    if not isinstance(doc, dict):
        return {}, ["Root must be an object"]
    name = doc.get("name") if isinstance(doc.get("name"), str) and doc["name"].strip() else "Untitled"
    loop = bool(doc.get("loop", False))
    version = int(doc.get("version", 1)) if isinstance(doc.get("version"), (int, float)) else 1
    raw_inst = doc.get("instructions")
    if not isinstance(raw_inst, list):
        return {}, ["Missing instructions array"]

    valid_actions = {"start", "stop", "set"}
    valid_waves = {"sine", "thrust", "spline"}
    instructions: list[dict] = []
    for idx, item in enumerate(raw_inst):
        if not isinstance(item, dict):
            errors.append(f"instructions[{idx}] must be an object")
            continue
        at = item.get("at")
        if not isinstance(at, (int, float)) or at < 0:
            errors.append(f"instructions[{idx}].at must be a non-negative number")
            continue
        action = str(item.get("action", ""))
        if action not in valid_actions:
            errors.append(f"instructions[{idx}].action must be start|stop|set")
            continue
        params = item.get("params") if isinstance(item.get("params"), dict) else {}
        inst: dict[str, Any] = {"at": int(round(at)), "action": action}
        if action == "set":
            out: dict[str, Any] = {}
            if "bpm" in params:
                out["bpm"] = max(10.0, min(300.0, float(params["bpm"])))
            if "depth" in params:
                out["depth"] = max(0.0, min(1.0, float(params["depth"])))
            if "depth_top" in params:
                out["depth_top"] = bool(params["depth_top"])
            if "reversed" in params:
                out["reversed"] = bool(params["reversed"])
            if "wave_func" in params:
                w = str(params["wave_func"])
                if w in valid_waves:
                    out["wave_func"] = w
                else:
                    errors.append(
                        f"instructions[{idx}].params.wave_func must be sine|thrust|spline"
                    )
            if "sharpness" in params:
                out["sharpness"] = max(0.0, min(1.0, float(params["sharpness"])))
            if "spline_points" in params:
                pts = params["spline_points"]
                if isinstance(pts, list) and len(pts) >= 2:
                    out["spline_points"] = [
                        max(0.0, min(1.0, float(p))) for p in pts
                    ]
                else:
                    errors.append(
                        f"instructions[{idx}].params.spline_points needs >= 2 numbers"
                    )
            inst["params"] = out
        elif action == "stop" and "position" in params:
            inst["params"] = {
                "position": max(0.0, min(1.0, float(params["position"])))
            }
        instructions.append(inst)

    instructions.sort(key=lambda i: i["at"])
    return (
        {"version": version, "name": name, "loop": loop, "instructions": instructions},
        errors,
    )


@app.command()
def macro(
    ctx: typer.Context,
    script_path: Optional[str] = typer.Argument(
        None, help="Path to macro JSON file (omit with --init)"
    ),
    init: Optional[str] = typer.Option(
        None,
        "--init",
        help="Write an example macro JSON to this path and exit",
    ),
    loop: bool = typer.Option(
        False, "--loop", help="Loop playback after the last instruction"
    ),
    speed: float = typer.Option(
        1.0, "--speed", "-s", help="Playback speed multiplier (e.g. 1.0)"
    ),
    mode: Optional[str] = typer.Option(
        None, "--mode", "-m", help="Override mode: wifi, ble, or serial"
    ),
):
    """Play a timestamped control macro (start/stop/set-config sequence)."""
    if init:
        out_path = init
        with open(out_path, "w", encoding="utf-8") as f:
            json.dump(EXAMPLE_MACRO_JSON, f, indent=2)
            f.write("\n")
        console.print(
            f"[bold green]✓ Wrote example macro to {out_path}[/bold green]"
        )
        raise typer.Exit(0)

    if not script_path:
        console.print(
            "[bold red]Error:[/bold red] Provide a macro JSON path, or use --init <file>."
        )
        raise typer.Exit(1)

    if not os.path.exists(script_path):
        console.print(f"[bold red]Error:[/bold red] File not found: {script_path}")
        raise typer.Exit(1)

    with open(script_path, "r", encoding="utf-8") as f:
        raw = json.load(f)

    doc, errors = _normalize_macro(raw)
    if errors:
        console.print("[bold red]Invalid macro:[/bold red]")
        for e in errors:
            console.print(f"  • {e}")
        raise typer.Exit(1)

    if loop:
        doc["loop"] = True

    instructions = doc["instructions"]
    if not instructions:
        console.print("[bold red]Error:[/bold red] Macro has no instructions.")
        raise typer.Exit(1)

    if speed <= 0:
        console.print("[bold red]Error:[/bold red] --speed must be > 0")
        raise typer.Exit(1)

    backend = resolve_backend(ctx, mode)

    async def _run_macro():
        console.print(
            f"[bold green]Starting macro “{doc['name']}” "
            f"({len(instructions)} steps) via {backend.mode.upper()}...[/bold green]"
        )
        status = await backend.get_status()
        live_config = dict(status.get("config") or {})
        if not live_config:
            live_config = {
                "bpm": 60.0,
                "depth": 1.0,
                "depth_top": True,
                "reversed": False,
                "wave_func": "sine",
                "sharpness": 0.5,
                "spline_points": [0.0, 1.0],
                "paused": True,
                "paused_position": 0.5,
            }
        live_config["streaming"] = False

        duration_ms = instructions[-1]["at"]
        duration_sec = (duration_ms / 1000.0) / speed

        try:
            idx = 0
            t0 = time.monotonic()
            with Live(console=console, refresh_per_second=8) as live:
                while True:
                    elapsed = (time.monotonic() - t0) * speed
                    elapsed_ms = elapsed * 1000.0

                    while idx < len(instructions) and instructions[idx]["at"] <= elapsed_ms:
                        inst = instructions[idx]
                        action = inst["action"]
                        params = inst.get("params") or {}
                        if action == "start":
                            await backend.set_paused(False)
                            live_config["paused"] = False
                        elif action == "stop":
                            pos = params.get("position")
                            await backend.set_paused(
                                True, position=float(pos) if pos is not None else None
                            )
                            live_config["paused"] = True
                            if pos is not None:
                                live_config["paused_position"] = float(pos)
                        elif action == "set":
                            live_config.update(params)
                            live_config["streaming"] = False
                            posted = await backend.post_full_config(live_config)
                            if isinstance(posted, dict) and posted:
                                live_config.update(posted)
                        idx += 1

                    next_desc = "—"
                    if idx < len(instructions):
                        nxt = instructions[idx]
                        next_desc = f"{nxt['action']} @ {nxt['at']}ms"
                    elif doc.get("loop"):
                        next_desc = "(loop)"

                    table = Table(title=f"Macro Playback ({os.path.basename(script_path)})")
                    table.add_column("Metric", style="cyan", no_wrap=True)
                    table.add_column("Value", style="bold green")
                    table.add_row(
                        "Progress",
                        f"{elapsed:.1f}s / {duration_sec:.1f}s",
                    )
                    table.add_row("Executed", f"{idx} / {len(instructions)}")
                    table.add_row("Next", next_desc)
                    table.add_row("Speed", f"{speed:.2f}x")
                    table.add_row("Loop", "yes" if doc.get("loop") else "no")
                    live.update(table)

                    if idx >= len(instructions) and elapsed_ms >= duration_ms:
                        if doc.get("loop") and duration_ms > 0:
                            idx = 0
                            t0 = time.monotonic()
                            continue
                        break

                    await asyncio.sleep(0.025)

            console.print("[bold green]✓ Macro playback completed.[/bold green]")
        except KeyboardInterrupt:
            console.print("\n[bold yellow]Macro interrupted by user.[/bold yellow]")
        finally:
            try:
                await backend.set_paused(True)
            except Exception:
                pass

    asyncio.run(_run_macro())


if __name__ == "__main__":
    app()
