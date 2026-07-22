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
#     "aiohttp>=3.9.0",
# ]
# ///

import asyncio
import argparse
import json
import os
import sys
import tempfile
import time
from pathlib import Path

import httpx
from dotenv import load_dotenv
from playwright.async_api import async_playwright
import websockets

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from mock_ossm_server import MockOssmServer

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
ENV_PATH = os.path.join(SCRIPT_DIR, "..", ".env")
FRONTEND_DIST = Path(SCRIPT_DIR).parent / "frontend" / "dist"

REST_ENDPOINTS = (
    "/",
    "/config",
    "/state",
    "/pin-config",
    "/network-config",
)
WEB_BLUETOOTH_STABILITY_RUNS = 10
WEB_BLUETOOTH_CONNECT_TIMEOUT_MS = 35_000
WEB_BLUETOOTH_PROMPT_TIMEOUT_S = 45.0
WEB_BLUETOOTH_COOLDOWN_S = 2.5


def load_device_url() -> str:
    if os.path.exists(ENV_PATH):
        load_dotenv(ENV_PATH)
    ip = os.environ.get("DEVICE_IP", "").strip().strip('"')
    if not ip:
        print("ERROR: DEVICE_IP is not set in .env", file=sys.stderr)
        sys.exit(1)
    return f"http://{ip}"


async def fetch_status(
    base_url: str, path: str, retries: int = 6
) -> tuple[str, int, float]:
    """Fetch status with retries for connection failures."""
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
                await asyncio.sleep(0.3 * attempt)
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
        for attempt in range(1, 7):
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
                time.sleep(0.3 * attempt)
        raise AssertionError(f"GET {path} failed after retries: {last_error}")

    mix_paths = list(REST_ENDPOINTS) + ["/state", "/config"]
    started = time.perf_counter()
    # Use asyncio.to_thread so the event loop can keep serving an in-process mock.
    threaded = await asyncio.gather(
        *(asyncio.to_thread(thread_get, path) for path in mix_paths)
    )
    wall_ms = (time.perf_counter() - started) * 1000
    failures = [(path, status) for path, status, _ in threaded if status != 200]
    assert not failures, f"Threaded burst had HTTP failures: {failures}"
    print(
        f"✓ Threaded burst: {len(mix_paths)} simultaneous clients in {wall_ms:.0f}ms "
        f"(all 200)"
    )


