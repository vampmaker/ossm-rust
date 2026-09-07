use core::sync::atomic::Ordering;

use embassy_time::{with_timeout, Duration, Instant, Timer};
use esp_hal::delay::Delay;
use esp_hal::dma::{DmaRxBuf, DmaTxBuf};
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::uart::uhci::{Uhci, UhciRx, UhciTx};
use esp_hal::uart::{Config, Uart};
use fixedvec::FixedVec;
use portable_atomic::AtomicU16;
use rmodbus::{client::ModbusRequest, guess_response_frame_len, ModbusProto};

use crate::context::AppContext;
use crate::error::{FirmwareError, Result};
use crate::modbus_rtu::{
    apply_capture_inject, classify_modbus_rx_miss, classify_recovered, find_modbus_response,
    get_inject_junk, InjectJunkMode,
};
use crate::motion::{CommandConsumer, LinkStats, LoopStats};
use crate::motor::Motor;
use crate::peripheral_bank::PeripheralBank;
use crate::storage::PinConfiguration;
use crate::uart_owner;
use ossm_common::{LinkStatsWindow, LoopStatsWindow};
use ossm_core::modbus::aim30;
use ossm_core::{Command, Engine, Micros, StateResponse, StreamStatus};

pub const TARGET_BAUD_RATE: u32 = 115200;
/// FC16/FC06 ACK length; production UHCI pkt_thres is set once to this.
const FC16_ACK_LEN: u16 = 8;

fn now_us() -> Micros {
    Instant::now().as_micros()
}

#[derive(Clone, Copy)]
struct PublishedPose {
    version: u32,
    position: f32,
    speed: f32,
    y: f32,
    shaped_y: f32,
    t: f32,
    x: f32,
    motor_connected: bool,
    pos_min: f32,
    pos_max: f32,
    stream: StreamStatus,
}

impl PublishedPose {
    fn from_snapshot(snap: &StateResponse) -> Self {
        Self {
            version: snap.config.version,
            position: snap.position,
            speed: snap.speed,
            y: snap.y,
            shaped_y: snap.shaped_y,
            t: snap.t,
            x: snap.x,
            motor_connected: snap.motor_connected,
            pos_min: snap.pos_min,
            pos_max: snap.pos_max,
            stream: snap.stream,
        }
    }

    fn changed(&self, snap: &StateResponse) -> bool {
        self.version != snap.config.version
            || self.position != snap.position
            || self.speed != snap.speed
            || self.y != snap.y
            || self.shaped_y != snap.shaped_y
            || self.t != snap.t
            || self.x != snap.x
            || self.motor_connected != snap.motor_connected
            || self.pos_min != snap.pos_min
            || self.pos_max != snap.pos_max
            || self.stream != snap.stream
    }
}

fn drain_motion(engine: &mut Engine, consumer: &mut CommandConsumer) {
    while let Some(cmd) = consumer.dequeue() {
        engine.apply(Command::from(cmd));
    }
}

static LAST_INJECT_NOTE: AtomicU16 = AtomicU16::new(u16::MAX);

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameTiming {
    pub round_trip_us: u16,
    pub slave_latency_us: u16,
    pub rx_duration_us: u16,
}

#[allow(dead_code)]
pub struct ModbusRTUMaster<'d> {
    uhci_rx: Option<UhciRx<'d, esp_hal::Async>>,
    uhci_tx: Option<UhciTx<'d, esp_hal::Async>>,
    dma_rx: Option<DmaRxBuf>,
    dma_tx: Option<DmaTxBuf>,
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
    timeout_symbols: u8,
    debug: bool,

    link_window: LinkStatsWindow,
}

