#!/usr/bin/env -S uv run
"""Playwright E2E for the ossm-wasm harness (mock, /ws/rs485, Web Serial ACM)."""

from __future__ import annotations

import argparse
import asyncio
import http.server
import os
import socketserver
import sys
import threading
import time
from pathlib import Path
from typing import TYPE_CHECKING

import httpx
import websockets
from dotenv import load_dotenv
from playwright.async_api import async_playwright

if TYPE_CHECKING:
    from ossm import DeviceBackend

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

REPO = Path(__file__).resolve().parents[1]
WASM_DIST = REPO / "web" / "apps" / "webui-wasm" / "dist"
RELEASE_HTML = REPO / "release" / "ossm-wasm.html"
ENV_PATH = REPO / ".env"

UPS_MIN = 300
DT_MAX_MS = 4.5
HOMING_TIMEOUT_MS = 90_000


def serve_dir(directory: Path, port: int = 0) -> tuple[socketserver.TCPServer, int]:
    directory = directory.resolve()

    class Handler(http.server.SimpleHTTPRequestHandler):
        def __init__(self, *args, **kwargs):
            super().__init__(*args, directory=str(directory), **kwargs)

        def log_message(self, fmt: str, *args) -> None:
            pass

    httpd = socketserver.TCPServer(("127.0.0.1", port), Handler)
    httpd.allow_reuse_address = True
    t = threading.Thread(target=httpd.serve_forever, daemon=True)
    t.start()
    return httpd, httpd.server_address[1]


async def wait_mode(ip: str, want: str, timeout_s: float = 15.0) -> None:
    deadline = time.monotonic() + timeout_s
    last = None
    while time.monotonic() < deadline:
        try:
            async with httpx.AsyncClient(timeout=2.0) as client:
                r = await client.get(f"http://{ip}/shell-config")
                if r.status_code == 200:
                    last = r.json().get("operating_mode")
                    if last == want:
                        return
        except Exception:
            pass
        await asyncio.sleep(0.4)
    raise TimeoutError(f"operating_mode did not become {want!r} (last={last!r})")


async def wait_ws_rs485(ip: str, timeout_s: float = 15.0) -> None:
    deadline = time.monotonic() + timeout_s
    url = f"ws://{ip}/ws/rs485"
    last_err: Exception | None = None
    while time.monotonic() < deadline:
        try:
            async with websockets.connect(url, open_timeout=2, close_timeout=1):
                return
        except Exception as e:
            last_err = e
            await asyncio.sleep(0.4)
    raise TimeoutError(f"/ws/rs485 not accepting: {last_err}")


async def wait_http(ip: str, timeout_s: float = 45.0) -> None:
    deadline = time.monotonic() + timeout_s
    url = f"http://{ip}/shell-config"
    last = None
    while time.monotonic() < deadline:
        try:
            async with httpx.AsyncClient(timeout=2.0) as client:
                r = await client.get(url)
                last = r.status_code
                if r.status_code == 200:
                    return
        except Exception as e:
            last = e
        await asyncio.sleep(0.5)
    raise TimeoutError(f"HTTP {url} not ready (last={last!r})")


async def restore_servo(wifi: DeviceBackend, ip: str) -> None:
    print(" -> wait for HTTP after ACM (WiFi may drop on USB reset)")
    await wait_http(ip)
    if getattr(wifi, "_http_client", None) is not None:
        await wifi._http_client.aclose()
        wifi._http_client = None
    print(" -> set pin.operating_mode servo (no restart)")
    pin = await wifi.get_pin_config()
    pin["operating_mode"] = "servo"
    await wifi.set_pin_config(pin)
    await wait_mode(ip, "servo")
    deadline = time.monotonic() + 45.0
    last = None
    while time.monotonic() < deadline:
        try:
            async with httpx.AsyncClient(timeout=2.0) as client:
                r = await client.get(f"http://{ip}/state")
                if r.status_code == 200:
                    last = r.json()
                    loop = last.get("loop_stats") or {}
                    ups = loop.get("ups")
                    dt_max = loop.get("dt_max_ms")
                    pos_min = last.get("pos_min")
                    pos_max = last.get("pos_max")
                    homed = (
                        isinstance(pos_min, (int, float))
                        and isinstance(pos_max, (int, float))
                        and pos_max > pos_min
                    )
                    if (
                        isinstance(ups, (int, float))
                        and ups >= UPS_MIN
                        and isinstance(dt_max, (int, float))
                        and dt_max < DT_MAX_MS
                        and homed
                    ):
                        print("✓ live switch back to servo")
                        return
        except Exception:
            pass
        await asyncio.sleep(0.5)
    raise AssertionError(f"servo did not recover: last={last}")


