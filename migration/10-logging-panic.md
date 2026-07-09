# 10: Logging & Panic Handling

## Scope

Replace `esp_idf_svc::log::EspLogger` with `esp-println` (log output) and `esp-backtrace` (panic handler). The `log` crate facade is preserved.

## Files to Modify

- `src/main.rs` (no explicit logger init needed)
- `Cargo.toml` (covered in [01-build-system](01-build-system.md))

---

## Target: Logger Initialization

```rust
#![no_std]
#![no_main]

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // esp-println's `log` feature auto-initializes the logger.
    // No explicit init call needed.
    log::info!("OSSM Rust firmware starting (esp-hal)");
    // ...
}
```

---

## USB Serial JTAG: Logs + CLI

Both `esp-println` (routing `log::info!()` etc.) and the CLI `UsbWriter` (see [08-serial-cli](08-serial-cli.md)) output to the same USB Serial JTAG port. This matches current behavior where ESP-IDF logs and `println!` share the USB serial connection.

Interleaved output is expected. The Python test scripts (`ossm.py monitor`) and browser flasher terminal handle this.

If garbled output becomes a problem, options include:
- Route logs to UART0 (requires external USB-UART adapter)
- Use `defmt` with a separate RTT channel (advanced)

For the initial migration, shared USB Serial JTAG is acceptable.

---

## `println!` Replacement

```rust
// In command.rs for CLI JSON output:
esp_println::println!("{}", json);
```

Or re-export in `main.rs`:

```rust
#[macro_use]
extern crate esp_println;
```

---

## Panic Behavior

`esp-backtrace` with `panic-handler` + `println` features:
1. Prints the panic message via `esp-println` (same USB port)
2. Prints a stack backtrace (requires debug symbols in ELF)
3. Halts the CPU

---

## Cargo.toml

```toml
esp-backtrace = { version = "0.15", features = ["panic-handler", "println"] }
esp-println = { version = "0.13", features = ["log"] }
log = "0.4"
```

---

## Summary

| Item | Current | Target |
|---|---|---|
| Logger init | `EspLogger::initialize_default()` | Automatic (`esp-println` `log` feature) |
| Panic handler | ESP-IDF built-in | `esp-backtrace` |
| `log::info!()` | Works | Works (unchanged) |
| `println!()` | `std::println!` | `esp_println::println!` |
| Output port | USB Serial JTAG | USB Serial JTAG (shared with CLI) |
