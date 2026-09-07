# OSSM-Rust — Developer Guide

Design intent, implementation choices, environment setup, and the full API reference.

For assembly, flashing, and day-to-day operation, see the [user manual](README.md). This document assumes you have a repository checkout and are comfortable with Rust, Cargo, and a terminal.

**Contents**

1. [Architecture](#1-architecture)
2. [Design intent](#2-design-intent)
3. [Implementation choices](#3-implementation-choices)
4. [Environment setup](#4-environment-setup)
5. [Building and running](#5-building-and-running)
6. [Testing](#6-testing)
7. [API reference](#7-api-reference)
8. [Changing the API surface](#8-changing-the-api-surface)

---

## 1. Architecture

One motion engine, three shells. The engine is the same code in all three; the shell supplies the clock, the serial bytes, and the network.

```
        shell (esp32 | std | wasm)
        owns: clock, UART/serial, HTTP/BLE/CLI, flash/filesystem, loop/link stat windows
                 │
                 │  synchronous calls — no .await crosses this line
                 ▼
        ossm-core   Engine::apply / tick(now) / snapshot
                    CRC / find_modbus_response / aim30 registers
                 ▲
        ossm-common   LoopStatsWindow / LinkStatsWindow / Pll (caller supplies micros)
```

### Crates

| Crate | Target | Role |
| --- | --- | --- |
| `ossm-common` | `no_std` + `alloc`, stable Rust | Telemetry aggregators, wire structs (`LoopStats`, `LinkStats`), and BBR-style `Pll`. No clock; shells call `record` / `poll` then `SetLoopStats` / `SetLinkStats`, and `next_wake` / `on_tx` / `on_ack`. |
| `ossm-core` | `no_std` + `alloc`, stable Rust | Motion state machine, telemetry **schema** (`StateResponse.loop_stats` / `link_stats`), JSON-RPC and CLI path catalog, Modbus RTU byte math (CRC, frame resync, 57AIM30 register literals). No I/O of any kind. |
| `ossm-esp32` | ESP32-C6 (RISC-V) / ESP32-S3 (Xtensa), `esp` toolchain | Device firmware. `esp-hal` + Embassy + `esp-rtos`. Owns GPIO, UART, WiFi, BLE, NVS, USB console. |
| `ossm-std` | Linux / Windows / macOS / Termux, stable Rust | Desktop process. tokio + axum. Host serial (kernel tty or userspace USB-host) is the motor bus. |
| `ossm-wasm` | `wasm32-unknown-unknown`, stable Rust | Browser page thread. Web Serial or a firmware `/ws/rs485` socket is the motor bus. |

### Web workspace (`web/`)

npm workspace, one Vite app per shipped HTML file.

| Package | Output | Notes |
| --- | --- | --- |
| `apps/webui-esp32` | `dist/index.html` → gzipped into firmware ROM | Full UI: motion, WiFi, BLE, pins, network |
| `apps/webui-std` | `dist/index.html` → compiled into the `ossm-std` binary | Motion only |
| `apps/webui-wasm` | `release/ossm-wasm.html` | Harness for the wasm page shell; not embedded |
| `apps/flasher` | `release/flasher.html` | Browser esptool + device configurator |
| `apps/motor-control` | `release/motor-control.html` | 57AIM30 vendor-style Modbus tool; not embedded |
| `packages/shared` | `@ossm/shared` | Vue components (`MotionWorkbench`, `MacroPlayer`, `SplineEditor`), wire↔domain mapper, i18n |
| `packages/client` | `@ossm/client` | WiFi / BLE transports and the WebSocket state machine |

### Scripts (`scripts/`)

A [uv](https://docs.astral.sh/uv/) project rooted at `pyproject.toml`. Every script has the shebang `#!/usr/bin/env -S uv run`, so `./scripts/ossm.py …` works after a single `uv sync`. `scripts/ossm.py` doubles as an importable library: it exports Pydantic v2 schemas (`MotorControllerConfig`, `StateResponse`, `PinConfiguration`, `NetworkConfiguration`) and the `DeviceBackend` class used by every other script.

---

## 2. Design intent

### The domain crate does no I/O

`ossm-core` is `no_std` + `alloc` and contains no `.await`, no files, no GPIO, no sockets, and no `std`. The caller passes the current time in as a `Micros` value. This is what lets bare-metal firmware, a tokio process, and a browser tab share byte-identical motion behaviour, and it is why the engine can be unit-tested on the host with `cargo +stable test -p ossm-core`.

The boundary is drawn at *domain*, not at *motion*. Modbus CRC, frame resynchronisation, and the 57AIM30 register literals live in core too, because they are pure byte math with one correct answer. UART timing, DE/RE direction, and DMA buffers do not, because they are properties of a particular bus.

The corollary: `Engine::apply` returns `()`. Persisting a config change is the shell's problem, and each shell solves it differently (NVS on the ESP32, an atomic JSON file on the desktop, IndexedDB in the browser).

### The engine is synchronous; the shells are not

`Engine::apply(cmd)` and `Engine::tick(now)` are plain synchronous calls that take `&mut Engine`. Only one task may hold that reference, and it is always the task that owns the motor bus.

Everything else — HTTP handlers, WebSocket sessions, BLE GATT callbacks, the serial CLI — reaches the engine by enqueuing a command, never by borrowing it. On the firmware this is a capacity-3 SPSC queue (`crates/ossm-esp32/src/motion.rs`); on the desktop it is a bounded tokio mpsc plus a wake signal into the dedicated `ossm-motor` std thread (`EngineHandle::send`). That is also why there are two RPC dispatchers: `ossm-core::rpc` is the synchronous one that mutates the engine, and `crates/ossm-esp32/src/rpc.rs` is a thin async adapter that enqueues and then encodes the reply using core.

### Config writes carry a causal version

`MotorControllerConfig` has a monotonic `version: u32`. Every accepted write bumps it. A write whose `version` is older than the current one is rejected — `409 Conflict` over REST, JSON-RPC error `-32001` over WebSocket.

This exists because the UI streams state at up to 30 FPS while the user drags a slider. Without a causal version, an in-flight background snapshot would land after the optimistic local edit and visibly revert the slider. The frontend tracks `authoritativeVersion` in `connectionStateMachine.ts` and drops any push that regresses.

The practical rule when writing firmware or desktop code: never assign `self.config.field = …` directly. Go through `set_config` / `commit_config` / `try_enqueue_config` so the version bump and the snapshot publish happen together.

### The device boots paused, and real hardware always homes

`MotorControllerConfig::default().paused` is `true`, and the post-homing `sync_to_position` forces pause again. A machine that starts stroking when power is applied is a safety problem, so there is no configuration that changes this.

Homing is not optional on a real bus. `pos_min` / `pos_max` are the *mechanical* travel limits of whatever the motor is bolted to, found by commanding each extreme and reading back where motion actually settles (`homing()` in each shell). Those limits change whenever the machine is rebuilt or the drive is re-referenced, so they are never cached in flash. `--mock` (desktop) and the browser mock skip homing only because they have no motor; they use a virtual `0..100` range. Switching an ESP32 into `servo` mode at runtime re-homes for the same reason.

There are two physical invariants. They are different jobs; do not fold one into the other.

**Homing power.** Travel homing commands ±100 rad so the 57AIM30 stalls on the mechanical ends. That is safe only while holding register `REG_POWER` (`0x18`) is `POWER_HOMING` (0.1, encoded 60). `POWER_RUN` (0.6, encoded 360) is written only after `homing()` returns `Ok`. `REG_POWER` lives on the drive: it survives ESP reset, UART re-init, and `servo` ↔ `rs485`. Every shell (firmware, `ossm-std`, `ossm-wasm`) must write `POWER_HOMING` and **read it back** before the first position seek (`reset_position` / ±100). An abort that leaves the last ±100 target is acceptable at homing power — the stall is the homing method. After a successful home, `PositionGenerator` maps `y ∈ [0, 1]` into the measured `[pos_min, pos_max]`; that bounds motion, not homing. Literals are in `ossm-core` `modbus/aim30.rs`.

**Command continuity.** After homing, the engine command must stay continuous in time: a config write must not teleport position or command unbounded speed. Homing ±100 seeks are the first model, not a continuity violation. This is an `ossm-core` rule (`motion/controller.rs`, `motion/shaper.rs`); shells must not write raw FC16 except during travel homing. `tick` clamps `dt` to 12 ms so a stalled loop cannot become a large step. Depth / `depth_top` lerp the shaper window (`TRANSITION_SPEED`). `reversed` is an involution (`y := 1 − y`) so shaped pose holds. Waveform / BPM / spline changes recreate the source then `follow(last_y, last_speed)` (periodic `follow` matches phase only — a sine cannot take an arbitrary velocity at a given `y`). Pause slews at `PAUSE_SPEED` from `last_y`. Streaming enters via `follow` and catch-up-schedules the first waypoint. `HomingComplete` rematches the encoder (or slews `held_shaped`); it never `follow(0.5)`. Tests in `crates/ossm-core/src/engine_tests.rs`.

### One owner per bus

RS-485 is half-duplex and there is exactly one transceiver. On the ESP32, UART1 has three mutually exclusive roles — `servo` (the engine drives the motor), `rtu_relay` (framed Modbus TCP + `/ws/modbus`), and `rs485` (a raw byte pipe on USB and `/ws/rs485`) — arbitrated by a single supervisor task (`uart_owner.rs`) that stops the current role and re-initialises the peripheral. Roles switch live; no reboot. On the desktop the same rule appears as `--mode servo` versus `--mode rtu-relay`.

The relay modes exist so that a PC tool (`motor-control.html`, or another `ossm-std`) can talk to the servo *through* the device without either side fighting over the bus.

---

## 3. Implementation choices

These are the non-obvious decisions. Most of them were forced by hardware behaviour and are expensive to rediscover.

### Firmware (`ossm-esp32`)

**Heap and snapshot publishing.** The heap is 96 KiB and WiFi plus BLE already claim most of it. Telemetry snapshots are published *in place* via `Arc::make_mut` / `copy_from`; a fresh `Arc::new(snapshot.clone())` on every motor cycle exhausts the heap within seconds. When the current `Arc` is shared, the replacement is allocated outside the critical section.

**Loop timing.** `compute_cycle()` clamps the step `dt` to 12 ms so that a preemption spike cannot translate into a servo jerk. Shells aggregate one-second window statistics into `StateResponse.loop_stats` (`ups`, `dt_min_ms`, `dt_max_ms`, `dt_avg_ms`, `dt_mdev_ms`) via `ossm-common::LoopStatsWindow`. The hard budget on ESP32-C6 under concurrent HTTP and WebSocket load is `loop_stats.dt_max_ms < 4.5`; `scripts/stress_dt_max.py` asserts it. Bus RTT and exchange counters live in `link_stats`, filled by each shell's `LinkStatsWindow`.

**Task placement.** On the C6 the UART1 owner runs on an `InterruptExecutor` at elevated priority; on the S3 it runs on Core 1 via `esp_rtos::start_second_core`, with its ~24 KiB stack placed in `.dram2_uninit`. Console, CLI, network, and BLE stay on the main executor / Core 0.

**Modbus over GDMA.** RS-485 uses the `UHCI0` + `DMA_CH0` GDMA path wrapping `esp_hal::uart::Uart`, not CPU FIFO reads. A two-phase CPU read (header, then body) leaves a re-arm gap in which bytes are lost at poll rates above ~330 Hz; DMA captures continuously across that gap. Defaults at 115200 baud: 750 µs RX inter-byte timeout (t1.5), 350 µs inter-frame quiet (t3.5, minus the ~174 µs the hardware already spent in `timeout_symbols`), 10 ms whole-frame timeout. End-to-end cycle stays under 3 ms, which is what makes >330 Hz `ups` possible.

**Console, not `esp-println`.** All runtime logging goes through `console.rs`, which owns USB Serial/JTAG, UART0, and RTT. `esp-println`'s `jtag-serial` backend latches a sticky `TIMED_OUT` when USB TX cannot drain, permanently silencing later writes — so `esp-println` is linked only to satisfy the `esp-backtrace` `println` feature gate, and its `log` feature is never enabled. USB TX is commit-then-wait: bytes already handed to the 64-byte IN FIFO are never rewritten. With no ACM reader attached the console marks itself stalled and drops the log ring rather than stuffing the endpoint, while CLI replies stay on a separate priority queue (`CLI_OUT_CH`) so an attached tool still gets its ACKs. RX stays armed while TX drains, which is what makes late attach work.

**Panics.** `#[panic_handler]` lives in `console.rs` and prints through `panic_write` + `esp_backtrace::arch::backtrace()`. `esp-backtrace`'s own `panic-handler` / `exception-handler` features are off (duplicate symbols). RISC-V needs `-C force-frame-pointers` for the frame walk; it is set in `.cargo/config.toml`.

**HTTP.** `edge-http` with four signal-gated acceptors and up to three concurrent WebSocket sessions on `StackResources<20>`. `REST_GATE` serialises mutating POSTs *only* — holding it across GETs or `/state` starves the acceptors and makes port 80 look dead under burst. WebSocket sessions end with `drop(socket)`, not `close(Both).await`, which faults in smoltcp after a peer or WiFi teardown.

**BLE.** `trouble-host`. Never `.await` or mutate the attribute table between a `GattEvent::Read`/`Write` and its `accept()` — an outstanding ATT request plus an await hangs the controller and starves WiFi. `CHAR_STATE` carries a compact ≤256 B telemetry subset so a plain ATT Read works without Read Blob; the full `StateResponse` is only available through JSON-RPC on `CHAR_RPC`. The BLE task stack must be at least 10 KiB (the 4 KiB default overflows into an instruction-access fault). BLE starts after WiFi and HTTP are up, and advertises at 250–500 ms to avoid contending with WiFi. If the runner exits, it recovers in-process; it never triggers a chip reset.

**Flash.** Erase and write starve the 2.4 GHz radio, so all flash I/O funnels through the single `storage_task` actor. Motor config persistence is debounced by at least 2 s and ignores pause-only changes, so a UI pause toggle never hits NVS.

**Linker.** ESP32-S3 uses a local `ld/esp32s3/linkall.x` + `ossm-rodata.x` that merges the `.flash.appdesc` alignment padding into a single DROM PROGBITS section; without it the bootloader rejects the image with *multiple DROM segments*.

### Desktop (`ossm-std`)

**No RTU timing.** A PC USB-serial stack cannot honour t1.5 / t3.5, so `modbus_rx.rs` treats RX as an unbounded stream: accumulate, parse the header length, validate CRC, emit, keep the leftover. The 57AIM30 register literals come from `ossm-core::modbus::aim30`, identical to the firmware.

**Direction control is the adapter's job.** `ossm-std` does not strip TX echoes (an FC06 ACK is byte-identical to the request, so echo stripping is ambiguous). USB-RS485 adapters must therefore switch direction automatically; TX-loopback adapters and host-driven DE/RE do not work. The ESP32 `rs485` pipe is fine because the firmware drives DE itself.

**Two serial backends.** `/dev/tty*` goes through `tokio-serial`. `/dev/bus/usb/…` goes through `usb_host.rs` (`android-usb-serial`, covering CH340 / CDC / CP210x / FTDI / PL2303) for Termux and Linux usbfs; when `TERMUX_USB_FD` is unset it re-execs itself under `termux-usb -r -e`. The motor path deasserts DTR/RTS. `--mode flash` and `--mode console` use `flash_port.rs` instead of the Modbus parser: flash drives the ESP ROM loader (USB-Serial-JTAG reset, SLIP, compressed write); console is a UART CLI (`--send`, `--until`, `--exit-rs485`). Do not enable `android-usb-serial`'s `serialport-compat` feature (it pulls libudev and breaks the musl zigbuild).

**Transmit scheduling.** Motor TX is paced by `ossm_common::Pll` on every bus: probe-freq measures max ACK rate, then probe-phase snaps the period (USB FS 1 ms slot) and hill-climbs TX delay to cut RTT. USB-host streaming still uses latest-wins plus a 6 ms stream CRC deadline. Homing stays blocking. The motion loop itself runs on a dedicated `ossm-motor` OS thread (`motor_thread.rs`), not a tokio worker, so HTTP/WS serialization cannot inflate `dt_max_ms`. Wakes go through `precise_wait`: Windows uses `CreateWaitableTimerExW` with `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION` (falling back to `timeBeginPeriod(1)`) so a 2.5 ms PLL period is not quantized to the 15.625 ms system tick; Unix uses a Condvar/futex and sets 10 µs `PR_SET_TIMERSLACK` on that thread. HTTP and the CLI reach the engine only through `EngineHandle::send` (try-send + wake).

**Persistence.** `{ motor }` JSON written temp → fsync → rename. Leftover `pin` / `net` keys from a firmware config file are ignored.

### Browser (`ossm-wasm`)

The engine, homing, and RTU I/O all live on the **page thread** in `web/apps/webui-wasm` (`WasmClient`). Vue talks only `OssmControl` and never sees an RTU frame. `requestPort()` stays on the page (it needs a user gesture); the resulting `SerialPort` is opened in-process (DTR/RTS off). Playwright uses a `WebSerialBridge` mock of `navigator.serial` on the same thread.

### Frontend

`WsDataManager` (`packages/client/src/client.ts`) is a single-owner WebSocket client: one connection, promise-routed JSON-RPC replies, an application-level watchdog for silent failures, and a reconnect loop that re-subscribes existing listeners without a page reload. It disconnects after 3 s of no listeners to free one of the three firmware WebSocket slots, and falls back to 1 Hz REST `/state` polling when only background data is needed.

`mapper.ts` keeps the UI's virtual waveforms (`macro`, `funscript`) separate from the concrete firmware waveforms it posts. Macro and Funscript playback is entirely client-side: the browser or `ossm.py` waits for each timestamp and issues the matching command.

---

## 4. Environment setup

### Rust

The workspace pins the Espressif **`esp`** channel in `rust-toolchain.toml` (needed for the Xtensa ESP32-S3 target). Install it with [espup](https://github.com/esp-rs/espup):

```bash
cargo install espup
espup install --targets esp32c6,esp32s3
source ~/export-esp.sh      # if your shell does not source it automatically
```

`ossm-common`, `ossm-core`, `ossm-std`, and `ossm-wasm` must also build on **stable** without `build-std`. Keep a stable toolchain installed and invoke it explicitly:

```bash
rustup toolchain install stable
cargo +stable test -p ossm-common -p ossm-core
```

### Flashing and debugging tools

```bash
cargo install espflash          # merged image generation + flashing
cargo install probe-rs-tools    # optional: RTT capture for the console tests
```

### Web

Node 20 or newer.

```bash
cd web && npm ci
```

### Python scripts

Python 3.11 or newer.

```bash
curl -LsSf https://astral.sh/uv/install.sh | sh
uv sync
```

Playwright-based tests additionally need browsers: `uv run playwright install chromium`.

### Cross-compilation (release only)

`ziglang` and `cargo-zigbuild` are declared as project dependencies, so `uv sync` already provides the cross-compilation toolchain that `scripts/release.py` uses for `ossm-std`. The macOS target additionally needs a MacOSX SDK for IOKit / CoreFoundation; on Linux the script downloads `MACOSX_SDK_URL` into `.macos-sdk/` unless `SDKROOT` is already set. Pass `--skip-macos` if you cannot provide one.

`wasm-bindgen-cli` is pinned to the version the crate expects; `release.py` installs it automatically if the version on `PATH` does not match.

### `.env`

Copy `.env.example` to `.env`. It is gitignored — **never commit WiFi credentials, IP addresses, or other secrets.**

| Key | Used by |
| --- | --- |
| `DEVICE_MODEL` | `setup_device.py`, `test_console.py` (`esp32c6` \| `esp32s3`) |
| `DEVICE_PORT`, `DEVICE_BAUD` | serial-mode scripts; also read by `ossm-std` as `--serial` / `--baud` aliases |
| `DEVICE_IP` | every live WiFi test; auto-updated by `setup_device.py` |
| `WIFI_SSID`, `WIFI_PASSWORD` | `setup_device.py` |
| `MODBUS_TX`, `MODBUS_RX`, `MODBUS_DERE` | `setup_device.py` |
| `MACOSX_SDK_URL` | `release.py` zigbuild for `aarch64-apple-darwin` |

Point scripts at a desktop instance with `DEVICE_IP=127.0.0.1:8080`.

---

## 5. Building and running

### Firmware

Cargo aliases in `.cargo/config.toml` carry the target, chip feature, `build-std`, and firmware-only release knobs (DWARF, overflow checks, `opt-level = 2`). `build-std` is deliberately *not* global — that would break `cargo +stable test`.

```bash
cargo b-c6 --release      # ESP32-C6, riscv32imac-unknown-none-elf
cargo b-s3 --release      # ESP32-S3, xtensa-esp32s3-none-elf
```

Lint (C6 covers both chips for most code):

```bash
cargo clippy -p ossm-esp32 --config 'unstable.build-std=["core","alloc"]' \
  --target riscv32imac-unknown-none-elf --features esp32c6 --no-default-features -- -D warnings
```

> A bare `cargo b-c6 --release` only refreshes `target/…/ossm-rust`. Flashing goes through the **merged** image in `release/`, produced by `espflash save-image --merge` inside `scripts/release.py`. Use `./scripts/setup_device.py --flash`; do not call `espflash` directly on the ELF for an app flash.

Rebuild `web/apps/webui-esp32` **before** the firmware, or ROM keeps serving the previous `index.html.gz`.

### Desktop

```bash
cargo +stable run -p ossm-std -- --mock --bind 127.0.0.1:8080
cargo +stable run -p ossm-std -- --serial /dev/ttyUSB0 --baud 115200
```

Rebuild `web/apps/webui-std` before building `ossm-std` if you changed the UI; `crates/ossm-std/build.rs` embeds `dist/index.html` at compile time. `--static-dir` overrides the embedded copy during Vite development.

Flags resolve as CLI > environment > default:

| Flag | Env | Default |
| --- | --- | --- |
| `--mode servo\|rtu-relay` | `OSSM_MODE` | `servo` |
| `--serial` | `OSSM_SERIAL`, `DEVICE_PORT` | none (implies `--mock`) |
| `--baud` | `OSSM_BAUD`, `DEVICE_BAUD` | `115200` |
| `--bind` | `OSSM_BIND` | `127.0.0.1:8080` |
| `--modbus-bind` | `OSSM_MODBUS_BIND` | `127.0.0.1:502` |
| `--relay-tcp` / `--relay-ws` / `--rs485-ws` | `OSSM_RELAY_TCP` / `OSSM_RELAY_WS` / `OSSM_RS485_WS` | none |
| `--config` | `OSSM_CONFIG` | `ossm-config.json` |
| `--slave-id` | `OSSM_SLAVE_ID` | `1` |
| `--static-dir` | `OSSM_STATIC_DIR` | bundled webui-std |
| `--mock` | `OSSM_MOCK` | off (auto-on with no bus) |
| `--repl` / `--no-repl` | `OSSM_REPL` / `OSSM_NO_REPL` | REPL on when stdin is a TTY |

### WebAssembly

Not a workspace default member; always name the target explicitly.

```bash
cargo +stable build -p ossm-wasm --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir web/apps/webui-wasm/src/pkg \
  target/wasm32-unknown-unknown/release/ossm_wasm.wasm
cd web && npm run build:html -w webui-wasm
```

The order matters — crate, then `wasm-bindgen`, then Vite. The wasm binary and all assets are inlined into a single self-contained HTML file (`release/ossm-wasm.html`), which can be opened directly in Chrome or Edge (including via `file://`).

### Web apps

```bash
cd web
npm run build:webui-esp32     # picked up by the firmware build
npm run build:webui-std       # picked up by the ossm-std build
npm run build:flasher
npm run build:motor-control
```

`flasher`, `motor-control`, and `webui-wasm` are standalone pages. Nothing embeds them, so after building copy `apps/<name>/dist/index.html` to the matching `release/*.html` yourself, or just run `scripts/release.py`, which does it.

### Everything at once

```bash
uv run scripts/release.py
```

Builds all five web apps, the wasm harness, the `ossm-std` binaries for five targets, and merged firmware images for both chips, into `release/` (known artifact files are deleted first; the directory itself is kept). Skips: `--skip-web`, `--skip-std`, `--skip-firmware`, `--skip-macos`. `webui-std` is always built before the zigbuild step so the embedded HTML is current.

Artifacts: `ossm-esp32c6.bin`, `ossm-esp32s3.bin`, `flasher.html`, `motor-control.html`, `ossm-wasm.html`, and `ossm-std-{linux-x64,linux-arm64,linux-armel,win-x64.exe,macos-arm64}`.

---

## 6. Testing

### Host tests — no hardware

```bash
cargo +stable test -p ossm-core
cargo +stable test -p ossm-wasm
cargo +stable run -p ossm-std -- --help
./scripts/test_modbus_resync.py     # CRC / frame-resync mirror of the firmware classifier
```

### Mock-backed end-to-end

```bash
./scripts/test_frontend.py --mock   # in-process MockOssmServer + Playwright
./scripts/test_webui_std.py         # ossm-std --mock serving the bundled UI
./scripts/test_wasm.py --mock       # wasm harness only
```

### Live hardware

Set `DEVICE_IP` (and `DEVICE_PORT` / `DEVICE_MODEL` where relevant) in `.env`.

| Script | What it exercises |
| --- | --- |
| `setup_device.py --flash` | Flash the merged image, push WiFi/pin config over USB, capture the assigned IP back into `.env` |
| `test_frontend.py` | Concurrent HTTP, BLE GATT, and Playwright coverage of the embedded UI including the macro editor |
| `test_websocket.py` | Every `/ws/command` JSON-RPC method, including live `subscribe-state` pushes |
| `test_ble.py` | GATT characteristics, JSON-RPC over BLE, chunked telemetry during real motion |
| `test_console.py` | USB CLI late attach, `ups` with ACM open and closed, probe-rs RTT, CLI fairness under a `MODBUS_DBG` flood |
| `test_modbus_debug_device.py` | CRC resync classification driven by `POST /modbus-inject`, captured over RTT |
| `test_modbus_relay.py`, `test_modbus_tcp.py` | `rtu_relay` mode over `/ws/modbus` and Modbus TCP `:502` |
| `test_rs485_transceiver.py` | Live switch to `rs485`, USB ACM + `/ws/rs485` through `ossm-std`, optional `--exit-rs485` |
| `test_termux.py` | SSH to Termux: flash C6 over usbfs, provision pins/WiFi, run phone `ossm-std` |
| `test_wasm.py` | Browser page shell against `/ws/rs485` and Web Serial |
| `stress_dt_max.py` | HTTP + WebSocket load; asserts `dt_max_ms < 4.5` |

`test_console.py` and `test_modbus_debug_device.py` capture logs over probe-rs RTT rather than USB serial, because `MODBUS_DBG` output floods the ACM endpoint. They may invoke `sudo` if you have no probe-rs udev rules. Scripts that enable `modbus_debug` disable it again on exit.

---

## 7. API reference

Availability differs per shell:

| Surface | `ossm-esp32` | `ossm-std` | `ossm-wasm` |
| --- | --- | --- | --- |
| `GET /`, `/config`, `/state`, `/paused`, `/restart`, `/ws/command` | yes | yes | — |
| `/shell-config`, `/modbus-inject` | yes | no | no |
| `/ws/modbus`, `/ws/rs485` | in the matching mode | `/ws/modbus` in `rtu-relay` | — |
| BLE GATT | yes | no | no |
| Serial CLI | full catalog | motor paths + actions | — |

All REST responses are CORS `Access-Control-Allow-Origin: *`.

### `MotorControllerConfig`

Returned by `GET /config`, accepted by `POST /config`, and nested inside `GET /state`.

```json
{
  "version": 42,
  "bpm": 60.0,
  "depth": 1.0,
  "depth_top": true,
  "reversed": false,
  "wave_func": "sine",
  "sharpness": 0.5,
  "spline_points": [0.0, 1.0],
  "paused": true,
  "paused_position": 0.5,
  "streaming": false
}
```

| Field | Meaning |
| --- | --- |
| `version` | Monotonic causal timestamp, incremented on every accepted write. Send `0` or omit to let the device assign the next value. Reject snapshots whose version regresses. |
| `bpm` | Strokes per minute. Must be > 0. |
| `depth` | Stroke length, `0.01`–`1.0`. |
| `depth_top` | `true`: stroke spans `[0.0, depth]`. `false`: stroke spans `[1.0 - depth, 1.0]`. |
| `reversed` | Mirrors the waveform in time. |
| `wave_func` | `sine`, `thrust`, or `spline`. The UI also shows `funscript` and `macro`, which are client-driven; posting those labels falls back to sine. |
| `sharpness` | `thrust` only, `0.01` (sharpest) – `0.99` (smoothest). |
| `spline_points` | Control points `0.0`–`1.0` for the `spline` waveform. Catmull-Rom interpolation passes exactly through each point. |
| `paused` | `true` holds the motor still. |
| `paused_position` | Where it holds, `0.0`–`1.0`. |
| `streaming` | Accept real-time waypoints from `/ws/command`. |

`POST /config` (and the WebSocket `set-config` method) deserialises into `MotorControllerConfig` directly, so it needs a **complete** object — only `version`, `spline_points`, and `streaming` carry serde defaults. To change one field, read the current config, merge, and post the whole thing; that is what `ossm.py config` and the web UI do. Request bodies are capped at 1024 bytes.

The response is the applied config with the new `version`, or `409 Conflict` (`-32001` over WebSocket) if the submitted `version` is stale.

### `POST /paused`

Partial body; any subset of:

```json
{ "paused": true, "position": 0.5, "adjust": -0.05 }
```

`position` sets the hold position absolutely, `adjust` moves it relatively. Returns the updated config. (The BLE `CHAR_PAUSED` characteristic takes the same shape; `paused_position` is accepted as an alias for `position`.)

### `GET /state` — `StateResponse`

```json
{
  "config": { "…": "MotorControllerConfig" },
  "t": 123.45,
  "x": 0.5,
  "y": 1.0,
  "shaped_y": 1.0,
  "position": 10000,
  "speed": 0.0,
  "stream": { "buffered": 0, "stream_time": 0.0, "underrun": false },
  "loop_stats": {
    "ups": 320,
    "dt_min_ms": 2.5,
    "dt_avg_ms": 3.1,
    "dt_max_ms": 4.0,
    "dt_mdev_ms": 0.2,
    "update_history": [300, 310, 320, 315],
    "position_history": [0.0, 0.5, 1.0, 0.5]
  },
  "link_stats": {
    "transport": "serial",
    "endpoint": "/dev/ttyACM0",
    "connected": true,
    "exchanges": 1200,
    "failures": 0,
    "success_rate": 1.0,
    "exchanges_per_sec": 320.0,
    "bytes_tx": 4800,
    "bytes_rx": 4800,
    "reconnects": 0,
    "last_error": "",
    "round_trip": { "min": 1.2, "pct5": 1.5, "pct50": 2.0, "pct95": 3.0, "max": 4.5, "mean": 2.1, "mdev": 0.3 },
    "slave_latency": { "min": 0.0, "pct5": 0.0, "pct50": 0.0, "pct95": 0.0, "max": 0.0, "mean": 0.0, "mdev": 0.0 },
    "rx_duration": { "min": 0.0, "pct5": 0.0, "pct50": 0.0, "pct95": 0.0, "max": 0.0, "mean": 0.0, "mdev": 0.0 }
  },
  "motor_connected": true,
  "pos_min": 0.0,
  "pos_max": 25.0
}
```

| Field | Meaning |
| --- | --- |
| `t` | Seconds since motion started. |
| `x` | Waveform phase, `0.0`–`1.0`. |
| `y` / `shaped_y` | Raw waveform output / output after depth and direction. |
| `position` | Absolute motor position in native units. |
| `speed` | Current speed. |
| `stream` | Streaming source status: buffered waypoint count, stream clock, underrun flag. |
| `loop_stats` | Motor-loop update rate and per-tick `dt_*` windows (shell-filled via `ossm-common`). |
| `link_stats` | Bus transport metadata, exchange counters, and RTT percentiles (shell-filled). |
| `motor_connected` | Whether the shell has completed at least one successful bus exchange. |
| `pos_min` / `pos_max` | Physical travel limits found during homing. |

`GET /link-stats` (`ossm-std` servo and `--mode rtu-relay`) returns the `link_stats` object alone for ~1 Hz UI polling without re-fetching full motion state.

Previously flat `ups` / `dt_*` / `update_history` / `modbus_stats` fields are nested under `loop_stats` and `link_stats` (breaking wire change; no shim).

### `GET` / `POST /shell-config` — firmware only

Single JSON document for GPIO/UART/BLE plus WiFi/mDNS (`ShellConfig`). Replaces `/pin-config` and `/network-config` (those paths are 404).

```json
{
  "modbus_tx": 18,
  "modbus_rx": 19,
  "modbus_de_re": 20,
  "modbus_timeout_ms": 0,
  "modbus_rx_timeout_us": 0,
  "modbus_scan_delay_us": 0,
  "modbus_inter_frame_delay_us": 0,
  "modbus_baud": 115200,
  "operating_mode": "servo",
  "ble_enabled": true,
  "modbus_debug": false,
  "wifi_enabled": true,
  "ssid": "",
  "password": "",
  "hostname": "ossm",
  "dhcp_enabled": true,
  "static_ip": "192.168.1.100",
  "static_mask": "255.255.255.0",
  "static_gateway": "192.168.1.1",
  "static_dns": "8.8.8.8"
}
```

| Field | Meaning |
| --- | --- |
| `modbus_tx` / `modbus_rx` / `modbus_de_re` | GPIO for MAX3485 DI / RO / DE+RE. Range 0–48. |
| `modbus_timeout_ms` | Whole-frame read timeout. `0` = auto (10 ms at 115200), max 1000. |
| `modbus_rx_timeout_us` | RX inter-byte timeout (t1.5). `0` = auto (750 µs at 115200). |
| `modbus_scan_delay_us` | Extra delay between baud/slave-ID probes. `0` = t3.5 only. |
| `modbus_inter_frame_delay_us` | Inter-frame quiet (t3.5). `0` = auto (350 µs at 115200). |
| `modbus_baud` | UART1 boot baud, 1200–3000000. In `rs485` mode the live baud follows USB `SET_LINE_CODING` (C6) or a `/ws/rs485` CFG packet. |
| `operating_mode` | `servo`, `rtu_relay`, or `rs485`. **Applies live, no reboot.** |
| `ble_enabled` | BLE GATT server on/off. |
| `modbus_debug` | Diagnostic RX mode. **Requires a restart.** |

`POST` applies `operating_mode` and UART pin remux immediately (the UART1 owner restarts its role). WiFi fields still need a restart. BLE GATT `CHAR_PIN_CONFIG` / `CHAR_NETWORK_CONFIG` remain pin vs WiFi **slices** of the same `shell` document.

`modbus_debug` shortens the RX deadline to 5 ms, classifies every reply (`empty` / `short` / `exact` / `long` / `long_resync` / `leading_junk` / `parse_fail`) and prints `MODBUS_DBG` hex lines. DMA buffers stay at 256 B and `pkt_thres` stays at the expected frame length — raising it to capture size produces false `empty` results after a cancel. Steady-state `ups` drops only a few percent (~310 vs ~320 Hz on C6), but USB log volume rises sharply; leave it off in production.

### `GET` / `POST /modbus-inject` — firmware only, `modbus_debug` required

RAM-only junk injection for exercising the resync path.

```json
{ "mode": "leading|trailing|both|off", "nbytes": 3 }
```

Equivalent CLI: `set inject leading 3`. Prefer the HTTP form when `MODBUS_DBG` is already flooding USB.

### `POST /restart` — firmware and ossm-std

`POST /restart` soft-resets the chip and replies `{"ok":true}`. On `ossm-std`, `/restart` exits the process. WiFi fields in `ShellConfig` take effect after this restart.

### `/ws/command` — JSON-RPC 2.0

Requests take the standard shape; a flat `{"cmd": "…"}` form is also accepted. Include an `id` to get a matching response.

```json
{ "jsonrpc": "2.0", "method": "set-config", "params": { "bpm": 60 }, "id": 1 }
```

| Method | Params | Result |
| --- | --- | --- |
| `ping` | — | `"pong"` |
| `status` | — | `StreamStatus` |
| `get-state` | — | full `StateResponse` |
| `set-config` | a complete `MotorControllerConfig` | applied config with new `version`; `-32001` if stale, or if the device is not in `servo` mode |
| `subscribe-state` | `{"interval_ms": 33}`, default 33, clamped to 20–60000 | `{"subscribed": true, "interval_ms": <resolved>}` |
| `unsubscribe-state` | — | `"unsubscribed"` |
| `reset-timestamp` | — | resets stream time to 0 |
| `append-waypoints` | `[{"ts": 1000, "pos": 0.5, "vel": 0.2}, …]` | appends to the stream buffer |
| `set-waypoints` | a list, or `{"waypoints": […], "reset-timestamp": true}` | replaces the buffer |
| `get-shell-config` / `set-shell-config` | `ShellConfig` | firmware only |

Waypoints: `ts` is a sender-clock millisecond timestamp, `pos` a normalised `0.0`–`1.0` target, `vel` an optional normalised velocity.

While subscribed, the server pushes notifications:

```json
{ "jsonrpc": "2.0", "method": "state", "params": { "…": "StateResponse" } }
```

Re-sending `subscribe-state` changes the interval in place. Firmware WebSocket sessions split into separate RX and TX tasks, so a high-rate push stream never delays incoming commands.

### BLE GATT

Service `6e400001-b5a3-f393-e0a9-e50e24dcca9e`.

| Characteristic | UUID suffix | Access | Payload |
| --- | --- | --- | --- |
| `CHAR_CONFIG` | `…0002` | Read / Write | `MotorControllerConfig` JSON |
| `CHAR_STATE` | `…0003` | Read / Notify | Compact telemetry, ≤256 B |
| `CHAR_PAUSED` | `…0004` | Write | `{"paused": true, "position": 0.0}` |
| `CHAR_PIN_CONFIG` | `…0005` | Read / Write | `PinConfiguration` JSON |
| `CHAR_RPC` | `…0006` | Write / Notify | JSON-RPC 2.0 request / reply |
| `CHAR_NETWORK_CONFIG` | `…0007` | Read / Write | `NetworkConfiguration` JSON |

Reads return the cached attribute value, which is populated on connect and refreshed on writes and telemetry pushes.

Notifications on `CHAR_STATE` and `CHAR_RPC` are chunked, because payloads exceed the ~244 B ATT MTU:

```
byte 0   chunk index, 0-based
byte 1   total chunk count
byte 2+  payload fragment (240 B max)
```

A single-chunk message is `[0, 1, …payload…]`. Reassemble in index order once `total` chunks have arrived; `BleChunkReassembler` in `scripts/ossm.py` does this for Python clients.

### Serial CLI

115200 baud, `\r\n` terminated. nmcli-style dotted paths. `paths` prints the live catalog on the device.

```
get <path>                     section JSON or scalar
set <path> <value>             quote values containing spaces
paths                          print the full catalog
```

Motor paths (`ossm-core::paths`, available on firmware and `ossm-std`):

```
motor                                     full MotorControllerConfig JSON
motor.bpm                                 > 0
motor.depth                               0.01..1
motor.depth_top / reversed / paused / streaming    true|false
motor.wave_func                           sine|thrust|spline
motor.sharpness                           0.01..0.99
motor.paused_position                     0..1
motor.spline_points                       space-separated floats
set motor {"bpm":36,...}                  bulk JSON (motor.version is read-only)
```

Hardware paths (`crates/ossm-esp32/src/hw_paths.rs`, firmware only):

```
pin.modbus_tx / modbus_rx / modbus_de_re          GPIO 0..48
pin.modbus_timeout_ms                             0..1000 (0 = auto)
pin.modbus_rx_timeout_us                          0..200000 (0 = auto)
pin.modbus_scan_delay_us                          0..200000 (0 = t3.5 only)
pin.modbus_inter_frame_delay_us                   0..200000 (0 = auto)
pin.modbus_baud                                   1200..3000000
pin.ble_enabled / pin.modbus_debug                true|false (debug needs reboot)
pin.operating_mode                                servo|rtu_relay|rs485
net.wifi_enabled / net.dhcp_enabled               true|false
net.ssid / net.password / net.hostname            string
net.static_ip / static_mask / static_gateway / static_dns    IPv4
inject                                            set inject <off|leading|trailing|both> <0..64>
```

Actions:

```
reset                          soft reset (ossm-std: exit the process)
get-state / get-status         telemetry JSON
reset-timestamp                reset motion stream time
set-waypoints <json>           replace the waypoint buffer
append-waypoints <json>        append waypoints
help / paths                   usage
quit / exit                    ossm-std only: leave the REPL, HTTP keeps running
```

A successful `set` acknowledges with `{path} set to {value}` on the CLI path *and* as a log line, so the browser flasher still sees the ACK when the log ring is full. A scalar `get` prints `{path}: {value}`.

### Control macro format

A macro is a shareable JSON sequence of timestamped instructions. Playback is client-side — the web Macro Player or `./scripts/ossm.py macro <file>` waits until each `at` (milliseconds from start) and issues the matching command.

```json
{
  "version": 1,
  "name": "Warm up",
  "loop": false,
  "instructions": [
    { "at": 0, "action": "set", "params": { "bpm": 40, "depth": 0.6, "wave_func": "sine" } },
    { "at": 0, "action": "start" },
    { "at": 15000, "action": "set", "params": { "bpm": 90, "wave_func": "thrust", "sharpness": 0.2 } },
    { "at": 60000, "action": "stop", "params": { "position": 0.0 } }
  ]
}
```

`set` params accept `bpm`, `depth`, `depth_top`, `reversed`, `wave_func`, `sharpness`, `spline_points`. `stop` optionally parks at `params.position`. Generate a starter file with `./scripts/ossm.py macro --init example.json`. Browser drafts live in `localStorage` under `ossm_macros_v1` / `ossm_macro_draft_v1`.

### `ossm.py`

One client for all three transports: `-m wifi` (REST + WebSocket), `-m ble` (persistent GATT connection), `-m serial` (USB CLI). Transport options (`-m`, `-i`, `-p`, `-b`, `-a`) live on the top-level callback, so they go **before** the subcommand; every subcommand additionally accepts a local `-m` override. Defaults come from `.env` (`DEVICE_IP`, `DEVICE_PORT`, `DEVICE_BAUD`, `DEVICE_BLE_ADDR`).

```bash
./scripts/ossm.py --help
./scripts/ossm.py -i 192.168.1.123 status
./scripts/ossm.py run --bpm 40 --depth 0.8 --wave spline -m wifi
./scripts/ossm.py pause --pos 0.0 -m ble
./scripts/ossm.py resume -m ble
./scripts/ossm.py config --bpm 55 --sharpness 0.3
./scripts/ossm.py -m serial -p /dev/ttyACM0 pins
./scripts/ossm.py net -m wifi
./scripts/ossm.py wifi --ssid MyNetwork --password secret -m serial
./scripts/ossm.py monitor -t 30 -m ble
./scripts/ossm.py play strokes.funscript --speed 1.0
./scripts/ossm.py macro --init /tmp/warmup.json
./scripts/ossm.py macro /tmp/warmup.json --loop -m wifi
./scripts/ossm.py restart
```

---

## 8. Changing the API surface

Adding a command or a config field touches several layers. Update the crate that owns it, then every consumer:

- **Motion, causal version, RPC and CLI catalog** — `ossm-core`: `config.rs`, `state.rs`, `command.rs`, `rpc.rs`, `rpc_types.rs`, `paths.rs`, `engine.rs`, plus `engine_tests.rs` / `rpc_tests.rs`.
- **Telemetry windows, wire structs, and link PLL** — `ossm-common`; shell drivers in `motor_57aim30.rs` / `engine_task.rs` / `motor_thread.rs` / `shell.rs` / `bus.rs`.
- **Firmware queue and enqueue path** — `crates/ossm-esp32/src/motion.rs`, `context.rs` (`try_enqueue_config`).
- **Firmware transports** — `http_api.rs` (pin / net / inject live here, not in core), `ble_api.rs`, `rpc.rs`, `command.rs` + `hw_paths.rs`.
- **Desktop shell** — `crates/ossm-std/src/`: `http.rs` (`GET /link-stats`), `cli.rs`, `engine_task.rs`, `motor_thread.rs`, `precise_wait.rs`, `link_stats.rs`, and `bus.rs` / `relay_server.rs` for relay modes.
- **Browser shell** — `crates/ossm-wasm/src/shell.rs`, `wasm_api.rs`; rebuild `release/ossm-wasm.html`.
- **Frontend** — `web/packages/shared` (types, `mapper.ts`, components) and `web/packages/client` (transports), then each of the three webui apps.
- **57AIM30 tool** — `web/apps/motor-control/src/lib/registers.ts`, `send-options.ts`, panels; rebuild `release/motor-control.html`. Keep the E2E ids `#motor-control-connection`, `#mc-connection-type`, `#mc-baud`, `#mc-ws-url` stable.
- **Scripts** — `ossm.py` schemas first, then the tests that assert on the new field.
- **Docs** — this file, `README.md`, `README.zh.md`.

### Wiring diagrams

Never hand-author the SVGs. Edit the Vue app sources in `web/apps/wiring-diagram/src/` (`PathAView.vue` for the USB-RS485 path, `PathBView.vue` for the ESP32 path), build, then export:

```bash
npm --prefix web run build:wiring-diagram
uv run scripts/export_wiring_svg.py
```

The app provides route-driven views (`#/std`, `#/std-zh`, `#/esp`, `#/esp-zh`) with reusable components and renders them through Playwright with `html-to-image`.
