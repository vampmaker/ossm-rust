# OSSM Rust - AI Agents Reference Guide

This document serves as an architectural overview, operational reference, and rulebook for AI assistants and coding agents working on the **OSSM Rust** repository.

---

## 1. Project Overview & Architecture

**OSSM Rust** is an embedded Rust firmware for ESP32-C6 and ESP32-S3 microcontrollers controlling an Open Source Sex Machine (OSSM) stepper motor (e.g., 57AIM30) over an RS-485 Modbus UART interface. It includes an embedded HTTP server, WebSocket JSON-RPC server, Vue-based web interface, and web flasher.

### Core Components & File Mapping
- **`src/main.rs`**: System initialization, WiFi connection, and main Embassy event loops. Spawns dedicated threads for network/HTTP serving and motor motion loops.
- **`src/http_api.rs`**: Implements the `edge-http` web server and the JSON-RPC 2.0 WebSocket command handler (`/ws/command`).
- **`src/command.rs`**: Handles serial (UART CLI) configuration commands (e.g., setting WiFi credentials, Modbus GPIOs, motor config).
- **`src/storage.rs`**: `StorageManager` interacting with Non-Volatile Storage (NVS) to persist WiFi credentials, Modbus pin configurations (`PinConfiguration`), and motor settings.
- **`src/motion.rs`**: Defines `MotorControllerConfig`, motion trajectory generation, and `StateResponse` data structures.
- **`src/motor_57aim30.rs` & `src/motor.rs`**: Modbus RTU communication and motor controller driver implementation.
- **`frontend/`**: Vue 3 + TypeScript single-page application embedded into the firmware for real-time motor control and configuration. Bundled into a single HTML file (`dist/index.html`) via `vite-plugin-singlefile`.
- **`flasher/`**: Standalone browser-based serial flasher and device configurator built with Vue 3, `esptool-js`, and xterm.js (`dist/index.html` -> `release/flasher.html`).
- **`scripts/`**: Python automation scripts for post-flashing device configuration and WebSocket testing.

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
*(Note: A backwards-compatible symlink `./release.sh -> scripts/release.sh` exists in the workspace root).*
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

### Automated Flashing & Configuration (`scripts/setup_device.py`)
To flash the release binary and configure the physical device over USB serial:
```bash
./scripts/setup_device.py --flash
```
- Automatically reads `DEVICE_MODEL`, `DEVICE_PORT`, `WIFI_SSID`, `WIFI_PASSWORD`, and Modbus GPIO pins from `.env`.
- Invokes `espflash write-bin` (if `--flash` is passed).
- Sends serial CLI commands (`set-wifi-ssid`, `set-pin-modbus-tx`, etc.), issues a soft `reset`, and monitors logs until WiFi connects.
- **Auto-Discovery**: Automatically captures the assigned IP address from WiFi logs and updates `DEVICE_IP="..."` in `.env`.

### Automated WebSocket Testing (`scripts/test_websocket.py`)
To run end-to-end verification of all WebSocket JSON-RPC endpoints against the live device:
```bash
./scripts/test_websocket.py
```
- Connects to `ws://<DEVICE_IP>/ws/command` (using `DEVICE_IP` from `.env`).
- Verifies `ping`, `status`, `get-state`, `set-config`, `subscribe-state` (with live push notification assertions), and `unsubscribe-state`.

---

## 5. Web Applications Architecture (`/frontend` & `/flasher`)

Both web applications in this repository are built using **Vue 3 (Composition API)**, **TypeScript**, **Tailwind CSS**, and **Vite**. A key architectural choice for both apps is the use of `vite-plugin-singlefile`, which bundles all HTML, JavaScript, and CSS into a single self-contained document without external asset dependencies.

### Embedded Web Interface (`/frontend`)
The frontend is a single-page application embedded directly into the ESP32 firmware to provide real-time motor control and monitoring over WiFi.
- **Single-File Embedding**: Compiles to `frontend/dist/index.html`. This file is statically included into the firmware binary at compile time via `include_bytes!("../frontend/dist/index.html")` in `src/http_api.rs` and served by `edge-http` directly from ROM without filesystem overhead.
- **Key Components (`frontend/src/components/`)**:
  - `MainControl.vue`: Manages core motion settings including BPM (speed), stroke depth, top/bottom depth anchoring, stroke reversal, and pause/resume functionality (supporting fixed-position and in-place pause modes).
  - `SplineEditor.vue`: Interactive curve editor allowing users to design custom periodic motion trajectories (`wave_func: 'spline'`) by manipulating spline control points.
  - `ModbusSettings.vue`: Configures RS-485 Modbus UART GPIO pins (`modbus_tx`, `modbus_rx`, `modbus_de_re`), communication timeout, scan delay, and device rebooting.
- **Communication & State (`frontend/src/api.ts` & `types.ts`)**:
  - Communicates with the firmware via HTTP REST/JSON endpoints (`/config`, `/state`, `/paused`, `/pin-config`, `/restart`).
  - Supports real-time state synchronization via WebSocket JSON-RPC 2.0 (`/ws/command`).
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
3. **`get-state`**: Returns current `StateResponse` object (motor position, speed, coordinates, and config).
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
8. **`waypoint`**: Streams motion waypoints into the buffer.

### Serial UART CLI Commands
When interacting over USB serial (`115200` baud, `\r\n` terminated):
- `set-wifi-ssid <ssid>` / `set-wifi-password <password>`
- `set-pin-modbus-tx <pin>` / `set-pin-modbus-rx <pin>` / `set-pin-modbus-de-re <pin>`
- `set-modbus-timeout-ms <ms>` / `set-modbus-scan-delay-us <us>`
- `set-motor-config <json_string>`
- `get-pin-configuration` / `get-motor-config` / `get-wifi-config`
- `reset` (Reboots microcontroller)

---

## 7. Critical Rules & Architectural Guidelines for AI Agents

1. **Stack Size Caution on ESP-IDF**:
   - The `edge-http` server future and its poll chain are ~29KB in memory. **Never** run the HTTP server directly on the main Embassy executor stack. Always spawn it on a dedicated `std::thread` with a heap-allocated stack of at least `65536` bytes (as seen in `src/main.rs`).
2. **Modbus UART Concurrency**:
   - Modbus communication happens over RS-485. Ensure pin DE/RE (Driver Enable / Receiver Enable) timing is respected if modifying UART drivers.
3. **Multi-Target Compatibility**:
   - Always ensure changes compile for both RISC-V (`esp32c6`) and Xtensa (`esp32s3`) architectures. Avoid using architecture-specific assembly or registers unless gated by conditional compilation (`#[cfg(target_arch = "...")]`).
4. **Keeping APIs Synchronized**:
   - When adding a new command or configuration property:
     - Update `src/motion.rs` (if state/config related).
     - Update `src/http_api.rs` (WebSocket JSON-RPC method handling).
     - Update `src/command.rs` (Serial UART command handling).
     - Update `frontend/src/types.ts` and `frontend/src/api.ts` (if web interface interaction or state properties change).
     - Update automated tests in `scripts/test_websocket.py` and `scripts/setup_device.py`.
     - Update user documentation in `README.md` and `README.zh.md`.
5. **Frontend & Firmware Build Dependency**:
   - The embedded web server (`src/http_api.rs`) statically embeds `frontend/dist/index.html` at compile time via `include_bytes!`. Whenever you modify code in `frontend/`, you **must** rebuild the frontend (`npm run build` inside `frontend/` or via `./scripts/release.sh`) before compiling or flashing the Rust firmware, otherwise changes will not be included in the binary.
