# OSSM Rust - AI Agents Reference Guide

This document serves as an architectural overview, operational reference, and rulebook for AI assistants and coding agents working on the **OSSM Rust** repository.

---

## 1. Project Overview & Architecture

**OSSM Rust** is a bare-metal (`no_std`) Rust firmware for ESP32-C6 and ESP32-S3 microcontrollers controlling an Open Source Sex Machine (OSSM) servo motor with embedded driver (e.g., 57AIM30) over an RS-485 Modbus UART interface. Built on **`esp-hal`** + **`esp-rtos`** + **Embassy** async, it includes an **`edge-http`** HTTP/WebSocket server, `trouble-host` BLE GATT server, Vue-based web interface, and web flasher.

### Core Components & File Mapping
- **`src/main.rs`**: System initialization via `esp_hal::init`, WiFi connection (with DHCP or static IP configuration) using `esp-radio`, and main Embassy event loops via `esp-rtos`. The `esp-println` logger **must** be initialized after `esp_rtos::start()` with an explicit `log::set_max_level(log::LevelFilter::Info)` call to override `esp-rtos`'s default log level. On ESP32-C6, the motor runs on an `InterruptExecutor` at elevated priority. On ESP32-S3, the motor runs on a dedicated `esp_rtos::embassy::Executor` on Core 1. Network tasks (`net_task`, `nvs_saver_task`, `http_task`, `ble_task`, `cli_task`) run on the main executor.
- **`src/http_api.rs`**: Implements the `edge-http` + `edge-nal-embassy` web server (4 acceptors, socket queue, serialized REST gate, up to 3 concurrent WebSocket sessions), REST endpoints (`/config`, `/state`, `/pin-config`, etc.), and the JSON-RPC 2.0 WebSocket command handler (`/ws/command`) via `edge-ws`. The frontend HTML is embedded at compile time via `include_bytes!`.
- **`src/ble_api.rs`**: Implements the Bluetooth Low Energy (BLE) GATT server (`6e400001-...`) using `trouble-host` with `bt-hci`. Characteristics for config, state, paused control, pin config, RPC, and network config. The BLE runner is wrapped in a retry loop for resilience against transient controller errors. Advertising uses relaxed intervals (250-500ms) for WiFi coexistence and includes the service UUID in scan response data.
- **`src/command.rs`**: Handles USB serial CLI via `esp_hal::uart::UsbSerialJtag` with `embedded-cli`. Configuration commands for WiFi credentials, mDNS hostname, static IP/DHCP mode, Modbus GPIOs, motor config, and BLE toggle.
- **`src/storage.rs`**: `StorageManager` using `esp-nvs` (backed by `esp-storage`) to persist WiFi credentials, Modbus pin configurations (`PinConfiguration`), network configurations (`NetworkConfiguration`), and motor settings.
- **`src/wifi.rs`**: WiFi station setup using `esp-radio` and `embassy-net` with `smoltcp`. Supports DHCP and static IP configuration.
- **`src/motion.rs`**: Defines `MotorControllerConfig`, motion trajectory generation, and `StateResponse` data structures (including physical bounds `pos_min`/`pos_max` and update rate telemetry `update_history`). Uses `libm` for `no_std` floating-point math.
- **`src/motor_57aim30.rs` & `src/motor.rs`**: Modbus RTU communication via `esp_hal::uart::Uart` and `rmodbus` with motor controller driver implementation.
- **`src/rpc.rs`**: Shared JSON-RPC 2.0 command dispatcher used by both HTTP/WebSocket and BLE code paths.
- **`src/context.rs`**: `AppContext` holding shared application state behind `embassy_sync::Mutex`.
- **`src/error.rs`**: Firmware error types.
- **`frontend/`**: Vue 3 + TypeScript single-page application embedded into the firmware for real-time motor control and configuration over WiFi. Bundled into a single HTML file (`dist/index.html`) via `vite-plugin-singlefile`.
- **`flasher/`**: Standalone browser-based serial flasher and device configurator built with Vue 3, `esptool-js`, and xterm.js (`dist/index.html` -> `release/flasher.html`).
- **`scripts/`**: Self-contained `uv` Python automation scripts. Includes `ossm.py` (unified dual-mode CLI and shared `DeviceBackend`), `setup_device.py`, `test_websocket.py`, `test_frontend.py`, and `test_ble.py`.
- **`assets/`**: Standalone HTML templates (`wiring_diagram.html`, `wiring_diagram_zh.html`) with dynamic JS auto-wiring and their exported vector graphics (`wiring_diagram.svg`, `wiring_diagram_zh.svg`) embedded in user documentation.

