# OSSM Rust - AI Agents Reference Guide

This document serves as an architectural overview, operational reference, and rulebook for AI assistants and coding agents working on the **OSSM Rust** repository.

---

## 1. Project Overview & Architecture

**OSSM Rust** is a bare-metal (`no_std`) Rust firmware for ESP32-C6 and ESP32-S3 microcontrollers controlling an Open Source Sex Machine (OSSM) servo motor with embedded driver (e.g., 57AIM30) over an RS-485 Modbus UART interface. Built on **`esp-hal`** + **`esp-rtos`** + **Embassy** async, it includes an **`edge-http`** HTTP/WebSocket server, `trouble-host` BLE GATT server, Vue-based web interface, and web flasher.

### Core Components & File Mapping
- **`crates/ossm-esp32/src/main.rs`**: System initialization via `esp_hal::init`, allocator/RTOS startup, WiFi startup, and task orchestration. Heap is **`96 KiB`** on both chips (`esp_alloc::heap_allocator!(size: 96 * 1024)`). ESP32-S3 also places a Core-1 motor/relay stack (~**24 KiB**) in RWDATA. After `esp_rtos::start()`, call `console::init_logger()` (channel-backed `log::Log`) so `log::set_max_level(Info)` overrides `esp-rtos`'s `log-04` default. Spawns `console_task` (sole owner of USB Serial/JTAG + UART0) before other I/O. Boot branches on `PinConfiguration.operating_mode`: **`servo`** runs motor on C6 `InterruptExecutor` / S3 Core 1; **`rtu_relay`** runs `modbus_relay` on the same core instead (exclusive UART1). Main executor tasks include console, CLI (channel-backed), `context::storage_task` (2s debounce; ignores pause-only diffs via `persistent_motor_changed`), WiFi net runner/connection tasks, and HTTP workers. Optional BLE is **deferred until after WiFi/HTTP are up** — `http_api::run_server` waits on `HTTP_READY` (bound + acceptors primed) before returning, then main spawns BLE (`Starting BLE after WiFi/HTTP initialization`) so association and TCP listen settle before BLE radio contention.
- **`crates/ossm-esp32/src/console.rs`**: Async `console_task` sole owner of USB Serial/JTAG + UART0 (+ RTT TX-only). Log OUT is a **4 KiB drop-newest byte ring** (`enqueue_bytes`; no `Channel::try_receive` from producers). IN is byte × 512 (`try_send`). **`LOG_LIVE`**: `ChannelLogger` returns before `write!` when USB IN is stalled. Per-sink TX **rings** (256 B, O(1) consume); **separate USB rings** for CLI vs log. USB TX is **commit-then-wait** (`write_byte_nb` only when the IN FIFO is free; consume the software ring immediately; never rewrite after `wr_done`). No ACM reader → **Stalled** (drop USB log ring, `LOG_LIVE=false`, keep CLI ring; **2 ms** IN probe, no CS dump of the log ring; re-emit `[console] ready` on recover). Do **not** set `USB_UART_CHIP_RST_DIS` (DTR/RTS chip reset is required for flashing) and do **not** `software_reset()` on USB stall/detach. USB RX is **split** from TX and armed even while draining. Batched log OUT (`TX_BATCH_MAX=3`, `LOOP_TX_BUDGET_MS=8`), priority **`CLI_OUT_CH`** (`CLI_BATCH_MAX=4`), capped log USB drain, **`IDLE_YIELD_US=500`** when queues empty. `write_line` does **not** truncate. Custom `#[panic_handler]` via `panic_write` + `esp_backtrace::arch::backtrace()` (needs `-C force-frame-pointers` on RISC-V).
- **`crates/ossm-esp32/src/modbus_relay.rs`**: When `PinConfiguration.operating_mode == "rtu_relay"`, owns UART1/UHCI and bridges Modbus RTU to TCP `:502` and binary WebSocket `/ws/modbus` (serialized bus access). Boot branches in `main.rs` spawn either this relay **or** the motor task — never both.
- **`crates/ossm-core/src/modbus/`**: Shared Modbus RTU helpers (`mod.rs`: CRC / `find_modbus_response` / inject) and **`aim30.rs`** (57AIM30 position pack/unpack). Used by `motor_57aim30`, `modbus_relay`, and `ossm-std`. Host mirror: `scripts/test_modbus_resync.py`.
- **`crates/ossm-esp32/src/http_api.rs`**: Implements the `edge-http` + `edge-nal-embassy` web server (4 acceptors, socket queue, up to 3 concurrent WebSocket sessions), REST endpoints (`/config`, `/state`, `/pin-config`, `/network-config`, `/paused`, `/restart`, `/modbus-inject`, etc.), and the JSON-RPC 2.0 WebSocket command handler (`/ws/command`) via `edge-ws`. **`GET/POST /modbus-inject`** (RAM-only; requires `modbus_debug`) sets capture fault-injection mode without serial CLI — prefer over `set inject` when `MODBUS_DBG` floods USB. In `rtu_relay` mode also serves `/ws/modbus` and Modbus TCP `:502`. `REST_GATE` serializes **mutating POSTs only** (not GETs/`/state`/`/restart`). Frontend HTML is gzip-compressed at build time and embedded via `include_bytes!(concat!(env!("OUT_DIR"), "/index.html.gz"))`. WebSocket sessions end with **`drop(socket)`** (do **not** call `close(Both).await` — that can Load-fault in smoltcp after a peer/WiFi teardown); REST still uses `finish_connection` → `close(Both)` when needed.
- **`crates/ossm-esp32/src/ble_api.rs`**: Implements the Bluetooth Low Energy (BLE) GATT server (`6e400001-...`) using `trouble-host` with `bt-hci`. Characteristics for config, state, paused control, pin config, RPC, and network config. Controller config uses a **10 KiB** task stack and `max_connections=1`. On **ESP32-C6** (NimBLE) also raise HCI/ACL buffer counts; on **ESP32-S3** (BTDM) those fields are absent — cfg-gate them. The BLE runner recovers **in-process** on exit (`BT::steal()` + 500ms cooldown) — **do not chip-reset** on runner failure (that kills WiFi/HTTP); only explicit `restart` RPC may soft-reset. GATT `Read` events accept cached attribute values only (no mutate/await before `accept`). Advertising uses relaxed intervals (250-500ms) and includes the service UUID in advertising / scan response data.
- **`crates/ossm-esp32/src/command.rs`**: Serial CLI via `embedded-cli` over console IN/OUT channels (does **not** own USB/UART). Echo and replies use `console::write_bytes` / `console::write_line` (priority **`CLI_OUT_CH`**). Set/get ACKs (`{path} set to {value}` / `{path}: {value}`) go through **`write_line`** as well as `log::*` so the flasher still sees them when the log queue is full. Large replies (`get-state`, `get-status`, pin/net/motor config) serialize via `serialize_to_scratchpad` then `console::write_line` (must stream full JSON — never a 256 B truncating buffer). Diagnostics use `log::*`.
- **`crates/ossm-esp32/src/storage.rs`**: `StorageManager` using `esp-nvs` (backed by `esp-storage`) to persist WiFi credentials, Modbus pin configurations (`PinConfiguration`), network configurations (`NetworkConfiguration`), and motor settings. Flash I/O runs only inside `context::storage_task` via `StorageHandle`.
- **`crates/ossm-esp32/src/wifi.rs`**: WiFi station setup using `esp-radio` and `embassy-net` with `smoltcp`. Supports DHCP and static IP. Embassy net uses **`StackResources<20>`** (headroom for DHCP + HTTP + Modbus TCP `:502` + concurrent TCP sockets).
- **`crates/ossm-core/src/motion/`** + **`crates/ossm-core/src/engine.rs`**: `MotorController::tick(now: Micros)` and `Engine::apply` / `try_set_config` (`apply` returns `()`; persistence is a shell concern). Firmware SPSC stays in `crates/ossm-esp32/src/motion.rs` (`COMMAND_QUEUE_SIZE = 3`); the motor task dequeues `MotionCommand` into `Engine::apply` then `tick`. **Default `paused: true`**. Snapshot config is version-gated. Sole config write path bumps `config_version`. After homing, `HomingComplete` → `sync_to_position` parks via `set_config(paused=true)`.
- **`crates/ossm-std/`**: Linux desktop shell (`cargo +stable run -p ossm-std`). Owns tokio-serial + axum REST/`/ws/command` around `Engine` (mpsc, not a shared mutex). **CLI > env > defaults** (`--serial`/`OSSM_SERIAL`/`DEVICE_PORT`, `--baud`/`DEVICE_BAUD`, `--bind`/`OSSM_BIND` default `127.0.0.1:8080`, `--config`/`OSSM_CONFIG`, `--mock`/`OSSM_MOCK`). Atomic `{pin,net,motor}` JSON via temp+fsync+rename. No BLE. Scripts: `DEVICE_IP=127.0.0.1:8080`.
- **`crates/ossm-core/src/rpc.rs`** + **`crates/ossm-core/src/paths.rs`**: Sync `dispatch_rpc(&mut Engine, request, out)` and CLI path catalog/`get`/`set`. ESP32 HTTP/BLE cannot hold `&mut Engine` (motor-task ownership); they use the async adapter.
- **`crates/ossm-esp32/src/motor_57aim30.rs` & `crates/ossm-esp32/src/motor.rs`**: `motor.rs` defines the `Motor` trait only. All Modbus RTU / UHCI GDMA / homing live in `motor_57aim30.rs`. The motor task exclusively owns `MotorController`; after `homing()`, calls `sync_to_position` then `flush_snapshot` / `update_snapshot` so telemetry matches the parked state before the control loop runs. When `PinConfiguration.modbus_debug` is true (NVS; reboot to apply), the master uses a **5 ms** RX deadline, keeps **`dma_buffers!(256, 256)`** + `pkt_thres=expected_len`, CRC-scans long/misaligned captures via [`crates/ossm-core/src/modbus/mod.rs`](crates/ossm-core/src/modbus/mod.rs) `find_modbus_response` (classes `exact` / `long` / `long_resync` / `leading_junk` / `parse_fail`), copies only the trimmed frame, and logs `MODBUS_DBG` with `skip`/`trim` (throttles `exact` when inject off) — increases USB log volume; steady-state `ups` modestly lower than production; leave off in normal use. Debug-only RAM fault injection: `set inject` / `GET|POST /modbus-inject` corrupts the software capture buffer (not the bus); verify with `scripts/test_modbus_resync.py` (host), `scripts/test_modbus_debug_device.py` (live C6, RTT), and `scripts/test_console_fairness.py` (USB serial CLI under flood).
- **`crates/ossm-esp32/src/rpc.rs`**: Async JSON-RPC adapter for HTTP/WebSocket and BLE. Encodes via `ossm_core::rpc::write_result` / `write_error`. Mutations use `try_enqueue_config` / `enqueue_motion` so `set-config` returns immediately with the authoritative configuration snapshot including the newly incremented `version`.
- **`crates/ossm-esp32/src/context.rs`**: `AppContext` is a `Copy` tip of domain handles — **not** one big shared mutex around all app state:
  - Motor task owns `MotorController`; HTTP/BLE/CLI send config/waypoints through `enqueue_motion` or non-blocking `try_enqueue_config` (`MotionCommand` SPSC, capacity 3). `try_enqueue_config` pre-increments `version`, rejects stale `config.version < snapshot.version` (`409` REST / RPC `-32001`), retries enqueue for ~250 ms on queue full, and returns the authoritative config blob for GATT/REST/RPC replies.
  - Telemetry is a published `Arc<StateResponse>` via `update_snapshot` / `load_snapshot` (in-place `copy_from` / `Arc::make_mut` when uniquely owned; allocate the new `Arc` **outside** the critical section when shared).
  - `StorageHandle` + `storage_task`: RAM caches for pin/net/motor; flash persist only in the storage actor.
