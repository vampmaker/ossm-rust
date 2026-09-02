//! Exclusive UART1 owner: servo / rtu_relay / rs485 switch at runtime.

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use ossm_core::Engine;

use crate::console;
use crate::context::AppContext;
use crate::error::Result;
use crate::modbus_relay;
use crate::motion::CommandConsumer;
use crate::motor_57aim30::{self, init_uart_and_modbus};
use crate::rs485;
use crate::storage::{OperatingMode, PinConfiguration};

static STOP_FLAG: AtomicBool = AtomicBool::new(false);
static STOP_SIG: Signal<CriticalSectionRawMutex, ()> = Signal::new();

pub fn request_restart() {
    STOP_FLAG.store(true, Ordering::Release);
    STOP_SIG.signal(());
}

pub fn stop_requested() -> bool {
    STOP_FLAG.load(Ordering::Acquire)
}

pub fn clear_stop() {
    STOP_FLAG.store(false, Ordering::Release);
    let _ = STOP_SIG.try_take();
}

pub async fn wait_stop() {
    if stop_requested() {
        return;
    }
    STOP_SIG.wait().await;
}

/// Yield in slices so a mode switch is not stuck behind a long Timer.
pub async fn sleep_or_stop(ms: u64) -> bool {
    let mut left = ms;
    while left > 0 {
        if stop_requested() {
            return true;
        }
        let slice = left.min(50);
        Timer::after(Duration::from_millis(slice)).await;
        left -= slice;
    }
    stop_requested()
}

fn steal_uart_peripherals() -> (
    esp_hal::peripherals::UART1<'static>,
    esp_hal::peripherals::UHCI0<'static>,
    esp_hal::peripherals::DMA_CH0<'static>,
) {
    unsafe {
        (
            esp_hal::peripherals::UART1::steal(),
            esp_hal::peripherals::UHCI0::steal(),
            esp_hal::peripherals::DMA_CH0::steal(),
        )
    }
}

fn drain_motion(consumer: &mut CommandConsumer) {
    while consumer.dequeue().is_some() {}
}

pub async fn run_uart_owner(
    app_context: AppContext,
    mut motion_consumer: CommandConsumer,
    uart: esp_hal::peripherals::UART1<'static>,
    uhci: esp_hal::peripherals::UHCI0<'static>,
    dma: esp_hal::peripherals::DMA_CH0<'static>,
) {
    let mut held = Some((uart, uhci, dma));
    loop {
        clear_stop();
        drain_motion(&mut motion_consumer);
        let pin = app_context.storage.pin();
        let mode = pin.mode();
        log::info!("UART1 owner: starting mode {:?}", mode);

        match mode {
            OperatingMode::Rs485 => console::set_usb_mux_pipe(true),
            _ => console::set_usb_mux_pipe(false),
        }

        let (uart, uhci, dma) = held.take().unwrap_or_else(steal_uart_peripherals);
        if let Err(e) = run_role(
            app_context,
            &mut motion_consumer,
            uart,
            uhci,
            dma,
            &pin,
            mode,
        )
        .await
        {
            log::error!("UART1 role {:?} failed: {}", mode, e);
        }

        console::set_usb_mux_pipe(false);
        modbus_relay::mark_active(false);
        rs485::mark_active(false);
        Timer::after(Duration::from_millis(20)).await;
    }
}

async fn run_role(
    app_context: AppContext,
    motion_consumer: &mut CommandConsumer,
    uart: esp_hal::peripherals::UART1<'static>,
    uhci: esp_hal::peripherals::UHCI0<'static>,
    dma: esp_hal::peripherals::DMA_CH0<'static>,
    pin: &PinConfiguration,
    mode: OperatingMode,
) -> Result<()> {
    match mode {
        OperatingMode::Servo => {
            let mut engine = Engine::new(app_context.storage.motor_config());
            engine.flush_snapshot();
            app_context.update_snapshot(engine.snapshot());
            log::info!("UART1 servo: travel homing (boot or mode switch)");
            let master = init_uart_and_modbus(uart, uhci, dma, pin)?;
            let motor = motor_57aim30::Modbus57AIM30Motor::new(master, pin.modbus_scan_delay_us);
            motor_57aim30::run_motor(app_context, motion_consumer, &mut engine, motor).await
        }
        OperatingMode::RtuRelay => {
            let master = init_uart_and_modbus(uart, uhci, dma, pin)?;
            modbus_relay::run_relay(master).await
        }
        OperatingMode::Rs485 => {
            let _uhci = uhci;
            let _dma = dma;
            rs485::run_rs485(uart, pin).await
        }
    }
}
