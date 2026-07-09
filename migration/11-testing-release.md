# 11: Testing Strategy & Release Process

## Scope

Define how to validate each migration phase, update the release build scripts, and set realistic expectations for test suite compatibility.

---

## Incremental Validation Strategy

### Phase 0: Risk Spike

Before full subsystem migration, validate on hardware:

```bash
# Minimal esp-hal firmware with:
# 1. InterruptExecutor motor loop + WiFi running
# 2. esp_radio::init() + WiFi + BLE advertising simultaneously
# 3. Modbus over manual DE/RE GPIO toggle
```

Checklist:
- [ ] Motor loop achieves stable UPS with WiFi associated
- [ ] BLE advertises while WiFi is connected
- [ ] Modbus read/write succeeds with manual DE/RE timing

### Phase 1: Build System, Storage & Entry Point

```bash
cargo b-c6 --release
cargo b-s3 --release
espflash flash --monitor target/riscv32imac-unknown-none-elf/release/ossm-rust
```

Checklist:
- [ ] Both targets compile
- [ ] Dual heap allocators initialize
- [ ] `esp_rtos::start()` succeeds
- [ ] NVS reads existing WiFi credentials from ESP-IDF flash
- [ ] `log::info!` appears on USB serial

### Phase 2: Motor Control & Shared State

```bash
./scripts/ossm.py status -m serial
```

Checklist:
- [ ] UART initializes with NVS-configured pins (or fallback discovery)
- [ ] Motor homing completes
- [ ] `compute_cycle()` + `write_position()` loop runs
- [ ] Loop stats logged (UPS, dt min/avg/max/mdev)
- [ ] Motor moves in response to config changes

### Phase 3: WiFi & HTTP

```bash
./scripts/test_websocket.py
./scripts/ossm.py status -m wifi
```

Checklist:
- [ ] WiFi connects with NVS credentials
- [ ] mDNS answers `<hostname>.local` and `_http._tcp.local`
- [ ] All REST endpoints return correct JSON
- [ ] CORS OPTIONS preflight works
- [ ] WebSocket JSON-RPC: ping, status, get-state, set-config
- [ ] subscribe-state push notifications at configured interval
- [ ] append-waypoints / set-waypoints / reset-timestamp
- [ ] Frontend HTML served (gzipped)
- [ ] `Connection: close` prevents connection slot exhaustion

### Phase 4: BLE

```bash
./scripts/test_ble.py
```

Checklist:
- [ ] Device advertises as "OSSM" with correct service UUID
- [ ] All characteristics readable/writable
- [ ] JSON-RPC over CHAR_RPC (ping, get-state, subscribe-state)
- [ ] State notifications push after subscribe-state
- [ ] No ATT 0x0E errors

### Phase 5: CLI

```bash
./scripts/ossm.py monitor -m serial
./scripts/setup_device.py  # without --flash
```

Checklist:
- [ ] All set-* / get-* commands work
- [ ] reset reboots device
- [ ] Waypoint CLI commands enqueue correctly

---

## Release Script Updates

```bash
#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$SCRIPT_DIR"
RELEASE_DIR="$SCRIPT_DIR/release"
mkdir -p "$RELEASE_DIR"

(cd frontend && npm ci && npm run build)
(cd flasher && npm ci && npm run build)
cp flasher/dist/index.html "$RELEASE_DIR/flasher.html"

cargo b-c6 --release
cargo b-s3 --release

espflash save-image --chip esp32c6 --merge \
    target/riscv32imac-unknown-none-elf/release/ossm-rust \
    "$RELEASE_DIR/ossm-esp32c6.bin"

espflash save-image --chip esp32s3 --merge \
    target/xtensa-esp32s3-none-elf/release/ossm-rust \
    "$RELEASE_DIR/ossm-esp32s3.bin"
```

Key changes:
- Target triples: `*-unknown-none-elf` / `*-none-elf` (not `*-esp-espidf`)
- No separate `--bootloader` argument (espflash embeds bootloader for bare-metal)
- No `ldproxy` or ESP-IDF CMake

---

## Host-Side Unit Testing

`motion.rs` JSON parsing tests currently live in `http_api.rs` as `#[cfg(test)]`. These cannot run on the embedded target (no `std`, no harness). Options:

1. **Move JSON parsing tests** to `src/rpc.rs` or a `tests/json_parsing.rs` host test crate
2. **Extract pure parsing** into functions testable on host with `cargo test`

The Python E2E suites remain the primary validation path.

---

## Test Suite Compatibility (realistic expectations)

| Script | Protocol | Expected changes |
|---|---|---|
| `scripts/test_websocket.py` | WebSocket JSON-RPC | None if `rpc.rs` preserves all methods and response shapes |
| `scripts/test_ble.py` | BLE GATT + JSON-RPC | None if subscribe-state and UUIDs preserved; **will fail** if BLE RPC subscribe not wired |
| `scripts/setup_device.py` | Serial CLI | None — same commands |
| `scripts/ossm.py` | WiFi REST + WS + BLE + Serial | None — same APIs |

Known differences after migration:
- Motor loop may respond faster (lower jitter)
- WebSocket reconnection behavior may differ (picoserve vs edge-http)
- Log output interleaves with CLI on same USB port

---

## Binary Size Comparison

```bash
ls -la release/ossm-esp32c6.bin
cargo size --target riscv32imac-unknown-none-elf --release -- -A
```

Expected: smaller than ESP-IDF build (no FreeRTOS/IDF components), but `serde_json` + dual heap add overhead.

---

## Regression Checklist (final validation)

Run the full suite on both ESP32-C6 and ESP32-S3:

```bash
./scripts/test_websocket.py
./scripts/test_ble.py
./scripts/ossm.py dash -m wifi
./scripts/ossm.py dash -m ble
```

Compare motor loop stats (UPS, dt mdev) before and after migration to confirm jitter improvement.
