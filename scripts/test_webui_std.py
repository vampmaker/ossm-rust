#!/usr/bin/env -S uv run
"""Playwright check: bundled ossm-std webui has motion controls and no WiFi/BLE settings."""

from __future__ import annotations

import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright

REPO = Path(__file__).resolve().parents[1]


def wait_tcp(host: str, port: int, timeout_s: float = 30.0) -> None:
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        try:
            with socket.create_connection((host, port), timeout=1.0):
                return
        except OSError:
            time.sleep(0.2)
    raise TimeoutError(f"TCP {host}:{port} not accepting")


def main() -> None:
    import argparse

    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--url",
        default=None,
        help="Existing ossm-std origin (skip spawning cargo). Example: http://192.168.24.222:8080",
    )
    args = parser.parse_args()
    if args.url:
        origin = args.url.rstrip("/")
        hostport = origin.split("://", 1)[-1]
        host, port_s = hostport.rsplit(":", 1) if ":" in hostport else (hostport, "80")
        wait_tcp(host, int(port_s), timeout_s=15.0)
        req = urllib.request.Request(
            f"{origin}/state",
            headers={"Origin": "http://example.com"},
        )
        with urllib.request.urlopen(req, timeout=5) as resp:
            acao = resp.headers.get("Access-Control-Allow-Origin")
            if acao != "*":
                raise AssertionError(f"GET /state ACAO={acao!r}")
        with sync_playwright() as p:
            browser = p.chromium.launch()
            page = browser.new_page()
            page.goto(f"{origin}/", wait_until="domcontentloaded", timeout=15_000)
            page.wait_for_selector("#speed-slider", timeout=10_000)
            assert page.locator("#ble-enabled").count() == 0
            assert page.locator("#wifi-ssid").count() == 0
            browser.close()
        print(f"✓ webui-std at {origin}: motion present, no WiFi/BLE; CORS * on /state")
        return

    port = 18090
    bind = f"127.0.0.1:{port}"
    with tempfile.TemporaryDirectory() as tmp:
        config = Path(tmp) / "ossm-config.json"
        proc = subprocess.Popen(
            [
                "cargo",
                "+stable",
                "run",
                "-p",
                "ossm-std",
                "--",
                "--mock",
                "--no-repl",
                "--bind",
                bind,
                "--config",
                str(config),
            ],
            cwd=REPO,
            start_new_session=True,
        )
        try:
            wait_tcp("127.0.0.1", port, timeout_s=180.0)
            req = urllib.request.Request(
                f"http://{bind}/state",
                headers={"Origin": "http://example.com"},
            )
            with urllib.request.urlopen(req, timeout=5) as resp:
                acao = resp.headers.get("Access-Control-Allow-Origin")
                if acao != "*":
                    raise AssertionError(f"GET /state ACAO={acao!r}")
            for gone in ("/pin-config", "/network-config", "/shell-config"):
                try:
                    urllib.request.urlopen(f"http://{bind}{gone}", timeout=5)
                    raise AssertionError(f"{gone} should 404")
                except urllib.error.HTTPError as e:
                    if e.code != 404:
                        raise AssertionError(f"{gone} expected 404 got {e.code}") from e
            with urllib.request.urlopen(f"http://{bind}/runtime-config", timeout=5) as resp:
                if resp.status != 200:
                    raise AssertionError(f"/runtime-config {resp.status}")
            with sync_playwright() as p:
                browser = p.chromium.launch()
                page = browser.new_page()
                page.goto(f"http://{bind}/", wait_until="domcontentloaded", timeout=15_000)
                page.wait_for_selector("#speed-slider", timeout=10_000)
                page.click('button[aria-label*="details"], button[title*="details"], button[title*="详情"]')
                page.wait_for_selector("#std-details-panel", timeout=5_000)
                assert page.locator("#ble-enabled").count() == 0
                assert page.locator("#wifi-ssid").count() == 0
                assert page.locator("#wifi-enabled").count() == 0
                browser.close()
            print("✓ webui-std bundled UI: motion present, no WiFi/BLE settings; CORS * on /state")
        finally:
            proc.terminate()
            try:
                proc.wait(timeout=8)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=5)


if __name__ == "__main__":
    try:
        main()
    except Exception as e:
        print(f"FAIL: {e}", file=sys.stderr)
        sys.exit(1)