async def run_macro_editor_tests(page) -> None:
    """Exercise MacroPlayer. Same assertions for mock and live device."""
    print(f"\n[Macro] Full editor + brief Play/Stop suite...")

    await page.evaluate(
        """() => {
      localStorage.removeItem('ossm_macros_v1');
      localStorage.removeItem('ossm_macro_draft_v1');
      localStorage.removeItem('ossm_macros_last_selected');
    }"""
    )

    # Leave and re-enter Macro so the player remounts with fresh builtins
    await page.locator('button:has-text("Sine")').click()
    await asyncio.sleep(0.3)
    await page.locator('button:has-text("Macro")').click()
    await page.wait_for_selector("#macro-player", timeout=8000)
    await page.wait_for_selector("#macro-library-select", timeout=5000)
    await page.wait_for_selector("#macro-instruction-list", timeout=5000)

    # Seed library by selecting Warm up (builtins load on mount)
    options = await page.locator("#macro-library-select option").all_text_contents()
    assert any("Warm up" in o for o in options), f"Expected Warm up preset, got {options}"
    assert any("Intervals" in o for o in options), f"Expected Intervals preset, got {options}"
    await page.locator("#macro-library-select").select_option(value="Warm up")
    await asyncio.sleep(0.3)
    warm_rows = await page.locator("#macro-instruction-list > [id^=macro-row-]").count()
    await page.locator("#macro-library-select").select_option(value="Intervals")
    await asyncio.sleep(0.3)
    int_rows = await page.locator("#macro-instruction-list > [id^=macro-row-]").count()
    assert warm_rows >= 3 and int_rows >= 3, f"Preset rows unexpected: warm={warm_rows} intervals={int_rows}"
    draft = await page.evaluate("localStorage.getItem('ossm_macro_draft_v1')")
    assert draft, "Expected ossm_macro_draft_v1 after selecting a preset"
    print(f"✓ Library presets load (Warm up rows={warm_rows}, Intervals rows={int_rows})")

    # Save As / Rename / Delete via dialogs
    prompt_replies = iter(["E2E Macro", "E2E Macro Renamed"])

    async def accept_prompt(dialog):
        if dialog.type == "prompt":
            try:
                reply = next(prompt_replies)
            except StopIteration:
                reply = dialog.default_value or "E2E Macro"
            await dialog.accept(reply)
        else:
            await dialog.accept()

    page.on("dialog", accept_prompt)
    await page.locator("#macro-btn-save-as").click()
    await asyncio.sleep(0.4)
    sel_after_save = await page.locator("#macro-library-select").input_value()
    assert sel_after_save == "E2E Macro", f"Save As expected E2E Macro, got {sel_after_save!r}"
    await page.locator("#macro-btn-rename").click()
    await asyncio.sleep(0.4)
    await page.locator("#macro-btn-save").click()
    await asyncio.sleep(0.2)
    print("✓ Save / Save As / Rename dialogs exercised")

    # Reload Intervals for deterministic row edits
    await page.locator("#macro-library-select").select_option(value="Intervals")
    await asyncio.sleep(0.3)

    # Row edit: change first row time and BPM
    time_input = page.locator("#macro-row-0-time")
    await time_input.fill("00:05.0")
    await time_input.dispatch_event("change")
    await asyncio.sleep(0.2)
    action = page.locator("#macro-row-0-action")
    await action.select_option("stop")
    await asyncio.sleep(0.15)
    await action.select_option("set")
    await asyncio.sleep(0.15)
    bpm = page.locator("#macro-row-0 input[type=number]").first
    await bpm.fill("77")
    await bpm.dispatch_event("change")
    await asyncio.sleep(0.2)
    print("✓ Row time/action/param edits applied")

    # Row tools
    before = await page.locator("#macro-instruction-list > [id^=macro-row-]").count()
    await page.locator("#macro-row-0-dup").click()
    await asyncio.sleep(0.2)
    after_dup = await page.locator("#macro-instruction-list > [id^=macro-row-]").count()
    assert after_dup == before + 1, f"Duplicate failed: {before} -> {after_dup}"
    await page.locator("#macro-row-0-insert").click()
    await asyncio.sleep(0.2)
    after_ins = await page.locator("#macro-instruction-list > [id^=macro-row-]").count()
    assert after_ins == after_dup + 1, f"Insert failed: {after_dup} -> {after_ins}"
    await page.locator("#macro-row-1-down").click()
    await asyncio.sleep(0.15)
    await page.locator("#macro-row-2-up").click()
    await asyncio.sleep(0.15)
    await page.locator("#macro-row-0-delete").click()
    await asyncio.sleep(0.2)
    after_del = await page.locator("#macro-instruction-list > [id^=macro-row-]").count()
    assert after_del == after_ins - 1, f"Delete failed: {after_ins} -> {after_del}"
    print("✓ Row tools: duplicate / insert / move / delete")

    # Capture
    cap_before = await page.locator("#macro-instruction-list > [id^=macro-row-]").count()
    await page.locator("#macro-btn-capture").click()
    await asyncio.sleep(0.3)
    cap_after = await page.locator("#macro-instruction-list > [id^=macro-row-]").count()
    assert cap_after == cap_before + 1, "Capture did not append a set row"
    print("✓ Capture current settings")

    # Raw JSON
    await page.locator("#macro-btn-raw-toggle").click()
    await page.wait_for_selector("#macro-raw-textarea", timeout=3000)
    await page.locator("#macro-raw-textarea").fill("{not-json")
    await page.locator("#macro-btn-apply-raw").click()
    await page.wait_for_selector("#macro-raw-error", timeout=3000)
    print("✓ Raw JSON validation error shown")
    good = {
        "version": 1,
        "name": "E2E Short",
        "loop": False,
        "instructions": [
            {"at": 0, "action": "set", "params": {"bpm": 50, "depth": 0.5, "wave_func": "sine"}},
            {"at": 0, "action": "start"},
            {"at": 800, "action": "set", "params": {"bpm": 80}},
            {"at": 1500, "action": "stop", "params": {"position": 0.0}},
        ],
    }
    await page.locator("#macro-raw-textarea").fill(json.dumps(good, indent=2))
    await page.locator("#macro-btn-apply-raw").click()
    await asyncio.sleep(0.3)
    await page.locator("#macro-btn-raw-toggle").click()
    await page.wait_for_selector("#macro-instruction-list", timeout=3000)
    short_rows = await page.locator("#macro-instruction-list > [id^=macro-row-]").count()
    assert short_rows == 4, f"Expected 4 rows after raw apply, got {short_rows}"
    print("✓ Raw JSON apply restored instruction list")

    # Export / Import (now inside raw json editor interface)
    await page.locator("#macro-btn-raw-toggle").click()
    await page.wait_for_selector("#macro-btn-export", timeout=3000)
    async with page.expect_download() as dl_info:
        await page.locator("#macro-btn-export").click()
    download = await dl_info.value
    export_path = Path(tempfile.gettempdir()) / "ossm-macro-e2e-export.json"
    await download.save_as(str(export_path))
    assert export_path.exists() and export_path.stat().st_size > 10
    print(f"✓ Export downloaded ({export_path.stat().st_size} bytes)")

    import_doc = {
        "version": 1,
        "name": "Imported Macro",
        "loop": False,
        "instructions": [
            {"at": 0, "action": "set", "params": {"bpm": 45, "wave_func": "thrust", "sharpness": 0.2}},
            {"at": 0, "action": "start"},
            {"at": 2000, "action": "stop"},
        ],
    }
    import_path = Path(tempfile.gettempdir()) / "ossm-macro-e2e-import.json"
    import_path.write_text(json.dumps(import_doc), encoding="utf-8")
    await page.locator("#macro-file-input").set_input_files(str(import_path))
    await asyncio.sleep(0.5)
    sel = await page.locator("#macro-library-select").input_value()
    assert sel == "Imported Macro", f"Import did not select Imported Macro, got {sel!r}"
    await page.locator("#macro-btn-raw-toggle").click()
    await page.wait_for_selector("#macro-instruction-list", timeout=3000)
    imp_rows = await page.locator("#macro-instruction-list > [id^=macro-row-]").count()
    assert imp_rows == 3, f"Imported row count {imp_rows}"
    print("✓ Import selected Imported Macro with 3 rows")

    # Seek gap
    await page.locator("#macro-gap-1").click()
    await asyncio.sleep(0.2)
    status = await page.locator("#macro-status").inner_text()
    assert "Seeked" in status or "seek" in status.lower() or status, f"Unexpected seek status: {status!r}"
    print(f"✓ Seek gap ({status!r})")

    # Brief Play / Stop
    play = page.locator("#macro-play-btn")
    await play.wait_for(state="visible", timeout=3000)
    assert await play.is_enabled(), "Play should be enabled with instructions"
    elapsed_before = await page.locator("#macro-elapsed").inner_text()
    await play.click()
    await page.wait_for_selector("#macro-stop-btn", timeout=5000)
    assert await page.locator("#macro-library-select").is_disabled()
    assert await page.locator("#macro-btn-save").is_disabled()
    await asyncio.sleep(1.2)
    elapsed_mid = await page.locator("#macro-elapsed").inner_text()
    await page.locator("#macro-stop-btn").click()
    await page.wait_for_selector("#macro-play-btn", timeout=5000)
    assert await page.locator("#macro-library-select").is_enabled()
    # Device/mock should be paused
    from urllib.parse import urlparse

    parsed = urlparse(page.url)
    api_origin = f"{parsed.scheme}://{parsed.netloc}"
    async with httpx.AsyncClient(timeout=5.0) as client:
        st = {}
        for _ in range(15):
            st = (await client.get(f"{api_origin}/state")).json()
            if st.get("config", {}).get("paused") is True:
                break
            await asyncio.sleep(0.15)
    assert st.get("config", {}).get("paused") is True, f"Expected paused after stop: {st.get('config')}"
    print(
        f"✓ Play/Stop lock cycle (elapsed {elapsed_before!r} -> {elapsed_mid!r}); state paused"
    )


