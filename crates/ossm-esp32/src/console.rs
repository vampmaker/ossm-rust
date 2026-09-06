//! Sole owner of USB Serial/JTAG + UART0 console I/O, plus RTT (TX only).
//!
//! Other tasks talk via bounded channels. USB TX is **commit-then-wait**: bytes
//! enter the IN FIFO only when `serial_in_ep_data_free` (via `write_byte_nb`)
//! and are consumed from the software ring immediately. Timed `flush()` waits
//! never rewrite committed bytes. No host → Stalled (skip log format, keep CLI
//! ring). USB RX is split from TX and armed even while draining. UART0/RTT use
//! nb + 100 µs yields. Per-sink TX rings drain without O(n) buffer shifts.
//! CLI and log USB bytes use separate TX rings so replies are not queued behind
//! log backlog on the wire. Do not disable USB-Serial-JTAG DTR/RTS chip reset
//! (required for flashing).

use core::cell::UnsafeCell;
use core::fmt::Write as FmtWrite;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, AtomicUsize, Ordering};

use embassy_futures::select::{select, Either};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};
use embedded_io_async::{Read as AsyncRead, Write as AsyncWrite};
use esp_hal::gpio::{AnyPin, Input, InputConfig, Output, OutputConfig, Pull};
use esp_hal::uart::{Config as UartConfig, Uart};
use esp_hal::usb_serial_jtag::{UsbSerialJtag, UsbSerialJtagRx, UsbSerialJtagTx};
use esp_hal::{Async, Blocking};
use heapless::Vec as HVec;
use rtt_target::{rtt_init, ChannelMode, UpChannel};

const OUT_CAP: usize = 192;
/// Drop-newest log byte ring (motor/WiFi/HTTP producers).
const LOG_RING_CAP: usize = 4096;
/// Priority OUT for CLI echo/replies (processed before log OUT).
const CLI_OUT_QUEUE: usize = 24;
const IN_QUEUE: usize = 512;
const SINK_TX_CAP: usize = 256;
const USB_EP_SIZE: usize = 64;
const MAX_YIELDS: u32 = 10;
/// Backoff when a sink cannot progress (UART/RTT or accept_out space).
const YIELD_US: u64 = 100;
/// Max wait for one USB EP IN packet ACK / RX wake select arm.
const USB_WAIT_US: u64 = 500;
/// No-host abandon budget for a single drain pass.
const TOTAL_TIMEOUT_MS: u64 = 10;
/// UART0 RX poll when idle (no pending OUT work).
const UART_POLL_US: u64 = 2000;
/// OUT chunks drained per main-loop iteration.
const TX_BATCH_MAX: usize = 3;
/// Max TX work per main-loop iteration before an RX turn.
const LOOP_TX_BUDGET_MS: u64 = 8;
/// Max time spent pushing one OUT chunk through sink rings during accept_out.
const ACCEPT_OUT_BUDGET_MS: u64 = 10;
/// USB EP writes per capped log drain; 4 × 64 B.
const USB_DRAIN_MAX_PACKETS: u32 = 4;
/// CLI OUT chunks per main-loop iteration (priority path).
const CLI_BATCH_MAX: usize = 4;
/// Extra yield when no OUT/CLI work pending (saves CPU for motor/WiFi).
const IDLE_YIELD_US: u64 = 500;
/// No-host USB IN probe cadence (not 500 µs — that starves the C6 motor ISR).
const STALL_PROBE_US: u64 = 2000;
const LOG_LINE_CAP: usize = 256;
const CONSOLE_BANNER: &[u8] = b"\r\n[console] ready\r\n";

pub type OutChunk = HVec<u8, OUT_CAP>;

static CLI_OUT_CH: Channel<CriticalSectionRawMutex, OutChunk, CLI_OUT_QUEUE> = Channel::new();
static IN_CH: Channel<CriticalSectionRawMutex, u8, IN_QUEUE> = Channel::new();
/// False while USB IN is stalled: ChannelLogger returns before formatting.
static LOG_LIVE: AtomicBool = AtomicBool::new(true);
const USB_MUX_CLI: u8 = 0;
const USB_MUX_PIPE: u8 = 1;
static USB_MUX: AtomicU8 = AtomicU8::new(USB_MUX_CLI);
static CDC_BAUD: AtomicU32 = AtomicU32::new(0);
const PIPE_CAP: usize = 512;

struct PipeRing {
    buf: UnsafeCell<[u8; PIPE_CAP]>,
    head: AtomicUsize,
    len: AtomicUsize,
}

unsafe impl Sync for PipeRing {}

static PIPE_HOST: PipeRing = PipeRing {
    buf: UnsafeCell::new([0; PIPE_CAP]),
    head: AtomicUsize::new(0),
    len: AtomicUsize::new(0),
};
static PIPE_BUS: PipeRing = PipeRing {
    buf: UnsafeCell::new([0; PIPE_CAP]),
    head: AtomicUsize::new(0),
    len: AtomicUsize::new(0),
};
static HOST_PIPE_SIG: Signal<CriticalSectionRawMutex, ()> = Signal::new();