---

## 2. Build & Release Workflow

### Toolchain & Cargo Aliases
This is a `no_std` bare-metal project using `esp-hal`. Custom target aliases are defined in `.cargo/config.toml`:
- **ESP32-C6 (`riscv32imac-unknown-none-elf`)**:
  ```bash
  cargo b-c6 --release
  ```
- **ESP32-S3 (`xtensa-esp32s3-none-elf`)**:
  ```bash
  cargo b-s3 --release
  ```
- **Clippy & Linting** (C6 only):
  ```bash
  cargo clippy --target riscv32imac-unknown-none-elf -- -D warnings
  ```

### Build System Details
- `build-std = ["core", "alloc"]` is enabled in `.cargo/config.toml` (no `std` available).
- `build.rs` emits `cargo:rustc-link-arg=-Tlinkall.x` for the esp-hal linker script.
- `esp-bootloader-esp-idf` provides the ESP-IDF app descriptor macro required by `espflash`.
- Feature flags (`esp32c6` / `esp32s3`) gate chip-specific dependencies across `esp-hal`, `esp-rtos`, `esp-radio`, `esp-storage`, `esp-nvs`, `esp-alloc`, `esp-backtrace`, `esp-println`, and `esp-bootloader-esp-idf`.

### Full Release Script
To build the frontend, flasher, and firmware for both ESP32-C6 and ESP32-S3, and generate merged `.bin` flash images:
```bash
./scripts/release.sh
```
Merged binaries are output to `release/ossm-esp32c6.bin` and `release/ossm-esp32s3.bin`.

**Important:** `cargo b-c6 --release` only updates `target/.../ossm-rust`. Flashing via `setup_device.py --flash` uses the merged `release/ossm-*.bin` image.
---

## 3. Environment Configuration & Secrets (`.env`)

**NEVER commit secrets, WiFi passwords, or hardcoded IP addresses to version control.**
All environment-specific configurations are stored in `.env` in the project root (ignored by git). See `.env.example` for the schema:

```env
# Target device chip model (esp32c6 or esp32s3)
DEVICE_MODEL="esp32c6"

# Serial port configuration
DEVICE_PORT="/dev/ttyACM0"
DEVICE_BAUD="115200"

# WiFi credentials
WIFI_SSID="Your-SSID"
WIFI_PASSWORD="Your-Password"

# Modbus pin configuration
MODBUS_TX="2"
MODBUS_RX="1"
MODBUS_DERE="0"

# Target device IP address (auto-updated by setup_device.py)
DEVICE_IP="192.168.x.x"
```

---

## 4. Device Setup & Flashing (`/scripts`)

All scripts in `/scripts` are formatted as PEP 723 self-contained inline script declarations, allowing execution directly with `uv run` without setting up a virtual environment manually.

