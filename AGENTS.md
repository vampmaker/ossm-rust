# OSSM Rust - AI Agents Reference Guide

Architectural overview, crate intents, and operational rules for agents working on this repository.

**OSSM Rust** drives an Open Source Sex Machine (OSSM) servo (typically 57AIM30 over RS-485 Modbus RTU) from two shells around one domain crate:

- **`ossm-core`** — portable domain (motion + RTU math + motion RPC/CLI)
- **`ossm-esp32`** — ESP32-C6 / ESP32-S3 device shell (`esp-hal` + Embassy)
- **`ossm-std`** — Linux desktop shell (tokio + axum)

If a crate cannot be stated in one paragraph of intent plus a short invariant list, it is mis-scoped and should be split or emptied — not given a vaguer name.

```
shell (esp32 | std) owns: clock, UART/serial, HTTP/BLE/CLI, flash/FS
        │  sync calls, no .await in core
        ▼
ossm-core  Engine::apply / tick(now) / snapshot
           CRC / find_modbus_response / aim30
```

---

## 1. Crate intents

A file belongs in a crate only if it serves that crate's intent. The tables below were checked against every `src/` file in the three crates.

### 1.1 `ossm-core` — portable domain

**Intent:** Pure `no_std` + `alloc` OSSM domain. Motion state machine, telemetry schema, motor CLI/RPC catalog, and Modbus RTU *byte math*. Shells own the world and call in.

**Invariants**

- No I/O, `.await`, files, NVS, GPIO, UART, radio, or process control.
- No `std`, Embassy, tokio, or `esp-hal`. Caller supplies `Micros`.
- `Engine { motion }` only (`Engine::new(motor)`). `apply` returns `()` — persistence is a shell concern.
- Default `paused: true`. Sole config write path bumps `version`. Stale `version` → `CoreError::StaleVersion`.
- `tick` clamps step `dt` to **12 ms**.
- JSON-RPC (`RpcRequest`) covers motion methods. `RpcAction::{Subscribe, Unsubscribe, Restart}` are **sentinels** for the shell, not side effects inside core.
- CLI `paths` / `get` / `set` are **motor** only.
- Modbus: CRC, `find_modbus_response`, `classify_*`, `aim30` pack/unpack. No inject knobs, no UART timing.

**Files (all hold)**

| File | Role |
| --- | --- |
| `lib.rs` | `no_std` + `alloc` root; re-exports |
| `engine.rs` | Facade: `apply` / `tick` / `try_set_config` / `CycleOutput` |
| `command.rs` | `Command` wraps `MotionCommand` + pause/homing/telemetry pokes |
| `config.rs` | `MotorControllerConfig` only |
| `motion/` | Waveform, shaper, stream source, `MotorController` |
| `state.rs` | `StateResponse` schema (includes `modbus_stats` fields shells fill) |
| `time.rs` | `Micros` |
| `error.rs` | `StaleVersion`, `InvalidConfig` |
| `rpc.rs` / `rpc_types.rs` | Sync dispatcher + wire types (`RpcRequest`; `cmd` / `method` alias) |
| `paths.rs` | Motor CLI catalog |
| `modbus/mod.rs` | CRC / resync / classify |
| `modbus/aim30.rs` | 57AIM30 position + homing register literals |
| `engine_tests.rs` / `rpc_tests.rs` | Host unit tests (`#[cfg(test)]`) |

`log` is used only for motion diagnostics (`controller` / `source`). `SetModbusStats` / `SetMotorConnected` are domain telemetry updates, not hardware drivers.

**Do not put in core:** GPIO, WiFi, BLE, inject, `operating_mode`, chip/process reset, DMA buffers.

### 1.2 `ossm-esp32` — ESP32 device shell

**Intent:** The microcontroller. Owns hardware, RF, flash, and transports. One motor *or* RTU-relay task exclusively owns UART1. HTTP/BLE/CLI never hold `&mut Engine`.

**Invariants**

- Heap **96 KiB**. Snapshot publish is in-place `Arc` (`copy_from` / `make_mut`). Never `Arc::new` every motor cycle.
- Boot: `esp_hal::init` → heap → `esp_rtos::start` → `console::init_logger` → `console_task` → CLI / motor-or-relay / storage → WiFi/HTTP (`HTTP_READY`) → BLE.
- `servo` vs `rtu_relay` is exclusive (same UHCI/UART1).
- C6: motor on `InterruptExecutor`. S3: motor on Core 1. Observed `dt_max_ms` **< 4.5** under HTTP+WS.
- Pin/net types and NVS live in `storage.rs`. RAM inject lives in `modbus_rtu.rs`.
- Soft-reset only for explicit `restart` / CLI `reset`. BLE runner failure must recover in-process.
- NVS debounce ≥ 2 s; ignore pause-only diffs.

