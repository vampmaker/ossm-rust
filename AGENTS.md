# OSSM Rust - AI Agents Reference Guide

Architectural overview, crate intents, and operational rules for agents working on this repository.

**OSSM Rust** drives an Open Source Sex Machine (OSSM) servo (typically 57AIM30 over RS-485 Modbus RTU) from shells around one domain crate and a shared common leaf:

- **`ossm-core`** — portable domain (motion + RTU math + motion RPC/CLI)
- **`ossm-common`** — telemetry windows + BBR-style link PLL (`LoopStats`, `LinkStats`, `Pll`)
- **`ossm-esp32`** — ESP32-C6 / ESP32-S3 device shell (`esp-hal` + Embassy)
- **`ossm-std`** — Linux / Termux desktop shell (tokio + axum)
- **`ossm-wasm`** — browser shell (page thread + Web Serial / `/ws/rs485`)

If a crate cannot be stated in one paragraph of intent plus a short invariant list, it is mis-scoped and should be split or emptied — not given a vaguer name.

```
shell (esp32 | std | wasm) owns: clock, UART/serial, HTTP/BLE/CLI, flash/FS, loop/link stats windows
        │  sync calls, no .await in core
        ▼
ossm-core  Engine::apply / tick(now) / snapshot
           CRC / find_modbus_response / aim30
        ▲
ossm-common  LoopStatsWindow / LinkStatsWindow / Pll (no clock — caller supplies micros)
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
- `tick` clamps step `dt` to **12 ms** (motion only — no telemetry aggregation in core).
- JSON-RPC (`RpcRequest`) covers motion methods. `RpcAction::{Subscribe, Unsubscribe, Restart}` are **sentinels** for the shell, not side effects inside core.
- CLI `paths` / `get` / `set` are **motor** only.
- Modbus: CRC, `find_modbus_response`, `classify_*`, `aim30` pack/unpack. No inject knobs, no UART timing.

**Files (all hold)**

| File | Role |
| --- | --- |
| `lib.rs` | `no_std` + `alloc` root; re-exports |
| `engine.rs` | Facade: `apply` / `tick` / `try_set_config` / `CycleOutput` |
| `command.rs` | `Command` wraps `MotionCommand` + pause/homing/telemetry pokes |
| `config.rs` | `MotorControllerConfig` (`WaveFunc` enum, `SplinePoints` fixed buffer, `Copy`) |
| `motion/` | Waveform, shaper, stream source, `MotorController` |
| `state.rs` | `StateResponse` schema (`loop_stats` / `link_stats` from `ossm-common`; shells fill via `SetLoopStats` / `SetLinkStats`) |
| `time.rs` | `Micros` |
| `error.rs` | `StaleVersion`, `InvalidConfig` |
| `rpc.rs` / `rpc_types.rs` | Sync dispatcher + wire types (`RpcRequest`; `cmd` / `method` alias) |
| `paths.rs` | Motor CLI catalog |
| `modbus/mod.rs` | CRC / resync / classify |
| `modbus/aim30.rs` | 57AIM30 position + homing register literals |
| `engine_tests.rs` / `rpc_tests.rs` | Host unit tests (`#[cfg(test)]`) |

`log` is used only for motion diagnostics (`controller` / `source`). `SetLoopStats` / `SetLinkStats` / `SetMotorConnected` are domain telemetry snapshot writes, not hardware drivers.

**Do not put in core:** GPIO, WiFi, BLE, inject, `operating_mode`, chip/process reset, DMA buffers, loop/link stat windows.

### 1.2 `ossm-common` — telemetry + link pacing

**Intent:** Leaf `no_std` + `alloc` crate. Owns `LoopStatsWindow`, `LinkStatsWindow`, percentile math, serde wire structs (`LoopStats`, `LinkStats`, `TimingWindowStats`), and the BBR-style `Pll` (probe-freq / probe-phase). No clock — every entry point takes caller-supplied micros.

**Invariants**

- No workspace crate dependencies; host-testable on stable (`cargo +stable test -p ossm-common`).
- Not a workspace `default-member` (firmware default build stays `ossm-esp32`).
- Shells call `record` / `poll` then `engine.apply(SetLoopStats(...))` and `engine.apply(SetLinkStats(...))` before `tick`.
- `Pll` is sync. `next_wake` / `on_tx` / `on_ack` / `on_timeout` only. ProbeFreq is ACK-paced (max rate); ProbePhase snaps period to the link slot and hill-climbs TX delay to cut RTT. Re-probes on a 10 s interval, RTT inflation, or timeout share.
- Firmware UART stays hardware-paced; `ossm-std` and `ossm-wasm` own the PLL.

### 1.3 `ossm-esp32` — ESP32 device shell

**Intent:** The microcontroller. Owns hardware, RF, flash, and transports. One motor *or* RTU-relay task exclusively owns UART1. HTTP/BLE/CLI never hold `&mut Engine`.

**Invariants**

- Heap **96 KiB**. Snapshot publish is in-place `Arc` (`copy_from` / `make_mut`). Never `Arc::new` every motor cycle.
- Boot: `esp_hal::init` → heap → `esp_rtos::start` → `console::init_logger` → `console_task` → CLI / UART1 owner / storage → WiFi/HTTP (`HTTP_READY`) → BLE.
- `servo` vs `rtu_relay` vs `rs485` is exclusive (same UART1). Roles switch at runtime via a stoppable UART1 owner (no reboot). Live switch **into** `servo` always re-homes (`run_motor` → travel homing); no cached `pos_min`/`pos_max`.
- USB/`/ws/rs485` magic-exit `F0 0F 4F 53 53 4D 1B 71` (`rs485::EXIT_MAGIC`) leaves transceiver mode without WiFi: firmware ACKs `shell.operating_mode set to servo` and re-homes.
- C6: motor on `InterruptExecutor`. S3: motor on Core 1. Observed `dt_max_ms` **< 4.5** under HTTP+WS.
- Pin/net types and NVS live in `storage.rs`. RAM inject lives in `modbus_rtu.rs`.
- Soft-reset only for explicit `restart` / CLI `reset`. BLE runner failure must recover in-process.
- NVS debounce ≥ 2 s; ignore pause-only diffs.

