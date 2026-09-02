#![no_std]
#![no_main]
#![recursion_limit = "512"]

extern crate alloc;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
esp_bootloader_esp_idf::esp_app_desc!();

use esp_hal::clock::CpuClock;
use esp_hal::interrupt::software::SoftwareInterruptControl;
#[cfg(feature = "esp32c6")]
use esp_hal::interrupt::Priority;
use esp_hal::timer::timg::TimerGroup;
#[cfg(feature = "esp32c6")]
use esp_rtos::embassy::InterruptExecutor;
use static_cell::StaticCell;

mod ble_api;
mod buffers;
mod command;
mod console;
mod context;
mod error;
mod http_api;
mod hw_paths;
mod modbus_relay;
mod modbus_rtu;
mod motion;
mod motor;
mod motor_57aim30;
mod rpc;
mod storage;
mod wifi;

use context::AppContext;
#[cfg(feature = "esp32c6")]
use motion::CommandConsumer;
#[cfg(feature = "esp32c6")]
use storage::PinConfiguration;

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(size: 96 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    console::init_logger();
    log::info!("OSSM Rust firmware starting (esp-hal)");

    // Console owns USB Serial/JTAG + UART0 (DevKit USB-UART bridge pins).
    #[cfg(feature = "esp32c6")]
    let uart_pins = console::UartPins::new(peripherals.GPIO16, peripherals.GPIO17);
    #[cfg(feature = "esp32s3")]
    let uart_pins = console::UartPins::new(peripherals.GPIO43, peripherals.GPIO44);

    spawner.spawn(console_task(peripherals.USB_DEVICE, peripherals.UART0, uart_pins).unwrap());

    let init = context::init_app_context(peripherals.FLASH);
    let app_context = init.ctx;
    let motion_consumer = init.motion_consumer;

    spawner.spawn(context::storage_task(init.storage_manager, init.storage_cmd).unwrap());

    let pin_config = app_context.storage.pin();
    let ble_enabled = pin_config.ble_enabled;
    let rtu_relay = pin_config.is_rtu_relay();

    if rtu_relay {
        log::info!("Boot mode: rtu_relay (Modbus bridge; motor controller disabled)");
    } else {
        log::info!("Boot mode: servo (motor controller)");
    }

    #[cfg(feature = "esp32c6")]
    {
        let motor_spawner = {
            static MOTOR_EXEC: StaticCell<InterruptExecutor<1>> = StaticCell::new();
            let exec = MOTOR_EXEC.init(InterruptExecutor::new(sw_interrupt.software_interrupt1));
            exec.start(Priority::Priority3)
        };
        if rtu_relay {
            motor_spawner.spawn(
                relay_task(
                    peripherals.UART1,
                    peripherals.UHCI0,
                    peripherals.DMA_CH0,
                    pin_config,
                )
                .unwrap(),
            );
            // Keep the unused motion consumer alive for AppContext lifetime.
            #[allow(clippy::forget_non_drop)]
            core::mem::forget(motion_consumer);
        } else {
            motor_spawner.spawn(
                motor_task(
                    app_context,
                    motion_consumer,
                    peripherals.UART1,
                    peripherals.UHCI0,
                    peripherals.DMA_CH0,
                    pin_config,
                )
                .unwrap(),
            );
        }
    }

    #[cfg(feature = "esp32s3")]
    {
        use esp_hal::system::Stack;

        static CORE1_STACK: StaticCell<Stack<{ 24 * 1024 }>> = StaticCell::new();
        let stack = CORE1_STACK.init(Stack::new());
        let uart = peripherals.UART1;
        let uhci = peripherals.UHCI0;
        let dma_ch = peripherals.DMA_CH0;
        if rtu_relay {
            core::mem::forget(motion_consumer);
            esp_rtos::start_second_core(
                peripherals.CPU_CTRL,
                sw_interrupt.software_interrupt1,
                stack,
                move || {
                    modbus_relay::run_relay_blocking(uart, uhci, dma_ch, pin_config);
                },
            );
        } else {
            esp_rtos::start_second_core(
                peripherals.CPU_CTRL,
                sw_interrupt.software_interrupt1,
                stack,
                move || {
                    motor_57aim30::run_motor_blocking(
                        app_context,
                        motion_consumer,
                        uart,
                        uhci,
                        dma_ch,
                        pin_config,
                    );
                },
            );
        }
    }

    spawner.spawn(cli_task(app_context).unwrap());
    if !rtu_relay {
        spawner.spawn(nvs_saver_task(app_context).unwrap());
    }

    // Hold BT until after WiFi/HTTP are up so association and TCP listen are
    // established before BLE radio contention begins.
    let bt_for_later = if ble_enabled {
        Some(peripherals.BT)
    } else {
        None
    };

    let net_config = app_context.storage.net();
    if net_config.wifi_enabled {
        let stack = wifi::start_wifi(peripherals.WIFI, app_context, &spawner).await;
        http_api::run_server(stack, app_context, &spawner, rtu_relay).await;
    } else {
        log::info!("WiFi is disabled in NetworkConfiguration.");
        if rtu_relay {
            log::warn!("RTU relay network endpoints require WiFi to be enabled.");
        }
    }

    if let Some(bt) = bt_for_later {
        log::info!("Starting BLE after WiFi/HTTP initialization");
        spawner.spawn(ble_task(bt, app_context).unwrap());
    }

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}

