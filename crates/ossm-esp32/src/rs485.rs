//! Raw RS-485 byte pipe (`operating_mode = rs485`).
//!
//! Host owns RTU timing. USB ACM and `/ws/rs485` are data planes; UART1 + DE/RE is the bus.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use embassy_futures::select::{select, select4, Either4};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Timer};
use esp_hal::gpio::{AnyPin, Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::uart::{Config, Uart};
use heapless::Vec as HVec;

use crate::console;
use crate::error::{FirmwareError, Result};
use crate::storage::PinConfiguration;
use crate::uart_owner;

pub const PKT_TX: u8 = 0;
pub const PKT_RX: u8 = 1;
pub const PKT_CFG: u8 = 2;
const FRAME_CAP: usize = 256;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static LIVE_BAUD: AtomicU32 = AtomicU32::new(115_200);
static TX_CH: Channel<CriticalSectionRawMutex, HVec<u8, FRAME_CAP>, 4> = Channel::new();
static BAUD_CH: Channel<CriticalSectionRawMutex, u32, 2> = Channel::new();
static RX_CH: [Channel<CriticalSectionRawMutex, HVec<u8, FRAME_CAP>, 4>; 2] =
    [Channel::new(), Channel::new()];
static RX_CLAIMED: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];

pub fn is_active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

pub fn mark_active(active: bool) {
    ACTIVE.store(active, Ordering::Relaxed);
}

pub fn live_baud() -> u32 {
    LIVE_BAUD.load(Ordering::Relaxed)
}

pub fn set_live_baud(baud: u32) {
    if baud >= 1200 && baud <= 3_000_000 {
        LIVE_BAUD.store(baud, Ordering::Relaxed);
        let _ = BAUD_CH.try_send(baud);
    }
}

pub fn claim_rx_slot() -> Option<usize> {
    for (i, claimed) in RX_CLAIMED.iter().enumerate() {
        if claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            while RX_CH[i].try_receive().is_ok() {}
            return Some(i);
        }
    }
    None
}

pub fn release_rx_slot(slot: usize) {
    if let Some(flag) = RX_CLAIMED.get(slot) {
        flag.store(false, Ordering::Release);
        while RX_CH[slot].try_receive().is_ok() {}
    }
}

pub async fn recv_rx(slot: usize) -> HVec<u8, FRAME_CAP> {
    RX_CH[slot].receive().await
}

pub fn try_send_tx(payload: &[u8]) -> Result<()> {
    if !is_active() {
        return Err(FirmwareError::Modbus("rs485 inactive"));
    }
    if payload.is_empty() || payload.len() > FRAME_CAP {
        return Err(FirmwareError::Modbus("bad rs485 len"));
    }
    let mut v = HVec::new();
    v.extend_from_slice(payload)
        .map_err(|_| FirmwareError::Modbus("rs485 tx full"))?;
    TX_CH.try_send(v)
        .map_err(|_| FirmwareError::QueueFull)
}

pub fn apply_cfg_json(json: &[u8]) -> Result<u32> {
    #[derive(serde::Deserialize)]
    struct Cfg {
        baud: u32,
    }
    let cfg: Cfg = serde_json::from_slice(json).map_err(|_| FirmwareError::Json)?;
    if !(1200..=3_000_000).contains(&cfg.baud) {
        return Err(FirmwareError::Config("baud"));
    }
    set_live_baud(cfg.baud);
    Ok(cfg.baud)
}

pub fn encode_packet(typ: u8, payload: &[u8]) -> HVec<u8, 260> {
    let mut out = HVec::new();
    let _ = out.push(typ);
    let len = payload.len() as u16;
    let _ = out.extend_from_slice(&len.to_le_bytes());
    let n = payload.len().min(FRAME_CAP);
    let _ = out.extend_from_slice(&payload[..n]);
    out
}

pub fn decode_packet(frame: &[u8]) -> Option<(u8, &[u8])> {
    if frame.len() < 3 {
        return None;
    }
    let typ = frame[0];
    let len = u16::from_le_bytes([frame[1], frame[2]]) as usize;
    if frame.len() < 3 + len {
        return None;
    }
    Some((typ, &frame[3..3 + len]))
}

