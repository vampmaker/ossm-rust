# 02: Entry Point & Initialization

## Scope

Replace `fn main()` with `#![no_std]` + `#[esp_rtos::main]`, initialize `esp-rtos`, dual heap allocators, shared radio controller, `InterruptExecutor` for motor, and thread executor for everything else.

## Files to Modify

- `src/main.rs` (complete rewrite of initialization; motor loop logic preserved in `motor_57aim30.rs`)

---

## Target Entry Point

```rust
#![no_std]
#![no_main]

extern crate alloc;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::timer::timg::TimerGroup;
use esp_rtos::embassy::InterruptExecutor;
use esp_rtos::Priority;
use static_cell::StaticCell;

mod ble_api;
mod command;
mod context;
mod error;
mod http_api;
mod motion;
mod motor;
mod motor_57aim30;
mod rpc;
mod storage;
mod wifi;

use context::AppContext;

// High-priority interrupt executor for the motor loop
static MOTOR_EXECUTOR: InterruptExecutor<1> = InterruptExecutor::new();
// Shared radio controller (WiFi + BLE coexistence)
static RADIO: StaticCell<esp_radio::Controller<'static>> = StaticCell::new();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Two heap regions (esp-radio recommendation)
    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    log::info!("OSSM Rust firmware starting (esp-hal)");

    // Storage (must be early — WiFi/BLE/motor read NVS at boot)
    let storage_ref: &'static _ = {
        static STORAGE: StaticCell<embassy_sync::mutex::Mutex<
            embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
            storage::StorageManager,
        >> = StaticCell::new();
        STORAGE.init(embassy_sync::mutex::Mutex::new(
            storage::StorageManager::new(peripherals.FLASH),
        ))
    };

    let motor_controller_ref: &'static _ = {
        static MC: StaticCell<embassy_sync::mutex::Mutex<
            embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
            Option<motion::MotorController>,
        >> = StaticCell::new();
        MC.init(embassy_sync::mutex::Mutex::new(None))
    };

    let app_context = AppContext {
        storage: storage_ref,
        motor_controller: motor_controller_ref,
    };

    // Initialize shared radio controller ONCE (before WiFi or BLE)
    let radio = RADIO.init(
        esp_radio::init().expect("esp_radio::init failed"),
    );

    let pin_config = {
        let sm = app_context.storage.lock().await;
        sm.get_pin_configuration().unwrap_or_default()
    };

    // --- Motor on interrupt executor (high priority) ---
    #[cfg(feature = "esp32c6")]
    {
        let motor_spawner = MOTOR_EXECUTOR.start(
            sw_interrupt.software_interrupt1,
            Priority::Priority3, // highest available; tune after profiling
        );
        motor_spawner.must_spawn(motor_task(
            app_context,
            peripherals.UART1,
            pin_config,
        ));
    }

    // --- Thread-executor tasks ---
    spawner.must_spawn(wifi_task(
        radio,
        peripherals.WIFI,
        app_context,
        spawner,
    ));

    spawner.must_spawn(nvs_saver_task(app_context));

    if pin_config.ble_enabled {
        spawner.must_spawn(ble_task(
            radio,
            peripherals.BT,
            app_context,
        ));
    }

    spawner.must_spawn(cli_task(
        peripherals.USB_DEVICE,
        app_context,
    ));

    // ESP32-S3: motor on Core 1 (see below)
    #[cfg(feature = "esp32s3")]
    {
        esp_rtos::start_second_core(
            sw_interrupt.software_interrupt2,
            move || {
                motor_57aim30::run_motor_blocking(app_context, peripherals.UART1, pin_config);
            },
        );
    }

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
```

> **Peripheral naming**: USB Serial JTAG is `peripherals.USB_DEVICE` in esp-hal 1.1 (not `USB_SERIAL_JTAG`).

---

## Task Declarations

```rust
#[embassy_executor::task]
async fn motor_task(
    app_context: AppContext,
    uart: esp_hal::peripherals::UART1,
    pin_config: storage::PinConfiguration,
) {
    if let Err(e) = motor_57aim30::run_motor(app_context, uart, pin_config).await {
        log::error!("Motor task failed: {}", e);
    }
}

#[embassy_executor::task]
async fn wifi_task(
    radio: &'static esp_radio::Controller<'static>,
    wifi_peripheral: esp_hal::peripherals::WIFI,
    app_context: AppContext,
    spawner: Spawner,
) {
    wifi::start_wifi(radio, wifi_peripheral, app_context, &spawner).await;
}

#[embassy_executor::task]
async fn ble_task(
    radio: &'static esp_radio::Controller<'static>,
    bt_peripheral: esp_hal::peripherals::BT,
    app_context: AppContext,
) {
    ble_api::run_ble_server(radio, bt_peripheral, app_context).await;
}

#[embassy_executor::task]
async fn cli_task(
    usb: esp_hal::peripherals::USB_DEVICE,
    app_context: AppContext,
) {
    command::handle_cli(usb, app_context).await;
}

#[embassy_executor::task]
async fn nvs_saver_task(app_context: AppContext) {
    // Initialize with current version once available so we don't save unchanged initial state
    let mut last_saved_version = None;
    loop {
        Timer::after(Duration::from_millis(500)).await;
        let to_save = {
            let mc_opt = app_context.motor_controller.lock().await;
            if let Some(mc) = mc_opt.as_ref() {
                let ver = mc.get_config_version();
                match last_saved_version {
                    None => {
                        last_saved_version = Some(ver);
                        None
                    }
                    Some(last) if ver != last => {
                        last_saved_version = Some(ver);
                        Some(mc.get_config())
                    }
                    _ => None,
                }
            } else {
                None
            }
        };
        if let Some(config) = to_save {
            log::info!("Saving motor config to NVS");
            let mut sm = app_context.storage.lock().await;
            if let Err(e) = sm.set_motor_config(&config) {
                log::error!("Failed to save motor config: {}", e);
            }
        }
    }
}
```