**Files (all hold)**

| File | Role |
| --- | --- |
| `main.rs` | Init, heap, task spawn, boot branch; S3 Core-1 stack in `.dram2_uninit`; `nvs_saver_task` (2 s motor persist debounce) |
| `context.rs` | `AppContext` handles (queue + snapshot + storage) — not one big mutex; `storage_task` is the sole flash I/O actor |
| `motion.rs` | Firmware SPSC aliases (`MotionCommand`, capacity 3) — not motion math |
| `motor.rs` | `Motor` trait |
| `motor_57aim30.rs` | UHCI GDMA master, homing, stoppable motor loop |
| `uart_owner.rs` | UART1 supervisor: servo / rtu_relay / rs485 until stop, then re-init |
| `rs485.rs` | Raw UART1 + DE/RE pipe; USB mux + `/ws/rs485` TX/RX/CFG; `EXIT_MAGIC` |
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

### 1.4 `ossm-std` — Linux desktop / Termux shell

**Intent:** The same `Engine` on a PC or Termux phone. Host USB-serial (kernel tty or userspace USB-host) is the motor bus; `--bind` is the HTTP listen address. The process *is* the device.

**Invariants**

- CLI > env > defaults (`--serial` / `OSSM_SERIAL` / `DEVICE_PORT`, `--baud`, `--bind` default `127.0.0.1:8080`, `--mode servo|rtu-relay|flash|console`, `--relay-tcp` / `--relay-ws` / `--rs485-ws`, `--modbus-bind` default `127.0.0.1:502`, `--config`, `--mock`). `--mode flash` / `--mode console` own the serial port exclusively (no Engine / HTTP).
- Persist `{ motor, shell }` JSON (temp + fsync + rename). Legacy top-level `pin`/`net` keys ignored. CLI > env > file > defaults.
- No BLE, GPIO, WiFi STA, `/pin-config`, `/network-config`, `/shell-config`, or `/modbus-inject`. Motor REST CORS is `*`. Bundled webui-std on `GET /` (`--static-dir` override).
- Stdin REPL is motor + motion actions. `reset` exits the process; `quit` leaves REPL.
- PC USB-serial cannot use RTU t1.5/t3.5; RX is an unbounded stream in `modbus_rx.rs` (header length + CRC, leftover kept). Homing uses core `aim30` literals.
- `--serial` USB-RS485 requires **automatic direction control**. TX-loopback adapters and host-driven DE/RE are unsupported. The ESP32 `rs485` pipe already drives DE in firmware. ossm-std does not strip TX echoes (FC06 ACKs are a copy of the request).
- `--serial termux-usb:/dev/bus/usb/…` re-execs `termux-usb` (keep the prefix in argv after `TERMUX_USB_FD`). Bare `/dev/bus/usb/…` is direct usbfs (no `termux-usb`). Motor path deasserts DTR/RTS. Do **not** enable `android-usb-serial` `serialport-compat` (pulls `libudev`, breaks musl zigbuild). `--mode flash` keeps DTR/RTS controllable for the USB-Serial-JTAG reset sequence. Workspace `[patch.crates-io] nusb` (`vendor/nusb`) skips USBDEVFS mmap after `EPERM` (Termux); upstream nusb warns on every zero-copy allocate.
- Real `--serial` / relay bus always homes; `--mock` only uses a virtual `0..100` range. There is no `--no-homing`.
- Motor TX is paced by `ossm_common::Pll` on every `ossm-std` bus (kernel tty, USB-host, TCP `:502`, `/ws/modbus`, `/ws/rs485`). Homing still uses blocking `exchange_rtu` (200 ms). Streaming CRC deadline is 12 ms on kernel tty and 6 ms on USB-host.
- Motor loop runs on the dedicated `ossm-motor` std thread (`motor_thread.rs`), never on a tokio worker; wake via `precise_wait` (Windows high-resolution waitable timer + event, Unix Condvar + 10 us timer slack); HTTP/CLI reach it only through `EngineHandle::send`.
- One bus owner: Engine (`--mode servo`) **or** RTU relay server (`--mode rtu-relay` serves `:502` + `/ws/modbus`) **or** `--mode flash` / `--mode console`. Never two of these on the same bus.

**Files (all hold)**

| File | Role |
| --- | --- |
| `main.rs` | Tokio runtime; flash/console branch before engine; else spawn engine + HTTP + optional REPL |
| `build.rs` | Embed `web/apps/webui-std/dist/index.html` |
| `args.rs` | CLI / env / defaults |
| `engine_task.rs` | Async setup + homing, then hands `Engine` to the motor thread |
| `motor_thread.rs` | Dedicated `ossm-motor` std thread: PLL wake, tick, stream submit |
| `precise_wait.rs` | Platform waiter (Windows waitable timer + event; Unix Condvar) |
| `http.rs` | axum `/` (bundled webui-std) `/config` `/state` `/link-stats` `/paused` `/restart` `/ws/command`; CORS `*` |
| `cli.rs` | Stdin REPL (`ossm_core::paths`) |
| `persist.rs` | `{ motor }` atomic JSON |
| `serial.rs` | tokio-serial FC03/06/16 + homing; DTR/RTS off; USB-host writer |
| `usb_host.rs` | `/dev/bus/usb/…` Termux `termux-usb` / Linux usbfs + `android-usb-serial` |
| `flash_port.rs` | Raw serial trait for flash/console (tty + USB-host) |
| `esptool/` | ROM-loader SLIP protocol (no stub); compressed write + MD5 |
| `console_mode.rs` | UART CLI `--send` / `--until` / `--exit-rs485` / `--raw` |
| `bus.rs` | Pluggable RTU bus: serial, TCP 502, `/ws/modbus`, `/ws/rs485`; records link RTT in `exchange_rtu` |
| `link_stats.rs` | `SharedLinkStats` (`Arc<Mutex<LinkStatsWindow>>`) for HTTP `/link-stats` |
| `relay_server.rs` | `--mode rtu-relay` Modbus TCP + `/ws/modbus` + `GET /link-stats` |
| `modbus_rx.rs` | Host RX accumulator (stream parse) |

