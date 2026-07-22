# OSSM-Rust Firmware

This repository contains the firmware for an OSSM controller, written in Rust. It's designed to run on ESP32-C6 or ESP32-S3 microcontrollers and control a 57AIM30 integrated servo motor.

This guide is designed to be friendly for everyone, even if you have no technical or engineering background! Please follow these step-by-step instructions to get your hardware wired and running smoothly.

## Part 1: Required Hardware

Here is a list of components you will need to build the controller.

| Component | Description | Notes |
| --- | --- | --- |
| **Microcontroller** | ESP32-C6 or ESP32-S3 Development Board | Any ESP32-C6 or ESP32-S3 board with a USB-C connector will work.<br><br>**Important Note on USB Ports:** Many development boards have **two** USB-C ports (often labeled "USB" / "Native" / "JTAG" vs "UART" / "COM"). Please always use the **Native JTAG / USB port** instead of the USB-to-UART converter port! The native JTAG port connects directly to the processor for faster, hassle-free flashing without needing external driver chips. |
| **Motor** | 57AIM30 Integrated Servo Motor | **Important:** Make sure to get the `57AIM30` model, not the `57AIM30H`. The `57AIM30` has a rated speed of 1500 RPM and 0.96 Nm of torque, which is ideal for this application. |
| **RS485 Transceiver**| MAX3485 Module | This small board translates signals between the ESP32 microcontroller and the motor. |
| **Power Supply (Motor)** | 24V DC Power Adapter | To power the servo motor. Make sure it can supply enough current for your mechanical load. |
| **Power Supply (ESP32)**| 5V USB Charger | Any standard USB phone charger with a USB-C cable will work to power the controller once everything is set up. |
| **Cables & Connectors** | - DC 5.5×2.1/2.5mm female jack<br>- **KF2EDGK-3.81 6P** terminal plug<br>- **PHB2.0 2×5P** cable / connector<br>- Jumper (DuPont) wires | **Important Note on Motor Connectors:** Check the box your 57AIM30 motor arrived in. If the matching green and white plugs were not included, you will need to buy them separately:<br>• **For Motor Power (front 6-pin socket):** buy a **KF2EDGK-3.81 6P** pluggable terminal block.<br>• **For Communication & Logic (back 10-pin socket):** buy a **PHB2.0 2×5P** (2.0mm pitch, 2x5 pin) connector or pre-crimped cable.<br>The DC female jack connects your 24V power adapter to the green KF2EDGK power terminal. |

## Part 2: Wiring & Hardware Assembly

Connecting the components correctly is crucial. Don't worry if you're new to this—just take it step by step! Please double-check all connections before plugging in any power supplies.

### Complete Hardware Wiring Diagram
Below is the complete visual system diagram illustrating the connections between your power adapters, ESP32 development board, MAX3485 transceiver, and the 57AIM30 servo motor's front and back terminals:

<div align="center">
  <img src="./assets/wiring_diagram.svg" alt="OSSM Hardware Wiring Diagram" width="100%"/>
</div>

> [!TIP]
> **Which connector goes where?**
> Your 57AIM30 motor has two sockets:
> • **Front 6-Pin Socket (Power):** Uses the green **KF2EDGK-3.81 6P** connector for the 24V heavy-duty motor power.
> • **Back 10-Pin Socket (Communication):** Uses the white **PHB2.0 2×5P** connector for 5V logic power and RS-485 signal wires.

### Step 1: Motor Power Connection (Front 6-Pin Terminal)

This terminal supplies the 24V power needed to drive the motor shaft. Use your green **KF2EDGK-3.81 6P** connector here.

| Motor Terminal (+V) | --> | 24V Power Supply (Positive `+`) |
| --- | --- | --- |
| Motor Terminal (GND) | --> | 24V Power Supply (Negative `-`) |

> [!CAUTION]
> **Check Polarity!** Double-check that positive (`+`) and negative (`-`) wires are not reversed before plugging in your 24V power adapter!

### Step 2: Communication & Logic Power (Back 2×5-Pin Terminal & ESP32)

This back terminal uses the **PHB2.0 2×5P** connector. It powers the motor's internal smart controller (5V) and sends control commands back and forth using the RS-485 transceiver board (MAX3485).