fn pipe_push(ring: &PipeRing, data: &[u8]) {
    if data.is_empty() {
        return;
    }
    critical_section::with(|_| {
        let len = ring.len.load(Ordering::Relaxed);
        let n = data.len().min(PIPE_CAP.saturating_sub(len));
        if n == 0 {
            return;
        }
        let head = ring.head.load(Ordering::Relaxed);
        let buf = unsafe { &mut *ring.buf.get() };
        for (i, &b) in data.iter().take(n).enumerate() {
            buf[(head + len + i) % PIPE_CAP] = b;
        }
        ring.len.store(len + n, Ordering::Relaxed);
    });
}

fn pipe_pop(ring: &PipeRing, dst: &mut [u8]) -> usize {
    critical_section::with(|_| {
        let len = ring.len.load(Ordering::Relaxed);
        let n = dst.len().min(len);
        if n == 0 {
            return 0;
        }
        let head = ring.head.load(Ordering::Relaxed);
        let buf = unsafe { &*ring.buf.get() };
        for (i, slot) in dst.iter_mut().take(n).enumerate() {
            *slot = buf[(head + i) % PIPE_CAP];
        }
        ring.head.store((head + n) % PIPE_CAP, Ordering::Relaxed);
        ring.len.store(len - n, Ordering::Relaxed);
        n
    })
}

fn pipe_clear(ring: &PipeRing) {
    critical_section::with(|_| {
        ring.head.store(0, Ordering::Relaxed);
        ring.len.store(0, Ordering::Relaxed);
    });
}

pub fn usb_mux_is_pipe() -> bool {
    USB_MUX.load(Ordering::Relaxed) == USB_MUX_PIPE
}

pub fn set_usb_mux_pipe(pipe: bool) {
    if pipe {
        USB_MUX.store(USB_MUX_PIPE, Ordering::Release);
        pipe_clear(&PIPE_HOST);
        pipe_clear(&PIPE_BUS);
        while IN_CH.try_receive().is_ok() {}
        crate::rs485::reset_host_magic_matcher();
    } else {
        USB_MUX.store(USB_MUX_CLI, Ordering::Release);
        pipe_clear(&PIPE_HOST);
        pipe_clear(&PIPE_BUS);
    }
}

pub fn pipe_push_host(data: &[u8]) {
    if data.is_empty() {
        return;
    }
    let mut filtered = [0u8; 80];
    let mut off = 0usize;
    while off < data.len() {
        let chunk = &data[off..data.len().min(off + filtered.len())];
        let (n, hit) = crate::rs485::filter_host_pipe(chunk, &mut filtered);
        if n > 0 {
            pipe_push(&PIPE_HOST, &filtered[..n]);
            HOST_PIPE_SIG.signal(());
        }
        if hit {
            crate::uart_owner::request_restart();
        }
        off += chunk.len();
    }
}

pub fn pipe_pop_host(dst: &mut [u8]) -> usize {
    pipe_pop(&PIPE_HOST, dst)
}

pub fn pipe_host_pending() -> bool {
    PIPE_HOST.len.load(Ordering::Relaxed) > 0
}

pub async fn wait_host_pipe() {
    if pipe_host_pending() {
        return;
    }
    HOST_PIPE_SIG.wait().await;
}

pub fn pipe_push_bus(data: &[u8]) {
    pipe_push(&PIPE_BUS, data);
}

pub fn pipe_pop_bus(dst: &mut [u8]) -> usize {
    pipe_pop(&PIPE_BUS, dst)
}

fn pipe_bus_pending() -> bool {
    PIPE_BUS.len.load(Ordering::Relaxed) > 0
}

pub fn take_cdc_baud() -> u32 {
    CDC_BAUD.swap(0, Ordering::AcqRel)
}

#[cfg(feature = "esp32c6")]
fn poll_cdc_baud() {
    let usb = esp_hal::peripherals::USB_DEVICE::regs();
    let baud = usb.set_line_code_w0().read().dw_dte_rate().bits();
    if (1200..=3_000_000).contains(&baud) {
        static LAST: AtomicU32 = AtomicU32::new(0);
        let prev = LAST.load(Ordering::Relaxed);
        if baud != prev {
            LAST.store(baud, Ordering::Relaxed);
            CDC_BAUD.store(baud, Ordering::Relaxed);
            if usb.int_st().read().set_line_code().bit_is_set() {
                usb.int_clr()
                    .write(|w| w.set_line_code().clear_bit_by_one());
            }
        }
    }
}

#[cfg(not(feature = "esp32c6"))]
fn poll_cdc_baud() {}

struct LogByteRing {
    buf: UnsafeCell<[u8; LOG_RING_CAP]>,
    head: AtomicUsize,
    len: AtomicUsize,
}

unsafe impl Sync for LogByteRing {}

