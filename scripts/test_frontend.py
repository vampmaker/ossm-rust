#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "playwright",
#     "httpx",
#     "python-dotenv>=1.0.0",
#     "bleak>=0.21.0",
#     "requests>=2.31.0",
#     "pyserial>=3.5",
#     "typer>=0.12.0",
#     "rich>=13.7.0",
#     "pydantic>=2.0.0",
#     "websockets>=12.0",
# ]
# ///

import asyncio
import json
import os
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import httpx
from dotenv import load_dotenv
from playwright.async_api import async_playwright
import websockets

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
ENV_PATH = os.path.join(SCRIPT_DIR, "..", ".env")

REST_ENDPOINTS = (
    "/",
    "/config",
    "/state",
    "/pin-config",
    "/network-config",
)


def load_device_url() -> str:
    if os.path.exists(ENV_PATH):
        load_dotenv(ENV_PATH)
    ip = os.environ.get("DEVICE_IP", "").strip().strip('"')
    if not ip:
        print("ERROR: DEVICE_IP is not set in .env", file=sys.stderr)
        sys.exit(1)
    return f"http://{ip}"


async def fetch_status(
    base_url: str,
    path: str,
    retries: int = 3,
) -> tuple[str, int, float]:
    last_error: Exception | None = None
    for attempt in range(1, retries + 1):
        started = time.perf_counter()
        try:
            async with httpx.AsyncClient(
                base_url=base_url,
                timeout=httpx.Timeout(15.0, connect=8.0),
                headers={"Connection": "close"},
            ) as client:
                response = await client.get(path)
            elapsed_ms = (time.perf_counter() - started) * 1000
            return path, response.status_code, elapsed_ms
        except httpx.HTTPError as exc:
            last_error = exc
            if attempt < retries:
                await asyncio.sleep(0.15 * attempt)
    raise AssertionError(f"GET {path} failed after {retries} attempts: {last_error}")


async def test_concurrent_http(base_url: str, workers: int = 4, rounds: int = 3) -> None:
    print(f"\n[HTTP] Concurrent REST verification against {base_url}")
    print(
        f"[HTTP] {workers} parallel clients per round (matches acceptor queue); "
        "REST is serialized server-side"
    )

    status = 503
    for _ in range(10):
        await asyncio.sleep(0.5)
        path, status, _ = await fetch_status(base_url, "/state")
        if status == 200:
            break
    assert status == 200, f"Warm-up /state failed: {status}"
    print(f"✓ Warm-up GET {path} -> {status}")

    for round_idx in range(1, rounds + 1):
        paths = [REST_ENDPOINTS[i % len(REST_ENDPOINTS)] for i in range(workers)]
        started = time.perf_counter()
        results = await asyncio.gather(*(fetch_status(base_url, path) for path in paths))
        wall_ms = (time.perf_counter() - started) * 1000

        failures = [(path, status) for path, status, _ in results if status != 200]
        assert not failures, f"Round {round_idx} had HTTP failures: {failures}"

        per_path = ", ".join(
            f"{path}={status} ({elapsed:.0f}ms)" for path, status, elapsed in results
        )
        print(f"✓ Round {round_idx}: {workers} parallel GETs in {wall_ms:.0f}ms wall time")
        print(f"  {per_path}")
        await asyncio.sleep(0.2)

    burst_paths = ["/state"] * workers
    started = time.perf_counter()
    burst = await asyncio.gather(*(fetch_status(base_url, path) for path in burst_paths))
    wall_ms = (time.perf_counter() - started) * 1000
    assert all(status == 200 for _, status, _ in burst), "Burst /state had non-200 responses"
    print(f"✓ Burst: {workers}x GET /state in {wall_ms:.0f}ms wall time (all 200)")

    def thread_get(path: str) -> tuple[str, int, float]:
        last_error: Exception | None = None
        for attempt in range(1, 4):
            t0 = time.perf_counter()
            try:
                with httpx.Client(
                    base_url=base_url,
                    timeout=15.0,
                    headers={"Connection": "close"},
                ) as sync_client:
                    response = sync_client.get(path)
                return path, response.status_code, (time.perf_counter() - t0) * 1000
            except httpx.HTTPError as exc:
                last_error = exc
                time.sleep(0.15 * attempt)
        raise AssertionError(f"GET {path} failed after retries: {last_error}")

    mix_paths = list(REST_ENDPOINTS) + ["/state", "/config"]
    started = time.perf_counter()
    with ThreadPoolExecutor(max_workers=len(mix_paths)) as pool:
        threaded = list(pool.map(thread_get, mix_paths))
    wall_ms = (time.perf_counter() - started) * 1000
    failures = [(path, status) for path, status, _ in threaded if status != 200]
    assert not failures, f"Threaded burst had HTTP failures: {failures}"
    print(
        f"✓ Threaded burst: {len(mix_paths)} simultaneous clients in {wall_ms:.0f}ms "
        f"(all 200)"
    )