- **`crates/ossm-esp32/src/error.rs`**: Firmware error types.
- **`frontend/`**: Vue 3 + TypeScript single-page application embedded into the firmware for real-time motor control and configuration over WiFi. Bundled into a single HTML file (`dist/index.html`) via `vite-plugin-singlefile`.
- **`flasher/`**: Standalone browser-based serial flasher and device configurator built with Vue 3, `esptool-js`, and xterm.js (`dist/index.html` -> `release/flasher.html`).
- **`scripts/`**: Self-contained `uv` Python automation scripts. Includes `ossm.py` (unified dual-mode CLI and shared `DeviceBackend`), `setup_device.py`, `test_websocket.py`, `test_frontend.py`, `test_ble.py`, `test_modbus_relay.py`, `test_modbus_tcp.py`, `test_modbus_resync.py` (host CRC/resync mirror), `test_modbus_debug_device.py` (live inject + `MODBUS_DBG` E2E), `test_console_fairness.py` (serial CLI under `MODBUS_DBG` TX flood), and `test_console_late_attach.py` (CLI ACK after ACM closed).
- **`motor-control/`**: Standalone YZ_AIM-style 57AIM30 Modbus PC tool (`dist/index.html` → `release/motor-control.html`). Vue 3 + Web Serial / Remote `/ws/modbus`; layout mimics `assets/57aim30_pc_control.frm` (Chinese groupbox captions). Protocol stays in `lib/modbus-rtu.ts` + `lib/registers.ts` — UI-only changes must not rewrite the Modbus layer.
- **`assets/`**: Standalone HTML templates (`wiring_diagram.html`, `wiring_diagram_zh.html`) with dynamic JS auto-wiring and their exported vector graphics (`wiring_diagram.svg`, `wiring_diagram_zh.svg`) embedded in user documentation. Also holds the vendor VB6 form reference `57aim30_pc_control.frm` for motor-control UI parity.

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
  cargo clippy -p ossm-esp32 --config=unstable.build-std=["core","alloc"] --target riscv32imac-unknown-none-elf --features esp32c6 --no-default-features -- -D warnings
  ```

### Build System Details
- Firmware is crate `ossm-esp32` (binary name `ossm-rust`). Host-safe crates `ossm-core` and `ossm-std` must compile on stable without `build-std` (`cargo +stable test -p ossm-core`, `cargo +stable run -p ossm-std -- --help`). Workspace `default-members` is `ossm-esp32` so bare `cargo build` still targets firmware.
- `build-std = ["core", "alloc"]` is passed on the `b-c6` / `b-s3` aliases and clippy (not globally — that would break `cargo +stable test -p ossm-core`).
- `crates/ossm-esp32/build.rs` emits `cargo:rustc-link-arg=-Tlinkall.x` for the esp-hal linker script. **ESP32-S3** uses `crates/ossm-esp32/ld/esp32s3/linkall.x` + `ossm-rodata.x` instead: merges `.flash.appdesc` alignment padding into one DROM PROGBITS section so espflash emits a single DROM segment (bootloader error: *multiple DROM segments*).
- `esp-bootloader-esp-idf` provides the ESP-IDF app descriptor macro required by `espflash`.
- Feature flags (`esp32c6` / `esp32s3`) gate chip-specific deps across `esp-hal`, `esp-rtos`, `esp-radio`, `esp-storage`, `esp-nvs`, `esp-alloc`, `esp-backtrace`, `esp-println`, `rtt-target`, and `esp-bootloader-esp-idf`.
- **Logging vs crash dump**: Runtime logs go through `console` (`log` + bounded OUT channel + USB/UART0/RTT). `esp-backtrace` **must** enable the `println` feature (build gate), which pulls in `esp-println` — keep `esp-println` on `jtag-serial` + `critical-section` only (**no** `log` feature, **never** call `esp_println::logger::init_logger_from_env()`). Do **not** enable `esp-backtrace` `panic-handler` or `exception-handler` (duplicate symbols with `console` / `esp-hal`). Panic frames: `console::panic_write` + `esp_backtrace::arch::backtrace()`. RISC-V needs `-C force-frame-pointers` in `.cargo/config.toml`.

### Full Release Script
To build the frontend, flasher, motor-control, and firmware for both ESP32-C6 and ESP32-S3, and generate merged `.bin` flash images:
```bash
./scripts/release.sh
```
Merged binaries are output to `release/ossm-esp32c6.bin` and `release/ossm-esp32s3.bin`. The script also copies standalone apps to `release/flasher.html` and `release/motor-control.html`.

**Important:** `cargo b-c6 --release` only updates `target/.../ossm-rust`. Flashing via `./scripts/setup_device.py --flash` uses the merged `release/ossm-*.bin` image. Rebuild `motor-control/` (`npm run build` → copy `dist/index.html` to `release/motor-control.html`, or via `./scripts/release.sh`) whenever that UI changes.
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
- **Commands**: `status` (display formatted status table), `set` (update parameters), `pause`/`resume`, `pins` (view/set GPIO + timing, `--modbus-debug`/`--no-modbus-debug`), `net` (view/configure network & mDNS settings), `wifi`, `restart`, `monitor` (live UART stream), `dash` (interactive live telemetry TUI dashboard), and `macro` (play or `--init` timestamped control-macro JSON files over WiFi/BLE/serial).
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
- Sends serial CLI commands (`set net.ssid`, `set pin.modbus_tx`, etc.) via `DeviceBackend`, issues a soft reset, and monitors logs until WiFi connects.
- **Auto-Discovery**: Automatically captures the assigned IP address from WiFi logs and updates `DEVICE_IP="..."` in `.env`.

### Frontend, BLE & Concurrent HTTP Testing (`scripts/test_frontend.py`)
Combined end-to-end verification of HTTP concurrency, BLE connectivity, and the embedded Vue UI against a live device:
```bash
./scripts/test_frontend.py
```
- Reads `DEVICE_IP` from `.env`.
- **Concurrent HTTP**: fires 4 parallel clients per round (matching the firmware acceptor queue), burst `/state` requests, and a threaded multi-endpoint burst. All must return HTTP 200. Mutating POSTs are serialized via `REST_GATE`; connections are still accepted and queued (not rejected).
- **BLE Integration**: connects to the OSSM BLE device, verifies GATT characteristic reads (pin config, motor config/state, network config), JSON-RPC over BLE (`ping`), config write/readback, and chunked state notification telemetry via `subscribe-state`. BLE tests run in a separate thread to avoid asyncio event loop conflicts with Playwright.
- **Playwright E2E**: loads the embedded UI, verifies WebSocket state streaming (~30 FPS), motor diagram toggle persistence, settings panel telemetry, main controls, and **MacroPlayer** editor/playback (`run_macro_editor_tests`: presets, Save/Save As/Rename/Delete, row tools, capture, raw JSON, gap seeking, brief Play/Stop). Supports `--mock` with in-process `mock_ossm_server.MockOssmServer` (no hardware) or live device via `DEVICE_IP`.
- **Motor-control E2E** (`test_motor_control`): loads `release/motor-control.html`, checks `#motor-control-connection` / `#mc-connection-type` / `#mc-baud` / `#mc-ws-url` (baud hidden in Remote WebSocket mode), and Chinese VB panel captions (`波形显示`, `开始读取`, …). Skips if the release HTML is missing. Does not automate Web Serial.

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

