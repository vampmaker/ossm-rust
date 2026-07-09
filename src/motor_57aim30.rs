use std::time;

use crate::motor::Motor;
use esp_idf_svc::hal::delay::Ets;
use esp_idf_svc::hal::gpio;
use esp_idf_svc::hal::uart::{AsyncUartDriver, UartDriver};

use embassy_time::{with_timeout, Duration, Timer};
use fixedvec::FixedVec;
use rmodbus::{client::ModbusRequest, guess_response_frame_len, ModbusProto};
use anyhow::Result;


pub struct ModbusRTUMaster<'a> {
    uart: AsyncUartDriver<'a, UartDriver<'a>>,
    ctrl_pin_driver: Option<gpio::PinDriver<'a, gpio::Output>>,
    device_id: u8,
    read_timeout: Duration,
    write_timeout: Duration,
    timeout_override_ms: u32,
}

impl<'a> ModbusRTUMaster<'a> {
    pub fn new(
        uart: AsyncUartDriver<'a, UartDriver<'a>>,
        ctrl_pin: Option<gpio::AnyOutputPin<'a>>,
        device_id: u8,
        timeout_override_ms: u32,
    ) -> Self {
        let ctrl_pin_driver = ctrl_pin.map(|ctrl_pin| gpio::PinDriver::output(ctrl_pin).unwrap());
        let baudrate: u32 = uart.driver().baudrate().unwrap().into();
        let timeout = Self::compute_timeout(baudrate, timeout_override_ms).unwrap();

        Self {
            uart,
            ctrl_pin_driver,
            device_id,
            read_timeout: timeout,
            write_timeout: timeout,
            timeout_override_ms,
        }
    }

    fn get_default_timeout(baudrate: u32) -> Result<Duration> {
        match baudrate {
            9600 => Ok(Duration::from_millis(100)),
            19200 => Ok(Duration::from_millis(50)),
            38400 => Ok(Duration::from_millis(25)),
            115200 | 115201 => Ok(Duration::from_millis(15)),
            _ => Err(anyhow::anyhow!("Invalid baud rate: {}", baudrate)),
        }
    }

    fn compute_timeout(baudrate: u32, override_ms: u32) -> Result<Duration> {
        if override_ms > 0 {
            Ok(Duration::from_millis(override_ms as u64))
        } else {
            Self::get_default_timeout(baudrate)
        }
    }

    async fn uart_read_exactly(&mut self, buf: &mut [u8]) -> Result<()> {
        let mut total_bytes_read = 0;
        while total_bytes_read < buf.len() {
            let bytes_read = with_timeout(
                self.read_timeout,
                self.uart.read(&mut buf[total_bytes_read..]),
            )
            .await
            .map_err(|_| anyhow::anyhow!("UART read timeout"))??;
            total_bytes_read += bytes_read;
        }
        Ok(())
    }

    async fn uart_write_all(&mut self, buf: &[u8]) -> Result<()> {
        let mut total_bytes_written = 0;
        while total_bytes_written < buf.len() {
            let bytes_written = with_timeout(
                self.write_timeout,
                self.uart.write(&buf[total_bytes_written..]),
            )
            .await
            .map_err(|_| anyhow::anyhow!("UART write timeout"))??;
            total_bytes_written += bytes_written;
        }
        with_timeout(self.write_timeout, self.uart.wait_tx_done())
            .await
            .map_err(|_| anyhow::anyhow!("UART tx done timeout"))??;
        Ok(())
    }

    async fn modbus_request(&mut self, req: &[u8], resp: &mut [u8]) -> Result<usize> {
        assert!(resp.len() >= 256);

        self.uart.driver().clear_rx()?;

        if let Some(ref mut ctrl_pin_driver) = self.ctrl_pin_driver {
            ctrl_pin_driver.set_high().unwrap();
            Ets::delay_us(10);
        }

        self.uart_write_all(req).await?;

        if let Some(ref mut ctrl_pin_driver) = self.ctrl_pin_driver {
            ctrl_pin_driver.set_low().unwrap();
            Ets::delay_us(10);
        }

        // Read 3 bytes first: addr + func + (byte_count or exception_code).
        // This is the minimum needed by guess_response_frame_len for all cases:
        //   FC1-4: uses f[2] (byte count) → variable length
        //   FC5,6,15,16: uses f[1] only → fixed 8
        //   Exception (func >= 0x80): uses f[1] only → fixed 5
        self.uart_read_exactly(&mut resp[..3]).await?;
        let len = guess_response_frame_len(&resp[..3], ModbusProto::Rtu)? as usize;
        if len > 3 {
            self.uart_read_exactly(&mut resp[3..len]).await?;
        }
        Ok(len)
    }

