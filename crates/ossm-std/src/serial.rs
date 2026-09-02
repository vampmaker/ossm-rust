use std::io::Read;
use std::time::{Duration, Instant};

use ossm_core::modbus::aim30;
use rmodbus::{client::ModbusRequest, ModbusProto};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_serial::{SerialPort as SerialPortExt, SerialPortBuilderExt};

use crate::modbus_rx::{expected_response_len, parse_fc03_u16, RxAccumulator};

const INTER_FRAME: Duration = Duration::from_millis(4);
const RX_DEADLINE: Duration = Duration::from_millis(20);
const STABLE_EPS: f32 = 0.002;
const STABLE_POLL: Duration = Duration::from_millis(100);

pub struct SerialPort {
    inner: tokio_serial::SerialStream,
    slave: u8,
    rx: RxAccumulator,
    last_rx: Option<Instant>,
}

impl SerialPort {
    pub fn open(path: &str, baud: u32, slave: u8) -> tokio_serial::Result<Self> {
        let inner = tokio_serial::new(path, baud)
            .data_bits(tokio_serial::DataBits::Eight)
            .parity(tokio_serial::Parity::None)
            .stop_bits(tokio_serial::StopBits::One)
            .open_native_async()?;
        Ok(Self {
            inner,
            slave,
            rx: RxAccumulator::new(),
            last_rx: None,
        })
    }

