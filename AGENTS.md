# OSSM Rust - AI Agents Reference Guide

This document serves as an architectural overview, operational reference, and rulebook for AI assistants and coding agents working on the **OSSM Rust** repository.

---

## 1. Project Overview & Architecture

**OSSM Rust** is a bare-metal (`no_std`) Rust firmware for ESP32-C6 and ESP32-S3 microcontrollers controlling an Open Source Sex Machine (OSSM) servo motor with embedded driver (e.g., 57AIM30) over an RS-485 Modbus UART interface. Built on **`esp-hal`** + **`esp-rtos`** + **Embassy** async, it includes an **`edge-http`** HTTP/WebSocket server, `trouble-host` BLE GATT server, Vue-based web interface, and web flasher.

### Core Components & File Mapping
- **`src/main.rs`**: System initialization via `esp_hal::init`, allocator/RTOS startup, WiFi startup, and task orchestration. Heap is **`96 KiB`** on both chips (`esp_alloc::heap_allocator!(size: 96 * 1024)`). ESP32-S3 also places a Core-1 motor stack (~**32 KiB**) in RWDATA. After `esp_rtos::start()`, call `console::init_logger()` (channel-backed `log::Log`) so `log::set_max_level(Info)` overrides `esp-rtos`'s `log-04` default. Spawns `console_task` (sole owner of USB Serial/JTAG + UART0) before other I/O. On ESP32-C6, the motor runs on an `InterruptExecutor` at elevated priority. On ESP32-S3, motor control runs on Core 1 via `esp_rtos::start_second_core(...)` and `run_motor_blocking(...)`. Main executor tasks include console, CLI (channel-backed), `context::storage_task` (2s debounce; ignores pause-only diffs via `persistent_motor_changed`), WiFi net runner/connection tasks, and HTTP workers. Optional BLE is **deferred until after WiFi/HTTP are up** — `http_api::run_server` waits on `HTTP_READY` (bound + acceptors primed) before returning, then main spawns BLE (`Starting BLE after WiFi/HTTP initialization`) so association and TCP listen settle before BLE radio contention.
- **`src/console.rs`**: Async `console_task` sole owner of USB Serial/JTAG + UART0 (+ RTT TX-only). Bounded OUT (`OutChunk` ≤192 B × 32 via `enqueue_bytes`) and IN (byte × 128) channels. Per-sink TX **rings** (256 B, O(1) consume). USB is `into_async()` with interrupt wakers; TX/RX waits are `select(..., Timer::from_micros(USB_WAIT_US))` so no host cannot hang. UART0/RTT stay Blocking + nb with **100 µs** yields; total drain abandon ~10 ms. RX: interrupt USB path (full FIFO drain) + short UART0 poll. `write_line` does **not** truncate — large CLI JSON is split across OUT chunks. `ChannelLogger` formats short log lines into a 256 B staging string. Custom `#[panic_handler]` via `panic_write` + `esp_backtrace::arch::backtrace()` (needs `-C force-frame-pointers` on RISC-V).
- **`src/http_api.rs`**: Implements the `edge-http` + `edge-nal-embassy` web server (4 acceptors, socket queue, up to 3 concurrent WebSocket sessions), REST endpoints (`/config`, `/state`, `/pin-config`, `/network-config`, `/paused`, `/restart`, etc.), and the JSON-RPC 2.0 WebSocket command handler (`/ws/command`) via `edge-ws`. `REST_GATE` serializes **mutating POSTs only** (not GETs/`/state`/`/restart`). Frontend HTML is gzip-compressed at build time and embedded via `include_bytes!(concat!(env!("OUT_DIR"), "/index.html.gz"))`. WebSocket sessions end with **`drop(socket)`** (do **not** call `close(Both).await` — that can Load-fault in smoltcp after a peer/WiFi teardown); REST still uses `finish_connection` → `close(Both)` when needed.
- **`src/ble_api.rs`**: Implements the Bluetooth Low Energy (BLE) GATT server (`6e400001-...`) using `trouble-host` with `bt-hci`. Characteristics for config, state, paused control, pin config, RPC, and network config. Controller config uses a **10 KiB** task stack and `max_connections=1`. On **ESP32-C6** (NimBLE) also raise HCI/ACL buffer counts; on **ESP32-S3** (BTDM) those fields are absent — cfg-gate them. The BLE runner recovers **in-process** on exit (`BT::steal()` + 500ms cooldown) — **do not chip-reset** on runner failure (that kills WiFi/HTTP); only explicit `restart` RPC may soft-reset. GATT `Read` events accept cached attribute values only (no mutate/await before `accept`). Advertising uses relaxed intervals (250-500ms) and includes the service UUID in advertising / scan response data.
- **`src/command.rs`**: Serial CLI via `embedded-cli` over console IN/OUT channels (does **not** own USB/UART). Echo uses `console::write_bytes` (passthrough). Large replies (`get-state`, `get-status`, pin/net/motor config) serialize via `serialize_to_scratchpad` then `console::write_line` (must stream full JSON — never a 256 B truncating buffer). Diagnostics use `log::*`.
- **`src/storage.rs`**: `StorageManager` using `esp-nvs` (backed by `esp-storage`) to persist WiFi credentials, Modbus pin configurations (`PinConfiguration`), network configurations (`NetworkConfiguration`), and motor settings. Flash I/O runs only inside `context::storage_task` via `StorageHandle`.
- **`src/wifi.rs`**: WiFi station setup using `esp-radio` and `embassy-net` with `smoltcp`. Supports DHCP and static IP. Embassy net uses **`StackResources<16>`** (headroom for DHCP + concurrent TCP sockets).
- **`src/motion.rs`**: Defines `MotorControllerConfig`, motion trajectory generation, and `StateResponse` (including physical bounds `pos_min`/`pos_max` and loop telemetry). Uses `libm` for `no_std` float math. Config / waypoints reach the motor via `MotionCommand` (`SetConfig`, waypoints). **Default `paused: true`** (start parked). Published snapshot config is **version-gated**: `refresh_snapshot` copies `self.config` only when `config_version` changes. **Sole post-init config write path is `commit_config` → always bumps `config_version`**; public `set_config` (and thus all HTTP/BLE/CLI `SetConfig`) must go through it. After Modbus homing, `sync_to_position` parks via `set_config(paused=true)` — **never** mutate `self.config.*` fields directly or the UI/API will show running while the motor is paused.
- **`src/motor_57aim30.rs` & `src/motor.rs`**: `motor.rs` defines the `Motor` trait only. All Modbus RTU / UHCI GDMA / homing live in `motor_57aim30.rs`. The motor task exclusively owns `MotorController`; after `homing()`, calls `sync_to_position` then `flush_snapshot` / `update_snapshot` so telemetry matches the parked state before the control loop runs.
- **`src/rpc.rs`**: Shared JSON-RPC 2.0 command dispatcher used by both HTTP/WebSocket and BLE code paths.
- **`src/context.rs`**: `AppContext` is a `Copy` tip of domain handles — **not** one big shared mutex around all app state:
  - Motor task owns `MotorController`; HTTP/BLE/CLI send config/waypoints through `enqueue_motion` (`MotionCommand` SPSC).
  - Telemetry is a published `Arc<StateResponse>` via `update_snapshot` / `load_snapshot` (in-place `copy_from` / `Arc::make_mut` when uniquely owned; allocate the new `Arc` **outside** the critical section when shared).
  - `StorageHandle` + `storage_task`: RAM caches for pin/net/motor; flash persist only in the storage actor.