Scripts against desktop: `DEVICE_IP=127.0.0.1:8080`.

### 1.5 `ossm-wasm` — browser shell

**Intent:** The same `Engine` on the page thread. Owns clock (`performance.now`), Web Serial or `/ws/rs485` byte I/O, homing, and motor persist (IndexedDB). Vue talks only `OssmControl`; the UI does not see RTU frames.

**Invariants**

- Window / dedicated-page shell (not a PWA Service Worker). `requestPort()` stays on the page (user gesture); `SerialPort` is opened in-process.
- Vue never holds `&mut Engine` and never sees RTU frames (`WasmClient` in `webui-wasm` owns `OssmShell`).
- Real bus always homes; mock uses virtual `0..100`. DTR/RTS off before serial open.
- Build with `cargo +stable --target wasm32-unknown-unknown`. Not a workspace `default-member`.

**Files**

| File | Role |
| --- | --- |
| `lib.rs` | Host-testable re-exports; wasm_bindgen gated on `wasm32` |
| `shell.rs` | Engine owner: RPC, pause, tick, persist dirty flag, `Pll` |
| `rtu.rs` | AIM30 frame build/parse + homing via JS `exchange` |
| `wasm_api.rs` | `OssmShell` bindgen surface |

### 1.5 Other packages (not Rust crates)

- **`web/apps/webui-esp32`** — Vue SPA gzip-embedded in firmware (`index.html.gz`); WiFi/BLE/pin/net settings.
- **`web/apps/webui-std`** — motion-only SPA compile-time-embedded in `ossm-std` (`GET /`).
- **`web/apps/flasher`** — browser esptool (`release/flasher.html`).
- **`web/apps/motor-control`** — YZ_AIM PC tool (`release/motor-control.html`); not embedded.
- **`web/apps/webui-wasm`** — wasm harness (`release/ossm-wasm.html`); page-thread `WasmClient` owns Engine + RTU; not embedded.
- **`web/packages/shared`** — `@ossm/shared` Vue components (`MotionWorkbench`), types, mapper, macro, i18n/vite helpers.
- **`web/packages/client`** — `@ossm/client` WiFi/BLE control transports.
- **`scripts/`** — `uv` project tools (`ossm.py`, `setup_device.py`, live tests including `test_wasm.py`). Root [`pyproject.toml`](pyproject.toml) / [`uv.lock`](uv.lock); shebang `uv run` (not PEP 723 `--script`).
- **`assets/`** — wiring HTML/SVG and vendor VB6 form reference.

**Audit:** every `src/` file in the four crates matches the crate intent. Core is *domain* (motion + RTU math + motion RPC/CLI), not “motion-only” — that is why CRC/`aim30` live here.

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
- Firmware is crate `ossm-esp32` (binary name `ossm-rust`). Host-safe crates `ossm-common`, `ossm-core`, `ossm-std`, and `ossm-wasm` must compile on stable without `build-std` (`cargo +stable test -p ossm-common -p ossm-core`, `cargo +stable test -p ossm-wasm`, `cargo +stable run -p ossm-std -- --help`). Workspace `default-members` is `ossm-esp32` so bare `cargo build` still targets firmware. Workspace `[profile.release]` is host-sized (`debug = false`); `b-c6` / `b-s3` pass `--config=profile.release.debug=true` (and overflow-checks / opt-level 2) so only firmware keeps DWARF.
- `build-std = ["core", "alloc"]` is passed on the `b-c6` / `b-s3` aliases and clippy (not globally — that would break `cargo +stable test -p ossm-core`).
- `crates/ossm-esp32/build.rs` emits `cargo:rustc-link-arg=-Tlinkall.x` for the esp-hal linker script. **ESP32-S3** uses `crates/ossm-esp32/ld/esp32s3/linkall.x` + `ossm-rodata.x` instead: merges `.flash.appdesc` alignment padding into one DROM PROGBITS section so espflash emits a single DROM segment (bootloader error: *multiple DROM segments*).
- `esp-bootloader-esp-idf` provides the ESP-IDF app descriptor macro required by `espflash`.
- Feature flags (`esp32c6` / `esp32s3`) gate chip-specific deps across `esp-hal`, `esp-rtos`, `esp-radio`, `esp-storage`, `esp-nvs`, `esp-alloc`, `esp-backtrace`, `esp-println`, `rtt-target`, and `esp-bootloader-esp-idf`.
- **Logging vs crash dump**: Runtime logs go through `console` (`log` + bounded OUT channel + USB/UART0/RTT). `esp-backtrace` **must** enable the `println` feature (build gate), which pulls in `esp-println` — keep `esp-println` on `jtag-serial` + `critical-section` only (**no** `log` feature, **never** call `esp_println::logger::init_logger_from_env()`). Do **not** enable `esp-backtrace` `panic-handler` or `exception-handler` (duplicate symbols with `console` / `esp-hal`). Panic frames: `console::panic_write` + `esp_backtrace::arch::backtrace()`. RISC-V needs `-C force-frame-pointers` in `.cargo/config.toml`.