### Unified Dual-Mode CLI & Shared Backend (`scripts/ossm.py`)
A comprehensive command-line tool and shared library class (`DeviceBackend`) used across the automation suite. BLE mode maintains a **persistent connection** across operations using a dedicated asyncio event loop, avoiding costly reconnections per operation.
- **Supported Modes**: `--mode wifi` (HTTP REST & WebSocket JSON-RPC), `--mode ble` (`bleak` GATT blobs & JSON-RPC notifications with persistent connection), `--mode serial` (USB UART CLI).
- **Commands**: `status` (display formatted status table), `set` (update parameters), `pause`/`resume`, `pins`, `net` (view/configure network & mDNS settings), `wifi`, `restart`, `monitor` (live UART stream), and `dash` (interactive live telemetry TUI dashboard).
- **Usage Example**:
  ```bash
  ./scripts/ossm.py status -m wifi
  ./scripts/ossm.py dash -m ble
  ```

### Automated Flashing & Configuration (`scripts/setup_device.py`)
To flash the release binary and configure the physical device over USB serial using `DeviceBackend`:
```bash
./scripts/setup_device.py --flash
```
- Automatically reads `DEVICE_MODEL`, `DEVICE_PORT`, `WIFI_SSID`, `WIFI_PASSWORD`, and Modbus GPIO pins from `.env`.
- Invokes `espflash write-bin` against `release/ossm-<model>.bin` (if `--flash` is passed). Ensure this merged image is current (see §2).
- Sends serial CLI commands (`set-wifi-ssid`, `set-pin-modbus-tx`, etc.) via `DeviceBackend`, issues a soft reset, and monitors logs until WiFi connects.
- **Auto-Discovery**: Automatically captures the assigned IP address from WiFi logs and updates `DEVICE_IP="..."` in `.env`.

### Frontend, BLE & Concurrent HTTP Testing (`scripts/test_frontend.py`)
Combined end-to-end verification of HTTP concurrency, BLE connectivity, and the embedded Vue UI against a live device:
```bash
./scripts/test_frontend.py
```
- Reads `DEVICE_IP` from `.env`.
- **Concurrent HTTP**: fires 4 parallel clients per round (matching the firmware acceptor queue), burst `/state` requests, and a threaded multi-endpoint burst. All must return HTTP 200 (REST is serialized server-side but connections are accepted and queued, not rejected).
- **BLE Integration**: connects to the OSSM BLE device, verifies GATT characteristic reads (pin config, motor config/state, network config), JSON-RPC over BLE (`ping`), config write/readback, and chunked state notification telemetry via `subscribe-state`. BLE tests run in a separate thread to avoid asyncio event loop conflicts with Playwright.
- **Playwright E2E**: loads the embedded UI, verifies WebSocket state streaming (~30 FPS), motor diagram toggle persistence, settings panel telemetry, and main controls.

### Automated WebSocket Testing (`scripts/test_websocket.py`)
To run end-to-end verification of all WebSocket JSON-RPC endpoints against the live device over WiFi:
```bash
./scripts/test_websocket.py
```
- Connects to `ws://<DEVICE_IP>/ws/command` (using `DeviceBackend` and `DEVICE_IP` from `.env`).
- Verifies `ping`, `status`, `get-state`, `set-config`, `subscribe-state` (with live push notification assertions), `unsubscribe-state`, `append-waypoints`, and `set-waypoints`.

### Automated BLE GATT & RPC Testing (`scripts/test_ble.py`)
To verify Bluetooth Low Energy (BLE) GATT characteristics, read/write configuration, execute JSON-RPC over BLE, and run live motor motion tests:
```bash
./scripts/test_ble.py
```
- Automatically scans for and connects to the OSSM BLE device using `DeviceBackend` with a **persistent BLE connection** (single connect for all operations).
- Verifies `CHAR_PIN_CONFIG`, `CHAR_CONFIG`, `CHAR_STATE`, `CHAR_PAUSED`, and JSON-RPC commands over `CHAR_RPC`.
- Runs the motor and verifies live chunked telemetry notifications over `CHAR_STATE` using `BleChunkReassembler`, asserting both notification count and JSON schema correctness.

---

## 5. Web Applications Architecture (`/frontend` & `/flasher`)