async def ossm_state(page) -> dict:
    return await page.evaluate("() => window.__ossmGetState()")


async def ossm_rpc(page, method: str, params=None):
    return await page.evaluate(
        """async ([method, params]) => window.__ossmRpc(method, params)""",
        [method, params],
    )


async def ossm_paused(page, payload: dict) -> dict:
    return await page.evaluate(
        """async (payload) => window.__ossmPaused(payload)""",
        payload,
    )


async def run_control_checks(page, *, mock: bool) -> None:
    ready = page.locator("#wasm-motor-ready")
    await ready.wait_for(state="visible", timeout=HOMING_TIMEOUT_MS if not mock else 15_000)
    print("✓ motor ready")

    snap = await ossm_state(page)
    assert isinstance(snap, dict), snap
    assert "config" in snap and "position" in snap, list(snap.keys())
    assert "loop_stats" in snap, list(snap.keys())
    assert "pos_min" in snap and "pos_max" in snap, list(snap.keys())
    pos_min, pos_max = snap["pos_min"], snap["pos_max"]
    assert isinstance(pos_min, (int, float)) and isinstance(pos_max, (int, float))
    assert pos_max > pos_min, snap
    if mock:
        assert snap.get("motor_connected") is True, snap
        assert pos_min == 0.0 and pos_max == 100.0, snap
    else:
        assert snap.get("motor_connected") is True or pos_max > pos_min, snap
    print("✓ get-state schema")

    methods = await page.evaluate("() => window.__ossmControlMethods")
    assert isinstance(methods, list)
    joined = " ".join(methods).lower()
    assert "modbus" not in joined or "modbus-ws" in joined
    for name in (
        "connect",
        "disconnect",
        "mock",
        "set-config",
        "get-state",
        "paused",
        "subscribe-state",
        "unsubscribe-state",
        "set-waypoints",
        "append-waypoints",
        "reset-timestamp",
        "restart",
    ):
        assert name in methods, methods
    print("✓ control API only")

    n0 = await page.evaluate("() => window.__ossmStateCount")
    await page.wait_for_timeout(800 if mock else 1500)
    n1 = await page.evaluate("() => window.__ossmStateCount")
    need = 3 if mock else 1
    assert n1 - n0 >= need, f"subscribe-state too few frames: {n0} -> {n1}"
    await ossm_rpc(page, "unsubscribe-state")
    n2 = await page.evaluate("() => window.__ossmStateCount")
    await page.wait_for_timeout(400)
    n3 = await page.evaluate("() => window.__ossmStateCount")
    assert n3 - n2 <= 2, f"unsubscribe did not stop pushes: {n2} -> {n3}"
    await ossm_rpc(page, "subscribe-state", {"interval_ms": 33})
    print("✓ subscribe-state / unsubscribe-state")

    await page.click('button[aria-label*="details"], button[title*="details"], button[title*="Details"]')
    await page.wait_for_selector("#wasm-telemetry", timeout=5_000)
    print("✓ telemetry panel")

    before = await ossm_state(page)
    cfg = dict(before["config"])
    version = int(cfg.get("version") or 0)
    cfg["bpm"] = 48.0
    cfg["depth"] = 0.4
    cfg["wave_func"] = "thrust"
    cfg["version"] = version
    applied = await ossm_rpc(page, "set-config", cfg)
    assert applied["bpm"] == 48.0, applied
    assert applied["depth"] == 0.4, applied
    assert applied["wave_func"] == "thrust", applied
    assert int(applied["version"]) > version, applied
    stale = dict(applied)
    stale["bpm"] = 12.0
    stale["version"] = int(applied["version"]) - 1
    stale_err = None
    try:
        await ossm_rpc(page, "set-config", stale)
    except Exception as e:
        stale_err = e
    assert stale_err is not None, "stale version should be rejected"
    print("✓ set-config version + stale reject")

    start = page.get_by_role("button", name="Start")
    stop = page.get_by_role("button", name="Stop")
    if await start.count():
        await start.first.click()
        await page.wait_for_timeout(300)
        resumed = await ossm_state(page)
        assert resumed["config"]["paused"] is False, resumed["config"]
        if await stop.count():
            await stop.first.click()
            await page.wait_for_timeout(200)
        paused = await ossm_state(page)
        assert paused["config"]["paused"] is True, paused["config"]
        await page.wait_for_timeout(1000)
        hold_a = float((await ossm_state(page))["position"])
        await page.wait_for_timeout(400)
        hold_b = float((await ossm_state(page))["position"])
        assert abs(hold_b - hold_a) < 15.0, (hold_a, hold_b)
        print("✓ pause/resume via UI")
    else:
        await ossm_paused(page, {"paused": False})
        await ossm_paused(page, {"paused": True, "position": 0.3})
        print("✓ pause via RPC")

    spline_cfg = dict((await ossm_state(page))["config"])
    spline_cfg["wave_func"] = "spline"
    spline_cfg["spline_points"] = [0.0, 0.25, 1.0, 0.5]
    spline_cfg["version"] = int(spline_cfg.get("version") or 0)
    splined = await ossm_rpc(page, "set-config", spline_cfg)
    assert splined["wave_func"] == "spline", splined
    got = [round(float(x), 4) for x in splined["spline_points"]]
    assert got == [0.0, 0.25, 1.0, 0.5], got
    print("✓ spline_points round-trip")

    stream_cfg = dict((await ossm_state(page))["config"])
    stream_cfg["paused"] = False
    stream_cfg["streaming"] = True
    stream_cfg["version"] = int(stream_cfg.get("version") or 0)
    await ossm_rpc(page, "set-config", stream_cfg)
    await ossm_rpc(
        page,
        "set-waypoints",
        {
            "waypoints": [{"ts": 0, "pos": 0.2}, {"ts": 80, "pos": 0.8}],
            "reset-timestamp": True,
        },
    )
    streamed = await ossm_state(page)
    buffered = (streamed.get("stream") or {}).get("buffered", 0)
    assert streamed["config"].get("streaming") is True or buffered >= 1, streamed
    await ossm_rpc(page, "append-waypoints", [{"ts": 160, "pos": 0.4}])
    await page.wait_for_timeout(400)
    idle_cfg = dict((await ossm_state(page))["config"])
    idle_cfg["streaming"] = False
    idle_cfg["paused"] = True
    idle_cfg["version"] = int(idle_cfg.get("version") or 0)
    await ossm_rpc(page, "set-config", idle_cfg)
    print("✓ set-waypoints / append-waypoints")

    await page.locator("#wasm-disconnect").click()
    await page.locator("#wasm-motor-ready").wait_for(state="hidden", timeout=10_000)
    await page.locator("#wasm-connect").click()
    await ready.wait_for(state="visible", timeout=HOMING_TIMEOUT_MS if not mock else 15_000)
    again = await ossm_state(page)
    assert again.get("motor_connected") is True, again
    assert again["pos_max"] > again["pos_min"], again
    print("✓ disconnect / reconnect + re-home")
    print("✓ control checks done")