**Files (all hold)**

| File | Role |
| --- | --- |
| `main.rs` | Init, heap, task spawn, boot branch; `nvs_saver_task` (2 s motor persist debounce) |
| `context.rs` | `AppContext` handles (queue + snapshot + storage) — not one big mutex; `storage_task` is the sole flash I/O actor |
| `motion.rs` | Firmware SPSC aliases (`MotionCommand`, capacity 3) — not motion math |
| `motor.rs` | `Motor` trait |
| `motor_57aim30.rs` | UHCI GDMA master, homing, motor loop; owns `Engine` |
| `modbus_relay.rs` | RTU ↔ TCP `:502` + `/ws/modbus` |
| `modbus_rtu.rs` | Re-export core CRC/find/aim30; own inject atomics |
| `storage.rs` | NVS + `PinConfiguration` + `NetworkConfiguration` |
| `hw_paths.rs` | Firmware `pin.*` / `net.*` path helpers |
| `wifi.rs` | STA (`StackResources<20>`) |
| `http_api.rs` | `edge-http` REST/WS; pin/net/inject endpoints |
| `ble_api.rs` | `trouble-host` GATT |
| `rpc.rs` | Async adapter: enqueue to motor task; encode via core |
| `command.rs` | Serial CLI over console channels (does not own UART) |
| `console.rs` | Sole owner USB Serial/JTAG + UART0 + RTT; panic handler |
| `buffers.rs` | Scratchpad JSON |
| `error.rs` | Firmware errors |

Two RPC dispatchers is deliberate: core is sync (`&mut Engine`); firmware cannot await the motor task, so `rpc.rs` enqueues `MotionCommand`.

### 1.3 `ossm-std` — Linux desktop shell

**Intent:** The same `Engine` on a PC. Host USB-serial is the motor bus; `--bind` is the HTTP listen address. The process *is* the device.

**Invariants**

- CLI > env > defaults (`--serial` / `OSSM_SERIAL` / `DEVICE_PORT`, `--baud`, `--bind` default `127.0.0.1:8080`, `--config`, `--mock`).
- Persist `{ motor }` JSON (temp + fsync + rename). Legacy `pin`/`net` keys ignored.
- No BLE, GPIO, WiFi STA, `/pin-config`, `/network-config`, or `/modbus-inject`.
- Stdin REPL is motor + motion actions. `reset` exits the process; `quit` leaves REPL.
- PC USB-serial cannot use RTU t1.5/t3.5; RX lives in `modbus_rx.rs` (accumulate + CRC). Homing uses core `aim30` literals.
- One engine actor (`mpsc`); HTTP/REPL send messages — no shared `Mutex<Engine>`.

**Files (all hold)**

| File | Role |
| --- | --- |
| `main.rs` | Tokio runtime, spawn engine + HTTP + optional REPL |
| `args.rs` | CLI / env / defaults |
| `engine_task.rs` | Owns `Engine`; tick + homing + persist |
| `http.rs` | axum `/config` `/state` `/paused` `/restart` `/ws/command` |
| `cli.rs` | Stdin REPL (`ossm_core::paths`) |
| `persist.rs` | `{ motor }` atomic JSON |
| `serial.rs` | tokio-serial FC03/06/16 + homing |
| `modbus_rx.rs` | Host RX accumulator |

Scripts against desktop: `DEVICE_IP=127.0.0.1:8080`.

### 1.4 Other packages (not Rust crates)

- **`frontend/`** — Vue SPA gzip-embedded in firmware (`index.html.gz`).
- **`flasher/`** — browser esptool (`release/flasher.html`).
- **`motor-control/`** — YZ_AIM PC tool (`release/motor-control.html`); not embedded.
- **`scripts/`** — `uv` PEP 723 tools (`ossm.py`, `setup_device.py`, live tests).
- **`assets/`** — wiring HTML/SVG and vendor VB6 form reference.