Both web applications in this repository are built using **Vue 3 (Composition API)**, **TypeScript**, **Tailwind CSS**, and **Vite**. A key architectural choice for both apps is the use of `vite-plugin-singlefile`, which bundles all HTML, JavaScript, and CSS into a single self-contained document without external asset dependencies.

### Embedded Web Interface (`/frontend`)
The frontend is a single-page application embedded directly into the ESP32 firmware to provide real-time motor control and monitoring over WiFi.
- **Single-File Embedding**: Compiles to `frontend/dist/index.html`. During build (`build.rs`), it is compressed to `index.html.gz` in `OUT_DIR`. This compressed file is statically included into the firmware binary at compile time via `include_bytes!(concat!(env!("OUT_DIR"), "/index.html.gz"))` in `src/http_api.rs` and served with `Content-Encoding: gzip` directly from ROM without filesystem overhead.
- **Key Components (`frontend/src/components/`)**:
  - `MotorPositionDiagram.vue`: Toggleable visual diagram displaying full stroke range (`0% - 100%`), hardware bounds (`pos_min`/`pos_max`), active stroke window (Left Limit & Right Limit markers), and real-time animated position indicator at 30 FPS.
  - `MainControl.vue`: Manages core motion settings including BPM (speed), stroke depth, top/bottom depth anchoring, stroke reversal, and pause/resume functionality (supporting fixed-position and in-place pause modes).
  - `SplineEditor.vue`: Interactive curve editor allowing users to design custom periodic motion trajectories (`wave_func: 'spline'`) by manipulating spline control points.
  - `ModbusSettings.vue`: Configures RS-485 Modbus UART GPIO pins (`modbus_tx`, `modbus_rx`, `modbus_de_re`), communication timeout, scan delay, device rebooting, and displays live motor update frequency (`updates/sec` rate in Hz).
- **Communication & State (`frontend/src/api.ts`, `client.ts`, & `types.ts`)**:
  - Communicates with the firmware via HTTP REST/JSON endpoints (`/config`, `/state`, `/paused`, `/pin-config`, `/restart`).
  - Implements a single-owner WebSocket client (`WsDataManager` in `client.ts`) for resilient real-time state synchronization via JSON-RPC 2.0 (`/ws/command`) at up to 30 FPS (`33ms`). Features automatic request/response promise routing (`pendingRequests`), application-level watchdog silent-failure detection, automatic reconnection loops (`triggerWsReconnect`) that re-subscribe active listeners without page refreshes, and automatic idle disconnection after 3 seconds when UI panels are closed.
  - When running locally via `npm run dev`, it proxies requests to `http://ossm.lan`.

### Web Serial Flasher & Configurator (`/flasher`)
The flasher is a standalone, browser-based tool (`flasher/dist/index.html` -> `release/flasher.html`) that enables users to flash firmware binaries and configure microcontrollers over USB without installing local command-line tools.
- **Web Serial API & esptool-js**: Uses `@w3c/web-serial` and `esptool-js` (`ESPLoader`, `Transport`) to connect to ESP32-C6 / ESP32-S3 bootloaders directly from supported browsers (Chrome, Edge, Opera).
- **Firmware Flashing**: Supports drag-and-drop or file selection of merged firmware images (e.g., `release/ossm-esp32c6.bin`) and writes them to flash memory (default address `0x0`).
- **Integrated Terminal (`@xterm/xterm`)**: Features an embedded serial terminal with `@xterm/addon-fit` for live serial monitoring and interactive UART CLI communication at `115200` baud.
- **Post-Flash Device Configuration**: Can send automated UART CLI commands (`set-wifi-ssid`, `set-wifi-password`, `set-pin-modbus-tx`, etc.) over serial to configure NVS settings on newly flashed devices. Config presets are saved locally in browser `localStorage` under `ossm-flasher-device-config-v1`.

---

