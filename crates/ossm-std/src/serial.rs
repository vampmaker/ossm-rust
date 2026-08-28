use std::time::Duration;

use ossm_core::modbus::aim30;
use ossm_core::modbus::find_modbus_response;
use rmodbus::{client::ModbusRequest, ModbusProto};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_serial::SerialPortBuilderExt;

pub struct SerialPort {
    inner: tokio_serial::SerialStream,
    slave: u8,
}

impl SerialPort {
    pub fn open(path: &str, baud: u32, slave: u8) -> tokio_serial::Result<Self> {
        let inner = tokio_serial::new(path, baud)
            .data_bits(tokio_serial::DataBits::Eight)
            .parity(tokio_serial::Parity::None)
            .stop_bits(tokio_serial::StopBits::One)
            .open_native_async()?;
        Ok(Self { inner, slave })
    }

    pub async fn write_position_radians(&mut self, position: f32) -> Result<(), String> {
        let counts = aim30::write_counts_for_radians(position);
        let regs = aim30::pack_position_i32(counts);
        self.write_holdings(aim30::REG_POSITION, &regs).await
    }

    async fn write_holdings(&mut self, addr: u16, values: &[u16]) -> Result<(), String> {
        let mut request = ModbusRequest::new(self.slave, ModbusProto::Rtu);
        let mut request_buf = fixedvec::alloc_stack!([u8; 256]);
        let mut frame_buf = fixedvec::FixedVec::new(&mut request_buf);
        request
            .generate_set_holdings_bulk(addr, values, &mut frame_buf)
            .map_err(|e| format!("modbus encode: {e:?}"))?;
        self.inner
            .write_all(frame_buf.as_slice())
            .await
            .map_err(|e| e.to_string())?;
        self.inner.flush().await.map_err(|e| e.to_string())?;

        let mut rx = [0u8; 256];
        let n = tokio::time::timeout(Duration::from_millis(20), self.inner.read(&mut rx))
            .await
            .map_err(|_| "rx timeout".to_string())?
            .map_err(|e| e.to_string())?;
        if find_modbus_response(&rx[..n], self.slave, 8).is_none() {
            tracing::debug!("no CRC-valid write ack ({} bytes)", n);
        }
        Ok(())
    }
}