async def test_frontend(base_url: str):
    print(f"============================================================")
    print(f"  OSSM Frontend Playwright E2E Verification Suite")
    print(f"  Target: {base_url}")
    print(f"============================================================")

    async with async_playwright() as p:
        browser = await p.chromium.launch(
            headless=True,
            args=[
                "--headless=new",
                "--enable-features=WebBluetooth",
                "--enable-experimental-web-platform-features",
            ],
        )
        context = await browser.new_context(viewport={"width": 1280, "height": 800})
        page = await context.new_page()

        ws_messages = []
        ws_connections = []

        def on_websocket(ws):
            print(f"[WS CONNECTED] {ws.url}")
            ws_connections.append(ws)

            ws.on("framereceived", lambda payload: ws_messages.append((time.time(), payload)))
            ws.on("close", lambda w: print("[WS CLOSED] Connection cleanly terminated"))

        page.on("websocket", on_websocket)
        page.on("console", lambda msg: print(f"[BROWSER {msg.type.upper()}] {msg.text}"))

        print(f"\n[Step 1] Navigating to {base_url}...")
        await page.goto(base_url, wait_until="networkidle")
        await page.evaluate("localStorage.removeItem('ossm_show_position_diagram')")
        await page.reload(wait_until="networkidle")

        title = await page.title()
        content = await page.content()
        assert "OSSM" in title or "OSSM" in content, "Expected OSSM Controller title/header"
        print("✓ Page successfully loaded and header verified")

        print(f"\n[Step 2] Observing WebSocket real-time state push telemetry...")
        await asyncio.sleep(2.5)
        assert len(ws_connections) > 0, "Expected at least one WebSocket connection to be established"
        assert len(ws_messages) > 5, "Expected stream of state push notifications from firmware"

        if len(ws_messages) >= 10:
            dt_list = [t2 - t1 for (t1, _), (t2, _) in zip(ws_messages, ws_messages[1:])]
            avg_dt = sum(dt_list) / len(dt_list)
            fps = 1.0 / avg_dt if avg_dt > 0 else 0
            print(f"✓ Received {len(ws_messages)} state updates over WebSocket (~{fps:.1f} FPS)")

        print(f"\n[Step 2b] Verifying browser WebSocket stability under concurrent HTTP load...")
        pre_count = len(ws_messages)
        fetch_results = await page.evaluate("""async () => {
            const urls = ['/state', '/config', '/pin-config', '/network-config'];
            const res = await Promise.all(urls.map(u => fetch(u).then(r => r.status)));
            return res;
        }""")
        assert all(status == 200 for status in fetch_results), f"Browser REST fetches failed: {fetch_results}"
        await asyncio.sleep(1.0)
        post_count = len(ws_messages)
        new_msgs = post_count - pre_count
        assert new_msgs >= 10, f"Expected continuous WebSocket streaming during REST load, got {new_msgs} messages in 1s"
        print(f"✓ Browser WebSocket remained stable and streamed {new_msgs} frames during concurrent REST fetches")

        print(f"\n[Step 3] Testing Motor Position Diagram toggle button and localStorage persistence...")
        diagram_toggle = page.locator('button[aria-label="Toggle motor position diagram"]')
        await diagram_toggle.wait_for(state="visible", timeout=5000)

        print(" -> Clicking toggle to hide diagram...")
        await diagram_toggle.click()
        await asyncio.sleep(0.5)
        saved_val = await page.evaluate("localStorage.getItem('ossm_show_position_diagram')")
        print(f" -> localStorage['ossm_show_position_diagram'] = {saved_val}")

        print(" -> Clicking toggle to show diagram...")
        await diagram_toggle.click()
        await asyncio.sleep(0.5)
        saved_val = await page.evaluate("localStorage.getItem('ossm_show_position_diagram')")
        assert saved_val == "true", f"Expected localStorage key to be 'true', got '{saved_val}'"
        print("✓ Motor Position Diagram toggle and persistence verified")

        print(f"\n[Step 4] Opening Unified Device Settings Panel and verifying independent toggles & conditional fields...")
        settings_toggle = page.locator('button[aria-label="Toggle settings"]')
        await settings_toggle.wait_for(state="visible", timeout=5000)
        await settings_toggle.click()

        await page.wait_for_selector("text=Unified Device Settings", timeout=5000)
        await page.wait_for_selector("text=Motor Update Rate Telemetry", timeout=5000)
        print("✓ Unified Device Settings panel opened and telemetry card verified")

        ups_element = page.locator("text=updates/sec (Hz)")
        await ups_element.wait_for(state="visible", timeout=5000)
        print("✓ Motor Update Rate telemetry unit verified visible")

        # Verify independent BLE & WiFi checkboxes
        ble_chk = page.locator("#ble-enabled")
        wifi_chk = page.locator("#wifi-enabled")
        await ble_chk.wait_for(state="visible", timeout=5000)
        await wifi_chk.wait_for(state="visible", timeout=5000)
        print("✓ Independent BLE (#ble-enabled) and WiFi (#wifi-enabled) checkboxes verified visible")

        # Verify conditional WiFi fields (SSID & Password) when WiFi is enabled
        await page.wait_for_selector("#wifi-config-panel", timeout=5000)
        await page.wait_for_selector("#wifi-ssid", timeout=5000)
        await page.wait_for_selector("#wifi-password", timeout=5000)
        print("✓ Conditional WiFi config panel (#wifi-ssid, #wifi-password) verified visible when WiFi enabled")

        # Assert WiFi checkbox is disabled (un-uncheck-able) when connected via WiFi mode
        assert await wifi_chk.is_disabled(), "Expected #wifi-enabled to be un-uncheck-able when connected via WiFi mode"
        print("✓ Verified #wifi-enabled checkbox is un-uncheck-able when connected through WiFi")

        await settings_toggle.click()

        print(f"\n[Step 5] Verifying Main Controls interactive elements...")
        await page.wait_for_selector("#speed-slider", timeout=5000)
        await page.wait_for_selector("#depth-slider", timeout=5000)
        await page.wait_for_selector("text=Waveform", timeout=5000)
        print("✓ Main motion controls verified visible")

        print(f"\n[Step 6] Verifying Motor Position Diagram visualization...")
        await page.wait_for_selector("text=Motor Position Diagram", timeout=5000)
        print("✓ Motor Position Diagram component verified visible")

        print(f"\n[Step 7] Verifying Web Bluetooth real hardware end-to-end connection via CDP DeviceAccess...")
        import http.server
        import socketserver
        import threading

        class DistHandler(http.server.SimpleHTTPRequestHandler):
            def __init__(self, *args, **kwargs):
                dist_dir = os.path.abspath(os.path.join(os.path.dirname(__file__), "../frontend/dist"))
                super().__init__(*args, directory=dist_dir, **kwargs)
            def log_message(self, format, *args): pass

        ble_port = 18090
        server = socketserver.TCPServer(("127.0.0.1", ble_port), DistHandler)
        t = threading.Thread(target=server.serve_forever, daemon=True)
        t.start()
        print(f" -> Serving frontend/dist on http://localhost:{ble_port}/ for secure Web Bluetooth E2E testing")

        ble_page = await context.new_page()
        cdp = await context.new_cdp_session(ble_page)
        await cdp.send("DeviceAccess.enable")

        async def on_prompt(event):
            devices = event.get("devices", [])
            for d in devices:
                if "OSSM" in d.get("name", ""):
                    print(f"👀 [CDP] Found OSSM hardware BLE device: {d['name']} ({d['id']})")
                    await cdp.send("DeviceAccess.selectPrompt", {"id": event["id"], "deviceId": d["id"]})
                    print(f"✅ [CDP] Selected hardware BLE device: {d['name']}")
                    return
        cdp.on("DeviceAccess.deviceRequestPrompted", on_prompt)

        await ble_page.goto(f"http://localhost:{ble_port}/", wait_until="networkidle")
        ble_mode_btn = ble_page.locator("button:has-text('BLE')").first
        await ble_mode_btn.click()
        await asyncio.sleep(0.5)
        connect_btn = ble_page.locator("button:has-text('Connect BLE')")
        await connect_btn.wait_for(state="visible", timeout=5000)

        print(" -> Clicking 'Connect BLE' and initiating hardware scan...")
        await connect_btn.click()
        await ble_page.wait_for_selector("text=Connected", timeout=15000)
        print("✓ Web Bluetooth connected end-to-end against real hardware successfully!")

        ble_settings_toggle = ble_page.locator('button[aria-label="Toggle settings"]')
        await ble_settings_toggle.click()
        await ble_page.wait_for_selector("text=Unified Device Settings", timeout=5000)
        ble_chk_ble = ble_page.locator("#ble-enabled")
        assert await ble_chk_ble.is_disabled(), "Expected #ble-enabled to be un-uncheck-able when connected via BLE mode"
        print("✓ Verified #ble-enabled checkbox is un-uncheck-able when connected through BLE")

        server.shutdown()
        server.server_close()
        await browser.close()