**Audit:** every `src/` file in the three crates matches the crate intent. Core is *domain* (motion + RTU math + motion RPC/CLI), not “motion-only” — that is why CRC/`aim30` live here. A fourth crate would be ceremony, not clarity.

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

### Console live tests (`scripts/test_console.py`)
USB Serial/JTAG CLI, motor-loop `ups`, and probe-rs RTT against a live device (`DEVICE_IP` / `DEVICE_PORT` / `DEVICE_MODEL` in `.env`):
```bash
./scripts/test_console.py
./scripts/test_console.py --only ups,rtt
```
- **`ups`**: ACM closed; `ups` ≥ 300 and `dt_max_ms` < 4.5 after the first 1 s window.
- **`acm-open`**: open ACM with **DTR=0 / RTS=0 before `open()`**. Linux CDC may still pulse chip reset; the test **keeps the port open through boot** and then asserts the same `ups`/`dt_max` bounds plus CLI `get pin.modbus_tx`.
- **`rtt`**: `sudo probe-rs attach --chip <model> --no-catch-reset` (ELF from `cargo b-c6 --release`; **no** `--rtt-scan-memory` — that can Load-fault during homing). Assert RTT has firmware logs, no `PANIC`, and `ups` stays healthy.
- **`late-attach`**: ACM closed ~12 s, then CLI `set pin.ble_enabled true` expects `pin.ble_enabled set to` within 2 s.
- **`fairness`**: enables `modbus_debug` (reboot), `POST /modbus-inject` trailing/leading flood, `get inject` over a held-open ACM, `ups` ≥ 100; disables debug on exit.

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
10. **`get-network-config`**: Firmware only. Returns current `NetworkConfiguration` (mDNS hostname, DHCP, static IP). Not in `ossm-core::dispatch_rpc` / `ossm-std`.
11. **`set-network-config`**: Firmware only. Updates `NetworkConfiguration` in NVS (reboot required).

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
When interacting over USB serial (`115200` baud, `\r\n` terminated), configuration uses nmcli-style **`get <path>`** / **`set <path> <value>`**. Type **`paths`** on-device for the full catalog (also in README). Pin/net/inject paths are **firmware-only**; `ossm-std` stdin REPL is motor + motion actions.

**Sections (firmware):** `get pin` | `get net` | `get motor` (full JSON); scalars: `get pin.modbus_tx`, `get net.wifi_enabled`, `get motor.bpm`, etc.

**Pin paths:** `pin.modbus_tx` / `modbus_rx` / `modbus_de_re` (GPIO 0..48); `pin.modbus_timeout_ms` (0..1000, 0=default); `pin.modbus_rx_timeout_us` / `modbus_scan_delay_us` / `modbus_inter_frame_delay_us` (0..200000, 0=auto); `pin.ble_enabled` / `pin.modbus_debug` (true|false, debug needs reboot); `pin.operating_mode` (servo|rtu_relay, reboot).

**Net paths:** `net.wifi_enabled` / `net.dhcp_enabled` (true|false); `net.ssid` / `password` / `hostname` (string); `net.static_ip` / `static_mask` / `static_gateway` / `static_dns` (IPv4).

**Motor paths:** `motor.bpm` (>0); `motor.depth` (0.01..1); `motor.depth_top` / `reversed` / `paused` / `streaming` (true|false); `motor.wave_func` (sine|thrust|spline); `motor.sharpness` (0.01..0.99); `motor.paused_position` (0..1); `motor.spline_points` (space-separated floats); bulk `set motor {"bpm":36,...}` (read-only: `motor.version`).

**Inject (RAM, modbus_debug only):** `get inject`; `set inject <off|leading|trailing|both> <nbytes 0..64>`.

**ACK format:** sets emit `{path} set to {value}` on the **CLI** path (`write_line`) and as `log::info` (flasher waits for these substrings).

**Actions:** `reset`, `get-state`, `get-status`, `reset-timestamp`, `set-waypoints <json>`, `append-waypoints <json>`, `help`, `paths`.

---

## 7. Critical rules

Firmware (`ossm-esp32`) unless a subsection says otherwise. `ossm-core` has no I/O; `ossm-std` uses host serial + axum instead of these hardware rules.

### 7.1 Toolchain and `no_std`

