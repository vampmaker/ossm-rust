#!/usr/bin/env -S uv run
"""Flash ESP32-C6 over Termux USB-host and exercise ossm-std on the phone."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from setup_device import update_env_ip
from ossm import load_env

from dotenv import load_dotenv

REPO = Path(__file__).resolve().parents[1]


def ssh(
    host: str,
    remote: str,
    check: bool = True,
    timeout: int | None = None,
    stdin: str | None = None,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["ssh", host, remote],
        check=check,
        text=True,
        capture_output=True,
        timeout=timeout,
        input=stdin,
    )


def scp(host: str, local: Path, remote: str) -> None:
    subprocess.run(["scp", str(local), f"{host}:{remote}"], check=True)


def ssh_hostname(host: str) -> str:
    out = subprocess.check_output(["ssh", "-G", host], text=True)
    for line in out.splitlines():
        if line.startswith("hostname "):
            return line.split(None, 1)[1].strip()
    return host


def list_usb(host: str) -> str:
    r = ssh(host, "termux-usb -l", check=True)
    m = re.search(r'"(/dev/bus/usb/[^"]+)"', r.stdout)
    if not m:
        raise SystemExit(f"no USB device from termux-usb -l:\n{r.stdout}{r.stderr}")
    return m.group(1)


def termux_usb_uri(usb: str) -> str:
    if usb.startswith("termux-usb:"):
        return usb
    return f"termux-usb:{usb}"


def remote_std(host: str, extra: list[str], timeout: int = 180) -> subprocess.CompletedProcess[str]:
    argv = ["./ossm-std-linux-arm64", *extra]
    shown = [
        "set net.password <redacted>" if a.startswith("set net.password") else a
        for a in argv
    ]
    print("$ ssh", host, " ".join(sh_quote(a) for a in shown))
    ssh(host, "cat > ~/ossm-argv.json", stdin=json.dumps(argv), check=True, timeout=15)
    runner = (
        "import json,os\n"
        "argv=json.load(open(os.path.expanduser('~/ossm-argv.json')))\n"
        "os.execv(argv[0], argv)\n"
    )
    ssh(host, "cat > ~/ossm_exec.py", stdin=runner, check=True, timeout=15)
    return ssh(host, "python3 ~/ossm_exec.py", check=False, timeout=timeout)


def sh_quote(s: str) -> str:
    if re.fullmatch(r"[-_./:@+=,%A-Za-z0-9]+", s):
        return s
    return "'" + s.replace("'", "'\\''") + "'"


def http_json(url: str, method: str = "GET", body: dict | None = None, timeout: float = 5.0):
    data = None
    headers = {}
    if body is not None:
        data = json.dumps(body).encode()
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read()
            if not raw:
                return None
            return json.loads(raw.decode())
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", "replace")
        raise RuntimeError(f"HTTP {e.code} {url}: {body}") from e


def wait_http_state(url: str, timeout_s: float, pred) -> dict:
    deadline = time.monotonic() + timeout_s
    last = None
    last_err = None
    while time.monotonic() < deadline:
        try:
            last = http_json(url, timeout=3.0)
            if last is not None and pred(last):
                return last
        except Exception as e:
            last_err = e
        time.sleep(0.5)
    raise AssertionError(f"timeout waiting on {url}: last={last} err={last_err}")


def zigbuild_std() -> Path:
    subprocess.run(
        [
            "cargo",
            "+stable",
            "zigbuild",
            "-p",
            "ossm-std",
            "--release",
            "--target",
            "aarch64-unknown-linux-musl",
        ],
        cwd=REPO,
        check=True,
    )
    src = REPO / "target/aarch64-unknown-linux-musl/release/ossm-std"
    dest = REPO / "release/ossm-std-linux-arm64"
    dest.parent.mkdir(exist_ok=True)
    dest.write_bytes(src.read_bytes())
    dest.chmod(0o755)
    return dest


def parse_pins(s: str) -> tuple[int, int, int]:
    parts = [p.strip() for p in s.split(",")]
    if len(parts) != 3:
        raise SystemExit("--pins must be tx,rx,de")
    return int(parts[0]), int(parts[1]), int(parts[2])


def console_sends(host: str, usb: str, sends: list[str], until: str | None = None, timeout: int = 30) -> str:
    extra = ["--mode", "console", "--serial", termux_usb_uri(usb), "--timeout", str(timeout)]
    for s in sends:
        extra += ["--send", s]
    if until:
        extra += ["--until", until]
    r = remote_std(host, extra, timeout=timeout + 60)
    text = (r.stdout or "") + (r.stderr or "")
    print(text)
    if "zero-copy" in text:
        raise SystemExit("nusb zero-copy mmap warning still present")
    if r.returncode != 0:
        raise SystemExit(f"console failed ({r.returncode})")
    return text


def try_exit_rs485(host: str, usb: str) -> None:
    extra = ["--mode", "console", "--serial", termux_usb_uri(usb), "--exit-rs485", "--timeout", "8"]
    r = remote_std(host, extra, timeout=25)
    print((r.stdout or "") + (r.stderr or ""))
    if r.returncode != 0:
        print("warning: --exit-rs485 did not ACK (already servo, or USB not granted)")


def ensure_usb_cli(host: str, usb: str) -> str:
    """Return `get pin` output. Magic-exit only if the CLI does not answer."""
    try:
        text = console_sends(host, usb, ["get pin"], timeout=10)
        if "operating_mode" in text:
            return text
    except SystemExit as e:
        print(f"warning: get pin failed ({e}); trying magic-exit")
    try_exit_rs485(host, usb)
    return console_sends(host, usb, ["get pin"], timeout=12)


def main() -> None:
    load_env()
    load_dotenv(REPO / ".env", override=True)
    parser = argparse.ArgumentParser(description="Termux ESP32-C6 flash + ossm-std live test")
    parser.add_argument("--host", default="termux")
    parser.add_argument("--usb", default=None, help="usbfs path; default: first termux-usb -l")
    parser.add_argument("--pins", default="2,1,0", help="modbus tx,rx,de-re")
    parser.add_argument("--bind", default="0.0.0.0:8080")
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--skip-flash", action="store_true")
    parser.add_argument("--skip-provision", action="store_true")
    parser.add_argument("--skip-servo", action="store_true")
    parser.add_argument("--skip-webui", action="store_true")
    parser.add_argument(
        "--mock",
        action="store_true",
        help="Phone ossm-std --mock (no RS-485 / homing)",
    )
    args = parser.parse_args()
    tx, rx, de = parse_pins(args.pins)
    host = args.host
    try:
        probe = subprocess.run(
            [
                "ssh",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=5",
                host,
                "true",
            ],
            capture_output=True,
            text=True,
            timeout=12,
            check=False,
        )
    except subprocess.TimeoutExpired:
        print(f"SKIP: ssh {host} unreachable (timeout)")
        return
    if probe.returncode != 0:
        err = (probe.stderr or probe.stdout or "").strip() or f"rc={probe.returncode}"
        print(f"SKIP: ssh {host} unreachable ({err})")
        return
    phone_ip = ssh_hostname(host)
    ssid = os.environ.get("WIFI_SSID", "").strip().strip('"')
    password = os.environ.get("WIFI_PASSWORD", "").strip().strip('"')
    image = REPO / "release/ossm-esp32c6.bin"
    if not image.is_file() and not args.skip_flash:
        raise SystemExit(f"missing {image}; build firmware first")

    if not args.skip_build:
        print("--- zigbuild ossm-std aarch64-linux-musl ---")
        zigbuild_std()
        scp(host, REPO / "release/ossm-std-linux-arm64", "~/ossm-std-linux-arm64")
        ssh(host, "chmod +x ~/ossm-std-linux-arm64")
    if not args.skip_flash:
        scp(host, image, "~/ossm-esp32c6.bin")

    usb = args.usb or list_usb(host)
    print(f"usb={usb} phone={phone_ip} pins={tx}/{rx}/{de}")
    print("If Termux shows a USB permission dialog, tap Allow.")
    ssh(host, "killall ossm-std-linux-arm64 2>/dev/null || true", check=False)
    time.sleep(0.4)
    print("--- USB CLI ---")
    pin_boot = ensure_usb_cli(host, usb)
    print(pin_boot)

    if not args.skip_flash:
        print("--- flash ---")
        r = remote_std(
            host,
            ["--mode", "flash", "--serial", termux_usb_uri(usb), "--image", "ossm-esp32c6.bin", "--usb-jtag"],
            timeout=180,
        )
        print((r.stdout or "") + (r.stderr or ""))
        if r.returncode != 0:
            raise SystemExit("flash failed")
        time.sleep(2)

    if args.skip_provision:
        text = ""
        device_ip = os.environ.get("DEVICE_IP", "").strip().strip('"')
    else:
        if not ssid:
            raise SystemExit("WIFI_SSID missing in .env")
        print("--- console: dump + provision ---")
        chunks = [
            ["help"],
            ["get pin"],
            ["get net"],
            [f"set net.ssid {ssid}"],
            [f"set net.password {password}"],
            [f"set pin.modbus_tx {tx}"],
            [f"set pin.modbus_rx {rx}"],
            [f"set pin.modbus_de_re {de}"],
            ["set pin.modbus_timeout_ms 0"],
            ["set pin.modbus_scan_delay_us 0"],
            ["get pin"],
        ]
        text = ""
        for chunk in chunks:
            try:
                text += console_sends(host, usb, chunk, timeout=12)
            except SystemExit as e:
                print(f"warning: console {chunk!r} failed ({e})")
        try:
            text += console_sends(host, usb, ["reset"], timeout=8)
        except SystemExit as e:
            print(f"warning: reset send failed ({e}); USB may drop across chip reset")
        time.sleep(4.0)
        try:
            text += console_sends(host, usb, [], until=r"ip:", timeout=40)
        except SystemExit as e:
            print(f"warning: provision until ip: failed ({e}); continuing")
        device_ip = os.environ.get("DEVICE_IP", "").strip().strip('"')
    if not args.skip_provision:
        compact = text.replace(" ", "")
        if f'"modbus_tx":{tx}' not in compact and f"modbus_tx set to {tx}" not in text:
            print("warning: could not confirm pin.modbus_tx in console output")
        ip_m = re.search(r"DEVICE_IP=([0-9.]+)", text)
        if not ip_m:
            ip_m = re.search(r"(?i)(?:sta ip:|got ip:|ip:)\s*([0-9.]+)", text)
        if ip_m:
            update_env_ip(ip_m.group(1))
            device_ip = ip_m.group(1)
        else:
            print("warning: no IP parsed from console; using .env DEVICE_IP")
        if device_ip:
            print(f"--- firmware /state on {device_ip} ---")
            try:
                st = wait_http_state(
                    f"http://{device_ip}/state",
                    20.0,
                    lambda v: isinstance((v.get("loop_stats") or {}).get("ups"), (int, float))
                    and (v["loop_stats"]["ups"] > 300),
                )
                print(f"✓ firmware ups={st['loop_stats']['ups']}")
            except Exception as e:
                print(f"warning: firmware HTTP not reachable ({e})")

    if args.skip_servo:
        print("skip servo test")
        return

    ssh(host, "killall ossm-std-linux-arm64 2>/dev/null || true", check=False)

    if args.mock:
        print("--- phone ossm-std --mock ---")
        ssh(
            host,
            f"nohup ./ossm-std-linux-arm64 --mock --no-repl --bind {sh_quote(args.bind)} "
            f"--config ~/ossm-config.json > ossm-std.log 2>&1 &",
            check=True,
        )
    else:
        compact = pin_boot.replace(" ", "")
        if '"operating_mode":"rs485"' in compact:
            print("firmware already in rs485")
        else:
            print("--- set pin.operating_mode rs485 ---")
            ack = console_sends(host, usb, ["set pin.operating_mode rs485"], timeout=15)
            if "operating_mode set to rs485" not in ack:
                ack = console_sends(host, usb, ["set pin.operating_mode rs485"], timeout=15)
            if "operating_mode set to rs485" not in ack:
                raise SystemExit("failed to switch firmware to rs485 (no CLI ACK)")
        ssh(host, "rm -f ~/ossm-config.json", check=False)
        print("--- phone ossm-std servo over USB ---")
        ssh(
            host,
            f"nohup ./ossm-std-linux-arm64 --serial {sh_quote(termux_usb_uri(usb))} --no-repl "
            f"--bind {sh_quote(args.bind)} --config ~/ossm-config.json > ossm-std.log 2>&1 &",
            check=True,
        )

    port = int(args.bind.rsplit(":", 1)[-1])
    base = f"http://{phone_ip}:{port}"
    try:
        st = wait_http_state(
            f"{base}/state",
            90.0 if not args.mock else 20.0,
            lambda v: args.mock
            or (
                isinstance(v.get("pos_min"), (int, float))
                and isinstance(v.get("pos_max"), (int, float))
                and v["pos_max"] > v["pos_min"]
            ),
        )
        print(f"✓ phone /state pos_min={st.get('pos_min')} pos_max={st.get('pos_max')} ups={(st.get('loop_stats') or {}).get('ups')}")
        log = ssh(host, "cat ~/ossm-std.log 2>/dev/null || true", check=False)
        servo_log = (log.stdout or "") + (log.stderr or "")
        if "zero-copy" in servo_log:
            print(servo_log)
            raise SystemExit("nusb zero-copy mmap warning still present in ossm-std.log")
        print("✓ no nusb zero-copy warning in phone ossm-std.log")
        if not args.mock:
            paused = http_json(f"{base}/paused", "POST", {"paused": False, "position": 0.0})
            print(f"resume {paused}")
            cfg = paused if isinstance(paused, dict) else {}
            if "version" not in cfg:
                cfg = (http_json(f"{base}/state") or {}).get("config") or {}
            cfg = dict(cfg)
            cfg["bpm"] = 20.0
            cfg["depth"] = 0.3
            http_json(f"{base}/config", "POST", cfg)
            st2 = wait_http_state(
                f"{base}/state",
                20.0,
                lambda v: isinstance((v.get("loop_stats") or {}).get("ups"), (int, float))
                and (v["loop_stats"]["ups"] >= 190)
                and isinstance((v.get("loop_stats") or {}).get("dt_max_ms"), (int, float))
                and v["loop_stats"]["dt_max_ms"] < 6.5,
            )
            ls = st2.get("loop_stats") or {}
            link = st2.get("link_stats") or {}
            print(
                f"moving position={st2.get('position')} "
                f"ups={ls.get('ups')} dt_avg={ls.get('dt_avg_ms')} dt_max={ls.get('dt_max_ms')} "
                f"link_eps={link.get('exchanges_per_sec')} rtt_mean={(link.get('round_trip') or {}).get('mean')}"
            )
            if (link.get("exchanges") or 0) < 1:
                raise SystemExit("phone servo: no RTU exchanges")
            print(f"✓ phone motion loop_stats ups={ls.get('ups')} dt_max_ms={ls.get('dt_max_ms')}")
            http_json(f"{base}/paused", "POST", {"paused": True, "position": 0.0})
        if not args.skip_webui:
            subprocess.run(
                [sys.executable, str(REPO / "scripts/test_webui_std.py"), "--url", base],
                check=True,
            )
    finally:
        ssh(host, "killall ossm-std-linux-arm64 2>/dev/null || true", check=False)
        time.sleep(0.5)

    if not args.mock:
        print("--- magic-exit rs485 ---")
        extra = ["--mode", "console", "--serial", termux_usb_uri(usb), "--exit-rs485", "--timeout", "20"]
        r = remote_std(host, extra, timeout=40)
        print((r.stdout or "") + (r.stderr or ""))
        if r.returncode != 0:
            raise SystemExit("magic-exit failed")
        pin_text = console_sends(host, usb, ["get pin"], timeout=15)
        if '"operating_mode":"servo"' not in pin_text.replace(" ", ""):
            raise SystemExit("firmware did not return operating_mode servo after magic-exit")
        print("✓ firmware USB CLI recovered (operating_mode=servo)")
        if device_ip:
            try:
                wait_http_state(
                    f"http://{device_ip}/state",
                    12.0,
                    lambda v: isinstance((v.get("loop_stats") or {}).get("ups"), (int, float))
                    and v["loop_stats"]["ups"] > 300,
                )
                print("✓ firmware WiFi /state ups recovered")
            except Exception as e:
                print(f"warning: firmware WiFi /state not reachable ({e})")

    print("✓ Termux tests PASSED")


if __name__ == "__main__":
    try:
        main()
    except subprocess.TimeoutExpired as e:
        print(f"FAIL: timeout {e}", file=sys.stderr)
        sys.exit(1)
    except Exception as e:
        print(f"FAIL: {e}", file=sys.stderr)
        sys.exit(1)
