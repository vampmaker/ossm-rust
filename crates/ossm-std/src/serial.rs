use std::time::{Duration, Instant};

use ossm_core::modbus::aim30;
use rmodbus::{client::ModbusRequest, ModbusProto};
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::mpsc;
use tokio_serial::{SerialPort as SerialPortExt, SerialPortBuilderExt};

use crate::modbus_rx::RxAccumulator;

const STREAM_DEADLINE: Duration = Duration::from_millis(12);
const RX_DEADLINE: Duration = Duration::from_millis(200);

struct ParsedFrame {
    bytes: Vec<u8>,
    t_rx: Instant,
}

pub struct WriteTiming {
    pub t_tx: Instant,
    pub t_rx: Option<Instant>,
}

pub struct SerialPort {
    writer: WriteHalf<tokio_serial::SerialStream>,
    slave: u8,
    frames: mpsc::Receiver<ParsedFrame>,
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
            writer,
            slave,
            frames,
        })
    }

    pub fn slave_id(&self) -> u8 {
        self.slave
    }

    pub async fn exchange_rtu(&mut self, frame: &[u8]) -> Result<Vec<u8>, String> {
        self.transact(frame, RX_DEADLINE)
            .await
            .map(|(rsp, _, _)| rsp)
            .map_err(|(e, _)| e)
    }

    /// Streaming position write: CRC miss is logged, not an error.
    pub async fn write_position_radians(&mut self, position: f32) -> Result<WriteTiming, String> {
        let counts = aim30::write_counts_for_radians(position);
        let regs = aim30::pack_position_i32(counts);
        let mut request = ModbusRequest::new(self.slave, ModbusProto::Rtu);
        let mut request_buf = fixedvec::alloc_stack!([u8; 256]);
        let mut frame_buf = fixedvec::FixedVec::new(&mut request_buf);
        request
            .generate_set_holdings_bulk(aim30::REG_POSITION, &regs, &mut frame_buf)
            .map_err(|e| format!("modbus encode: {e:?}"))?;
        let tx = frame_buf.as_slice().to_vec();
        match self.transact(&tx, STREAM_DEADLINE).await {
            Ok((rsp, t_tx, t_rx)) => {
                if let Err(e) = request.parse_ok(&rsp) {
                    tracing::debug!("modbus parse: {e:?}");
                }
                Ok(WriteTiming {
                    t_tx,
                    t_rx: Some(t_rx),
                })
            }
            Err((e, t_tx)) if is_crc_miss(&e) => {
                tracing::debug!("{e}");
                Ok(WriteTiming { t_tx, t_rx: None })
            }
            Err((e, _)) => Err(e),
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
        if let Err(e) = self.writer.write_all(tx).await {
            return Err((e.to_string(), t_tx));
        }
        if let Err(e) = self.writer.flush().await {
            return Err((e.to_string(), t_tx));
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

fn is_crc_miss(err: &str) -> bool {
    err.starts_with("no CRC-valid response")
}