### Modbus debug resync & inject testing (`scripts/test_modbus_resync.py`, `scripts/test_modbus_debug_device.py`)
Host CRC/resync mirror (no hardware):
```bash
./scripts/test_modbus_resync.py
```
Live device E2E (requires `modbus_debug` enabled + reboot, motor connected):
```bash
./scripts/test_modbus_debug_device.py
```
- Captures logs via **probe-rs RTT** (`attach --rtt-scan-memory`) instead of USB serial — avoids `MODBUS_DBG` write-timeout floods on ACM.
- Uses **`POST /modbus-inject`** to set `leading`/`trailing` junk injection while motor runs briefly.
- Asserts `MODBUS_DBG ok=true class=long` (trailing) and `class=long_resync skip=3` (leading) in the RTT capture.
- Disables `modbus_debug` and restarts on exit. Requires `probe-rs` (may invoke `sudo` without udev rules).

### Console fairness testing (`scripts/test_console_fairness.py`)
Verifies serial CLI stays responsive when `modbus_debug` floods USB TX with `MODBUS_DBG` lines (motor running + fault injection):
```bash
./scripts/test_console_fairness.py
```
- Requires `modbus_debug` enabled + reboot and motor connected (`DEVICE_IP` / `DEVICE_PORT` in `.env`).
- Uses **`POST /modbus-inject`** (`trailing` / `leading`, `nbytes=3`) and unpauses the motor to generate the log flood.
- Sends `get inject` over USB serial (`write_timeout=2` s) and expects an ACK within retries.
- Asserts motor `ups` remains healthy (≥ 100) during the flood.
- Disables inject, pauses motor, turns off `modbus_debug`, and restarts on exit.
- Complements `test_modbus_debug_device.py` (RTT capture avoids ACM write stalls) by exercising the **USB serial CLI path** under the same diagnostic load.

