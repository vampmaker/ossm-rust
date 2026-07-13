use core::f32::consts::PI;

use embassy_time::{with_timeout, Duration, Instant, Timer};
use esp_hal::delay::Delay;
use esp_hal::gpio::{AnyPin, Level, Output, OutputConfig};
use esp_hal::uart::{Config, Uart};
use fixedvec::FixedVec;
use rmodbus::{client::ModbusRequest, guess_response_frame_len, ModbusProto};

use crate::context::AppContext;
use crate::error::{FirmwareError, Result};
use crate::motion::{
    LoopStats, ModbusStats, MotorController, MotorControllerConfig, TimingWindowStats,
};
use crate::motor::Motor;
use crate::storage::PinConfiguration;

pub const TARGET_BAUD_RATE: u32 = 115200;

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameTiming {
    pub round_trip_us: u16,
    pub slave_latency_us: u16,
    pub rx_duration_us: u16,
}

#[allow(dead_code)]
pub struct ModbusRTUMaster<'d> {
    uart: Uart<'d, esp_hal::Async>,
    de_re: Option<Output<'d>>,
    device_id: u8,
    read_timeout: Duration,
    rx_inter_byte_timeout: Duration,
    write_timeout: Duration,
    timeout_override_ms: u32,
    rx_timeout_override_us: u32,
    inter_frame_delay: Duration,
    inter_frame_delay_override_us: u32,
    baudrate: u32,

    last_stats_window: Instant,
    current_successes: u32,
    current_failures: u32,
    current_round_trip_us: alloc::vec::Vec<u16>,
    current_slave_latency_us: alloc::vec::Vec<u16>,
    current_rx_duration_us: alloc::vec::Vec<u16>,
    pub latest_stats: ModbusStats,
}

fn compute_timing_stats(samples: &mut [u16]) -> TimingWindowStats {
    if samples.is_empty() {
        return TimingWindowStats::default();
    }
    samples.sort_unstable();
    let n = samples.len();
    let min = samples[0];
    let max = samples[n - 1];
    let pct5 = samples[(n * 5) / 100];
    let pct10 = samples[(n * 10) / 100];
    let pct50 = samples[(n * 50) / 100];
    let pct90 = samples[(n * 90) / 100];
    let pct95 = samples[(n * 95) / 100];

    let sum: u32 = samples.iter().map(|&x| x as u32).sum();
    let mean = (sum / n as u32) as u16;

    let dev_sum: u32 = samples
        .iter()
        .map(|&x| (x as i32 - mean as i32).unsigned_abs())
        .sum();
    let mdev = (dev_sum / n as u32) as u16;

    TimingWindowStats {
        min,
        max,
        pct5,
        pct10,
        pct50,
        pct90,
        pct95,
        mean,
        mdev,
    }
}

impl<'d> ModbusRTUMaster<'d> {
    pub fn new(
        uart: Uart<'d, esp_hal::Async>,
        de_re: Option<Output<'d>>,
        device_id: u8,
        timeout_override_ms: u32,
        rx_timeout_override_us: u32,
        inter_frame_delay_override_us: u32,
        baudrate: u32,
    ) -> Self {
        let timeout = Self::compute_timeout(baudrate, timeout_override_ms).unwrap();
        let rx_inter_byte_timeout =
            Self::compute_rx_inter_byte_timeout(baudrate, rx_timeout_override_us).unwrap();
        let inter_frame_delay =
            Self::compute_inter_frame_delay(baudrate, inter_frame_delay_override_us).unwrap();
        Self {
            uart,
            de_re,
            device_id,
            read_timeout: timeout,
            rx_inter_byte_timeout,
            write_timeout: timeout,
            timeout_override_ms,
            rx_timeout_override_us,
            inter_frame_delay,
            inter_frame_delay_override_us,
            baudrate,
            last_stats_window: Instant::now(),
            current_successes: 0,
            current_failures: 0,
            current_round_trip_us: alloc::vec::Vec::with_capacity(350),
            current_slave_latency_us: alloc::vec::Vec::with_capacity(350),
            current_rx_duration_us: alloc::vec::Vec::with_capacity(350),
            latest_stats: ModbusStats::default(),
        }
    }

