#!/usr/bin/env -S uv run
"""Export assets/wiring_diagram.html (+ _zh) to SVG via Playwright + html-to-image."""

from __future__ import annotations

import asyncio
from pathlib import Path
from urllib.parse import unquote

from playwright.async_api import async_playwright

REPO = Path(__file__).resolve().parents[1]
ASSETS = REPO / "assets"
JOBS = [
    (ASSETS / "wiring_diagram.html", ASSETS / "wiring_diagram.svg"),
    (ASSETS / "wiring_diagram_zh.html", ASSETS / "wiring_diagram_zh.svg"),
    (ASSETS / "wiring_diagram_std.html", ASSETS / "wiring_diagram_std.svg"),
    (ASSETS / "wiring_diagram_std_zh.html", ASSETS / "wiring_diagram_std_zh.svg"),
]


async def export_one(page, html: Path, svg: Path) -> None:
    url = html.resolve().as_uri()
    await page.goto(url, wait_until="networkidle")
    await page.wait_for_function("() => window.htmlToImage || window.htmlToImage === undefined || true")
    await page.wait_for_timeout(400)
    data_url = await page.evaluate(
        """async () => {
          const el = document.getElementById('diagram');
          if (!el) throw new Error('missing #diagram');
          if (typeof htmlToImage === 'undefined' || !htmlToImage.toSvg) {
            throw new Error('html-to-image did not load');
          }
          return await htmlToImage.toSvg(el, { pixelRatio: 1 });
        }"""
    )
    if not isinstance(data_url, str) or not data_url.startswith("data:image/svg+xml"):
        raise RuntimeError(f"unexpected toSvg result from {html.name}")
    _, payload = data_url.split(",", 1)
    if ";base64," in data_url[:80]:
        import base64

        raw = base64.b64decode(payload)
    else:
        raw = unquote(payload).encode("utf-8")
    svg.write_bytes(raw)
    print(f"✓ {html.name} → {svg.name} ({svg.stat().st_size} bytes)")


async def main() -> None:
    async with async_playwright() as p:
        browser = await p.chromium.launch(headless=True, args=["--headless=new"])
        page = await browser.new_page(viewport={"width": 1300, "height": 1100})
        try:
            for html, svg in JOBS:
                await export_one(page, html, svg)
        finally:
            await browser.close()


if __name__ == "__main__":
    asyncio.run(main())