## 6. API Protocols & Command Reference

### WebSocket JSON-RPC 2.0 (`/ws/command`)
All requests must adhere to JSON-RPC 2.0 format:
```json
{
  "jsonrpc": "2.0",
  "method": "<command_name>",
  "params": { ... },
  "id": 1
}
```

#### Supported Methods
1. **`ping`**: Returns `"pong"` in `result`. Used for application-level liveness checking.
2. **`status`**: Returns stream buffer status (`{"buffered": 0, "stream_time": 0.0, "underrun": false}`).
3. **`get-state`**: Returns current `StateResponse` object (motor position, speed, coordinates, physical bounds `pos_min`/`pos_max`, live loop telemetry `ups`, `dt_min_ms`, `dt_max_ms`, `dt_avg_ms`, `dt_mdev_ms`, update rate `update_history`, and config).
4. **`set-config`**: Accepts partial or full `MotorControllerConfig` in `params` to dynamically update motion parameters.
5. **`subscribe-state`**: Accepts `{"interval_ms": 300}` in `params` (range: 20–60000ms, default: 33ms). Acknowledges with `"result": {"subscribed": true, "interval_ms": 300}` confirming the resolved push rate. Re-sending `subscribe-state` dynamically adjusts the interval without requiring an unsubscribe. Begins pushing periodic notifications from the server:
   ```json
   {
     "jsonrpc": "2.0",
     "method": "state",
     "cmd": "state",
     "params": { ...StateResponse... },
     "state": { ...StateResponse... }
   }
   ```
6. **`unsubscribe-state`**: Stops periodic state push notifications. Returns `"result": "unsubscribed"`.
7. **`reset-timestamp`**: Resets motion trajectory time `t` to 0.
8. **`append-waypoints`**: Appends a list of motion waypoints (`[{"ts": 100, "pos": 0.5, "vel": 0.1}, ...]`) to the stream buffer.
9. **`set-waypoints`**: Clears currently buffered waypoints and sets a new list of motion waypoints in the buffer. Supports passing a list directly or an object `{"waypoints": [...], "reset-timestamp": true}` to optionally reset the stream time anchoring.
10. **`get-network-config`**: Returns current `NetworkConfiguration` object (mDNS hostname, DHCP status, static IP settings).
11. **`set-network-config`**: Updates `NetworkConfiguration` object in NVS (reboot required).

### Bluetooth Low Energy (BLE) GATT API
When connected via Bluetooth Low Energy (Service UUID: `6e400001-b5a3-f393-e0a9-e50e24dcca9e`):
- **`CHAR_CONFIG` (`...0002`)**: Read/Write JSON string of `MotorControllerConfig`. Both the config and state characteristics are refreshed with current motor data on every GATT read event and synchronized when `CHAR_PAUSED` is written, so clients always get up-to-date values.
- **`CHAR_STATE` (`...0003`)**: Read/Notify JSON string of `StateResponse`. Direct GATT reads return a **compact** state (excluding `update_history` and `position_history` arrays) to fit within the 512-byte GATT attribute value limit. Full state including history arrays is available via `subscribe-state` push notifications. State and config characteristics are pre-populated on BLE connection so reads work immediately without requiring a subscribe first.
- **`CHAR_PAUSED` (`...0004`)**: Write JSON string `{"paused": true, "paused_position": 0.0}` for immediate pause/resume control. Automatically synchronizes the `CHAR_CONFIG` attribute value.
- **`CHAR_PIN_CONFIG` (`...0005`)**: Read/Write JSON string of `PinConfiguration` (including `"ble_enabled": true`). Pre-populated on connection and dynamically refreshed on GATT reads.
- **`CHAR_RPC` (`...0006`)**: Write/Notify JSON-RPC 2.0 command strings (e.g., `ping`, `get-state`, `subscribe-state`, `unsubscribe-state`, `restart`, `get-network-config`, `set-network-config`). All notifications use the **chunked notification protocol** (see below).
- **`CHAR_NETWORK_CONFIG` (`...0007`)**: Read/Write JSON string of `NetworkConfiguration` (including mDNS hostname, DHCP status, and static IP settings). Pre-populated on connection and dynamically refreshed on GATT reads.