    fn get_default_timeout(baudrate: u32) -> Result<Duration> {
        match baudrate {
            9600 => Ok(Duration::from_millis(100)),
            19200 => Ok(Duration::from_millis(50)),
            38400 => Ok(Duration::from_millis(25)),
            115200 | 115201 => Ok(Duration::from_millis(10)),
            _ => Err(FirmwareError::Modbus("invalid baud rate")),
        }
    }

    fn compute_timeout(baudrate: u32, override_ms: u32) -> Result<Duration> {
        if override_ms > 0 {
            Ok(Duration::from_millis(override_ms as u64))
        } else {
            Self::get_default_timeout(baudrate)
        }
    }

    fn get_default_rx_inter_byte_timeout(baudrate: u32) -> Result<Duration> {
        // According to official Modbus RTU spec (over Serial Line Guide V1.02, Sec 2.5.1.1):
        // For baud rates > 19200 Bps, fixed inter-character timeout (t1.5) recommended is 750 us.
        // On embedded async executors controlling external stepper controllers (e.g., 57AIM30),
        // we use 1750 us (1.75 ms) as the default inter-byte read timeout to accommodate
        // slave MCU interrupt jitter and async task scheduling without false timeouts.
        match baudrate {
            9600 => Ok(Duration::from_micros(4000)),
            19200 => Ok(Duration::from_micros(2500)),
            38400 => Ok(Duration::from_micros(1750)),
            115200 | 115201 => Ok(Duration::from_micros(1750)),
            _ => Err(FirmwareError::Modbus("invalid baud rate")),
        }
    }

    fn compute_rx_inter_byte_timeout(baudrate: u32, override_us: u32) -> Result<Duration> {
        if override_us > 0 {
            Ok(Duration::from_micros(override_us as u64))
        } else {
            Self::get_default_rx_inter_byte_timeout(baudrate)
        }
    }

    fn get_default_inter_frame_delay(baudrate: u32) -> Result<Duration> {
        // To achieve our 300Hz position update rate target (<= 3ms total end-to-end
        // request-response duration), we use 350 us inter-frame delay (~3.5 character times)
        // after a complete frame reception, while maintaining 1750 us for rx_inter_byte_timeout.
        match baudrate {
            9600 => Ok(Duration::from_micros(4000)),
            19200 => Ok(Duration::from_micros(2000)),
            38400 => Ok(Duration::from_micros(1000)),
            115200 | 115201 => Ok(Duration::from_micros(350)),
            _ => Err(FirmwareError::Modbus("invalid baud rate")),
        }
    }

    fn compute_inter_frame_delay(baudrate: u32, override_us: u32) -> Result<Duration> {
        if override_us > 0 {
            Ok(Duration::from_micros(override_us as u64))
        } else {
            Self::get_default_inter_frame_delay(baudrate)
        }
    }

    fn record_sample(&mut self, timing: FrameTiming) {
        self.current_successes = self.current_successes.saturating_add(1);
        if self.current_round_trip_us.len() < 350 {
            self.current_round_trip_us.push(timing.round_trip_us);
            self.current_slave_latency_us.push(timing.slave_latency_us);
            self.current_rx_duration_us.push(timing.rx_duration_us);
        }
        self.maybe_flush_stats();
    }

    fn record_failure(&mut self) {
        self.current_failures = self.current_failures.saturating_add(1);
        self.maybe_flush_stats();
    }