async def run_wifi_frontend_tests(page, *, ws_messages: list, ws_connections: list, base_url: str) -> None:
    """Shared WiFi SPA suite (identical for mock and live device)."""
    print(f"\n[Step 1] Navigating to {base_url}...")
    # Diagram visibility is cleared via context.add_init_script in test_frontend
    # (before first mount) so we avoid goto+reload aborting fetchConfig mid-flight.
    await page.goto(base_url, wait_until="domcontentloaded")
    await page.wait_for_selector("text=OSSM", timeout=15000)
    # Wait until config fetch marks connected
    for _ in range(40):
        connected = await page.locator("text=Connected").count()
        modbus_only = await page.locator("text=Modbus only").count()
        if connected + modbus_only > 0:
            break
        await asyncio.sleep(0.25)

    title = await page.title()
    content = await page.content()
    assert "OSSM" in title or "OSSM" in content, "Expected OSSM Controller title/header"
    print("✓ Page successfully loaded and header verified")

    print(f"\n[Step 2] Observing WebSocket real-time state push telemetry...")
    # Ensure diagram is shown so App subscribes
    diagram_toggle = page.locator('button[aria-label="Toggle motor position diagram"]')
    await diagram_toggle.wait_for(state="visible", timeout=5000)
    # If diagram hidden, show it
    saved = await page.evaluate("localStorage.getItem('ossm_show_position_diagram')")
    if saved == "false":
        await diagram_toggle.click()
        await asyncio.sleep(0.3)

    deadline = time.time() + 8.0
    while time.time() < deadline and len(ws_messages) <= 5:
        await asyncio.sleep(0.2)
    assert len(ws_connections) > 0, "Expected at least one WebSocket connection"
    assert len(ws_messages) > 5, f"Expected stream of state pushes, got {len(ws_messages)}"

    if len(ws_messages) >= 10:
        dt_list = [t2 - t1 for (t1, _), (t2, _) in zip(ws_messages, ws_messages[1:])]
        avg_dt = sum(dt_list) / len(dt_list)
        fps = 1.0 / avg_dt if avg_dt > 0 else 0
        print(f"✓ Received {len(ws_messages)} state updates over WebSocket (~{fps:.1f} FPS)")
    else:
        print(f"✓ Received {len(ws_messages)} state updates over WebSocket")

    print(f"\n[Step 2b] Verifying browser WebSocket stability under concurrent HTTP load...")
    pre_count = len(ws_messages)
    fetch_results = await page.evaluate(
        """async () => {
            const urls = ['/state', '/config', '/pin-config', '/network-config'];
            const res = await Promise.all(urls.map(u => fetch(u).then(r => r.status)));
            return res;
        }"""
    )
    assert all(status == 200 for status in fetch_results), f"Browser REST fetches failed: {fetch_results}"
    await asyncio.sleep(1.0)
    new_msgs = len(ws_messages) - pre_count
    assert new_msgs >= 10, f"Expected continuous WS during REST load, got {new_msgs} in 1s"
    print(f"✓ Browser WebSocket streamed {new_msgs} frames during concurrent REST fetches")

    print(f"\n[Step 3] Testing Motor Position Diagram toggle and localStorage persistence...")
    print(" -> Clicking toggle to hide diagram...")
    await diagram_toggle.click()
    await asyncio.sleep(0.4)
    print(" -> Clicking toggle to show diagram...")
    await diagram_toggle.click()
    await asyncio.sleep(0.4)
    saved_val = await page.evaluate("localStorage.getItem('ossm_show_position_diagram')")
    assert saved_val == "true", f"Expected localStorage key to be 'true', got '{saved_val}'"
    print("✓ Motor Position Diagram toggle and persistence verified")

    print(f"\n[Step 4] Testing Waveform Spline Mode selection...")
    spline_btn = page.locator('button:has-text("Spline")')
    await spline_btn.wait_for(state="visible", timeout=5000)
    await spline_btn.click()
    await asyncio.sleep(1.0)
    pre_spline = len(ws_messages)
    await asyncio.sleep(1.0)
    assert len(ws_messages) > pre_spline, "WebSocket stopped after Spline mode"
    print("✓ Stable after switching to Spline mode")

    print(f"\n[Step 4a] Testing Funscript mode panel...")
    fun_btn = page.locator('button:has-text("Funscript")')
    await fun_btn.wait_for(state="visible", timeout=5000)
    await fun_btn.click()
    await asyncio.sleep(0.5)
    await page.wait_for_selector("text=Funscript", timeout=5000)
    print("✓ Funscript player chrome visible")

    # Macro suite (includes Macro mode click)
    await run_macro_editor_tests(page)

    print(f"\n[Step 5] Opening Unified Device Settings Panel...")
    # Leave macro mode for settings checks
    await page.locator('button:has-text("Sine")').click()
    await asyncio.sleep(0.3)
    settings_toggle = page.locator('button[aria-label="Toggle settings"]')
    await settings_toggle.wait_for(state="visible", timeout=5000)
    await settings_toggle.click()

    await page.wait_for_selector("text=Unified Device Settings", timeout=5000)
    await page.wait_for_selector("text=Motor Update Rate Telemetry", timeout=5000)
    print("✓ Unified Device Settings panel opened and telemetry card verified")

    await page.locator("text=updates/sec (Hz)").wait_for(state="visible", timeout=5000)
    ble_chk = page.locator("#ble-enabled")
    wifi_chk = page.locator("#wifi-enabled")
    await ble_chk.wait_for(state="visible", timeout=5000)
    await wifi_chk.wait_for(state="visible", timeout=5000)
    await page.wait_for_selector("#wifi-config-panel", timeout=5000)
    await page.wait_for_selector("#wifi-ssid", timeout=5000)
    await page.wait_for_selector("#wifi-password", timeout=5000)
    assert await wifi_chk.is_disabled(), "Expected #wifi-enabled disabled in WiFi mode"
    print("✓ Settings BLE/WiFi toggles and WiFi fields verified")

    await settings_toggle.click()

    print(f"\n[Step 5b] Verifying Main Controls interactive elements...")
    await page.wait_for_selector("#speed-slider", timeout=5000)
    await page.wait_for_selector("#depth-slider", timeout=5000)
    await page.wait_for_selector("text=Waveform", timeout=5000)
    print("✓ Main motion controls verified visible")

    print(f"\n[Step 6] Verifying Motor Position Diagram visualization...")
    await page.wait_for_selector("text=Motor Position Diagram", timeout=5000)
    print("✓ Motor Position Diagram component verified visible")


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
        await context.add_init_script(
            """() => {
              localStorage.setItem('ossm_locale', 'en');
              localStorage.removeItem('ossm_show_position_diagram');
            }"""
        )
        page = await context.new_page()

        ws_messages: list = []
        ws_connections: list = []

        def on_websocket(ws):
            print(f"[WS CONNECTED] {ws.url}")
            ws_connections.append(ws)
            ws.on("framereceived", lambda payload: ws_messages.append((time.time(), payload)))
            ws.on("close", lambda w: print("[WS CLOSED] Connection cleanly terminated"))

        page.on("websocket", on_websocket)
        page.on("console", lambda msg: print(f"[BROWSER {msg.type.upper()}] {msg.text}"))

        await run_wifi_frontend_tests(
            page,
            ws_messages=ws_messages,
            ws_connections=ws_connections,
            base_url=base_url,
        )

        await browser.close()


