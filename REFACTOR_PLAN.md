# Refactoring OSSM-Rust for `std` and Wasm

Transition the firmware from a single `no_std` crate into a **Cargo workspace** with **Inversion of Control**. Core logic stays pure (`no_std` + `alloc`). Device shells own hardware, clocks, I/O, and persistence.

This document is the implementation spec: crate layout, module map, public types, and sprints with exit criteria.

---

## 0. Non-goals and invariants

**Non-goals (this program)**

- Do not introduce HAL traits (`trait Uart`, `trait Nvs`) that `ossm-core` calls into.
- Do not put `embassy-executor`, `tokio`, `esp-hal`, or `wasm-bindgen` in `ossm-core`.
- Do not change the JSON-RPC / REST / CLI wire protocol in a breaking way.
- Do not move Vue apps (`frontend/`, `flasher/`, `motor-control/`) into Rust crates.

**Invariants to preserve**

| Area | Rule |
| --- | --- |
| Motion | `compute_cycle` clamp `dt ≤ 12 ms`; post-homing pause via `set_config` (version bump); default `paused: true` |
| Config | Sole write path bumps `version`; stale `version` → RPC `-32001` / HTTP 409 |
| Heap | ESP32 heap stays **96 KiB**; snapshot publish stays in-place `Arc` (shell), never `Arc::new` every cycle |
| Motor loop | C6: `InterruptExecutor`; S3: Core 1. Observed `dt_max_ms < 4.5 ms` under HTTP+WS |
| Transport | WS sessions `drop(socket)`; BLE GATT Read is accept-cached only; BLE after `HTTP_READY` |
| Persistence | Debounce ≥ 2 s; ignore pause-only diffs (`paused` / `paused_position`) |

---

## 1. Cargo workspace

```
ossm-rust/                          # git root
  Cargo.toml                        # [workspace]
  rust-toolchain.toml               # stays `esp` (C6 + S3 Xtensa)
  .cargo/config.toml                # aliases target ossm-esp32
  crates/
    ossm-core/                      # pure engine (no_std + alloc)
    ossm-esp32/                     # today's firmware (embassy + esp-hal)
    ossm-std/                       # desktop (tokio)
    ossm-wasm/                      # browser (wasm-bindgen)
  frontend/  flasher/  motor-control/  scripts/  assets/
```

### 1.1 Root `Cargo.toml` (target)

```toml
[workspace]
resolver = "2"
members = [
    "crates/ossm-core",
    "crates/ossm-esp32",
    "crates/ossm-std",
    "crates/ossm-wasm",
]
default-members = ["crates/ossm-esp32"]

[workspace.package]
version = "0.2.0"
edition = "2021"
license = "MIT"
```

`ossm-esp32` keeps the current `[features]` (`esp32c6` / `esp32s3`), `esp_app_desc!`, `build.rs` (gzip `index.html` + S3 linker), and `[[bin]] harness = false`.

Aliases become:

```toml
b-c6 = "build -p ossm-esp32 --target riscv32imac-unknown-none-elf --features esp32c6 --no-default-features"
b-s3 = "build -p ossm-esp32 --target xtensa-esp32s3-none-elf --features esp32s3 --no-default-features"
```

Host crates are **never** default members so `cargo b-c6` cannot pull `tokio` into the RISC-V build.

### 1.2 Toolchain

| Crate | Compiler |
| --- | --- |
| `ossm-esp32` | workspace `esp` channel (required for Xtensa S3) |
| `ossm-core` | must compile on **both** `esp` and stable (`cargo +stable test -p ossm-core`) |
| `ossm-std` / `ossm-wasm` | `cargo +stable` (or `esp` host std if it works; CI uses stable) |

Do not add a second `rust-toolchain.toml` under `crates/` — it would override the workspace pin for firmware builds. Document `+stable` in scripts/CI instead.

### 1.3 Dependency rules

```
ossm-core     → serde, serde_json (alloc), serde-json-core, libm, log, portable-atomic
                NO embassy-*, esp-*, heapless, tokio, wasm-bindgen

ossm-esp32    → ossm-core + current firmware deps (esp-hal, embassy, edge-http, …)

ossm-std      → ossm-core + tokio, tokio-serial, axum (or warp), serde_json/std, clap

ossm-wasm     → ossm-core + wasm-bindgen, js-sys, web-sys (SerialPort is JS-side)
```

