# OSSM Rust - AI Agents Reference Guide

This document serves as an architectural overview, operational reference, and rulebook for AI assistants and coding agents working on the **OSSM Rust** repository.

---

## 1. Project Overview & Architecture

**OSSM Rust** is an embedded Rust firmware for ESP32-C6 and ESP32-S3 microcontrollers controlling an Open Source Sex Machine (OSSM) stepper motor (e.g., 57AIM30) over an RS-485 Modbus UART interface. It includes an embedded HTTP server, WebSocket JSON-RPC server, Vue-based web interface, and web flasher.

### Core Components & File Mapping
- **`src/main.rs`**: System initialization, WiFi connection (with DHCP or static IP configuration), mDNS responder (`EspMdns`), and main Embassy event loops. Runs `motor_task`, `net_task`, and `nvs_saver_task` cooperatively on the main executor thread, and spawns dedicated OS threads for heavy blocking servers (`http_server` with 64KB heap stack, `ble_server`, `stdin_command`).
- **`src/http_api.rs`**: Implements the `edge-http` web server, REST endpoints (`/network-config`, `/pin-config`, etc.), and the JSON-RPC 2.0 WebSocket command handler (`/ws/command`).
- **`src/ble_api.rs`**: Implements the Bluetooth Low Energy (BLE) GATT server (`6e400001-...`) using `esp32-nimble`. Uses a buffered MPSC channel and asynchronous worker task (`ble_worker`) with a 15ms delay to safely execute GATT writes, configuration updates, and live telemetry push notifications without creating new OS threads or triggering ATT write collisions (0x0E).
- **`src/command.rs`**: Handles serial (UART CLI) configuration commands (e.g., setting WiFi credentials, mDNS hostname, static IP/DHCP mode, Modbus GPIOs, motor config, and BLE toggle).
- **`src/storage.rs`**: `StorageManager` interacting with Non-Volatile Storage (NVS) to persist WiFi credentials, Modbus pin configurations (`PinConfiguration`), network configurations (`NetworkConfiguration`), and motor settings.
- **`src/motion.rs`**: Defines `MotorControllerConfig`, motion trajectory generation, and `StateResponse` data structures (including physical bounds `pos_min`/`pos_max` and update rate telemetry `update_history`).
- **`src/motor_57aim30.rs` & `src/motor.rs`**: Modbus RTU communication and motor controller driver implementation.
- **`frontend/`**: Vue 3 + TypeScript single-page application embedded into the firmware for real-time motor control and configuration over WiFi. Bundled into a single HTML file (`dist/index.html`) via `vite-plugin-singlefile`.
- **`flasher/`**: Standalone browser-based serial flasher and device configurator built with Vue 3, `esptool-js`, and xterm.js (`dist/index.html` -> `release/flasher.html`).
- **`scripts/`**: Self-contained `uv` Python automation scripts. Includes `ossm.py` (unified dual-mode CLI and shared `DeviceBackend`), `setup_device.py`, `test_websocket.py`, and `test_ble.py`.
- **`assets/`**: Standalone HTML templates (`wiring_diagram.html`, `wiring_diagram_zh.html`) with dynamic JS auto-wiring and their exported vector graphics (`wiring_diagram.svg`, `wiring_diagram_zh.svg`) embedded in user documentation.

---

## 2. Build & Release Workflow

### Toolchain & Cargo Aliases
This repository uses custom target aliases defined in `.cargo/config.toml`. When building or checking code, always use the explicit aliases:
- **ESP32-C6 (`riscv32imac-esp-espidf`)**:
  ```bash
  cargo b-c6 --release
  ```
- **ESP32-S3 (`xtensa-esp32s3-espidf`)**:
  ```bash
  cargo b-s3 --release
  ```
- **Clippy & Linting**:
  ```bash
  cargo clippy --target riscv32imac-esp-espidf -- -D warnings
  ```