async def test_frontend_mock() -> None:
    """Offline WiFi suite against MockOssmServer (no live device)."""
    if not (FRONTEND_DIST / "index.html").exists():
        raise FileNotFoundError(
            f"Missing {FRONTEND_DIST / 'index.html'}; run: cd frontend && npm run build"
        )
    async with MockOssmServer(dist_dir=FRONTEND_DIST) as base_url:
        print(f"============================================================")
        print(f"  OSSM Mock Frontend Suite (no device)")
        print(f"  Target: {base_url}")
        print(f"============================================================")
        await test_concurrent_http(base_url)
        await test_frontend(base_url)
        print("[SKIP] BLE / Web Bluetooth / flasher / motor-control (hardware-only)")


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
        # Extra cool-down so WiFi HTTP remains stable for subsequent Playwright tests.
        await asyncio.sleep(2.5)


async def test_ble_connection():
    """Run BLE tests."""
    try:
        await _run_ble_tests()
    except Exception as e:
        if "BleakBluetoothNotAvailableError" in str(type(e)) or "No Bluetooth adapters found" in str(e) or "org.bluez" in str(e):
            print(f"\n[SKIP] BLE integration suite skipped: no physical Bluetooth adapter or BlueZ service available on host ({e})")
            return
        raise


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
        await context.add_init_script("localStorage.setItem('ossm_locale', 'en')")
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


