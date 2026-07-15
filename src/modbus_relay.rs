//! Modbus RTU bridge for `operating_mode = rtu_relay`.
//!
//! Owns UART1/UHCI and serializes bus transactions for TCP :502 and `/ws/modbus`.

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use heapless::Vec as HVec;

use crate::error::{FirmwareError, Result};
use crate::motor_57aim30::init_uart_and_modbus;
use crate::storage::PinConfiguration;

const FRAME_CAP: usize = 256;

static RELAY_ACTIVE: AtomicBool = AtomicBool::new(false);
static EXCHANGE_GATE: Mutex<CriticalSectionRawMutex, ()> = Mutex::new(());
static REQ_CH: Channel<CriticalSectionRawMutex, HVec<u8, FRAME_CAP>, 1> = Channel::new();
static RESP_CH: Channel<CriticalSectionRawMutex, Result<HVec<u8, FRAME_CAP>>, 1> = Channel::new();

pub fn is_active() -> bool {
    RELAY_ACTIVE.load(Ordering::Relaxed)
}

pub fn mark_active(active: bool) {
    RELAY_ACTIVE.store(active, Ordering::Relaxed);
}

fn calc_crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xffff;
    for &b in data {
        crc ^= u16::from(b);
        for _ in 0..8 {
            if (crc & 0x0001) != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

pub fn verify_rtu_crc(frame: &[u8]) -> bool {
    if frame.len() < 4 {
        return false;
    }
    let n = frame.len();
    let expected = u16::from_le_bytes([frame[n - 2], frame[n - 1]]);
    calc_crc16(&frame[..n - 2]) == expected
}

pub fn append_rtu_crc(frame: &mut HVec<u8, FRAME_CAP>) -> Result<()> {
    let crc = calc_crc16(frame);
    frame
        .extend_from_slice(&crc.to_le_bytes())
        .map_err(|_| FirmwareError::Modbus("frame too large"))
}

/// Serialize an RTU request/response exchange through the relay task.
pub async fn exchange_rtu(frame: &[u8]) -> Result<HVec<u8, FRAME_CAP>> {
    if !is_active() {
        return Err(FirmwareError::Modbus("relay inactive"));
    }
    if frame.len() < 4 || frame.len() > FRAME_CAP {
        return Err(FirmwareError::Modbus("bad frame len"));
    }
    if !verify_rtu_crc(frame) {
        return Err(FirmwareError::Modbus("bad crc"));
    }

    let mut req = HVec::new();
    req.extend_from_slice(frame)
        .map_err(|_| FirmwareError::Modbus("frame too large"))?;

    let _gate = EXCHANGE_GATE.lock().await;
    REQ_CH.send(req).await;
    RESP_CH.receive().await
}

/// Convert unit + PDU into an RTU frame, exchange, return PDU (no unit/CRC).
pub async fn exchange_modbus_tcp(unit_id: u8, pdu: &[u8]) -> Result<HVec<u8, FRAME_CAP>> {
    if pdu.is_empty() || pdu.len() + 3 > FRAME_CAP {
        return Err(FirmwareError::Modbus("bad pdu"));
    }
    let mut rtu = HVec::new();
    rtu.push(unit_id)
        .map_err(|_| FirmwareError::Modbus("frame too large"))?;
    rtu.extend_from_slice(pdu)
        .map_err(|_| FirmwareError::Modbus("frame too large"))?;
    append_rtu_crc(&mut rtu)?;

    let resp = exchange_rtu(&rtu).await?;
    if resp.len() < 3 {
        return Err(FirmwareError::Modbus("short response"));
    }
    let pdu_len = resp.len() - 3;
    let mut out = HVec::new();
    out.extend_from_slice(&resp[1..1 + pdu_len])
        .map_err(|_| FirmwareError::Modbus("frame too large"))?;
    Ok(out)
}

pub async fn run_relay(
    uart_periph: esp_hal::peripherals::UART1<'static>,
    uhci_periph: esp_hal::peripherals::UHCI0<'static>,
    dma_channel: esp_hal::peripherals::DMA_CH0<'static>,
    pin_config: PinConfiguration,
) -> Result<()> {
    log::info!("Starting Modbus RTU relay (operating_mode=rtu_relay)");
    let mut master = init_uart_and_modbus(uart_periph, uhci_periph, dma_channel, &pin_config)?;
    mark_active(true);
    log::info!("Modbus RTU relay bus ready");

    loop {
        let req = REQ_CH.receive().await;
        let mut resp_buf = [0u8; FRAME_CAP];
        let result = match master.modbus_request(&req, &mut resp_buf).await {
            Ok(len) => {
                let mut out = HVec::new();
                if out.extend_from_slice(&resp_buf[..len]).is_ok() {
                    Ok(out)
                } else {
                    Err(FirmwareError::Modbus("resp too large"))
                }
            }
            Err(e) => Err(e),
        };
        RESP_CH.send(result).await;
    }
}

#[cfg(feature = "esp32s3")]
pub fn run_relay_blocking(
    uart_periph: esp_hal::peripherals::UART1<'static>,
    uhci_periph: esp_hal::peripherals::UHCI0<'static>,
    dma_channel: esp_hal::peripherals::DMA_CH0<'static>,
    pin_config: PinConfiguration,
) {
    crate::motor_57aim30::run_relay_on_core1(uart_periph, uhci_periph, dma_channel, pin_config);
}
