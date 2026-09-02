# OSSM-Rust

The same 57AIM30 motion Engine runs on **two independent paths**. Pick **one** — they do not share a controller:

| | **Path A — `ossm-std`** | **Path B — `ossm-esp32`** |
| --- | --- | --- |
| **What it is** | Linux desktop process. The PC *is* the device. | Firmware on ESP32-C6 / ESP32-S3. WiFi / BLE / USB CLI. |
| **Hardware** | PC + USB-RS485 dongle (automatic DE/RE). **No microcontroller.** | ESP32 DevKit + TTL MAX3485 (DI / RO / DE+RE). |
| **Wiring** | Dongle A/B/GND to the motor. 24 V shaft power + 5 V logic. | ESP32 GPIOs to MAX3485; ESP32 5 V / GND to motor logic. |
| **Config** | `--serial` / `--bind` / `--config` (or `.env`). HTTP on `127.0.0.1:8080`. | NVS `pin.*` / `net.*` via flasher, web UI, or USB CLI. |
| **UI** | `http://127.0.0.1:8080` (set `DEVICE_IP=127.0.0.1:8080` for scripts) | Device WiFi IP or `http://ossm.local` |

Both paths use the same motor connectors and 24 V supply.

> [!TIP]
> **Motor plugs (both paths)**
> • **Front 6-pin (power):** green **KF2EDGK-3.81 6P** — 24 V.
> • **Back 10-pin (comm / logic):** white **PHB2.0 2×5P** — 5 V + RS-485. Never put 24 V on the back connector.

---

## Path A: Desktop (`ossm-std`) — no microcontroller

### A.1 Hardware

| Component | Notes |
| --- | --- |
| **Linux PC** | Runs `ossm-std` (stable Rust). |
| **USB-RS485 adapter** | **Automatic DE/RE** on the dongle. TX-loopback and host-driven DE/RE are unsupported (ossm-std does not strip TX echoes). Typical device `/dev/ttyUSB0`. |
| **Motor** | `57AIM30` (not `57AIM30H`). |
| **24 V DC supply** | Shaft power to the front terminal. |
| **5 V USB charger** | Motor logic 5 V / COM on the back terminal (or a 5 V pin on the adapter if it can supply the load). |
| **Cables** | DC 5.5×2.1/2.5 mm jack, **KF2EDGK-3.81 6P**, **PHB2.0 2×5P**. |

### A.2 Wiring

<div align="center">
  <img src="./assets/wiring_diagram_std.svg" alt="ossm-std USB-RS485 wiring (no ESP32)" width="100%"/>
</div>

| Motor | --> | Host side |
| --- | --- | --- |
| Front **+V** / **GND** | --> | 24 V adapter `+` / `−` |
| Back **10: 5V** / **6: COM** | --> | 5 V charger 5 V / GND |
| Back **2: 485A** / **3: 485B** | --> | USB-RS485 **A** / **B** |
| Back **COM** | --> | USB-RS485 **GND** (signal reference) |

> [!CAUTION]
> Check 24 V polarity before plugging in. Do not feed 24 V into the back 10-pin connector.

### A.3 Configuration

CLI flags override environment (`.env` aliases `DEVICE_PORT`, `DEVICE_BAUD`); then defaults. Motor JSON is `--config` (default `ossm-config.json`). There are no `pin.*` / `net.*` settings.

```bash
# UI only, no motor:
cargo +stable run -p ossm-std -- --mock --bind 127.0.0.1:8080

# USB-RS485 (always homes):
cargo +stable run -p ossm-std -- --serial /dev/ttyUSB0 --baud 115200 \
  --bind 127.0.0.1:8080 --config ossm-config.json
```

Open `http://127.0.0.1:8080`. Scripts: `DEVICE_IP=127.0.0.1:8080`.

`--mode servo` (default) is the RTU master. `--mode rtu-relay` serves Modbus TCP `--modbus-bind` (default `127.0.0.1:502`) and `/ws/modbus` instead of owning motion. `--mock` uses a virtual `0..100` range.

---

## Path B: ESP32 firmware (`ossm-esp32`)

### B.1 Hardware

| Component | Description | Notes |
| --- | --- | --- |
| **Microcontroller** | ESP32-C6 or ESP32-S3 DevKit with USB-C | **Two USB-C ports** on many boards:<br>• **USB / Native / JTAG** — USB Serial/JTAG. Flash and the firmware CLI (`/dev/ttyACM*` on Linux).<br>• **UART / COM** — USB-UART to UART0 (C6: GPIO16 TX / GPIO17 RX). Console logs only. Not the motor bus; not the browser flasher. |
| **Motor** | 57AIM30 (not `57AIM30H`) | 1500 RPM, 0.96 N·m. |
| **RS-485 transceiver** | MAX3485 TTL module | Between ESP32 UART and the motor. |
| **24 V DC supply** | Motor shaft power | Size for your mechanical load. |
| **5 V USB charger** | Powers the ESP32 after setup | USB-C. Motor logic 5 V comes from the ESP32 5 V pin. |
| **Cables** | DC jack, **KF2EDGK-3.81 6P**, **PHB2.0 2×5P**, DuPont jumpers | Same motor plugs as Path A. |