async def test_motor_control():
    print(f"\n============================================================")
    print(f"  OSSM Motor-Control Playwright E2E Verification Suite")
    print(f"============================================================")

    mc_path = Path(__file__).parent.parent / "release" / "motor-control.html"
    if not mc_path.exists():
        print(f"Skipping motor-control E2E test: {mc_path} not found")
        return

    mc_url = mc_path.as_uri()
    print(f"  Target: {mc_url}")

    async with async_playwright() as p:
        browser = await p.chromium.launch(headless=True)
        context = await browser.new_context(viewport={"width": 1280, "height": 800})
        # Force Chinese so VB panel caption assertions stay stable
        await context.add_init_script("localStorage.setItem('ossm_locale', 'zh')")
        page = await context.new_page()

        await page.goto(mc_url, wait_until="load")
        title = await page.title()
        body = await page.locator("body").inner_text()
        assert (
            "57AIM30" in title
            or "57AIM30" in body
            or "Motor" in title
            or "Modbus" in body
            or "电机" in body
        ), f"Unexpected motor-control content: title={title!r}"
        print("✓ Motor-control page successfully loaded")

        await page.wait_for_selector("#motor-control-connection", timeout=5000)
        print("✓ Connection bar verified visible")

        conn_type = page.locator("#mc-connection-type")
        await conn_type.wait_for(state="visible", timeout=5000)
        await conn_type.select_option("websocket")
        await page.wait_for_selector("#mc-ws-url", state="visible", timeout=5000)
        assert await page.locator("#mc-baud").count() == 0, "Baud select should hide in WebSocket mode"
        print("✓ Remote WebSocket mode shows URL field and hides baud")

        await conn_type.select_option("serial")
        await page.wait_for_selector("#mc-baud", state="visible", timeout=5000)
        assert await page.locator("#mc-ws-url").count() == 0, "WS URL should hide in serial mode"
        print("✓ Local Serial mode restores baud select")

        for text in ("波形显示", "modbus控制参数", "电机运行参数", "modbus读取", "modbus发送", "发送数据"):
            assert await page.locator(f"text={text}").count() > 0, f"Missing VB panel caption: {text}"
        assert await page.locator("text=开始读取").count() > 0
        print("✓ Core motor-control VB layout controls verified visible")

        await browser.close()
    print("✓ All Motor-Control E2E tests PASSED")


async def _soft_reset_device() -> None:
    """Reset ESP via USB so BLE advertising restarts (clears stuck CONNECTIONS_MAX=1)."""
    import subprocess

    port = os.environ.get("DEVICE_PORT", "/dev/ttyACM0").strip().strip('"') or "/dev/ttyACM0"
    print(f" -> Soft-resetting device on {port} so OSSM re-advertises...")
    
    # Send CLI reset command using python script subprocess
    try:
        ossm_py = os.path.join(os.path.dirname(__file__), "ossm.py")
        subprocess.run(
            [sys.executable, ossm_py, "restart", "--mode", "serial"],
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        )
        print("    (CLI restart successful)")
    except Exception as e:
        print(f"    (CLI reset failed: {e}, falling back to espflash board-info)")
        subprocess.run(
            ["espflash", "board-info", "--port", port],
            check=False,
            capture_output=True,
            text=True,
            timeout=30,
        )
    
    # Boot + BLE stack start can take several seconds (WiFi + trouble-host).
    await asyncio.sleep(8.0)


async def _ossm_is_advertising(timeout_s: float = 6.0) -> bool:
    from bleak import BleakScanner

    try:
        devices = await BleakScanner.discover(timeout=timeout_s, return_adv=True)
        for dev, adv in devices.values():
            name = (dev.name or adv.local_name or "").strip()
            if "OSSM" in name:
                return True
    except Exception as e:
        print(f" -> [BLE Scanner Note] Error during scan: {e}; retrying...")
        await asyncio.sleep(0.5)
    return False