`embassy-sync` channels are executor-agnostic but **pull Embassy mutex types into core**. Do not depend on them. Shells own queues; core is called synchronously.

---

## 2. IoC architecture

`ossm-core` is a **pure state machine**. Shells own the world and call in.

```
┌─────────────────────────────────────────────────────────────┐
│  SHELL  (esp32 / std / wasm)                                │
│  owns: clock, UART/WebSerial, HTTP/BLE/CLI, NVS/FS/JS store │
│                                                             │
│   bytes in ──► Modbus codec (core, pure) ──► Feedback       │
│   RPC JSON ──► Engine::dispatch_rpc ──► RpcAction           │
│   now_us   ──► Engine::tick ──► CycleOutput { pos, speed }  │
│   Event::Persist* ──► shell writes storage                  │
│   CycleOutput ──► encode RTU ──► bytes out                  │
└────────────────────────────▲────────────────────────────────┘
                             │  sync calls, no .await in core
                             ▼
                  ┌─────────────────────┐
                  │  ossm-core::Engine  │
                  │  MotorController    │
                  │  pin / net caches   │
                  └─────────────────────┘
```

### 2.1 Clock (replace `embassy_time::Instant`)

Today `MotorController` and `WaveformMotionSource` call `Instant::now()`. That cannot exist in core.

```rust
// crates/ossm-core/src/time.rs
pub type Micros = u64;

pub fn dt_seconds(prev: Micros, now: Micros) -> f32 {
    now.saturating_sub(prev) as f32 / 1_000_000.0
}
```

- `MotorController::tick(now: Micros) -> CycleOutput` (renamed from `compute_cycle`).
- `reset_cycle_clock(now: Micros)` — shell passes current time after homing.
- Waveform `t0` is a `Micros` epoch, not `Instant`.

Shell clocks:

| Shell | Source |
| --- | --- |
| esp32 | `embassy_time::Instant::now().as_micros()` (or elapsed since boot) |
| std | `std::time::Instant` converted to µs |
| wasm | `js_sys::Date::now()` or `performance.now()` → µs |

### 2.2 Commands in, events out

Core does **not** own `heapless::spsc`. The ESP32 SPSC (`COMMAND_QUEUE_SIZE = 3`) stays in `ossm-esp32::context` as a transport between HTTP/BLE/CLI and the motor task. The motor task dequeues and calls `engine.apply(...)`.

```rust
// crates/ossm-core/src/command.rs
pub enum Command {
    SetConfig(MotorControllerConfig),
    AppendWaypoints(Vec<StreamWaypoint>),
    SetWaypoints { waypoints: Vec<StreamWaypoint>, reset_timestamp: bool },
    ResetTimestamp,
    SetPaused { paused: bool, position: Option<f32> },
    SetPin(PinConfiguration),
    SetNet(NetworkConfiguration),
    HomingComplete { pos_min: f32, pos_max: f32, position: f32 },
    SetMotorConnected(bool),
    SetModbusStats(ModbusStats),
    SetInject { mode: InjectJunkMode, nbytes: u8 },
}

// crates/ossm-core/src/event.rs
pub enum Event {
    /// Motor config changed (version already bumped). Shell persists (debounced).
    PersistMotor(MotorControllerConfig),
    PersistPin(PinConfiguration),
    PersistNet(NetworkConfiguration),
    RestartRequested,
}
```

`apply` is sync and returns drained `Event`s (small `heapless::Vec` in core is OK if it is a **local** buffer type, or `EventSink` callback). Prefer returning a tiny `Events` struct with `Option`s to avoid allocating on the motor hot path:

```rust
pub struct Events {
    pub persist_motor: Option<MotorControllerConfig>,
    pub persist_pin: Option<PinConfiguration>,
    pub persist_net: Option<NetworkConfiguration>,
    pub restart: bool,
}
```

Pause-only diffs: core still emits `PersistMotor`; the **shell** continues to drop them in `persistent_motor_changed` before flash/FS write.

### 2.3 Tick / motor I/O

