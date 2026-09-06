#!/usr/bin/env -S uv run
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

DEVICE_IP = os.environ.get("DEVICE_IP", "192.168.24.63").strip().strip('"')
DURATION_S = int(os.environ.get("STRESS_DURATION_S", "300"))
HTTP_WORKERS = 4
HTTP_FAIL_LIMIT = 20
HTTP_ERROR_LOG_S = 2.0


@dataclass
class Stats:
    samples: int = 0
    global_max_dt: float = 0.0
    over_45: int = 0
    over_10: int = 0
    consecutive_http_fail: int = 0
    last_ok: float = 0.0
    last_http_error_log: float = 0.0
    sta_dead: bool = False
    lock: asyncio.Lock = field(default_factory=asyncio.Lock)

    async def observe(self, dt_max: float, source: str) -> None:
        async with self.lock:
            self.samples += 1
            self.consecutive_http_fail = 0
            self.last_ok = time.monotonic()
            if dt_max > self.global_max_dt:
                self.global_max_dt = dt_max
                print(f"[NEW PEAK] dt_max_ms={dt_max:.3f} via {source}", flush=True)
            if dt_max >= 4.5:
                self.over_45 += 1
            if dt_max >= 10.0:
                self.over_10 += 1

    async def note_http_error(self, err: Exception, stop: asyncio.Event) -> None:
        async with self.lock:
            self.consecutive_http_fail += 1
            now = time.monotonic()
            if now - self.last_http_error_log >= HTTP_ERROR_LOG_S:
                print(
                    f"[http] error ({self.consecutive_http_fail} consecutive): {err}",
                    flush=True,
                )
                self.last_http_error_log = now
            if self.consecutive_http_fail >= HTTP_FAIL_LIMIT and (
                self.last_ok == 0.0 or now - self.last_ok >= 8.0
            ):
                self.sta_dead = True
                stop.set()


stats = Stats()


def parse_dt(payload: dict) -> float | None:
    loop = payload.get("loop_stats")
    if isinstance(loop, dict) and "dt_max_ms" in loop:
        return float(loop["dt_max_ms"])
    if "dt_max_ms" in payload:
        return float(payload["dt_max_ms"])
    params = payload.get("params") or payload.get("state")
    if isinstance(params, dict):
        pl = params.get("loop_stats")
        if isinstance(pl, dict) and "dt_max_ms" in pl:
            return float(pl["dt_max_ms"])
        if "dt_max_ms" in params:
            return float(params["dt_max_ms"])
    return None


async def wait_http_and_pause() -> None:
    url = f"http://{DEVICE_IP}/state"
    deadline = time.monotonic() + 45.0
    last = None
    async with httpx.AsyncClient() as client:
        while time.monotonic() < deadline:
            try:
                r = await client.get(url, timeout=3.0)
                if r.status_code == 200:
                    await client.post(
                        f"http://{DEVICE_IP}/paused",
                        json={"paused": True, "position": 0.0},
                        timeout=5.0,
                    )
                    print("HTTP up; motor paused", flush=True)
                    stats.last_ok = time.monotonic()
                    return
                last = f"HTTP {r.status_code}"
            except Exception as e:
                last = e
            await asyncio.sleep(1.0)
    raise SystemExit(f"device HTTP not reachable at {url} ({last})")


async def http_worker(client: httpx.AsyncClient, stop: asyncio.Event) -> None:
    while not stop.is_set():
        try:
            r = await client.get(f"http://{DEVICE_IP}/state", timeout=5.0)
            if r.status_code == 200:
                data = r.json()
                dt = parse_dt(data)
                if dt is not None:
                    await stats.observe(dt, "http")
            else:
                await stats.note_http_error(
                    RuntimeError(f"HTTP {r.status_code}"), stop
                )
        except Exception as e:
            await stats.note_http_error(e, stop)
        await asyncio.sleep(0.01)


async def ws_listener(stop: asyncio.Event) -> None:
    uri = f"ws://{DEVICE_IP}/ws/command"
    while not stop.is_set():
        try:
            async with websockets.connect(
                uri, open_timeout=5.0, ping_interval=None, ping_timeout=None
            ) as ws:
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
            if stop.is_set():
                return
            print(f"[ws] reconnect after: {e}", flush=True)
            await asyncio.sleep(1.0)


async def reporter(stop: asyncio.Event) -> None:
    start = time.monotonic()
    while not stop.is_set():
        try:
            await asyncio.wait_for(stop.wait(), timeout=30.0)
            return
        except TimeoutError:
            elapsed = time.monotonic() - start
            async with stats.lock:
                print(
                    f"[{elapsed:5.0f}s] samples={stats.samples} "
                    f"peak_dt_max={stats.global_max_dt:.3f}ms "
                    f"over_4.5ms={stats.over_45} over_10ms={stats.over_10}",
                    flush=True,
                )


async def main() -> int:
    print(
        f"Stress test: {DURATION_S}s on {DEVICE_IP} "
        f"({HTTP_WORKERS} HTTP workers + WS 33ms, protocol pings off)",
        flush=True,
    )
    await wait_http_and_pause()
    stop = asyncio.Event()

    async with httpx.AsyncClient() as client:
        tasks = [
            asyncio.create_task(http_worker(client, stop))
            for _ in range(HTTP_WORKERS)
        ]
        tasks.append(asyncio.create_task(ws_listener(stop)))
        tasks.append(asyncio.create_task(reporter(stop)))

        try:
            await asyncio.wait_for(stop.wait(), timeout=DURATION_S)
        except TimeoutError:
            pass
        stop.set()
        await asyncio.gather(*tasks, return_exceptions=True)

    async with stats.lock:
        print("\n=== FINAL RESULTS ===")
        print(f"Duration:     {DURATION_S}s")
        print(f"Samples:      {stats.samples}")
        print(f"Peak dt_max:  {stats.global_max_dt:.3f} ms")
        print(f"Count >=4.5ms: {stats.over_45}")
        print(f"Count >=10ms:  {stats.over_10}")
        if stats.sta_dead:
            print("FAIL: HTTP circuit-breaker (STA/device unreachable)")
            return 1
        ok = stats.samples > 0 and stats.global_max_dt < 4.5
        print(f"PASS (<4.5ms): {ok}")
        return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