### Full Release Script
To build the three webuis, flasher, motor-control, wasm harness, `ossm-std` zigbuild binaries, and firmware for both ESP32-C6 and ESP32-S3:
```bash
uv run scripts/release.py
```
Merged binaries are output to `release/ossm-esp32c6.bin` and `release/ossm-esp32s3.bin`. Standalone apps: `release/flasher.html`, `release/motor-control.html`, `release/ossm-wasm.html`. `ossm-std` artifacts: `ossm-std-linux-x64` / `linux-arm64` (Termux) / `linux-armel` / `win-x64.exe` / `macos-arm64`. Optional `--skip-firmware` / `--skip-std` / `--skip-web`. Linux hosts without a cached SDK download `MACOSX_SDK_URL` into `.macos-sdk/` (or set `SDKROOT`). `--skip-macos` skips that target. `webui-std` is built **before** zigbuild so the HTML is inside each `ossm-std-*` binary. Wasm order is crate → wasm-bindgen → Vite HTML (not one hidden `npm run build:wasm`).

**Important:** `cargo b-c6 --release` only updates `target/.../ossm-rust`. Flashing via `./scripts/setup_device.py --flash` uses the merged `release/ossm-*.bin` image. Rebuild `motor-control/` (`npm run build` → copy `dist/index.html` to `release/motor-control.html`, or via `uv run scripts/release.py`) whenever that UI changes.
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

# macOS SDK tarball for zigbuild aarch64-apple-darwin on Linux
MACOSX_SDK_URL="https://github.com/phracker/MacOSX-SDKs/releases/download/11.3/MacOSX11.3.sdk.tar.xz"
```

---

## 4. Device Setup & Flashing (`/scripts`)

Scripts under `/scripts` use the root uv project (`pyproject.toml`). Run `uv sync` once, then `./scripts/ossm.py` / `uv run scripts/test_wasm.py`. Shebang is `#!/usr/bin/env -S uv run` (not `--script`).

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
Live device E2E (enables `modbus_debug` + reboot if needed, motor connected; restores off on exit):
```bash
./scripts/test_modbus_debug_device.py
```
- Captures logs via **probe-rs RTT** (`attach --no-catch-reset`, no `--rtt-scan-memory`) instead of USB serial — avoids `MODBUS_DBG` write-timeout floods on ACM.
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
- **`late-attach`**: ACM closed ~12 s, then CLI `set shell.ble_enabled true` expects `shell.ble_enabled set to` within 2 s.
- **`fairness`**: enables `modbus_debug` (reboot), `POST /modbus-inject` trailing/leading flood, `get inject` over a held-open ACM, `ups` ≥ 100; disables debug on exit.

### RS-485 transceiver live tests (`scripts/test_rs485_transceiver.py`)
Live-switch `operating_mode=rs485` **without reboot**, then exercise USB ACM and `/ws/rs485` via `ossm-std --mode rtu-relay`:
```bash
./scripts/test_rs485_transceiver.py
```
- HTTP `set pin.operating_mode rs485` (no restart); wait until `/ws/rs485` accepts.
- USB path: `ossm-std --mode rtu-relay --serial $DEVICE_PORT` (DTR=0/RTS=0) serving Modbus TCP; FC03 unit 1.
- WS path: `ossm-std --mode rtu-relay --rs485-ws ws://$DEVICE_IP/ws/rs485`; CFG baud then FC03.
- Restore `operating_mode=servo` (no restart) and assert `/state` `ups` recovers.
- Optional ACM magic-exit: `ossm-std --mode console --exit-rs485` expects `shell.operating_mode set to servo`.

### Termux USB-host flash + phone shell (`scripts/test_termux.py`)
Flash the C6 plugged into the Android phone (SSH host `termux`) and run `ossm-std` there:
```bash
./scripts/test_termux.py
./scripts/test_termux.py --skip-flash   # only the phone servo / --mock path
./scripts/test_termux.py --mock
```
- zigbuild `ossm-std-linux-arm64`, `scp` binary + `release/ossm-esp32c6.bin`.
- `ossm-std --mode console` dumps old pin/net; `--mode flash` writes the merged image over `termux-usb`.
- Re-provisions `net.*` and `pin.modbus_tx/rx/de_re` (default `2,1,0`), waits for `ip:`, updates `.env` `DEVICE_IP`, asserts firmware `/state` `ups` > 300.
- Live-switch firmware `rs485`, run phone `ossm-std --serial /dev/bus/usb/… --bind 0.0.0.0:8080`, homing + motion over HTTP; `--exit-rs485` then firmware `ups` recovers.
- First `termux-usb -r` needs a USB permission tap on the phone.

### WASM harness live tests (`scripts/test_wasm.py`)
Playwright E2E against `web/apps/webui-wasm` (or `release/ossm-wasm.html`). Live runs switch the device to `operating_mode=rs485` (no reboot):
```bash
./scripts/test_wasm.py           # mock + /ws/rs485 + Web Serial ACM
./scripts/test_wasm.py --mock    # harness only
./scripts/test_wasm.py --skip-serial
```
- Uses `WebSerialBridge` with DTR/RTS **off before open**.
- Restore `servo` and assert `/state` `ups` recovered.

### Bundled ossm-std UI (`scripts/test_webui_std.py`)
Playwright against `ossm-std --mock` serving the compile-time-embedded webui-std:
```bash
./scripts/test_webui_std.py
./scripts/test_webui_std.py --url http://192.168.24.222:8080
```
- Motion controls present (`#speed-slider`); **no** `#ble-enabled` / `#wifi-ssid`.

---

## 5. Web Applications Architecture (`/web`)