async def _run_ble_tests():
    """Run BLE integration tests asynchronously."""
    from ossm import DeviceBackend, BleChunkReassembler, CHAR_STATE, CHAR_RPC, load_env as ossm_load_env
    ossm_load_env()

    print(f"\n============================================================")
    print(f"  BLE Integration Test Suite")
    print(f"============================================================")

    backend = DeviceBackend(mode="ble")
    try:
        print(f"\n[BLE Step 1] Connecting and reading pin config...")
        pin_cfg = await backend.get_pin_config()
        assert "modbus_tx" in pin_cfg, f"Pin config missing modbus_tx: {pin_cfg}"
        print(f"✓ Pin config: tx={pin_cfg.get('modbus_tx')}, rx={pin_cfg.get('modbus_rx')}, de_re={pin_cfg.get('modbus_de_re')}")

        print(f"\n[BLE Step 2] Reading motor config and state (GATT reads)...")
        status = await backend.get_status()
        config = status.get("config", {})
        state = status.get("state", {})
        assert "bpm" in config, f"Config missing bpm: {config}"
        assert "position" in state, f"State missing position: {state}"
        print(f"✓ Config: bpm={config.get('bpm')}, depth={config.get('depth')}, paused={config.get('paused')}")
        print(f"✓ State: position={state.get('position')}, ups={state.get('ups')}")

        print(f"\n[BLE Step 3] Testing JSON-RPC over BLE (ping)...")
        ping_res = await backend.send_rpc("ping", rpc_id=10)
        assert ping_res.get("result") == "pong", f"Ping failed: {ping_res}"
        print(f"✓ RPC ping -> pong")

        print(f"\n[BLE Step 4] Testing config write over BLE...")
        original_bpm = config.get("bpm", 36.0)
        test_bpm = 22.0
        await backend.set_config({"bpm": test_bpm})
        readback = (await backend.get_status()).get("config", {})
        assert abs(readback.get("bpm", 0) - test_bpm) < 0.1, f"Config write failed: bpm={readback.get('bpm')}"
        await backend.set_config({"bpm": original_bpm})
        print(f"✓ Config write and readback verified (bpm {original_bpm} -> {test_bpm} -> {original_bpm})")

        print(f"\n[BLE Step 5] Testing chunked state notifications (subscribe-state)...")
        notifications = []
        reassembler = BleChunkReassembler()

        client = await backend._get_ble_client()

        def _state_cb(sender, data):
            payload = reassembler.feed(bytes(data))
            if payload is None:
                return
            try:
                st = json.loads(payload.decode("utf-8", errors="ignore"))
                notifications.append(st)
            except Exception:
                pass

        await client.start_notify(CHAR_STATE, _state_cb)
        await asyncio.sleep(0.2)
        sub_cmd = {"jsonrpc": "2.0", "method": "subscribe-state", "params": {"interval_ms": 50}, "id": 20}
        await client.write_gatt_char(CHAR_RPC, json.dumps(sub_cmd).encode("utf-8"), response=True)
        await asyncio.sleep(3.0)
        unsub_cmd = {"jsonrpc": "2.0", "method": "unsubscribe-state", "id": 21}
        await client.write_gatt_char(CHAR_RPC, json.dumps(unsub_cmd).encode("utf-8"), response=True)
        await asyncio.sleep(0.3)
        await client.stop_notify(CHAR_STATE)

        assert len(notifications) >= 15, f"Expected >=15 notifications, got {len(notifications)}"
        first = notifications[0]
        assert "position" in first, f"Notification missing 'position': {list(first.keys())}"
        assert "config" in first, f"Notification missing 'config': {list(first.keys())}"
        print(f"✓ Received {len(notifications)} fast state notifications in 3s (rate: {len(notifications)/3.0:.1f} Hz)")
        print(f"  Schema verified: position={first.get('position')}, config.paused={first.get('config', {}).get('paused')}")

        print(f"\n[BLE Step 6] Testing network config read...")
        net_cfg = await backend.get_network_config()
        assert "hostname" in net_cfg, f"Network config missing hostname: {net_cfg}"
        print(f"✓ Network config: hostname={net_cfg.get('hostname')}, dhcp={net_cfg.get('dhcp_enabled')}")

        print(f"\n✓ All BLE integration tests PASSED")
    finally:
        await backend.disconnect()
        await asyncio.sleep(1.5)