### B.2 Wiring

<div align="center">
  <img src="./assets/wiring_diagram.svg" alt="ossm-esp32 ESP32 + MAX3485 wiring" width="100%"/>
</div>

**Motor 24 V (front 6-pin)**

| Motor Terminal (+V) | --> | 24V Power Supply (Positive `+`) |
| --- | --- | --- |
| Motor Terminal (GND) | --> | 24V Power Supply (Negative `-`) |

**RS-485 + logic (back 10-pin and ESP32)**

| Motor (485A) | --> | MAX3485 **A** |
| --- | --- | --- |
| Motor (485B) | --> | MAX3485 **B** |

| ESP32 Pin | --> | Component Pin | Purpose |
| --- | --- | --- | --- |
| 5V (or VBUS/VIN) | --> | Motor Terminal (5V) | Motor logic |
| GND | --> | Motor Terminal (COM / GND) | Logic ground |
| 3.3V | --> | MAX3485 VCC | Transceiver power |
| GND | --> | MAX3485 GND | Transceiver ground |
| GPIO 18 (UART **TX**) | --> | MAX3485 **DI** | MCU transmits |
| GPIO 19 (UART **RX**) | --> | MAX3485 **RO** | MCU receives |
| GPIO 20 | --> | MAX3485 **DE** and **RE** (tie together) | Direction |

> [!WARNING]
> UART is **crossed**: ESP32 **TX → DI**, ESP32 **RX → RO**. Modules labeled TXD/RXD often have **TXD = RO** and **RXD = DI** — do not wire MCU TX to TXD.
>
> NVS defaults: `pin.modbus_tx=18`, `pin.modbus_rx=19`, `pin.modbus_de_re=20`. Change them if your board differs (ESP32-S3 19/20 are often USB D−/D+). No GPIO auto-detect; boot only scans Modbus baud / slave ID if the motor does not answer at `115200`.

Power the ESP32 from USB-C on **USB / Native / JTAG** (flash / CLI). **UART / COM** is UART0 console only.

### Building from Source (Optional for Developers)

Pre-compiled firmware is in Part 3 (flash). To compile:

The firmware is **`esp-hal`** + Embassy + **`esp-rtos`**, `no_std`. Cargo aliases in `.cargo/config.toml`:

```bash
cargo b-c6 --release   # ESP32-C6 (RISC-V)
cargo b-s3 --release   # ESP32-S3 (Xtensa)
```

Path A (`ossm-std`) uses **stable** Rust: `cargo +stable run -p ossm-std` (see **A.3**).