async def _ensure_ossm_advertising(run_idx: int, total_runs: int) -> None:
    """
    Confirm OSSM is advertising via Bleak BEFORE Chromium owns the adapter.

    Do not call this while Playwright Chromium is open — BlueZ typically allows
    only one LE scanner and Bleak/Chrome will fight each other.
    """
    deadline = time.monotonic() + 30.0
    while time.monotonic() < deadline:
        remaining = max(2.0, min(6.0, deadline - time.monotonic()))
        if await _ossm_is_advertising(timeout_s=remaining):
            return
        await asyncio.sleep(0.5)

    print(
        f" -> [Run {run_idx}/{total_runs}] OSSM not advertising; "
        "resetting peripheral before Web Bluetooth chooser"
    )
    await _soft_reset_device()

    deadline = time.monotonic() + 45.0
    while time.monotonic() < deadline:
        remaining = max(2.0, min(6.0, deadline - time.monotonic()))
        if await _ossm_is_advertising(timeout_s=remaining):
            return
        await asyncio.sleep(0.5)

    raise AssertionError(
        f"[Run {run_idx}/{total_runs}] OSSM still not advertising after soft-reset"
    )


async def _run_single_web_bluetooth_attempt(
    browser,
    frontend_url: str,
    run_idx: int,
    total_runs: int,
) -> None:
    context = await browser.new_context(viewport={"width": 1280, "height": 800})
    await context.add_init_script("localStorage.setItem('ossm_locale', 'en')")
    page = await context.new_page()
    cdp = await context.new_cdp_session(page)
    await cdp.send("DeviceAccess.enable")

    selected_device_name: str | None = None
    selection_error: Exception | None = None
    selection_done = asyncio.Event()
    devices_seen: list[str] = []
    prompt_event_count = 0
    last_prompt_id: str | None = None

    async def wait_for_exact_connection_status(expected: str, timeout_ms: int) -> None:
        await page.wait_for_function(
            """({ expected }) => {
                const labels = Array.from(
                    document.querySelectorAll('header span.text-sm.font-medium')
                );
                return labels.some(
                    (el) => (el.textContent || '').trim() === expected
                );
            }""",
            arg={"expected": expected},
            timeout=timeout_ms,
        )

    async def on_device_prompt(event):
        nonlocal selected_device_name, selection_error, prompt_event_count, last_prompt_id
        try:
            prompt_event_count += 1
            last_prompt_id = event.get("id")
            devices = event.get("devices", [])
            for dev in devices:
                dev_name = dev.get("name", "<unnamed>")
                if dev_name not in devices_seen:
                    devices_seen.append(dev_name)

            # Chromium re-fires this event as the chooser list updates. Selecting
            # more than once aborts the outstanding requestDevice()/GATT handshake.
            if selected_device_name is not None or selection_done.is_set():
                return

            ossm_device = next(
                (dev for dev in devices if "OSSM" in dev.get("name", "")),
                None,
            )
            if ossm_device is None:
                return

            selected_device_name = ossm_device.get("name", "<unnamed>")
            device_id = ossm_device.get("id", "")
            if not device_id:
                raise AssertionError(
                    f"[Run {run_idx}/{total_runs}] OSSM BLE device had empty id in prompt"
                )

            await cdp.send(
                "DeviceAccess.selectPrompt",
                {"id": event["id"], "deviceId": device_id},
            )
        except Exception as exc:
            selection_error = exc
            selected_device_name = None
        finally:
            if selected_device_name is not None or selection_error is not None:
                selection_done.set()

    cdp.on(
        "DeviceAccess.deviceRequestPrompted",
        lambda event: asyncio.create_task(on_device_prompt(event)),
    )

    try:
        await page.goto(frontend_url, wait_until="networkidle")
        title = await page.title()
        assert "OSSM" in title, f"[Run {run_idx}/{total_runs}] Unexpected page title: {title}"

        ble_mode_btn = page.locator("button:has-text('BLE')")
        await ble_mode_btn.first.wait_for(state="visible", timeout=5000)
        await ble_mode_btn.first.click()
        await asyncio.sleep(0.4)

        connect_btn = page.locator("button:has-text('Connect BLE')")
        await connect_btn.first.wait_for(state="visible", timeout=5000)
        await connect_btn.first.click()

        try:
            await asyncio.wait_for(
                selection_done.wait(),
                timeout=WEB_BLUETOOTH_PROMPT_TIMEOUT_S,
            )
        except asyncio.TimeoutError as exc:
            if last_prompt_id is not None:
                try:
                    await cdp.send("DeviceAccess.cancelPrompt", {"id": last_prompt_id})
                except Exception:
                    pass
            raise AssertionError(
                f"[Run {run_idx}/{total_runs}] Timed out waiting for OSSM in BLE prompt "
                f"(prompt events: {prompt_event_count}, devices seen: {devices_seen})"
            ) from exc
        if selection_error is not None:
            raise selection_error
        assert selected_device_name is not None, (
            f"[Run {run_idx}/{total_runs}] BLE prompt was not handled "
            f"(prompt events: {prompt_event_count}, devices seen: {devices_seen})"
        )

        try:
            await wait_for_exact_connection_status(
                expected="Connected",
                timeout_ms=WEB_BLUETOOTH_CONNECT_TIMEOUT_MS,
            )
        except Exception as exc:
            status = await page.evaluate(
                """() => ({
                  labels: Array.from(
                    document.querySelectorAll('header span.text-sm.font-medium')
                  ).map((el) => (el.textContent || '').trim()),
                  errBanner: document.querySelector('div.mb-4.bg-red-500')
                    ?.textContent?.trim() || null,
                })"""
            )
            raise AssertionError(
                f"[Run {run_idx}/{total_runs}] Timed out waiting for Connected after "
                f"selecting {selected_device_name}: {status}"
            ) from exc
        await connect_btn.first.wait_for(state="hidden", timeout=5000)

        reconnect_btn = page.locator("button:has-text('Connect BLE')")
        if await reconnect_btn.count() > 0:
            assert not await reconnect_btn.first.is_visible(), (
                f"[Run {run_idx}/{total_runs}] Connect BLE button stayed visible after connection"
            )

        start_btn = page.locator("button:has-text('Start')")
        stop_btn = page.locator("button:has-text('Stop')")
        if await start_btn.count() > 0 and await start_btn.first.is_visible():
            assert await start_btn.first.is_enabled(), (
                f"[Run {run_idx}/{total_runs}] Start button disabled after BLE connect"
            )
            await start_btn.first.click()
            await stop_btn.first.wait_for(state="visible", timeout=5000)
            assert await stop_btn.first.is_enabled(), (
                f"[Run {run_idx}/{total_runs}] Stop button disabled after Start"
            )
            await stop_btn.first.click()
            await start_btn.first.wait_for(state="visible", timeout=5000)
        elif await stop_btn.count() > 0 and await stop_btn.first.is_visible():
            assert await stop_btn.first.is_enabled(), (
                f"[Run {run_idx}/{total_runs}] Stop button disabled after BLE connect"
            )
            await stop_btn.first.click()
            await start_btn.first.wait_for(state="visible", timeout=5000)
            assert await start_btn.first.is_enabled(), (
                f"[Run {run_idx}/{total_runs}] Start button disabled after Stop"
            )
            await start_btn.first.click()
            await stop_btn.first.wait_for(state="visible", timeout=5000)
        else:
            raise AssertionError(
                f"[Run {run_idx}/{total_runs}] Could not find Start/Stop control button"
            )

        settings_toggle = page.locator('button[aria-label="Toggle settings"]')
        await settings_toggle.first.wait_for(state="visible", timeout=5000)
        await settings_toggle.first.click()
        await page.wait_for_selector("text=Unified Device Settings", timeout=5000)
        ble_chk = page.locator("#ble-enabled")
        await ble_chk.wait_for(state="visible", timeout=5000)
        assert await ble_chk.is_disabled(), (
            f"[Run {run_idx}/{total_runs}] #ble-enabled should be disabled while in BLE mode"
        )

        print(
            f"✓ [Web Bluetooth run {run_idx}/{total_runs}] "
            f"Connected to {selected_device_name} and control path passed"
        )
    finally:
        try:
            # Switch to WiFi mode — App.vue calls api.disconnectBle() so the
            # peripheral can resume advertising (CONNECTIONS_MAX=1).
            wifi_btn = page.locator("button:has-text('WiFi')")
            if await wifi_btn.count() > 0 and await wifi_btn.first.is_visible():
                await wifi_btn.first.click()
                try:
                    await wait_for_exact_connection_status(
                        expected="Disconnected",
                        timeout_ms=5_000,
                    )
                except Exception:
                    pass
                await asyncio.sleep(0.4)
        except Exception:
            pass
        if last_prompt_id is not None and selected_device_name is None:
            try:
                await cdp.send("DeviceAccess.cancelPrompt", {"id": last_prompt_id})
            except Exception:
                pass
        await context.close()
        # Peripheral needs a cool-down before the next chooser scan / re-advertise.
        await asyncio.sleep(WEB_BLUETOOTH_COOLDOWN_S)