static LOG_RING: LogByteRing = LogByteRing {
    buf: UnsafeCell::new([0; LOG_RING_CAP]),
    head: AtomicUsize::new(0),
    len: AtomicUsize::new(0),
};

fn log_ring_len() -> usize {
    LOG_RING.len.load(Ordering::Relaxed)
}

/// Push a line. Drop-newest if it does not fit. Short CS (index + memcpy).
fn log_ring_push(data: &[u8]) {
    if data.is_empty() {
        return;
    }
    if data.len() > LOG_RING_CAP {
        return;
    }
    critical_section::with(|_| {
        let len = LOG_RING.len.load(Ordering::Relaxed);
        if LOG_RING_CAP.saturating_sub(len) < data.len() {
            return;
        }
        let head = LOG_RING.head.load(Ordering::Relaxed);
        let buf = unsafe { &mut *LOG_RING.buf.get() };
        for (i, &b) in data.iter().enumerate() {
            buf[(head + len + i) % LOG_RING_CAP] = b;
        }
        LOG_RING.len.store(len + data.len(), Ordering::Relaxed);
    });
}

fn log_ring_pop(dst: &mut [u8]) -> usize {
    critical_section::with(|_| {
        let len = LOG_RING.len.load(Ordering::Relaxed);
        let n = dst.len().min(len);
        if n == 0 {
            return 0;
        }
        let head = LOG_RING.head.load(Ordering::Relaxed);
        let buf = unsafe { &*LOG_RING.buf.get() };
        for (i, slot) in dst.iter_mut().take(n).enumerate() {
            *slot = buf[(head + i) % LOG_RING_CAP];
        }
        LOG_RING
            .head
            .store((head + n) % LOG_RING_CAP, Ordering::Relaxed);
        LOG_RING.len.store(len - n, Ordering::Relaxed);
        n
    })
}

fn set_log_live(live: bool) {
    LOG_LIVE.store(live, Ordering::Relaxed);
}

/// USB Serial/JTAG IN endpoint TX protocol state.
#[derive(Clone, Copy, PartialEq, Eq)]
enum UsbTxState {
    /// FIFO free; next `write_byte_nb` should succeed.
    Idle,
    /// Packet committed (`wr_done`); waiting for `serial_in_empty`.
    InFlight,
    /// Host not reading. Do not poke EP1 IN until flush succeeds.
    Stalled,
}

/// Fixed ring buffer: O(1) push/pop (no memmove on consume).
struct Ring {
    buf: [u8; SINK_TX_CAP],
    head: usize,
    len: usize,
}

impl Ring {
    const fn new() -> Self {
        Self {
            buf: [0; SINK_TX_CAP],
            head: 0,
            len: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn len(&self) -> usize {
        self.len
    }

    fn push(&mut self, b: u8) -> bool {
        if self.len >= SINK_TX_CAP {
            return false;
        }
        let i = (self.head + self.len) % SINK_TX_CAP;
        self.buf[i] = b;
        self.len += 1;
        true
    }

    fn peek_byte(&self) -> Option<u8> {
        if self.len == 0 {
            None
        } else {
            Some(self.buf[self.head])
        }
    }

    /// Contiguous bytes from head to end of ring storage (may be < len if wrapped).
    fn peek_contiguous(&self) -> &[u8] {
        if self.len == 0 {
            return &[];
        }
        let n = (SINK_TX_CAP - self.head).min(self.len);
        &self.buf[self.head..self.head + n]
    }

    fn consume(&mut self, n: usize) {
        let n = n.min(self.len);
        self.head = (self.head + n) % SINK_TX_CAP;
        self.len -= n;
        if self.len == 0 {
            self.head = 0;
        }
    }
}

struct ChannelLogger;

impl log::Log for ChannelLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Info
    }

    fn log(&self, record: &log::Record) {
        if !LOG_LIVE.load(Ordering::Relaxed) {
            return;
        }
        if !self.enabled(record.metadata()) {
            return;
        }
        let mut line = heapless::String::<LOG_LINE_CAP>::new();
        let _ = write!(
            &mut line,
            "[{}] {}: {}\r\n",
            record.level(),
            record.target(),
            record.args()
        );
        enqueue_bytes(line.as_bytes());
    }

    fn flush(&self) {}
}

static LOGGER: ChannelLogger = ChannelLogger;

/// Register the channel-backed logger (call after `esp_rtos::start`).
pub fn init_logger() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
}

/// Enqueue log bytes. Drop-newest if the ring has no room. No-op when USB is stalled.
pub fn enqueue_bytes(data: &[u8]) {
    if data.is_empty() || !LOG_LIVE.load(Ordering::Relaxed) {
        return;
    }
    log_ring_push(data);
}

fn enqueue_cli_bytes(data: &[u8]) {
    if data.is_empty() {
        return;
    }
    for piece in data.chunks(OUT_CAP) {
        let mut chunk = OutChunk::new();
        let _ = chunk.extend_from_slice(piece);
        let _ = CLI_OUT_CH.try_send(chunk);
    }
}