### Console late-attach testing (`scripts/test_console_late_attach.py`)
Verifies the configurator ACK path after the chip has been up with nobody reading USB:
```bash
./scripts/test_console_late_attach.py
```
- Soft-resets (best-effort), leaves ACM **closed** for ~12 s, then opens **without** DTR so native USB-Serial-JTAG is not reset.
- Sends `set pin.ble_enabled true` and expects `pin.ble_enabled set to` within 2 s.
- Catches USB IN FIFO wedges from retrying `write_async` into a full endpoint, and set-ACKs dropped on the log queue.

---

## 5. Web Applications Architecture (`/frontend`, `/flasher`, `/motor-control`)

The web apps in this repository are built using **Vue 3 (Composition API)**, **TypeScript**, **Tailwind CSS**, and **Vite**. A key architectural choice is `vite-plugin-singlefile`, which bundles each app into a single self-contained HTML document without external asset dependencies.

### Embedded Web Interface (`/frontend`)
The frontend is a single-page application embedded directly into the ESP32 firmware to provide real-time motor control and monitoring over WiFi.
- **Single-File Embedding**: Compiles to `frontend/dist/index.html`. During build (`crates/ossm-esp32/build.rs`), it is compressed to `index.html.gz` in `OUT_DIR`. This compressed file is statically included into the firmware binary at compile time via `include_bytes!(concat!(env!("OUT_DIR"), "/index.html.gz"))` in `crates/ossm-esp32/src/http_api.rs` and served with `Content-Encoding: gzip` directly from ROM without filesystem overhead.
- **Key Components (`frontend/src/components/`)**:
  - `MotorPositionDiagram.vue`: Toggleable visual diagram displaying full stroke range (`0% - 100%`), hardware bounds (`pos_min`/`pos_max`), active stroke window (Left Limit & Right Limit markers), and real-time animated position indicator at 30 FPS.
  - `MainControl.vue`: Manages core motion settings including BPM (speed), stroke depth, top/bottom depth anchoring, stroke reversal, and pause/resume functionality (supporting fixed-position and in-place pause modes).
  - `SplineEditor.vue`: Interactive curve editor allowing users to design custom periodic motion trajectories (`wave_func: 'spline'`) by manipulating spline control points.
  - `MacroPlayer.vue`: Interactive macro sequence player with top-level **Active Macro** selection above management actions (`Save`, `Save As`, `Rename`, `Delete`), clickable instruction gaps (`#macro-gap-X`) for seeking, exact `0%`/`100%` edge locating, `100%` wrap-around playback, and bilingual translations (`en.json` / `zh.json`). Logic in `macro.ts` (`BUILTIN_PRESETS`, validation, playback engine, `localStorage` keys `ossm_macros_v1` / `ossm_macro_draft_v1`).
  - `ModbusSettings.vue`: Configures RS-485 Modbus UART GPIO pins (`modbus_tx`, `modbus_rx`, `modbus_de_re`), communication timeouts / inter-frame delay, `modbus_debug`, BLE enable, firmware restart, and displays live motor update frequency (`updates/sec` rate in Hz).