    fn maybe_flush_stats(&mut self) {
        if Instant::now().duration_since(self.last_stats_window) >= Duration::from_secs(1) {
            let total = self.current_successes + self.current_failures;
            let success_rate = if total > 0 {
                (self.current_successes as f32 / total as f32) * 100.0
            } else {
                0.0
            };

            let round_trip = compute_timing_stats(&mut self.current_round_trip_us);
            let slave_latency = compute_timing_stats(&mut self.current_slave_latency_us);
            let rx_duration = compute_timing_stats(&mut self.current_rx_duration_us);

            self.latest_stats = ModbusStats {
                successful_requests: self.current_successes,
                failed_requests: self.current_failures,
                success_rate,
                round_trip,
                slave_latency,
                rx_duration,
            };

            self.current_successes = 0;
            self.current_failures = 0;
            self.current_round_trip_us.clear();
            self.current_slave_latency_us.clear();
            self.current_rx_duration_us.clear();
            self.last_stats_window = Instant::now();
        }
    }

    pub fn get_stats(&mut self) -> ModbusStats {
        self.maybe_flush_stats();
        self.latest_stats.clone()
    }

    async fn uart_read_exactly(
        &mut self,
        buf: &mut [u8],
        deadline: Instant,
        first_byte_seen: &mut bool,
        t4: &mut Option<Instant>,
    ) -> Result<()> {
        let mut total_bytes_read = 0;
        while total_bytes_read < buf.len() {
            let now = Instant::now();
            if now >= deadline {
                return Err(FirmwareError::Uart("read timeout"));
            }
            let remaining = deadline.duration_since(now);
            let step_timeout = if !*first_byte_seen {
                remaining
            } else {
                self.rx_inter_byte_timeout.min(remaining)
            };

            let bytes_read = with_timeout(
                step_timeout,
                self.uart.read_async(&mut buf[total_bytes_read..]),
            )
            .await
            .map_err(|_| FirmwareError::Uart("read timeout"))?
            .map_err(|_| FirmwareError::Uart("read error"))?;

            if bytes_read > 0 && !*first_byte_seen {
                *first_byte_seen = true;
                *t4 = Some(Instant::now());
            }
            total_bytes_read += bytes_read;
        }
        Ok(())
    }

    async fn uart_write_all(&mut self, buf: &[u8]) -> Result<()> {
        let mut total_bytes_written = 0;
        while total_bytes_written < buf.len() {
            let bytes_written = with_timeout(
                self.write_timeout,
                self.uart.write_async(&buf[total_bytes_written..]),
            )
            .await
            .map_err(|_| FirmwareError::Uart("write timeout"))?
            .map_err(|_| FirmwareError::Uart("write error"))?;
            total_bytes_written += bytes_written;
        }
        self.uart
            .flush_async()
            .await
            .map_err(|_| FirmwareError::Uart("flush error"))?;
        Ok(())
    }

    async fn modbus_request_inner(
        &mut self,
        req: &[u8],
        resp: &mut [u8],
    ) -> Result<(usize, FrameTiming)> {
        let t0 = Instant::now();
        let deadline = t0 + self.read_timeout;

        if let Some(ref mut pin) = self.de_re {
            pin.set_high();
            Delay::new().delay_micros(10);
        }

        self.uart_write_all(req).await?;

        if let Some(ref mut pin) = self.de_re {
            Delay::new().delay_micros(150);
            pin.set_low();
        }
        let t2 = Instant::now();

        let mut first_byte_seen = false;
        let mut t4: Option<Instant> = None;

        self.uart_read_exactly(&mut resp[..3], deadline, &mut first_byte_seen, &mut t4)
            .await?;
        let len = guess_response_frame_len(&resp[..3], ModbusProto::Rtu)
            .map_err(|_| FirmwareError::Modbus("bad frame"))? as usize;
        if len > 3 {
            self.uart_read_exactly(&mut resp[3..len], deadline, &mut first_byte_seen, &mut t4)
                .await?;
        }
        let t5 = Instant::now();
        Timer::after(self.inter_frame_delay).await;

        let round_trip_us = t5.duration_since(t0).as_micros().min(u16::MAX as u64) as u16;
        let slave_latency_us = t4
            .unwrap_or(t5)
            .duration_since(t2)
            .as_micros()
            .min(u16::MAX as u64) as u16;
        let rx_duration_us = t5
            .duration_since(t4.unwrap_or(t5))
            .as_micros()
            .min(u16::MAX as u64) as u16;

        let timing = FrameTiming {
            round_trip_us,
            slave_latency_us,
            rx_duration_us,
        };
        Ok((len, timing))
    }

