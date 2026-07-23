#!/usr/bin/env -S uv run --script
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
"""Take high-resolution Playwright screenshots of the OSSM Macro Player (`MacroPlayer.vue`)

Captures:
1. Full macro player interface in light theme (`macro-player-full.png`)
2. Element screenshot of the macro player container (`macro-player-element.png`)
3. Detailed screenshot of the top bar highlighting Save, Save As, Rename, Delete badge buttons (`macro-player-buttons.png`)
"""

import asyncio
import os
import shutil
from pathlib import Path
from playwright.async_api import async_playwright

from test_frontend import MockOssmServer, FRONTEND_DIST

LOCAL_DIR = Path(__file__).parent.parent / "screenshots"


async def take_screenshots():
    if not (FRONTEND_DIST / "index.html").exists():
        raise FileNotFoundError(f"Missing {FRONTEND_DIST / 'index.html'}; please build frontend first.")

    LOCAL_DIR.mkdir(parents=True, exist_ok=True)

    async with MockOssmServer(dist_dir=FRONTEND_DIST) as base_url:
        print(f"[Screenshot Tool] Launched MockOssmServer at {base_url}")
        async with async_playwright() as p:
            browser = await p.chromium.launch(
                headless=True,
                args=["--headless=new"]
            )
            context = await browser.new_context(
                viewport={"width": 1400, "height": 950},
                device_scale_factor=2  # High-res 2x retina shots for crisp text & icons
            )
            await context.add_init_script("""() => {
              localStorage.setItem('ossm_locale', 'en');
            }""")
            page = await context.new_page()

            print("[Screenshot Tool] Navigating to OSSM Web Application...")
            await page.goto(base_url, wait_until="networkidle")

            # Click on Macro tab
            print("[Screenshot Tool] Opening Macro Player tab...")
            await page.locator('button:has-text("Macro")').click()
            await page.wait_for_selector("#macro-player", timeout=8000)
            await page.wait_for_selector("#macro-library-select", timeout=5000)

            # Select 'Intervals' preset for rich data visualization
            await page.locator("#macro-library-select").select_option(value="Intervals")
            await asyncio.sleep(0.5)  # Let transitions settle

            # 1. Full Page Screenshot
            full_path = LOCAL_DIR / "macro_player_full.png"
            await page.screenshot(path=full_path, full_page=True)
            print(f"✓ Saved full page screenshot: {full_path}")

            # 2. Macro Player Element Screenshot
            element_path = LOCAL_DIR / "macro_player_element.png"
            await page.locator("#macro-player").screenshot(path=element_path)
            print(f"✓ Saved element screenshot: {element_path}")

            # 3. Top Bar / Buttons Detailed Screenshot
            buttons_path = LOCAL_DIR / "macro_player_buttons.png"
            await page.locator("#macro-player > div").first.screenshot(path=buttons_path)
            print(f"✓ Saved buttons detailed screenshot: {buttons_path}")

            # 4. Raw JSON Editor Interface Screenshot (with relocated Import/Export buttons)
            await page.locator("#macro-btn-raw-toggle").click()
            await page.wait_for_selector("#macro-raw-textarea", timeout=3000)
            await asyncio.sleep(0.3)
            raw_path = LOCAL_DIR / "macro_player_raw_json.png"
            await page.locator("#macro-player").screenshot(path=raw_path)
            print(f"✓ Saved raw JSON editor interface screenshot: {raw_path}")

            await browser.close()

    print("\n[Screenshot Tool] All screenshots captured successfully!")


if __name__ == "__main__":
    asyncio.run(take_screenshots())