- Firmware and `ossm-core` are `no_std` + `alloc`. Do not use `std`. Use `alloc::string::String`, `alloc::vec::Vec`, `alloc::format!`. Floating-point: `libm` (e.g. `libm::floorf()` not `f32::floor()`). Durations: `embassy_time::Duration::from_micros()`, not `from_secs_f32()`.
- `ossm-std` is the exception: it is `std` + tokio and must compile on **stable** (`cargo +stable test -p ossm-core`, `cargo +stable run -p ossm-std -- --help`).
- Both chips use the Espressif **`esp`** channel in `rust-toolchain.toml`. `esp` `embassy-executor` `spawn()` may return `()` instead of `Result`. Gate chip paths with `#[cfg(feature = "esp32c6")]` / `#[cfg(feature = "esp32s3")]`.
- Compile for **both** RISC-V C6 and Xtensa S3. No architecture-specific assembly or registers unless cfg-gated.

### 7.2 Motion, snapshots, and causal version

- Motor loop runs `compute_cycle()` at high frequency and reports 1-second window stats (`ups`, `dt_min_ms`, `dt_max_ms`, `dt_avg_ms`, `dt_mdev_ms`) in `StateResponse`. `compute_cycle()` clamps step duration (`dt.min(0.012)`) so CPU preemption does not jerk the servo.
- Publish telemetry with `AppContext::update_snapshot` (in-place `copy_from` / `Arc::make_mut`). **Never** `Arc::new(snapshot.clone())` every motor cycle — that OOMs the WiFi+BLE **96 KiB** heap. Allocate a new `Arc` **outside** the critical section only when the current snapshot is shared.
- Boot policy is **paused** (`MotorControllerConfig::default().paused = true`; post-homing `sync_to_position` also forces pause). Snapshot `config` updates only when `config_version` advances — after any logical config change call `set_config` / `commit_config` or non-blocking `try_enqueue_config`, **not** bare `self.config.field = ...`. Every update increments `version: u32`. The web frontend (`connectionStateMachine.ts`) discards stale background pushes. Skipping the bump leaves `/config` and `/state` advertising stale versions while `MotionMode::Paused` holds the motor still.
- `try_enqueue_config` pre-increments `version`, rejects stale `config.version < snapshot.version` (`409` REST / RPC `-32001`), retries enqueue for ~250 ms on queue full, and returns the authoritative config blob.
- Firmware SPSC is `MotionCommand` capacity **3** (`crates/ossm-esp32/src/motion.rs`). The motor task dequeues into `Engine::apply` then `tick`. After homing: `HomingComplete` → `sync_to_position` parks via `set_config(paused=true)`, then `flush_snapshot` / `update_snapshot` before the control loop.
- Performance (required): on **ESP32-C6** under concurrent HTTP + WebSocket, observed `dt_max_ms` must stay **< 4.5 ms**. Scratchpad in hot paths must use Embassy mutex fast-path (non-yield expected path), not long critical-section serialization.
- The web UI disconnects the live WebSocket after 3 seconds of inactivity when no components are subscribed.

### 7.3 Task placement and cores

- **ESP32-C6:** `motor_task` (or `modbus_relay`) on `esp_rtos::embassy::InterruptExecutor` at elevated priority. Console, CLI, network, BLE on the main Embassy executor.
- **ESP32-S3:** motor/relay on **Core 1** via `esp_rtos::start_second_core(...)` → `run_motor_blocking(...)`. WiFi/HTTP on Core 0. Console + CLI stay on Core 0.
- Boot: `esp_hal::init()` → `esp_alloc` (**96 KiB** heap; S3 also places a ~**24 KiB** Core-1 motor/relay stack in RWDATA) → `esp_rtos::start()` → `console::init_logger()` → spawn `console_task` → CLI / motor-or-relay / storage → WiFi/HTTP (`HTTP_READY`) → then BLE.
- `servo` and `rtu_relay` are exclusive (same UART1/UHCI). Never spawn both.

### 7.4 Firmware Modbus (GDMA, timing, debug)