/// Enqueue raw bytes for CLI (no forced newline). Priority over log OUT.
pub fn write_bytes(data: &[u8]) {
    enqueue_cli_bytes(data);
}

/// Enqueue a line for CLI replies (appends `\r\n` on the last chunk).
pub fn write_line(s: &str) {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        enqueue_cli_bytes(b"\r\n");
        return;
    }
    let mut remaining = bytes;
    while !remaining.is_empty() {
        let take = remaining.len().min(OUT_CAP);
        if take == remaining.len() && take + 2 <= OUT_CAP {
            let mut chunk = OutChunk::new();
            let _ = chunk.extend_from_slice(remaining);
            let _ = chunk.extend_from_slice(b"\r\n");
            let _ = CLI_OUT_CH.try_send(chunk);
            return;
        }
        enqueue_cli_bytes(&remaining[..take]);
        remaining = &remaining[take..];
    }
    enqueue_cli_bytes(b"\r\n");
}

/// Blocking recv one input byte from console (USB or UART0).
pub async fn read_byte() -> u8 {
    IN_CH.receive().await
}

fn try_push_in(b: u8) {
    let _ = IN_CH.try_send(map_del(b));
}

fn push_in_slice(data: &[u8]) {
    if usb_mux_is_pipe() {
        pipe_push_host(data);
        return;
    }
    for &b in data {
        try_push_in(b);
    }
}

fn poll_usb_rx_nb(usb_rx: &mut UsbSerialJtagRx<'_, Async>) {
    if usb_mux_is_pipe() {
        let mut tmp = [0u8; 64];
        let mut n = 0usize;
        while n < tmp.len() {
            match usb_rx.read_byte() {
                Ok(b) => {
                    tmp[n] = b;
                    n += 1;
                }
                Err(_) => break,
            }
        }
        if n > 0 {
            pipe_push_host(&tmp[..n]);
        }
        return;
    }
    while let Ok(b) = usb_rx.read_byte() {
        try_push_in(b);
    }
}

fn poll_uart_rx_nb(uart: &mut Uart<'_, Blocking>) {
    while uart.read_ready() {
        let mut byte = [0u8; 1];
        match uart.read(&mut byte) {
            Ok(1) => try_push_in(byte[0]),
            _ => break,
        }
    }
}

fn out_pending() -> bool {
    log_ring_len() > 0 || !CLI_OUT_CH.is_empty()
}

fn map_del(b: u8) -> u8 {
    if b == 0x7F {
        0x08
    } else {
        b
    }
}

fn push_into(ring: &mut Ring, data: &[u8], offset: &mut usize) -> bool {
    let mut progressed = false;
    while *offset < data.len() && ring.len() < SINK_TX_CAP {
        if ring.push(data[*offset]) {
            *offset += 1;
            progressed = true;
        } else {
            break;
        }
    }
    progressed
}

/// Wait until the USB IN FIFO is free, or `USB_WAIT_US`. Does not write bytes.
async fn wait_usb_in_free(tx: &mut UsbSerialJtagTx<'_, Async>) -> bool {
    match select(
        AsyncWrite::flush(tx),
        Timer::after(Duration::from_micros(USB_WAIT_US)),
    )
    .await
    {
        Either::First(Ok(())) => true,
        Either::First(Err(_)) | Either::Second(()) => false,
    }
}

/// Commit at most one USB IN packet from `ring`. Bytes are consumed as they
/// enter the HW FIFO and are never rewritten after `wr_done`.
async fn drain_usb_packet(
    tx: &mut UsbSerialJtagTx<'_, Async>,
    ring: &mut Ring,
    state: &mut UsbTxState,
) -> bool {
    if *state != UsbTxState::Idle {
        if wait_usb_in_free(tx).await {
            *state = UsbTxState::Idle;
        } else {
            *state = UsbTxState::Stalled;
            return false;
        }
    }

    if ring.is_empty() {
        return false;
    }

    let mut n = 0usize;
    let mut retries = 0u32;
    while n < USB_EP_SIZE && !ring.is_empty() {
        let Some(b) = ring.peek_byte() else {
            break;
        };
        match tx.write_byte_nb(b) {
            Ok(()) => {
                ring.consume(1);
                n += 1;
            }
            Err(nb::Error::WouldBlock) => {
                if n == 0 {
                    if retries >= 2 || !wait_usb_in_free(tx).await {
                        *state = UsbTxState::Stalled;
                        return false;
                    }
                    retries += 1;
                    continue;
                }
                break;
            }
            Err(nb::Error::Other(_)) => return false,
        }
    }

    if n == 0 {
        return false;
    }

    let _ = tx.flush_tx_nb();
    *state = UsbTxState::InFlight;
    if wait_usb_in_free(tx).await {
        *state = UsbTxState::Idle;
    } else {
        *state = UsbTxState::Stalled;
    }
    true
}

