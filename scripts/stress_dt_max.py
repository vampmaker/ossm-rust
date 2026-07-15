#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#     "httpx>=0.27.0",
#     "websockets>=12.0",
#     "python-dotenv>=1.0.0",
# ]
# ///
"""Long-running HTTP+WS stress test; tracks peak dt_max_ms from /state and WS pushes."""

import asyncio
import json
import os
import sys
import time
from dataclasses import dataclass, field

import httpx
import websockets
from dotenv import load_dotenv

load_dotenv(os.path.join(os.path.dirname(__file__), "..", ".env"))

DEVICE_IP = os.environ.get("DEVICE_IP", "192.168.24.63")
DURATION_S = int(os.environ.get("STRESS_DURATION_S", "300"))
HTTP_WORKERS = 4


@dataclass
class Stats:
    samples: int = 0
    global_max_dt: float = 0.0
    over_45: int = 0
    over_10: int = 0
    lock: asyncio.Lock = field(default_factory=asyncio.Lock)

    async def observe(self, dt_max: float, source: str) -> None:
        async with self.lock:
            self.samples += 1
            if dt_max > self.global_max_dt:
                self.global_max_dt = dt_max
                print(f"[NEW PEAK] dt_max_ms={dt_max:.3f} via {source}", flush=True)
            if dt_max >= 4.5:
                self.over_45 += 1
            if dt_max >= 10.0:
                self.over_10 += 1


stats = Stats()


def parse_dt(payload: dict) -> float | None:
    if "dt_max_ms" in payload:
        return float(payload["dt_max_ms"])
    params = payload.get("params") or payload.get("state")
    if isinstance(params, dict) and "dt_max_ms" in params:
        return float(params["dt_max_ms"])
    return None


async def http_worker(client: httpx.AsyncClient, stop: asyncio.Event) -> None:
    while not stop.is_set():
        try:
            r = await client.get(f"http://{DEVICE_IP}/state", timeout=5.0)
            if r.status_code == 200:
                data = r.json()
                dt = parse_dt(data)
                if dt is not None:
                    await stats.observe(dt, "http")
        except Exception as e:
            print(f"[http] error: {e}", flush=True)
        await asyncio.sleep(0.01)


async def ws_listener(stop: asyncio.Event) -> None:
    uri = f"ws://{DEVICE_IP}/ws/command"
    while not stop.is_set():
        try:
            async with websockets.connect(uri, ping_interval=20, ping_timeout=20) as ws:
                await ws.send(
                    json.dumps(
                        {
                            "jsonrpc": "2.0",
                            "method": "subscribe-state",
                            "params": {"interval_ms": 33},
                            "id": 1,
                        }
                    )
                )
                await ws.recv()
                while not stop.is_set():
                    msg = await asyncio.wait_for(ws.recv(), timeout=5.0)
                    data = json.loads(msg)
                    dt = parse_dt(data)
                    if dt is not None:
                        await stats.observe(dt, "ws")
        except Exception as e:
            print(f"[ws] reconnect after: {e}", flush=True)
            await asyncio.sleep(1.0)


async def reporter(stop: asyncio.Event) -> None:
    start = time.monotonic()
    while not stop.is_set():
        await asyncio.sleep(30.0)
        elapsed = time.monotonic() - start
        async with stats.lock:
            print(
                f"[{elapsed:5.0f}s] samples={stats.samples} "
                f"peak_dt_max={stats.global_max_dt:.3f}ms "
                f"over_4.5ms={stats.over_45} over_10ms={stats.over_10}",
                flush=True,
            )


async def main() -> int:
    print(f"Stress test: {DURATION_S}s on {DEVICE_IP} ({HTTP_WORKERS} HTTP workers + WS 33ms)")
    stop = asyncio.Event()

    async with httpx.AsyncClient() as client:
        tasks = [
            asyncio.create_task(http_worker(client, stop))
            for _ in range(HTTP_WORKERS)
        ]
        tasks.append(asyncio.create_task(ws_listener(stop)))
        tasks.append(asyncio.create_task(reporter(stop)))

        await asyncio.sleep(DURATION_S)
        stop.set()
        await asyncio.gather(*tasks, return_exceptions=True)

    async with stats.lock:
        print("\n=== FINAL RESULTS ===")
        print(f"Duration:     {DURATION_S}s")
        print(f"Samples:      {stats.samples}")
        print(f"Peak dt_max:  {stats.global_max_dt:.3f} ms")
        print(f"Count >=4.5ms: {stats.over_45}")
        print(f"Count >=10ms:  {stats.over_10}")
        ok = stats.global_max_dt < 4.5
        print(f"PASS (<4.5ms): {ok}")
        return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
