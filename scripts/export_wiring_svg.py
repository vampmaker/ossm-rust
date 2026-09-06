#!/usr/bin/env -S uv run
"""Export assets wiring diagram SVGs via Playwright + web/apps/wiring-diagram."""

from __future__ import annotations

import asyncio
from pathlib import Path
from urllib.parse import unquote

from playwright.async_api import async_playwright

REPO = Path(__file__).resolve().parents[1]
ASSETS = REPO / "assets"
DIST_HTML = REPO / "web" / "apps" / "wiring-diagram" / "dist" / "index.html"

JOBS = [
    ("std", ASSETS / "wiring_diagram_std.svg"),
    ("std-zh", ASSETS / "wiring_diagram_std_zh.svg"),
    ("esp", ASSETS / "wiring_diagram.svg"),
    ("esp-zh", ASSETS / "wiring_diagram_zh.svg"),
]


async def export_one(page, route: str, svg: Path) -> None:
    url = f"{DIST_HTML.resolve().as_uri()}#/{route}"
    await page.goto(url, wait_until="networkidle")
    await page.wait_for_selector("#diagram")
    await page.wait_for_selector("#wires path")
    await page.wait_for_timeout(300)
    data_url = await page.evaluate(
        """async () => {
          const el = document.getElementById('diagram');
          if (!el) throw new Error('missing #diagram');
          if (typeof window.htmlToImage === 'undefined' || !window.htmlToImage.toSvg) {
            throw new Error('html-to-image did not load');
          }
          return await window.htmlToImage.toSvg(el, { pixelRatio: 1 });
        }"""
    )
    if not isinstance(data_url, str) or not data_url.startswith("data:image/svg+xml"):
        raise RuntimeError(f"unexpected toSvg result from route {route}")
    _, payload = data_url.split(",", 1)
    if ";base64," in data_url[:80]:
        import base64

        raw = base64.b64decode(payload)
    else:
        raw = unquote(payload).encode("utf-8")
    # Ensure white-space: nowrap is preserved for SVG renderers lacking CSS4 text-wrap
    raw = raw.replace(b"text-wrap: nowrap;", b"white-space: nowrap; text-wrap: nowrap;")
    svg.write_bytes(raw)
    print(f"✓ route '#/{route}' → {svg.name} ({svg.stat().st_size} bytes)")


async def main() -> None:
    if not DIST_HTML.exists():
        raise FileNotFoundError(f"Missing {DIST_HTML}. Run 'npm run build:wiring-diagram' in web/ first.")
    async with async_playwright() as p:
        browser = await p.chromium.launch(headless=True, args=["--headless=new"])
        page = await browser.new_page(viewport={"width": 1300, "height": 1100})
        try:
            for route, svg in JOBS:
                await export_one(page, route, svg)
        finally:
            await browser.close()


if __name__ == "__main__":
    asyncio.run(main())