- **Communication & State (`frontend/src/api.ts`, `client.ts`, `connectionStateMachine.ts`, `mapper.ts`, & `types.ts`)**:
  - Communicates with the firmware via HTTP REST/JSON endpoints (`/config`, `/state`, `/paused`, `/pin-config`, `/network-config`, `/restart`) and optionally BLE (see `ble.ts` / `ConnectionMode`).
  - Implements a single-owner WebSocket client (`WsDataManager` in `client.ts`) for resilient real-time state synchronization via JSON-RPC 2.0 (`/ws/command`) at up to 30 FPS (`33ms`). Features automatic request/response promise routing (`pendingRequests`), application-level watchdog silent-failure detection, automatic reconnection loops (`triggerReconnect`) that re-subscribe active listeners without page refreshes, and automatic idle disconnection after 3 seconds when UI panels are closed.
  - **Causal Consistency & Data Mapping**: Tracks `authoritativeVersion` across mutations in `connectionStateMachine.ts`, rejecting stale remote config when `remoteConfig.version < authoritativeVersion`. `beginEnginePlayback` / `endEnginePlayback` (`MACRO` / `FUNSCRIPT`) and `beginLocalEdit` / `endLocalEdit` suppress background merges during virtual playback or optimistic edits; `scheduleOptimisticMutation` debounces slider POSTs. When diagram/settings panels are closed, falls back to **1 Hz** REST `/state` polling instead of WebSocket subscribe. `mapper.ts` (`wireToDomainConfig`, `domainToWireConfig`, `sanitizeMotorState`, `isConfigEqual`) preserves virtual `wave_func` (`macro` / `funscript`) in the UI while posting concrete firmware waveforms.
  - When opened from hostname `localhost`, the frontend API client targets `http://ossm.lan`; otherwise it uses the current page origin.

### Web Serial Flasher & Configurator (`/flasher`)
The flasher is a standalone, browser-based tool (`flasher/dist/index.html` -> `release/flasher.html`) that enables users to flash firmware binaries and configure microcontrollers over USB without installing local command-line tools.
- **Web Serial API & esptool-js**: Uses `@w3c/web-serial` and `esptool-js` (`ESPLoader`, `Transport`) to connect to ESP32-C6 / ESP32-S3 bootloaders directly from supported browsers (Chrome, Edge). Status-bar actions: **Connect** (ROM bootloader), **Flash Firmware**, **Monitor** / **Stop**, **Reset**, **Disconnect**.
- **Firmware Flashing**: Drag-and-drop or file selection of merged firmware images (e.g. `release/ossm-esp32c6.bin`) written at a configurable address (default `0x0`). After flash, `hard_reset` runs and status becomes `flash_done` while keeping the serial port so configuration can proceed without reconnecting.
- **Dual terminals**: **Console** (tool progress / ACKs) and **Serial** (device UART). Config send auto-attaches Serial monitor afterward.
- **Device Configuration**: Right-hand panel for WiFi enable/SSID/password, Modbus GPIO pins, Modbus timing overrides (`0` = firmware auto defaults), `modbus_debug`, and BLE enable. **Send Configuration** runs CLI commands then soft `reset`. Presets persist in browser `localStorage` under `ossm-flasher-device-config-v1`; **Reset to defaults** restores form defaults.

