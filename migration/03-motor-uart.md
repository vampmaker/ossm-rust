# 03: Motor Control & UART (Modbus RTU)

## Scope

Migrate `src/motor_57aim30.rs` from `esp_idf_svc::hal::uart::AsyncUartDriver` to `esp_hal::uart::Uart` in async mode. Preserve the **split architecture**: `MotorController` (pure math/state) in the shared mutex, `Modbus57AIM30Motor` (UART I/O driver) as a local variable in `run_motor()`.

## Files to Modify

- `src/motor_57aim30.rs` (UART driver, Modbus RTU, `run_motor()` loop)
- `src/motor.rs` (minimal: `anyhow` → `crate::error`)
- `src/motion.rs` (see mechanical changes below and [09-shared-state](09-shared-state.md))

---

## Motor Architecture (preserve this split)

```
run_motor() local scope:
  Modbus57AIM30Motor  ←── async UART I/O (write_position, cycle, homing)
         ↑
         │ position/speed from
         │
  MotorController     ←── in AppContext mutex (compute_cycle, config, command queue)
```

The motor driver is **not** stored in `AppContext`. Only `MotorController` (state/math) is shared across HTTP/BLE/CLI tasks.

---

## Motor Loop (preserve current logic)

```rust
pub async fn run_motor(
    app_context: AppContext,
    uart_peripheral: esp_hal::peripherals::UART1,
    pin_config: storage::PinConfiguration,
) -> crate::error::Result<()> {
    let mut gpio_pins = GpioPins::new(/* take from peripherals or static init */);
    let mut motor = init_motor_driver(uart_peripheral, &mut gpio_pins, &pin_config, &app_context).await?;

    // ... homing, set_max_power, sync_to_position (see Init Sequence below) ...

    *app_context.motor_controller.lock().await = Some(motor_controller);

    let mut last_stats_log = embassy_time::Instant::now();

    loop {
        // Pure computation under brief lock (< 5 µs, no I/O)
        let target = {
            let mut mc_lock = app_context.motor_controller.lock().await;
            mc_lock.as_mut().map(|mc| mc.compute_cycle())
        };

        // Motor I/O outside the lock — await yields to other tasks/executors
        match target {
            Some((position, speed)) => {
                motor.write_position(position, speed).await?;
                motor.cycle().await?;  // Modbus commit in driver
            }
            None => {
                log::error!("Motor controller lost, stopping motor loop");
                break;
            }
        }

        // Log loop stats every 5 seconds
        if embassy_time::Instant::now().duration_since(last_stats_log)
            >= embassy_time::Duration::from_secs(5)
        {
            if let Some(mc) = app_context.motor_controller.lock().await.as_ref() {
                let st = &mc.last_loop_stats;
                if st.ups > 0 {
                    log::info!(
                        "Motor loop stats (1s): UPS={}, dt(ms) min/avg/max/mdev = {:.2} / {:.2} / {:.2} / {:.2}",
                        st.ups, st.min_dt_ms, st.avg_dt_ms, st.max_dt_ms, st.mdev_dt_ms,
                    );
                }
            }
            last_stats_log = embassy_time::Instant::now();
        }
    }
    Ok(())
}
```

---

## Init Sequence (match current `main.rs`)

```rust
// 1. UART + Modbus init with pin selection / fallback (see GPIO section)
let mut motor = Modbus57AIM30Motor::new(modbus, pin_config.modbus_scan_delay_us);

// 2. Modbus scan with retries (same logic as current firmware)
// enable_modbus_communication → modbus_scan → set_baud_rate if needed

// 3. Load motor config from NVS (or save default)
let motor_config = /* from storage or MotorControllerConfig::default() */;

// 4. Homing + motor parameter setup (async I/O)
motor.homing().await?;
motor.set_max_power(0.6).await?;
motor.set_acceleration(4000.0).await?;
motor.set_position_ring_ratio(3000.0).await?;
motor.set_speed_ring_ratio(3000.0).await?;
let current_position = motor.read_position().await?;

// 5. Create MotorController (pure state) and sync position
let mut motor_controller = MotorController::new(motor_config);
motor_controller.sync_to_position(motor.pos_min(), motor.pos_max(), current_position)?;

// 6. Store controller in shared mutex; motor driver stays local
*app_context.motor_controller.lock().await = Some(motor_controller);
```