- RS-485 via ESP-HAL GDMA `Uhci` (`UHCI0` + `DMA_CH0`) wrapping `esp_hal::uart::Uart`. `UhciRx` + `UhciTx` with `esp_hal::dma_buffers!(256, 256)` stream bytes into memory without per-FIFO CPU interrupts.
- **RX inter-byte timeout ($t_{1.5}$):** default `750 µs` (`compute_rx_inter_byte_timeout`) — Modbus RTU spec for >19200 bps. This is the SOFTWARE `with_timeout()` guard per `read_async`. Whole-frame timeout default at 115200 is **10 ms**.
- **Hardware UART FIFO idle (`timeout_symbols`):** when `modbus_rx_timeout_us == 0`, default **`2` symbols** (~**174 µs** at 115200). For variable-length GDMA `UhciRx` frames this triggers `RX_TOUT` and completes the DMA transfer without splitting frames.
- **Re-arm race:** CPU two-phase reads (`uart_read_exactly(&resp[..3])` then `resp[3..len]`) could drop bytes in the re-arm gap. GDMA captures continuously between phases so high-rate polling (>330 Hz) does not lose frames.
- **Inter-frame quiet ($t_{3.5}$):** default `350 µs` at `115200` (`get_default_inter_frame_delay`). Subtract `hardware_silence_us` (~174 µs of `timeout_symbols`) from the wait so the bus is not silenced twice. End-to-end RTU cycle **< 3 ms** for **> 330 Hz** `ups`.
- **`modbus_debug`:** keep DMA at **`dma_buffers!(256, 256)`** and `pkt_thres` at expected frame length. Do **not** raise `pkt_thres` to capture capacity (false `empty` after cancel). Debug only: 5 ms software RX deadline, CRC-resync (`exact` / `long` / `long_resync` / `leading_junk` / `parse_fail` in `ossm-core` `find_modbus_response`), `MODBUS_DBG` logs (`skip`/`trim`; throttle `exact` when inject off). Raises USB log volume; `ups` modestly lower; leave off in production.
- RAM inject (firmware `modbus_rtu.rs`, not the bus): `set inject` or **`POST /modbus-inject`** `{"mode":"leading|trailing|both|off","nbytes":N}`. Host: `scripts/test_modbus_resync.py`. Live C6: `scripts/test_modbus_debug_device.py`. Console fairness: `scripts/test_console.py`.
- `ossm-std` cannot use t1.5/t3.5 on PC USB-serial; its RX is `modbus_rx.rs` (accumulate + CRC). Same `aim30` literals as firmware.

### 7.5 HTTP and WebSocket

- `edge-http`: 4 signal-gated acceptors, socket queue, up to 3 concurrent WebSocket sessions. Embassy net **`StackResources<20>`** (DHCP + HTTP + Modbus TCP `:502` + concurrent TCP).
- **`REST_GATE` serializes mutating POSTs only.** GETs, `/state`, and `/restart` must not hold the gate for the whole request (starves acceptors; port 80 looks dead under burst).
- JSON REST: `"Connection": "close"`. Gzip HTML `/` does not need it. WebSocket: `FrameType::Text(false)` (final text). End WS sessions with **`drop(socket)`**, not `close(Both).await` (smoltcp Load-fault after peer/WiFi teardown). REST may still `finish_connection` → `close(Both)`.
- Prefer **`GET/POST /modbus-inject`** over serial `set inject` when `MODBUS_DBG` floods USB. `rtu_relay` also serves `/ws/modbus` and Modbus TCP `:502`.
- `ossm-std` HTTP is axum: `/config` `/state` `/paused` `/restart` `/ws/command` only (no pin/net/inject). `restart` exits the process.

### 7.6 BLE (`trouble-host`)

- GATT service `6e400001-...`. Compact telemetry `format_compact_state()` / `set_compact_state()` on `CHAR_STATE` (≤256 B). Full `StateResponse` via JSON-RPC `get-state` on `CHAR_RPC`. Pre-populate on connect; update on **writes** / telemetry notify. GATT `Read` accepts the cached attribute only.
- **GATT accept timing:** never `.await` or mutate the attribute table between `GattEvent::Read`/`Write` and `accept()`/`send()`. Read path is accept-only. Awaiting (scratchpad lock) with an outstanding ATT request hangs the controller (BlueZ Unlikely Error) and can starve WiFi.
- Chunked notify on `CHAR_STATE` / `CHAR_RPC`: `[index, total, ...payload]` (see §6). Helpers: `chunked_notify_state()`, `chunked_notify_rpc()`. Python: `BleChunkReassembler` in `scripts/ossm.py`. Do not send unsolicited `CHAR_CONFIG` notifies on write without CCCD.
- Accept connection-parameter updates: `req.accept(None, stack).await`.
- **Runner resilience:** `runner.run()` selected against `serve_gatt`. On exit, recover **in-process** (~500 ms cooldown + `BT::steal()`). **Do not** `software_reset()` when the runner ends. Soft-reset only for explicit `restart` RPC. After disconnect, wait ~500 ms before re-advertising.
- `esp_radio::ble::Config`: `task_stack_size` ≥ **10 KiB** (default 4 KiB overflows: `Instruction access fault` at `mepc=0x80000100`). `max_connections=1`. **C6 (NimBLE):** raise `hci_high_buffer_count` / `acl_buf_count` as in `ble_api.rs`. **S3 (BTDM):** those fields are absent — cfg-gate them.
- Start BLE **after** WiFi/HTTP (`HTTP_READY`) so STA association and TCP listen settle first.
- Advertising interval 250–500 ms (default ~160 ms contends with WiFi). Primary ADV: Flags + CompleteLocalName + 128-bit service UUID; scan response also carries the UUID.
- `push_telemetry` yields `Timer::after(100ms)` when not subscribed. Active interval `25..=5000 ms`. `Disconnected` / `ChannelClosed` ends the session; transient errors backoff and retry.