- **`src/error.rs`**: Firmware error types.
- **`frontend/`**: Vue 3 + TypeScript single-page application embedded into the firmware for real-time motor control and configuration over WiFi. Bundled into a single HTML file (`dist/index.html`) via `vite-plugin-singlefile`.
- **`flasher/`**: Standalone browser-based serial flasher and device configurator built with Vue 3, `esptool-js`, and xterm.js (`dist/index.html` -> `release/flasher.html`).
- **`scripts/`**: Self-contained `uv` Python automation scripts. Includes `ossm.py` (unified dual-mode CLI and shared `DeviceBackend`), `setup_device.py`, `test_websocket.py`, `test_frontend.py`, and `test_ble.py`.
- **`assets/`**: Standalone HTML templates (`wiring_diagram.html`, `wiring_diagram_zh.html`) with dynamic JS auto-wiring and their exported vector graphics (`wiring_diagram.svg`, `wiring_diagram_zh.svg`) embedded in user documentation.

---

## 2. Build & Release Workflow

### Toolchain & Cargo Aliases
This is a `no_std` bare-metal project using `esp-hal`. Both chips use the Espressif **`esp`** channel pinned in `rust-toolchain.toml`. Custom target aliases are defined in `.cargo/config.toml`:
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
- Feature flags (`esp32c6` / `esp32s3`) gate chip-specific deps across `esp-hal`, `esp-rtos`, `esp-radio`, `esp-storage`, `esp-nvs`, `esp-alloc`, `esp-backtrace`, `esp-println`, `rtt-target`, and `esp-bootloader-esp-idf`.
- **Logging vs crash dump**: Runtime logs go through `console` (`log` + bounded OUT channel + USB/UART0/RTT). `esp-backtrace` **must** enable the `println` feature (build gate), which pulls in `esp-println` — keep `esp-println` on `jtag-serial` + `critical-section` only (**no** `log` feature, **never** call `esp_println::logger::init_logger_from_env()`). Do **not** enable `esp-backtrace` `panic-handler` or `exception-handler` (duplicate symbols with `console` / `esp-hal`). Panic frames: `console::panic_write` + `esp_backtrace::arch::backtrace()`. RISC-V needs `-C force-frame-pointers` in `.cargo/config.toml`.