### Full Release Script
To build the frontend, flasher, and firmware for both ESP32-C6 and ESP32-S3, and generate merged `.bin` flash images:
```bash
./scripts/release.sh
```
Merged binaries are output to `release/ossm-esp32c6.bin` and `release/ossm-esp32s3.bin`.

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
A comprehensive command-line tool and shared library class (`DeviceBackend`) used across the automation suite.
- **Supported Modes**: `--mode wifi` (HTTP REST & WebSocket JSON-RPC), `--mode ble` (`bleak` GATT blobs & JSON-RPC notifications), `--mode serial` (USB UART CLI).
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
- Invokes `espflash write-bin` (if `--flash` is passed).
- Sends serial CLI commands (`set-wifi-ssid`, `set-pin-modbus-tx`, etc.) via `DeviceBackend`, issues a soft reset, and monitors logs until WiFi connects.
- **Auto-Discovery**: Automatically captures the assigned IP address from WiFi logs and updates `DEVICE_IP="..."` in `.env`.

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
- Automatically scans for and connects to the OSSM BLE device using `DeviceBackend`.
- Verifies `CHAR_PIN_CONFIG`, `CHAR_CONFIG`, `CHAR_STATE`, `CHAR_PAUSED`, and JSON-RPC commands over `CHAR_RPC`.
- Runs the motor and verifies live telemetry notifications over `CHAR_STATE`.

---

## 5. Web Applications Architecture (`/frontend` & `/flasher`)

Both web applications in this repository are built using **Vue 3 (Composition API)**, **TypeScript**, **Tailwind CSS**, and **Vite**. A key architectural choice for both apps is the use of `vite-plugin-singlefile`, which bundles all HTML, JavaScript, and CSS into a single self-contained document without external asset dependencies.

### Embedded Web Interface (`/frontend`)
The frontend is a single-page application embedded directly into the ESP32 firmware to provide real-time motor control and monitoring over WiFi.
- **Single-File Embedding**: Compiles to `frontend/dist/index.html`. This file is statically included into the firmware binary at compile time via `include_bytes!("../frontend/dist/index.html")` in `src/http_api.rs` and served by `edge-http` directly from ROM without filesystem overhead.
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
5. **`subscribe-state`**: Accepts `{"interval_ms": 300}` in `params`. Acknowledges with `"result": "subscribed"` and begins pushing periodic notifications from the server:
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
- **`CHAR_CONFIG` (`...0002`)**: Read/Write JSON string of `MotorControllerConfig`.
- **`CHAR_STATE` (`...0003`)**: Read/Notify JSON string of `StateResponse`. Supports live telemetry push notifications when subscribed via JSON-RPC.
- **`CHAR_PAUSED` (`...0004`)**: Write JSON string `{"paused": true, "paused_position": 0.0}` for immediate pause/resume control.
- **`CHAR_PIN_CONFIG` (`...0005`)**: Read/Write JSON string of `PinConfiguration` (including `"ble_enabled": true`).
- **`CHAR_RPC` (`...0006`)**: Write/Notify JSON-RPC 2.0 command strings (e.g., `ping`, `get-state`, `subscribe-state`, `unsubscribe-state`, `restart`, `get-network-config`, `set-network-config`). Note: Large state objects should be read directly from `CHAR_STATE` or `CHAR_CONFIG` blob endpoints since GATT notifications on `CHAR_RPC` are constrained by MTU packet size.
- **`CHAR_NETWORK_CONFIG` (`...0007`)**: Read/Write JSON string of `NetworkConfiguration` (including mDNS hostname, DHCP status, and static IP settings).

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

1. **Stack Size Caution on ESP-IDF**:
   - The `edge-http` server future and its poll chain are ~29KB in memory. **Never** run the HTTP server directly on the main Embassy executor stack. Always spawn it on a dedicated `std::thread` with a heap-allocated stack of at least `65536` bytes (as seen in `src/main.rs`).
2. **Modbus UART Concurrency**:
   - Modbus communication happens over RS-485. Ensure pin DE/RE (Driver Enable / Receiver Enable) timing is respected if modifying UART drivers.
3. **Multi-Target Compatibility**:
   - Always ensure changes compile for both RISC-V (`esp32c6`) and Xtensa (`esp32s3`) architectures. Avoid using architecture-specific assembly or registers unless gated by conditional compilation (`#[cfg(target_arch = "...")]`).