async fn drain_usb(
    tx: &mut UsbSerialJtagTx<'_, Async>,
    ring: &mut Ring,
    max_packets: Option<u32>,
    state: &mut UsbTxState,
) {
    let deadline = Instant::now() + Duration::from_millis(TOTAL_TIMEOUT_MS);
    let mut packets = 0u32;
    while !ring.is_empty() {
        if Instant::now() >= deadline {
            return;
        }
        if let Some(limit) = max_packets {
            if packets >= limit {
                return;
            }
        }
        if !drain_usb_packet(tx, ring, state).await {
            return;
        }
        packets += 1;
        if *state == UsbTxState::Stalled {
            return;
        }
    }
}

async fn drain_uart(uart: &mut Uart<'_, Blocking>, ring: &mut Ring) {
    let deadline = Instant::now() + Duration::from_millis(TOTAL_TIMEOUT_MS);
    let mut stalled_once = false;
    while !ring.is_empty() {
        let before = ring.len();
        while !ring.is_empty() && uart.write_ready() {
            let cont = ring.peek_contiguous();
            match uart.write(cont) {
                Ok(0) => break,
                Ok(n) => ring.consume(n),
                Err(_) => break,
            }
        }
        if ring.is_empty() {
            return;
        }
        if ring.len() < before {
            stalled_once = false;
            continue;
        }
        // Do not serialize the console behind UART0 with no host.
        if stalled_once || Instant::now() >= deadline {
            return;
        }
        Timer::after(Duration::from_micros(YIELD_US)).await;
        stalled_once = true;
    }
}

async fn drain_rtt(rtt: &mut UpChannel, ring: &mut Ring) {
    let deadline = Instant::now() + Duration::from_millis(TOTAL_TIMEOUT_MS);
    let mut stalled_once = false;
    while !ring.is_empty() {
        let before = ring.len();
        let cont = ring.peek_contiguous();
        let n = rtt.write(cont);
        if n > 0 {
            ring.consume(n);
        }
        if ring.is_empty() {
            return;
        }
        if ring.len() < before {
            stalled_once = false;
            continue;
        }
        // Host not reading (or buffer too full for progress). Drop remainder so
        // a later attach is not blocked by a forever-full TX ring.
        if stalled_once || Instant::now() >= deadline {
            ring.consume(ring.len());
            return;
        }
        Timer::after(Duration::from_micros(YIELD_US)).await;
        stalled_once = true;
    }
}

struct ConsoleSinks<'a, 'd> {
    usb_tx: &'a mut UsbSerialJtagTx<'d, Async>,
    uart: &'a mut Uart<'d, Blocking>,
    rtt: &'a mut UpChannel,
    usb_tx_cli: &'a mut Ring,
    usb_tx_log: &'a mut Ring,
    uart_tx: &'a mut Ring,
    rtt_tx: &'a mut Ring,
    usb_tx_state: UsbTxState,
    need_banner: bool,
}

impl ConsoleSinks<'_, '_> {
    fn enter_stall(&mut self) {
        if self.usb_tx_state != UsbTxState::Stalled {
            self.need_banner = true;
        }
        self.usb_tx_state = UsbTxState::Stalled;
        self.usb_tx_log.consume(self.usb_tx_log.len());
        set_log_live(false);
    }

    fn recover_idle(&mut self) {
        self.usb_tx_state = UsbTxState::Idle;
        set_log_live(true);
        self.maybe_emit_banner();
    }

    fn maybe_emit_banner(&mut self) {
        if usb_mux_is_pipe() {
            self.need_banner = false;
            return;
        }
        if !self.need_banner || self.usb_tx_state != UsbTxState::Idle {
            return;
        }
        self.need_banner = false;
        let mut off = 0usize;
        let _ = push_into(self.usb_tx_cli, CONSOLE_BANNER, &mut off);
    }
}

async fn drain_side_sinks(sinks: &mut ConsoleSinks<'_, '_>) {
    if !sinks.uart_tx.is_empty() {
        drain_uart(sinks.uart, sinks.uart_tx).await;
    }
    if !sinks.rtt_tx.is_empty() {
        drain_rtt(sinks.rtt, sinks.rtt_tx).await;
    }
}

async fn drain_sinks_progress(sinks: &mut ConsoleSinks<'_, '_>) {
    if sinks.usb_tx_state != UsbTxState::Idle {
        if wait_usb_in_free(sinks.usb_tx).await {
            sinks.recover_idle();
        } else {
            sinks.enter_stall();
            return;
        }
    }

    drain_usb(
        sinks.usb_tx,
        sinks.usb_tx_cli,
        None,
        &mut sinks.usb_tx_state,
    )
    .await;
    if sinks.usb_tx_state == UsbTxState::Stalled {
        sinks.enter_stall();
        drain_side_sinks(sinks).await;
        return;
    }

    drain_usb(
        sinks.usb_tx,
        sinks.usb_tx_log,
        Some(USB_DRAIN_MAX_PACKETS),
        &mut sinks.usb_tx_state,
    )
    .await;
    if sinks.usb_tx_state == UsbTxState::Stalled {
        sinks.enter_stall();
    }
    drain_side_sinks(sinks).await;
}