```rust
pub struct CycleOutput {
    pub position: f32,
    pub speed: f32,
}

impl Engine {
    pub fn apply(&mut self, cmd: Command) -> Events { /* … */ }

    /// Advance trajectory. Caller writes `position` to the drive.
    pub fn tick(&mut self, now: Micros) -> CycleOutput { /* MotorController::tick */ }

    pub fn snapshot(&self) -> &StateResponse { self.motion.snapshot() }
}
```

Shell motor loop (all three targets):

1. Drain inbound `Command`s → `engine.apply`.
2. `out = engine.tick(now_us)`.
3. Encode 57AIM30 write-position RTU (core helper) → write bytes.
4. Read/decode response (core `find_modbus_response`) → `SetMotorConnected` / `SetModbusStats`.
5. Publish `engine.snapshot()` (ESP32: existing `AppContext::update_snapshot`).

Homing stays in the **shell** (async UART). After success: `engine.apply(HomingComplete { ... })` which calls today's `sync_to_position` (forces pause + version bump).

### 2.4 RPC stays transport-agnostic

Move `dispatch_rpc` into core as **synchronous** JSON in / JSON out. No `AppContext`, no Embassy, no scratchpad mutex.

```rust
pub enum RpcAction {
    Respond,           // bytes already written to `out`
    Subscribe { interval_ms: u64 },
    Unsubscribe,
    Restart,
}

pub fn dispatch_rpc(
    engine: &mut Engine,
    request_json: &[u8],
    out: &mut [u8],
) -> Result<(RpcAction, usize), RpcError>;
```

`Subscribe` / `Unsubscribe` are **session** concerns. Core returns the action; the HTTP/BLE task owns the timer and calls `engine.snapshot()` to build notifications (`build_state_notification_into`).

Causal version check (`try_enqueue_config` retry ~250 ms) moves:

- **Hot path (motor task):** `engine.apply(SetConfig)` is non-blocking.
- **Control path (HTTP/BLE/CLI):** shell still pre-increments version against `snapshot.config.version`, rejects stale, retries queue-full. On ESP32 that queue is the existing SPSC. On std/wasm it can be `tokio::sync::mpsc` / a JS-fed buffer.

`ossm-esp32` keeps `buffers::try_with_scratchpad` as the `out` buffer source. Core never locks a global.

---

## 3. Crate module maps

### 3.1 `ossm-core` (new)

```
crates/ossm-core/src/
  lib.rs                 # re-exports; #![no_std] + extern crate alloc
  error.rs               # CoreError (Config, Json, QueueFull, StaleVersion, RelayMode)
  time.rs                # Micros, dt_seconds
  config.rs              # MotorControllerConfig, PinConfiguration, NetworkConfiguration
                         #   OPERATING_MODE_*, PinConfiguration::is_rtu_relay
  state.rs               # StateResponse, StreamStatus, LoopStats, ModbusStats, TimingWindowStats
  command.rs             # Command
  event.rs               # Events
  engine.rs              # Engine { motion, pin, net, inject }
  rpc.rs                 # dispatch_rpc, subscribe_ack helpers (write into &mut [u8])
  rpc_types.rs           # RpcRequest (today's WsMessage), WaypointsInput, SubscribeParams,
                         #   PausedControl
  paths.rs               # nmcli-style get/set path catalog (CLI + std REPL share this)
  motion/
    mod.rs
    waveform.rs          # Sine / Thrust / Spline  (today ~L20–310)
    source.rs            # MotionSource, paused / waveform / streaming
    shaper.rs            # Shaper, PositionGenerator
    controller.rs        # MotorController — tick(now), no SPSC, no Instant
  modbus/
    mod.rs
    crc.rs               # calc_crc16, verify_rtu_crc
    frame.rs             # find_modbus_response, classify_*, apply_capture_inject
    inject.rs            # InjectJunkMode + AtomicU8 knobs (RAM, reboot-cleared)
    aim30.rs             # encode/decode 57AIM30 position / scan / baud frames (no UART)
```

**Host tests** (`crates/ossm-core/tests/` or `#[cfg(test)]`):

- Waveform evaluate / `find_x_for_y` / spline hermite
- `tick` dt clamp and pause-after-homing version bump
- Stale config rejection
- Modbus CRC + resync (port `scripts/test_modbus_resync.py` cases into Rust so they cannot drift)
- `dispatch_rpc` ping / set-config / waypoints golden JSON

