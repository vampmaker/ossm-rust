#[cfg(target_os = "linux")]
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ossm_core::modbus::aim30;
use rmodbus::{client::ModbusRequest, ModbusProto};
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::mpsc;
use tokio_serial::{SerialPort as SerialPortExt, SerialPortBuilderExt};

#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "linux")]
use tokio::sync::oneshot;

#[cfg(target_os = "linux")]
use crate::link_stats::micros_now;
use crate::link_stats::SharedLinkStats;
use crate::modbus_rx::RxAccumulator;
#[cfg(target_os = "linux")]
use crate::precise_wait::AckWaker;

#[cfg(target_os = "linux")]
const HOST_STREAM_DEADLINE: Duration = Duration::from_millis(6);
const RX_DEADLINE: Duration = Duration::from_millis(200);

/// Capacity-1 setpoint: a newer submit replaces an unsent frame.
pub(crate) struct LatestWins {
    slot: Mutex<Option<Vec<u8>>>,
}

impl LatestWins {
    pub(crate) fn new() -> Self {
        Self {
            slot: Mutex::new(None),
        }
    }

    pub(crate) fn submit(&self, data: Vec<u8>) {
        *self.slot.lock().unwrap() = Some(data);
    }

    pub(crate) fn take(&self) -> Option<Vec<u8>> {
        self.slot.lock().unwrap().take()
    }
}

struct ParsedFrame {
    bytes: Vec<u8>,
    t_rx: Instant,
}

#[derive(Clone, Copy)]
pub(crate) struct AckSample {
    pub t_tx: Instant,
    pub t_rx: Option<Instant>,
}

enum PortWriter {
    Tty(WriteHalf<tokio_serial::SerialStream>),
    #[cfg(target_os = "linux")]
    UsbHost(UsbHostWriter),
}

#[cfg(target_os = "linux")]
struct UsbHostWriter {
    writes: std::sync::mpsc::Sender<UsbWriteReq>,
    stream: Arc<LatestWins>,
    link: Arc<Mutex<Option<SharedLinkStats>>>,
    alive: Arc<AtomicBool>,
    ack: Arc<Mutex<Option<AckSample>>>,
    ack_waker: Arc<AckWaker>,
}

#[cfg(target_os = "linux")]
pub(crate) struct UsbStreamParts {
    pub stream: Arc<LatestWins>,
    pub alive: Arc<AtomicBool>,
    pub ack: Arc<Mutex<Option<AckSample>>>,
    pub ack_waker: Arc<AckWaker>,
    pub slave: u8,
}

#[cfg(target_os = "linux")]
struct UsbWriteReq {
    data: Vec<u8>,
    reply: oneshot::Sender<Result<(), String>>,
}

pub struct SerialPort {
    writer: PortWriter,
    slave: u8,
    frames: mpsc::Receiver<ParsedFrame>,
    #[cfg(target_os = "linux")]
    _usb_keepalive: Option<std::fs::File>,
}