    #[allow(dead_code)]
    pub async fn read_holding_register(&mut self, addr: u16) -> Result<u16> {
        let mut result = [0u16];
        self.read_holding_registers(addr, 1, &mut result).await?;
        Ok(result[0])
    }

    pub async fn read_holding_registers(
        &mut self,
        addr: u16,
        count: u16,
        result: &mut [u16],
    ) -> Result<()> {
        assert!(result.len() == count as usize);

        let mut request = ModbusRequest::new(self.device_id, ModbusProto::Rtu);
        let mut request_buf = fixedvec::alloc_stack!([u8; 256]);
        let mut response_buf = [0; 256];

        let mut frame_buf = FixedVec::new(&mut request_buf);

        request.generate_get_holdings(addr, count, &mut frame_buf)?;
        let len = self.modbus_request(frame_buf.as_slice(), &mut response_buf).await?;

        let mut result_vec = FixedVec::new(result);
        request.parse_u16(&response_buf[..len], &mut result_vec)?;
        Ok(())
    }

    pub async fn write_holding_register(&mut self, addr: u16, value: u16) -> Result<()> {
        let mut request = ModbusRequest::new(self.device_id, ModbusProto::Rtu);
        let mut request_buf = fixedvec::alloc_stack!([u8; 256]);
        let mut response_buf = [0; 256];

        let mut frame_buf = FixedVec::new(&mut request_buf);

        request.generate_set_holding(addr, value, &mut frame_buf)?;
        let len = self.modbus_request(frame_buf.as_slice(), &mut response_buf).await?;

        request.parse_ok(&response_buf[..len])?;
        Ok(())
    }

    pub async fn write_holding_registers(&mut self, addr: u16, values: &[u16]) -> Result<()> {
        let mut request = ModbusRequest::new(self.device_id, ModbusProto::Rtu);
        let mut request_buf = fixedvec::alloc_stack!([u8; 256]);
        let mut response_buf = [0; 256];

        let mut frame_buf = FixedVec::new(&mut request_buf);

        request.generate_set_holdings_bulk(addr, values, &mut frame_buf)?;
        let len = self.modbus_request(frame_buf.as_slice(), &mut response_buf).await?;

        request.parse_ok(&response_buf[..len])?;
        Ok(())
    }

    pub fn set_baudrate(&mut self, baudrate: u32) -> Result<()> {
        self.uart.driver().change_baudrate(baudrate)?;
        let timeout = Self::compute_timeout(baudrate, self.timeout_override_ms)?;
        self.read_timeout = timeout;
        self.write_timeout = timeout;
        Ok(())
    }
}

pub struct Modbus57AIM30Motor<'a> {
    client: ModbusRTUMaster<'a>,
    pos_min: f32,
    pos_max: f32,
    scan_delay_us: u32,
}

impl<'a> Modbus57AIM30Motor<'a> {
    pub fn new(modbus_client: ModbusRTUMaster<'a>, scan_delay_us: u32) -> Self {
        Self {
            client: modbus_client,
            pos_min: 0.0,
            pos_max: 0.0,
            scan_delay_us,
        }
    }

    async fn write_position_raw(&mut self, position: i32) -> Result<(), anyhow::Error> {
        let data = [position as u16, (position >> 16) as u16];
        self.client.write_holding_registers(0x16, &data).await?;
        Ok(())
    }

    async fn wait_stable_position(&mut self, timeout_ms: u32) -> Result<f32, anyhow::Error> {
        let start_time = time::Instant::now();
        let timeout = time::Duration::from_millis(timeout_ms as u64);
        let mut position = self.read_position().await?;
        Timer::after_millis(100).await;
        while start_time.elapsed() < timeout {
            let new_position = self.read_position().await?;
            if (new_position - position).abs() < 0.002 {
                return Ok(new_position);
            }
            position = new_position;
            Timer::after_millis(100).await;
        }
        Err(anyhow::anyhow!("Timeout waiting for stable position"))
    }

    async fn reset_position(&mut self) -> Result<(), anyhow::Error> {
        self.write_position_raw(0).await?;
        Ok(())
    }

    pub async fn modbus_scan(&mut self) -> Result<ModbusScanResult> {
        let baud_rates: [u32; _] = [115200, 9600, 19200, 38400];
        for baud_rate in baud_rates {
            self.client.set_baudrate(baud_rate)?;
            let delay_us = Self::modbus_t3_5_us(baud_rate).max(self.scan_delay_us);
            for device_id in 1..=247 {
                self.client.device_id = device_id;
                Timer::after_micros(delay_us as u64).await;
                if self.client.write_holding_register(0x00, 0x01).await.is_ok() {
                    return Ok(ModbusScanResult {
                        baud_rate,
                        device_id,
                    });
                }
            }
        }
        Err(anyhow::anyhow!("no response"))
    }

