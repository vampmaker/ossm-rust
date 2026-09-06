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
use crate::peripheral_bank::PeripheralBank;
use crate::rs485;
use crate::storage::{OperatingMode, PinConfiguration, OPERATING_MODE_SERVO};

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

fn drain_motion(consumer: &mut CommandConsumer) {
    while consumer.dequeue().is_some() {}
}

pub async fn run_uart_owner(
    app_context: AppContext,
    mut motion_consumer: CommandConsumer,
    mut bank: PeripheralBank,
) {
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

        let tx = pin.modbus_tx as u8;
        let rx = pin.modbus_rx as u8;
        let de = pin.modbus_de_re as u8;
        if let Err(e) = run_role(app_context, &mut motion_consumer, &pin, mode, &mut bank).await {
            log::error!("UART1 role {:?} failed: {}", mode, e);
        }
        bank.release_role(tx, rx, de);

        console::set_usb_mux_pipe(false);
        modbus_relay::mark_active(false);
        rs485::mark_active(false);
        if rs485::take_magic_exit() {
            let mut pin = app_context.storage.pin();
            pin.operating_mode = alloc::string::String::from(OPERATING_MODE_SERVO);
            app_context.storage.set_pin(pin);
            console::write_line("shell.operating_mode set to servo");
            log::info!("shell.operating_mode set to servo");
        }
        Timer::after(Duration::from_millis(20)).await;
    }
}

async fn run_role(
    app_context: AppContext,
    motion_consumer: &mut CommandConsumer,
    pin: &PinConfiguration,
    mode: OperatingMode,
    bank: &mut PeripheralBank,
) -> Result<()> {
    match mode {
        OperatingMode::Servo => {
            let mut engine = Engine::new(app_context.storage.motor_config());
            engine.flush_snapshot();
            app_context.update_snapshot(engine.snapshot());
            log::info!("UART1 servo: travel homing (boot or mode switch)");
            let master = init_uart_and_modbus(pin, bank)?;
            let motor = motor_57aim30::Modbus57AIM30Motor::new(master, pin.modbus_scan_delay_us);
            motor_57aim30::run_motor(app_context, motion_consumer, &mut engine, motor).await
        }
        OperatingMode::RtuRelay => {
            let master = init_uart_and_modbus(pin, bank)?;
            modbus_relay::run_relay(master).await
        }
        OperatingMode::Rs485 => rs485::run_rs485(pin, bank).await,
    }
}
