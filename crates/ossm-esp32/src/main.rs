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
#[cfg(feature = "esp32c6")]
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
mod peripheral_bank;
mod rpc;
mod rs485;
mod storage;
mod uart_owner;
mod wifi;

use context::AppContext;
#[cfg(feature = "esp32c6")]
use motion::CommandConsumer;
use peripheral_bank::PeripheralBank;

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

    let mut peripheral_bank =
        PeripheralBank::new(peripherals.UART1, peripherals.UHCI0, peripherals.DMA_CH0);
    #[cfg(feature = "esp32c6")]
    peripheral_bank_insert_pins!(
        peripheral_bank,
        peripherals,
        GPIO0,
        GPIO1,
        GPIO2,
        GPIO3,
        GPIO4,
        GPIO5,
        GPIO6,
        GPIO7,
        GPIO8,
        GPIO9,
        GPIO10,
        GPIO11,
        GPIO12,
        GPIO13,
        GPIO14,
        GPIO15,
        GPIO18,
        GPIO19,
        GPIO20,
        GPIO21,
        GPIO22,
        GPIO23,
        GPIO24,
        GPIO25,
        GPIO26,
        GPIO27,
        GPIO28,
        GPIO29,
        GPIO30
    );
    #[cfg(feature = "esp32s3")]
    peripheral_bank_insert_pins!(
        peripheral_bank,
        peripherals,
        GPIO0,
        GPIO1,
        GPIO2,
        GPIO3,
        GPIO4,
        GPIO5,
        GPIO6,
        GPIO7,
        GPIO8,
        GPIO9,
        GPIO10,
        GPIO11,
        GPIO12,
        GPIO13,
        GPIO14,
        GPIO15,
        GPIO16,
        GPIO17,
        GPIO18,
        GPIO19,
        GPIO20,
        GPIO21,
        GPIO26,
        GPIO27,
        GPIO28,
        GPIO29,
        GPIO30,
        GPIO31,
        GPIO32,
        GPIO33,
        GPIO34,
        GPIO35,
        GPIO36,
        GPIO37,
        GPIO38,
        GPIO39,
        GPIO40,
        GPIO41,
        GPIO42,
        GPIO45,
        GPIO46,
        GPIO47,
        GPIO48
    );

    let init = context::init_app_context(peripherals.FLASH);
    let app_context = init.ctx;
    let motion_consumer = init.motion_consumer;

    spawner.spawn(context::storage_task(init.storage_manager, init.storage_cmd).unwrap());

    let pin_config = app_context.storage.pin();
    let ble_enabled = pin_config.ble_enabled;
    log::info!("Boot UART1 mode: {:?}", pin_config.mode());

    #[cfg(feature = "esp32c6")]
    {
        let motor_spawner = {
            static MOTOR_EXEC: StaticCell<InterruptExecutor<1>> = StaticCell::new();
            let exec = MOTOR_EXEC.init(InterruptExecutor::new(sw_interrupt.software_interrupt1));
            exec.start(Priority::Priority3)
        };
        motor_spawner
            .spawn(uart_owner_task(app_context, motion_consumer, peripheral_bank).unwrap());
    }

    #[cfg(feature = "esp32s3")]
    {
        use core::mem::MaybeUninit;
        use esp_hal::system::Stack;

        // Reclaimed 2nd-stage bootloader DRAM (~72 KiB). Keeps the Core-1 stack
        // out of RWDATA so `.stack` does not overflow `dram_seg`.
        #[unsafe(link_section = ".dram2_uninit")]
        static mut CORE1_STACK: MaybeUninit<Stack<{ 24 * 1024 }>> = MaybeUninit::uninit();
        // SAFETY: dram2 is unused after the bootloader; this block runs once.
        let stack = unsafe { (*(&raw mut CORE1_STACK)).write(Stack::new()) };
        esp_rtos::start_second_core(
            peripherals.CPU_CTRL,
            sw_interrupt.software_interrupt1,
            stack,
            move || {
                motor_57aim30::run_uart_owner_blocking(
                    app_context,
                    motion_consumer,
                    peripheral_bank,
                );
            },
        );
    }

    spawner.spawn(cli_task(app_context).unwrap());
    spawner.spawn(nvs_saver_task(app_context).unwrap());

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
        http_api::run_server(stack, app_context, &spawner).await;
    } else {
        log::info!("WiFi is disabled in NetworkConfiguration.");
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
async fn uart_owner_task(
    app_context: AppContext,
    motion_consumer: CommandConsumer,
    peripheral_bank: PeripheralBank,
) {
    uart_owner::run_uart_owner(app_context, motion_consumer, peripheral_bank).await;
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
        if !app_context.storage.pin().is_servo() {
            continue;
        }
        let config = app_context.load_snapshot().config;
        let should_save = match &last_saved {
            None => {
                last_saved = Some(config);
                false
            }
            Some(prev) if prev.persistent_motor_changed(&config) => {
                last_saved = Some(config);
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