async def test_mock(base_url: str) -> None:
    print("==> wasm harness mock")
    async with async_playwright() as p:
        browser = await p.chromium.launch(headless=True, args=["--no-sandbox"])
        context = await browser.new_context(viewport={"width": 1280, "height": 800})
        await context.add_init_script("localStorage.setItem('ossm_locale', 'en')")
        page = await context.new_page()
        await page.goto(base_url, wait_until="load")
        await page.locator("#wasm-connection").wait_for(timeout=10_000)
        if await page.locator('#wasm-connection-type option[value="mock"]').count() == 0:
            print("skip mock (release build without VITE_OSSM_ALLOW_MOCK)")
            await browser.close()
            return
        await page.locator("#wasm-connection-type").select_option("mock")
        await page.locator("#wasm-connect").click()
        await run_control_checks(page, mock=True)
        await page.locator("#wasm-disconnect").click()
        await browser.close()
    print("✓ mock PASSED")


async def test_rs485_ws(base_url: str, ip: str) -> None:
    print("==> wasm harness /ws/rs485")
    async with async_playwright() as p:
        browser = await p.chromium.launch(headless=True, args=["--no-sandbox"])
        context = await browser.new_context(viewport={"width": 1280, "height": 800})
        await context.add_init_script("localStorage.setItem('ossm_locale', 'en')")
        page = await context.new_page()
        page.on("console", lambda m: print(f"  console: {m.text}", flush=True))
        await page.goto(base_url, wait_until="load")
        await page.locator("#wasm-connection-type").select_option("rs485")
        await page.locator("#wasm-ws-url").wait_for(state="visible")
        await page.locator("#wasm-ws-url").fill(f"ws://{ip}/ws/rs485")
        await page.locator("#wasm-connect").click()
        try:
            await run_control_checks(page, mock=False)
        except Exception:
            err = page.locator("p.text-rose-600")
            if await err.count():
                print(f"  wasm error: {await err.first.inner_text()}", flush=True)
            print(
                f"  ready hidden={await page.locator('#wasm-motor-ready').is_hidden()} "
                f"homing={await page.locator('#wasm-homing').is_visible()}",
                flush=True,
            )
            raise
        await page.locator("#wasm-disconnect").click()
        await browser.close()
    print("✓ /ws/rs485 PASSED")