### Full Release Script
To build the frontend, flasher, and firmware for both ESP32-C6 and ESP32-S3, and generate merged `.bin` flash images:
```bash
./scripts/release.sh
```
Merged binaries are output to `release/ossm-esp32c6.bin` and `release/ossm-esp32s3.bin`.

**Important:** `cargo b-c6 --release` only updates `target/.../ossm-rust`. Flashing via `./scripts/setup_device.py --flash` uses the merged `release/ossm-*.bin` image.
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
- **Pause payload**: `CHAR_PAUSED` / HTTP `/paused` use canonical JSON `{"paused": true, "position": 0.0}` (field name is `position`, not the motor-config field `paused_position`). Firmware also accepts `paused_position` as an alias.
- **BLE reconnect**: `DeviceBackend` re-scans and clears BlueZ device handles after failed connects / disconnect cool-downs so stale `dev_XX_...` paths do not poison the next attempt.
- **Usage Example**:
  ```bash
  ./scripts/ossm.py status -m wifi
  ./scripts/ossm.py dash -m ble
  ```

### Automated Flashing & Configuration (`scripts/setup_device.py`)
To flash the release binary and configure the physical device over USB serial using `DeviceBackend`.
To flash the device, always use `./scripts/setup_device.py --flash`.
** Do not use espflash directly for app flashes without the merged image. **
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
- **Concurrent HTTP**: fires 4 parallel clients per round (matching the firmware acceptor queue), burst `/state` requests, and a threaded multi-endpoint burst. All must return HTTP 200. Mutating POSTs are serialized via `REST_GATE`; connections are still accepted and queued (not rejected).
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
  - `ModbusSettings.vue`: Configures RS-485 Modbus UART GPIO pins (`modbus_tx`, `modbus_rx`, `modbus_de_re`), communication timeouts / inter-frame delay, BLE enable, firmware restart, and displays live motor update frequency (`updates/sec` rate in Hz).
- **Communication & State (`frontend/src/api.ts`, `client.ts`, & `types.ts`)**:
  - Communicates with the firmware via HTTP REST/JSON endpoints (`/config`, `/state`, `/paused`, `/pin-config`, `/network-config`, `/restart`) and optionally BLE (see `ble.ts` / `ConnectionMode`).
  - Implements a single-owner WebSocket client (`WsDataManager` in `client.ts`) for resilient real-time state synchronization via JSON-RPC 2.0 (`/ws/command`) at up to 30 FPS (`33ms`). Features automatic request/response promise routing (`pendingRequests`), application-level watchdog silent-failure detection, automatic reconnection loops (`triggerReconnect`) that re-subscribe active listeners without page refreshes, and automatic idle disconnection after 3 seconds when UI panels are closed.
  - When opened from hostname `localhost`, the frontend API client targets `http://ossm.lan`; otherwise it uses the current page origin.