Install the Espressif toolchain with [espup](https://github.com/esp-rs/espup): `espup install --targets esp32c6,esp32s3` (then `source ~/export-esp.sh` if needed).

```bash
./scripts/release.sh
```

builds frontend, flasher, both firmware images. Chip features: `esp32c6` / `esp32s3`.

## Part 3: Flashing the Firmware (`ossm-esp32` only)

You don't need to build from source. Pre-compiled **merged** flash images (`ossm-esp32c6.bin` / `ossm-esp32s3.bin`) are in the repository **Releases**, along with a standalone browser flasher (`flasher.html`) that uses the [Web Serial API](https://developer.mozilla.org/en-US/docs/Web/API/Web_Serial_API) — no command-line tools required.

> [!IMPORTANT]
> **Use the USB / Native / JTAG port to flash!**
> If the board has **two** USB-C ports, plug the flasher into **USB / Native / JTAG**, not **UART / COM**. Native USB talks to the chip directly (Chrome / Edge, no extra USB–UART drivers). UART/COM is only the UART0 console.

### What the flasher looks like

`flasher.html` is a single page with:

| Area | Purpose |
| --- | --- |
| **Status bar** (top) | Connection state plus **Connect**, **Monitor**, **Stop**, **Reset**, and **Disconnect** |
| **Flash Firmware** (left) | Drop / pick a `.bin`, optional flash address (default `0x0`), **Flash Firmware** |
| **Device Configuration** (right) | WiFi, Modbus GPIO pins, Modbus timing, BLE — then **Send Configuration** |
| **Console** (bottom left) | Flasher progress and ACK messages |
| **Serial** (bottom right) | Live device UART output (boot logs, WiFi IP, motor messages) |

Settings you enter are remembered in the browser (`localStorage` key `ossm-flasher-device-config-v1`) so the next visit can reuse them. **Reset to defaults** clears the form back to factory pin / timing defaults.

Language: the embedded controller UI, `flasher.html`, and `motor-control.html` support **English** and **中文** via a header toggle. The choice is stored as `localStorage` key `ossm_locale` (`en` | `zh`). If unset, the apps follow the browser language (`zh*` → Chinese, otherwise English).

### Step-by-step: flash

1.  Download and extract the latest release package (`.zip`) from **Releases**.
2.  Connect the ESP32-C6 or ESP32-S3 with a USB-C **data** cable on the **Native USB / JTAG** port.
3.  Open `flasher.html` in **Google Chrome** or **Microsoft Edge** (Web Serial; desktop browsers only — not Safari / Firefox / most mobile browsers). Double-click the file or drag it into a tab. A file URL is fine; no local web server is required.
4.  Click **Connect** in the status bar. Choose your ESP32 serial port in the browser picker.
    > [!TIP]
    > **Which port is my board?**
    > If several ports appear, use a quick plug test: note the list → cancel → **unplug** the ESP32 → **Connect** again and see which port vanished → plug back in → **Connect** and select the port that reappears.
5.  Under **Flash Firmware**, drag-and-drop (or browse for) the matching merged image:
    - ESP32-C6 → `ossm-esp32c6.bin`
    - ESP32-S3 → `ossm-esp32s3.bin`
    Leave **Flash address** at `0x0` unless you know you need another offset (merged release images are written from `0x0`).
6.  Click **Flash Firmware**. Watch the progress bar and the **Console** panel (`Flash complete` / device reset).
7.  When flashing finishes, the flasher hard-resets the chip and the status becomes **Flash complete**. You usually **do not** need to Connect again before configuration — the USB port stays open.

> [!NOTE]
> **Connect** enters the ROM bootloader (needed for flashing). After a successful flash, the tool leaves bootloader mode so you can configure and monitor the running firmware on the same page.

## Part 4: Network & Device Configuration

Stay on the same `flasher.html` page after flashing. Fill in **Device Configuration** (right column), then click **Send Configuration**.

### Fields

| Section | What to set |
| --- | --- |
| **WiFi** | Toggle **Enable WiFi**. When enabled, enter SSID and password. Disable WiFi if you will use BLE / USB-only. |
| **GPIO Pins** | Modbus **TX** (UART TX → MAX3485 DI), **RX** (UART RX → RO), **DE/RE** (defaults `18` / `19` / `20`). Change these if your wiring differs (especially on ESP32-S3 where 19/20 may be USB). |
| **Motor / Modbus** | Optional timing: read timeout (ms), RX inter-byte (µs), inter-frame quiet (µs), scan delay (µs). Leave at `0` for firmware auto defaults (at 115200: **10 ms** frame timeout, **750 µs** inter-byte, **350 µs** inter-frame). Baud rate `115200` and device ID `1` are fixed in the UI. |
| **BLE** | **Enable Bluetooth Low Energy (BLE)** — leave on unless you want BLE off. |

### Send Configuration

1.  Keep USB connected. Status should be **Flash complete**, **Connected to …**, or **Configuration complete** (those states enable **Send Configuration**). If you previously **Disconnect**ed, click **Connect** again first.
2.  Click **Send Configuration**. The flasher resets the device, waits for boot, then sends UART CLI commands (`set net.*`, `set pin.*`, then `reset`) and waits for each ACK in **Console**.
3.  After a successful send, it automatically starts **Serial** monitoring. Look for WiFi association and a line like `http://<hostname>.local` / an assigned IP. **Copy the IP** (or use `http://ossm.local` if mDNS works on your network) to open the control UI later.
4.  Use **Stop** to pause monitoring, **Monitor** to attach again, or **Reset** to pulse the chip without sending config. **Disconnect** releases the Web Serial port.

If configuration fails (timeout waiting for ACK), check **Console** / **Serial**, ensure you are on the Native USB port, then **Connect** (if needed) and try **Send Configuration** again.

**Note on Motor Initialization**: On every boot the firmware talks to the servo over Modbus. If the motor does not answer at `115200`, the firmware scans other baud rates / slave IDs. If it finds the motor on a different baud rate, it rewrites the drive to `115200` and asks you (in the serial log) to power-cycle the **motor's 24V** supply — unplug motor power for ~3 seconds, then plug it back in.

## Part 5: Usage

Once the device is on your network, you can control it through a web interface or an API. The serial commands are also available for control.

### Web Interface

The firmware hosts an embedded, real-time single-page web interface. Open a browser on a device connected to the same WiFi network and navigate to the IP address or mDNS hostname of your ESP32:

Example: `http://192.168.1.123` or `http://ossm.local`

Key interface features include:
- **Toggleable Real-Time Motor Position Diagram**: Visualizes the full stroke range (`0% – 100%`), physical hardware limits (`pos_min` / `pos_max`), active stroke window with distinct **Left Limit** and **Right Limit** boundaries, and an animated puck showing live motor position at 30 FPS. Can be toggled on or off using the Activity icon in the header, with toggle state persisted across browser sessions.
- **Resilient High-Frequency Live Telemetry (`WsDataManager`)**: Uses a single-owner WebSocket client (`client.ts`) to stream real-time motor updates and loop statistics (`ups`, `dt_min_ms`, `dt_max_ms`, `dt_avg_ms`, `dt_mdev_ms`) at up to **30 FPS (`33ms`)** over WiFi WebSocket (`/ws/command`) when the diagram or settings panel is open. When those panels are closed, the UI falls back to **1 Hz** REST `/state` polling to save WebSocket slots. BLE live telemetry uses a separate path (`ble.ts` / GATT `CHAR_STATE` notifications), not this WebSocket manager. On the firmware side, WebSocket sessions use **split asynchronous RX/TX tasks** (`edge_nal::TcpSplit`) and heap-allocated string payloads backed by a **4-buffer shared pool (`NET_BUFFER_POOL`)**, preventing buffer starvation when concurrent HTTP REST requests arrive during active streaming.
- **Causal config versioning**: Every motor config mutation bumps a monotonic `version` field. The UI tracks `authoritativeVersion` and ignores stale background snapshots so slider edits do not self-revert under concurrent `/state` pushes.
- **Interactive Motion Control, Spline Editor & Macro Player**: Adjust BPM (speed), stroke depth, top/bottom depth anchoring, stroke reversal, pause/resume modes, design custom periodic trajectories with interactive spline control points, or compose timestamped **control macros** (waveform button **Macro**) with import/export, gap seeking, and loop playback. **Funscript** mode is also client-driven.

### Serial Commands

You can also drive the device over USB serial (115200 baud, `\r\n`). In `flasher.html`, use the **Serial** panel (or **Monitor**) for interactive CLI; the **Console** panel is for flash/config tool messages only. The same commands work in any serial terminal.

The firmware `console` task owns USB Serial/JTAG and UART0. Log output (`log::*` / `MODBUS_DBG`) and CLI echo/replies share the wire but use **separate paths**: a priority **`CLI_OUT_CH`** queue and dedicated USB TX ring for CLI bytes. USB TX is **commit-then-wait** (never rewrite a packet already in the 64-byte IN FIFO). With no ACM reader the console **stalls** and drops logs rather than stuffing the endpoint; CLI ACKs stay on the priority path. RX stays armed while draining so late attach still accepts commands. Non-debug motor `ups` remains ~320 Hz on ESP32-C6; `modbus_debug` adds USB log volume but steady-state `ups` is typically only a few percent lower once homing settles.

Configuration uses nmcli-style **`get`** / **`set`** over dotted paths. Type **`paths`** on-device (or `help get` / `help set`) for the full catalog.

```
get <path>                     - Get config section JSON or scalar value
set <path> <value>             - Set config (quote strings with spaces)
paths                          - Print full path/value catalog on device

# Sections (full JSON)
get pin | get net | get motor

# Examples
set net.ssid "MyNetwork"
set net.password secret
set net.wifi_enabled true
set pin.modbus_tx 2
set pin.modbus_debug false
set motor.paused true
set motor {"bpm":36,"depth":1.0,"wave_func":"sine","paused":true}

# Config path catalog
pin.modbus_tx / modbus_rx / modbus_de_re          GPIO 0..48
pin.modbus_timeout_ms                             0..1000 (0 = default ~10 ms)
pin.modbus_rx_timeout_us                          0..200000 (0 = auto)
pin.modbus_scan_delay_us                          0..200000 (0 = t3.5)
pin.modbus_inter_frame_delay_us                   0..200000 (0 = auto)
pin.ble_enabled / pin.modbus_debug                true|false (debug: reboot)
pin.operating_mode                                servo|rtu_relay|rs485
pin.modbus_baud                                   1200..3000000 (default 115200)
net.wifi_enabled / net.dhcp_enabled               true|false
net.ssid / net.password / net.hostname          string
net.static_ip / static_mask / static_gateway / static_dns   IPv4
motor.bpm                                         > 0
motor.depth                                       0.01..1
motor.depth_top / reversed / paused / streaming   true|false
motor.wave_func                                   sine|thrust|spline
motor.sharpness                                   0.01..0.99
motor.paused_position                             0..1
motor.spline_points                               space-separated floats
motor (bulk)                                      full MotorControllerConfig JSON
inject                                            get inject | set inject <off|leading|trailing|both> <nbytes 0..64>

# Actions (not config)
reset                          - Soft reset the MCU
get-state / get-status         - Telemetry JSON
reset-timestamp                - Reset motion stream time
set-waypoints <json>           - Replace waypoint buffer
append-waypoints <json>        - Append waypoints
```

Set ACKs are emitted on the CLI path as `{path} set to {value}` (e.g. `pin.modbus_tx set to 2`) and also logged. Scalar gets print `{path}: {value}`.

### Automated Device Tests (Developers)

Live-hardware scripts in `scripts/` (run with `uv run`); set `DEVICE_IP` in `.env`:

| Script | Purpose |
| --- | --- |
| `./scripts/test_console.py` | USB CLI late-attach, motor `ups` (ACM closed/open), probe-rs RTT, CLI under `MODBUS_DBG` flood |
| `./scripts/test_modbus_debug_device.py` | Modbus CRC resync / inject via **probe-rs RTT** (avoids USB ACM stalls) |
| `./scripts/test_modbus_resync.py` | Host-only CRC/resync mirror (no hardware) |
| `./scripts/test_rs485_transceiver.py` | Live `operating_mode=rs485` (no reboot); USB + `/ws/rs485` via ossm-std |
| `./scripts/stress_dt_max.py` | HTTP+WebSocket load; asserts `dt_max_ms` < 4.5 ms |

### Multi-Mode Control CLI (`ossm.py`)

The repository includes a comprehensive, unified command-line tool (`scripts/ossm.py`) built as a self-contained PEP 723 script. Using [`uv`](https://docs.astral.sh/uv/), you can run it directly without manually configuring a Python virtual environment.

It supports controlling and monitoring your OSSM device across three communication modes: **WiFi** (HTTP REST & WebSocket JSON-RPC), **Bluetooth Low Energy** (`ble`), and **USB Serial** (`serial`).

**Prerequisites:** Install [uv](https://docs.astral.sh/uv/) (`curl -LsSf https://astral.sh/uv/install.sh | sh`).

**Basic Usage & Syntax:**
```bash
# View available commands and options
./scripts/ossm.py --help

# Check device status over WiFi (default mode, IP defaults to 192.168.24.63 or set via -i)
./scripts/ossm.py status -i 192.168.1.123

# Check status over Bluetooth Low Energy (auto-discovers OSSM BLE device)
./scripts/ossm.py status -m ble

# Start the motor at 40 BPM with 80% depth using a spline waveform
./scripts/ossm.py run --bpm 40 --depth 0.8 --wave spline -m wifi

# Immediately pause the motor or park at bottom (pos 0.0)
./scripts/ossm.py pause --pos 0.0 -m ble
./scripts/ossm.py resume -m ble

# View or configure RS-485 Modbus pins and BLE enable toggle over USB serial
./scripts/ossm.py pins -m serial -p /dev/ttyACM0

# Configure network & mDNS hostname
./scripts/ossm.py net -m wifi

# Launch real-time telemetry monitoring TUI dashboard
./scripts/ossm.py monitor -m ble

# Write an example control macro JSON, then play it
./scripts/ossm.py macro --init /tmp/warmup.json
./scripts/ossm.py macro /tmp/warmup.json --loop --speed 1.0 -m wifi
```

When imported as a Python library, `ossm.py` also exports validated Pydantic v2 schemas (`MotorControllerConfig`, `StateResponse`, `PinConfiguration`, `NetworkConfiguration`, etc.) and the `DeviceBackend` class for custom Python automation scripts.

### Advanced Control: The Spline Wave

The `spline` wave is a powerful feature for creating custom motion patterns. Instead of being limited to predefined motions like `sine` or `thrust`, you can define a completely custom movement by providing a sequence of points. The motor will then travel through these points smoothly.

This gives you the creative freedom to design intricate and varied patterns. The firmware uses a technique called Catmull-Rom spline interpolation to generate a smooth, continuous curve that passes exactly through each point you've defined.

**How to use it:**

1.  **Set the points:** Use the `set-spline-points` command, followed by a space-separated list of numbers between 0.0 (fully retracted) and 1.0 (fully extended).
2.  **Activate the wave:** Use the `set-wave spline` command to switch to your custom pattern.

**Examples:**

*   **Simple Stroke:** A basic linear movement.
    `set-spline-points 0 1`
*   **Thrust:** A rapid forward motion followed by a stepped retraction.
    `set-spline-points 0 0 1 0.8 0.5 0.2`
*   **Triangle:** A smooth ramping up and down.
    `set-spline-points 0 0.2 0.4 0.6 0.8 1.0 0.8 0.6 0.4 0.2`
*   **Square Wave:** Holds at the start, then instantly moves and holds at the end.
    `set-spline-points 0 0 0 0 0 1 1 1 1 1`
*   **Vibration:** A jittery, vibrational motion.
    `set-spline-points 0 0.2 0.1 0.4 0.3 0.6 0.5`

### HTTP API

The firmware also provides an HTTP API for programmatic control. All endpoints support CORS, so they can be accessed from web applications running on different domains.

#### `GET /config`

*   **Method:** `GET`
*   **Description:** Retrieves the current motor configuration.
*   **Response Body:** A JSON object representing the motor controller's configuration.

```json
{
  "version": 42,
  "bpm": 60.0,
  "depth": 1.0,
  "depth_top": true,
  "reversed": false,
  "wave_func": "sine",
  "sharpness": 0.5,
  "spline_points": [0.0, 1.0],
  "paused": true,
  "paused_position": 0.5,
  "streaming": false
}
```

*   `version` (number): Monotonically increasing causal timestamp for the configuration. Incremented on every successful config write; clients should reject stale snapshots where `version` regresses. Omit or send `0` to let the firmware assign the next version.
*   `bpm` (number): Beats per minute. Controls the speed of the motion cycle.
*   `depth` (number): The stroke depth, from 0.0 (no movement) to 1.0 (full range).
*   `depth_top` (boolean): Determines the direction of the stroke.
    *   `true`: The stroke moves from the fully retracted position (0.0) to the specified `depth`. For example, a depth of 0.8 would move in the range [0.0, 0.8].
    *   `false`: The stroke moves from `1.0 - depth` to the fully extended position (1.0). For example, a depth of 0.8 would move in the range [0.2, 1.0].
*   `reversed` (boolean): When `true`, reverses the direction of the waveform.
*   `wave_func` (string): The motion pattern. Firmware generators support `"sine"`, `"thrust"`, and `"spline"`. The web UI also offers `"funscript"` and `"macro"` modes; those are client-driven (the UI or `ossm.py` issues timed `set-config` / pause commands) and the firmware falls back to sine if those labels are posted as `wave_func`.
*   `sharpness` (number): Only affects the `"thrust"` waveform. Controls the duration of the thrust, from 0.01 (sharpest) to 0.99 (smoothest).
*   `spline_points` (array of numbers): An array of points (0.0 to 1.0) that define the custom motion path for the `"spline"` waveform.
*   `paused` (boolean): `true` to pause the motor, `false` to run it.
*   `paused_position` (number): The position (0.0 to 1.0) the motor will hold when paused.
*   `streaming` (boolean): When `true`, the motor controller accepts real-time streaming motion commands over WebSocket (`/ws/command`).

##### Control macros (web UI + `ossm.py macro`)

A **macro** is a shareable JSON sequence of timestamped control instructions (`start` / `stop` / `set`). Playback is entirely client-side: the browser Macro Player (waveform button **Macro**) or `./scripts/ossm.py macro <file.json>` waits until each `at` (milliseconds from start) and issues the matching REST/BLE/serial command.

```json
{
  "version": 1,
  "name": "Warm up",
  "loop": false,
  "instructions": [
    { "at": 0, "action": "set", "params": { "bpm": 40, "depth": 0.6, "wave_func": "sine" } },
    { "at": 0, "action": "start" },
    { "at": 15000, "action": "set", "params": { "bpm": 90, "wave_func": "thrust", "sharpness": 0.2 } },
    { "at": 60000, "action": "stop", "params": { "position": 0.0 } }
  ]
}
```

*   `set` params may include any of: `bpm`, `depth`, `depth_top`, `reversed`, `wave_func` (`sine`|`thrust`|`spline`), `sharpness`, `spline_points`.
*   `stop` may optionally park with `params.position` (0.0–1.0).
*   Export/import from the web UI, or generate a starter file with `./scripts/ossm.py macro --init example.json`.

#### `POST /config`

*   **Method:** `POST`
*   **Description:** Updates the motor configuration. You must send a full configuration object, as partial updates are not supported. If you include `version` and it is older than the device's current config, the server responds with **409 Conflict** (`Stale causal version`).
*   **Request Body:** A JSON object with the same structure as the `GET /config` response.
*   **Response Body:** The applied configuration as a JSON object, including the newly incremented `version`.

#### `POST /paused`

*   **Method:** `POST`
*   **Description:** Controls the motor's state when paused. This is useful for making fine adjustments to the position without starting a full motion cycle.
*   **Request Body:** A JSON object with one or more of the following optional fields:
    *   `paused` (boolean): Set to `true` to pause the motor, `false` to resume.
    *   `position` (number): Sets the absolute paused position (from 0.0 to 1.0).
    *   `adjust` (number): Adjusts the position relatively. For example, `0.1` moves it forward by 10%, and `-0.1` moves it back.
*   **Response Body:** The updated configuration as a JSON object.

**Example Request:**
```json
{
  "paused": true,
  "adjust": -0.05
}
```

#### `GET /state`

*   **Method:** `GET`
*   **Description:** Retrieves the current real-time state of the motor. This is useful for UIs that need to display the motor's live position and other metrics.
*   **Response Body:** A JSON object containing the motor's complete current state.

```json
{
  "config": {
    "version": 42,
    "bpm": 60.0,
    "depth": 1.0,
    "depth_top": true,
    "reversed": false,
    "wave_func": "sine",
    "sharpness": 0.5,
    "spline_points": [0.0, 1.0],
    "paused": true,
    "paused_position": 0.5,
    "streaming": false
  },
  "t": 123.45,
  "x": 0.5,
  "y": 1.0,
  "shaped_y": 1.0,
  "position": 10000,
  "speed": 0.0,
  "stream": {
    "buffered": 0,
    "stream_time": 0.0,
    "underrun": false
  },
  "update_history": [60, 60, 60, 60],
  "position_history": [0.0, 0.5, 1.0, 0.5],
  "pos_min": 0.0,
  "pos_max": 25.0
}
```

*   `config`: The full `MotorControllerConfig` object at this moment.
*   `t`: Time offset in seconds since the motion started.
*   `x`: The current phase of the waveform, from 0.0 to 1.0.
*   `y`: The raw output of the waveform generator, from 0.0 to 1.0.
*   `shaped_y`: The waveform output after depth and direction have been applied.
*   `position`: The current absolute position of the motor in its native units.
*   `speed`: The current speed of the motor.
*   `stream`: The status of the streaming motion source (`buffered` waypoints count, current `stream_time`, and whether a buffer `underrun` occurred).
*   `update_history`: Array of recent 1-second window counts of motor position updates (for telemetry update rate in Hz).
*   `position_history`: Array of recent 1-second sampled motor position values.
*   `pos_min`: Physical minimum position limit of the motor.
*   `pos_max`: Physical maximum position limit of the motor.

#### `GET /pin-config`

*   **Method:** `GET`
*   **Description:** Retrieves the current GPIO pin configuration and Modbus communication timing settings.
*   **Response Body:** A JSON object representing the hardware pin and timing configuration.

```json
{
  "modbus_tx": 18,
  "modbus_rx": 19,
  "modbus_de_re": 20,
  "modbus_timeout_ms": 0,
  "modbus_scan_delay_us": 0,
  "modbus_inter_frame_delay_us": 0,
  "ble_enabled": true,
  "modbus_debug": false
}
```

*   `modbus_tx` (number): GPIO pin number assigned to Modbus TX (DI).
*   `modbus_rx` (number): GPIO pin number assigned to Modbus RX (RO).
*   `modbus_de_re` (number): GPIO pin number assigned to Modbus DE/RE control.
*   `modbus_timeout_ms` (number): Modbus per-read timeout in milliseconds (`0` = auto based on baud rate: **10ms** @ 115200 baud, max `1000`).
*   `modbus_rx_timeout_us` (number): Modbus RX inter-byte timeout ($t_{1.5}$) in microseconds (`0` = auto: **750µs** @ 115200 baud per Modbus RTU for >19200 bps).
*   `modbus_scan_delay_us` (number): Modbus scan inter-probe delay in microseconds (`0` = use Modbus t3.5 timing only, up to `200000`).
*   `modbus_inter_frame_delay_us` (number): Modbus inter-frame quiet interval ($t_{3.5}$) in microseconds (`0` = auto based on baud rate: 350µs @ 115200 baud, enabling <3ms total end-to-end request-response cycle time for >300 Hz position updates).
*   `ble_enabled` (boolean): Enable Bluetooth Low Energy GATT server.
*   `modbus_debug` (boolean): Diagnostic Modbus RX mode — **5 ms** RX deadline (same **256 B** UHCI DMA buffers as production), classifies each reply (`empty` / `short` / `exact` / `long` / `long_resync` / `leading_junk` / `parse_fail`), and prints `MODBUS_DBG` TX/RX hex lines on the USB console. **Restart required.** Increases USB log volume; steady-state motor `ups` is typically only a few percent below non-debug (~310 vs ~320 Hz on ESP32-C6). Serial CLI stays responsive under the flood via console **TX/RX fairness** (`CLI_OUT_CH`, separate CLI/log USB rings). Verify with `./scripts/test_console.py --only fairness`; disable after diagnosis. Also via CLI `set pin.modbus_debug`, flasher, or `ossm.py pins --modbus-debug`.
*   `operating_mode` (string): `"servo"` (OSSM motion), `"rtu_relay"` (framed Modbus TCP `:502` + `/ws/modbus`), or `"rs485"` (raw UART1 pipe on USB ACM + `/ws/rs485`). **Live switch — no reboot.** USB CLI is unavailable while `rs485` is active (leave via HTTP/BLE).
*   `modbus_baud` (number): UART1 boot baud (default `115200`). In `rs485` mode, live baud follows USB `SET_LINE_CODING` (ESP32-C6) and `/ws/rs485` CFG packets.

#### `POST /pin-config`

*   **Method:** `POST`
*   **Description:** Updates the GPIO pin configuration and Modbus timing settings. `operating_mode` and UART pin remux apply immediately (UART1 owner restarts the current role). WiFi/SSID still need a restart.
*   **Request Body:** A JSON object with the same structure as the `GET /pin-config` response.
*   **Response Body:** The updated configuration as a JSON object.

### RTU Relay Mode

When `operating_mode` is `"rtu_relay"` (Settings UI, `POST /pin-config`, or CLI `set pin.operating_mode rtu_relay`; **no reboot**), the firmware does not run the motor controller. Instead it bridges RS-485 Modbus RTU:

* **Modbus TCP** on port **502** (standard MBAP)
* **WebSocket** `ws://<device>/ws/modbus` — binary full RTU frames (with CRC)
* Embedded UI shows a relay banner and hides motor controls
* `release/motor-control.html` supports **Remote WebSocket** connection mode
* Tests: `./scripts/test_modbus_relay.py --switch-mode`, `./scripts/test_modbus_tcp.py`

### RS-485 transceiver mode

When `operating_mode` is `"rs485"`, UART1 is a raw byte pipe (host owns RTU timing):

* USB ACM is a data plane (no CLI / USB logs). UART0/RTT logs stay. Leave via HTTP/BLE `operating_mode`.
* WebSocket `ws://<device>/ws/rs485` — packets `u8 type | u16le len | payload` (`0` TX, `1` RX, `2` CFG JSON `{"baud":115200}`)
* ESP32-C6 applies USB `SET_LINE_CODING` `dwDTERate` to UART1
* Firmware drives DE/RE for the pipe. Host `--serial` USB-RS485 adapters must also auto-switch direction (ossm-std does not strip TX echoes; loopback adapters are unsupported).
* `ossm-std --mode rtu-relay --serial …` or `--rs485-ws ws://HOST/ws/rs485` serves Modbus TCP for motor-control
* Tests: `./scripts/test_rs485_transceiver.py`

#### `POST /restart`

*   **Method:** `POST`
*   **Description:** Triggers a soft reset and restarts the ESP32 microcontroller.
*   **Response Body:** `{"ok":true}`

#### `GET /ws/command`

*   **Method:** `GET` (WebSocket Upgrade)
*   **Description:** A WebSocket endpoint for real-time streaming of motion commands. This allows external applications to stream trajectory waypoints dynamically or query streaming status.
*   **Protocol & Format:** Supports both traditional flat JSON command frames (`{"cmd": "..."}`) and JSON-RPC 2.0 style command/response messages (`{"jsonrpc": "2.0", "method": "...", "params": {...}, "id": 1}`). If an `id` is provided in the request, the server replies with a matching JSON-RPC response (`{"jsonrpc": "2.0", "id": 1, "result": ...}`).
*   **Supported Commands**:
    *   **Append Waypoints**: Appends a list of trajectory waypoints to the streaming buffer.
```json
{
  "jsonrpc": "2.0",
  "method": "append-waypoints",
  "params": [
    { "ts": 1000, "pos": 0.5, "vel": 0.2 },
    { "ts": 1050, "pos": 0.7 }
  ],
  "id": 1
}
```
        *   `ts` (number): Sender clock timestamp in milliseconds.
        *   `pos` (number): Target normalized position (from 0.0 to 1.0).
        *   `vel` (optional number): Target velocity in normalized units/second.
    *   **Set Waypoints**: Clears currently buffered waypoints and replaces them with a new list of trajectory waypoints. Optionally accepts an object with `reset-timestamp` set to `true` to drop the current stream time anchoring so that the first waypoint re-anchors the clock.
```json
{
  "jsonrpc": "2.0",
  "method": "set-waypoints",
  "params": {
    "waypoints": [
      { "ts": 1000, "pos": 0.5, "vel": 0.2 }
    ],
    "reset-timestamp": true
  },
  "id": 2
}
```
    *   **Reset Timestamp**: Clears buffered waypoints and resets the streaming timestamp epoch so that the next received waypoint re-anchors the stream.
```json
{
  "jsonrpc": "2.0",
  "method": "reset-timestamp",
  "id": 2
}
```
    *   **Status**: Requests the current `StreamStatus` (`{"buffered": 0, "stream_time": 0.0, "underrun": false}`).
```json
{
  "jsonrpc": "2.0",
  "method": "status",
  "id": 3
}
```
    *   **Ping**: Application-level liveness check. Replies with `"pong"`.
```json
{
  "jsonrpc": "2.0",
  "method": "ping",
  "id": 4
}
```
    *   **Get State**: Returns the comprehensive motor controller state (`StateResponse`, including real-time loop telemetry `ups`, `dt_min_ms`, `dt_max_ms`, `dt_avg_ms`, and `dt_mdev_ms`).
```json
{
  "jsonrpc": "2.0",
  "method": "get-state",
  "id": 5
}
```
    *   **Set Config**: Dynamically updates the motor controller configuration (`MotorControllerConfig`). Returns the applied config including the incremented `version`. Rejects stale `params.version` with JSON-RPC error `-32001` (`Stale causal version`).
```json
{
  "jsonrpc": "2.0",
  "method": "set-config",
  "params": {
    "bpm": 60,
    "depth": 0.8,
    "paused": false
  },
  "id": 6
}
```
    *   **Subscribe State**: Subscribes the WebSocket connection to periodic state notifications pushed by the server.
```json
{
  "jsonrpc": "2.0",
  "method": "subscribe-state",
  "params": {
    "interval_ms": 33
  },
  "id": 7
}
```
        *   When subscribed, the server periodically pushes lightweight JSON-RPC notifications of the form `{"jsonrpc": "2.0", "method": "state", "params": { ...StateResponse... }}`. Supports intervals down to `20ms` (`50 FPS`). Re-sending `subscribe-state` adjusts the interval without unsubscribing first.
        *   **Asynchronous RX/TX Architecture**: WebSocket sessions split the underlying TCP connection into separate read (`ws_recv`) and write (`ws_send`) tasks communicating via an Embassy channel with heap-allocated strings. High-frequency state pushes never block or delay incoming JSON-RPC command processing.
        *   **BLE Telemetry**: GATT reads/notifications on `CHAR_STATE` use a **compact** JSON subset (chunked when needed). Full `StateResponse` (loop telemetry, history arrays, etc.) is available via JSON-RPC `get-state` on `CHAR_RPC`. BLE `subscribe-state` pushes the compact format on `CHAR_STATE`, not the full WiFi WebSocket payload.
        *   **Idle Auto-Disconnect**: To conserve device memory and WebSocket session slots (`WS_MAX` = 3 concurrent), the embedded web interface automatically disconnects the WebSocket after 3 seconds of inactivity when no active listeners remain.
        *   **Persistent Position Diagram Toggle**: The UI remembers the visibility toggle state of the Motor Position Diagram in browser `localStorage`.
    *   **Unsubscribe State**: Stops periodic state pushing.
```json
{
  "jsonrpc": "2.0",
  "method": "unsubscribe-state",
  "id": 8
}
```
