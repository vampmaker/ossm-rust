# ESP-HAL Migration: Overview & Execution Order

## Goal

Migrate the OSSM Rust firmware from `esp-idf-svc` (std, FreeRTOS) to `esp-hal` + `esp-rtos` (no_std, bare-metal Embassy) to achieve deterministic motor timing via **interrupt-priority execution** on a dedicated `InterruptExecutor`.

## Core Architectural Guarantee: Eliminating Motor Interruption by WiFi / HTTP / WebSocket

A primary pain point of the `esp-idf-svc` (FreeRTOS) architecture is that high-priority WiFi radio interrupts (`lwip`, IPC ISRs) and HTTP/WebSocket handlers stall the motor control loop. This migration plan solves that pain point completely through three structural pillars:

1. **Strict Execution Priority Isolation**:
   - On **ESP32-C6 (Single-Core)**: The motor loop (`motor_task`) executes on an `InterruptExecutor` bound to software interrupt `software_interrupt1` at **Priority 3** (above normal thread level). WiFi radio tasks, `embassy-net` TCP/IP handling, and `picoserve` HTTP/WebSocket handlers run on the normal Thread Executor (**Priority 0**). Whenever a motor step is due, the hardware interrupt **instantly preempts** any running HTTP request, JSON serialization, or network activity.
   - On **ESP32-S3 (Dual-Core)**: **Core 0** runs all WiFi radio ISRs, HTTP/WebSocket servers, BLE GATT, and NVS tasks. **Core 1** runs `motor_task` exclusively. There is zero CPU preemption between WiFi/HTTP packet processing and motor steps.
2. **Strict Data Ownership & Lock-Free I/O**:
   - The Modbus UART driver (`Modbus57AIM30Motor`) is owned exclusively on the `motor_task` stack. Network tasks never touch Modbus hardware.
3. **Sub-Microsecond Mutex Discipline**:
   - `AppContext.motor_controller` is protected by `CriticalSectionRawMutex`. When HTTP/WebSocket endpoints access state (`get-state`, `set-config`), the lock is held for **< 2 µs** of pure CPU float math or struct copying. All heavy work—including JSON serialization (`serde_json`), memory allocation, and socket writing—is performed strictly **outside** the lock.

## Architecture Summary

```
┌───────────────────────────────────────────────────────────────┐
│                    esp-rtos scheduler                         │
├───────────────────────────────────────────────────────────────┤
│  Interrupt-Level Executor (SW interrupt, high priority)        │
│  ┌──────────────────────────────────────┐                    │
│  │ motor_task: Modbus UART at ~250 Hz   │                    │
│  │   compute_cycle() under brief lock   │                    │
│  │   write_position/cycle() outside lock│                    │
│  └──────────────────────────────────────┘                    │
├───────────────────────────────────────────────────────────────┤
│  Thread-Level Executor (main thread, #[esp_rtos::main])      │
│  ┌────────────┐ ┌─────────────┐ ┌────────────┐              │
│  │ wifi_task   │ │ http_tasks  │ │ nvs_saver  │              │
│  └────────────┘ └─────────────┘ └────────────┘              │
│  ┌────────────┐ ┌─────────────┐ ┌────────────┐              │
│  │ ble_runner  │ │ ble_gatt    │ │ cli_task   │              │
│  └────────────┘ └─────────────┘ └────────────┘              │
├───────────────────────────────────────────────────────────────┤
│  ESP32-S3 Only: Core 1 (blocking motor executor)             │
│  ┌──────────────────────────────────────┐                    │
│  │ motor_task pinned to Core 1          │                    │
│  │ (InterruptExecutor or block_on loop) │                    │
│  └──────────────────────────────────────┘                    │
├───────────────────────────────────────────────────────────────┤
│  Shared radio (WiFi + BLE coexistence)                        │
│  esp_radio::init() once in main → WifiController + BleConnector│
└───────────────────────────────────────────────────────────────┘
```

