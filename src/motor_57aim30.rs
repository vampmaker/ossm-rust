use std::time;

use crate::motor::Motor;
use esp_idf_svc::hal::delay::{Ets, FreeRtos, TickType_t};
use esp_idf_svc::hal::gpio::{self, AnyOutputPin};
use esp_idf_svc::hal::uart;
use esp_idf_svc::hal::delay::TICK_RATE_HZ;

use fixedvec::FixedVec;
use rmodbus::{client::ModbusRequest, guess_response_frame_len, ModbusProto};
use anyhow::Result;


pub struct ModbusRTUMaster<'a> {
    uart: uart::UartDriver<'a>,
    ctrl_pin_driver: Option<gpio::PinDriver<'a, AnyOutputPin, gpio::Output>>,
    device_id: u8,
    read_timeout: TickType_t,
    write_timeout: TickType_t,
}

impl<'a> ModbusRTUMaster<'a> {
    pub fn new(
        uart: uart::UartDriver<'a>,
        ctrl_pin: Option<gpio::AnyOutputPin>,
        device_id: u8,
    ) -> Self {
        let ctrl_pin_driver = if let Some(ctrl_pin) = ctrl_pin {
            Some(gpio::PinDriver::output(ctrl_pin).unwrap())
        } else {
            None
        };
        let timeout = Self::get_operation_timeout(uart.baudrate().unwrap().into()).unwrap();
        let result = Self {
            uart,
            ctrl_pin_driver,
            device_id,
            read_timeout: timeout,
            write_timeout: timeout,
        };
        result
    }

    fn get_operation_timeout(baudrate: u32) -> Result<TickType_t> {
        match baudrate {
            9600 => Ok(TICK_RATE_HZ / 10),
            19200 => Ok(TICK_RATE_HZ / 20),
            38400 => Ok(TICK_RATE_HZ / 40),
            115200 | 115201 => Ok(TICK_RATE_HZ / 200),
            _ => Err(anyhow::anyhow!("Invalid baud rate: {}", baudrate)),
        }
    }

    fn uart_read_exactly(&mut self, buf: &mut [u8]) -> Result<()> {
        let mut total_bytes_read = 0;
        while total_bytes_read < buf.len() {
            let bytes_read = self
                .uart
                .read(&mut buf[total_bytes_read..], self.read_timeout)?;
            total_bytes_read += bytes_read;
        }
        Ok(())
    }

    fn uart_write_all(&mut self, buf: &[u8]) -> Result<()> {
        let mut total_bytes_written = 0;
        while total_bytes_written < buf.len() {
            let bytes_written = self.uart.write(&buf[total_bytes_written..])?;
            total_bytes_written += bytes_written;
        }
        self.uart.wait_tx_done(self.write_timeout)?;
        Ok(())
    }

    fn modbus_request(&mut self, req: &[u8], resp: &mut [u8]) -> Result<usize> {
        assert!(resp.len() >= 256);

        if let Some(ref mut ctrl_pin_driver) = self.ctrl_pin_driver {
            ctrl_pin_driver.set_high().unwrap();
            Ets::delay_us(10);
        }

        self.uart_write_all(req)?;

        if let Some(ref mut ctrl_pin_driver) = self.ctrl_pin_driver {
            ctrl_pin_driver.set_low().unwrap();
            Ets::delay_us(10);
        }
        
        self.uart_read_exactly(&mut resp[..6])?;
        let len = guess_response_frame_len(&resp[..6], ModbusProto::Rtu)? as usize;
        if len > 6 {
            self.uart_read_exactly(&mut resp[6..len])?;
        }
        Ok(len)
    }

    pub fn read_holding_register(&mut self, addr: u16) -> Result<u16> {
        let mut result = [0u16];
        self.read_holding_registers(addr, 1, &mut result)?;
        Ok(result[0])
    }

    pub fn read_holding_registers(
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
        let len = self.modbus_request(frame_buf.as_slice(), &mut response_buf)?;

        let mut result_vec = FixedVec::new(result);
        request.parse_u16(&response_buf[..len], &mut result_vec)?;
        Ok(())
    }

    pub fn write_holding_register(&mut self, addr: u16, value: u16) -> Result<()> {
        let mut request = ModbusRequest::new(self.device_id, ModbusProto::Rtu);
        let mut request_buf = fixedvec::alloc_stack!([u8; 256]);
        let mut response_buf = [0; 256];

        let mut frame_buf = FixedVec::new(&mut request_buf);

        request.generate_set_holding(addr, value, &mut frame_buf)?;
        let len = self.modbus_request(frame_buf.as_slice(), &mut response_buf)?;

        request.parse_ok(&response_buf[..len])?;
        Ok(())
    }

    pub fn write_holding_registers(&mut self, addr: u16, values: &[u16]) -> Result<()> {
        let mut request = ModbusRequest::new(self.device_id, ModbusProto::Rtu);
        let mut request_buf = fixedvec::alloc_stack!([u8; 256]);
        let mut response_buf = [0; 256];

        let mut frame_buf = FixedVec::new(&mut request_buf);

        request.generate_set_holdings_bulk(addr, values, &mut frame_buf)?;
        let len = self.modbus_request(frame_buf.as_slice(), &mut response_buf)?;

        request.parse_ok(&response_buf[..len])?;
        Ok(())
    }