### BLE Chunked Notification Protocol
All BLE notifications on `CHAR_STATE`, `CHAR_RPC`, and `CHAR_CONFIG` use a multi-packet chunking protocol to safely transmit JSON payloads that may exceed the BLE ATT MTU (typically 244 bytes with a standard 247-byte MTU). Each notification carries a 2-byte header:

```
Byte 0: index   — 0-based chunk sequence number
Byte 1: total   — total number of chunks in this message
Bytes 2+:       — payload fragment
```

- **Single-chunk messages** (payload ≤ 240 bytes): `[0, 1, ...payload...]`
- **Multi-chunk messages**: `[0, N, ...frag0...]`, `[1, N, ...frag1...]`, ..., `[N-1, N, ...fragN-1...]`
- The receiver concatenates fragments in index order after all `total` chunks arrive.
- Chunk payload size is fixed at 240 bytes (`BLE_CHUNK_DATA`), keeping each notification at 242 bytes (well within the 244-byte ATT limit for 247 MTU).
- The Python client provides `BleChunkReassembler` in `scripts/ossm.py` for transparent reassembly.

### Serial UART CLI Commands
When interacting over USB serial (`115200` baud, `\r\n` terminated):
- `set-wifi-ssid <ssid>` / `set-wifi-password <password>`
- `set-hostname <hostname>` / `set-dhcp-enabled <true|false>`
- `set-static-ip <ip>` / `set-static-mask <mask>` / `set-static-gateway <gw>` / `set-static-dns <dns>`
- `set-pin-modbus-tx <pin>` / `set-pin-modbus-rx <pin>` / `set-pin-modbus-de-re <pin>`
- `set-modbus-timeout-ms <ms>` / `set-modbus-scan-delay-us <us>`
- `set-ble-enabled <true|false>`
- `set-motor-config <json_string>`
- `get-pin-configuration` / `get-network-config` / `get-motor-config` / `get-wifi-config`
- `reset` (Reboots microcontroller)

---

## 7. Critical Rules & Architectural Guidelines for AI Agents

1. **`no_std` Environment**:
   - This is a bare-metal `no_std` project using `alloc` (via `esp-alloc`). Do not use anything from `std`. Use `alloc::string::String`, `alloc::vec::Vec`, `alloc::format!`, etc. For floating-point math, use `libm` (e.g., `libm::floorf()` instead of `f32::floor()`). For time durations, use `embassy_time::Duration::from_micros()` instead of `Duration::from_secs_f32()`.
2. **Dual-Toolchain Build**:
   - ESP32-C6 (RISC-V) builds with the standard Rust nightly toolchain. ESP32-S3 (Xtensa) requires the Espressif `esp` toolchain (`RUSTUP_TOOLCHAIN=esp`). The `esp` toolchain may have a different `embassy-executor` API surface (e.g., `spawn()` returns `()` instead of `Result`). Use `cfg(feature = "esp32s3")` / `cfg(feature = "esp32c6")` guards for target-specific code paths.
3. **Modbus UART Concurrency & Timing Architecture**:
   - Modbus communication happens over RS-485 via `esp_hal::uart::Uart`. Use async methods (`read_async`, `write_async`) within `embassy_time::with_timeout`. Ensure pin DE/RE (Driver Enable / Receiver Enable) timing is respected.
   - **RX Inter-Byte Timeout ($t_{1.5}$)**: Default is `1750 µs` (`compute_rx_inter_byte_timeout`) with hardware RX FIFO timeout clamped to `clamp(4, 127)` symbols (`init_uart_and_modbus`) so incoming multi-byte responses from slave stepper drivers (e.g., 57AIM30) are not aborted mid-frame due to interrupt jitter.
   - **Inter-Frame Quiet Interval ($t_{3.5}$)**: Default is `350 µs` at `115200` baud (`get_default_inter_frame_delay`), keeping total end-to-end Modbus RTU request-response cycle duration **< 3 ms** to support a **> 300 Hz** position update rate (`ups`).