### 7.7 Console, logging, panic

- **Runtime logger is `console`, not `esp-println`.** `esp-println` `jtag-serial` sets sticky `TIMED_OUT` when USB TX cannot drain — forever silencing later writes. Never route normal logs through `esp_println::println!` / its `log` feature. Never call `esp_println::logger::init_logger_from_env()`.
- One `console_task` owns USB Serial/JTAG and UART0 (C6 DevKit GPIO16 TX / GPIO17 RX; S3 GPIO43/44) and fans out to RTT (TX only). Per-sink TX **rings** (**256 B**, head/len — no O(n) `memmove`). **Separate USB rings** for CLI vs log.
- **Log byte ring + `LOG_LIVE`:** 4 KiB drop-newest (`enqueue_bytes`), not `OUT_CH`. Producers (including the C6 motor ISR) must **not** `Channel::try_receive`. Short CS covers one line (≤256 B). USB IN stalled → `LOG_LIVE=false` and `ChannelLogger` returns **before** `write!`. Full ring drops newest. CLI stays on **`CLI_OUT_CH`**.
- **USB TX is commit-then-wait**, not cancellation of `write_async`. Split RX/TX. TX: `write_byte_nb` only when `serial_in_ep_data_free`; consume the software ring as bytes enter the HW FIFO; `flush_tx_nb` (`wr_done`). Wait for empty with `select(flush, 500 µs)` while Idle/InFlight. Never rewrite committed bytes. `UsbTxState`: Idle / InFlight / Stalled.
- **Stalled** (no ACM reader): drop the USB **log** ring only; `LOG_LIVE=false`; keep the **CLI** ring; **do not** CS-dump the log byte ring; `select` USB RX vs a **2 ms** probe, then one `flush()` (skip UART/RTT drain). Re-emit `[console] ready` on recover. RX (`serial_out_recv_pkt`) is **armed even while TX is draining**. Yields elsewhere **`from_micros(100)`**, not 1 ms. UART0/RTT ~**10 ms** abandon per drain pass. Do **not** use esp-hal `write_async` (stuffs EP1 without a free check; not cancellation-safe).
- **Leave DTR/RTS chip reset enabled** (`SET_CONTROL_LINE_STATE`) for esptool / the browser flasher. Never write `USB_UART_CHIP_RST_DIS`. Host ACM close resetting the SoC is expected. **Do not** `software_reset()` on USB error, flush timeout, or stall — only explicit `restart` RPC / CLI `reset`.
- **`accept_out`:** USB is primary (return once all bytes are in the USB ring); UART0/RTT best-effort (stall: at most once × 100 µs then abandon that sink). Drain USB CLI ring before log ring in **`drain_sinks_progress`**. Batch log OUT (`TX_BATCH_MAX=3`, `LOOP_TX_BUDGET_MS=8`), **`CLI_OUT_CH`** (`CLI_BATCH_MAX=4`), capped log USB drain (`USB_DRAIN_MAX_PACKETS=4`), **512-byte** IN `try_send`, **`IDLE_YIELD_US=500`** when empty.
- **`write_line` / `write_bytes`:** CLI channel; `write_line` appends `\r\n` on the **last** chunk — do **not** stage into a small `heapless::String` (that truncated `get-state` / `get-status`). Set/get ACKs (`{path} set to {value}`) use `write_line` **and** `log::*` so the flasher still sees them when the log ring is full. Large replies serialize via `serialize_to_scratchpad` then `write_line`.
- **RX:** Always `poll_rx_nb` then `select` USB async read (even when TX has work). Idle: read vs `UART_POLL_US` (~**2 ms**). Stalled + empty CLI: read vs **2 ms** probe.
- Motor error logs in `motor_57aim30` rate-limited to **1 Hz**. 5 s loop stats stay. `MODBUS_DBG` unchanged when debug is on.
- **`esp-backtrace`:** `features = ["println"]` + chip gate. Link `esp-println` only for that build gate (`jtag-serial` + `critical-section`, **no** `log`). Do **not** enable `panic-handler` or `exception-handler`. `#[panic_handler]` in `console.rs` prints via `panic_write` + `esp_backtrace::arch::backtrace()`. RISC-V: `-C force-frame-pointers`.
- Register `console::init_logger()` **after** `esp_rtos::start()` so `log::set_max_level(Info)` overrides esp-rtos `log-04`.