    pub fn set_baudrate(&mut self, baudrate: u32) -> Result<()> {
        self.uart.change_baudrate(baudrate)?;
        let timeout = Self::get_operation_timeout(baudrate)?;
        self.read_timeout = timeout;
        self.write_timeout = timeout;
        Ok(())
    }
}

pub struct Modbus57AIM30Motor<'a> {
    client: ModbusRTUMaster<'a>,
    pos_min: i32,
    pos_max: i32,
}

impl<'a> Modbus57AIM30Motor<'a> {
    pub fn new(modbus_client: ModbusRTUMaster<'a>) -> Self {
        Self {
            client: modbus_client,
            pos_min: 0,
            pos_max: 0,
        }
    }

    fn write_position_raw(&mut self, position: i32) -> Result<(), anyhow::Error> {
        let data = [position as u16, (position >> 16) as u16];
        self.client.write_holding_registers(0x16, &data)?;
        Ok(())
    }

    fn wait_stable_position(&mut self, timeout_ms: u32) -> Result<i32, anyhow::Error> {
        let start_time = time::SystemTime::now();
        let timeout = time::Duration::from_millis(timeout_ms as u64);
        let mut position = self.read_position()?;
        while start_time.elapsed()? < timeout {
            let new_position = self.read_position()?;
            if (new_position - position).abs() < 10 {
                return Ok(new_position);
            }
            position = new_position;
            FreeRtos::delay_ms(100);
        }
        Err(anyhow::anyhow!("Timeout waiting for stable position"))
    }

    fn reset_position(&mut self) -> Result<(), anyhow::Error> {
        self.write_position_raw(0)?;
        Ok(())
    }

    pub fn modbus_scan(&mut self) -> Result<ModbusScanResult> {
        let baud_rates: [u32; _] = [115200, 9600, 19200, 38400];
        for baud_rate in baud_rates {
            self.client.set_baudrate(baud_rate)?;
            for device_id in 1..=247 {
                self.client.device_id = device_id;
                match self.client.read_holding_register(0x00) {
                    Ok(_) => {
                        return Ok(ModbusScanResult {
                            baud_rate,
                            device_id,
                        });
                    }
                    _ => {}
                }
            }
        }
        Err(anyhow::anyhow!("no response"))
    }

    pub fn modbus_set_baud_rate(&mut self, baud_rate: u32) -> Result<(), anyhow::Error> {
        let baud_rate_code = match baud_rate {
            9600 => 800,
            19200 => 801,
            38400 => 802,
            115200 => 803,
            _ => return Err(anyhow::anyhow!("Invalid baud rate")),
        };
        self.client.write_holding_register(0x00, 1)?;
        self.client.write_holding_register(0x03, baud_rate_code)?;
        self.client.write_holding_register(0x04, 129)?;
        self.client.write_holding_register(0x00, 506)?;
        Ok(())
    }

    pub fn enable_modbus_communication(&mut self) -> Result<(), anyhow::Error> {
        self.client.write_holding_register(0x00, 0x01)?;
        Ok(())
    }
}

impl<'a> Motor for Modbus57AIM30Motor<'a> {
    fn read_position(&mut self) -> Result<i32, anyhow::Error> {
        let mut rsp = [0u16; 2];
        self.client.read_holding_registers(0x16, 2, &mut rsp)?;
        let low = rsp[0];
        let high = rsp[1];
        let position = (high as i32) << 16 | low as i32;
        Ok(position)
    }

    fn write_position(&mut self, position: i32, _speed: f32) -> Result<(), anyhow::Error> {
        if position == 0 {
            self.write_position_raw(1)
        } else {
            self.write_position_raw(position)
        }
    }

    fn set_max_power(&mut self, power: u16) -> Result<(), anyhow::Error> {
        self.client.write_holding_register(0x18, power)?;
        Ok(())
    }

    fn set_acceleration(&mut self, acceleration: u16) -> Result<(), anyhow::Error> {
        self.client.write_holding_register(0x03, acceleration)?;
        Ok(())
    }

    fn set_position_ring_ratio(&mut self, ratio: u16) -> Result<(), anyhow::Error> {
        self.client.write_holding_register(0x07, ratio)?;
        Ok(())
    }

    fn set_speed_ring_ratio(&mut self, ratio: u16) -> Result<(), anyhow::Error> {
        self.client.write_holding_register(0x05, ratio)?;
        Ok(())
    }

    fn homing(&mut self) -> Result<(), anyhow::Error> {
        assert!(
            self.pos_min == 0 && self.pos_max == 0,
            "Motor already homed"
        );

        self.set_max_power(60)?;
        self.set_acceleration(10000)?;
        self.reset_position()?;
        self.write_position(-1000000, 0.0)?;
        FreeRtos::delay_ms(5000);
        self.pos_min = self.wait_stable_position(5000)? + 3000;

        self.write_position(1000000, 0.0)?;
        FreeRtos::delay_ms(5000);
        self.pos_max = self.wait_stable_position(5000)? - 3000;

        self.write_position((self.pos_min + self.pos_max) / 2, 0.0)?;
        FreeRtos::delay_ms(5000);
        self.wait_stable_position(5000)?;

        Ok(())
    }

    fn pos_min(&self) -> i32 {
        self.pos_min
    }

    fn pos_max(&self) -> i32 {
        self.pos_max
    }

    fn cycle(&mut self) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct ModbusScanResult {
    pub baud_rate: u32,
    pub device_id: u8,
}
