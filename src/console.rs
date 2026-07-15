//! Sole owner of USB Serial/JTAG + UART0 console I/O, plus RTT (TX only).
//!
//! Other tasks talk via bounded channels. USB uses interrupt-driven async I/O with
//! select timeouts (no hang without a host). UART0/RTT use nb + 100 µs yields.
//! Per-sink TX rings drain without busy-wait or O(n) buffer shifts.

use core::fmt::Write as FmtWrite;

use embassy_futures::select::{select, select3, Either, Either3};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Instant, Timer};
use embedded_io_async::{Read as AsyncRead, Write as AsyncWrite};
use esp_hal::gpio::{AnyPin, Input, InputConfig, Output, OutputConfig, Pull};
use esp_hal::uart::{Config as UartConfig, Uart};
use esp_hal::usb_serial_jtag::UsbSerialJtag;
use esp_hal::{Async, Blocking};
use heapless::Vec as HVec;
use rtt_target::{rtt_init, ChannelMode, UpChannel};

const OUT_CAP: usize = 192;
const OUT_QUEUE: usize = 32;
const IN_QUEUE: usize = 128;
const SINK_TX_CAP: usize = 256;
const USB_EP_SIZE: usize = 64;
const MAX_YIELDS: u32 = 10;
/// Backoff when a sink cannot progress (UART/RTT or accept_out space).
const YIELD_US: u64 = 100;
/// Max wait for one USB EP IN packet ACK / RX wake select arm.
const USB_WAIT_US: u64 = 500;
/// No-host abandon budget for a single drain pass.
const TOTAL_TIMEOUT_MS: u64 = 10;
/// UART0 RX poll while blocked on USB/OUT (USB RX is interrupt-driven).
const UART_POLL_US: u64 = 500;
const LOG_LINE_CAP: usize = 256;

pub type OutChunk = HVec<u8, OUT_CAP>;

static OUT_CH: Channel<CriticalSectionRawMutex, OutChunk, OUT_QUEUE> = Channel::new();
static IN_CH: Channel<CriticalSectionRawMutex, u8, IN_QUEUE> = Channel::new();

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

/// Enqueue raw bytes, split across OUT chunks of at most `OUT_CAP`.
pub fn enqueue_bytes(data: &[u8]) {
    if data.is_empty() {
        return;
    }
    for piece in data.chunks(OUT_CAP) {
        let mut chunk = OutChunk::new();
        let _ = chunk.extend_from_slice(piece);
        let _ = OUT_CH.try_send(chunk);
    }
}

/// Enqueue raw bytes for CLI (no forced newline).
pub fn write_bytes(data: &[u8]) {
    enqueue_bytes(data);
}

/// Enqueue a line for CLI replies (appends `\r\n`). Streams large JSON via `enqueue_bytes`.
pub fn write_line(s: &str) {
    enqueue_bytes(s.as_bytes());
    enqueue_bytes(b"\r\n");
}

/// Blocking recv one input byte from console (USB or UART0).
pub async fn read_byte() -> u8 {
    IN_CH.receive().await
}