### Web Serial Flasher & Configurator (`/flasher`)
The flasher is a standalone, browser-based tool (`flasher/dist/index.html` -> `release/flasher.html`) that enables users to flash firmware binaries and configure microcontrollers over USB without installing local command-line tools.
- **Web Serial API & esptool-js**: Uses `@w3c/web-serial` and `esptool-js` (`ESPLoader`, `Transport`) to connect to ESP32-C6 / ESP32-S3 bootloaders directly from supported browsers (Chrome, Edge). Status-bar actions: **Connect** (ROM bootloader), **Flash Firmware**, **Monitor** / **Stop**, **Reset**, **Disconnect**.
- **Firmware Flashing**: Drag-and-drop or file selection of merged firmware images (e.g. `release/ossm-esp32c6.bin`) written at a configurable address (default `0x0`). After flash, `hard_reset` runs and status becomes `flash_done` while keeping the serial port so configuration can proceed without reconnecting.
- **Dual terminals**: **Console** (tool progress / ACKs) and **Serial** (device UART). Config send auto-attaches Serial monitor afterward.
- **Device Configuration**: Right-hand panel for WiFi enable/SSID/password, Modbus GPIO pins, Modbus timing overrides (`0` = firmware auto defaults), and BLE enable. **Send Configuration** runs CLI commands then soft `reset`. Presets persist in browser `localStorage` under `ossm-flasher-device-config-v1`; **Reset to defaults** restores form defaults.

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
- **`CHAR_CONFIG` (`...0002`)**: Read/Write JSON string of `MotorControllerConfig`. Pre-populated on connect; updated on **config/paused writes**. GATT **Read** returns the cached attribute only (no mid-Read refresh).
- **`CHAR_STATE` (`...0003`)**: Read/Notify compact telemetry JSON (`position`, `speed`, `ups`, `y`/`shaped_y`, plus minimal config fields). Attribute buffer is ≤ **256 B** so ATT Read works without Read Blob. Pre-populated on connect; push notifications use the compact format via the chunked protocol. Full `StateResponse` is via JSON-RPC `get-state` on `CHAR_RPC`.
- **`CHAR_PAUSED` (`...0004`)**: Write JSON string `{"paused": true, "position": 0.0}` for immediate pause/resume (`paused_position` accepted as an alias). Updates the cached `CHAR_CONFIG` attribute value; does **not** send unsolicited config notifications on write.
- **`CHAR_PIN_CONFIG` (`...0005`)**: Read/Write JSON string of `PinConfiguration` (including `"ble_enabled": true`). Pre-populated on connection and updated on write; GATT Read returns the cached value.
- **`CHAR_RPC` (`...0006`)**: Write/Notify JSON-RPC 2.0 command strings (e.g., `ping`, `get-state`, `subscribe-state`, `unsubscribe-state`, `restart`, `get-network-config`, `set-network-config`). All notifications use the **chunked notification protocol** (see below).
- **`CHAR_NETWORK_CONFIG` (`...0007`)**: Read/Write JSON string of `NetworkConfiguration` (including mDNS hostname, DHCP status, and static IP settings). Pre-populated on connection and updated on write; GATT Read returns the cached value.

### BLE Chunked Notification Protocol
BLE notifications on **`CHAR_STATE`** (telemetry) and **`CHAR_RPC`** (RPC replies) use a multi-packet chunking protocol to safely transmit JSON payloads that may exceed the BLE ATT MTU (typically 244 bytes with a standard 247-byte MTU). Each notification carries a 2-byte header:

```
Byte 0: index   — 0-based chunk sequence number
Byte 1: total   — total number of chunks in this message
Bytes 2+:       — payload fragment
```