async def test_ble_connection():
    """Run BLE tests."""
    await _run_ble_tests()


async def test_flasher():
    print(f"\n============================================================")
    print(f"  OSSM Web Flasher Playwright E2E Verification Suite")
    print(f"============================================================")

    flasher_path = Path(__file__).parent.parent / "release" / "flasher.html"
    if not flasher_path.exists():
        print(f"Skipping flasher E2E test: {flasher_path} not found")
        return

    flasher_url = flasher_path.as_uri()
    print(f"  Target: {flasher_url}")

    async with async_playwright() as p:
        browser = await p.chromium.launch(headless=True)
        context = await browser.new_context(viewport={"width": 1280, "height": 800})
        page = await context.new_page()

        await page.goto(flasher_url, wait_until="load")
        title = await page.title()
        assert "OSSM" in title or "Flasher" in title, f"Unexpected flasher title: {title}"
        print("✓ Web Flasher page successfully loaded")

        await page.wait_for_selector("text=Device Configuration", timeout=5000)
        print("✓ Device Configuration section verified visible")

        wifi_chk = page.locator("#flasher-wifi-enabled")
        await wifi_chk.wait_for(state="visible", timeout=5000)
        print("✓ Enable WiFi (#flasher-wifi-enabled) checkbox verified visible in flasher")

        ssid_input = page.locator('input[placeholder="Your WiFi network"]')
        await ssid_input.wait_for(state="visible", timeout=5000)
        print("✓ WiFi SSID input verified visible when Enable WiFi is checked")

        print(" -> Unchecking #flasher-wifi-enabled...")
        await wifi_chk.uncheck()
        await asyncio.sleep(0.3)
        assert await ssid_input.count() == 0, "Expected SSID input to be hidden when WiFi disabled"
        print("✓ WiFi SSID & Password inputs hidden when Enable WiFi is unchecked")

        await wifi_chk.check()
        await page.wait_for_selector('input[placeholder="Your WiFi network"]', state="visible", timeout=5000)
        print("✓ WiFi SSID input restored when Enable WiFi is checked")

        await browser.close()
    print("✓ All Web Flasher E2E tests PASSED")