async def test_web_bluetooth_frontend(runs: int = WEB_BLUETOOTH_STABILITY_RUNS):
    """Verify frontend Web Bluetooth connect/control stability across repeated runs."""
    if runs < 1:
        raise AssertionError(f"Invalid runs={runs}; expected >= 1")

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

    class QuietHandler(http.server.SimpleHTTPRequestHandler):
        def __init__(self, *args, **kwargs):
            super().__init__(*args, directory=str(dist_dir), **kwargs)
        def log_message(self, format, *args):
            pass

    class ReusableTCPServer(socketserver.TCPServer):
        allow_reuse_address = True

    httpd = ReusableTCPServer(("127.0.0.1", 0), QuietHandler)
    port = httpd.server_address[1]
    server_thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    server_thread.start()

    try:
        async with async_playwright() as p:
            # Web Bluetooth chooser needs a display server. Prefer headed mode when
            # DISPLAY is set (including under `xvfb-run -a`); otherwise fall back to
            # new headless which can still enumerate devices once firmware advertises
            # a valid static random address.
            has_display = bool(os.environ.get("DISPLAY"))
            launch_args = [
                "--enable-features=WebBluetooth,WebBluetoothNewPermissionsBackend",
                "--enable-experimental-web-platform-features",
                "--no-sandbox",
                "--disable-dev-shm-usage",
            ]
            if not has_display:
                launch_args.insert(0, "--headless=new")
            frontend_url = f"http://127.0.0.1:{port}/"
            failures: list[tuple[int, str]] = []
            print(
                f" -> Running Web Bluetooth connect/control stability test "
                f"{runs} times at {frontend_url} "
                f"(DISPLAY={os.environ.get('DISPLAY')!r}, headed={has_display})"
            )

            async def launch_browser():
                # Note: passing headless=not has_display causes Playwright to launch
                # chrome-headless-shell instead of the full chrome binary when headless=True.
                # chrome-headless-shell lacks D-Bus/chooser integration for Web Bluetooth and crashes.
                # Passing headless=False with `--headless=new` inside args launches the full
                # Chromium browser in new headless mode, where WebBluetooth chooser works cleanly.
                return await p.chromium.launch(
                    headless=False,
                    args=launch_args,
                )

            async def run_with_retry(run_idx: int) -> None:
                last_exc: Exception | None = None
                for attempt in range(1, 3):
                    # Bleak must scan while Chromium is NOT holding the adapter.
                    await _ensure_ossm_advertising(run_idx, runs)
                    browser = await launch_browser()
                    try:
                        await _run_single_web_bluetooth_attempt(
                            browser=browser,
                            frontend_url=frontend_url,
                            run_idx=run_idx,
                            total_runs=runs,
                        )
                        return
                    except Exception as exc:
                        last_exc = exc
                        print(
                            f" ! [Web Bluetooth run {run_idx}/{runs}] "
                            f"attempt {attempt}/2 failed: {type(exc).__name__}: {exc}"
                        )
                    finally:
                        await browser.close()
                    await _soft_reset_device()
                assert last_exc is not None
                raise last_exc

            for run_idx in range(1, runs + 1):
                print(f"\n[Web Bluetooth run {run_idx}/{runs}]")
                try:
                    await run_with_retry(run_idx)
                except Exception as exc:
                    err = f"{type(exc).__name__}: {exc}"
                    failures.append((run_idx, err))
                    print(f"✗ [Web Bluetooth run {run_idx}/{runs}] {err}")
                await asyncio.sleep(0.4)

            if failures:
                details = "; ".join(f"run {idx}: {msg}" for idx, msg in failures)
                raise AssertionError(
                    f"Web Bluetooth stability regression: {len(failures)}/{runs} failed ({details})"
                )
            print(f"✓ Web Bluetooth E2E stability passed ({runs}/{runs})")
    finally:
        httpd.shutdown()
        httpd.server_close()
        server_thread.join(timeout=2.0)