    async fn modbus_request(&mut self, req: &[u8], resp: &mut [u8]) -> Result<usize> {
        match self.modbus_request_inner(req, resp).await {
            Ok((len, timing)) => {
                self.record_sample(timing);
                Ok(len)
            }
            Err(e) => {
                self.record_failure();
                Err(e)
            }
        }
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

        request
            .generate_get_holdings(addr, count, &mut frame_buf)
            .map_err(|_| FirmwareError::Modbus("request"))?;
        let len = self
            .modbus_request(frame_buf.as_slice(), &mut response_buf)
            .await?;

        let mut result_vec = FixedVec::new(result);
        request
            .parse_u16(&response_buf[..len], &mut result_vec)
            .map_err(|_| FirmwareError::Modbus("parse"))?;
        Ok(())
    }

    pub async fn write_holding_register(&mut self, addr: u16, value: u16) -> Result<()> {
        let mut request = ModbusRequest::new(self.device_id, ModbusProto::Rtu);
        let mut request_buf = fixedvec::alloc_stack!([u8; 256]);
        let mut response_buf = [0; 256];
        let mut frame_buf = FixedVec::new(&mut request_buf);

        request
            .generate_set_holding(addr, value, &mut frame_buf)
            .map_err(|_| FirmwareError::Modbus("request"))?;
        let len = self
            .modbus_request(frame_buf.as_slice(), &mut response_buf)
            .await?;
        request
            .parse_ok(&response_buf[..len])
            .map_err(|_| FirmwareError::Modbus("parse"))?;
        Ok(())
    }

    pub async fn write_holding_registers(&mut self, addr: u16, values: &[u16]) -> Result<()> {
        let mut request = ModbusRequest::new(self.device_id, ModbusProto::Rtu);
        let mut request_buf = fixedvec::alloc_stack!([u8; 256]);
        let mut response_buf = [0; 256];
        let mut frame_buf = FixedVec::new(&mut request_buf);

        request
            .generate_set_holdings_bulk(addr, values, &mut frame_buf)
            .map_err(|_| FirmwareError::Modbus("request"))?;
        let len = self
            .modbus_request(frame_buf.as_slice(), &mut response_buf)
            .await?;
        request
            .parse_ok(&response_buf[..len])
            .map_err(|_| FirmwareError::Modbus("parse"))?;
        Ok(())
    }

    pub fn set_baudrate(&mut self, baudrate: u32) -> Result<()> {
        self.uart
            .apply_config(&Config::default().with_baudrate(baudrate))
            .map_err(|_| FirmwareError::Uart("baud"))?;
        self.baudrate = baudrate;
        let timeout = Self::compute_timeout(baudrate, self.timeout_override_ms)?;
        self.read_timeout = timeout;
        self.write_timeout = timeout;
        self.inter_frame_delay =
            Self::compute_inter_frame_delay(baudrate, self.inter_frame_delay_override_us)?;
        Ok(())
    }
}

pub struct Modbus57AIM30Motor<'d> {
    client: ModbusRTUMaster<'d>,
    pos_min: f32,
    pos_max: f32,
    scan_delay_us: u32,
}

impl<'d> Modbus57AIM30Motor<'d> {
    pub fn new(modbus_client: ModbusRTUMaster<'d>, scan_delay_us: u32) -> Self {
        Self {
            client: modbus_client,
            pos_min: 0.0,
            pos_max: 0.0,
            scan_delay_us,
        }
    }

    pub fn get_stats(&mut self) -> ModbusStats {
        self.client.get_stats()
    }

    async fn write_position_raw(&mut self, position: i32) -> Result<()> {
        let data = [position as u16, (position >> 16) as u16];
        self.client.write_holding_registers(0x16, &data).await
    }