async fn drain_cli_out_batch(sinks: &mut ConsoleSinks<'_, '_>) -> bool {
    let mut progressed = false;
    for _ in 0..CLI_BATCH_MAX {
        let Ok(chunk) = CLI_OUT_CH.try_receive() else {
            break;
        };
        progressed = true;
        accept_out(sinks, false, chunk.as_slice()).await;
        if sinks.usb_tx_state == UsbTxState::Stalled {
            break;
        }
    }
    if !sinks.usb_tx_cli.is_empty() && sinks.usb_tx_state != UsbTxState::Stalled {
        drain_usb(
            sinks.usb_tx,
            sinks.usb_tx_cli,
            None,
            &mut sinks.usb_tx_state,
        )
        .await;
        progressed = true;
        if sinks.usb_tx_state == UsbTxState::Stalled {
            sinks.enter_stall();
        }
    }
    progressed
}

async fn drain_out_batch(sinks: &mut ConsoleSinks<'_, '_>, tx_deadline: Instant) -> bool {
    let mut progressed = false;
    if sinks.usb_tx_state == UsbTxState::Stalled {
        return progressed;
    }
    for _ in 0..TX_BATCH_MAX {
        if Instant::now() >= tx_deadline {
            break;
        }
        let mut buf = [0u8; OUT_CAP];
        let n = log_ring_pop(&mut buf);
        if n == 0 {
            break;
        }
        progressed = true;
        accept_out(sinks, true, &buf[..n]).await;
        if sinks.usb_tx_state == UsbTxState::Stalled {
            break;
        }
    }
    if !sinks.usb_tx_cli.is_empty()
        || !sinks.usb_tx_log.is_empty()
        || !sinks.uart_tx.is_empty()
        || !sinks.rtt_tx.is_empty()
        || sinks.usb_tx_state != UsbTxState::Idle
    {
        drain_sinks_progress(sinks).await;
    }
    progressed
}

async fn tx_step(sinks: &mut ConsoleSinks<'_, '_>) {
    if sinks.usb_tx_state != UsbTxState::Idle {
        if wait_usb_in_free(sinks.usb_tx).await {
            sinks.recover_idle();
        } else {
            sinks.enter_stall();
            return;
        }
    }

    if usb_mux_is_pipe() {
        let mut buf = [0u8; USB_EP_SIZE];
        let n = pipe_pop_bus(&mut buf);
        if n > 0 {
            let mut off = 0usize;
            let _ = push_into(sinks.usb_tx_cli, &buf[..n], &mut off);
        }
        if !sinks.usb_tx_cli.is_empty() && sinks.usb_tx_state != UsbTxState::Stalled {
            drain_usb(
                sinks.usb_tx,
                sinks.usb_tx_cli,
                None,
                &mut sinks.usb_tx_state,
            )
            .await;
            if sinks.usb_tx_state == UsbTxState::Stalled {
                sinks.enter_stall();
                return;
            }
        }
        let _ = drain_cli_out_batch(sinks).await;
        let tx_deadline = Instant::now() + Duration::from_millis(LOOP_TX_BUDGET_MS);
        let _ = drain_out_batch(sinks, tx_deadline).await;
        return;
    }

    let _ = drain_cli_out_batch(sinks).await;
    if sinks.usb_tx_state == UsbTxState::Stalled {
        return;
    }
    let tx_deadline = Instant::now() + Duration::from_millis(LOOP_TX_BUDGET_MS);
    let _ = drain_out_batch(sinks, tx_deadline).await;
}

