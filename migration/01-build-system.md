# 01: Build System & Toolchain

## Scope

Migrate `Cargo.toml`, `.cargo/config.toml`, `rust-toolchain.toml`, and `build.rs` from the ESP-IDF CMake build to bare-metal esp-hal targets. Remove `embuild`, `sdkconfig.defaults*`, and all ESP-IDF linkage.

## Files to Modify

- `Cargo.toml`
- `.cargo/config.toml`
- `rust-toolchain.toml`
- `build.rs`

## Files to Delete

- `sdkconfig.defaults`
- `sdkconfig.defaults.esp32s3`
- `components_esp32c6.lock`
- `components_esp32s3.lock`

---

## Target: `Cargo.toml`

```toml
[package]
name = "ossm-rust"
version = "0.2.0"
authors = ["vampmaker <vampmaker@outlook.com>"]
edition = "2021"
resolver = "2"

[profile.release]
opt-level = 2          # MUST be 2 or 3 — radio blobs malfunction at "s"/"z"
debug = true
overflow-checks = true
lto = "thin"

[profile.dev]
debug = true
opt-level = "z"        # Fast iteration for non-radio code

# Radio blobs REQUIRE opt-level >= 2. Scope this to esp-radio only in dev builds.
[profile.dev.package.esp-radio]
opt-level = 3

[features]
default = ["esp32c6"]
esp32c6 = [
    "esp-hal/esp32c6",
    "esp-rtos/esp32c6",
    "esp-radio/esp32c6",
    "esp-storage/esp32c6",
    "esp-nvs/esp32c6",
    "esp-alloc/esp32c6",
    "esp-backtrace/esp32c6",
    "esp-println/esp32c6",
]
esp32s3 = [
    "esp-hal/esp32s3",
    "esp-rtos/esp32s3",
    "esp-radio/esp32s3",
    "esp-storage/esp32s3",
    "esp-nvs/esp32s3",
    "esp-alloc/esp32s3",
    "esp-backtrace/esp32s3",
    "esp-println/esp32s3",
]

[dependencies]
# HAL & Runtime
esp-hal = { version = "1.1", features = ["unstable"] }
esp-rtos = { version = "0.3", features = ["embassy", "esp-radio", "esp-alloc"] }
esp-radio = { version = "1.0.0-beta.0", features = ["wifi", "ble", "coex", "unstable"] }
esp-alloc = "0.10"
esp-backtrace = { version = "0.15", features = ["panic-handler", "println"] }
esp-println = { version = "0.13", features = ["log"] }

# Storage
esp-storage = { version = "0.8", features = ["low-level"] }
esp-nvs = "0.4"

# Embassy async
embassy-executor = { version = "0.10", features = ["task"] }
embassy-time = "0.5"
embassy-sync = "0.8"
embassy-futures = "0.1"
embassy-net = { version = "0.8", features = ["tcp", "dhcpv4", "medium-ethernet", "dns"] }
static_cell = "2"

# HTTP server
picoserve = { version = "0.18", features = ["embassy", "ws", "alloc"] }

# BLE
trouble-host = { version = "0.6", features = ["gatt", "derive", "peripheral"] }
bt-hci = "0.9"

# Motor / Modbus
rmodbus = { version = "0.9", features = ["fixedvec"] }
fixedvec = "0.2"

# Serialization
serde = { version = "1.0", default-features = false, features = ["derive", "alloc"] }
serde_json = { version = "1.0", default-features = false, features = ["alloc"] }

# Utilities
heapless = "0.9"
embedded-cli = "0.2"
embedded-io = "0.6"
embedded-io-async = "0.7"
libm = "0.2"
log = "0.4"
critical-section = "1.2"

[build-dependencies]
flate2 = { version = "1.0", default-features = false, features = ["rust_backend"] }
```

### Key changes:
- **Removed**: `esp-idf-svc`, `esp-idf-sys`, `esp32-nimble`, `edge-http`, `edge-nal-std`, `edge-ws`, `embuild`, `anyhow`
- **Added**: `esp-hal`, `esp-rtos`, `esp-radio` (with **`coex`**), `esp-alloc`, `esp-backtrace`, `esp-println`, `esp-storage`, `esp-nvs`, `picoserve`, `trouble-host`, `bt-hci`, `embassy-net`
- **New modules**: `src/error.rs`, `src/rpc.rs` (see subsystem plans)
- **Dev opt-level**: Only `esp-radio` gets `opt-level = 3` in dev; rest of firmware compiles at `"z"`