4. **Multi-Target Compatibility**:
   - Always ensure changes compile for both RISC-V (`esp32c6`) and Xtensa (`esp32s3`) architectures. Avoid using architecture-specific assembly or registers unless gated by conditional compilation (`#[cfg(feature = "...")]`).
5. **BLE GATT via trouble-host**:
   - The BLE stack uses `trouble-host` with `bt-hci`. GATT services are defined declaratively using `#[gatt_service]` / `#[gatt_server]` derive macros. GATT attribute values are capped at 512 bytes by the `trouble-host` stack; for `StateResponse`, a compact serialization (excluding `update_history`, `position_history`, and redundant config fields like `wave_func`/`sharpness`/`spline_points`/`reversed`/`depth_top`) is used for GATT reads to stay within this limit. Full state data is available via `subscribe-state` push notifications. Both state and config characteristics are refreshed on every GATT `Read` event and pre-populated on BLE connection.
   - **Chunked Notification Protocol**: All BLE notifications (`CHAR_STATE`, `CHAR_RPC`, `CHAR_CONFIG`) use a `[index, total, ...payload]` multi-packet protocol to safely transmit JSON payloads exceeding the BLE ATT MTU limit (see §6 "BLE Chunked Notification Protocol"). Each chunk carries up to 240 bytes of payload data. Firmware helpers: `chunked_notify_state()`, `chunked_notify_rpc()`, `chunked_notify_config()`. Python reassembly: `BleChunkReassembler` class in `scripts/ossm.py`.
   - **BLE Connection Parameter Negotiation**: Connection parameter update requests (`GattConnectionEvent::RequestConnectionParams(req)`) are explicitly accepted (`req.accept(None, stack).await`) to prevent mobile OS / host connection drops.
   - **BLE Runner Resilience & Session Cooldown**: `runner.run()` is wrapped in a `loop` so transient controller errors do not permanently kill the BLE stack. On error, the runner waits 500ms and restarts. When a BLE session ends (`select` terminates), a 200ms cooldown (`Timer::after(200ms)`) is executed before restarting advertising so the hardware controller cleanly settles.
   - **Advertising Parameters**: Advertising interval is set to 250-500ms (vs. the default 160ms) to reduce radio contention with WiFi. Scan response data includes the 128-bit service UUID for faster device discovery without requiring a full GATT service discovery.
   - **Telemetry Push & Circuit Breaker**: The `push_telemetry` task properly yields with `Timer::after(100ms)` when not subscribed, avoiding busy-spin loops that would starve BLE event processing. Minimum push interval is clamped to 50ms. If `notify()` encounters `trouble_host::Error::Disconnected` / `ChannelClosed`, or 5 consecutive notification failures occur, the loop immediately terminates and cleanly resets the session.
6. **Keeping APIs Synchronized**:
   - When adding a new command or configuration property:
     - Update `src/motion.rs` (if state/config related).
     - Update `src/http_api.rs` (WebSocket JSON-RPC method handling).
     - Update `src/ble_api.rs` (BLE GATT characteristics and JSON-RPC handling).
     - Update `src/rpc.rs` (shared RPC dispatcher).
     - Update `src/command.rs` (Serial UART command handling).
     - Update `frontend/src/types.ts` and `frontend/src/api.ts` (if web interface interaction or state properties change).
     - Update automated test suites and backend tools in `scripts/ossm.py`, `scripts/test_websocket.py`, `scripts/test_frontend.py`, `scripts/test_ble.py`, and `scripts/setup_device.py`.
     - Update user documentation in `README.md` and `README.zh.md`.
