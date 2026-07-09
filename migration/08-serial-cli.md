# 08: Serial CLI (USB Serial JTAG)

## Scope

Replace `std::fs::File::from_raw_fd(0)` stdin reading with `esp_hal::usb_serial_jtag::UsbSerialJtag` in async mode. The `embedded-cli` command parsing logic remains unchanged.

## Files to Modify

- `src/command.rs` (I/O layer rewrite; `execute_command()` preserved)

---

## USB Peripheral Naming

In esp-hal 1.1, the peripheral is `peripherals.USB_DEVICE` (not `USB_SERIAL_JTAG`). Use consistently in `main.rs` and `command.rs`.

---

## Target: CLI I/O

```rust
use esp_hal::usb_serial_jtag::UsbSerialJtag;
use embassy_time::Timer;
use embedded_io::Write as EmbeddedWrite;
use embedded_io::ErrorType;

struct UsbWriter<'d> {
    usb: &'d mut UsbSerialJtag<'d, esp_hal::Async>,
}

impl EmbeddedWrite for UsbWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, core::convert::Infallible> {
        let _ = self.usb.write(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> Result<(), core::convert::Infallible> {
        Ok(())
    }
}

pub async fn handle_cli(
    usb_peripheral: esp_hal::peripherals::USB_DEVICE,
    app_context: AppContext,
) {
    let mut usb = UsbSerialJtag::new(usb_peripheral).into_async();

    let mut cli = CliBuilder::default()
        .writer(UsbWriter { usb: &mut usb })
        .command_buffer([0u8; 2048])
        .history_buffer([0u8; 2048])
        .build()
        .unwrap();

    let mut processor = BaseCommand::processor(|_cli, command| {
        execute_command(command, &app_context);
        Ok(())
    });

    let mut buf = [0u8; 1];
    loop {
        match embedded_io_async::Read::read(&mut usb, &mut buf).await {
            Ok(1) => {
                let byte = if buf[0] == 0x7F { 0x08 } else { buf[0] };
                let _ = cli.process_byte::<BaseCommand<'_>, _>(byte, &mut processor);
            }
            Ok(_) | Err(_) => {
                Timer::after_millis(10).await;
            }
        }
    }
}
```

---

## Sync Callback + Async Mutex

`embedded-cli`'s processor callback is synchronous. For storage/motor access from CLI, use a **blocking mutex** (not `embassy_sync::mutex::Mutex::try_lock()`, which may not exist on the async mutex):

```rust
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use core::cell::RefCell;

// In AppContext (see 09-shared-state.md) OR access via blocking wrapper:
app_context.storage_blocking.lock(|sm| {
    let mut sm = sm.borrow_mut();
    sm.set_ssid(ssid).unwrap();
});
```

Alternative: use `critical_section::with()` around a brief `futures::executor::block_on(storage.lock())` — but blocking mutex is cleaner.

The motor task runs on the **interrupt executor** (high priority), so CLI blocking on the thread executor does not stall the motor loop.

---

## Restart Command

`embassy_time::block_for` is not available in `no_std` without the `std` feature. Use an async restart from a spawned task:

```rust
BaseCommand::Reset => {
    log::info!("Rebooting...");
    // Spawn a one-shot task to delay then reset (see 05-http-websocket post_restart)
    esp_hal::reset::software_reset();
}
```

Or if already in async context, `Timer::after_millis(100).await` before reset.

---

## Output: CLI vs Logging

Both `esp-println` (for `log::info!`) and the CLI `UsbWriter` write to USB Serial JTAG. This is acceptable — log output and CLI responses interleave on the same serial port, same as the current ESP-IDF firmware where `println!` and logs share USB serial.

For `get-*` commands that print JSON, use `esp_println::println!` or write through the `UsbWriter` adapter.

---

## Waypoint Commands

CLI waypoint commands (`set-waypoints`, `append-waypoints`) enqueue via `MotorController::get_command_sender()`:

```rust
let cmd = MotionCommand::AppendWaypoints(waypoints);
if let Some(mc) = app_context.motor_controller.lock().await.as_ref() {
    let sender = mc.get_command_sender();
    sender.lock().await.enqueue(cmd).map_err(|_| FirmwareError::QueueFull)?;
}
```

In the sync CLI callback, use the blocking mutex variant of the command sender (see [09-shared-state](09-shared-state.md)).

---

## Command Definitions

The `#[derive(Command)]` enum `BaseCommand` is **completely unchanged** — `embedded-cli` is already `no_std` compatible.