---

## Target: `.cargo/config.toml`

```toml
[target.riscv32imac-unknown-none-elf]
runner = "espflash flash --monitor"

[target.xtensa-esp32s3-none-elf]
runner = "espflash flash --monitor"

[unstable]
build-std = ["core", "alloc"]

[alias]
b-c6 = "build --target riscv32imac-unknown-none-elf --features esp32c6 --no-default-features"
b-s3 = "build --target xtensa-esp32s3-none-elf --features esp32s3 --no-default-features"
```

> **Usage Note**: Per repository guidelines (`AGENTS.md`), always build using explicit aliases: `cargo b-c6 --release` for ESP32-C6 and `cargo b-s3 --release` for ESP32-S3.

---

## Target: `rust-toolchain.toml`

Replace the current ESP-IDF toolchain (`channel = "esp"`) with standard nightly channel and required `rust-src` component for `-Zbuild-std`:

```toml
[toolchain]
channel = "nightly"
components = ["rust-src"]
```

---

## Target: `build.rs`

```rust
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use flate2::write::GzEncoder;
use flate2::Compression;

fn main() {
    println!("cargo:rerun-if-changed=frontend/dist/index.html");

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("index.html.gz");

    let html_path = Path::new("frontend/dist/index.html");
    let content = if html_path.exists() {
        fs::read(html_path).unwrap()
    } else {
        b"<html><body><h1>Frontend not built</h1></body></html>".to_vec()
    };

    let mut encoder = GzEncoder::new(File::create(&dest_path).unwrap(), Compression::best());
    encoder.write_all(&content).unwrap();
    encoder.finish().unwrap();

    if let Some(parent) = html_path.parent() {
        if parent.exists() {
            let dist_gz = parent.join("index.html.gz");
            if let Ok(file) = File::create(&dist_gz) {
                let mut encoder = GzEncoder::new(file, Compression::best());
                let _ = encoder.write_all(&content);
                let _ = encoder.finish();
            }
        }
    }
}
```

No `embuild` call. esp-hal linker scripts are pulled in via the `esp-hal` crate dependency — no extra `build.rs` linker setup needed for basic builds.

---

## Heap Allocators (in `main.rs`, not Cargo.toml)

esp-radio recommends two heap regions. In `main.rs`:

```rust
// Reclaimed DRAM region (larger pool for WiFi/BLE driver blobs)
esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
// General heap for serde_json, String, Vec
esp_alloc::heap_allocator!(size: 36 * 1024);
```

A single 96 KB allocator is insufficient when WiFi + BLE + JSON serialization run concurrently.

---

## Custom Error Type (replaces `anyhow`)

Create `src/error.rs`:

```rust
use core::fmt;

#[derive(Debug)]
pub enum FirmwareError {
    Uart(&'static str),
    Modbus(&'static str),
    Storage(&'static str),
    Wifi(&'static str),
    Config(&'static str),
    Json,
    PinUnavailable,
    MotorNotInitialized,
    QueueFull,
}

impl fmt::Display for FirmwareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uart(msg) => write!(f, "UART: {}", msg),
            Self::Modbus(msg) => write!(f, "Modbus: {}", msg),
            Self::Storage(msg) => write!(f, "Storage: {}", msg),
            Self::Wifi(msg) => write!(f, "WiFi: {}", msg),
            Self::Config(msg) => write!(f, "Config: {}", msg),
            Self::Json => write!(f, "JSON parse error"),
            Self::PinUnavailable => write!(f, "GPIO pin unavailable"),
            Self::MotorNotInitialized => write!(f, "Motor controller not initialized"),
            Self::QueueFull => write!(f, "Command queue full"),
        }
    }
}

pub type Result<T> = core::result::Result<T, FirmwareError>;
```

This replaces all `anyhow::Result<T>` and `anyhow::anyhow!(...)` calls throughout the codebase.