## Dependency Map: Current → Target

| Current (esp-idf/std) | Target (esp-hal/no_std) | Plan |
|---|---|---|
| `esp-idf-svc` 0.52 | `esp-hal` ~1.1 + `unstable` | [01-build-system](01-build-system.md) |
| `esp-idf-sys` 0.37 | removed | [01-build-system](01-build-system.md) |
| `embuild` 0.33 | removed | [01-build-system](01-build-system.md) |
| `esp-idf-svc` WiFi | `esp-radio` 1.0.0-beta.0 (`wifi`, `ble`, `coex`) | [04-wifi-networking](04-wifi-networking.md) |
| `esp-idf-svc::mdns::EspMdns` | Minimal mDNS responder on embassy-net UDP | [04-wifi-networking](04-wifi-networking.md) |
| `esp32-nimble` 0.12 | `trouble-host` 0.6 + `bt-hci` 0.9 + `BleConnector` | [06-ble-gatt](06-ble-gatt.md) |
| `esp-idf-svc::nvs::EspNvs` | `esp-nvs` + `esp-storage` | [07-storage](07-storage.md) |
| `edge-http` + `edge-nal-std` | `picoserve` (embassy) | [05-http-websocket](05-http-websocket.md) |
| `edge-ws` | `picoserve` ws feature | [05-http-websocket](05-http-websocket.md) |
| `embassy-executor` (platform-std) | thread executor + `InterruptExecutor` + `esp-rtos` 0.3 | [02-entry-point](02-entry-point.md) |
| `embassy-time` (generic-queue-8) | `embassy-time` (esp-rtos driver) | [02-entry-point](02-entry-point.md) |
| `std::sync::Mutex` + `Arc` | `embassy_sync::Mutex` + `&'static` | [09-shared-state](09-shared-state.md) |
| `std::thread` | `esp-rtos` threads / embassy tasks | [02-entry-point](02-entry-point.md) |
| `anyhow` | custom error enum | [01-build-system](01-build-system.md) |
| `log` + `EspLogger` | `esp-println` + `log` | [10-logging-panic](10-logging-panic.md) |

## Execution Order (Phased)

### Phase 0: Risk Spike (before full migration)

Validate the three highest-risk integrations on hardware before rewriting subsystems:

1. **InterruptExecutor + motor UART** — confirm ~250 Hz loop with WiFi running
2. **WiFi + BLE coexistence** — `esp_radio::init()` + `coex` feature, both stacks alive
3. **RS-485 manual DE/RE** — esp-hal has no `RS485HalfDuplex` mode; verify Modbus with manual pin toggle

### Phase 1: Build System, Storage & Entry Point

1. [01-build-system.md](01-build-system.md) — Cargo.toml, .cargo/config.toml, rust-toolchain.toml, build.rs
2. [07-storage.md](07-storage.md) — esp-nvs + esp-storage (needed before WiFi reads credentials)
3. [02-entry-point.md](02-entry-point.md) — `#![no_std]`, `#[esp_rtos::main]`, InterruptExecutor, radio init
4. [10-logging-panic.md](10-logging-panic.md) — esp-println, esp-backtrace, log crate

### Phase 2: Motor Control & Shared State (Highest Value)

5. [09-shared-state.md](09-shared-state.md) — embassy-sync primitives, motion command queue
6. [03-motor-uart.md](03-motor-uart.md) — esp-hal UART async, Modbus RTU, motor loop

### Phase 3: Networking

7. [04-wifi-networking.md](04-wifi-networking.md) — esp-radio WiFi, embassy-net, mDNS
8. [05-http-websocket.md](05-http-websocket.md) — picoserve HTTP/WebSocket, JSON-RPC, `src/rpc.rs`

### Phase 4: BLE

9. [06-ble-gatt.md](06-ble-gatt.md) — trouble-host GATT server (uses shared `BleConnector`)

### Phase 5: CLI & Polish