- **Single-chunk messages** (payload ≤ 240 bytes): `[0, 1, ...payload...]`
- **Multi-chunk messages**: `[0, N, ...frag0...]`, `[1, N, ...frag1...]`, ..., `[N-1, N, ...fragN-1...]`
- The receiver concatenates fragments in index order after all `total` chunks arrive.
- Chunk payload size is fixed at 240 bytes (`BLE_CHUNK_DATA`), keeping each notification at 242 bytes (well within the 244-byte ATT limit for 247 MTU).
- Firmware helpers: `chunked_notify_state()`, `chunked_notify_rpc()`. The Python client provides `BleChunkReassembler` in `scripts/ossm.py` for transparent reassembly.

### Serial UART CLI Commands
When interacting over USB serial (`115200` baud, `\r\n` terminated):
- `set-wifi-ssid <ssid>` / `set-wifi-password <password>`
- `set-wifi-enabled <true|false>` / `get-wifi-enabled`
- `set-hostname <hostname>` / `set-dhcp-enabled <true|false>`
- `set-static-ip <ip>` / `set-static-mask <mask>` / `set-static-gateway <gw>` / `set-static-dns <dns>`
- `set-pin-modbus-tx <pin>` / `set-pin-modbus-rx <pin>` / `set-pin-modbus-de-re <pin>`
- `set-modbus-timeout-ms <ms>` / `get-modbus-timeout-ms`
- `set-modbus-rx-timeout-us <us>` / `get-modbus-rx-timeout-us`
- `set-modbus-scan-delay-us <us>` / `get-modbus-scan-delay-us`
- `set-modbus-inter-frame-delay-us <us>` / `get-modbus-inter-frame-delay-us`
- `set-ble-enabled <true|false>` / `get-ble-enabled`
- `set-motor-config <json_string>`
- `get-pin-configuration` / `get-network-config` / `get-motor-config` / `get-wifi-config`
- `get-state` / `get-status`
- `reset-timestamp` / `set-waypoints <json>` / `append-waypoints <json>`
- `reset` (Reboots microcontroller)

---

## 7. Critical Rules & Architectural Guidelines for AI Agents

1. **`no_std` Environment**:
   - This is a bare-metal `no_std` project using `alloc` (via `esp-alloc`). Do not use anything from `std`. Use `alloc::string::String`, `alloc::vec::Vec`, `alloc::format!`, etc. For floating-point math, use `libm` (e.g., `libm::floorf()` instead of `f32::floor()`). For time durations, use `embassy_time::Duration::from_micros()` instead of `Duration::from_secs_f32()`.
2. **Dual-Toolchain Build**:
   - Both chips use the Espressif **`esp`** channel pinned in `rust-toolchain.toml` (RISC-V C6 + Xtensa S3 targets). The `esp` toolchain may have a different `embassy-executor` API surface (e.g., `spawn()` returns `()` instead of `Result`). Use `cfg(feature = "esp32s3")` / `cfg(feature = "esp32c6")` guards for target-specific code paths.