/// Stream `data` into sink rings. USB is the primary console path: once all
/// bytes are queued (and drained when the USB ring is full), return without
/// waiting on UART0/RTT stalls — those are best-effort only.
async fn accept_out(sinks: &mut ConsoleSinks<'_, '_>, is_log: bool, data: &[u8]) {
    if data.is_empty() {
        return;
    }
    if usb_mux_is_pipe() {
        let mut uart_off = 0usize;
        let mut rtt_off = 0usize;
        let _ = push_into(sinks.uart_tx, data, &mut uart_off);
        let _ = push_into(sinks.rtt_tx, data, &mut rtt_off);
        drain_side_sinks(sinks).await;
        return;
    }
    if is_log && sinks.usb_tx_state == UsbTxState::Stalled {
        sinks.enter_stall();
        let mut uart_off = 0usize;
        let mut rtt_off = 0usize;
        let _ = push_into(sinks.uart_tx, data, &mut uart_off);
        let _ = push_into(sinks.rtt_tx, data, &mut rtt_off);
        drain_side_sinks(sinks).await;
        return;
    }

    let mut usb_off = 0usize;
    let mut uart_off = 0usize;
    let mut rtt_off = 0usize;
    let mut yields = 0u32;
    let deadline = Instant::now() + Duration::from_millis(ACCEPT_OUT_BUDGET_MS);
    let usb_cap = if is_log {
        Some(USB_DRAIN_MAX_PACKETS)
    } else {
        None
    };

    loop {
        let p0 = {
            let usb_ring = if is_log {
                &mut *sinks.usb_tx_log
            } else {
                &mut *sinks.usb_tx_cli
            };
            push_into(usb_ring, data, &mut usb_off)
        };
        // Best-effort side sinks — never gate completion on them.
        let _ = push_into(sinks.uart_tx, data, &mut uart_off);
        let _ = push_into(sinks.rtt_tx, data, &mut rtt_off);

        let usb_ring_empty = if is_log {
            sinks.usb_tx_log.is_empty()
        } else {
            sinks.usb_tx_cli.is_empty()
        };

        if !usb_ring_empty || !sinks.uart_tx.is_empty() || !sinks.rtt_tx.is_empty() {
            let usb_ring = if is_log {
                &mut *sinks.usb_tx_log
            } else {
                &mut *sinks.usb_tx_cli
            };
            drain_usb(sinks.usb_tx, usb_ring, usb_cap, &mut sinks.usb_tx_state).await;
            if sinks.usb_tx_state == UsbTxState::Stalled {
                sinks.enter_stall();
                drain_side_sinks(sinks).await;
                return;
            }
            drain_uart(sinks.uart, sinks.uart_tx).await;
            drain_rtt(sinks.rtt, sinks.rtt_tx).await;
        }

        // Primary path done: all bytes fed into the USB ring.
        if usb_off >= data.len() {
            let _ = push_into(sinks.uart_tx, data, &mut uart_off);
            let _ = push_into(sinks.rtt_tx, data, &mut rtt_off);

            let usb_ring_empty = if is_log {
                sinks.usb_tx_log.is_empty()
            } else {
                sinks.usb_tx_cli.is_empty()
            };
            if !usb_ring_empty {
                let usb_ring = if is_log {
                    &mut *sinks.usb_tx_log
                } else {
                    &mut *sinks.usb_tx_cli
                };
                drain_usb(sinks.usb_tx, usb_ring, usb_cap, &mut sinks.usb_tx_state).await;
                if sinks.usb_tx_state == UsbTxState::Stalled {
                    sinks.enter_stall();
                }
            }
            drain_side_sinks(sinks).await;
            return;
        }

        if p0 {
            continue;
        }
        // USB ring full and not progressing — short yield, then retry.
        if yields >= MAX_YIELDS.saturating_mul(5) || Instant::now() >= deadline {
            return;
        }
        Timer::after(Duration::from_micros(YIELD_US)).await;
        yields += 1;
    }
}

/// Best-effort panic dump that bypasses the async OUT queue.
pub fn panic_write(s: &str) {
    #[cfg(feature = "esp32c6")]
    const USB_FIFO: usize = 0x6000_F000;
    #[cfg(feature = "esp32c6")]
    const USB_CONF: usize = 0x6000_F004;
    #[cfg(feature = "esp32s3")]
    const USB_FIFO: usize = 0x6003_8000;
    #[cfg(feature = "esp32s3")]
    const USB_CONF: usize = 0x6003_8004;

    for &b in s.as_bytes() {
        let mut spins = 8_000u32;
        while spins > 0 {
            let full = unsafe { (USB_CONF as *const u32).read_volatile() & 0b010 == 0 };
            if !full {
                unsafe {
                    (USB_FIFO as *mut u32).write_volatile(b as u32);
                }
                break;
            }
            spins -= 1;
        }
    }
    unsafe {
        (USB_CONF as *mut u32).write_volatile(0b001);
    }
    if let Some(mut ch) = unsafe { UpChannel::conjure(0) } {
        let _ = ch.write(s.as_bytes());
    }
}

/// Chip-specific UART0 TX/RX pins for the console.
pub struct UartPins {
    tx: AnyPin<'static>,
    rx: AnyPin<'static>,
}

impl UartPins {
    pub fn new(tx: impl Into<AnyPin<'static>>, rx: impl Into<AnyPin<'static>>) -> Self {
        Self {
            tx: tx.into(),
            rx: rx.into(),
        }
    }
}

fn cli_tx_pending(sinks: &ConsoleSinks<'_, '_>) -> bool {
    !CLI_OUT_CH.is_empty() || !sinks.usb_tx_cli.is_empty()
}

fn tx_pending(sinks: &ConsoleSinks<'_, '_>) -> bool {
    if usb_mux_is_pipe() && (pipe_bus_pending() || !sinks.usb_tx_cli.is_empty()) {
        return true;
    }
    if sinks.usb_tx_state == UsbTxState::Stalled {
        return cli_tx_pending(sinks);
    }
    out_pending()
        || !sinks.usb_tx_cli.is_empty()
        || !sinks.usb_tx_log.is_empty()
        || !sinks.uart_tx.is_empty()
        || !sinks.rtt_tx.is_empty()
        || sinks.usb_tx_state != UsbTxState::Idle
        || sinks.need_banner
}