First, connect the motor's communication wires to the MAX3485 RS-485 module:

| Motor Terminal (485A) | --> | MAX3485 Module (A) |
| --- | --- | --- |
| Motor Terminal (485B) | --> | MAX3485 Module (B) |

Next, connect your ESP32 development board to the MAX3485 module and the motor's logic power pins:

| ESP32 Pin | --> | Component Pin | Purpose |
| --- | --- | --- | --- |
| 5V (or VBUS/VIN) | --> | Motor Terminal (5V) | Powers the motor's internal logic chip |
| GND (Ground) | --> | Motor Terminal (COM / GND) | Common ground for signal stability |
| 3.3V (or 3V3) | --> | MAX3485 Module (VCC / 3.3V) | Powers the RS-485 module |
| GND (Ground) | --> | MAX3485 Module (GND) | Ground for the RS-485 module |
| GPIO 18 (TX) | --> | MAX3485 Module (DI / TX) | Sends data to motor |
| GPIO 19 (RX) | --> | MAX3485 Module (RO / RX) | Receives data from motor |
| GPIO 20 | --> | MAX3485 Module (DE / RE) | Controls data transmission direction |

> [!NOTE]
> **About GPIO pins**: GPIO 18, 19, and 20 are the default Modbus pins in firmware NVS (same defaults on both chips). On ESP32-S3 boards, pins 19 and 20 are often reserved for USB Serial/JTAG — wire Modbus TX/RX/DE-RE to free GPIOs and set them via the web UI, serial CLI (`set-pin-modbus-*`), or the flasher config panel. There is **no automatic GPIO pin detection**; only Modbus baud rate / slave ID scanning runs at boot when the motor does not respond at `115200`.

### Step 3: Powering the ESP32 Board

Finally, connect your ESP32 board to a standard 5V USB charger or your computer using a USB-C cable:

| ESP32 Type-C Port | --> | 5V USB Charger / Computer |
| --- | --- | --- |

> [!IMPORTANT]
> Remember: If your ESP32 board has two USB-C ports, always plug your cable into the **Native JTAG / USB port** (not the UART/COM port)!

### Building from Source (Optional for Developers)

You don't need to build the code yourself—pre-compiled firmware is provided in Part 3! But if you are a developer and wish to compile from source:

The firmware is built on **`esp-hal`** (bare-metal) with **Embassy** async and **`esp-rtos`** for task scheduling, targeting `no_std`. Both targets are supported via cargo aliases defined in `.cargo/config.toml`:

```bash
# Both targets use the Espressif `esp` channel pinned in rust-toolchain.toml
cargo b-c6 --release   # ESP32-C6 (RISC-V)
cargo b-s3 --release   # ESP32-S3 (Xtensa)
```