7. **Frontend & Firmware Build Dependency**:
   - The embedded web server (`src/http_api.rs`) statically embeds `frontend/dist/index.html` at compile time via `include_bytes!`. Whenever you modify code in `frontend/`, you **must** rebuild the frontend (`npm run build` inside `frontend/` or via `./scripts/release.sh`) before compiling or flashing the Rust firmware, otherwise changes will not be included in the binary.
8. **Documentation Wiring Diagrams & Vector Graphics Workflow**:
   - **Never generate raw SVG diagrams by hand**, as manual coordinate calculations are error-prone and often lead to text or box collisions.
   - Always design visual diagrams as clean HTML/CSS templates in `assets/` (`wiring_diagram.html` and `wiring_diagram_zh.html`) using bright themes, structured CSS Grid layouts, and dynamic JavaScript midpoint calculations to ensure wire routing corridors remain clear of element borders.
   - When modifying diagram layouts, use headless Chromium via Playwright and `html-to-image` to export vector `.svg` files (`wiring_diagram.svg` and `wiring_diagram_zh.svg`).
9. **Embedded HTTP & WebSocket Connection Lifecycle**:
   - The HTTP server uses `edge-http` with 4 signal-gated acceptors, a socket queue, a single serialized REST dispatcher (`REST_GATE`), and up to 3 concurrent WebSocket session tasks. Non-WebSocket REST endpoints must include `"Connection": "close"` headers. WebSocket frames must use `FrameType::Text(false)` (final text frames) when sending via `edge-ws`.
10. **Motor Loop Telemetry & Step `dt` Clamping**:
    - The motor control loop executes `compute_cycle()` at high frequency and reports 1-second window loop statistics (`ups`, `min_dt_ms`, `max_dt_ms`, `avg_dt_ms`, `mdev_dt_ms`) in `StateResponse`. To prevent temporary CPU preemptions from causing sudden start/stop jumps on the servo motor, `compute_cycle()` clamps the effective step duration (`dt.min(0.012)`). The web UI automatically disconnects the live WebSocket stream after 3 seconds of inactivity when no components are actively subscribed.
11. **Task Scheduling & Core Allocation Strategy**:
    - On single-core targets (**ESP32-C6**), `motor_task` runs on an `esp_rtos::embassy::InterruptExecutor` at elevated priority, while network, BLE, and CLI tasks run on the main Embassy executor.
    - On dual-core targets (**ESP32-S3**), the motor loop runs on a dedicated `esp_rtos::embassy::Executor` on **Core 1**, ensuring zero preemption from WiFi/HTTP tasks running on Core 0.
12. **Linker & Build Script**:
    - `build.rs` must emit `cargo:rustc-link-arg=-Tlinkall.x` for the esp-hal linker script. The `esp-bootloader-esp-idf` crate provides the `esp_app_desc!()` macro required by `espflash` for the ESP-IDF bootloader compatibility layer.
13. **USB Serial Output & Logger Initialization**:
    - `esp-println` is configured with `jtag-serial` mode (not `auto`) to ensure reliable output via USB Serial/JTAG. The `auto` mode can misroute output to UART0 during early boot when no USB host is detected.
    - The logger **must** be initialized after `esp_rtos::start()` (which internally registers its own logger via the `log-04` feature) and followed by `log::set_max_level(log::LevelFilter::Info)` to re-assert the desired log level. Without this explicit level reset, `esp-rtos` overrides the max log level to `Off`, silencing all `log::info!`/`log::warn!`/`log::error!` output while `esp_println::println!()` still works.
    - Initialization order in `main`: `esp_hal::init()` → `esp_alloc` → `esp_rtos::start()` → `esp_println::logger::init_logger_from_env()` → `log::set_max_level(Info)`.


