#!/usr/bin/env -S uv run
"""Reproduce Macro Player playback bug where clicking play resets wave_func to sine."""

import asyncio
from pathlib import Path
from playwright.async_api import async_playwright
from test_frontend import MockOssmServer, FRONTEND_DIST

async def main():
    print("[E2E Test] Starting reproduction test against MockOssmServer...")
    async with MockOssmServer(dist_dir=FRONTEND_DIST) as base_url:
        async with async_playwright() as p:
            browser = await p.chromium.launch(headless=True)
            context = await browser.new_context()
            await context.add_init_script("""() => {
              localStorage.setItem('ossm_locale', 'en');
            }""")
            page = await context.new_page()
            
            # 1. Open the app
            print("[E2E Test] Navigating to OSSM Web Application...")
            await page.goto(base_url, wait_until="networkidle")
            
            # 2. Click Macro tab
            print("[E2E Test] Clicking Macro tab...")
            await page.locator('button:has-text("Macro")').click()
            
            # Check that macro player is visible
            await page.wait_for_selector("#macro-player", timeout=8000)
            print("[E2E Test] Macro Player interface is visible.")
            
            # Select 'Warm up' preset so that first instruction sets wave_func to 'sine'
            await page.locator("#macro-library-select").select_option(value="Warm up")
            await asyncio.sleep(0.3)
            
            # 3. Click Play inside Macro Player
            print("[E2E Test] Clicking Play button inside Macro Player...")
            await page.click("#macro-play-btn")
            
            # 4. Wait 1.5 seconds for CONTROL_REFRESH_INTERVAL_MS (1000ms) poll / telemetry sync
            print("[E2E Test] Waiting 1.5s for background telemetry poll (which returns wave_func='sine')...")
            await asyncio.sleep(1.5)
            
            # 5. Check if #macro-player is still visible or if it disappeared and fell back to MainControl (sine wave)
            macro_player_visible = await page.is_visible("#macro-player")
            main_control_visible = await page.is_visible("#main-control-panel")
            
            print(f"[E2E Test] After Play clicked + poll -> #macro-player visible: {macro_player_visible}")
            print(f"[E2E Test] After Play clicked + poll -> #main-control-panel visible: {main_control_visible}")
            
            if not macro_player_visible and main_control_visible:
                print("====================================================================")
                print("✓ BUG REPRODUCED: Clicking Play destroyed Macro Player and reverted to Sine Waveform!")
                print("====================================================================")
            else:
                print("Macro Player stayed visible.")
            
            await browser.close()

if __name__ == "__main__":
    asyncio.run(main())