Install the toolchain with [espup](https://github.com/esp-rs/espup): `espup install --targets esp32c6,esp32s3` (then `source ~/export-esp.sh` if your shell does not pick up `esp` automatically).

To build everything (frontend, flasher, both firmware targets) and produce merged flash images:
```bash
./scripts/release.sh
```

Chip selection is via Cargo features / aliases (`esp32c6` / `esp32s3`). Default Modbus GPIO numbers are the same for both; change them in NVS if your board wiring differs.

## Part 3: Flashing the Firmware

You don't need to build from source. Pre-compiled **merged** flash images (`ossm-esp32c6.bin` / `ossm-esp32s3.bin`) are in the repository **Releases**, along with a standalone browser flasher (`flasher.html`) that uses the [Web Serial API](https://developer.mozilla.org/en-US/docs/Web/API/Web_Serial_API) — no command-line tools required.

> [!IMPORTANT]
> **Use the Native USB / JTAG Port!**
> If your ESP32 board has **two** USB-C ports (often labeled "USB", "Native", or "JTAG" vs "UART", "COM", or "CP2102"), **always plug into the Native JTAG / USB port**. That port talks to the chip directly and works with Chrome / Edge without extra USB–UART drivers.

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
| **GPIO Pins** | Modbus **TX**, **RX**, and **DE/RE** (defaults `18` / `19` / `20`). Change these if your wiring differs (especially on ESP32-S3 where 19/20 may be USB). |
| **Motor / Modbus** | Optional timing: read timeout (ms), RX inter-byte (µs), inter-frame quiet (µs), scan delay (µs). Leave at `0` for firmware auto defaults (at 115200: **10 ms** frame timeout, **750 µs** inter-byte, **350 µs** inter-frame). Baud rate `115200` and device ID `1` are fixed in the UI. |
| **BLE** | **Enable Bluetooth Low Energy (BLE)** — leave on unless you want BLE off. |

### Send Configuration

1.  Keep USB connected. Status should be **Flash complete**, **Connected to …**, or **Configuration complete** (those states enable **Send Configuration**). If you previously **Disconnect**ed, click **Connect** again first.
2.  Click **Send Configuration**. The flasher resets the device, waits for boot, then sends UART CLI commands (`set-wifi-*`, `set-pin-modbus-*`, `set-modbus-*`, `set-ble-enabled`, then `reset`) and waits for each ACK in **Console**.
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

```
help                           - Show this help message
reset                          - Soft reset the MCU
set-wifi-ssid <ssid>           - Set WiFi SSID
set-wifi-password <password>   - Set WiFi password
get-pin-configuration          - Get pin configuration in JSON format
set-pin-modbus-tx <pin>        - Set Modbus TX pin
set-pin-modbus-rx <pin>        - Set Modbus RX pin
set-pin-modbus-de-re <pin>     - Set Modbus DE/RE pin
get-motor-config               - Get motor config in JSON format
set-motor-config <json>        - Set motor config from a JSON string
pause                          - Pause the motor
start                          - Start the motor
set-bpm <bpm>                  - Set motor BPM
set-wave <sine|thrust|spline>  - Set motor waveform
set-paused-position <position> - Set motor position when paused (0.0 to 1.0)
set-depth <depth>              - Set motor stroke depth (0.0 to 1.0)
set-depth-top <true|false>     - Set depth direction
set-sharpness <sharpness>      - Set sharpness for thrust wave (0.01 to 0.99)
set-spline-points <p1> <p2>... - Set points for spline wave (0.0 to 1.0)
set-modbus-timeout-ms <val>    - Set Modbus per-read timeout in ms (0 = use default for baud rate)
get-modbus-timeout-ms          - Get Modbus per-read timeout in ms
set-modbus-scan-delay-us <val> - Set Modbus scan inter-probe delay in us (0 = use Modbus t3.5 only)
get-modbus-scan-delay-us       - Get Modbus scan inter-probe delay in us
```

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
*   `modbus_debug` (boolean): Diagnostic Modbus RX mode — **5 ms** RX deadline (same **256 B** UHCI DMA buffers as production), classifies each reply (`empty` / `short` / `exact` / `long` / `leading_zero` / `leading_junk` / `parse_fail`), and prints `MODBUS_DBG` TX/RX hex lines on the USB console. **Restart required.** Collapses motor UPS; disable after diagnosis. Also via CLI `set-modbus-debug`, flasher, or `ossm.py pins --modbus-debug`.
*   `operating_mode` (string): `"servo"` (default OSSM motion controller) or `"rtu_relay"` (Modbus RTU bridge). **Restart required.**

#### `POST /pin-config`

*   **Method:** `POST`
*   **Description:** Updates the GPIO pin configuration and Modbus timing settings. You must restart the microcontroller for pin assignment / `operating_mode` changes to take effect.
*   **Request Body:** A JSON object with the same structure as the `GET /pin-config` response.
*   **Response Body:** The updated configuration as a JSON object.

### RTU Relay Mode

When `operating_mode` is `"rtu_relay"` (Settings UI, `POST /pin-config`, or CLI `set-operating-mode rtu_relay`, then restart), the firmware does not run the motor controller. Instead it bridges RS-485 Modbus RTU:

* **Modbus TCP** on port **502** (standard MBAP)
* **WebSocket** `ws://<device>/ws/modbus` — binary full RTU frames (with CRC)
* Embedded UI shows a relay banner and hides motor controls
* `release/motor-control.html` supports **Remote WebSocket** connection mode
* Tests: `./scripts/test_modbus_relay.py --switch-mode`, `./scripts/test_modbus_tcp.py`

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