### 57AIM30 Modbus PC Control (`/motor-control`)
Standalone browser tool (`motor-control/dist/index.html` → `release/motor-control.html`) that mimics the vendor **YZ_AIM** VB6 form (`assets/57aim30_pc_control.frm`): Chinese groupbox captions, light-gray/Win32-ish chrome via shared `GroupBox.vue`, thin title bar (`YZ_AIM` / 57AIM30 PC Control) — not the embedded OSSM frontend.
- **Transport** (`lib/transport.ts`): `SerialModbusClient` (Web Serial) or `WebSocketModbusClient` (OSSM RTU relay at `/ws/modbus`). Keep both in the connection panel; baud is shown only for Local Serial.
- **Layout** (`App.vue`): desktop CSS grid — top `波形显示` | `modbus控制参数` (+ `参数保存` → write reg 20=1); bottom five cells `电机运行参数`, `驱动器设置参数`, `modbus读取`, `驱动器运行状态` (+ connect), `modbus发送`; strip `发送数据` (last TX echo from `onLog`); optional collapsed `通信日志`.
- **Waveform** (`WaveformChart.vue`): X = sample index **0..100** (clear/wrap like VB `plot_line.Index`); Y grid **±8**; plot **raw signed** holding values **`/32768`** into mid-scale (match `Pic1.Line`); channels current / PWM / speed / voltage (`DriveState.*Raw` from `registers.ts`).
- **Panels**: `ParamListPanel`, `TelemetryPanel`, `DriverSettingsPanel`, `ModbusReadPanel` (addr / 开始读取 / 地址扫描 / poll radios 20–200 ms), `StatusConnectPanel` (`#motor-control-connection`, `#mc-connection-type`, `#mc-baud`, `#mc-ws-url`), `ModbusSendPanel` (`send-options.ts`; raw hex under **高级**), `SendEchoStrip`, `CommLog`.
- **Stable E2E ids**: preserve `#motor-control-connection`, `#mc-connection-type`, `#mc-baud`, `#mc-ws-url` when restyling. Rebuild and update `release/motor-control.html` after UI changes.

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
4. **`set-config`**: Accepts partial or full `MotorControllerConfig` in `params` to dynamically update motion parameters. Returns the updated configuration object including the newly incremented causal version number (`version: u32`) which clients use to reject stale background state pushes. Rejects writes where `params.version` is older than the current snapshot (`-32001 Stale causal version`). `POST /config` returns **409 Conflict** for the same case.
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
- **`CHAR_CONFIG` (`...0002`)**: Read/Write JSON string of `MotorControllerConfig` (includes monotonic `version`). Pre-populated on connect; updated on **config/paused writes** with the **applied** config returned by `try_enqueue_config`. GATT **Read** returns the cached attribute only (no mid-Read refresh).
- **`CHAR_STATE` (`...0003`)**: Read/Notify compact telemetry JSON (`position`, `speed`, `ups`, `y`/`shaped_y`, plus minimal config fields). Attribute buffer is ≤ **256 B** so ATT Read works without Read Blob. Pre-populated on connect; push notifications use the compact format via the chunked protocol. Full `StateResponse` is via JSON-RPC `get-state` on `CHAR_RPC`.
- **`CHAR_PAUSED` (`...0004`)**: Write JSON string `{"paused": true, "position": 0.0}` for immediate pause/resume (`paused_position` accepted as an alias). Updates the cached `CHAR_CONFIG` attribute value; does **not** send unsolicited config notifications on write.
- **`CHAR_PIN_CONFIG` (`...0005`)**: Read/Write JSON string of `PinConfiguration` (including `"ble_enabled": true` and `"modbus_debug": false`). Pre-populated on connection and updated on write; GATT Read returns the cached value.
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
When interacting over USB serial (`115200` baud, `\r\n` terminated), configuration uses nmcli-style **`get <path>`** / **`set <path> <value>`**. Type **`paths`** on-device for the full catalog (also in README).

**Sections:** `get pin` | `get net` | `get motor` (full JSON); scalars: `get pin.modbus_tx`, `get net.wifi_enabled`, `get motor.bpm`, etc.

**Pin paths:** `pin.modbus_tx` / `modbus_rx` / `modbus_de_re` (GPIO 0..48); `pin.modbus_timeout_ms` (0..1000, 0=default); `pin.modbus_rx_timeout_us` / `modbus_scan_delay_us` / `modbus_inter_frame_delay_us` (0..200000, 0=auto); `pin.ble_enabled` / `pin.modbus_debug` (true|false, debug needs reboot); `pin.operating_mode` (servo|rtu_relay, reboot).

**Net paths:** `net.wifi_enabled` / `net.dhcp_enabled` (true|false); `net.ssid` / `password` / `hostname` (string); `net.static_ip` / `static_mask` / `static_gateway` / `static_dns` (IPv4).

**Motor paths:** `motor.bpm` (>0); `motor.depth` (0.01..1); `motor.depth_top` / `reversed` / `paused` / `streaming` (true|false); `motor.wave_func` (sine|thrust|spline); `motor.sharpness` (0.01..0.99); `motor.paused_position` (0..1); `motor.spline_points` (space-separated floats); bulk `set motor {"bpm":36,...}` (read-only: `motor.version`).

**Inject (RAM, modbus_debug only):** `get inject`; `set inject <off|leading|trailing|both> <nbytes 0..64>`.

**ACK format:** sets emit `{path} set to {value}` on the **CLI** path (`write_line`) and as `log::info` (flasher waits for these substrings).

**Actions:** `reset`, `get-state`, `get-status`, `reset-timestamp`, `set-waypoints <json>`, `append-waypoints <json>`, `help`, `paths`.

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
   - **`modbus_debug`**: Keep DMA at **`dma_buffers!(256, 256)`** and always set `pkt_thres` to the expected frame length (same as production). Do **not** raise `pkt_thres` to the capture capacity — that prevents length-EOF on short Modbus replies and yields false `empty` after cancel. Debug only widens the software RX deadline to **5 ms**, CRC-resyncs long captures (`long` / `long_resync`), and adds classify/`MODBUS_DBG` logging. Optional RAM inject: `set inject` or **`POST /modbus-inject`** `{"mode":"leading|trailing|both|off","nbytes":N}` (see `crates/ossm-core/src/modbus.rs`).
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
     - Update `crates/ossm-esp32/src/motion.rs` (if state/config related).
     - Update `crates/ossm-esp32/src/http_api.rs` (WebSocket JSON-RPC method handling).
     - Update `crates/ossm-esp32/src/ble_api.rs` (BLE GATT characteristics and JSON-RPC handling).
     - Update `crates/ossm-core/src/rpc.rs` (sync dispatcher) and `crates/ossm-esp32/src/rpc.rs` (async adapter).
     - Update `crates/ossm-esp32/src/command.rs` (Serial UART command handling).
     - Update `frontend/src/types.ts`, `frontend/src/api.ts`, `frontend/src/mapper.ts`, `frontend/src/macro.ts`, and `frontend/src/connectionStateMachine.ts` (if web interface interaction, causal versioning, or state properties change).
     - For 57AIM30 Modbus register map / RTU send types: update `motor-control/src/lib/registers.ts`, `send-options.ts`, and related panels; rebuild `release/motor-control.html`.
     - Update automated test suites and backend tools in `scripts/ossm.py`, `scripts/test_websocket.py`, `scripts/test_frontend.py` (includes `test_motor_control`), `scripts/test_ble.py`, `scripts/test_modbus_debug_device.py`, `scripts/test_console_fairness.py`, `scripts/test_console_late_attach.py`, and `scripts/setup_device.py`.
     - Update user documentation in `README.md` and `README.zh.md`.