10. [08-serial-cli.md](08-serial-cli.md) — USB Serial JTAG async, embedded-cli

### Phase 6: Validation & Release

11. [11-testing-release.md](11-testing-release.md) — Test strategy, release script updates, binary size

## Files Changed by Migration

| File | Changes |
|---|---|
| `src/motion.rs` (~1400 lines) | `std::time::Instant` → `embassy_time::Instant`; `libm` for all f32 math (`sinf`, `cosf`, `asinf`, `sqrtf`); `Arc<Mutex<Producer>>` → `&'static Mutex<Producer>` for command queue (see [09-shared-state](09-shared-state.md)) |
| `src/motor.rs` (trait) | `anyhow::Result` → `crate::error::Result` |
| `src/motor_57aim30.rs` | UART driver swap; motor loop preserved |
| `src/main.rs` | Complete rewrite of init; motor loop logic preserved |
| `src/rpc.rs` | **New** — shared JSON-RPC dispatch for HTTP WS and BLE |
| `src/error.rs` | **New** — replaces `anyhow` |
| `frontend/` | No firmware deps (rebuild before flash) |
| `flasher/` | No firmware deps |
| `scripts/` | Protocol-level tests; should work unchanged if APIs preserved |

## Critical Build Constraints

1. **Optimization level**: `esp-radio` vendor blobs **require** `opt-level = 2` or `3`. Use `[profile.dev.package.esp-radio] opt-level = 3` so the rest of dev builds stay fast; release uses `opt-level = 2`.
2. **Heap**: esp-radio recommends **two allocators** (~64 KB reclaimed + ~36 KB). A single 96 KB heap is likely insufficient for WiFi + BLE + `serde_json`.
3. **Scheduler before radio**: `esp_rtos::start(timer, sw_interrupt)` must be called **before** `esp_radio::init()` or any `WifiController::new()` / `BleConnector::new()`.
4. **WiFi + BLE coexistence**: Enable `coex` feature on `esp-radio`. Call `esp_radio::init()` once in `main()`, then pass the returned controller to both `WifiController::new()` and `BleConnector::new(&ctrl, peripherals.BT, ...)`.
5. **Async driver core affinity**: esp-hal async drivers cannot be sent between cores. On ESP32-S3, UART must be initialized on Core 1.
6. **Entry point macro**: Use `#[esp_rtos::main]` (not `#[esp_hal::main]`). Do **not** enable `arch-*` features on `embassy-executor`.
7. **f32 math**: `no_std` lacks `f32::sin()` etc. Use `libm` (`sinf`, `cosf`, `asinf`, `sqrtf`).
8. **RS-485**: esp-hal 1.1 has no `RS485HalfDuplex` UART mode. Use manual DE/RE GPIO toggle (see [03-motor-uart](03-motor-uart.md)); validate on hardware.
9. **Interrupt executor for motor**: Motor task must run on `esp_rtos::embassy::InterruptExecutor`, not the thread executor, to achieve the jitter improvement that motivates this migration.

## Risk Mitigation

| Risk | Mitigation |
|---|---|
| `esp-radio` 1.0.0-beta.0 | Pin exact version; run Phase 0 coex spike early |
| WiFi + BLE simultaneous | `coex` feature + single `esp_radio::init()`; prototype before full BLE port |
| `trouble-host` less mature than NimBLE | Run `test_ble.py` in Phase 4; handle MTU limits for large state |
| No NVS backward compat | Use `esp-nvs` crate which reads ESP-IDF NVS format |
| RS-485 regression | Manual DE/RE timing; hardware validation in Phase 0 |
| `serde_json` heap pressure | Dual heap allocators; consider `serde-json-core` later |
| Dual-target (C6 + S3) | esp-hal supports both; C6 uses InterruptExecutor, S3 uses Core 1 pinning |
| Motion command queue | Keep `heapless::spsc::Queue`; replace inner `Arc<Mutex<Producer>>` with `&'static Mutex` |