3. **Modbus UART Concurrency, GDMA & Timing Architecture**:
   - Modbus communication happens over RS-485 using ESP-HAL General Purpose DMA (GDMA) via `Uhci` (`UHCI0` + `DMA_CH0`) wrapping `esp_hal::uart::Uart`. Using GDMA (`UhciRx` + `UhciTx` with circular DMA buffers via `esp_hal::dma_buffers!(256, 256)`) allows the hardware DMA controller to continuously capture incoming bytes directly into memory without relying on CPU interrupt responsiveness per FIFO threshold.
   - **RX Inter-Byte Timeout ($t_{1.5}$)**: Default is `750 µs` (`compute_rx_inter_byte_timeout`) — matching the Modbus RTU spec for >19200 bps. This is the SOFTWARE `with_timeout()` guard per `read_async` call. Whole-frame Modbus timeout default at 115200 is **10 ms**.
   - **Hardware UART FIFO Idle Timeout (`timeout_symbols`)**: When `modbus_rx_timeout_us == 0`, default is **`2` symbols** (~**174 µs** at 115200 baud). For variable-length frames read via GDMA `UhciRx`, this hardware threshold controls when the UART RX FIFO triggers `RX_TOUT` and flushes buffered bytes to complete the async DMA transfer without splitting frames.
   - **Elimination of the Re-Arm Race via GDMA**: Previously with CPU-driven UART reads, reading a Modbus frame in two phases (`uart_read_exactly(&resp[..3])` then `uart_read_exactly(&resp[3..len])`) created a re-arm gap where bytes arriving during the gap could miss FIFO idle interrupts. With GDMA enabled, incoming bytes are continuously streamed into DMA buffers regardless of CPU task scheduling between read phases, ensuring zero dropped bytes and 100% Modbus request success rates at high polling rates (>330 Hz).
   - **Inter-Frame Quiet Interval ($t_{3.5}$)**: Default is `350 µs` at `115200` baud (`get_default_inter_frame_delay`). Because `UhciRx::read` completes after observing `timeout_symbols` (~**174 µs** with the default of 2) of line silence, that duration (`hardware_silence_us`) is subtracted from the inter-frame delay wait so the bus is not kept silent twice, keeping total end-to-end Modbus RTU request-response cycle duration **< 3 ms** to support a **> 330 Hz** position update rate (`ups`).
4. **Multi-Target Compatibility**:
   - Always ensure changes compile for both RISC-V (`esp32c6`) and Xtensa (`esp32s3`) architectures. Avoid using architecture-specific assembly or registers unless gated by conditional compilation (`#[cfg(feature = "...")]`).
5. **BLE GATT via trouble-host**:
   - The BLE stack uses `trouble-host` with `bt-hci`. GATT services are defined declaratively using `#[gatt_service]` / `#[gatt_server]` derive macros. Compact telemetry from `format_compact_state()` / `set_compact_state()` feeds `CHAR_STATE` (≤256 B attribute). Full `StateResponse` is via JSON-RPC `get-state` on `CHAR_RPC`. Characteristics are pre-populated on connect and updated on **writes** / telemetry notify — GATT `Read` accepts the cached attribute only.
   - **GATT accept timing**: Never `.await` or mutate the attribute table between receiving a `GattEvent::Read`/`Write` and completing `accept()`/`send()`. Read path is accept-only. Awaiting (e.g. scratchpad lock) with an outstanding ATT request hangs the controller (BlueZ Unlikely Error) and can starve WiFi on the same executor.
   - **Chunked Notification Protocol**: Notifications on `CHAR_STATE` and `CHAR_RPC` use `[index, total, ...payload]` chunks (see §6). Helpers: `chunked_notify_state()`, `chunked_notify_rpc()`. Python reassembly: `BleChunkReassembler` in `scripts/ossm.py`. Do not send unsolicited `CHAR_CONFIG` notifies on write without CCCD (drops/disconnects hosts).
   - **BLE Connection Parameter Negotiation**: Connection parameter update requests (`GattConnectionEvent::RequestConnectionParams(req)`) are explicitly accepted (`req.accept(None, stack).await`) to prevent mobile OS / host connection drops.
   - **BLE Runner Resilience (no chip reset)**: `runner.run()` is selected against `serve_gatt`. On exit, recover **in-process** with ~500ms cooldown and `BT::steal()` — **do not** `software_reset()` when the runner ends (that tears down WiFi/HTTP). Soft-reset only for explicit `restart` RPC. After a session disconnect, wait ~500ms before re-advertising.
   - **Controller stack & buffers**: `esp_radio::ble::Config` must use `task_stack_size` ≥ **10 KiB** under WiFi coexistence. The default 4 KiB overflows (`Instruction access fault` at `mepc=0x80000100`) and hard-hangs the chip. Keep `max_connections=1`. On **ESP32-C6** (NimBLE), also raise `hci_high_buffer_count` / `acl_buf_count` as in `ble_api.rs`. On **ESP32-S3** (BTDM), those HCI/ACL fields are not part of `Config` — do not call them (cfg-gate).
   - **Boot order**: Start BLE **after** WiFi/HTTP initialization so STA association and TCP listen complete before BLE radio contention.
   - **Advertising Parameters**: Interval 250-500ms (vs. default ~160ms) to reduce WiFi contention. Primary ADV includes Flags + CompleteLocalName + 128-bit service UUID; scan response also carries the service UUID.
   - **Telemetry Push Behavior**: `push_telemetry` yields with `Timer::after(100ms)` when not subscribed. Active interval clamped to `25..=5000 ms`. `Disconnected` / `ChannelClosed` on notify ends the session; transient errors backoff and retry.
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
   - The embedded web server (`src/http_api.rs`) embeds **`index.html.gz`** from `OUT_DIR` (produced by `build.rs` from `frontend/dist/index.html`). Whenever you modify code in `frontend/`, you **must** rebuild the frontend (`npm run build` inside `frontend/` or via `./scripts/release.sh`) before compiling or flashing the Rust firmware, otherwise changes will not be included in the binary.