### 3.2 `ossm-esp32` (today's `src/`, after the move)

```
crates/ossm-esp32/src/
  main.rs                # init, spawn order unchanged
  lib.rs                 # optional; bin may be the only crate type
  console.rs             # USB/UART0/RTT — unchanged
  command.rs             # embedded-cli I/O; path logic calls ossm_core::paths
  context.rs             # AppContext, SPSC, snapshot Arc, storage_task
  storage.rs             # StorageManager NVS only (types live in core)
  buffers.rs             # Embassy scratchpad / net buffer pool
  http_api.rs            # edge-http transport; REST_GATE; calls Engine via AppContext
  ble_api.rs             # trouble-host GATT; RPC → dispatch_rpc
  wifi.rs
  motor.rs               # delete or shrink: Motor trait is shell-local if still useful
  motor_57aim30.rs       # UHCI/GDMA/UART1 + timing; encode via ossm_core::modbus::aim30
  modbus_relay.rs        # RTU↔TCP/:502 /ws/modbus; CRC from core
  rpc.rs                 # thin wrapper: lock context → ossm_core::dispatch_rpc
```

`AppContext` remains the ESP32 **shared handle** (Copy tip). Internally the motor task owns `Engine`. Control tasks never call `tick`; they only enqueue `Command`.

Operating mode `rtu_relay`: do not construct `MotorController`; relay shell uses `ossm_core::modbus` only. RPC still returns `-32001 rtu_relay mode` for motion methods.

### 3.3 `ossm-std`

```
crates/ossm-std/src/
  main.rs                # clap: --serial /dev/ttyUSB0 --config ./ossm.json --bind 0.0.0.0:80
  engine_task.rs         # tokio task: serial I/O + engine.tick at ~1 kHz or after each RX
  serial.rs              # tokio-serial RS-485 (DE/RE via gpio if needed, or USB-UART dongle)
  persist.rs             # atomic JSON: write config.tmp → sync → rename config.json
  http.rs                # axum: GET/POST /config /state /paused /pin-config /network-config
                         #        WS /ws/command  (same JSON-RPC as firmware)
  cli.rs                 # stdin REPL using ossm_core::paths (optional)
```

Atomic persist (from original plan): temp file in the same directory, flush, `rename`. Schema = `{ pin, net, motor }` JSON matching NVS blobs so a file can be copied to the device later.

No BLE. No UHCI. Homing: same register sequence as `motor_57aim30::homing`, using core `aim30` frames.

### 3.4 `ossm-wasm`

```
crates/ossm-wasm/src/
  lib.rs                 # #[wasm_bindgen] OssmEngine
  bridge.rs              # JsValue ↔ Command / snapshot JSON
```

JS owns WebSerial **and** the page HTTP/WS (or the Vue app talks to the wasm engine in-process).

Exported surface (stable names — frontend will call these):

```rust
#[wasm_bindgen]
impl OssmEngine {
    #[wasm_bindgen(constructor)]
    pub fn new() -> OssmEngine;

    /// JSON-RPC 2.0 request string → response string (or subscribe sentinel).
    pub fn dispatch_rpc(&mut self, request: &str) -> String;

    /// Drive one control cycle. `now_us` from performance.now()*1000.
    pub fn tick(&mut self, now_us: f64) -> CycleJs; // { position, speed }

    pub fn snapshot_json(&self) -> String;

    /// Raw RTU bytes from WebSerial RX (optional; JS may also pass decoded feedback).
    pub fn ingest_rtu(&mut self, bytes: &[u8]);

    /// Next RTU frame to write (empty if none).
    pub fn take_tx(&mut self) -> Vec<u8>;

    /// Persist payloads for JS (localStorage / IndexedDB). Empty if none.
    pub fn take_persist_json(&mut self) -> Option<String>;
}
```

JS loop (sketch): `requestAnimationFrame` or `setInterval(3ms)` → `tick` → `writer.write(take_tx())` → on serial `readable` → `ingest_rtu`. Config persist: `take_persist_json` → `localStorage.setItem('ossm-config', ...)`.

---

## 4. File-level migration (current → new)