The web apps live under `web/` as an npm workspace (one Vite app per shipped HTML). Shared Vue/TS is `@ossm/shared`; transports are `@ossm/client`.

### Embedded Web Interface (`web/apps/webui-esp32`)
The firmware SPA provides real-time motor control over WiFi/BLE, including pin/net/WiFi/BLE settings (`UnifiedSettings`).
- **Single-File Embedding**: Compiles to `web/apps/webui-esp32/dist/index.html`. During build (`crates/ossm-esp32/build.rs`), it is compressed to `index.html.gz` in `OUT_DIR`. This compressed file is statically included into the firmware binary at compile time via `include_bytes!(concat!(env!("OUT_DIR"), "/index.html.gz"))` in `crates/ossm-esp32/src/http_api.rs` and served with `Content-Encoding: gzip` directly from ROM without filesystem overhead.
- **`web/apps/webui-std`**: motion-only (no pin/net/WiFi/BLE). `crates/ossm-std/build.rs` embeds `dist/index.html`; `GET /` serves it. `--static-dir` overrides for live Vite. REST CORS is `Access-Control-Allow-Origin: *`.
- **Key Components (`web/packages/shared/src/components/`)**:
  - `MotionWorkbench.vue`: diagram + MainControl / Spline / Funscript / Macro stack shared by all three webuis.
  - `MotorPositionDiagram.vue`: Toggleable visual diagram displaying full stroke range (`0% - 100%`), hardware bounds (`pos_min`/`pos_max`), active stroke window (Left Limit & Right Limit markers), and real-time animated position indicator at 30 FPS.
  - `MainControl.vue`: Manages core motion settings including BPM (speed), stroke depth, top/bottom depth anchoring, stroke reversal, and pause/resume functionality (supporting fixed-position and in-place pause modes).
  - `SplineEditor.vue`: Interactive curve editor allowing users to design custom periodic motion trajectories (`wave_func: 'spline'`) by manipulating spline control points.
  - `MacroPlayer.vue`: Interactive macro sequence player with top-level **Active Macro** selection above management actions (`Save`, `Save As`, `Rename`, `Delete`), clickable instruction gaps (`#macro-gap-X`) for seeking, exact `0%`/`100%` edge locating, `100%` wrap-around playback, and bilingual translations (`en.json` / `zh.json`). Logic in `macro.ts` (`BUILTIN_PRESETS`, validation, playback engine, `localStorage` keys `ossm_macros_v1` / `ossm_macro_draft_v1`).
  - `UnifiedSettings.vue` (`webui-esp32`): Modbus GPIO, `operating_mode`, `modbus_debug`, BLE enable, WiFi SSID/password, restart, motor `ups` telemetry.
- **Communication & State (`@ossm/client` + `@ossm/shared`)**:
  - Communicates with the firmware via HTTP REST/JSON endpoints (`/config`, `/state`, `/paused`, `/shell-config`, `/restart`) and optionally BLE (see `ble.ts` / `ConnectionMode`).
  - Implements a single-owner WebSocket client (`WsDataManager` in `client.ts`) for resilient real-time state synchronization via JSON-RPC 2.0 (`/ws/command`) at up to 30 FPS (`33ms`). Features automatic request/response promise routing (`pendingRequests`), application-level watchdog silent-failure detection, automatic reconnection loops (`triggerReconnect`) that re-subscribe active listeners without page refreshes, and automatic idle disconnection after 3 seconds when UI panels are closed.
  - **Causal Consistency & Data Mapping**: Tracks `authoritativeVersion` across mutations in `connectionStateMachine.ts`, rejecting stale remote config when `remoteConfig.version < authoritativeVersion`. `beginEnginePlayback` / `endEnginePlayback` (`MACRO` / `FUNSCRIPT`) and `beginLocalEdit` / `endLocalEdit` suppress background merges during virtual playback or optimistic edits; `scheduleOptimisticMutation` debounces slider POSTs. When diagram/settings panels are closed, falls back to **1 Hz** REST `/state` polling instead of WebSocket subscribe. `mapper.ts` (`wireToDomainConfig`, `domainToWireConfig`, `sanitizeMotorState`, `isConfigEqual`) preserves virtual `wave_func` (`macro` / `funscript`) in the UI while posting concrete firmware waveforms.
  - When `webui-esp32` is opened from hostname `localhost` in Vite DEV, the API client targets `http://ossm.lan`; production uses the page origin. `webui-std` Vite DEV targets `http://127.0.0.1:8080` (`VITE_OSSM_API`); the bundled binary uses same-origin.

### Web Serial Flasher & Configurator (`/flasher`)
The flasher is a standalone, browser-based tool (`web/apps/flasher/dist/index.html` -> `release/flasher.html`) that enables users to flash firmware binaries and configure microcontrollers over USB without installing local command-line tools.
- **Web Serial API & esptool-js**: Uses `@w3c/web-serial` and `esptool-js` (`ESPLoader`, `Transport`) to connect to ESP32-C6 / ESP32-S3 bootloaders directly from supported browsers (Chrome, Edge). Status-bar actions: **Connect** (ROM bootloader), **Flash Firmware**, **Monitor** / **Stop**, **Reset**, **Disconnect**.
- **Firmware Flashing**: Drag-and-drop or file selection of merged firmware images (e.g. `release/ossm-esp32c6.bin`) written at a configurable address (default `0x0`). After flash, `hard_reset` runs and status becomes `flash_done` while keeping the serial port so configuration can proceed without reconnecting.
- **Dual terminals**: **Console** (tool progress / ACKs) and **Serial** (device UART). Config send auto-attaches Serial monitor afterward.
- **Device Configuration**: Right-hand panel for WiFi enable/SSID/password, Modbus GPIO pins, Modbus timing overrides (`0` = firmware auto defaults), `modbus_debug`, and BLE enable. **Send Configuration** runs CLI commands then soft `reset`. Presets persist in browser `localStorage` under `ossm-flasher-device-config-v1`; **Reset to defaults** restores form defaults.