impl SerialPort {
    pub fn open(path: &str, baud: u32, slave: u8) -> tokio_serial::Result<Self> {
        let mut inner = tokio_serial::new(path, baud)
            .data_bits(tokio_serial::DataBits::Eight)
            .parity(tokio_serial::Parity::None)
            .stop_bits(tokio_serial::StopBits::One)
            .dtr_on_open(false)
            .open_native_async()?;
        let _ = SerialPortExt::write_data_terminal_ready(&mut inner, false);
        let _ = SerialPortExt::write_request_to_send(&mut inner, false);

        let (reader, writer) = tokio::io::split(inner);
        let (frame_tx, frames) = mpsc::channel(32);
        tokio::spawn(rx_task(reader, frame_tx));

        Ok(Self {
            writer: PortWriter::Tty(writer),
            slave,
            frames,
            #[cfg(target_os = "linux")]
            _usb_keepalive: None,
        })
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn from_usb_host(
        handle: android_usb_serial::SerialPortHandle,
        slave: u8,
        keepalive: Option<std::fs::File>,
    ) -> Result<Self, String> {
        let (frame_tx, frames) = mpsc::channel(32);
        let (writes_tx, writes_rx) = std::sync::mpsc::channel();
        let stream = Arc::new(LatestWins::new());
        let link = Arc::new(Mutex::new(None::<SharedLinkStats>));
        let alive = Arc::new(AtomicBool::new(true));
        let ack = Arc::new(Mutex::new(None::<AckSample>));
        let ack_waker = Arc::new(AckWaker::new());
        let stream_thread = stream.clone();
        let link_thread = link.clone();
        let alive_thread = alive.clone();
        let ack_thread = ack.clone();
        let ack_waker_thread = ack_waker.clone();
        let slave_thread = slave;
        std::thread::Builder::new()
            .name("ossm-usb-host".into())
            .spawn(move || {
                usb_host_thread(
                    handle,
                    writes_rx,
                    frame_tx,
                    stream_thread,
                    link_thread,
                    alive_thread,
                    ack_thread,
                    ack_waker_thread,
                    slave_thread,
                )
            })
            .map_err(|e| format!("usb host thread: {e}"))?;
        Ok(Self {
            writer: PortWriter::UsbHost(UsbHostWriter {
                writes: writes_tx,
                stream,
                link,
                alive,
                ack,
                ack_waker,
            }),
            slave,
            frames,
            _usb_keepalive: keepalive,
        })
    }

    pub fn slave_id(&self) -> u8 {
        self.slave
    }

    pub fn is_usb_host(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            matches!(self.writer, PortWriter::UsbHost(_))
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn usb_stream_parts(&self) -> Option<UsbStreamParts> {
        match &self.writer {
            PortWriter::UsbHost(w) => Some(UsbStreamParts {
                stream: w.stream.clone(),
                alive: w.alive.clone(),
                ack: w.ack.clone(),
                ack_waker: w.ack_waker.clone(),
                slave: self.slave,
            }),
            PortWriter::Tty(_) => None,
        }
    }

    pub fn attach_link(&mut self, stats: SharedLinkStats) {
        #[cfg(target_os = "linux")]
        if let PortWriter::UsbHost(w) = &self.writer {
            *w.link.lock().unwrap() = Some(stats);
        }
        #[cfg(not(target_os = "linux"))]
        let _ = stats;
    }

    pub async fn exchange_rtu(&mut self, frame: &[u8]) -> Result<Vec<u8>, String> {
        self.transact(frame, RX_DEADLINE)
            .await
            .map(|(rsp, _, _)| rsp)
            .map_err(|(e, _)| e)
    }

    async fn write_frame(&mut self, tx: &[u8]) -> Result<(), String> {
        match &mut self.writer {
            PortWriter::Tty(w) => {
                w.write_all(tx).await.map_err(|e| e.to_string())?;
                w.flush().await.map_err(|e| e.to_string())
            }
            #[cfg(target_os = "linux")]
            PortWriter::UsbHost(w) => {
                let (reply, rx) = oneshot::channel();
                w.writes
                    .send(UsbWriteReq {
                        data: tx.to_vec(),
                        reply,
                    })
                    .map_err(|_| "usb host thread ended".to_string())?;
                rx.await.map_err(|_| "usb host thread ended".to_string())?
            }
        }
    }

    async fn transact(
        &mut self,
        tx: &[u8],
        deadline: Duration,
    ) -> Result<(Vec<u8>, Instant, Instant), (String, Instant)> {
        let req_fc = tx.get(1).copied().unwrap_or(0x10);

        while self.frames.try_recv().is_ok() {}

        let t_tx = Instant::now();
        if let Err(e) = self.write_frame(tx).await {
            return Err((e, t_tx));
        }
        tracing::debug!("rtu tx {:02x?}", tx);

        let until = t_tx + deadline;
        loop {
            let remaining = until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(("no CRC-valid response".into(), t_tx));
            }
            match tokio::time::timeout(remaining, self.frames.recv()).await {
                Err(_) => return Err(("no CRC-valid response".into(), t_tx)),
                Ok(None) => return Err(("serial rx task ended".into(), t_tx)),
                Ok(Some(frame)) => {
                    if frame.bytes.len() < 2 || frame.bytes[0] != self.slave {
                        tracing::debug!(
                            "unmatched rtu frame {:02x?}",
                            &frame.bytes[..frame.bytes.len().min(8)]
                        );
                        continue;
                    }
                    let fc = frame.bytes[1];
                    if fc != req_fc && fc != (req_fc | 0x80) {
                        tracing::debug!("unmatched rtu fc {fc:#x} want {req_fc:#x}");
                        continue;
                    }
                    if (fc & 0x80) != 0 {
                        let code = frame.bytes.get(2).copied().unwrap_or(0);
                        return Err((format!("modbus exception {code}"), t_tx));
                    }
                    return Ok((frame.bytes, t_tx, frame.t_rx));
                }
            }
        }
    }
}

async fn rx_task(
    mut reader: ReadHalf<tokio_serial::SerialStream>,
    frames: mpsc::Sender<ParsedFrame>,
) {
    let mut acc = RxAccumulator::new();
    let mut tmp = [0u8; 256];
    loop {
        let n = match reader.read(&mut tmp).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                tracing::warn!("serial rx: {e}");
                break;
            }
        };
        acc.push(&tmp[..n]);