### 7.8 Persistence / NVS

- Flash erase/write starves 2.4 GHz radio and can drop BLE/WiFi. Flash I/O runs only in `context::storage_task` via `StorageHandle`.
- `nvs_saver_task` debounces motor persist (≥ **2 s**) and ignores pause-only diffs (`persistent_motor_changed` excludes `paused` / `paused_position`). Do not add hot-path NVS writes from BLE/HTTP pause toggles.
- Pin/net/motor types for NVS live in firmware `storage.rs`. `ossm-std` persists `{ motor }` JSON (temp + fsync + rename) and ignores leftover `pin`/`net` keys.

### 7.9 Linker and build script

- `crates/ossm-esp32/build.rs` emits `cargo:rustc-link-arg=-Tlinkall.x`. **ESP32-S3:** `ld/esp32s3/linkall.x` + `ossm-rodata.x` merges `.flash.appdesc` padding into one DROM PROGBITS section (bootloader error: *multiple DROM segments*).
- `esp-bootloader-esp-idf` `esp_app_desc!()` is required by `espflash`. RISC-V: keep `-C force-frame-pointers`.
- Embed `frontend/dist/index.html` → `OUT_DIR/index.html.gz` → `include_bytes!` in `http_api.rs`.

### 7.10 Keep APIs synchronized

When adding a command or configuration property, update the crate that owns it, then every consumer:

- **Motion / causal version / RPC catalog:** `ossm-core` (`config`, `state`, `command`, `rpc`, `rpc_types`, `paths`, `engine`) plus tests.
- **Firmware SPSC / enqueue:** `crates/ossm-esp32/src/motion.rs`, `context.rs` (`try_enqueue_config`).
- **Firmware HTTP/WS:** `http_api.rs`. Pin/net/inject live here, not in core.
- **Firmware BLE:** `ble_api.rs`.
- **Firmware async RPC adapter:** `crates/ossm-esp32/src/rpc.rs` (enqueue; still handles get/set-network-config on storage).
- **Firmware CLI:** `command.rs` + `hw_paths.rs` (pin/net/inject). Motor paths stay in `ossm_core::paths`.
- **Desktop shell:** `ossm-std` `http.rs` / `cli.rs` / `engine_task.rs` (motor + motion RPC only).
- **Frontend:** `types.ts`, `api.ts`, `mapper.ts`, `macro.ts`, `connectionStateMachine.ts`.
- **57AIM30 PC tool:** `motor-control/src/lib/registers.ts`, `send-options.ts`, panels; rebuild `release/motor-control.html`.
- **Scripts:** `ossm.py`, `test_websocket.py`, `test_frontend.py`, `test_ble.py`, `test_modbus_debug_device.py`, `test_console.py`, `setup_device.py`.
- **Docs:** `README.md`, `README.zh.md`.

### 7.11 Frontend embed and wiring diagrams

- After `frontend/` changes, **`npm run build`** (or `./scripts/release.sh`) before compiling/flashing firmware — otherwise ROM still has the old `index.html.gz`.
- `motor-control/` is **not** embedded; rebuild and copy to `release/motor-control.html`.
- **Never** hand-author SVG wiring diagrams. Design HTML/CSS in `assets/wiring_diagram.html` and `wiring_diagram_zh.html` (bright theme, CSS Grid, JS midpoint routing). Export SVG with headless Chromium (Playwright + `html-to-image`).