fn try_push_in(b: u8) {
    let _ = IN_CH.try_send(b);
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

/// Interrupt USB TX: write ≤64 B then await IN_EMPTY with timeout (no hang).
async fn drain_usb(usb: &mut UsbSerialJtag<'_, Async>, ring: &mut Ring) {
    let deadline = Instant::now() + Duration::from_millis(TOTAL_TIMEOUT_MS);
    while !ring.is_empty() {
        if Instant::now() >= deadline {
            return;
        }
        let cont = ring.peek_contiguous();
        let n = cont.len().min(USB_EP_SIZE);
        if n == 0 {
            return;
        }
        let mut tmp = [0u8; USB_EP_SIZE];
        tmp[..n].copy_from_slice(&cont[..n]);

        match select(
            AsyncWrite::write(usb, &tmp[..n]),
            Timer::after(Duration::from_micros(USB_WAIT_US)),
        )
        .await
        {
            Either::First(Ok(written)) => {
                ring.consume(written.min(n));
            }
            Either::First(Err(_)) | Either::Second(()) => {
                // No host / still full: leave remainder for a later pass.
                return;
            }
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

async fn drain_sinks(
    usb: &mut UsbSerialJtag<'_, Async>,
    uart: &mut Uart<'_, Blocking>,
    rtt: &mut UpChannel,
    usb_tx: &mut Ring,
    uart_tx: &mut Ring,
    rtt_tx: &mut Ring,
) {
    // USB first — do not serialize behind UART/RTT stalls.
    drain_usb(usb, usb_tx).await;
    drain_uart(uart, uart_tx).await;
    drain_rtt(rtt, rtt_tx).await;
}

fn push_into_in_from_slice(data: &[u8]) {
    for &b in data {
        try_push_in(map_del(b));
    }
}

fn poll_usb_rx_nb(usb: &mut UsbSerialJtag<'_, Async>) {
    while let Ok(b) = usb.read_byte() {
        try_push_in(map_del(b));
    }
}

fn poll_uart_rx(uart: &mut Uart<'_, Blocking>) {
    while uart.read_ready() {
        let mut byte = [0u8; 1];
        match uart.read(&mut byte) {
            Ok(1) => try_push_in(map_del(byte[0])),
            _ => break,
        }
    }
}

/// Stream `data` into sink rings. USB is the primary console path: once all
/// bytes are queued (and drained when the USB ring is full), return without
/// waiting on UART0/RTT stalls — those are best-effort only.
async fn accept_out(
    usb: &mut UsbSerialJtag<'_, Async>,
    uart: &mut Uart<'_, Blocking>,
    rtt: &mut UpChannel,
    usb_tx: &mut Ring,
    uart_tx: &mut Ring,
    rtt_tx: &mut Ring,
    data: &[u8],
) {
    if data.is_empty() {
        return;
    }
    let mut usb_off = 0usize;
    let mut uart_off = 0usize;
    let mut rtt_off = 0usize;
    let mut yields = 0u32;
    let deadline = Instant::now() + Duration::from_millis(TOTAL_TIMEOUT_MS.saturating_mul(20));

    loop {
        let p0 = push_into(usb_tx, data, &mut usb_off);
        // Best-effort side sinks — never gate completion on them.
        let _ = push_into(uart_tx, data, &mut uart_off);
        let _ = push_into(rtt_tx, data, &mut rtt_off);

        if !usb_tx.is_empty() || !uart_tx.is_empty() || !rtt_tx.is_empty() {
            drain_sinks(usb, uart, rtt, usb_tx, uart_tx, rtt_tx).await;
        }

        // Primary path done: all bytes fed into the USB ring.
        if usb_off >= data.len() {
            // Opportunistic last push into UART/RTT with remaining free space.
            let _ = push_into(uart_tx, data, &mut uart_off);
            let _ = push_into(rtt_tx, data, &mut rtt_off);
            if !usb_tx.is_empty() {
                drain_usb(usb, usb_tx).await;
            }
            if !uart_tx.is_empty() || !rtt_tx.is_empty() {
                drain_uart(uart, uart_tx).await;
                drain_rtt(rtt, rtt_tx).await;
            }
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

    let mut usb = UsbSerialJtag::new(usb_dev).into_async();
    let tx_pin = Output::new(pins.tx, esp_hal::gpio::Level::High, OutputConfig::default());
    let rx_pin = Input::new(pins.rx, InputConfig::default().with_pull(Pull::Up));
    let mut uart = Uart::new(uart0, UartConfig::default().with_baudrate(115_200))
        .expect("UART0 init")
        .with_tx(tx_pin)
        .with_rx(rx_pin);

    let mut usb_tx = Ring::new();
    let mut uart_tx = Ring::new();
    let mut rtt_tx = Ring::new();
    let mut rx_tmp = [0u8; USB_EP_SIZE];

    accept_out(
        &mut usb,
        &mut uart,
        &mut rtt,
        &mut usb_tx,
        &mut uart_tx,
        &mut rtt_tx,
        b"\r\n[console] ready\r\n",
    )
    .await;

    loop {
        while let Ok(chunk) = OUT_CH.try_receive() {
            accept_out(
                &mut usb,
                &mut uart,
                &mut rtt,
                &mut usb_tx,
                &mut uart_tx,
                &mut rtt_tx,
                chunk.as_slice(),
            )
            .await;
        }
        drain_sinks(
            &mut usb,
            &mut uart,
            &mut rtt,
            &mut usb_tx,
            &mut uart_tx,
            &mut rtt_tx,
        )
        .await;
        poll_usb_rx_nb(&mut usb);
        poll_uart_rx(&mut uart);

        match select3(
            OUT_CH.receive(),
            AsyncRead::read(&mut usb, &mut rx_tmp),
            Timer::after(Duration::from_micros(UART_POLL_US)),
        )
        .await
        {
            Either3::First(chunk) => {
                accept_out(
                    &mut usb,
                    &mut uart,
                    &mut rtt,
                    &mut usb_tx,
                    &mut uart_tx,
                    &mut rtt_tx,
                    chunk.as_slice(),
                )
                .await;
                poll_usb_rx_nb(&mut usb);
                poll_uart_rx(&mut uart);
            }
            Either3::Second(Ok(n)) => {
                push_into_in_from_slice(&rx_tmp[..n]);
                poll_usb_rx_nb(&mut usb);
                poll_uart_rx(&mut uart);
            }
            Either3::Second(Err(_)) => {
                poll_uart_rx(&mut uart);
            }
            Either3::Third(()) => {
                if !usb_tx.is_empty() || !uart_tx.is_empty() || !rtt_tx.is_empty() {
                    drain_sinks(
                        &mut usb,
                        &mut uart,
                        &mut rtt,
                        &mut usb_tx,
                        &mut uart_tx,
                        &mut rtt_tx,
                    )
                    .await;
                }
                poll_usb_rx_nb(&mut usb);
                poll_uart_rx(&mut uart);
            }
        }
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    critical_section::with(|_| {
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, "\r\n====================== PANIC ======================\r\n");
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
            panic_write(
                "No backtrace available - make sure to force frame-pointers.\r\n",
            );
        }
        panic_write("\r\n");
    });
    loop {}
}