#[embassy_executor::task]
async fn console_task(
    usb: esp_hal::peripherals::USB_DEVICE<'static>,
    uart0: esp_hal::peripherals::UART0<'static>,
    pins: console::UartPins,
) {
    console::run(usb, uart0, pins).await;
}

#[cfg(feature = "esp32c6")]
#[embassy_executor::task]
async fn motor_task(
    app_context: AppContext,
    motion_consumer: CommandConsumer,
    uart: esp_hal::peripherals::UART1<'static>,
    uhci: esp_hal::peripherals::UHCI0<'static>,
    dma_ch: esp_hal::peripherals::DMA_CH0<'static>,
    pin_config: PinConfiguration,
) {
    if let Err(e) =
        motor_57aim30::run_motor(app_context, motion_consumer, uart, uhci, dma_ch, pin_config).await
    {
        log::error!("Motor task failed: {}", e);
    }
}

#[cfg(feature = "esp32c6")]
#[embassy_executor::task]
async fn relay_task(
    uart: esp_hal::peripherals::UART1<'static>,
    uhci: esp_hal::peripherals::UHCI0<'static>,
    dma_ch: esp_hal::peripherals::DMA_CH0<'static>,
    pin_config: PinConfiguration,
) {
    if let Err(e) = modbus_relay::run_relay(uart, uhci, dma_ch, pin_config).await {
        log::error!("Modbus relay task failed: {}", e);
    }
}

#[embassy_executor::task]
async fn ble_task(bt: esp_hal::peripherals::BT<'static>, app_context: AppContext) {
    ble_api::run_ble_server(bt, app_context).await;
}

#[embassy_executor::task]
async fn cli_task(app_context: AppContext) {
    command::handle_cli(app_context).await;
}

#[embassy_executor::task]
async fn nvs_saver_task(app_context: AppContext) {
    let mut last_saved: Option<crate::motion::MotorControllerConfig> = None;

    loop {
        // Longer debounce: flash erase during an active BLE/WiFi session
        // starves the RF controller and drops connections.
        Timer::after(Duration::from_millis(2000)).await;
        let config = app_context.load_snapshot().config.clone();
        let should_save = match &last_saved {
            None => {
                last_saved = Some(config.clone());
                false
            }
            Some(prev) if persistent_motor_changed(prev, &config) => {
                last_saved = Some(config.clone());
                true
            }
            Some(_) => false,
        };
        if should_save {
            log::info!("Saving motor config to NVS");
            app_context.storage.set_motor_config(config);
        }
    }
}

/// Pause toggles are ephemeral — flashing NVS for them mid-BLE is harmful.
fn persistent_motor_changed(
    a: &crate::motion::MotorControllerConfig,
    b: &crate::motion::MotorControllerConfig,
) -> bool {
    a.bpm != b.bpm
        || a.depth != b.depth
        || a.depth_top != b.depth_top
        || a.reversed != b.reversed
        || a.wave_func != b.wave_func
        || (a.sharpness - b.sharpness).abs() > f32::EPSILON
        || a.spline_points != b.spline_points
        || a.streaming != b.streaming
}