8. **Documentation Wiring Diagrams & Vector Graphics Workflow**:
   - **Never generate raw SVG diagrams by hand**, as manual coordinate calculations are error-prone and often lead to text or box collisions.
   - Always design visual diagrams as clean HTML/CSS templates in `assets/` (`wiring_diagram.html` and `wiring_diagram_zh.html`) using bright themes, structured CSS Grid layouts, and dynamic JavaScript midpoint calculations to ensure wire routing corridors remain clear of element borders.
   - When modifying diagram layouts, use headless Chromium via Playwright and `html-to-image` to export vector `.svg` files (`wiring_diagram.svg` and `wiring_diagram_zh.svg`).
9. **Embedded HTTP & WebSocket Connection Lifecycle**:
   - The HTTP server uses `edge-http` with 4 signal-gated acceptors, a socket queue, and up to 3 concurrent WebSocket session tasks. **`REST_GATE` serializes mutating POSTs only** — GETs, `/state`, and `/restart` must not hold the gate across the full request (holding it starves acceptors and makes port 80 look dead under burst load). JSON REST endpoints should include `"Connection": "close"`; the gzip HTML `/` response does not require it. WebSocket frames must use `FrameType::Text(false)` (final text frames) when sending via `edge-ws`. End WebSocket sessions with **`drop(socket)`**, not `close(Both).await`.
10. **Motor Loop Telemetry, Config Snapshot & Step `dt` Clamping**:
    - The motor control loop executes `compute_cycle()` at high frequency and reports 1-second window loop statistics (`ups`, `dt_min_ms`, `dt_max_ms`, `dt_avg_ms`, `dt_mdev_ms`) in `StateResponse`. To prevent temporary CPU preemptions from causing sudden start/stop jumps on the servo motor, `compute_cycle()` clamps the effective step duration (`dt.min(0.012)`). The web UI automatically disconnects the live WebSocket stream after 3 seconds of inactivity when no components are actively subscribed.
    - Publish telemetry with `AppContext::update_snapshot` (in-place `copy_from` / `Arc::make_mut`). **Never** allocate a fresh `Arc::new(snapshot.clone())` every motor cycle — that OOMs the WiFi+BLE **96 KiB** heap.
    - **Config vs mode consistency:** Boot policy is **paused** (`MotorControllerConfig::default().paused = true`; post-homing `sync_to_position` also forces pause). Snapshot `config` updates only when `config_version` advances — after any logical config change call `set_config` / `commit_config`, **not** bare `self.config.field = ...`. Forgetting the version bump leaves `/config` and `/state` advertising unpaused while `MotionMode::Paused` holds the motor still.
    - **Performance target (required):** On **ESP32-C6** under concurrent HTTP + WebSocket load, the observed motor-loop `dt_max_ms` must remain **< 4.5 ms**. Scratchpad access in hot paths must use Embassy mutex fast-path semantics (non-yield/no-op expected path), not long critical-section serialization.