    async fn wait_stable_position(&mut self, timeout_ms: u32) -> Result<f32> {
        let start_time = Instant::now();
        let timeout = Duration::from_millis(timeout_ms as u64);
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
        Err(FirmwareError::Modbus("stable position timeout"))
    }

    async fn reset_position(&mut self) -> Result<()> {
        self.write_position_raw(0).await
    }

    pub async fn modbus_scan(&mut self) -> Result<ModbusScanResult> {
        let baud_rates = [115200u32, 9600, 19200, 38400]; // baudrate supported by 57AIM30
        for baud_rate in baud_rates {
            let _ = self.client.uart.flush_async().await;
            self.client.set_baudrate(baud_rate)?;
            Timer::after_micros(5000).await; // ensure motor modbus rx state machine is in a clean state before proceeding: wait for t3.5 for 9600

            // reset the uart into a clean state
            let _ = self.client.uart.flush_async().await;
            let _ = self.client.uart.check_for_rx_errors();

            let delay_us = Self::modbus_t3_5_us(baud_rate).max(self.scan_delay_us);
            for device_id in 1..=247u8 {
                // full address space for modbus device id (0 is for broadcast, 248-255 are reserved)
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
        Err(FirmwareError::Modbus("no response"))
    }

    fn modbus_t3_5_us(baud_rate: u32) -> u32 {
        let calculated = 3_5 * 11 * 1_000_000 / (baud_rate * 10);
        calculated.max(1750)
    }

    pub async fn modbus_set_baud_rate(&mut self, baud_rate: u32) -> Result<()> {
        let baud_rate_code = match baud_rate {
            9600 => 800,
            19200 => 801,
            38400 => 802,
            115200 => 803,
            _ => return Err(FirmwareError::Modbus("invalid baud rate")),
        };
        self.client.write_holding_register(0x00, 1).await?;
        self.client
            .write_holding_register(0x03, baud_rate_code)
            .await?;
        self.client.write_holding_register(0x04, 129).await?;
        self.client.write_holding_register(0x00, 506).await?;
        Ok(())
    }

    pub async fn enable_modbus_communication(&mut self) -> Result<()> {
        self.client.write_holding_register(0x00, 0x01).await
    }
}

impl<'d> Motor for Modbus57AIM30Motor<'d> {
    async fn read_position(&mut self) -> Result<f32> {
        let mut rsp = [0u16; 2];
        self.client
            .read_holding_registers(0x16, 2, &mut rsp)
            .await?;
        let low = rsp[0];
        let high = rsp[1];
        let position = (high as i32) << 16 | low as i32;
        Ok(position as f32 / 32768.0 * 2.0 * PI)
    }

    async fn write_position(&mut self, position: f32, _speed: f32) -> Result<()> {
        let position_i32 = (position / (2.0 * PI) * 32768.0) as i32;
        if position_i32 == 0 {
            self.write_position_raw(1).await
        } else {
            self.write_position_raw(position_i32).await
        }
    }

    async fn set_max_power(&mut self, power: f32) -> Result<()> {
        let power_val = (power * 60.0) as u16 * 10;
        self.client.write_holding_register(0x18, power_val).await
    }

    async fn set_acceleration(&mut self, acceleration: f32) -> Result<()> {
        let acceleration_val = (acceleration * 60.0 / (2.0 * PI)) as u16;
        self.client
            .write_holding_register(0x03, acceleration_val)
            .await
    }

    async fn set_position_ring_ratio(&mut self, ratio: f32) -> Result<()> {
        self.client.write_holding_register(0x07, ratio as u16).await
    }

    async fn set_speed_ring_ratio(&mut self, ratio: f32) -> Result<()> {
        self.client.write_holding_register(0x05, ratio as u16).await
    }

    async fn homing(&mut self) -> Result<()> {
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

        self.write_position((self.pos_min + self.pos_max) / 2.0, 0.0)
            .await?;
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

fn init_uart_and_modbus(
    uart_periph: esp_hal::peripherals::UART1<'static>,
    pin_config: &PinConfiguration,
) -> Result<ModbusRTUMaster<'static>> {
    let tx_pin = unsafe { AnyPin::steal(pin_config.modbus_tx as u8) };
    let rx_pin = unsafe { AnyPin::steal(pin_config.modbus_rx as u8) };
    let de_pin = unsafe { AnyPin::steal(pin_config.modbus_de_re as u8) };

    let de_re = Output::new(de_pin, Level::Low, OutputConfig::default());
    let rx_timeout_us = if pin_config.modbus_rx_timeout_us == 0 {
        1750
    } else {
        pin_config.modbus_rx_timeout_us
    };
    let timeout_symbols = (rx_timeout_us / 87).clamp(4, 127) as u8;
    let rx_config = esp_hal::uart::RxConfig::default().with_timeout(timeout_symbols);
    let uart_config = Config::default()
        .with_baudrate(TARGET_BAUD_RATE)
        .with_rx(rx_config);
    let uart = Uart::new(uart_periph, uart_config)
        .map_err(|_| FirmwareError::Uart("init"))?
        .with_tx(tx_pin)
        .with_rx(rx_pin)
        .into_async();

    Ok(ModbusRTUMaster::new(
        uart,
        Some(de_re),
        1,
        pin_config.modbus_timeout_ms,
        pin_config.modbus_rx_timeout_us,
        pin_config.modbus_inter_frame_delay_us,
        TARGET_BAUD_RATE,
    ))
}

pub async fn run_motor(
    app_context: AppContext,
    uart_periph: esp_hal::peripherals::UART1<'static>,
    pin_config: PinConfiguration,
) -> Result<()> {
    let motor_config = {
        let mut sm = app_context.storage.lock().await;
        match sm.get_motor_config() {
            Ok(config) => {
                log::info!("Loaded motor config from NVS");
                config
            }
            Err(_) => {
                log::info!("No motor config in NVS, using default");
                let default_config = MotorControllerConfig::default();
                let _ = sm.set_motor_config(&default_config);
                default_config
            }
        }
    };

    {
        let mut mc_lock = app_context.motor_controller.lock().await;
        *mc_lock = Some(MotorController::new(motor_config.clone()));
    }

    log::info!("Waiting 3s for WiFi/BLE initialization to settle before scanning motor...");
    Timer::after_millis(3000).await;

    let mut motor = Modbus57AIM30Motor::new(
        init_uart_and_modbus(uart_periph, &pin_config)?,
        pin_config.modbus_scan_delay_us,
    );

    let mut modbus_ok = false;
    for init_attempt in 1..=2 {
        if init_attempt > 1 {
            log::info!(
                "Retrying motor initialization (attempt {}/2)...",
                init_attempt
            );
            Timer::after_millis(1000).await;
        }
        match motor.enable_modbus_communication().await {
            Ok(()) => {
                modbus_ok = true;
                break;
            }
            Err(e) => {
                log::info!(
                    "Failed to enable modbus (attempt {}/2), trying scan: {}",
                    init_attempt,
                    e
                );
                let mut scan_result = Err(FirmwareError::Modbus("scan not attempted"));
                for attempt in 1..=3 {
                    match motor.modbus_scan().await {
                        Ok(result) => {
                            scan_result = Ok(result);
                            break;
                        }
                        Err(e) => {
                            log::warn!("Scan attempt {}/3 failed: {}", attempt, e);
                            scan_result = Err(e);
                        }
                    }
                }
                if let Ok(motor_scan_result) = scan_result {
                    log::info!(
                        "Motor found, baud={}, id={}",
                        motor_scan_result.baud_rate,
                        motor_scan_result.device_id
                    );
                    if motor_scan_result.baud_rate != TARGET_BAUD_RATE {
                        motor
                            .modbus_set_baud_rate(TARGET_BAUD_RATE)
                            .await
                            .map_err(|_| FirmwareError::Modbus("set baud"))?;
                        log::info!(
                            "Motor baud set to {}, power cycle motor if needed",
                            TARGET_BAUD_RATE
                        );
                    }
                    modbus_ok = true;
                    break;
                }
            }
        }
    }

    if !modbus_ok {
        log::warn!(
            "Modbus motor not detected after retries, running in disconnected telemetry mode"
        );
        loop {
            Timer::after_millis(100).await;
            let mut mc_lock = app_context.motor_controller.lock().await;
            if let Some(mc) = mc_lock.as_mut() {
                mc.set_motor_connected(false);
                let _ = mc.get_current_state();
            }
        }
    }

    motor.enable_modbus_communication().await?;
    motor.homing().await?;

    let current_position = motor.read_position().await.unwrap_or(0.0);
    if let Some(mc) = app_context.motor_controller.lock().await.as_mut() {
        mc.set_motor_connected(true);
        let _ = mc.sync_to_position(motor.pos_min(), motor.pos_max(), current_position);
    }
    let _ = motor.set_max_power(0.6).await;
    let _ = motor.set_acceleration(4000.0).await;
    let _ = motor.set_position_ring_ratio(3000.0).await;
    let _ = motor.set_speed_ring_ratio(3000.0).await;

    let mut last_stats_log = Instant::now();
    let mut pending_stats: Option<LoopStats> = None;

    loop {
        let log_stats = Instant::now().duration_since(last_stats_log) >= Duration::from_secs(5);

        let modbus_stats = motor.get_stats();
        let target = {
            let mut mc_lock = app_context.motor_controller.lock().await;
            if let Some(mc) = mc_lock.as_mut() {
                mc.modbus_stats = modbus_stats.clone();
                mc.motor_connected = modbus_stats.successful_requests > 0;
                let result = mc.compute_cycle();
                if log_stats {
                    pending_stats = Some(mc.last_loop_stats);
                }
                Some(result)
            } else {
                None
            }
        };

        if let Some(st) = pending_stats.take() {
            last_stats_log = Instant::now();
            if st.ups > 0 {
                log::info!(
                    "Motor loop stats (1s): UPS={}, dt(ms) min/avg/max/mdev = {:.2} / {:.2} / {:.2} / {:.2}",
                    st.ups,
                    st.min_dt_ms,
                    st.avg_dt_ms,
                    st.max_dt_ms,
                    st.mdev_dt_ms,
                );
            }
        }

        match target {
            Some((position, speed)) => {
                if let Err(e) = motor.write_position(position, speed).await {
                    log::warn!("Motor write error: {}, retrying...", e);
                    Timer::after_millis(10).await;
                    continue;
                }
                if let Err(e) = motor.cycle().await {
                    log::warn!("Motor cycle error: {}, retrying...", e);
                    Timer::after_millis(10).await;
                    continue;
                }
            }
            None => {
                log::error!("Motor controller lost, stopping motor loop");
                break;
            }
        }
    }

    Ok(())
}

#[cfg(feature = "esp32s3")]
pub fn run_motor_blocking(
    app_context: AppContext,
    uart_periph: esp_hal::peripherals::UART1<'static>,
    pin_config: PinConfiguration,
) {
    use static_cell::StaticCell;
    static CORE1_EXECUTOR: StaticCell<esp_rtos::embassy::Executor> = StaticCell::new();

    let executor = CORE1_EXECUTOR.init(esp_rtos::embassy::Executor::new());
    executor.run(|spawner| {
        spawner.spawn(core1_motor_task(app_context, uart_periph, pin_config).unwrap());
    });
}

#[cfg(feature = "esp32s3")]
#[embassy_executor::task]
async fn core1_motor_task(
    app_context: AppContext,
    uart_periph: esp_hal::peripherals::UART1<'static>,
    pin_config: PinConfiguration,
) {
    if let Err(e) = run_motor(app_context, uart_periph, pin_config).await {
        log::error!("Motor task on Core 1 failed: {}", e);
    }
}
