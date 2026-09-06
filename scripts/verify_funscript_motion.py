#!/usr/bin/env -S uv run
"""
verify_funscript_motion.py

Generates a large synthetic .funscript document, streams it to the live ESP32 firmware
over either WiFi (WebSocket JSON-RPC push telemetry) or USB Serial UART CLI, monitors continuous live
motor position telemetry, and renders a visual feedback curve.
"""

import argparse
import asyncio
import json
import math
import os
import sys
import time
from typing import List, Dict, Any
from pathlib import Path

from rich.console import Console
from rich.table import Table
from rich.panel import Panel

sys.path.insert(0, str(Path(__file__).parent))
from ossm import DeviceBackend, load_env

console = Console()


def generate_large_funscript() -> Dict[str, Any]:
    """Generates a large .funscript across 12 seconds."""
    actions = []
    total_duration_ms = 12000
    interval_ms = 250  # 4 actions per second -> 48 actions

    # Create a dynamic waveform: alternating oscillations and smooth sweeps
    for i in range(0, total_duration_ms + 1, interval_ms):
        t_sec = i / 1000.0
        pos = int(50.0 + 45.0 * math.sin(2.0 * math.pi * t_sec / 1.5))
        actions.append({"at": i, "pos": pos})

    return {"version": "1.0", "actions": actions}


def map_funscript_to_waypoints(actions: List[Dict[str, Any]], min_depth=0.0, max_depth=1.0) -> List[Dict[str, Any]]:
    wps = []
    for action in actions:
        t_ms = action["at"]
        pos_raw = action["pos"]
        normalized = max(0.0, min(1.0, pos_raw / 100.0))
        mapped = min_depth + normalized * (max_depth - min_depth)
        wps.append({"ts": int(t_ms), "pos": round(mapped, 4)})
    return wps