7. **Frontend & Firmware Build Dependency**:
   - The embedded web server (`crates/ossm-esp32/src/http_api.rs`) embeds **`index.html.gz`** from `OUT_DIR` (produced by `build.rs` from `frontend/dist/index.html`). Whenever you modify code in `frontend/`, you **must** rebuild the frontend (`npm run build` inside `frontend/` or via `./scripts/release.sh`) before compiling or flashing the Rust firmware, otherwise changes will not be included in the binary.
   - `motor-control/` is **not** embedded in firmware; after UI changes rebuild it and refresh `release/motor-control.html` (see §2 / §5).
8. **Documentation Wiring Diagrams & Vector Graphics Workflow**:
   - **Never generate raw SVG diagrams by hand**, as manual coordinate calculations are error-prone and often lead to text or box collisions.
   - Always design visual diagrams as clean HTML/CSS templates in `assets/` (`wiring_diagram.html` and `wiring_diagram_zh.html`) using bright themes, structured CSS Grid layouts, and dynamic JavaScript midpoint calculations to ensure wire routing corridors remain clear of element borders.
   - When modifying diagram layouts, use headless Chromium via Playwright and `html-to-image` to export vector `.svg` files (`wiring_diagram.svg` and `wiring_diagram_zh.svg`).
9. **Embedded HTTP & WebSocket Connection Lifecycle**:
   - The HTTP server uses `edge-http` with 4 signal-gated acceptors, a socket queue, and up to 3 concurrent WebSocket session tasks. **`REST_GATE` serializes mutating POSTs only** — GETs, `/state`, and `/restart` must not hold the gate across the full request (holding it starves acceptors and makes port 80 look dead under burst load). JSON REST endpoints should include `"Connection": "close"`; the gzip HTML `/` response does not require it. WebSocket frames must use `FrameType::Text(false)` (final text frames) when sending via `edge-ws`. End WebSocket sessions with **`drop(socket)`**, not `close(Both).await`.
10. **Motor Loop Telemetry, Config Snapshot & Step `dt` Clamping**:
    - The motor control loop executes `compute_cycle()` at high frequency and reports 1-second window loop statistics (`ups`, `dt_min_ms`, `dt_max_ms`, `dt_avg_ms`, `dt_mdev_ms`) in `StateResponse`. To prevent temporary CPU preemptions from causing sudden start/stop jumps on the servo motor, `compute_cycle()` clamps the effective step duration (`dt.min(0.012)`). The web UI automatically disconnects the live WebSocket stream after 3 seconds of inactivity when no components are actively subscribed.
    - Publish telemetry with `AppContext::update_snapshot` (in-place `copy_from` / `Arc::make_mut`). **Never** allocate a fresh `Arc::new(snapshot.clone())` every motor cycle — that OOMs the WiFi+BLE **96 KiB** heap.
    - **Config vs mode consistency:** Boot policy is **paused** (`MotorControllerConfig::default().paused = true`; post-homing `sync_to_position` also forces pause). Snapshot `config` updates only when `config_version` advances — after any logical config change call `set_config` / `commit_config` or non-blocking `try_enqueue_config`, **not** bare `self.config.field = ...`. Every update automatically increments `version: u32` (monotonically increasing causal timestamp), which the web frontend (`connectionStateMachine.ts`) uses to discard stale background state notifications and ensure zero self-reverting UI mutations. Forgetting the version bump leaves `/config` and `/state` advertising stale versions and unpaused states while `MotionMode::Paused` holds the motor still.
    - **Performance target (required):** On **ESP32-C6** under concurrent HTTP + WebSocket load, the observed motor-loop `dt_max_ms` must remain **< 4.5 ms**. Scratchpad access in hot paths must use Embassy mutex fast-path semantics (non-yield/no-op expected path), not long critical-section serialization.
11. **Task Scheduling & Core Allocation Strategy**:
    - On single-core targets (**ESP32-C6**), `motor_task` runs on an `esp_rtos::embassy::InterruptExecutor` at elevated priority, while **console**, CLI, network, and BLE tasks run on the main Embassy executor.
    - On dual-core targets (**ESP32-S3**), the motor loop runs on **Core 1** via `esp_rtos::start_second_core(...)` calling `run_motor_blocking(...)`, isolating it from WiFi/HTTP work on Core 0. Console + CLI still run on Core 0.
12. **NVS during RF activity**:
    - Flash erase/write starves the shared 2.4 GHz radio and can drop BLE/WiFi sessions. `nvs_saver_task` debounces motor-config persist (≥ **2s**) and ignores ephemeral pause-only diffs (`persistent_motor_changed` excludes `paused` / `paused_position`). Do not add hot-path NVS writes from BLE/HTTP pause toggles.
13. **Linker & Build Script**:
    - `build.rs` emits `cargo:rustc-link-arg=-Tlinkall.x` for the esp-hal linker script (ESP32-S3: `ld/esp32s3/linkall.x` — see §2). The `esp-bootloader-esp-idf` crate provides the `esp_app_desc!()` macro required by `espflash` for the ESP-IDF bootloader compatibility layer. RISC-V targets must keep `-C force-frame-pointers` for `esp-backtrace` unwinds.