### 57AIM30 Modbus PC Control (`/motor-control`)
Standalone browser tool (`motor-control/dist/index.html` → `release/motor-control.html`) that mimics the vendor **YZ_AIM** VB6 form (`assets/57aim30_pc_control.frm`): Chinese groupbox captions, light-gray/Win32-ish chrome via shared `GroupBox.vue`, thin title bar (`YZ_AIM` / 57AIM30 PC Control) — not the embedded OSSM frontend.
- **Transport** (`lib/transport.ts`): `SerialModbusClient` (Web Serial), `WebSocketModbusClient` (`/ws/modbus`), or `WebSocketRs485Client` (`/ws/rs485` TX/RX/CFG). Baud is shown for Local Serial and RS-485 WebSocket.
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
3. **`get-state`**: Returns current `StateResponse` object (motor position, speed, coordinates, physical bounds `pos_min`/`pos_max`, `loop_stats` (`ups`, `dt_*`, `update_history`, `position_history`), `link_stats` (transport RTT / exchange counters), and config).
4. **`set-config`**: Accepts partial or full `MotorControllerConfig` in `params` to dynamically update motion parameters. Returns the updated configuration object including the newly incremented causal version number (`version: u32`) which clients use to reject stale background state pushes. Rejects writes where `params.version` is older than the current snapshot (`-32001 Stale causal version`). `POST /config` returns **409 Conflict** for the same case.
5. **`subscribe-state`**: Accepts `{"interval_ms": 300}` in `params` (range: 20–60000ms, default: 33ms). Acknowledges with `"result": {"subscribed": true, "interval_ms": 300}` confirming the resolved push rate. Re-sending `subscribe-state` dynamically adjusts the interval without requiring an unsubscribe. Begins pushing periodic notifications from the server:
   ```json
   {
     "jsonrpc": "2.0",
     "method": "state",
     "cmd": "state",
     "params": { ...StateResponse... }
   }
   ```
6. **`unsubscribe-state`**: Stops periodic state push notifications. Returns `"result": "unsubscribed"`.
7. **`reset-timestamp`**: Resets motion trajectory time `t` to 0.
8. **`append-waypoints`**: Appends a list of motion waypoints (`[{"ts": 100, "pos": 0.5, "vel": 0.1}, ...]`) to the stream buffer.
9. **`set-waypoints`**: Clears currently buffered waypoints and sets a new list of motion waypoints in the buffer. Supports passing a list directly or an object `{"waypoints": [...], "reset-timestamp": true}` to optionally reset the stream time anchoring.
10. **`get-shell-config`**: Firmware only. Returns current `ShellConfig` (GPIO/UART/BLE plus WiFi/mDNS). Not in `ossm-core::dispatch_rpc` / `ossm-std`.
11. **`set-shell-config`**: Firmware only. Updates `ShellConfig` in NVS (WiFi fields need reboot; `operating_mode` applies live).

### Bluetooth Low Energy (BLE) GATT API
When connected via Bluetooth Low Energy (Service UUID: `6e400001-b5a3-f393-e0a9-e50e24dcca9e`):
- **`CHAR_CONFIG` (`...0002`)**: Read/Write JSON string of `MotorControllerConfig` (includes monotonic `version`). Pre-populated on connect; updated on **config/paused writes** with the **applied** config returned by `try_enqueue_config`. GATT **Read** returns the cached attribute only (no mid-Read refresh).
- **`CHAR_STATE` (`...0003`)**: Read/Notify compact telemetry JSON (`position`, `speed`, `ups`, `y`/`shaped_y`, plus minimal config fields). Attribute buffer is ≤ **256 B** so ATT Read works without Read Blob. Pre-populated on connect; push notifications use the compact format via the chunked protocol. Full `StateResponse` is via JSON-RPC `get-state` on `CHAR_RPC`.
- **`CHAR_PAUSED` (`...0004`)**: Write JSON string `{"paused": true, "position": 0.0}` for immediate pause/resume (`paused_position` accepted as an alias). Updates the cached `CHAR_CONFIG` attribute value; does **not** send unsolicited config notifications on write.
- **`CHAR_PIN_CONFIG` (`...0005`)**: Read/Write JSON string of `PinConfiguration` (including `"ble_enabled": true` and `"modbus_debug": false`). Pre-populated on connection and updated on write; GATT Read returns the cached value.
- **`CHAR_RPC` (`...0006`)**: Write/Notify JSON-RPC 2.0 command strings (e.g., `ping`, `get-state`, `subscribe-state`, `unsubscribe-state`, `restart`, `get-shell-config`, `set-shell-config`). All notifications use the **chunked notification protocol** (see below).
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
When interacting over USB serial (`115200` baud, `\r\n` terminated), configuration uses nmcli-style **`get <path>`** / **`set <path> <value>`**. Type **`paths`** on-device for the full catalog (also in README). Shell/inject paths are **firmware-only**; `ossm-std` stdin REPL is motor + motion actions.

**Sections (firmware):** `get shell` | `get motor` (full JSON); scalars: `get shell.modbus_tx`, `get shell.ssid`, `get motor.bpm`, etc. Legacy `pin.*` / `net.*` aliases still set; ACK is `shell.<key> set to`.