| Current | Destination | Notes |
| --- | --- | --- |
| `src/motion.rs` (~1.6k) | `ossm-core/src/motion/*` + `config.rs` + `state.rs` | Split by layer; drop `CommandProducer`/`Consumer` types |
| `src/rpc.rs` | `ossm-core/src/rpc.rs` | Sync; `&mut [u8]` out |
| `src/http_api.rs` `WsMessage`, `WaypointsInput`, `SubscribeParams`, `PausedControl` | `ossm-core/src/rpc_types.rs` | HTTP crate keeps handlers only |
| `src/storage.rs` types | `ossm-core/src/config.rs` | `StorageManager` stays in esp32 |
| `src/modbus_rtu.rs` | `ossm-core/src/modbus/` | Already mostly pure |
| 57AIM30 register packing in `motor_57aim30.rs` | `ossm-core/src/modbus/aim30.rs` | UART/DMA stays in esp32 |
| `src/error.rs` | split: `CoreError` vs `FirmwareError` | Shell maps UART/WiFi/BLE |
| `src/command.rs` path table | `ossm-core/src/paths.rs` | `embedded-cli` derive stays in esp32 |
| `src/context.rs` | `ossm-esp32` | Snapshot Arc + SPSC + `storage_task` |
| `src/buffers.rs` | `ossm-esp32` | Embassy mutex |
| `src/http_api.rs` rest | `ossm-esp32` + later copy protocol tests to std |
| `src/ble_api.rs`, `wifi.rs`, `console.rs`, `main.rs` | `ossm-esp32` | Unchanged responsibility |
| `src/modbus_relay.rs` | `ossm-esp32` (std may get a TCP relay later) | CRC from core |
| `src/lib.rs` (`pub mod modbus_rtu`) | delete after move | tests import `ossm_core` |
| `scripts/test_modbus_resync.py` | keep as cross-check; add Rust tests as source of truth | |

---

## 5. Engine API (canonical)

```rust
pub struct Engine {
    motion: MotorController,
    pin: PinConfiguration,
    net: NetworkConfiguration,
}

impl Engine {
    pub fn new(motor: MotorControllerConfig, pin: PinConfiguration, net: NetworkConfiguration) -> Self;

    pub fn apply(&mut self, cmd: Command) -> Events;
    pub fn tick(&mut self, now: Micros) -> CycleOutput;
    pub fn snapshot(&self) -> &StateResponse;
    pub fn pin(&self) -> &PinConfiguration;
    pub fn net(&self) -> &NetworkConfiguration;
    pub fn is_rtu_relay(&self) -> bool;

    /// Causal set-config used by RPC/REST. Returns applied config or StaleVersion / RelayMode.
    pub fn try_set_config(&mut self, cfg: MotorControllerConfig) -> Result<MotorControllerConfig, CoreError>;
}
```

`MotorController` no longer stores `command_consumer`. Streaming/config commands go through `apply` only.

`try_set_config` contains the version gate that today lives in `AppContext::try_enqueue_config` **minus** the 250 ms retry (retry is a shell queue concern).

---

## 6. Storage strategy (unchanged policy, split ownership)

Core never opens files or NVS. `Events.persist_*` is the only signal.

| Shell | Backend |
| --- | --- |
| esp32 | `storage_task` + `esp-nvs` (2 s debounce, skip pause-only) |
| std | `{pin,net,motor}` in `config.json` via temp+rename+fsync |
| wasm | JS `localStorage` / IndexedDB from `take_persist_json` |

Pin GPIO numbers on wasm/std are advisory (serial path is a CLI flag / JS port picker). Still persist them so the same JSON works on-device.

---

## 7. Wasm / JS bridge

- **Motor:** WebSerial in JS; wasm only sees RTU bytes + `tick`.
- **Control plane:** either (a) Vue talks to wasm in-process (no WiFi stack) or (b) JS hosts WS/REST and forwards JSON into `dispatch_rpc`. Prefer (a) for the first wasm demo; (b) can wrap the same `OssmEngine`.
- Do not compile `edge-http` or `embassy-net` to wasm.

Frontend change (late sprint): a `ConnectionMode = 'wasm'` next to wifi/ble that constructs `OssmEngine` and skips `DEVICE_IP`. Until then, wasm is verified with a small `crates/ossm-wasm/examples/web/` or a Playwright harness.

---

## 8. Sprints