---

## RS-485 / DE·RE Pin Handling

### Current (esp-idf-hal)

Uses `uart::config::Mode::RS485HalfDuplex` with RTS pin wired to DE/~RE. `ModbusRTUMaster::new(uart, None, ...)` — driver auto-toggles RTS.

### Target (esp-hal 1.1)

**esp-hal has no `RS485HalfDuplex` UART mode.** Use manual DE/RE GPIO control:

```rust
let uart = Uart::new(uart_peripheral, uart_config)?
    .with_tx(tx_pin)
    .with_rx(rx_pin)
    .into_async();

// Manual DE/RE output (active-high transmit enable)
let de_re = Output::new(de_re_pin, Level::Low);

let modbus = ModbusRTUMaster::new(
    uart,
    Some(de_re),   // manual toggle in modbus_request()
    1,
    pin_config.modbus_timeout_ms,
    TARGET_BAUD_RATE,
);
```

In `modbus_request()`:

```rust
async fn modbus_request(&mut self, req: &[u8], resp: &mut [u8]) -> Result<usize> {
    // Flush/drain any stale RX bytes non-blockingly
    let _ = self.uart.flush_rx();

    if let Some(ref mut pin) = self.ctrl_pin {
        pin.set_high();
        // 10 µs synchronous busy-wait (~1,600 CPU cycles at 160 MHz) for RS-485 Driver Enable setup time.
        // Synchronous delay is used here because async Timer::after overhead (~20 µs context switch)
        // exceeds the required hardware transceiver setup time.
        esp_hal::delay::Delay::new().delay_micros(10);
    }
    self.uart_write_all(req).await?;
    // Async guard delay: wait 100 µs for final stop bit to physically shift out of TX pin
    embassy_time::Timer::after(embassy_time::Duration::from_micros(100)).await;
    if let Some(ref mut pin) = self.ctrl_pin {
        pin.set_low();
    }
    // ESP32 UART RX hardware FIFO automatically buffers incoming bytes from the slave
    // ... read response ...
}
```

> **Hardware validation required**: Manual DE/RE timing and the 100 µs post-transmission guard time must be verified on real RS-485 hardware. This is a Phase 0 spike item.

---

## Target: `ModbusRTUMaster`

```rust
use esp_hal::uart::Uart;
use esp_hal::gpio::Output;
use embassy_time::{with_timeout, Duration};
use embedded_io_async::{Read, Write};

pub struct ModbusRTUMaster<'d> {
    uart: Uart<'d, esp_hal::Async>,
    ctrl_pin: Option<Output<'d>>,
    device_id: u8,
    read_timeout: Duration,
    write_timeout: Duration,
    timeout_override_ms: u32,
}
```

### UART Read/Write

```rust
async fn uart_read_exactly(&mut self, buf: &mut [u8]) -> Result<()> {
    let mut total = 0;
    while total < buf.len() {
        let n = with_timeout(self.read_timeout, self.uart.read(&mut buf[total..]))
            .await
            .map_err(|_| FirmwareError::Uart("read timeout"))?
            .map_err(|_| FirmwareError::Uart("read error"))?;
        total += n;
    }
    Ok(())
}

async fn uart_write_all(&mut self, buf: &[u8]) -> Result<()> {
    let mut total = 0;
    while total < buf.len() {
        let n = with_timeout(self.write_timeout, self.uart.write(&buf[total..]))
            .await
            .map_err(|_| FirmwareError::Uart("write timeout"))?
            .map_err(|_| FirmwareError::Uart("write error"))?;
        total += n;
    }
    let _ = self.uart.flush().await;
    Ok(())
}
```

---

## GPIO Pin Selection