**Shell paths:** `shell.modbus_tx` / `modbus_rx` / `modbus_de_re` (GPIO 0..48); `shell.modbus_timeout_ms` (0..1000, 0=default); `shell.modbus_rx_timeout_us` / `modbus_scan_delay_us` / `modbus_inter_frame_delay_us` (0..200000, 0=auto); `shell.modbus_baud` (1200..3000000, default 115200); `shell.ble_enabled` / `shell.modbus_debug` (true|false, debug needs reboot); `shell.operating_mode` (servo|rtu_relay|rs485, live); `shell.wifi_enabled` / `dhcp_enabled`; `shell.ssid` / `password` / `hostname`; `shell.static_ip` / `static_mask` / `static_gateway` / `static_dns`.

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

- Motor loop runs `compute_cycle()` at high frequency; shells aggregate 1-second window stats into `StateResponse.loop_stats` via `LoopStatsWindow` + `SetLoopStats`. `compute_cycle()` clamps step duration (`dt.min(0.012)`) so CPU preemption does not jerk the servo. Link timing (`link_stats`) is aggregated per shell (`LinkStatsWindow` + `SetLinkStats`) from UART/serial/WebSocket exchanges.
- Publish telemetry with `AppContext::update_snapshot` (in-place `copy_from` / `Arc::make_mut`). **Never** `Arc::new(snapshot.clone())` every motor cycle — that OOMs the WiFi+BLE **96 KiB** heap. Allocate a new `Arc` **outside** the critical section only when the current snapshot is shared.
- Boot policy is **paused** (`MotorControllerConfig::default().paused = true`; post-homing `sync_to_position` also forces pause). Snapshot `config` updates only when `config_version` advances — after any logical config change call `set_config` / `commit_config` or non-blocking `try_enqueue_config`, **not** bare `self.config.field = ...`. Every update increments `version: u32`. The web frontend (`connectionStateMachine.ts`) discards stale background pushes. Skipping the bump leaves `/config` and `/state` advertising stale versions while `MotionMode::Paused` holds the motor still.
- `try_enqueue_config` pre-increments `version`, rejects stale `config.version < snapshot.version` (`409` REST / RPC `-32001`), retries enqueue for ~250 ms on queue full, and returns the authoritative config blob.
- Firmware SPSC is `MotionCommand` capacity **3** (`crates/ossm-esp32/src/motion.rs`). The motor task dequeues into `Engine::apply` then `tick`. After homing: `HomingComplete` → `sync_to_position` parks via `set_config(paused=true)`, then `flush_snapshot` / `update_snapshot` before the control loop.
- Performance (required): on **ESP32-C6** under concurrent HTTP + WebSocket, observed `loop_stats.dt_max_ms` must stay **< 4.5 ms**. Scratchpad in hot paths must use Embassy mutex fast-path (non-yield expected path), not long critical-section serialization.
- The web UI disconnects the live WebSocket after 3 seconds of inactivity when no components are subscribed.

### 7.3 Task placement and cores

- **ESP32-C6:** UART1 owner (`uart_owner_task`) on `esp_rtos::embassy::InterruptExecutor` at elevated priority. Console, CLI, network, BLE on the main Embassy executor.
- **ESP32-S3:** UART1 owner on **Core 1** via `esp_rtos::start_second_core(...)` → `run_uart_owner_blocking(...)`. WiFi/HTTP on Core 0. Console + CLI stay on Core 0.
- Boot: `esp_hal::init()` → `esp_alloc` (**96 KiB** heap; S3 also places a ~**24 KiB** Core-1 motor/relay stack in `.dram2_uninit`) → `esp_rtos::start()` → `console::init_logger()` → spawn `console_task` → CLI / UART1 owner / storage → WiFi/HTTP (`HTTP_READY`) → then BLE.
- `servo`, `rtu_relay`, and `rs485` are exclusive (same UART1). The owner task stops the current role and re-inits; never spawn two UART1 roles at once.

### 7.4 Firmware Modbus (GDMA, timing, debug)

- RS-485 via ESP-HAL GDMA `Uhci` (`UHCI0` + `DMA_CH0`) wrapping `esp_hal::uart::Uart`. `UhciRx` + `UhciTx` with `esp_hal::dma_buffers!(256, 256)` stream bytes into memory without per-FIFO CPU interrupts.
- **RX inter-byte timeout ($t_{1.5}$):** default `750 µs` (`compute_rx_inter_byte_timeout`) — Modbus RTU spec for >19200 bps. This is the SOFTWARE `with_timeout()` guard per `read_async`. Whole-frame timeout default at 115200 is **10 ms**.
- **Hardware UART FIFO idle (`timeout_symbols`):** when `modbus_rx_timeout_us == 0`, default symbols cover $t_{3.5}$ (**5** at 115200, ~**434 µs**) so software IFD is 0. Production `pkt_thres` is set once to FC16 ACK length **8**; FC03 reads raise it for that exchange then restore. Debug mode uses `pkt_thres=256` plus idle-EOF.
- **Re-arm race:** CPU two-phase reads (`uart_read_exactly(&resp[..3])` then `resp[3..len]`) could drop bytes in the re-arm gap. GDMA captures continuously between phases so high-rate polling (>330 Hz) does not lose frames.
- **Inter-frame quiet ($t_{3.5}$):** default `350 µs` at `115200` (`get_default_inter_frame_delay`). Hardware idle covers that silence; remaining software `Delay` is 0 unless the user shortened RX timeout. End-to-end RTU cycle **< 3 ms** for **> 330 Hz** `ups` while moving. Paused loops still write FC16 every cycle (same rate as moving).
- **`modbus_debug`:** keep DMA at **`dma_buffers!(256, 256)`**. Do **not** raise production `pkt_thres` to capture capacity (false `empty` after cancel). Debug only: 5 ms software RX deadline, CRC-resync (`exact` / `long` / `long_resync` / `leading_junk` / `parse_fail` in `ossm-core` `find_modbus_response`), `MODBUS_DBG` logs (`skip`/`trim`; throttle `exact` when inject off). Raises USB log volume; `ups` modestly lower; leave off in production.
- RAM inject (firmware `modbus_rtu.rs`, not the bus): `set inject` or **`POST /modbus-inject`** `{"mode":"leading|trailing|both|off","nbytes":N}`. Host: `scripts/test_modbus_resync.py`. Live C6: `scripts/test_modbus_debug_device.py`. Console fairness: `scripts/test_console.py`.
- `ossm-std` cannot use t1.5/t3.5 on PC USB-serial; its RX is `modbus_rx.rs` (unbounded stream, emit on header+CRC). Same `aim30` literals as firmware.