11. **Task Scheduling & Core Allocation Strategy**:
    - On single-core targets (**ESP32-C6**), `motor_task` runs on an `esp_rtos::embassy::InterruptExecutor` at elevated priority, while **console**, CLI, network, and BLE tasks run on the main Embassy executor.
    - On dual-core targets (**ESP32-S3**), the motor loop runs on **Core 1** via `esp_rtos::start_second_core(...)` calling `run_motor_blocking(...)`, isolating it from WiFi/HTTP work on Core 0. Console + CLI still run on Core 0.
12. **NVS during RF activity**:
    - Flash erase/write starves the shared 2.4 GHz radio and can drop BLE/WiFi sessions. `nvs_saver_task` debounces motor-config persist (≥ **2s**) and ignores ephemeral pause-only diffs (`persistent_motor_changed` excludes `paused` / `paused_position`). Do not add hot-path NVS writes from BLE/HTTP pause toggles.
13. **Linker & Build Script**:
    - `build.rs` must emit `cargo:rustc-link-arg=-Tlinkall.x` for the esp-hal linker script. The `esp-bootloader-esp-idf` crate provides the `esp_app_desc!()` macro required by `espflash` for the ESP-IDF bootloader compatibility layer. RISC-V targets must keep `-C force-frame-pointers` for `esp-backtrace` unwinds.
14. **Console I/O, Logging & Panic/Backtrace**:
    - **Runtime logger is `console`, not `esp-println`.** `esp-println`'s `jtag-serial` mode sets a sticky `TIMED_OUT` when the USB TX FIFO cannot drain without a host — forever silencing later writes. Never route normal logs through `esp_println::println!` / its `log` feature.
    - A single `console_task` owns USB Serial/JTAG and UART0 (C6 DevKit defaults: GPIO16 TX / GPIO17 RX; S3: GPIO43/44) and fans out to RTT (TX only). Each sink has a pending TX **ring** (**256 B**, head/len — no O(n) `memmove` on consume).
    - **USB is interrupt-driven async** (`UsbSerialJtag::into_async()`). TX waits on `serial_in_empty` / write future and RX on `serial_out_recv_pkt`, always wrapped in `select(..., Timer::after(Duration::from_micros(USB_WAIT_US)))` (~**500 µs** per EP). Yields/backoffs elsewhere are **`from_micros(100)`**, not 1 ms. A ~**10 ms** total drain budget abandons progress when no host is attached — **no sticky TIMED_OUT**, no unbounded await.
    - **TX path:** producers `try_send` to the OUT channel (`enqueue_bytes` / `write_bytes` / `write_line`). The console task pulls chunks into sinks via **`accept_out`**, which treats **USB as primary** (return once all bytes are in the USB ring) and UART0/RTT as best-effort — stalls yield at most **once × 100 µs** then abandon that sink for the pass so a missing UART host cannot serialize echo/logging. Drain USB first in **`drain_sinks`**. Late ACM attach must show subsequent output when Free returns.
    - **`write_line`:** enqueue payload + `\r\n` via `enqueue_bytes` only — **do not** stage into a small `heapless::String` (that silently truncated `get-state` / `get-status` ~1 KB replies). Log lines may still format into a 256 B staging string in `ChannelLogger`.
    - **RX:** USB path is interrupt-driven (`select` on OUT receive vs USB `read_async`); on wake, drain the full FIFO into IN. UART0 RX still needs a short poll arm (~**500 µs**). CLI: `console::read_byte()` + `write_bytes` for echo; large replies via `write_line`.
    - **`esp-backtrace`:** Keep dep with `features = ["println"]` + chip gate (`esp-backtrace/esp32c6|s3`). Link `esp-println` only to satisfy that build gate. Do **not** enable `panic-handler` or `exception-handler`. Our `#[panic_handler]` in `console.rs` prints the panic message and `esp_backtrace::arch::backtrace()` frames via `panic_write`.
    - Register `console::init_logger()` after `esp_rtos::start()` (which registers its own `log-04` logger) so `log::set_max_level(Info)` re-asserts the level.
    - Initialization order in `main`: `esp_hal::init()` → `esp_alloc` (**96 KiB** heap) → `esp_rtos::start()` → `console::init_logger()` → spawn `console_task` → CLI / motor / storage → WiFi/HTTP (`HTTP_READY`) → then BLE.