async def test_serial(base_url: str, device_port: str, ip: str) -> None:
    from webserial_bridge import WebSerialBridge

    print("==> wasm harness Web Serial ACM")
    async with async_playwright() as p:
        browser = await p.chromium.launch(
            headless=True,
            args=["--no-sandbox", "--enable-features=WebSerial", "--enable-experimental-web-platform-features"],
        )
        context = await browser.new_context(viewport={"width": 1280, "height": 800})
        await context.add_init_script("localStorage.setItem('ossm_locale', 'en')")
        page = await context.new_page()
        page.on("console", lambda m: print(f"  console: {m.text}", flush=True))
        bridge = WebSerialBridge(page, auto_select_port=device_port, dtr=False, rts=False)
        await bridge.setup()
        await page.goto(base_url, wait_until="load")
        await page.locator("#wasm-connection-type").select_option("serial")
        await page.locator("#wasm-connect").click()
        try:
            await page.locator("#wasm-motor-ready").wait_for(timeout=20_000)
        except Exception:
            print(" -> first serial connect timed out (possible ACM reset); retry")
            await wait_mode(ip, "rs485", timeout_s=20.0)
            await wait_ws_rs485(ip, timeout_s=20.0)
            await page.locator("#wasm-connect").click()
        await run_control_checks(page, mock=False)
        await page.locator("#wasm-disconnect").click()
        await browser.close()
    print("✓ Web Serial ACM PASSED")


def harness_dir() -> Path:
    if (WASM_DIST / "index.html").exists():
        return WASM_DIST
    if RELEASE_HTML.exists():
        return RELEASE_HTML.parent
    raise SystemExit(
        f"wasm harness HTML missing. Build with: npm run build -w webui-wasm (in web/)\n"
        f"looked at {WASM_DIST} and {RELEASE_HTML}"
    )


async def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mock", action="store_true", help="Harness only, no device")
    parser.add_argument("--skip-serial", action="store_true", help="Skip USB ACM path")
    args = parser.parse_args()

    dist = harness_dir()
    httpd, port = serve_dir(dist if dist.is_dir() else dist.parent)
    base_url = f"http://127.0.0.1:{port}/"
    if dist.is_file():
        base_url = f"http://127.0.0.1:{port}/{dist.name}"
    elif (dist / "index.html").exists():
        base_url = f"http://127.0.0.1:{port}/"
    print(f"serving {dist} at {base_url}")

    try:
        await test_mock(base_url)
        if args.mock:
            print("✓ wasm mock-only PASSED")
            return

        from ossm import DeviceBackend, load_env

        load_env()
        ip = os.environ.get("DEVICE_IP", "").strip().strip('"')
        device_port = os.environ.get("DEVICE_PORT", "").strip().strip('"')
        if not ip:
            raise SystemExit("DEVICE_IP missing in .env")
        wifi = DeviceBackend(mode="wifi", ip=ip)
        pin = await wifi.get_pin_config()
        print(f"boot operating_mode={pin.get('operating_mode')}")
        pin["operating_mode"] = "rs485"
        await wifi.set_pin_config(pin)
        await wait_mode(ip, "rs485")
        await wait_ws_rs485(ip)
        print("✓ live switch to rs485")
        try:
            await test_rs485_ws(base_url, ip)
            if not args.skip_serial:
                if not device_port:
                    raise SystemExit("DEVICE_PORT missing in .env")
                await wait_mode(ip, "rs485", timeout_s=20.0)
                await wait_ws_rs485(ip, timeout_s=20.0)
                await test_serial(base_url, device_port, ip)
        finally:
            await restore_servo(wifi, ip)
        print("✓ wasm live E2E PASSED")
    finally:
        httpd.shutdown()
        httpd.server_close()


if __name__ == "__main__":
    asyncio.run(main())