14. **Console I/O, Logging & Panic/Backtrace**:
    - **Runtime logger is `console`, not `esp-println`.** `esp-println`'s `jtag-serial` mode sets a sticky `TIMED_OUT` when the USB TX FIFO cannot drain without a host — forever silencing later writes. Never route normal logs through `esp_println::println!` / its `log` feature.
    - A single `console_task` owns USB Serial/JTAG and UART0 (C6 DevKit defaults: GPIO16 TX / GPIO17 RX; S3: GPIO43/44) and fans out to RTT (TX only). Each sink has a pending TX **ring** (**256 B**, head/len — no O(n) `memmove` on consume).
    - **Log byte ring + `LOG_LIVE`:** Logs go to a **4 KiB drop-newest** byte ring (`enqueue_bytes`), not `OUT_CH`. Producers (including the C6 motor ISR) must **not** `Channel::try_receive`. Short CS covers one line (≤256 B). When USB IN is stalled, **`LOG_LIVE=false`** and `ChannelLogger` returns **before** formatting. Full ring drops newest; never drop-oldest via `try_receive`. CLI stays on **`CLI_OUT_CH`**.
    - **USB TX is commit-then-wait**, not cancellation of `write_async`. `UsbSerialJtag` is **split** into RX/TX. TX uses `write_byte_nb` (checks `serial_in_ep_data_free`) and consumes the software ring **as bytes enter the HW FIFO**, then `flush_tx_nb` (`wr_done`). Wait for empty with `select(flush, 500 µs)` while Idle/InFlight. Never rewrite bytes already committed. `UsbTxState`: Idle / InFlight / Stalled. **Stalled** (no ACM reader): drop the USB **log** ring only; set `LOG_LIVE=false`; keep the **CLI** ring; **do not** CS-dump the log byte ring; do not treat TX as pending every loop — `select` USB RX vs a **2 ms** probe, then one `flush()` (skip UART/RTT drain). Re-emit `[console] ready` on recover. RX uses `serial_out_recv_pkt` and is **armed even while TX is draining**. Yields/backoffs elsewhere are **`from_micros(100)`**, not 1 ms. UART0/RTT still use a ~**10 ms** abandon per drain pass. Do **not** use esp-hal `write_async` here — it stuffs EP1 without a free check and is not cancellation-safe.
    - **Leave USB-Serial-JTAG DTR/RTS chip reset enabled** (`SET_CONTROL_LINE_STATE`) for esptool / the browser flasher. Never write `USB_UART_CHIP_RST_DIS` / `CONFIG_UPDATE` for that bit. Host close of ACM resetting the SoC is expected. Do **not** `software_reset()` on USB error, flush timeout, or stall — only explicit `restart` RPC / CLI `reset`.
    - **TX path:** producers push the log byte ring (`enqueue_bytes`, **drop-newest** when full, no-op when `!LOG_LIVE`) or priority CLI OUT (`write_bytes` / `write_line` → `CLI_OUT_CH`). The console task pulls bytes into sinks via **`accept_out`**, which treats **USB as primary** (return once all bytes are in the USB ring) and UART0/RTT as best-effort — stalls yield at most **once × 100 µs** then abandon that sink for the pass so a missing UART host cannot serialize echo/logging. Drain USB CLI ring before log ring in **`drain_sinks_progress`**. Late ACM attach must show subsequent output (and `[console] ready`) when IN returns to Free. Batch log OUT (`TX_BATCH_MAX=3`, `LOOP_TX_BUDGET_MS=8`), **`CLI_OUT_CH`** (`CLI_BATCH_MAX=4`), separate USB TX rings, capped log USB drain (`USB_DRAIN_MAX_PACKETS=4`), **512-byte** IN queue with `try_send`, **`IDLE_YIELD_US=500`** when queues empty — CLI stays responsive under `MODBUS_DBG` without starving motor `ups`.
    - **`write_line` / `write_bytes`:** enqueue via `CLI_OUT_CH` (priority over log OUT); `write_line` appends `\r\n` on the **last** chunk — **do not** stage into a small `heapless::String` (that silently truncated `get-state` / `get-status` ~1 KB replies). Log lines format into a 256 B staging string in `ChannelLogger` only when `LOG_LIVE`. Set/get ACKs use `write_line` so they are not dropped with the log ring.
    - **RX:** Always `poll_rx_nb` then `select` USB async read (even when TX has work). Idle: read vs `UART_POLL_US` (~**2 ms**). Stalled + empty CLI: read vs **2 ms** probe. CLI: `console::read_byte()` + `write_bytes` for echo; large replies via `write_line`.
    - **Motor error logs:** `Motor write/cycle error` in `motor_57aim30` is rate-limited to **1 Hz**. 5 s loop stats stay. `MODBUS_DBG` is unchanged when debug is on.
    - **`esp-backtrace`:** Keep dep with `features = ["println"]` + chip gate (`esp-backtrace/esp32c6|s3`). Link `esp-println` only to satisfy that build gate. Do **not** enable `panic-handler` or `exception-handler`. Our `#[panic_handler]` in `console.rs` prints the panic message and `esp_backtrace::arch::backtrace()` frames via `panic_write`.
    - Register `console::init_logger()` after `esp_rtos::start()` (which registers its own `log-04` logger) so `log::set_max_level(Info)` re-asserts the level.
    - Initialization order in `main`: `esp_hal::init()` → `esp_alloc` (**96 KiB** heap) → `esp_rtos::start()` → `console::init_logger()` → spawn `console_task` → CLI / motor / storage → WiFi/HTTP (`HTTP_READY`) → then BLE.