/// Async console owner: interrupt USB + timed waits; UART0/RTT nb + 100 µs yields.
pub async fn run(
    usb_dev: esp_hal::peripherals::USB_DEVICE<'static>,
    uart0: esp_hal::peripherals::UART0<'static>,
    pins: UartPins,
) -> ! {
    let channels = rtt_init! {
        up: {
            0: {
                size: 1024,
                // Trim (not Skip): write what fits when the host lags. Skip would
                // reject the entire ring slice if the RTT buffer cannot take it
                // all, permanently filling our TX ring when no debugger is present.
                mode: ChannelMode::NoBlockTrim,
                name: "Terminal"
            }
        }
    };
    let mut rtt = channels.up.0;

    let usb = UsbSerialJtag::new(usb_dev).into_async();
    let (mut usb_rx, mut usb_tx) = usb.split();
    let tx_pin = Output::new(pins.tx, esp_hal::gpio::Level::High, OutputConfig::default());
    let rx_pin = Input::new(pins.rx, InputConfig::default().with_pull(Pull::Up));
    let mut uart = Uart::new(uart0, UartConfig::default().with_baudrate(115_200))
        .expect("UART0 init")
        .with_tx(tx_pin)
        .with_rx(rx_pin);

    let mut usb_tx_cli = Ring::new();
    let mut usb_tx_log = Ring::new();
    let mut uart_tx = Ring::new();
    let mut rtt_tx = Ring::new();
    let mut rx_tmp = [0u8; USB_EP_SIZE];

    let mut sinks = ConsoleSinks {
        usb_tx: &mut usb_tx,
        uart: &mut uart,
        rtt: &mut rtt,
        usb_tx_cli: &mut usb_tx_cli,
        usb_tx_log: &mut usb_tx_log,
        uart_tx: &mut uart_tx,
        rtt_tx: &mut rtt_tx,
        usb_tx_state: UsbTxState::Idle,
        need_banner: false,
    };

    accept_out(&mut sinks, true, CONSOLE_BANNER).await;

    loop {
        poll_cdc_baud();
        poll_usb_rx_nb(&mut usb_rx);
        poll_uart_rx_nb(sinks.uart);

        if sinks.usb_tx_state == UsbTxState::Stalled && !cli_tx_pending(&sinks) {
            match select(
                AsyncRead::read(&mut usb_rx, &mut rx_tmp),
                Timer::after(Duration::from_micros(STALL_PROBE_US)),
            )
            .await
            {
                Either::First(Ok(n)) => push_in_slice(&rx_tmp[..n]),
                Either::First(Err(_)) => {}
                Either::Second(()) => {
                    if wait_usb_in_free(sinks.usb_tx).await {
                        sinks.recover_idle();
                    }
                }
            }
            poll_usb_rx_nb(&mut usb_rx);
            poll_uart_rx_nb(sinks.uart);
        } else if tx_pending(&sinks) {
            match select(
                AsyncRead::read(&mut usb_rx, &mut rx_tmp),
                tx_step(&mut sinks),
            )
            .await
            {
                Either::First(Ok(n)) => push_in_slice(&rx_tmp[..n]),
                Either::First(Err(_)) => {}
                Either::Second(()) => {}
            }
            poll_usb_rx_nb(&mut usb_rx);
            poll_uart_rx_nb(sinks.uart);
            Timer::after(Duration::from_micros(YIELD_US)).await;
        } else {
            match select(
                AsyncRead::read(&mut usb_rx, &mut rx_tmp),
                Timer::after(Duration::from_micros(UART_POLL_US)),
            )
            .await
            {
                Either::First(Ok(n)) => push_in_slice(&rx_tmp[..n]),
                Either::First(Err(_)) => {}
                Either::Second(()) => {}
            }
            poll_usb_rx_nb(&mut usb_rx);
            poll_uart_rx_nb(sinks.uart);
            Timer::after(Duration::from_micros(IDLE_YIELD_US)).await;
        }
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    critical_section::with(|_| {
        let mut buf = heapless::String::<256>::new();
        let _ = write!(
            &mut buf,
            "\r\n====================== PANIC ======================\r\n"
        );
        let _ = write!(&mut buf, "{}\r\n\r\nBacktrace:\r\n", info);
        panic_write(buf.as_str());

        let frames = esp_backtrace::arch::backtrace();
        let mut any = false;
        for addr in frames.into_iter().flatten() {
            any = true;
            let mut line = heapless::String::<32>::new();
            let _ = write!(&mut line, "0x{:x}\r\n", addr.saturating_sub(4));
            panic_write(line.as_str());
        }
        if !any {
            panic_write("No backtrace available - make sure to force frame-pointers.\r\n");
        }
        panic_write("\r\n");
    });
    loop {}
}