4. **BLE GATT Concurrency & Notifications**:
   - When implementing or modifying NimBLE GATT write callbacks (`on_write`), **never** synchronously invoke `set_value` or `notify` on the same characteristic within the callback execution context. Doing so causes ATT Protocol Error `0x0E` (Unlikely Error / Write Collision) on client devices. Always dispatch attribute updates and notifications via a buffered MPSC channel (`BleJob`) to an asynchronous background worker task (`ble_worker` in `src/ble_api.rs`) with a short delay (~15ms). This avoids spawning OS threads per callback and eliminates ATT write collisions. Additionally, keep in mind that single GATT notifications cannot exceed `MTU - 3` bytes; route large payload reads through dedicated GATT read blob characteristics (`CHAR_STATE`, `CHAR_CONFIG`).
5. **Keeping APIs Synchronized**:
   - When adding a new command or configuration property:
     - Update `src/motion.rs` (if state/config related).
     - Update `src/http_api.rs` (WebSocket JSON-RPC method handling).
     - Update `src/ble_api.rs` (BLE GATT characteristics and JSON-RPC handling).
     - Update `src/command.rs` (Serial UART command handling).
     - Update `frontend/src/types.ts` and `frontend/src/api.ts` (if web interface interaction or state properties change).
     - Update automated test suites and backend tools in `scripts/ossm.py`, `scripts/test_websocket.py`, `scripts/test_ble.py`, and `scripts/setup_device.py`.
     - Update user documentation in `README.md` and `README.zh.md`.
6. **Frontend & Firmware Build Dependency**:
   - The embedded web server (`src/http_api.rs`) statically embeds `frontend/dist/index.html` at compile time via `include_bytes!`. Whenever you modify code in `frontend/`, you **must** rebuild the frontend (`npm run build` inside `frontend/` or via `./scripts/release.sh`) before compiling or flashing the Rust firmware, otherwise changes will not be included in the binary.
7. **Documentation Wiring Diagrams & Vector Graphics Workflow**:
   - **Never generate raw SVG diagrams by hand**, as manual coordinate calculations are error-prone and often lead to text or box collisions.
   - Always design visual diagrams as clean HTML/CSS templates in `assets/` (`wiring_diagram.html` and `wiring_diagram_zh.html`) using bright themes, structured CSS Grid layouts, and dynamic JavaScript midpoint calculations to ensure wire routing corridors remain clear of element borders.
   - When modifying diagram layouts, use headless Chromium via Playwright and `html-to-image` to export vector `.svg` files (`wiring_diagram.svg` and `wiring_diagram_zh.svg`).
8. **Embedded HTTP & WebSocket Connection Lifecycle**:
   - Because `edge-http` runs with limited concurrent handler task slots (`HANDLER_TASKS`), non-WebSocket REST endpoints (`/config`, `/pin-config`, etc.) **must** include `"Connection": "close"` headers in their responses (`CONNECTION_CLOSE`). This ensures browsers do not hold keep-alive connections open across task slots, preventing WebSocket upgrade requests (`/ws/command`) from pending or timing out due to task slot exhaustion.
9. **Motor Loop Telemetry & Step `dt` Clamping**:
   - The motor control loop (`src/main.rs`) executes `compute_cycle()` at high frequency and reports 1-second window loop statistics (`ups`, `min_dt_ms`, `max_dt_ms`, `avg_dt_ms`, `mdev_dt_ms`) in `StateResponse`. To prevent temporary CPU preemptions (e.g., during network/JSON-RPC serialization) from causing sudden start/stop jumps on the stepper motor, `compute_cycle()` clamps the effective step duration (`dt.min(0.012)`). Furthermore, to avoid unnecessary background CPU/network load, the web UI automatically disconnects the live WebSocket stream after 3 seconds of inactivity when no components are actively subscribed.
10. **Task Scheduling, Priority Preemption & Core Allocation Strategy**:
    - To prevent WebSocket/HTTP JSON serialization from causing motor step timing jitter or skipped updates, the real-time motor loop runs at **FreeRTOS Priority 15** (well above normal Priority 5 application/network threads).
    - On single-core targets (**ESP32-C6**), `motor_task` runs cooperatively with `net_task` and `nvs_saver_task` on the main Embassy executor thread elevated to Priority 15, while heavy blocking servers (`http_server`, `ble_server`, `stdin_command`) run on dedicated lower-priority OS threads.
    - On dual-core targets (**ESP32-S3**), the motor loop (`run_motor`) runs on a dedicated OS thread pinned to **Core 1** (`Core::Core1`) at **Priority 15** via `ThreadSpawnConfiguration`, ensuring zero preemption from WiFi/HTTP tasks running on Core 0.