Each sprint is mergeable. Firmware on C6/S3 must still flash and pass existing device tests unless the sprint says otherwise. Do not start sprint *N+1* until *N*'s exit criteria pass.

### Sprint 0 — Workspace skeleton (no behavior change)

**Goal:** Cargo workspace exists; firmware path is `crates/ossm-esp32`; `ossm-core` is an empty `no_std` crate.

**Work**

- Add `[workspace]`, move package to `crates/ossm-esp32/` (include `build.rs`, `ld/`, `frontend/` gzip still referenced from esp32 `build.rs` via `CARGO_MANIFEST_DIR` + `../../frontend/dist`).
- Retarget `.cargo/config.toml` aliases and `scripts/release.sh` (`-p ossm-esp32`).
- `crates/ossm-core` with `#![no_std]`, empty `lib.rs`, `cargo +stable check -p ossm-core`.
- Update `AGENTS.md` paths (`src/main.rs` → `crates/ossm-esp32/src/main.rs`).

**Exit**

- `cargo b-c6 --release` and `cargo b-s3 --release` succeed.
- `./scripts/release.sh` produces `release/ossm-esp32c6.bin` (or documented equivalent).
- Device still boots (spot-check).

**Risk:** `build.rs` / `include_bytes!(OUT_DIR)` paths; S3 `ld/esp32s3` relative to the new manifest.

---

### Sprint 1 — Extract pure types + Modbus codec

**Goal:** DTOs and Modbus CRC/resync live in `ossm-core`. Firmware re-exports them. No motion math move yet.

**Work**

- Move `MotorControllerConfig`, `StateResponse`, `StreamWaypoint`, `MotionCommand`, `PinConfiguration`, `NetworkConfiguration`, `PausedControl`, `WsMessage`, `WaypointsInput`, `SubscribeParams`.
- Move `modbus_rtu.rs` → `ossm-core::modbus`.
- Add Rust tests mirroring `scripts/test_modbus_resync.py`.
- `ossm-esp32` `use ossm_core::{...}`. Keep `motion.rs` in esp32 for this sprint.

**Exit**

- `cargo +stable test -p ossm-core`
- `cargo test --manifest-path crates/ossm-core/Cargo.toml` (no esp toolchain required)
- `cargo b-c6 --release`
- `uv run scripts/test_modbus_resync.py` still green

---

### Sprint 2 — Motion engine without Embassy time

**Goal:** `MotorController::tick(now: Micros)` in core; firmware motor task supplies time.

**Work**

- Split `motion.rs` into `ossm-core/src/motion/*`.
- Remove `heapless::spsc` from `MotorController`; `drain_commands` → `Engine::apply`.
- `ossm-esp32` motor task: dequeue SPSC → `apply` → `tick(now)` → existing UART write.
- `sync_to_position` / `commit_config` / snapshot rules unchanged.
- Host tests: waveform, dt clamp, homing pause + version bump, stale config.

**Exit**

- `cargo +stable test -p ossm-core`
- C6 device: `ups` healthy, `dt_max_ms < 4.5` under `scripts/test_frontend.py` concurrent HTTP (or `scripts/stress_dt_max.py` if used)
- Pause/resume + set-config version still causal (`test_websocket.py` `set-config`)

**Risk:** µs wrap / first-tick dt spike — call `reset_cycle_clock(now)` after homing exactly as today.

---

### Sprint 3 — RPC + path catalog in core; ESP32 is a shell

**Goal:** `dispatch_rpc` and CLI path get/set are core; esp32 only transports.

**Work**

- Sync `dispatch_rpc(&mut Engine, &[u8], &mut [u8])`.
- `http_api` / `ble_api` / `command.rs` call it.
- `paths.rs`: `get("motor.bpm")` / `set("pin.modbus_tx", "2")` used by serial CLI.
- `Events.persist_*` wired to existing `storage_task` (debounce logic stays in esp32).
- RPC golden tests in core (ping, stale version, rtu_relay rejection).

**Exit**

- `scripts/test_websocket.py` on device
- `scripts/test_ble.py` GATT + RPC (if BLE enabled)
- Serial CLI `get motor` / `set motor.bpm` / `get-state` (full JSON, no 256 B truncate)

---

### Sprint 4 — `ossm-std` desktop shell