Replace `all_pins: Vec<Option<AnyIOPin>>` with a chip-specific `GpioPins` struct. Array size must cover all valid GPIO indices (48 for ESP32-S3, 31 for ESP32-C6):

```rust
#[cfg(feature = "esp32c6")]
pub type PinArray = [Option<AnyPin>; 31];

#[cfg(feature = "esp32s3")]
pub type PinArray = [Option<AnyPin>; 49];  // GPIO0..GPIO48

pub struct GpioPins {
    pins: PinArray,
}

impl GpioPins {
    pub fn take(&mut self, pin_num: usize) -> Option<AnyPin> {
        if pin_num < self.pins.len() {
            self.pins[pin_num].take()
        } else {
            None
        }
    }

    /// Fallback: find first three available pins (same as current firmware)
    pub fn take_three_available(&mut self) -> Option<(AnyPin, AnyPin, AnyPin)> {
        let mut taken = Vec::new();
        for (i, slot) in self.pins.iter_mut().enumerate() {
            if let Some(pin) = slot.take() {
                taken.push((i, pin));
                if taken.len() == 3 { break; }
            }
        }
        if taken.len() == 3 {
            Some((taken[0].1, taken[1].1, taken[2].1))
        } else {
            // Put back any taken pins on failure
            for (i, pin) in taken { self.pins[i] = Some(pin); }
            None
        }
    }
}
```

### Pin fallback (preserve current behavior)

```rust
let tx = gpio_pins.take(pin_config.modbus_tx as usize);
let rx = gpio_pins.take(pin_config.modbus_rx as usize);
let de_re = gpio_pins.take(pin_config.modbus_de_re as usize);

let (tx, rx, de_re) = match (tx, rx, de_re) {
    (Some(t), Some(r), Some(d)) => (t, r, d),
    _ => {
        log::warn!("Configured pins unavailable, searching for alternatives");
        let (t, r, d) = gpio_pins.take_three_available()
            .ok_or(FirmwareError::PinUnavailable)?;
        // Save discovered pins to NVS
        let new_config = PinConfiguration {
            modbus_tx: /* index of t */,
            modbus_rx: /* index of r */,
            modbus_de_re: /* index of d */,
            ..pin_config
        };
        app_context.storage.lock().await.set_pin_configuration(&new_config)?;
        (t, r, d)
    }
};
```

Exclude USB-serial pins from the pool: C6 GPIO12/13, S3 GPIO19/20.

> **Default pin config fix**: `PinConfiguration::default()` uses `rx=19, de_re=20` which conflicts with USB-JTAG on ESP32-S3. Use chip-specific defaults or the pin-fallback path.

---

## Changes to `src/motion.rs`

Mechanical `std` → `core`/`alloc` substitutions:

| Current | Target |
|---|---|
| `use std::time;` | `use embassy_time;` |
| `use std::collections::VecDeque;` | `use alloc::collections::VecDeque;` |
| `use std::sync::{Arc, Mutex};` | `use embassy_sync::mutex::Mutex` + `&'static` (see [09-shared-state](09-shared-state.md)) |
| `std::f32::consts::PI` | `core::f32::consts::PI` |
| `time::Instant::now()` | `embassy_time::Instant::now()` |
| `now.duration_since(t0).as_secs_f32()` | `(now - t0).as_millis() as f32 / 1000.0` |
| `f32::sin()`, `f32::cos()` | `libm::sinf()`, `libm::cosf()` |
| `f32::asin()` | `libm::asinf()` |
| `var.sqrt()` | `libm::sqrtf(var)` |
| `Box::leak(Box::new(Queue::new()))` | Same (requires `alloc`) |
| `Arc<Mutex<CommandProducer>>` | `&'static Mutex<CriticalSectionRawMutex, CommandProducer>` |

The `MotorController` struct, `compute_cycle()`, waveform generators, streaming source, and all motion math remain **identical** in logic.

---

## Changes to `src/motor.rs`

```rust
// Current:
use anyhow::Result;

// Target:
use crate::error::Result;
```

The async trait itself is unchanged.