> **nvs_saver fix**: The old `last_saved_version != 0` guard skipped the first config change after boot. Initialize `last_saved_version` with the first discovered `get_config_version()` so subsequent changes are persisted properly without triggering an immediate redundant NVS write on boot.

---

## ESP32-S3 Core Pinning

On ESP32-S3, the motor loop runs on Core 1 via `esp_rtos::start_second_core()`. Async UART drivers cannot cross cores — `into_async()` must be called inside the Core 1 closure. Using a dedicated static `embassy_executor::Executor` on Core 1 ensures full timer/waker integration:

```rust
static CORE1_EXECUTOR: StaticCell<embassy_executor::Executor> = StaticCell::new();

#[embassy_executor::task]
async fn core1_motor_task(
    app_context: AppContext,
    uart: esp_hal::peripherals::UART1,
    pin_config: PinConfiguration,
) {
    if let Err(e) = run_motor(app_context, uart, pin_config).await {
        log::error!("Motor task on Core 1 failed: {}", e);
    }
}

pub fn run_motor_blocking(
    app_context: AppContext,
    uart: esp_hal::peripherals::UART1,
    pin_config: PinConfiguration,
) {
    let executor = CORE1_EXECUTOR.init(embassy_executor::Executor::new());
    executor.run(|spawner| {
        spawner.spawn(core1_motor_task(app_context, uart, pin_config)).ok();
    });
}
```

On S3, Core 1 isolation provides the jitter benefit (WiFi runs on Core 0).

---

## Shared Radio Initialization (WiFi + BLE)

OSSM runs WiFi and BLE simultaneously. Both must share a single `esp_radio::init()` initialized with the required hardware timer and RNG peripherals:

```rust
// In main(), pass system timer and RNG peripherals required by esp-radio:
let radio_init = esp_radio::init(peripherals.TIMG1, peripherals.RNG, peripherals.RADIO_CLK)
    .expect("Failed to initialize esp-radio");
let radio = RADIO.init(radio_init);

// WiFi (in wifi.rs):
let controller = WifiController::new(radio, wifi_peripheral, Default::default())?;

// BLE (in ble_api.rs):
use esp_radio::ble::controller::BleConnector;
let connector = BleConnector::new(radio, bt_peripheral, Default::default())?;
```

Requires `coex` feature on `esp-radio` (see [01-build-system](01-build-system.md)).

---

## InterruptExecutor vs Thread Executor

| Executor | Tasks | Priority |
|---|---|---|
| `InterruptExecutor` (SW int) | `motor_task` | High — preempts thread executor |
| Thread executor (`#[esp_rtos::main]`) | wifi, http, ble, cli, nvs_saver | Normal |

Cross-executor locking uses `embassy_sync::mutex::Mutex<CriticalSectionRawMutex, T>`. The motor holds the lock for <5 µs (pure `compute_cycle()`); HTTP/BLE tasks yield via `.lock().await`.

---

## Removed Constructs

| Current | Replacement |
|---|---|
| `esp_idf_svc::sys::link_patches()` | Not needed |
| `EspLogger::initialize_default()` | `esp-println` `log` feature (see [10-logging-panic](10-logging-panic.md)) |
| `EspSystemEventLoop::take()` | Not needed |
| `EspDefaultNvsPartition::take()` | `esp_storage::FlashStorage` via [07-storage](07-storage.md) |
| `MountedEventfs::mount(5)` | Not needed |
| `BlockingStdIo::usb_serial(...)` | `esp_hal::usb_serial_jtag::UsbSerialJtag` |
| `std::thread::Builder` for http/ble/cli | Embassy tasks on thread executor |
| `EXECUTOR.init(Executor::new())` + `executor.run()` | `#[esp_rtos::main]` |
| `vTaskPrioritySet(null, 15)` | `InterruptExecutor` at `Priority::Priority3` |
| `ThreadSpawnConfiguration` (Core 1) | `esp_rtos::start_second_core` (S3 only) |

---

## GPIO Pin Pool

The `all_pins: Arc<Mutex<Vec<Option<AnyIOPin>>>>` pattern is replaced by `GpioPins` (see [03-motor-uart](03-motor-uart.md)). Pin selection happens once at motor init, with fallback auto-discovery when NVS pins are unavailable.
