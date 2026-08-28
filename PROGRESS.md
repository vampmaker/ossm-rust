# Sprint progress

Update after **every** numbered step. Do not batch.

| Step | Status | Notes |
| --- | --- | --- |
| 0.1–1.4 | **done** | Workspace + DTO/modbus extraction |
| 2.1 time + Command/Events/CoreError | **done** | |
| 2.2 Split motion into core; tick(now) | **done** | |
| 2.3 Engine + host tests | **done** | 8 tests on stable |
| 2.4 Wire run_motor apply/tick | **done** | |
| 2.5 Sprint 2 verify | **done** | host tests + C6 release; device E2E skipped |
| 3.1 PROGRESS + serde-json-core | **done** | |
| 3.2 Remove persist_*; apply → () | **done** | |
| 3.3 Core rpc.rs dispatch_rpc | **done** | |
| 3.4 Core paths.rs | **done** | |
| 3.5 ESP32 RPC/CLI adapter | **done** | |
| 3.6 Host tests + C6 compile | **done** | 12 tests; device E2E skipped |
| 4.1 PROGRESS + workspace | **done** | |
| 4.2 aim30 pack/unpack | **done** | |
| 4.3 clap/env Args | **done** | |
| 4.4 Atomic persist | **done** | |
| 4.5 engine_task + axum | **done** | |
| 4.6 Frontend origin + verify | **done** | host-only |

## Log

### 0.1 Create PROGRESS.md

- Result: pass
- Created this checklist.

### 0.2 Workspace + empty `ossm-core`

- Result: pass
- Root `Cargo.toml` is a workspace (`ossm-core`, `ossm-esp32`).
- `crates/ossm-core` is `no_std` with no deps yet.

### 0.3 Move firmware to `crates/ossm-esp32`

- Result: pass
- `git mv` `src/`, `build.rs`, `ld/` into `crates/ossm-esp32/`.
- Package name `ossm-esp32`; bin still `ossm-rust`; lib still `ossm_rust`.
- `build.rs` reads `frontend/dist/index.html` via `CARGO_MANIFEST_DIR/../../frontend`.

### 1.1 Core modules + deps

- Result: pass (sources landed)
- Added `config.rs`, `state.rs`, `rpc_types.rs`; moved codec to `modbus.rs`.
- Deps: serde/serde_json alloc, rmodbus, fixedvec, portable-atomic.

### 1.2 Point firmware at core

- Result: pass (rewire done; compile in 1.4)
- Firmware re-exports core types; `pub use ossm_core::modbus as modbus_rtu`.
- Removed `ossm-esp32` lib crate.

### 1.3 Codec tests + Python docstring

- Result: pass
- `scripts/test_modbus_resync.py` docstring points at `crates/ossm-core/src/modbus.rs`.
- Rust in-module tests already cover exact / trailing / leading resync / bad CRC.

### 1.4 Sprint 1 verify

- `cargo +stable test -p ossm-core` — pass (4 tests). Fixed leading-junk CRC to match Python (CRC over payload only).
- `uv run scripts/test_modbus_resync.py` — pass
- `cargo b-c6 --release` — pass (no warnings)

### 2.1 time + Command/Events/CoreError

- Result: pass (types landed)
- Added `time.rs`, `error.rs`, `command.rs`, `event.rs`; `libm` + `log` deps.

### 2.2 Split motion into core; tick(now)

- Result: pass
- `ossm-core/src/motion/{waveform,source,shaper,controller}.rs`; `tick(now: Micros)`; no SPSC.

### 2.3 Engine + host tests

- Result: pass (`cargo +stable test -p ossm-core`, 8 tests)

### 2.4 Wire run_motor apply/tick

- Result: pass (rewire done; compile in 2.5)
- Thin `crates/ossm-esp32/src/motion.rs` keeps SPSC types; `run_motor` uses `Engine`.

### 2.5 Sprint 2 verify

- `cargo +stable test -p ossm-core` — pass (8 tests)
- `cargo b-c6 --release` — pass (no warnings)
- Device `dt_max_ms` / `test_websocket.py` — skipped (compile-only this session)

### 3.1 PROGRESS + serde-json-core

- Result: pass
- Sprint 3 checklist rows added.
- `ossm-core` depends on `serde-json-core` 0.6 for RPC `to_slice` into caller buffers.

### 3.2 Remove persist_*; apply → ()

- Result: pass
- Dropped `Events` / `persist_*`. `Engine::apply` returns `()`. Flash stay in ESP32 `StorageHandle` / `nvs_saver_task`.

### 3.3 Core rpc.rs dispatch_rpc

- Result: pass (sources landed)
- Sync `dispatch_rpc(&mut Engine, &[u8], &mut [u8])` plus `write_result` / `write_error` / notification helpers.

### 3.4 Core paths.rs

- Result: pass (sources landed)
- `PATHS_CATALOG`, `get`/`set` on Engine, field helpers for CLI.

### 3.5 ESP32 RPC/CLI adapter

- Result: pass (rewire done; compile in 3.6)
- Firmware `rpc.rs` encodes via core helpers; CLI uses `paths` catalog/field helpers. HTTP/BLE still enqueue via `AppContext`.

### 3.6 Host tests + C6 compile

- `cargo +stable test -p ossm-core` — pass (12 tests: ping, stale version, rtu_relay, path get/set)
- `cargo b-c6 --release` — pass (no warnings)
- Device `test_websocket.py` / `test_ble.py` / serial CLI — skipped (compile-only this session)

### 4.1 PROGRESS + workspace

- Result: pass
- Workspace members include `crates/ossm-std`; `default-members` remains `ossm-esp32`.

### 4.2 aim30 pack/unpack

- Result: pass (sources landed)
- `ossm-core/src/modbus/aim30.rs`; firmware position R/W uses pack/unpack.

### 4.3 clap/env Args

- Result: pass
- Flags override env (including `.env` `DEVICE_PORT` / `DEVICE_BAUD`); env overrides defaults.

### 4.4 Atomic persist

- Result: pass
- `config.json.tmp` + `sync_all` + `rename`; tests cover replace and crash-before-rename.

### 4.5 engine_task + axum

- Result: pass
- Mock tick publishes `/state`; WS/RPC `ping` via engine oneshot; REST matches firmware routes.

### 4.6 Frontend origin + verify

- `frontend/src/api.ts`: `ossm.lan` only when `import.meta.env.DEV`.
- `cargo +stable test -p ossm-core` — pass
- `cargo +stable test -p ossm-std` — pass (persist + mock `/state` + ping)
- `cargo +stable run -p ossm-std -- --help` — pass
- `cargo b-c6 --release` — pass
- Live 57AIM30 / Playwright — skipped (host-only this session)