        while let Some(bytes) = acc.pop_frame() {
            let frame = ParsedFrame {
                bytes,
                t_rx: Instant::now(),
            };
            if frames.send(frame).await.is_err() {
                return;
            }
        }
    }
}

pub(crate) fn encode_position(slave: u8, position: f32) -> Result<Vec<u8>, String> {
    let counts = aim30::write_counts_for_radians(position);
    let regs = aim30::pack_position_i32(counts);
    let mut request = ModbusRequest::new(slave, ModbusProto::Rtu);
    let mut request_buf = fixedvec::alloc_stack!([u8; 256]);
    let mut frame_buf = fixedvec::FixedVec::new(&mut request_buf);
    request
        .generate_set_holdings_bulk(aim30::REG_POSITION, &regs, &mut frame_buf)
        .map_err(|e| format!("modbus encode: {e:?}"))?;
    Ok(frame_buf.as_slice().to_vec())
}

#[cfg(target_os = "linux")]
fn usb_write_all(
    handle: &mut android_usb_serial::SerialPortHandle,
    mut data: &[u8],
) -> Result<(), String> {
    while !data.is_empty() {
        let n = handle.write(data).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("usb host write returned 0".into());
        }
        data = &data[n..];
    }
    Ok(())
}

#[cfg(target_os = "linux")]
#[allow(clippy::too_many_arguments)]
fn usb_host_thread(
    mut handle: android_usb_serial::SerialPortHandle,
    writes: std::sync::mpsc::Receiver<UsbWriteReq>,
    frames: mpsc::Sender<ParsedFrame>,
    stream: Arc<LatestWins>,
    link: Arc<Mutex<Option<SharedLinkStats>>>,
    alive: Arc<AtomicBool>,
    ack: Arc<Mutex<Option<AckSample>>>,
    ack_waker: Arc<AckWaker>,
    slave: u8,
) {
    let mut acc = RxAccumulator::new();
    let mut buf = [0u8; 256];
    let mut expect_rx = false;
    loop {
        while let Ok(req) = writes.try_recv() {
            let _ = req.reply.send(usb_write_all(&mut handle, &req.data));
            expect_rx = true;
        }
        if expect_rx {
            match usb_pump_rx(&mut handle, &mut acc, &mut buf, &frames, true) {
                Err(()) => break,
                Ok(true) => {
                    expect_rx = false;
                    continue;
                }
                Ok(false) => {
                    std::thread::sleep(Duration::from_micros(50));
                    continue;
                }
            }
        }
        if let Some(data) = stream.take() {
            usb_discard_rx(&mut handle, &mut acc, &mut buf);
            let req_fc = data.get(1).copied().unwrap_or(0x10);
            let t_tx = Instant::now();
            if let Err(e) = usb_write_all(&mut handle, &data) {
                tracing::debug!("usb host stream tx: {e}");
                record_stream_failure(&link, Some("usb host write"));
                publish_ack(&ack, &ack_waker, t_tx, None);
                continue;
            }
            let t_rx = usb_wait_stream_ack(
                &mut handle,
                &mut acc,
                &mut buf,
                slave,
                req_fc,
                HOST_STREAM_DEADLINE,
            );
            if t_rx.is_none() {
                usb_discard_rx(&mut handle, &mut acc, &mut buf);
            }
            record_stream_result(&link, t_tx, t_rx, data.len() as u32);
            publish_ack(&ack, &ack_waker, t_tx, t_rx);
            continue;
        }
        match writes.recv_timeout(Duration::from_micros(200)) {
            Ok(req) => {
                let _ = req.reply.send(usb_write_all(&mut handle, &req.data));
                expect_rx = true;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    alive.store(false, Ordering::Relaxed);
}

#[cfg(target_os = "linux")]
fn publish_ack(
    ack: &Mutex<Option<AckSample>>,
    ack_waker: &AckWaker,
    t_tx: Instant,
    t_rx: Option<Instant>,
) {
    if let Ok(mut g) = ack.lock() {
        *g = Some(AckSample { t_tx, t_rx });
    }
    ack_waker.notify();
}

#[cfg(target_os = "linux")]
fn usb_pump_rx(
    handle: &mut android_usb_serial::SerialPortHandle,
    acc: &mut RxAccumulator,
    buf: &mut [u8],
    frames: &mpsc::Sender<ParsedFrame>,
    forward: bool,
) -> Result<bool, ()> {
    let mut got = false;
    loop {
        match handle.try_read(buf) {
            Ok(0) => break,
            Ok(n) => {
                got = true;
                acc.push(&buf[..n]);
                while let Some(bytes) = acc.pop_frame() {
                    if forward {
                        let frame = ParsedFrame {
                            bytes,
                            t_rx: Instant::now(),
                        };
                        if frames.blocking_send(frame).is_err() {
                            return Err(());
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("usb host rx: {e}");
                return Err(());
            }
        }
    }
    Ok(got)
}

/// Drop leftover UART bytes so a late previous ACK cannot match the next TX.
#[cfg(target_os = "linux")]
fn usb_discard_rx(
    handle: &mut android_usb_serial::SerialPortHandle,
    acc: &mut RxAccumulator,
    buf: &mut [u8],
) {
    loop {
        match handle.try_read(buf) {
            Ok(0) => break,
            Ok(n) => acc.push(&buf[..n]),
            Err(_) => break,
        }
    }
    acc.clear();
}

#[cfg(target_os = "linux")]
fn usb_wait_stream_ack(
    handle: &mut android_usb_serial::SerialPortHandle,
    acc: &mut RxAccumulator,
    buf: &mut [u8],
    slave: u8,
    req_fc: u8,
    deadline: Duration,
) -> Option<Instant> {
    let until = Instant::now() + deadline;
    loop {
        loop {
            match handle.try_read(buf) {
                Ok(0) => break,
                Ok(n) => acc.push(&buf[..n]),
                Err(e) => {
                    tracing::warn!("usb host rx: {e}");
                    return None;
                }
            }
        }
        while let Some(bytes) = acc.pop_frame() {
            if bytes.len() >= 2
                && bytes[0] == slave
                && (bytes[1] == req_fc || bytes[1] == (req_fc | 0x80))
                && (bytes[1] & 0x80) == 0
            {
                return Some(Instant::now());
            }
        }
        if Instant::now() >= until {
            return None;
        }
        std::thread::sleep(Duration::from_micros(50));
    }
}

#[cfg(target_os = "linux")]
fn record_stream_result(
    link: &Mutex<Option<SharedLinkStats>>,
    t_tx: Instant,
    t_rx: Option<Instant>,
    tx_len: u32,
) {
    let Some(t_rx) = t_rx else {
        return;
    };
    let Some(stats) = link.lock().ok().and_then(|g| g.clone()) else {
        return;
    };
    let rtt_us = (t_rx - t_tx).as_micros().min(u16::MAX as u128) as u16;
    stats.record_success(micros_now(), rtt_us, None, None, tx_len, 8);
}

#[cfg(target_os = "linux")]
fn record_stream_failure(link: &Mutex<Option<SharedLinkStats>>, _err: Option<&str>) {
    let Some(stats) = link.lock().ok().and_then(|g| g.clone()) else {
        return;
    };
    stats.record_failure(micros_now());
}

pub(crate) fn is_crc_miss(err: &str) -> bool {
    err.starts_with("no CRC-valid response")
}

#[cfg(test)]
mod tests {
    use super::LatestWins;

    #[test]
    fn latest_wins_replaces_pending() {
        let slot = LatestWins::new();
        slot.submit(vec![1]);
        slot.submit(vec![2, 2]);
        assert_eq!(slot.take(), Some(vec![2, 2]));
        assert_eq!(slot.take(), None);
    }
}