### 7.5 HTTP and WebSocket

- `edge-http`: 4 signal-gated acceptors, socket queue, up to 3 concurrent WebSocket sessions. Embassy net **`StackResources<20>`** (DHCP + HTTP + Modbus TCP `:502` + concurrent TCP).
- **`REST_GATE` serializes mutating POSTs only.** GETs, `/state`, and `/restart` must not hold the gate for the whole request (starves acceptors; port 80 looks dead under burst).
- JSON REST: `"Connection": "close"`. Gzip HTML `/` sends `Content-Length` (not chunked) and does not need `Connection: close`. WebSocket: `FrameType::Text(false)` (final text). End WS sessions with **`drop(socket)`**, not `close(Both).await` (smoltcp Load-fault after peer/WiFi teardown). REST may still `finish_connection` → `close(Both)`.
- Prefer **`GET/POST /modbus-inject`** over serial `set inject` when `MODBUS_DBG` floods USB. `rtu_relay` serves `/ws/modbus` and Modbus TCP `:502`. `rs485` serves `/ws/rs485` (binary `u8 type | u16le len | payload`: 0 TX, 1 RX, 2 CFG). A TX payload equal to `EXIT_MAGIC` (`F0 0F OSSM ESC q`) exits rs485 the same way USB does. Wrong-mode WS paths return 404.
- `ossm-std` HTTP is axum: `--mode servo` → `/` (embedded webui-std) `/config` `/state` `/link-stats` `/paused` `/restart` `/ws/command` (CORS `*`); `--mode rtu-relay` → `/ws/modbus` + Modbus TCP `--modbus-bind` + `GET /link-stats`. `restart` exits the process.

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
- Embed `web/apps/webui-esp32/dist/index.html` → `OUT_DIR/index.html.gz` → `include_bytes!` in `http_api.rs`.
- `ossm-std` `build.rs` embeds `web/apps/webui-std/dist/index.html` (override with `--static-dir`).

### 7.10 Keep APIs synchronized

When adding a command or configuration property, update the crate that owns it, then every consumer:

- **Motion / causal version / RPC catalog:** `ossm-core` (`config`, `state`, `command`, `rpc`, `rpc_types`, `paths`, `engine`) plus tests.
- **Telemetry windows, wire structs, and link PLL** — `ossm-common`; shell drivers in `motor_57aim30.rs` / `engine_task.rs` / `motor_thread.rs` / `shell.rs` / `bus.rs`.
- **Firmware SPSC / enqueue:** `crates/ossm-esp32/src/motion.rs`, `context.rs` (`try_enqueue_config`).
- **Firmware HTTP/WS:** `http_api.rs`. Pin/net/inject live here, not in core.
- **Firmware BLE:** `ble_api.rs`.
- **Firmware async RPC adapter:** `crates/ossm-esp32/src/rpc.rs` (enqueue; still handles get/set-shell-config on storage).
- **Firmware CLI:** `command.rs` + `hw_paths.rs` (pin/net/inject). Motor paths stay in `ossm_core::paths`.
- **Desktop shell:** `ossm-std` `http.rs` / `cli.rs` / `engine_task.rs` / `motor_thread.rs` / `precise_wait.rs` / `bus.rs` / `relay_server.rs` / `flash_port.rs` / `esptool/` / `console_mode.rs` (motor + motion RPC, rtu-relay, or flash/console). CORS `*` on REST. Bundled webui-std.
- **webuis:** `web/packages/shared` + `web/packages/client` + `web/apps/webui-esp32` / `webui-std` / `webui-wasm`.
- **WASM harness:** `web/apps/webui-wasm` + `crates/ossm-wasm`; rebuild `release/ossm-wasm.html`.
- **57AIM30 PC tool:** `web/apps/motor-control/src/lib/registers.ts`, `send-options.ts`, panels; rebuild `release/motor-control.html`.
- **Scripts:** `ossm.py`, `test_websocket.py`, `test_frontend.py`, `test_webui_std.py`, `test_wasm.py`, `test_ble.py`, `test_modbus_debug_device.py`, `test_console.py`, `test_rs485_transceiver.py`, `test_termux.py`, `setup_device.py`.
- **Docs:** `README.md`, `README.zh.md`.

### 7.11 Frontend embed and wiring diagrams

- After `web/apps/webui-esp32/` changes, **`npm run build -w webui-esp32`** in `web/` (or `uv run scripts/release.py`) before compiling/flashing firmware — otherwise ROM still has the old `index.html.gz`.
- After `web/apps/webui-std/` changes, rebuild **before** `cargo`/`zigbuild` `-p ossm-std` so the embedded HTML is current.
- `web/apps/motor-control/` and `web/apps/webui-wasm/` are **not** firmware-embedded; rebuild and copy to `release/motor-control.html` / `release/ossm-wasm.html`.
- **Never** hand-author SVG wiring diagrams. Design HTML/CSS in `assets/wiring_diagram.html` / `wiring_diagram_zh.html` (Path B ESP32) and `wiring_diagram_std.html` / `wiring_diagram_std_zh.html` (Path A USB-RS485). Bright theme, CSS Grid, JS midpoint routing. Export with `scripts/export_wiring_svg.py` (Playwright + `html-to-image`).
