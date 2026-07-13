#![no_std]
#![no_main]
#![recursion_limit = "512"]

extern crate alloc;

use esp_backtrace as _;
esp_bootloader_esp_idf::esp_app_desc!();

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::timer::timg::TimerGroup;
#[cfg(feature = "esp32c6")]
use esp_hal::interrupt::Priority;
#[cfg(feature = "esp32c6")]
use esp_rtos::embassy::InterruptExecutor;
use static_cell::StaticCell;

mod ble_api;
mod buffers;
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

use context::{AppContext, CsMutex};
use motion::MotorController;
use storage::{PinConfiguration, StorageManager};

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(size: 96 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    esp_println::logger::init_logger_from_env();
    log::set_max_level(log::LevelFilter::Info);
    log::info!("OSSM Rust firmware starting (esp-hal)");

    let storage_ref: &'static CsMutex<StorageManager> = {
        static STORAGE: StaticCell<CsMutex<StorageManager>> = StaticCell::new();
        STORAGE.init(embassy_sync::mutex::Mutex::new(StorageManager::new(
            peripherals.FLASH,
        )))
    };

    let motor_controller_ref: &'static CsMutex<Option<MotorController>> = {
        static MC: StaticCell<CsMutex<Option<MotorController>>> = StaticCell::new();
        MC.init(embassy_sync::mutex::Mutex::new(None))
    };

    let app_context = AppContext {
        storage: storage_ref,
        motor_controller: motor_controller_ref,
    };

    let pin_config = {
        let mut sm = app_context.storage.lock().await;
        sm.get_pin_configuration().unwrap_or_default()
    };

    let ble_enabled = pin_config.ble_enabled;

    #[cfg(feature = "esp32c6")]
    {
        let motor_spawner = {
            static MOTOR_EXEC: StaticCell<InterruptExecutor<1>> = StaticCell::new();
            let exec = MOTOR_EXEC.init(InterruptExecutor::new(sw_interrupt.software_interrupt1));
            exec.start(Priority::Priority3)
        };
        motor_spawner.spawn(motor_task(app_context, peripherals.UART1, pin_config).unwrap());
    }

    #[cfg(feature = "esp32s3")]
    {
        use esp_hal::system::Stack;

        static CORE1_STACK: StaticCell<Stack<{ 32 * 1024 }>> = StaticCell::new();
        let stack = CORE1_STACK.init(Stack::new());
        let uart = peripherals.UART1;
        esp_rtos::start_second_core(
            peripherals.CPU_CTRL,
            sw_interrupt.software_interrupt1,
            stack,
            move || {
                motor_57aim30::run_motor_blocking(app_context, uart, pin_config);
            },
        );
    }

    spawner.spawn(cli_task(peripherals.USB_DEVICE, app_context).unwrap());
    spawner.spawn(nvs_saver_task(app_context).unwrap());

    if ble_enabled {
        spawner.spawn(ble_task(peripherals.BT, app_context).unwrap());
    }

    let net_config = {
        let mut sm = app_context.storage.lock().await;
        sm.get_network_configuration().unwrap_or_default()
    };
    if net_config.wifi_enabled {
        let stack = wifi::start_wifi(peripherals.WIFI, app_context, &spawner).await;
        http_api::run_server(stack, app_context, &spawner).await;
    } else {
        log::info!("WiFi is disabled in NetworkConfiguration.");
    }

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}

#[allow(dead_code)]
#[embassy_executor::task]
async fn motor_task(
    app_context: AppContext,
    uart: esp_hal::peripherals::UART1<'static>,
    pin_config: PinConfiguration,
) {
    if let Err(e) = motor_57aim30::run_motor(app_context, uart, pin_config).await {
        log::error!("Motor task failed: {}", e);
    }
}

#[embassy_executor::task]
async fn ble_task(bt: esp_hal::peripherals::BT<'static>, app_context: AppContext) {
    ble_api::run_ble_server(bt, app_context).await;
}

#[embassy_executor::task]
async fn cli_task(usb: esp_hal::peripherals::USB_DEVICE<'static>, app_context: AppContext) {
    command::handle_cli(usb, app_context).await;
}

#[embassy_executor::task]
async fn nvs_saver_task(app_context: AppContext) {
    let mut last_saved_version = None::<u32>;

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
