#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "aiohttp>=3.9.0",
# ]
# ///
"""Full-fidelity OSSM WiFi frontend mock: REST + /ws/command + static SPA."""

from __future__ import annotations

import asyncio
import copy
import json
import math
import time
from pathlib import Path
from typing import Any, Optional

from aiohttp import WSMsgType, web

DEFAULT_CONFIG: dict[str, Any] = {
    "bpm": 60.0,
    "depth": 1.0,
    "depth_top": True,
    "reversed": False,
    "wave_func": "sine",
    "sharpness": 0.5,
    "spline_points": [0.0, 1.0],
    "paused": True,
    "paused_position": 0.5,
    "streaming": False,
}

DEFAULT_PIN: dict[str, Any] = {
    "modbus_tx": 18,
    "modbus_rx": 19,
    "modbus_de_re": 20,
    "modbus_timeout_ms": 0,
    "modbus_rx_timeout_us": 0,
    "modbus_scan_delay_us": 0,
    "modbus_inter_frame_delay_us": 0,
    "ble_enabled": True,
    "modbus_debug": False,
    "operating_mode": "servo",
    "modbus_baud": 115200,
}

DEFAULT_NET: dict[str, Any] = {
    "wifi_enabled": True,
    "ssid": "MockSSID",
    "password": "mockpass",
    "hostname": "ossm",
    "dhcp_enabled": True,
    "static_ip": "192.168.1.100",
    "static_mask": "255.255.255.0",
    "static_gateway": "192.168.1.1",
    "static_dns": "8.8.8.8",
}


def _cors_headers() -> dict[str, str]:
    return {
        "Access-Control-Allow-Origin": "*",
        "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
        "Access-Control-Allow-Headers": "Content-Type",
        "Connection": "close",
    }


class MockDeviceState:
    """In-memory motor / pin / network state with simple kinematics."""

    def __init__(self) -> None:
        self.config = copy.deepcopy(DEFAULT_CONFIG)
        self.pin = copy.deepcopy(DEFAULT_PIN)
        self.net = copy.deepcopy(DEFAULT_NET)
        self.t = 0.0
        self.x = 0.0
        self.y = 0.5
        self.shaped_y = 0.5
        self.position = 0.5
        self.speed = 0.0
        self.pos_min = 0.0
        self.pos_max = 1.0
        self.ups = 320
        self.dt_min_ms = 2.0
        self.dt_avg_ms = 3.1
        self.dt_max_ms = 4.0
        self.dt_mdev_ms = 0.2
        self.update_history = [300, 310, 320, 315, 320]
        self.motor_connected = True
        self.modbus_stats = {
            "successful_requests": 1000,
            "failed_requests": 0,
            "success_rate": 100.0,
            "round_trip": _empty_timing(),
            "slave_latency": _empty_timing(),
            "rx_duration": _empty_timing(),
        }
        self.stream_buffered = 0
        self.stream_time = 0.0
        self.stream_underrun = False
        self.waypoints: list[dict[str, Any]] = []
        self.restart_requested = False
        self._last_tick = time.monotonic()
        self._lock = asyncio.Lock()

    def tick(self) -> None:
        now = time.monotonic()
        dt = max(0.0, now - self._last_tick)
        self._last_tick = now
        streaming = bool(self.config.get("streaming"))
        paused = bool(self.config.get("paused"))

        if streaming and self.waypoints:
            # Consume ~one waypoint every 50ms worth of buffer for Funscript gating
            consume = max(1, int(dt * 20))
            self.stream_buffered = max(0, self.stream_buffered - consume)
            self.stream_time += dt
            if self.waypoints:
                wp = self.waypoints[0]
                self.position = float(wp.get("pos", self.position))
                self.shaped_y = self.position
                self.y = self.position
        elif not paused and not streaming:
            bpm = float(self.config.get("bpm", 60.0) or 60.0)
            self.t += dt * (bpm / 60.0)
            phase = self.t % 1.0
            depth = float(self.config.get("depth", 1.0) or 1.0)
            depth_top = bool(self.config.get("depth_top", True))
            wave = math.sin(phase * 2.0 * math.pi) * 0.5 + 0.5
            if depth_top:
                y = wave * depth
            else:
                y = 1.0 - depth + wave * depth
            if self.config.get("reversed"):
                y = 1.0 - y
            self.y = y
            self.shaped_y = y
            self.x = phase
            self.position = y
            self.speed = depth * (bpm / 60.0) * math.pi
        else:
            park = float(self.config.get("paused_position", 0.5) or 0.5)
            self.y = park
            self.shaped_y = park
            self.position = park
            self.speed = 0.0

    def snapshot(self) -> dict[str, Any]:
        self.tick()
        return {
            "config": copy.deepcopy(self.config),
            "t": self.t,
            "x": self.x,
            "y": self.y,
            "shaped_y": self.shaped_y,
            "position": self.position,
            "speed": self.speed,
            "stream": {
                "buffered": self.stream_buffered,
                "stream_time": self.stream_time,
                "underrun": self.stream_underrun,
            },
            "update_history": list(self.update_history),
            "position_history": [self.position] * 5,
            "pos_min": self.pos_min,
            "pos_max": self.pos_max,
            "ups": self.ups,
            "dt_min_ms": self.dt_min_ms,
            "dt_avg_ms": self.dt_avg_ms,
            "dt_max_ms": self.dt_max_ms,
            "dt_mdev_ms": self.dt_mdev_ms,
            "motor_connected": self.motor_connected,
            "modbus_stats": copy.deepcopy(self.modbus_stats),
        }

    def apply_config(self, body: dict[str, Any]) -> dict[str, Any]:
        merged = copy.deepcopy(self.config)
        merged.update(body)
        if "spline_points" in body and isinstance(body["spline_points"], list):
            merged["spline_points"] = list(body["spline_points"])
        self.config = merged
        return copy.deepcopy(self.config)

    def apply_paused(self, body: dict[str, Any]) -> dict[str, Any]:
        if "paused" in body:
            self.config["paused"] = bool(body["paused"])
        pos = body.get("position", body.get("paused_position"))
        if pos is not None:
            p = max(0.0, min(1.0, float(pos)))
            self.config["paused_position"] = p
            self.position = p
            self.y = p
            self.shaped_y = p
        if "adjust" in body:
            adj = float(body["adjust"])
            p = max(0.0, min(1.0, float(self.config.get("paused_position", 0.5)) + adj))
            self.config["paused_position"] = p
            self.position = p
            self.y = p
            self.shaped_y = p
        return copy.deepcopy(self.config)