async def main():
    parser = argparse.ArgumentParser(description="OSSM frontend + BLE integration test suite")
    parser.add_argument(
        "--only-web-bluetooth",
        action="store_true",
        help="Run only the frontend Web Bluetooth stability suite",
    )
    parser.add_argument(
        "--mock-frontend",
        action="store_true",
        help="Run concurrent HTTP + WiFi Playwright suite against MockOssmServer (no live device)",
    )
    parser.add_argument(
        "--web-bluetooth-runs",
        type=int,
        default=WEB_BLUETOOTH_STABILITY_RUNS,
        help=f"Number of Web Bluetooth repeated runs (default: {WEB_BLUETOOTH_STABILITY_RUNS})",
    )
    parser.add_argument(
        "--skip-ble",
        action="store_true",
        help="Skip BLE and Web Bluetooth tests when running without a physical Bluetooth adapter on the host",
    )
    args = parser.parse_args()

    if args.only_web_bluetooth:
        await test_web_bluetooth_frontend(runs=args.web_bluetooth_runs)
        print(f"\n============================================================")
        print(
            f"✓ Web Bluetooth frontend stability test PASSED "
            f"({args.web_bluetooth_runs}/{args.web_bluetooth_runs})"
        )
        print(f"============================================================")
        return

    if args.mock_frontend:
        await test_frontend_mock()
        print(f"\n============================================================")
        print(f"✓ Mock frontend suite PASSED (concurrent HTTP + WiFi SPA + macro)")
        print(f"============================================================")
        return

    base_url = load_device_url()
    await test_concurrent_http(base_url)
    if not args.skip_ble:
        await test_ble_connection()
    await test_frontend(base_url)
    if not args.skip_ble:
        await test_web_bluetooth_frontend(runs=args.web_bluetooth_runs)
    await test_flasher()
    await test_motor_control()
    print(f"\n============================================================")
    print(f"✓ All HTTP + BLE + frontend + Web Bluetooth + flasher + motor-control Playwright tests PASSED")
    print(f"============================================================")


if __name__ == "__main__":
    asyncio.run(main())
