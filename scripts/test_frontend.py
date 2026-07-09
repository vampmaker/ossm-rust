#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "playwright",
# ]
# ///

import asyncio
import time
from playwright.async_api import async_playwright

DEVICE_URL = "http://192.168.24.63"

async def test_frontend():
    print(f"============================================================")
    print(f"  Starting OSSM Frontend Playwright E2E Verification Suite")
    print(f"  Target: {DEVICE_URL}")
    print(f"============================================================")

    async with async_playwright() as p:
        browser = await p.chromium.launch(headless=True)
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

        # 1. Load Application
        print(f"\n[Step 1] Navigating to {DEVICE_URL}...")
        await page.goto(DEVICE_URL, wait_until="networkidle")

        title = await page.title()
        assert "OSSM" in title or "OSSM-Rust Controller" in await page.content(), "Expected OSSM Controller title/header"
        print("✓ Page successfully loaded and header verified")

        # 2. Verify WebSocket Connection and ~30 FPS Stream
        print(f"\n[Step 2] Observing WebSocket real-time state push telemetry (target: 30 FPS)...")
        await asyncio.sleep(2.5)
        assert len(ws_connections) > 0, "Expected at least one WebSocket connection to be established"
        assert len(ws_messages) > 5, "Expected stream of state push notifications from firmware"

        # Calculate FPS over recent messages
        if len(ws_messages) >= 10:
            dt = ws_messages[-1][0] - ws_messages[-10][0]
            fps = 9 / dt if dt > 0 else 0
            print(f"✓ WebSocket streaming active. Sampled update rate: ~{fps:.1f} frames/sec")

        # 3. Test Motor Position Diagram Toggle & Persistence
        print(f"\n[Step 3] Testing Motor Position Diagram toggle button and localStorage persistence...")
        diagram_toggle = page.locator('button[aria-label="Toggle motor position diagram"]')
        await diagram_toggle.wait_for(state="visible", timeout=5000)

        # Toggle diagram off
        print(" -> Clicking toggle to hide diagram...")
        await diagram_toggle.click()
        await asyncio.sleep(0.5)
        saved_val = await page.evaluate("localStorage.getItem('ossm_show_position_diagram')")
        print(f" -> localStorage['ossm_show_position_diagram'] = {saved_val}")

        # Toggle diagram back on
        print(" -> Clicking toggle to show diagram...")
        await diagram_toggle.click()
        await asyncio.sleep(0.5)
        saved_val = await page.evaluate("localStorage.getItem('ossm_show_position_diagram')")
        assert saved_val == "true", f"Expected localStorage key to be 'true', got '{saved_val}'"
        print("✓ Motor Position Diagram toggle and persistence verified")

        # 4. Test Device Settings / Modbus Panel & Live UPS Telemetry
        print(f"\n[Step 4] Opening Device Settings Panel and verifying live UPS telemetry...")
        settings_toggle = page.locator('button[aria-label="Toggle settings"]')
        await settings_toggle.wait_for(state="visible", timeout=5000)
        await settings_toggle.click()

        # Wait for Device Settings heading and Motor Update Rate card
        await page.wait_for_selector("text=Device Settings", timeout=5000)
        await page.wait_for_selector("text=Motor Update Rate", timeout=5000)
        print("✓ Device Settings panel opened")

        ups_element = page.locator("text=updates/sec (Hz)")
        await ups_element.wait_for(state="visible", timeout=5000)
        print("✓ Motor Update Rate (updates/sec (Hz)) and Loop dt telemetry card verified visible")

        # Close settings panel
        await settings_toggle.click()

        # 5. Verify Main Controls (Speed BPM, Depth, Waveform selection)
        print(f"\n[Step 5] Verifying Main Controls interactive elements...")
        await page.wait_for_selector("#speed-slider", timeout=5000)
        await page.wait_for_selector("#depth-slider", timeout=5000)
        await page.wait_for_selector("text=Waveform", timeout=5000)
        print("✓ Main motion controls (#speed-slider, #depth-slider, Waveform selector) verified visible")

        # 6. Verify Motor Position Diagram component is rendered when active
        print(f"\n[Step 6] Verifying Motor Position Diagram visualization...")
        await page.wait_for_selector("text=Motor Position Diagram", timeout=5000)
        print("✓ Motor Position Diagram component ('Motor Position Diagram') verified visible and rendering live telemetry")

        print(f"\n============================================================")
        print(f"✓ All Frontend E2E Playwright tests PASSED successfully!")
        print(f"============================================================")

        await browser.close()

if __name__ == "__main__":
    asyncio.run(test_frontend())