**Goal:** Linux binary speaks the same HTTP/WS API and drives a real 57AIM30 over `tokio-serial`.

**Work**

- `aim30.rs` encode/decode extracted if not done in sprint 1–2.
- Motor loop task + atomic `config.json`.
- axum REST + `/ws/command` (same methods).
- Bind `127.0.0.1:80` or `:8080`; document `DEVICE_IP` override for UI.
- Homing + pause default.

**Exit**

- `cargo +stable run -p ossm-std -- --help`
- Without hardware: mock serial (PTY) + `tick` publishes `/state`
- With hardware: frontend at `http://127.0.0.1:8080` moves the motor
- Kill -9 during a config write leaves either old or new `config.json`, never a half write

**Out of scope:** BLE, USB console fairness, UHCI debug inject (optional: inject still in core RAM for protocol tests).

---

### Sprint 5 — `ossm-wasm` + JS bridge

**Goal:** `wasm-bindgen` engine runs in Chromium; WebSerial optional.

**Work**

- `OssmEngine` exports from §3.4.
- `wasm-pack build crates/ossm-wasm --target web`.
- Harness page: construct engine, `dispatch_rpc` ping/get-state/set-config, `tick` 100 times, assert snapshot `version` and `paused`.
- WebSerial path: reuse patterns from `scripts/test_frontend.py` `WebSerialBridge` / flasher (disconnect must end the stream — already tested).
- Persist hook to `localStorage`.

**Exit**

- `wasm-pack build` succeeds on stable
- Playwright (new `scripts/test_wasm_engine.py` or a frontend `--wasm` flag): ping + set-config version + tick
- Optional live: WebSerial to the same 57AIM30

---

### Sprint 6 — Relay mode, docs, CI

**Goal:** Feature parity gaps closed; workspace is the default developer path.

**Work**

- `rtu_relay` on `ossm-std` (TCP `:502` + `/ws/modbus`) using core codec; wasm relay optional.
- CI: `ossm-core` tests on stable; firmware build job unchanged (`esp`).
- README / README.zh / AGENTS.md: workspace commands, `ossm-std` flags, wasm build.
- Delete leftover shims (`pub use` re-exports in esp32 that exist only for old paths).

**Exit**

- `scripts/test_modbus_tcp.py` / `test_modbus_relay.py` against `ossm-std` **or** device
- Docs match actual crate paths
- `release.sh` still flashes C6 and S3

---

## 9. Suggested order inside a sprint

1. Extract / compile (`check` / `test -p ossm-core`).
2. Point firmware at the new API (thin adapters).
3. `cargo b-c6 --release` + device smoke (`get-state`, pause).
4. Only then grow std/wasm.

Never land a core API change without an `ossm-esp32` compile in the same PR.

---

## 10. Verification matrix

| Check | Sprint | Command / method |
| --- | --- | --- |
| Core unit tests | 1–3 | `cargo +stable test -p ossm-core` |
| Modbus resync | 1 | Rust tests + `scripts/test_modbus_resync.py` |
| C6/S3 release image | 0, every firmware PR | `./scripts/release.sh` |
| HTTP + WS + UI | 2–3 | `scripts/test_frontend.py` / `test_websocket.py` |
| BLE | 3 | `scripts/test_ble.py` |
| Motor `dt_max` | 2 | C6 + concurrent HTTP; `< 4.5 ms` |
| Desktop API | 4 | curl `/state`, WS ping |
| Atomic config | 4 | kill during POST `/config` |
| Wasm bindgen | 5 | `wasm-pack build` + Playwright |
| Relay | 6 | `test_modbus_tcp.py` |

---

## 11. Open decisions (resolve in sprint 0 or 4)

1. **std HTTP port:** `8080` (no root) vs `80` (firmware parity). Recommend **8080** + frontend origin detection.
2. **std executor rate:** busy-poll `tick` vs wait on serial. Recommend **select**: serial RX, interval 3 ms, command channel.
3. **Wasm in the shipped firmware UI:** later; sprint 5 is a standalone harness so firmware gzip size does not grow.
4. **`Motor` trait** (`src/motor.rs`): drop from core; keep as a private async trait in `ossm-esp32` if it still documents the 57AIM30 shell, or delete if unused after IoC.