fn fanout_rx(data: &[u8]) {
    if data.is_empty() {
        return;
    }
    console::pipe_push_bus(data);
    for (i, claimed) in RX_CLAIMED.iter().enumerate() {
        if !claimed.load(Ordering::Relaxed) {
            continue;
        }
        let mut v = HVec::new();
        if v.extend_from_slice(data).is_ok() {
            let _ = RX_CH[i].try_send(v);
        }
    }
}

fn uart_config(baud: u32) -> Config {
    // 2 symbol times (~174 µs at 115200) so read_async returns a whole RTU burst
    // instead of the first byte.
    Config::default()
        .with_baudrate(baud.max(1200))
        .with_rx(esp_hal::uart::RxConfig::default().with_timeout(2))
}

pub async fn run_rs485(
    uart_periph: esp_hal::peripherals::UART1<'static>,
    pin: &PinConfiguration,
) -> Result<()> {
    log::info!("Starting RS-485 transceiver (operating_mode=rs485)");
    let baud = if pin.modbus_baud == 0 {
        115_200
    } else {
        pin.modbus_baud
    };
    LIVE_BAUD.store(baud, Ordering::Relaxed);

    let tx_pin = unsafe { AnyPin::steal(pin.modbus_tx as u8) };
    let rx_pin = Input::new(
        unsafe { AnyPin::steal(pin.modbus_rx as u8) },
        InputConfig::default().with_pull(Pull::Up),
    );
    let de_pin = unsafe { AnyPin::steal(pin.modbus_de_re as u8) };
    let mut de_re = Output::new(de_pin, Level::Low, OutputConfig::default());

    let mut uart = Uart::new(uart_periph, uart_config(baud))
        .map_err(|_| FirmwareError::Uart("rs485 init"))?
        .with_tx(tx_pin)
        .with_rx(rx_pin)
        .into_async();

    while TX_CH.try_receive().is_ok() {}
    while BAUD_CH.try_receive().is_ok() {}
    mark_active(true);
    log::info!("RS-485 bus ready baud={}", baud);

    let mut rx_buf = [0u8; 64];
    loop {
        if uart_owner::stop_requested() {
            break;
        }

        match select4(
            uart.read_async(&mut rx_buf),
            TX_CH.receive(),
            uart_owner::wait_stop(),
            select(
                console::wait_host_pipe(),
                Timer::after(Duration::from_millis(15)),
            ),
        )
        .await
        {
            Either4::First(Ok(n)) if n > 0 => {
                let n = drain_uart_rx(&mut uart, &mut rx_buf, n);
                fanout_rx(&rx_buf[..n]);
            }
            Either4::First(_) => {}
            Either4::Second(frame) => {
                tx_uart(&mut uart, &mut de_re, &frame).await;
            }
            Either4::Third(()) => break,
            Either4::Fourth(_) => {}
        }

        let mut host_buf = [0u8; 64];
        let n_host = console::pipe_pop_host(&mut host_buf);
        if n_host > 0 {
            tx_uart(&mut uart, &mut de_re, &host_buf[..n_host]).await;
        }

        while let Ok(new_baud) = BAUD_CH.try_receive() {
            if uart.apply_config(&uart_config(new_baud)).is_err() {
                log::warn!("RS-485 apply baud {} failed", new_baud);
            } else {
                log::info!("RS-485 UART1 baud={}", new_baud);
            }
        }

        let cdc = console::take_cdc_baud();
        if cdc != 0 && cdc != live_baud() {
            set_live_baud(cdc);
        }
    }

    mark_active(false);
    log::info!("RS-485 transceiver stopped");
    Ok(())
}

async fn tx_uart(
    uart: &mut Uart<'static, esp_hal::Async>,
    de_re: &mut Output<'static>,
    data: &[u8],
) {
    if data.is_empty() {
        return;
    }
    de_re.set_high();
    Timer::after(Duration::from_micros(10)).await;
    let _ = uart.write_async(data).await;
    let _ = uart.flush_async().await;
    de_re.set_low();
}

fn drain_uart_rx(
    uart: &mut Uart<'static, esp_hal::Async>,
    buf: &mut [u8],
    mut n: usize,
) -> usize {
    while n < buf.len() {
        if !uart.read_ready() {
            break;
        }
        match uart.read(&mut buf[n..]) {
            Ok(0) | Err(_) => break,
            Ok(m) => n += m,
        }
    }
    n
}