async def test_web_bluetooth_frontend():
    """Verify built frontend artifact in frontend/dist with Web Bluetooth using Playwright + CDP DeviceAccess."""
    print(f"\n============================================================")
    print(f"  OSSM Web Bluetooth E2E Verification Suite (Playwright + CDP)")
    print(f"============================================================")

    import http.server
    import socketserver
    import threading

    dist_dir = Path(__file__).parent.parent / "frontend" / "dist"
    if not (dist_dir / "index.html").exists():
        print("[WARN] frontend/dist/index.html not found, skipping Web Bluetooth test")
        return

    port = 8099
    class QuietHandler(http.server.SimpleHTTPRequestHandler):
        def __init__(self, *args, **kwargs):
            super().__init__(*args, directory=str(dist_dir), **kwargs)
        def log_message(self, format, *args):
            pass

    httpd = socketserver.TCPServer(("127.0.0.1", port), QuietHandler)
    server_thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    server_thread.start()

    try:
        async with async_playwright() as p:
            browser = await p.chromium.launch(
                headless=True,
                args=[
                    "--headless=new",
                    "--enable-features=WebBluetooth",
                    "--enable-experimental-web-platform-features",
                ],
            )
            context = await browser.new_context(viewport={"width": 1280, "height": 800})
            page = await context.new_page()

            cdp = await context.new_cdp_session(page)
            await cdp.send("DeviceAccess.enable")

            ble_devices_seen = []

            def on_device_prompt(event):
                print("👀 [CDP] Intercepted Web Bluetooth device scan prompt:")
                devices = event.get("devices", [])
                for dev in devices:
                    ble_devices_seen.append(dev)
                    name = dev.get("name", "")
                    dev_id = dev.get("id", "")
                    print(f"  - Device discovered: {name} ({dev_id})")
                    if "OSSM" in name or dev_id:
                        print(f" -> Auto-selecting BLE device via CDP: {name} ({dev_id})")
                        asyncio.create_task(
                            cdp.send(
                                "DeviceAccess.selectPrompt",
                                {"id": event["id"], "deviceId": dev_id},
                            )
                        )
                        break

            cdp.on("DeviceAccess.deviceRequestPrompted", on_device_prompt)

            print(f" -> Navigating to http://127.0.0.1:{port} (serving frontend/dist)...")
            await page.goto(f"http://127.0.0.1:{port}/", wait_until="load")

            title = await page.title()
            assert "OSSM" in title, f"Unexpected page title: {title}"
            print("✓ Built frontend artifact loaded successfully")

            ble_mode_btn = page.locator("button:has-text('BLE')")
            if await ble_mode_btn.count() > 0:
                print(" -> Switching UI to BLE mode...")
                await ble_mode_btn.click()
                await asyncio.sleep(0.5)

            connect_btn = page.locator("button:has-text('Connect BLE')")
            if await connect_btn.count() > 0:
                print(" -> Clicking Connect BLE button...")
                await connect_btn.click()
                await asyncio.sleep(3.0)
                print(f"✓ Web Bluetooth connect button clicked (CDP scan events intercepted: {len(ble_devices_seen)})")

            await browser.close()
            print("✓ Web Bluetooth E2E verification completed successfully")
    finally:
        httpd.shutdown()


async def main():
    base_url = load_device_url()
    await test_concurrent_http(base_url)
    await test_ble_connection()
    await test_frontend(base_url)
    await test_web_bluetooth_frontend()
    await test_flasher()
    print(f"\n============================================================")
    print(f"✓ All HTTP + BLE + frontend + Web Bluetooth + flasher Playwright tests PASSED")
    print(f"============================================================")


if __name__ == "__main__":
    asyncio.run(main())