def _empty_timing() -> dict[str, float]:
    return {
        "min": 100.0,
        "max": 500.0,
        "pct5": 120.0,
        "pct10": 140.0,
        "pct50": 200.0,
        "pct90": 350.0,
        "pct95": 400.0,
        "mean": 220.0,
        "mdev": 30.0,
    }


class MockOssmServer:
    """Async context manager yielding http://127.0.0.1:<port>."""

    def __init__(self, dist_dir: Optional[Path] = None) -> None:
        root = Path(__file__).resolve().parent.parent
        self.dist_dir = Path(dist_dir) if dist_dir else root / "frontend" / "dist"
        self.state = MockDeviceState()
        self._runner: Optional[web.AppRunner] = None
        self._site: Optional[web.TCPSite] = None
        self.base_url = ""
        self._subscribers: dict[web.WebSocketResponse, float] = {}
        self._push_task: Optional[asyncio.Task] = None
        self._index_bytes: bytes = b""

    async def __aenter__(self) -> str:
        index = self.dist_dir / "index.html"
        if not index.exists():
            raise FileNotFoundError(
                f"frontend dist missing: {index} (run: npm run build in frontend/)"
            )
        self._index_bytes = index.read_bytes()

        app = web.Application()
        app["mock"] = self
        app.router.add_route("OPTIONS", "/{path:.*}", self._options)
        app.router.add_get("/", self._index)
        app.router.add_get("/config", self._get_config)
        app.router.add_post("/config", self._post_config)
        app.router.add_get("/state", self._get_state)
        app.router.add_post("/paused", self._post_paused)
        app.router.add_get("/pin-config", self._get_pin)
        app.router.add_post("/pin-config", self._post_pin)
        app.router.add_get("/network-config", self._get_net)
        app.router.add_post("/network-config", self._post_net)
        app.router.add_post("/restart", self._post_restart)
        app.router.add_get("/ws/command", self._ws_command)

        self._runner = web.AppRunner(app, shutdown_timeout=1.0)
        await self._runner.setup()
        self._site = web.TCPSite(self._runner, "127.0.0.1", 0)
        await self._site.start()
        addresses = self._runner.addresses
        if not addresses:
            raise RuntimeError("MockOssmServer failed to bind a port")
        _host, port, *_rest = addresses[0]
        self.base_url = f"http://127.0.0.1:{port}"
        self._push_task = asyncio.create_task(self._push_loop())
        return self.base_url

    async def __aexit__(self, *exc: Any) -> None:
        for ws in list(self._subscribers):
            try:
                await ws.close()
            except Exception:
                pass
        self._subscribers.clear()
        if self._push_task:
            self._push_task.cancel()
            try:
                await self._push_task
            except asyncio.CancelledError:
                pass
            self._push_task = None
        if self._site:
            await self._site.stop()
            self._site = None
        if self._runner:
            await self._runner.cleanup()
            self._runner = None

    async def _push_loop(self) -> None:
        try:
            while True:
                await asyncio.sleep(0.02)
                if not self._subscribers:
                    continue
                stale: list[web.WebSocketResponse] = []
                snap = self.state.snapshot()
                msg = json.dumps(
                    {
                        "jsonrpc": "2.0",
                        "method": "state",
                        "params": snap,
                        "state": snap,
                    }
                )
                for ws, interval_ms in list(self._subscribers.items()):
                    if ws.closed:
                        stale.append(ws)
                        continue
                    last = getattr(ws, "_mock_last_push", 0.0)
                    now = time.monotonic()
                    if (now - last) * 1000.0 < interval_ms:
                        continue
                    try:
                        await ws.send_str(msg)
                        setattr(ws, "_mock_last_push", now)
                    except Exception:
                        stale.append(ws)
                for ws in stale:
                    self._subscribers.pop(ws, None)
        except asyncio.CancelledError:
            return

    async def _options(self, request: web.Request) -> web.Response:
        return web.Response(status=204, headers=_cors_headers())

    async def _index(self, request: web.Request) -> web.Response:
        return web.Response(
            body=self._index_bytes,
            content_type="text/html",
            headers=_cors_headers(),
        )

    async def _json(self, data: Any, status: int = 200) -> web.Response:
        return web.Response(
            text=json.dumps(data),
            status=status,
            content_type="application/json",
            headers=_cors_headers(),
        )

    async def _get_config(self, request: web.Request) -> web.Response:
        return await self._json(copy.deepcopy(self.state.config))

    async def _post_config(self, request: web.Request) -> web.Response:
        body = await request.json()
        return await self._json(self.state.apply_config(body))

    async def _get_state(self, request: web.Request) -> web.Response:
        return await self._json(self.state.snapshot())

    async def _post_paused(self, request: web.Request) -> web.Response:
        body = await request.json()
        return await self._json(self.state.apply_paused(body))

    async def _get_pin(self, request: web.Request) -> web.Response:
        return await self._json(copy.deepcopy(self.state.pin))

    async def _post_pin(self, request: web.Request) -> web.Response:
        body = await request.json()
        self.state.pin.update(body)
        return await self._json(copy.deepcopy(self.state.pin))

    async def _get_net(self, request: web.Request) -> web.Response:
        return await self._json(copy.deepcopy(self.state.net))

    async def _post_net(self, request: web.Request) -> web.Response:
        body = await request.json()
        self.state.net.update(body)
        return await self._json(copy.deepcopy(self.state.net))

    async def _post_restart(self, request: web.Request) -> web.Response:
        self.state.restart_requested = True
        return web.Response(status=200, headers=_cors_headers())

    async def _ws_command(self, request: web.Request) -> web.WebSocketResponse:
        ws = web.WebSocketResponse(heartbeat=30.0)
        await ws.prepare(request)
        try:
            async for msg in ws:
                if msg.type != WSMsgType.TEXT:
                    continue
                try:
                    data = json.loads(msg.data)
                except json.JSONDecodeError:
                    continue
                method = data.get("method")
                rpc_id = data.get("id", 1)
                params = data.get("params")
                result: Any = None

                if method == "ping":
                    result = "pong"
                elif method == "get-state":
                    result = self.state.snapshot()
                elif method == "status":
                    snap = self.state.snapshot()
                    result = snap.get("stream", {})
                elif method == "subscribe-state":
                    interval = 33
                    if isinstance(params, dict):
                        interval = int(params.get("interval_ms", 33))
                    interval = max(20, min(60000, interval))
                    self._subscribers[ws] = float(interval)
                    result = {"subscribed": True, "interval_ms": interval}
                elif method == "unsubscribe-state":
                    self._subscribers.pop(ws, None)
                    result = "unsubscribed"
                elif method == "set-config":
                    if isinstance(params, dict):
                        result = self.state.apply_config(params)
                    else:
                        result = copy.deepcopy(self.state.config)
                elif method == "set-waypoints":
                    wps: list[dict[str, Any]] = []
                    if isinstance(params, dict):
                        wps = list(params.get("waypoints") or [])
                        if params.get("reset-timestamp"):
                            self.state.stream_time = 0.0
                    elif isinstance(params, list):
                        wps = list(params)
                    self.state.waypoints = wps
                    self.state.stream_buffered = len(wps)
                    self.state.stream_underrun = False
                    self.state.config["streaming"] = True
                    result = {"ok": True, "buffered": self.state.stream_buffered}
                elif method == "append-waypoints":
                    wps = list(params) if isinstance(params, list) else []
                    self.state.waypoints.extend(wps)
                    self.state.stream_buffered += len(wps)
                    result = {"ok": True, "buffered": self.state.stream_buffered}
                elif method == "reset-timestamp":
                    self.state.stream_time = 0.0
                    result = "ok"
                else:
                    await ws.send_str(
                        json.dumps(
                            {
                                "jsonrpc": "2.0",
                                "id": rpc_id,
                                "error": {
                                    "code": -32601,
                                    "message": f"Unknown method: {method}",
                                },
                            }
                        )
                    )
                    continue

                await ws.send_str(
                    json.dumps({"jsonrpc": "2.0", "id": rpc_id, "result": result})
                )
        finally:
            self._subscribers.pop(ws, None)
        return ws


async def _main() -> None:
    async with MockOssmServer() as url:
        print(f"Mock OSSM server listening at {url}")
        print("Ctrl+C to stop")
        while True:
            await asyncio.sleep(3600)


if __name__ == "__main__":
    try:
        asyncio.run(_main())
    except KeyboardInterrupt:
        pass