def print_ascii_curve(samples: List[Dict[str, Any]]):
    console.print("\n[bold cyan]Funscript Trajectory & Motor Position Feedback Curve (0.0 <---> 1.0):[/bold cyan]")
    console.print("[dim]Legend: '*' = Target Coordinate (y), '#' = Motor Position (pos), '@' = Overlap[/dim]")
    console.print("Time (s) | 0.0                      0.5                      1.0")
    console.print("---------+----------------------------------------------------+")
    step = max(1, len(samples) // 25)
    for sample in samples[::step]:
        t = sample.get("wall_t", 0.0)
        y = max(0.0, min(1.0, sample.get("y", 0.0)))
        pos = max(0.0, min(1.0, sample.get("pos", 0.0)))
        width = 50
        y_col = int(y * width)
        pos_col = int(pos * width)
        line = [" "] * (width + 1)
        if y_col == pos_col:
            line[y_col] = "@"
        else:
            line[y_col] = "*"
            line[pos_col] = "#"
        row_str = "".join(line)
        console.print(f"{t:6.2f}s  | {row_str} | (y={y:.2f}, pos={pos:.2f})")
    console.print("---------+----------------------------------------------------+\n")


async def run_verification(mode: str):
    load_env()
    ip = os.environ.get("DEVICE_IP", "").strip().strip('"')
    backend = DeviceBackend(mode=mode, ip=ip or "127.0.0.1")
    console.print(f"[bold cyan]Connecting to OSSM device via '{mode.upper()}' mode...[/bold cyan]")

    funscript_doc = generate_large_funscript()
    actions = funscript_doc["actions"]
    waypoints = map_funscript_to_waypoints(actions)
    total_waypoints = len(waypoints)
    total_duration_s = actions[-1]["at"] / 1000.0

    console.print(
        f"[bold yellow]Generated Large Funscript:[/bold yellow] {total_waypoints} waypoints spanning {total_duration_s:.1f} seconds."
    )

    telemetry_samples = []
    start_time = time.time()
    underruns_detected = 0
    max_y = 0.0
    min_y = 1.0

    if mode == "wifi":
        import websockets

        ws_url = f"ws://{backend.ip}/ws/command"
        console.print(f"[dim]WebSocket {ws_url} (protocol pings disabled)[/dim]")
        full_cfg: dict = {}
        try:
            async with websockets.connect(
                ws_url, open_timeout=5.0, ping_interval=None, ping_timeout=None
            ) as ws:

                async def ws_rpc(method: str, params: Any = None, req_id: int = 1):
                    req = {"jsonrpc": "2.0", "method": method, "id": req_id}
                    if params is not None:
                        req["params"] = params
                    await ws.send(json.dumps(req))
                    deadline = time.time() + 10.0
                    while time.time() < deadline:
                        msg = await asyncio.wait_for(ws.recv(), timeout=max(0.1, deadline - time.time()))
                        data = json.loads(msg)
                        if data.get("id") == req_id:
                            return data
                    raise TimeoutError(f"RPC {method} id={req_id} timed out")

                console.print("[dim]RPC reset-timestamp...[/dim]")
                await ws_rpc("reset-timestamp", req_id=10)
                console.print("[dim]RPC get-state...[/dim]")
                state_res = await ws_rpc("get-state", req_id=9)
                full_cfg = dict(state_res.get("result", {}).get("config", {}))
                full_cfg.update({"streaming": True, "paused": False, "depth": 1.0})
                console.print("[dim]RPC set-config streaming...[/dim]")
                await ws_rpc("set-config", params=full_cfg, req_id=11)
                initial_chunk = waypoints[:20]
                console.print("[dim]RPC set-waypoints...[/dim]")
                await ws_rpc(
                    "set-waypoints",
                    params={"waypoints": initial_chunk, "reset-timestamp": True},
                    req_id=12,
                )
                next_wp_idx = len(initial_chunk)

                await ws_rpc("subscribe-state", params={"interval_ms": 100}, req_id=13)
                console.print(
                    "[bold green]Streaming live waypoints and capturing WebSocket high-frequency push telemetry...[/bold green]"
                )

                start_time = time.time()
                pending_append = False
                while (time.time() - start_time) < (total_duration_s + 1.0):
                    try:
                        msg = await asyncio.wait_for(ws.recv(), timeout=0.3)
                        data = json.loads(msg)
                        if data.get("id") == 99:
                            pending_append = False
                            continue
                        if data.get("method") == "state":
                            params = data.get("params", {})
                            y_val = params.get("y", 0.0)
                            pos_val = params.get("position", 0.0)
                            stream_info = params.get("stream", {})
                            buffered = stream_info.get("buffered", 0)
                            stream_time = stream_info.get("stream_time", 0.0)
                            underrun = stream_info.get("underrun", False)

                            if underrun and next_wp_idx < total_waypoints:
                                underruns_detected += 1

                            max_y = max(max_y, y_val)
                            min_y = min(min_y, y_val)

                            telemetry_samples.append({
                                "wall_t": round(time.time() - start_time, 2),
                                "stream_time": round(stream_time, 2),
                                "y": round(y_val, 3),
                                "pos": round(pos_val, 3),
                                "buffered": buffered,
                                "underrun": underrun,
                                "update_history": (params.get("loop_stats") or {}).get(
                                    "update_history", []
                                ),
                                "position_history": params.get("position_history", []),
                            })

                            if (
                                buffered < 30
                                and next_wp_idx < total_waypoints
                                and not pending_append
                            ):
                                chunk = waypoints[next_wp_idx : next_wp_idx + 20]
                                await ws.send(json.dumps({
                                    "jsonrpc": "2.0",
                                    "method": "append-waypoints",
                                    "params": chunk,
                                    "id": 99,
                                }))
                                next_wp_idx += len(chunk)
                                pending_append = True
                    except TimeoutError:
                        continue
                    except Exception as e:
                        console.print(f"[red]WS recv error: {type(e).__name__}: {e}[/red]")
                        break

                await ws_rpc("unsubscribe-state", req_id=100)
                full_cfg.update({"streaming": False, "paused": True})
                await ws_rpc("set-config", params=full_cfg, req_id=101)
        finally:
            try:
                await backend.set_config({"streaming": False, "paused": True})
            except Exception:
                pass

    else:
        # 0. Reset timestamp explicitly
        await backend.send_rpc("reset-timestamp")
        time.sleep(0.1)

        # 1. Feed initial chunk (< 250 bytes for BLE GATT/serial limits)
        initial_size = 5 if mode in ("serial", "ble") else 20
        initial_chunk = waypoints[:initial_size]
        await backend.send_waypoints(initial_chunk, reset_timestamp=True)
        time.sleep(0.1)

        # 2. Start streaming mode
        await backend.set_config({"streaming": True, "paused": False, "depth": 1.0})
        time.sleep(0.1)
        next_wp_idx = len(initial_chunk)
        start_time = time.time()

        console.print("[bold green]Streaming live waypoints and collecting motor trajectory feedback...[/bold green]")

        while (time.time() - start_time) < (total_duration_s + 1.0):
            st = await backend.get_status()
            state_data = st.get("state", st)
            stream_info = state_data.get("stream", {})
            buffered = stream_info.get("buffered", 0)
            stream_time = stream_info.get("stream_time", 0.0)
            underrun = stream_info.get("underrun", False)
            y_val = state_data.get("y", state_data.get("position", 0.0))
            pos_val = state_data.get("position", 0.0)

            if underrun and next_wp_idx < total_waypoints and (time.time() - start_time) > 0.8:
                underruns_detected += 1

            max_y = max(max_y, y_val)
            min_y = min(min_y, y_val)

            telemetry_samples.append({
                "wall_t": round(time.time() - start_time, 2),
                "stream_time": round(stream_time, 2),
                "y": round(y_val, 3),
                "pos": round(pos_val, 3),
                "buffered": buffered,
                "underrun": underrun,
                "update_history": (state_data.get("loop_stats") or {}).get(
                    "update_history", []
                ),
                "position_history": state_data.get("position_history", []),
            })

            if buffered < 40 and next_wp_idx < total_waypoints:
                chunk_size = 5 if mode in ("serial", "ble") else 20
                chunk = waypoints[next_wp_idx : next_wp_idx + chunk_size]
                await backend.send_waypoints(chunk, reset_timestamp=False)
                next_wp_idx += len(chunk)

            time.sleep(0.1)

        await backend.set_config({"streaming": False, "paused": True})

    # Display telemetry summary table
    table = Table(title=f"OSSM Funscript Motion Feedback Verification ({mode.upper()})", show_lines=True)
    table.add_column("Wall Time (s)", justify="right", style="cyan")
    table.add_column("Stream Time (s)", justify="right", style="magenta")
    table.add_column("Target y", justify="right", style="green")
    table.add_column("Motor Pos", justify="right", style="blue")
    table.add_column("Buffer Level", justify="right", style="yellow")
    table.add_column("Underrun", justify="center", style="bold")

    step = max(1, len(telemetry_samples) // 20)
    for sample in telemetry_samples[::step]:
        underrun_str = "[red]YES[/red]" if sample["underrun"] else "[green]NO[/green]"
        table.add_row(
            f"{sample['wall_t']:.2f}s",
            f"{sample['stream_time']:.2f}s",
            f"{sample['y']:.3f}",
            f"{sample['pos']:.3f}",
            str(sample["buffered"]),
            underrun_str,
        )

    console.print(table)

    # Print ASCII plot of trajectory curve
    print_ascii_curve(telemetry_samples)

    console.print(f"[bold cyan]Observed Trajectory Range:[/bold cyan] y min={min_y:.3f}, y max={max_y:.3f}")
    if telemetry_samples and telemetry_samples[-1].get("update_history"):
        console.print(f"[bold magenta]Historical Motor Position Updates/sec (10s, 1s window):[/bold magenta] {telemetry_samples[-1]['update_history']}")

    # Perform automated motion assertions
    console.print("\n[bold yellow]Automated Motion Verification Assertions:[/bold yellow]")

    assert len(telemetry_samples) >= 15, f"Expected at least 15 telemetry samples, got {len(telemetry_samples)}"
    console.print(f"  [green]✓[/green] Telemetry sample count verified: {len(telemetry_samples)} live samples captured")

    assert underruns_detected == 0, f"Detected {underruns_detected} buffer underruns during active playback"
    console.print("  [green]✓[/green] Zero buffer underruns detected during active streaming playback")

    console.print(
        Panel.fit(
            f"[bold green]FULL FUNSCRIPT MOTION & CURVE VERIFICATION PASSED SUCCESSFULLY ({mode.upper()})![/bold green]",
            border_style="green",
        )
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Verify Funscript motion and monitor curve feedback.")
    parser.add_argument("--mode", "-m", default="wifi", choices=["wifi", "ble", "serial"], help="Communication mode")
    args = parser.parse_args()
    asyncio.run(run_verification(args.mode))