    /// Streaming position write: CRC miss is logged, not an error.
    pub async fn write_position_radians(&mut self, position: f32) -> Result<(), String> {
        let counts = aim30::write_counts_for_radians(position);
        let regs = aim30::pack_position_i32(counts);
        match self.write_holdings(aim30::REG_POSITION, &regs).await {
            Ok(()) => Ok(()),
            Err(e) if is_crc_miss(&e) => {
                tracing::debug!("{e}");
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    pub async fn write_position_radians_strict(&mut self, position: f32) -> Result<(), String> {
        let counts = aim30::write_counts_for_radians(position);
        let regs = aim30::pack_position_i32(counts);
        self.write_holdings(aim30::REG_POSITION, &regs).await
    }

    pub async fn read_position_radians(&mut self) -> Result<f32, String> {
        let regs = self.read_holdings(aim30::REG_POSITION, 2).await?;
        if regs.len() < 2 {
            return Err("short position read".into());
        }
        let counts = aim30::unpack_position_i32([regs[0], regs[1]]);
        Ok(aim30::counts_to_radians(counts))
    }

    pub async fn enable_modbus(&mut self) -> Result<(), String> {
        self.write_holding(aim30::REG_ENABLE, aim30::ENABLE_MODBUS)
            .await
    }

    pub async fn set_max_power(&mut self, power: f32) -> Result<(), String> {
        self.write_holding(aim30::REG_POWER, aim30::encode_max_power(power))
            .await
    }

    pub async fn set_acceleration(&mut self, acceleration: f32) -> Result<(), String> {
        self.write_holding(aim30::REG_ACCEL, aim30::encode_acceleration(acceleration))
            .await
    }

    pub async fn set_position_ring_ratio(&mut self, ratio: u16) -> Result<(), String> {
        self.write_holding(aim30::REG_POS_RING, ratio).await
    }

    pub async fn set_speed_ring_ratio(&mut self, ratio: u16) -> Result<(), String> {
        self.write_holding(aim30::REG_SPEED_RING, ratio).await
    }

    pub async fn reset_position(&mut self) -> Result<(), String> {
        let regs = aim30::pack_position_i32(0);
        self.write_holdings(aim30::REG_POSITION, &regs).await
    }

    pub async fn wait_stable_position(&mut self, timeout: Duration) -> Result<f32, String> {
        let start = Instant::now();
        let mut position = self.read_position_radians().await?;
        tokio::time::sleep(STABLE_POLL).await;
        while start.elapsed() < timeout {
            let new_position = self.read_position_radians().await?;
            if (new_position - position).abs() < STABLE_EPS {
                return Ok(new_position);
            }
            position = new_position;
            tokio::time::sleep(STABLE_POLL).await;
        }
        Err("stable position timeout".into())
    }

    /// Firmware `Modbus57AIM30Motor::homing` register sequence. Returns `(pos_min, pos_max)`.
    pub async fn homing(&mut self) -> Result<(f32, f32), String> {
        tracing::info!("Homing motor");
        self.set_max_power(aim30::POWER_HOMING).await?;
        self.set_acceleration(aim30::ACCEL_HOMING).await?;
        self.reset_position().await?;

        tracing::info!("Writing position to -100.0");
        self.write_position_radians_strict(-100.0).await?;
        tokio::time::sleep(Duration::from_millis(5000)).await;
        let pos_min = self
            .wait_stable_position(Duration::from_millis(5000))
            .await?
            + 0.1;
        tracing::info!("pos_min: {pos_min}");

        tracing::info!("Writing position to 100.0");
        self.write_position_radians_strict(100.0).await?;
        tokio::time::sleep(Duration::from_millis(5000)).await;
        let pos_max = self
            .wait_stable_position(Duration::from_millis(5000))
            .await?
            - 0.1;
        tracing::info!("pos_max: {pos_max}");

        self.write_position_radians_strict((pos_min + pos_max) / 2.0)
            .await?;
        tokio::time::sleep(Duration::from_millis(5000)).await;
        self.wait_stable_position(Duration::from_millis(5000))
            .await?;
        Ok((pos_min, pos_max))
    }

    pub async fn apply_run_gains(&mut self) -> Result<(), String> {
        self.set_max_power(aim30::POWER_RUN).await?;
        self.set_acceleration(aim30::ACCEL_RUN).await?;
        self.set_position_ring_ratio(aim30::RING_RATIO_RUN).await?;
        self.set_speed_ring_ratio(aim30::RING_RATIO_RUN).await
    }

    fn drain_kernel_rx(&mut self) {
        let mut tmp = [0u8; 256];
        loop {
            let n = match self.inner.bytes_to_read() {
                Ok(n) if n > 0 => n as usize,
                _ => break,
            };
            let want = n.min(tmp.len());
            match Read::read(&mut self.inner, &mut tmp[..want]) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
        self.rx.clear();
    }

    async fn inter_frame_gap(&self) {
        let Some(last) = self.last_rx else {
            return;
        };
        if let Some(remain) = INTER_FRAME.checked_sub(last.elapsed()) {
            if !remain.is_zero() {
                tokio::time::sleep(remain).await;
            }
        }
    }

    async fn transact(&mut self, tx: &[u8]) -> Result<Vec<u8>, String> {
        let expected = expected_response_len(tx);
        let req_fc = tx.get(1).copied().unwrap_or(0x10);

        self.drain_kernel_rx();
        self.inter_frame_gap().await;

        self.inner.write_all(tx).await.map_err(|e| e.to_string())?;
        self.inner.flush().await.map_err(|e| e.to_string())?;

        let deadline = Instant::now() + RX_DEADLINE;
        let mut tmp = [0u8; 256];
        loop {
            self.rx.skip_echo(tx);
            if let Some(frame) = self.rx.extract(self.slave, expected, req_fc) {
                if frame.len() >= 2 && (frame[1] & 0x80) != 0 {
                    let code = frame.get(2).copied().unwrap_or(0);
                    return Err(format!("modbus exception {code}"));
                }
                let out = frame.to_vec();
                self.last_rx = Some(Instant::now());
                return Ok(out);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!("no CRC-valid response ({} bytes)", self.rx.len()));
            }
            match tokio::time::timeout(remaining, AsyncReadExt::read(&mut self.inner, &mut tmp))
                .await
            {
                Err(_) => {
                    return Err(format!("no CRC-valid response ({} bytes)", self.rx.len()));
                }
                Ok(Err(e)) => return Err(e.to_string()),
                Ok(Ok(0)) => {
                    return Err(format!("no CRC-valid response ({} bytes)", self.rx.len()));
                }
                Ok(Ok(n)) => {
                    self.last_rx = Some(Instant::now());
                    self.rx.push(&tmp[..n]);
                }
            }
        }
    }

    async fn write_holding(&mut self, addr: u16, value: u16) -> Result<(), String> {
        let mut request = ModbusRequest::new(self.slave, ModbusProto::Rtu);
        let mut request_buf = fixedvec::alloc_stack!([u8; 256]);
        let mut frame_buf = fixedvec::FixedVec::new(&mut request_buf);
        request
            .generate_set_holding(addr, value, &mut frame_buf)
            .map_err(|e| format!("modbus encode: {e:?}"))?;
        let tx = frame_buf.as_slice().to_vec();
        let rsp = self.transact(&tx).await?;
        request
            .parse_ok(&rsp)
            .map_err(|e| format!("modbus parse: {e:?}"))
    }

    async fn write_holdings(&mut self, addr: u16, values: &[u16]) -> Result<(), String> {
        let mut request = ModbusRequest::new(self.slave, ModbusProto::Rtu);
        let mut request_buf = fixedvec::alloc_stack!([u8; 256]);
        let mut frame_buf = fixedvec::FixedVec::new(&mut request_buf);
        request
            .generate_set_holdings_bulk(addr, values, &mut frame_buf)
            .map_err(|e| format!("modbus encode: {e:?}"))?;
        let tx = frame_buf.as_slice().to_vec();
        let rsp = self.transact(&tx).await?;
        request
            .parse_ok(&rsp)
            .map_err(|e| format!("modbus parse: {e:?}"))
    }

    async fn read_holdings(&mut self, addr: u16, count: u16) -> Result<Vec<u16>, String> {
        let mut request = ModbusRequest::new(self.slave, ModbusProto::Rtu);
        let mut request_buf = fixedvec::alloc_stack!([u8; 256]);
        let mut frame_buf = fixedvec::FixedVec::new(&mut request_buf);
        request
            .generate_get_holdings(addr, count, &mut frame_buf)
            .map_err(|e| format!("modbus encode: {e:?}"))?;
        let tx = frame_buf.as_slice().to_vec();
        let rsp = self.transact(&tx).await?;
        parse_fc03_u16(&rsp).ok_or_else(|| "modbus parse: fc03".into())
    }
}

fn is_crc_miss(err: &str) -> bool {
    err.starts_with("no CRC-valid response")
}