    fn modbus_t3_5_us(baud_rate: u32) -> u32 {
        // t3.5 = 3.5 characters × 11 bits/char × 1_000_000 / baud_rate
        // Minimum 1750 µs for baud rates > 19200 per Modbus spec
        let calculated = 3_5 * 11 * 1_000_000 / (baud_rate * 10);
        calculated.max(1750)
    }

    pub async fn modbus_set_baud_rate(&mut self, baud_rate: u32) -> Result<(), anyhow::Error> {
        let baud_rate_code = match baud_rate {
            9600 => 800,
            19200 => 801,
            38400 => 802,
            115200 => 803,
            _ => return Err(anyhow::anyhow!("Invalid baud rate")),
        };
        self.client.write_holding_register(0x00, 1).await?;
        self.client.write_holding_register(0x03, baud_rate_code).await?;
        self.client.write_holding_register(0x04, 129).await?;
        self.client.write_holding_register(0x00, 506).await?;
        Ok(())
    }

    pub async fn enable_modbus_communication(&mut self) -> Result<(), anyhow::Error> {
        self.client.write_holding_register(0x00, 0x01).await?;
        Ok(())
    }
}

impl<'a> Motor for Modbus57AIM30Motor<'a> {
    async fn read_position(&mut self) -> Result<f32, anyhow::Error> {
        let mut rsp = [0u16; 2];
        self.client.read_holding_registers(0x16, 2, &mut rsp).await?;
        let low = rsp[0];
        let high = rsp[1];
        let position = (high as i32) << 16 | low as i32;
        Ok(position as f32 / 32768.0 * 2.0 * std::f32::consts::PI)
    }

    async fn write_position(&mut self, position: f32, _speed: f32) -> Result<(), anyhow::Error> {
        let position_i32 = (position / (2.0 * std::f32::consts::PI) * 32768.0) as i32;
        if position_i32 == 0 {
            self.write_position_raw(1).await
        } else {
            self.write_position_raw(position_i32).await
        }
    }

    async fn set_max_power(&mut self, power: f32) -> Result<(), anyhow::Error> {
        let power_val = (power * 60.0) as u16 * 10;
        self.client.write_holding_register(0x18, power_val).await?;
        Ok(())
    }

    async fn set_acceleration(&mut self, acceleration: f32) -> Result<(), anyhow::Error> {
        let acceleration_val = (acceleration * 60.0 / (2.0 * std::f32::consts::PI)) as u16;    // rad/s^2 => rpm
        self.client.write_holding_register(0x03, acceleration_val).await?;
        Ok(())
    }

    async fn set_position_ring_ratio(&mut self, ratio: f32) -> Result<(), anyhow::Error> {
        self.client.write_holding_register(0x07, ratio as u16).await?;
        Ok(())
    }

    async fn set_speed_ring_ratio(&mut self, ratio: f32) -> Result<(), anyhow::Error> {
        self.client.write_holding_register(0x05, ratio as u16).await?;
        Ok(())
    }

    async fn homing(&mut self) -> Result<(), anyhow::Error> {
        assert!(
            self.pos_min == 0.0 && self.pos_max == 0.0,
            "Motor already homed"
        );

        log::info!("Homing motor");

        self.set_max_power(0.1).await?;
        self.set_acceleration(1000.0).await?;
        self.reset_position().await?;
        log::info!("Writing position to -100.0");
        self.write_position(-100.0, 0.0).await?;
        Timer::after_millis(5000).await;
        self.pos_min = self.wait_stable_position(5000).await? + 0.1;
        log::info!("pos_min: {}", self.pos_min);

        log::info!("Writing position to 100.0");
        self.write_position(100.0, 0.0).await?;
        Timer::after_millis(5000).await;
        self.pos_max = self.wait_stable_position(5000).await? - 0.1;
        log::info!("pos_max: {}", self.pos_max);

        self.write_position((self.pos_min + self.pos_max) / 2.0, 0.0).await?;
        Timer::after_millis(5000).await;
        self.wait_stable_position(5000).await?;

        Ok(())
    }

    fn pos_min(&self) -> f32 {
        self.pos_min
    }

    fn pos_max(&self) -> f32 {
        self.pos_max
    }

    async fn cycle(&mut self) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct ModbusScanResult {
    pub baud_rate: u32,
    pub device_id: u8,
}