impl<'d> ModbusRTUMaster<'d> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        uhci_rx: UhciRx<'d, esp_hal::Async>,
        uhci_tx: UhciTx<'d, esp_hal::Async>,
        dma_rx: DmaRxBuf,
        dma_tx: DmaTxBuf,
        de_re: Option<Output<'d>>,
        device_id: u8,
        timeout_override_ms: u32,
        rx_timeout_override_us: u32,
        inter_frame_delay_override_us: u32,
        baudrate: u32,
        timeout_symbols: u8,
        debug: bool,
    ) -> Self {
        let timeout = if debug {
            Duration::from_millis(5)
        } else {
            Self::compute_timeout(baudrate, timeout_override_ms).unwrap()
        };
        let rx_inter_byte_timeout =
            Self::compute_rx_inter_byte_timeout(baudrate, rx_timeout_override_us).unwrap();
        let inter_frame_delay =
            Self::compute_inter_frame_delay(baudrate, inter_frame_delay_override_us).unwrap();
        Self {
            uhci_rx: Some(uhci_rx),
            uhci_tx: Some(uhci_tx),
            dma_rx: Some(dma_rx),
            dma_tx: Some(dma_tx),
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
            timeout_symbols,
            debug,
            link_window: {
                let mut w = LinkStatsWindow::new();
                w.set_link_info(ossm_common::LinkTransport::Uart);
                w.set_connected(true);
                w
            },
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

    /// Default software inter-byte timeout for the `with_timeout()` guard in `uart_read_exactly`.
    ///
    /// This is the SOFTWARE deadline for each individual `read_async` call after the first byte
    /// has been seen.
    ///
    /// ### GDMA UHCI RX & Hardware Idle Timeout (`timeout_symbols`)
    ///
    /// With GDMA UHCI RX enabled (`UhciRx::read(dma_rx)`), the hardware DMA controller captures
    /// incoming bytes continuously into a 256-byte DMA buffer. Because Modbus RTU response frames
    /// (7–9 bytes) do not fill the DMA buffer, the DMA transfer completes when the UART hardware
    /// triggers an `RX_TOUT` (RX FIFO idle timeout) interrupt after observing line silence of
    /// `timeout_symbols` symbol times after the last received byte.
    ///
    /// At 115200 baud (1 symbol ≈ 86.8 µs), `timeout_symbols` is raised so hardware
    /// idle covers $t_{3.5}$ (350 µs → 5 symbols). That drops the software busy-wait
    /// IFD on the C6 interrupt executor. `UhciRx::read` already returns after that
    /// silence, so remaining IFD is 0.
    fn get_default_rx_inter_byte_timeout(baudrate: u32) -> Result<Duration> {
        // Modbus RTU spec (>19200 bps): fixed t1.5 = 750µs.
        // Hardware idle covers t3.5; software 750µs is the per-read guard only.
        match baudrate {
            9600 => Ok(Duration::from_micros(4000)),
            19200 => Ok(Duration::from_micros(2500)),
            38400 => Ok(Duration::from_micros(1750)),
            115200 | 115201 => Ok(Duration::from_micros(750)),
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

    fn record_sample(&mut self, now: Micros, timing: FrameTiming, tx_len: u32, rx_len: u32) {
        self.link_window.record_success(
            now,
            timing.round_trip_us,
            Some(timing.slave_latency_us),
            Some(timing.rx_duration_us),
            tx_len,
            rx_len,
        );
    }

    fn record_failure(&mut self, now: Micros) {
        self.link_window.record_failure(now);
    }

    pub fn take_link_stats(&mut self) -> Option<LinkStats> {
        self.link_window.take_flushed()
    }

    pub fn link_has_exchanges(&self) -> bool {
        self.link_window.total_exchanges() > 0
    }

    async fn dma_write_all(&mut self, req: &[u8]) -> Result<()> {
        let uhci_tx = self
            .uhci_tx
            .take()
            .ok_or(FirmwareError::Uart("uhci tx taken"))?;
        let mut dma_tx = self
            .dma_tx
            .take()
            .ok_or(FirmwareError::Uart("dma tx taken"))?;

        if req.len() > dma_tx.as_mut_slice().len() {
            self.uhci_tx = Some(uhci_tx);
            self.dma_tx = Some(dma_tx);
            return Err(FirmwareError::Uart("req too large"));
        }
        dma_tx.as_mut_slice()[..req.len()].copy_from_slice(req);
        dma_tx.set_length(req.len());

        let mut tx_transfer = uhci_tx.write(dma_tx).map_err(|(_, tx, buf)| {
            self.uhci_tx = Some(tx);
            self.dma_tx = Some(buf);
            FirmwareError::Uart("dma tx start")
        })?;
        tx_transfer.wait_for_done().await;
        let (tx_res, mut uhci_tx, dma_tx) = tx_transfer.wait();
        let flush_res = uhci_tx.uart_tx.flush_async().await;
        self.uhci_tx = Some(uhci_tx);
        self.dma_tx = Some(dma_tx);
        tx_res.map_err(|_| FirmwareError::Uart("dma tx err"))?;
        flush_res.map_err(|_| FirmwareError::Uart("dma tx flush"))?;
        Ok(())
    }

    async fn modbus_request_inner(
        &mut self,
        req: &[u8],
        resp: &mut [u8],
    ) -> Result<(usize, FrameTiming, Micros)> {
        let t0 = Instant::now();
        let deadline = t0 + self.read_timeout;

        if let Some(ref mut pin) = self.de_re {
            pin.set_high();
            Delay::new().delay_micros(2);
        }

        self.dma_write_all(req).await?;

        if let Some(ref mut pin) = self.de_re {
            pin.set_low();
        }
        let t2 = Instant::now();

        let uhci_rx = self
            .uhci_rx
            .take()
            .ok_or(FirmwareError::Uart("uhci rx taken"))?;
        let mut dma_rx = self
            .dma_rx
            .take()
            .ok_or(FirmwareError::Uart("dma rx taken"))?;
        let cap = dma_rx.capacity();
        dma_rx.set_length(cap);
        let expected_len = expected_modbus_response_len(req);
        // pkt_thres is set at UART init (FC16 ACK = 8) or around FC03 reads.
        // Do not rewrite it every exchange.

        let mut rx_transfer = uhci_rx.read(dma_rx).map_err(|(_, rx, buf)| {
            self.uhci_rx = Some(rx);
            self.dma_rx = Some(buf);
            FirmwareError::Uart("dma rx start")
        })?;

        let remaining = deadline.saturating_duration_since(Instant::now());
        let wait_res = with_timeout(remaining, rx_transfer.wait_for_done()).await;
        let _t3 = Instant::now();

        let mut capture = [0u8; 256];
        let timed_out = wait_res.is_err();
        let to_copy = if timed_out {
            let (uhci_rx, dma_rx) = rx_transfer.cancel();
            self.uhci_rx = Some(uhci_rx);
            if !self.debug {
                self.dma_rx = Some(dma_rx);
                return Err(FirmwareError::Uart("read timeout"));
            }
            // Best-effort: may still be 0 if descriptors were never EOF-finalized.
            let n = dma_rx.read_received_data(&mut capture);
            self.dma_rx = Some(dma_rx);
            n
        } else {
            let (rx_res, uhci_rx, dma_rx) = rx_transfer.wait();
            self.uhci_rx = Some(uhci_rx);
            if rx_res.is_err() {
                self.dma_rx = Some(dma_rx);
                if self.debug {
                    self.log_modbus_dbg(req, &[], expected_len as usize, false, "dma_err", 0, 0);
                }
                return Err(FirmwareError::Uart("dma rx err"));
            }
            let n = if self.debug {
                dma_rx.read_received_data(&mut capture)
            } else {
                dma_rx.read_received_data(resp)
            };
            self.dma_rx = Some(dma_rx);
            n
        };

        if self.debug && timed_out {
            log::info!(
                "MODBUS_DBG note=rx_wait_timeout_ms={} bytes_after_cancel={}",
                self.read_timeout.as_millis(),
                to_copy
            );
        }

        let (rx_bytes, rx_len) = if self.debug {
            let mut len = to_copy.min(capture.len());
            let (inject_mode, inject_n) = get_inject_junk();
            if inject_mode != InjectJunkMode::Off && inject_n > 0 {
                let key = ((inject_mode as u16) << 8) | u16::from(inject_n);
                if LAST_INJECT_NOTE.swap(key, Ordering::Relaxed) != key {
                    log::info!(
                        "MODBUS_DBG note=inject mode={} n={}",
                        inject_mode.as_str(),
                        inject_n
                    );
                }
                len = apply_capture_inject(&mut capture, len);
            }
            let slave = req.first().copied().unwrap_or(1);
            let expected = expected_len as usize;
            if let Some((offset, frame_len)) =
                find_modbus_response(&capture[..len], slave, expected)
            {
                let class = classify_recovered(len, offset, frame_len);
                let log_dbg = inject_mode != InjectJunkMode::Off
                    || class != "exact"
                    || offset > 0
                    || len > frame_len;
                if log_dbg {
                    self.log_modbus_dbg(
                        req,
                        &capture[..len],
                        expected,
                        true,
                        class,
                        offset,
                        frame_len,
                    );
                }
                resp[..frame_len].copy_from_slice(&capture[offset..offset + frame_len]);
                (frame_len.min(3), frame_len)
            } else {
                let class = classify_modbus_rx_miss(&capture[..len], req, expected);
                self.log_modbus_dbg(req, &capture[..len], expected, false, class, 0, 0);
                return Err(FirmwareError::Modbus(match class {
                    "empty" => "empty frame",
                    "short" => "incomplete frame",
                    "leading_zero" | "leading_junk" => "bad frame",
                    _ => "parse fail",
                }));
            }
        } else {
            if to_copy < 4 {
                return Err(FirmwareError::Modbus("bad frame"));
            }
            (to_copy, to_copy)
        };

        let len = guess_response_frame_len(&resp[..rx_bytes.min(3)], ModbusProto::Rtu)
            .map_err(|_| FirmwareError::Modbus("bad frame"))? as usize;
        if rx_len < len {
            return Err(FirmwareError::Modbus("incomplete frame"));
        }

        let t5 = Instant::now();
        #[cfg(feature = "byte-timing-diag")]
        if self.current_successes % 200 == 1 {
            log::info!(
                "TIME_BREAKDOWN: tx={}us rx_wait={}us rx_post={}us",
                t2.duration_since(t0).as_micros(),
                _t3.duration_since(t2).as_micros(),
                t5.duration_since(_t3).as_micros()
            );
        }

        // When UHCI DMA read completes, the UART hardware has already observed line silence of
        // `timeout_symbols` symbol times after the last response byte arrived on the wire.
        let hardware_silence_us =
            ((self.timeout_symbols as u64 * 10_000_000) / self.baudrate as u64) as u32;

        let remaining_ifd_us = self
            .inter_frame_delay
            .as_micros()
            .saturating_sub(hardware_silence_us as u64);
        // Default symbols cover t3.5, so this is 0. Busy-wait only if the user
        // shortened hardware idle below the IFD override.
        if remaining_ifd_us > 0 {
            Delay::new().delay_micros(remaining_ifd_us as u32);
        }

        let round_trip_us = t5
            .duration_since(t0)
            .as_micros()
            .saturating_sub(hardware_silence_us as u64)
            .min(u16::MAX as u64) as u16;
        let slave_latency_us = t5
            .duration_since(t2)
            .as_micros()
            .saturating_sub(hardware_silence_us as u64)
            .min(u16::MAX as u64) as u16;
        let rx_duration_us = 0;

        #[cfg(feature = "byte-timing-diag")]
        log::info!(
            "GDMA_RX_DIAG: bytes_received={} round_trip_us={} slave_latency_us={}",
            rx_len,
            round_trip_us,
            slave_latency_us,
        );

        let timing = FrameTiming {
            round_trip_us,
            slave_latency_us,
            rx_duration_us,
        };

        Ok((len, timing, t5.as_micros()))
    }

    #[allow(clippy::too_many_arguments)]
    fn log_modbus_dbg(
        &self,
        tx: &[u8],
        rx: &[u8],
        expected: usize,
        ok: bool,
        class: &str,
        skip: usize,
        frame_len: usize,
    ) {
        let tx_b64 = bytes_to_base64(tx, 64);
        let rx_b64 = bytes_to_base64(rx, 64);
        let trim = rx.len().saturating_sub(skip + frame_len);
        log::info!(
            "MODBUS_DBG ok={} class={} expected={} skip={} frame_len={} trim={} tx={} rx_len={} rx={}",
            ok,
            class,
            expected,
            skip,
            frame_len,
            trim,
            tx_b64,
            rx.len(),
            rx_b64
        );
    }

    pub async fn modbus_request(&mut self, req: &[u8], resp: &mut [u8]) -> Result<usize> {
        match self.modbus_request_inner(req, resp).await {
            Ok((len, timing, now)) => {
                self.record_sample(now, timing, req.len() as u32, len as u32);
                Ok(len)
            }
            Err(e) => {
                self.record_failure(now_us());
                Err(e)
            }
        }
    }

    /// Relay path: raise UHCI `pkt_thres` to the expected FC03/FC16 length for this frame.
    pub async fn relay_request(&mut self, req: &[u8], resp: &mut [u8]) -> Result<usize> {
        if !self.debug {
            configure_uhci0_pkt_thres(expected_modbus_response_len(req));
        }
        let result = self.modbus_request(req, resp).await;
        if !self.debug {
            configure_uhci0_pkt_thres(FC16_ACK_LEN);
        }
        result
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
        if !self.debug {
            configure_uhci0_pkt_thres(expected_modbus_response_len(frame_buf.as_slice()));
        }
        let req_res = self
            .modbus_request(frame_buf.as_slice(), &mut response_buf)
            .await;
        if !self.debug {
            configure_uhci0_pkt_thres(FC16_ACK_LEN);
        }
        let len = req_res?;

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

    #[allow(dead_code)]
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
        self.baudrate = baudrate;
        let timeout = if self.debug {
            Duration::from_millis(5)
        } else {
            Self::compute_timeout(baudrate, self.timeout_override_ms)?
        };
        self.read_timeout = timeout;
        self.write_timeout = timeout;
        self.inter_frame_delay =
            Self::compute_inter_frame_delay(baudrate, self.inter_frame_delay_override_us)?;
        if self.rx_timeout_override_us == 0 {
            self.timeout_symbols =
                timeout_symbols_covering_ifd(baudrate, self.inter_frame_delay.as_micros());
        }
        let rx_config = esp_hal::uart::RxConfig::default().with_timeout(self.timeout_symbols);
        let config = Config::default().with_baudrate(baudrate).with_rx(rx_config);
        if let Some(ref mut tx) = self.uhci_tx {
            tx.uart_tx
                .apply_config(&config)
                .map_err(|_| FirmwareError::Uart("baud tx"))?;
        }
        if let Some(ref mut rx) = self.uhci_rx {
            rx.uart_rx
                .apply_config(&config)
                .map_err(|_| FirmwareError::Uart("baud rx"))?;
        }
        configure_uart1_rx_idle_threshold(self.timeout_symbols);
        #[cfg(feature = "esp32c6")]
        unsafe {
            let u1 = &*esp_hal::peripherals::UART1::ptr();
            log::info!(
                "SET_BAUD_REGS: rx_idle_thrhd={} rx_tout_thrhd={} rx_tout_en={}",
                u1.idle_conf().read().rx_idle_thrhd().bits(),
                u1.tout_conf().read().rx_tout_thrhd().bits(),
                u1.tout_conf().read().rx_tout_en().bit(),
            );
        }
        Ok(())
    }

    pub async fn flush_async(&mut self) -> Result<()> {
        if let Some(ref mut tx) = self.uhci_tx {
            let _ = tx.uart_tx.flush_async().await;
        }
        Ok(())
    }

    pub fn check_for_rx_errors(&mut self) {
        if let Some(ref mut rx) = self.uhci_rx {
            let _ = rx.uart_rx.check_for_errors();
        }
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

    pub fn take_link_stats(&mut self) -> Option<LinkStats> {
        self.client.take_link_stats()
    }

    pub fn link_has_exchanges(&self) -> bool {
        self.client.link_has_exchanges()
    }

    async fn write_position_raw(&mut self, position: i32) -> Result<()> {
        let req = aim30::pack_position_fc16(self.client.device_id, position);
        let mut resp = [0u8; FC16_ACK_LEN as usize];
        let len = self.client.modbus_request(&req, &mut resp).await?;
        let frame = &resp[..len.min(resp.len())];
        if len < 8 || frame[1] != 0x10 || !crate::modbus_rtu::verify_rtu_crc(frame) {
            return Err(FirmwareError::Modbus("parse"));
        }
        Ok(())
    }

    async fn wait_stable_position(&mut self, timeout_ms: u32) -> Result<f32> {
        let start_time = Instant::now();
        let timeout = Duration::from_millis(timeout_ms as u64);
        let mut position = self.read_position().await?;
        Timer::after_millis(100).await;
        while start_time.elapsed() < timeout {
            if uart_owner::stop_requested() {
                return Err(FirmwareError::Modbus("stopped"));
            }
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
            let _ = self.client.flush_async().await;
            self.client.set_baudrate(baud_rate)?;
            Timer::after_micros(5000).await; // ensure motor modbus rx state machine is in a clean state before proceeding: wait for t3.5 for 9600

            // reset the uart into a clean state
            let _ = self.client.flush_async().await;
            self.client.check_for_rx_errors();

            let delay_us = Self::modbus_t3_5_us(baud_rate).max(self.scan_delay_us);
            for device_id in 1..=247u8 {
                if uart_owner::stop_requested() {
                    return Err(FirmwareError::Modbus("stopped"));
                }
                // full address space for modbus device id (0 is for broadcast, 248-255 are reserved)
                self.client.device_id = device_id;
                Timer::after_micros(delay_us as u64).await;
                if self
                    .client
                    .write_holding_register(aim30::REG_ENABLE, aim30::ENABLE_MODBUS)
                    .await
                    .is_ok()
                {
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
        self.client
            .write_holding_register(aim30::REG_ENABLE, aim30::ENABLE_MODBUS)
            .await?;
        self.client
            .write_holding_register(0x03, baud_rate_code)
            .await?;
        self.client.write_holding_register(0x04, 129).await?;
        self.client.write_holding_register(0x00, 506).await?;
        Ok(())
    }

    pub async fn enable_modbus_communication(&mut self) -> Result<()> {
        self.client
            .write_holding_register(aim30::REG_ENABLE, aim30::ENABLE_MODBUS)
            .await
    }
}

impl<'d> Motor for Modbus57AIM30Motor<'d> {
    async fn read_position(&mut self) -> Result<f32> {
        let mut rsp = [0u16; 2];
        self.client
            .read_holding_registers(aim30::REG_POSITION, 2, &mut rsp)
            .await?;
        let position = aim30::unpack_position_i32(rsp);
        Ok(aim30::counts_to_radians(position))
    }

    async fn write_position(&mut self, position: f32, _speed: f32) -> Result<()> {
        let position_i32 = aim30::write_counts_for_radians(position);
        self.write_position_raw(position_i32).await
    }

    async fn set_max_power(&mut self, power: f32) -> Result<()> {
        self.client
            .write_holding_register(aim30::REG_POWER, aim30::encode_max_power(power))
            .await
    }

    async fn set_acceleration(&mut self, acceleration: f32) -> Result<()> {
        self.client
            .write_holding_register(aim30::REG_ACCEL, aim30::encode_acceleration(acceleration))
            .await
    }

    async fn set_position_ring_ratio(&mut self, ratio: f32) -> Result<()> {
        self.client
            .write_holding_register(aim30::REG_POS_RING, ratio as u16)
            .await
    }

    async fn set_speed_ring_ratio(&mut self, ratio: f32) -> Result<()> {
        self.client
            .write_holding_register(aim30::REG_SPEED_RING, ratio as u16)
            .await
    }

    async fn homing(&mut self) -> Result<()> {
        assert!(
            self.pos_min == 0.0 && self.pos_max == 0.0,
            "Motor already homed"
        );

        log::info!("Homing motor");
        let want = aim30::encode_max_power(aim30::POWER_HOMING);
        let mut power_ok = false;
        for attempt in 1..=8 {
            if uart_owner::stop_requested() {
                return Err(FirmwareError::Modbus("stopped"));
            }
            match self.set_max_power(aim30::POWER_HOMING).await {
                Ok(()) => {
                    power_ok = true;
                    break;
                }
                Err(e) => {
                    log::warn!("set_max_power POWER_HOMING ({attempt}/8): {e}");
                    if uart_owner::sleep_or_stop(200).await {
                        return Err(FirmwareError::Modbus("stopped"));
                    }
                }
            }
        }
        if !power_ok {
            return Err(FirmwareError::Modbus("POWER_HOMING failed after retries"));
        }
        let mut got = [0u16; 1];
        self.client
            .read_holding_registers(aim30::REG_POWER, 1, &mut got)
            .await?;
        if got[0] != want {
            log::error!("REG_POWER readback {} want {want} (POWER_HOMING)", got[0]);
            return Err(FirmwareError::Modbus("POWER_HOMING readback mismatch"));
        }
        log::info!("REG_POWER={want} after POWER_HOMING");

        self.set_acceleration(aim30::ACCEL_HOMING).await?;
        self.reset_position().await?;
        log::info!("Writing position to -100.0");
        self.write_position(-100.0, 0.0).await?;
        if uart_owner::sleep_or_stop(5000).await {
            return Err(FirmwareError::Modbus("stopped"));
        }
        self.pos_min = self.wait_stable_position(5000).await? + 0.1;
        log::info!("pos_min: {}", self.pos_min);

        log::info!("Writing position to 100.0");
        self.write_position(100.0, 0.0).await?;
        if uart_owner::sleep_or_stop(5000).await {
            return Err(FirmwareError::Modbus("stopped"));
        }
        self.pos_max = self.wait_stable_position(5000).await? - 0.1;
        log::info!("pos_max: {}", self.pos_max);

        self.write_position((self.pos_min + self.pos_max) / 2.0, 0.0)
            .await?;
        if uart_owner::sleep_or_stop(5000).await {
            return Err(FirmwareError::Modbus("stopped"));
        }
        self.wait_stable_position(5000).await?;
        Ok(())
    }

    fn pos_min(&self) -> f32 {
        self.pos_min
    }

    fn pos_max(&self) -> f32 {
        self.pos_max
    }
}

#[derive(Debug)]
pub struct ModbusScanResult {
    pub baud_rate: u32,
    pub device_id: u8,
}

pub(crate) fn init_uart_and_modbus(
    pin_config: &PinConfiguration,
    bank: &mut PeripheralBank,
) -> Result<ModbusRTUMaster<'static>> {
    let (uart_periph, uhci_periph, dma_channel) = bank.take_uart()?;
    let tx_pin = bank.take_pin(pin_config.modbus_tx as u8)?;
    let rx_pin = Input::new(
        bank.take_pin(pin_config.modbus_rx as u8)?,
        InputConfig::default().with_pull(Pull::Up),
    );
    let de_pin = bank.take_pin(pin_config.modbus_de_re as u8)?;

    let de_re = Output::new(de_pin, Level::Low, OutputConfig::default());
    let baud = if pin_config.modbus_baud == 0 {
        TARGET_BAUD_RATE
    } else {
        pin_config.modbus_baud
    };
    let timeout_symbols = if pin_config.modbus_rx_timeout_us == 0 {
        let ifd_us = if pin_config.modbus_inter_frame_delay_us == 0 {
            default_ifd_us(baud)
        } else {
            u64::from(pin_config.modbus_inter_frame_delay_us)
        };
        timeout_symbols_covering_ifd(baud, ifd_us)
    } else {
        (pin_config.modbus_rx_timeout_us / 87).clamp(2, 127) as u8
    };
    let rx_config = esp_hal::uart::RxConfig::default().with_timeout(timeout_symbols);
    let uart_config = Config::default().with_baudrate(baud).with_rx(rx_config);
    let uart = Uart::new(uart_periph, uart_config)
        .map_err(|_| FirmwareError::Uart("init"))?
        .with_tx(tx_pin)
        .with_rx(rx_pin);

    // Keep RX DMA at 256 B: larger UHCI RX buffers have been observed to prevent
    // descriptor EOF finalization on this path (all reads look empty / timeout).
    let (rx_buffer, rx_descriptors, tx_buffer, tx_descriptors) = esp_hal::dma_buffers!(256, 256);
    let dma_rx =
        DmaRxBuf::new(rx_descriptors, rx_buffer).map_err(|_| FirmwareError::Uart("dma rx buf"))?;
    let dma_tx =
        DmaTxBuf::new(tx_descriptors, tx_buffer).map_err(|_| FirmwareError::Uart("dma tx buf"))?;

    let mut uhci = Uhci::new(uart, uhci_periph, dma_channel);
    uhci.apply_rx_config(&esp_hal::uart::uhci::RxConfig::default())
        .map_err(|_| FirmwareError::Uart("uhci rx cfg"))?;
    uhci.apply_tx_config(&esp_hal::uart::uhci::TxConfig::default())
        .map_err(|_| FirmwareError::Uart("uhci tx cfg"))?;

    let (uhci_rx, uhci_tx) = uhci.into_async().split();

    configure_uart1_rx_idle_threshold(timeout_symbols);
    configure_uhci0_pkt_thres(if pin_config.modbus_debug {
        256
    } else {
        FC16_ACK_LEN
    });

    if pin_config.modbus_debug {
        log::info!("Modbus debug mode ON: 5ms RX deadline, base64 dumps on console (DMA RX 256 B)");
    }

    Ok(ModbusRTUMaster::new(
        uhci_rx,
        uhci_tx,
        dma_rx,
        dma_tx,
        Some(de_re),
        1,
        pin_config.modbus_timeout_ms,
        pin_config.modbus_rx_timeout_us,
        pin_config.modbus_inter_frame_delay_us,
        baud,
        timeout_symbols,
        pin_config.modbus_debug,
    ))
}

fn configure_uart1_rx_idle_threshold(symbols: u8) {
    let bits = (symbols as u16).clamp(2, 1023);
    unsafe {
        let regs = &*esp_hal::peripherals::UART1::ptr();
        regs.idle_conf().modify(|_, w| w.rx_idle_thrhd().bits(bits));
        #[cfg(feature = "esp32c6")]
        regs.tout_conf().modify(|_, w| w.rx_tout_thrhd().bits(bits));
    }
}

fn configure_uhci0_pkt_thres(len: u16) {
    unsafe {
        let u0 = &*esp_hal::peripherals::UHCI0::ptr();
        u0.pkt_thres().modify(|_, w| w.pkt_thrs().bits(len));
    }
}

/// Symbol count so UART idle-EOF silence ≥ `ifd_us` (covers software t3.5).
fn timeout_symbols_covering_ifd(baudrate: u32, ifd_us: u64) -> u8 {
    let symbols = ifd_us.saturating_mul(baudrate as u64).div_ceil(10_000_000);
    symbols.clamp(2, 127) as u8
}

fn default_ifd_us(baudrate: u32) -> u64 {
    match baudrate {
        9600 => 4000,
        19200 => 2000,
        38400 => 1000,
        _ => 350,
    }
}

fn expected_modbus_response_len(req: &[u8]) -> u16 {
    if req.len() >= 6 {
        match req[1] {
            0x03 | 0x04 => {
                let qty = ((req[4] as u16) << 8) | (req[5] as u16);
                5 + qty * 2
            }
            0x06 | 0x10 => 8,
            _ => 128,
        }
    } else {
        128
    }
}

fn bytes_to_base64(data: &[u8], max_bytes: usize) -> alloc::string::String {
    const CHARSET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let n = data.len().min(max_bytes);
    let mut s = alloc::string::String::with_capacity(n.div_ceil(3) * 4);
    let mut i = 0;
    while i < n {
        let b0 = data[i];
        let b1 = if i + 1 < n { data[i + 1] } else { 0 };
        let b2 = if i + 2 < n { data[i + 2] } else { 0 };

        s.push(CHARSET[(b0 >> 2) as usize] as char);
        s.push(CHARSET[((b0 & 3) << 4 | b1 >> 4) as usize] as char);
        if i + 1 < n {
            s.push(CHARSET[((b1 & 15) << 2 | b2 >> 6) as usize] as char);
        } else {
            s.push('=');
        }
        if i + 2 < n {
            s.push(CHARSET[(b2 & 63) as usize] as char);
        } else {
            s.push('=');
        }
        i += 3;
    }
    if data.len() > max_bytes {
        s.push_str(" …");
    }
    s
}

pub async fn run_motor(
    app_context: AppContext,
    motion_consumer: &mut crate::motion::CommandConsumer,
    engine: &mut Engine,
    mut motor: Modbus57AIM30Motor<'_>,
) -> Result<()> {
    log::info!("Using motor config from storage cache");

    log::info!("Waiting 3s for WiFi/BLE initialization to settle before scanning motor...");
    if uart_owner::sleep_or_stop(3000).await {
        return Ok(());
    }

    #[cfg(feature = "esp32c6")]
    unsafe {
        let u1 = &*esp_hal::peripherals::UART1::ptr();
        log::info!(
            "DIAG_REGS: rx_idle_thrhd={} rx_tout_thrhd={} rx_tout_en={}",
            u1.idle_conf().read().rx_idle_thrhd().bits(),
            u1.tout_conf().read().rx_tout_thrhd().bits(),
            u1.tout_conf().read().rx_tout_en().bit(),
        );
    }

    let mut modbus_ok = false;
    for init_attempt in 1..=2 {
        if uart_owner::stop_requested() {
            return Ok(());
        }
        if init_attempt > 1 {
            log::info!(
                "Retrying motor initialization (attempt {}/2)...",
                init_attempt
            );
            if uart_owner::sleep_or_stop(1000).await {
                return Ok(());
            }
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
                    if uart_owner::stop_requested() {
                        return Ok(());
                    }
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
            if uart_owner::stop_requested() {
                return Ok(());
            }
            Timer::after_millis(10).await;
            drain_motion(engine, motion_consumer);
            engine.apply(Command::SetMotorConnected(false));
            let _ = engine.tick(now_us());
            app_context.update_snapshot(engine.snapshot());
        }
    }

    motor.enable_modbus_communication().await?;
    match motor.homing().await {
        Ok(()) => {}
        Err(e) if uart_owner::stop_requested() => {
            log::info!("Homing interrupted by mode switch: {}", e);
            return Ok(());
        }
        Err(e) => return Err(e),
    }

    let current_position = motor.read_position().await.unwrap_or(0.0);
    engine.apply(Command::SetMotorConnected(true));
    engine.apply(Command::HomingComplete {
        pos_min: motor.pos_min(),
        pos_max: motor.pos_max(),
        position: current_position,
    });
    engine.flush_snapshot();
    engine.reset_cycle_clock(now_us());
    app_context.update_snapshot(engine.snapshot());

    let _ = motor.set_max_power(aim30::POWER_RUN).await;
    let _ = motor.set_acceleration(aim30::ACCEL_RUN).await;
    let _ = motor
        .set_position_ring_ratio(aim30::RING_RATIO_RUN as f32)
        .await;
    let _ = motor
        .set_speed_ring_ratio(aim30::RING_RATIO_RUN as f32)
        .await;

    let mut last_stats_log_us = now_us();
    let mut last_error_log = Instant::now() - Duration::from_secs(1);
    let mut pending_stats: Option<LoopStats> = None;
    let mut loop_window = LoopStatsWindow::new();
    let mut last_tick_us = now_us();
    loop_window.reset_clock(last_tick_us);
    let mut last_pub = PublishedPose {
        version: u32::MAX,
        position: f32::NAN,
        speed: f32::NAN,
        y: f32::NAN,
        shaped_y: f32::NAN,
        t: f32::NAN,
        x: f32::NAN,
        motor_connected: false,
        pos_min: f32::NAN,
        pos_max: f32::NAN,
        stream: StreamStatus::default(),
    };
    loop {
        if uart_owner::stop_requested() {
            engine.apply(Command::SetPaused {
                paused: true,
                position: None,
            });
            drain_motion(engine, motion_consumer);
            let now = now_us();
            let dt_ms = (now.saturating_sub(last_tick_us)) as f32 / 1000.0;
            loop_window.record(now, dt_ms, engine.snapshot().position);
            if let Some(stats) = loop_window.take_flushed() {
                engine.apply(Command::SetLoopStats(stats));
            }
            if let Some(stats) = motor.take_link_stats() {
                engine.apply(Command::SetLinkStats(stats));
            }
            let connected = motor.link_has_exchanges();
            if engine.snapshot().motor_connected != connected {
                engine.apply(Command::SetMotorConnected(connected));
            }
            let out = engine.tick(now);
            let _ = motor.write_position(out.position, out.speed).await;
            app_context.update_snapshot(engine.snapshot());
            log::info!("Motor loop stopped for UART1 role switch");
            return Ok(());
        }

        drain_motion(engine, motion_consumer);

        let now = now_us();
        let log_stats = now.saturating_sub(last_stats_log_us) >= 5_000_000;
        let dt_ms = (now.saturating_sub(last_tick_us)) as f32 / 1000.0;
        last_tick_us = now;
        loop_window.record(now, dt_ms, engine.snapshot().position);
        let mut stats_dirty = false;
        if let Some(stats) = loop_window.take_flushed() {
            engine.apply(Command::SetLoopStats(stats));
            stats_dirty = true;
        }
        if let Some(stats) = motor.take_link_stats() {
            engine.apply(Command::SetLinkStats(stats));
            stats_dirty = true;
        }
        let connected = motor.link_has_exchanges();
        if engine.snapshot().motor_connected != connected {
            engine.apply(Command::SetMotorConnected(connected));
        }
        let out = engine.tick(now);
        let position = out.position;
        let speed = out.speed;
        let snap = engine.snapshot();
        let stats_opt = if log_stats {
            Some(snap.loop_stats)
        } else {
            None
        };
        // Paused pose is static (t/x stay 0). Skip CS+copy_from so idle UPS
        // keeps the ~20 Hz headroom vs moving. Stats / config / pose still publish.
        if stats_dirty || last_pub.changed(snap) {
            app_context.update_snapshot(snap);
            last_pub = PublishedPose::from_snapshot(snap);
        }

        if let Some(st) = stats_opt {
            pending_stats = Some(st);
        }

        if let Some(st) = pending_stats.take() {
            last_stats_log_us = now;
            if st.ups > 0 {
                log::info!(
                    "Motor loop stats (1s): UPS={}, dt(ms) min/avg/max/mdev = {:.2} / {:.2} / {:.2} / {:.2}",
                    st.ups,
                    st.dt_min_ms,
                    st.dt_avg_ms,
                    st.dt_max_ms,
                    st.dt_mdev_ms,
                );
            }
        }

        if let Err(e) = motor.write_position(position, speed).await {
            if Instant::now().duration_since(last_error_log) >= Duration::from_secs(1) {
                last_error_log = Instant::now();
                log::warn!("Motor write error: {}, retrying...", e);
            }
            Timer::after_millis(10).await;
            continue;
        }
    }
}

#[cfg(feature = "esp32s3")]
fn core1_executor() -> &'static mut esp_rtos::embassy::Executor {
    use static_cell::StaticCell;
    static CORE1_EXECUTOR: StaticCell<esp_rtos::embassy::Executor> = StaticCell::new();
    CORE1_EXECUTOR.init(esp_rtos::embassy::Executor::new())
}

#[cfg(feature = "esp32s3")]
pub fn run_uart_owner_blocking(
    app_context: AppContext,
    motion_consumer: crate::motion::CommandConsumer,
    peripheral_bank: crate::peripheral_bank::PeripheralBank,
) {
    let executor = core1_executor();
    executor.run(|spawner| {
        spawner
            .spawn(core1_uart_owner_task(app_context, motion_consumer, peripheral_bank).unwrap());
    });
}

#[cfg(feature = "esp32s3")]
#[embassy_executor::task]
async fn core1_uart_owner_task(
    app_context: AppContext,
    motion_consumer: crate::motion::CommandConsumer,
    peripheral_bank: crate::peripheral_bank::PeripheralBank,
) {
    crate::uart_owner::run_uart_owner(app_context, motion_consumer, peripheral_bank).await;
}
