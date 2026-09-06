//! Loop-rate and bus-link telemetry aggregators for OSSM shells.
//!
//! Pure sample math and 1-second windows; callers supply microsecond timestamps.
//! API JSON lives in `ossm-core` (`StateResponse`), not here.

use alloc::vec::Vec;

use crate::pll::{PllInfo, PllState};

pub const LOOP_HISTORY_LEN: usize = 10;
pub const LINK_SAMPLE_CAP: usize = 350;
pub const WINDOW_US: u64 = 1_000_000;

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct TimingWindowStats {
    pub min: u16,
    pub max: u16,
    pub pct5: u16,
    pub pct10: u16,
    pub pct50: u16,
    pub pct90: u16,
    pub pct95: u16,
    pub mean: u16,
    pub mdev: u16,
}

/// Fixed-capacity history (JSON array of the used prefix is serialized in core).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Hist<T: Copy + Default, const N: usize> {
    data: [T; N],
    len: u8,
}

impl<T: Copy + Default, const N: usize> Default for Hist<T, N> {
    fn default() -> Self {
        Self {
            data: [T::default(); N],
            len: 0,
        }
    }
}

impl<T: Copy + Default, const N: usize> Hist<T, N> {
    pub fn as_slice(&self) -> &[T] {
        &self.data[..self.len as usize]
    }

    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn push(&mut self, v: T) {
        if (self.len as usize) < N {
            self.data[self.len as usize] = v;
            self.len += 1;
        } else {
            self.data.copy_within(1.., 0);
            self.data[N - 1] = v;
        }
    }

    pub fn last(&self) -> Option<T> {
        if self.len == 0 {
            None
        } else {
            Some(self.data[self.len as usize - 1])
        }
    }
}

pub type LoopHistoryU32 = Hist<u32, LOOP_HISTORY_LEN>;
pub type LoopHistoryF32 = Hist<f32, LOOP_HISTORY_LEN>;

/// Bus kind on the link snapshot. Wire names are kebab-case (`as_str`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LinkTransport {
    #[default]
    None,
    Mock,
    Uart,
    Serial,
    UsbHost,
    Rs485Ws,
    ModbusWs,
    RelayTcp,
}

impl LinkTransport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Mock => "mock",
            Self::Uart => "uart",
            Self::Serial => "serial",
            Self::UsbHost => "usb-host",
            Self::Rs485Ws => "rs485-ws",
            Self::ModbusWs => "modbus-ws",
            Self::RelayTcp => "relay-tcp",
        }
    }

    pub fn from_wire(s: &str) -> Self {
        match s {
            "mock" => Self::Mock,
            "uart" => Self::Uart,
            "serial" => Self::Serial,
            "usb-host" => Self::UsbHost,
            "rs485-ws" => Self::Rs485Ws,
            "modbus-ws" => Self::ModbusWs,
            "relay-tcp" => Self::RelayTcp,
            _ => Self::None,
        }
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct LoopStats {
    pub ups: u32,
    pub dt_min_ms: f32,
    pub dt_avg_ms: f32,
    pub dt_max_ms: f32,
    pub dt_mdev_ms: f32,
    pub update_history: LoopHistoryU32,
    pub position_history: LoopHistoryF32,
}

impl LoopStats {
    pub fn copy_from(&mut self, src: &Self) {
        *self = *src;
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct LinkStats {
    pub transport: LinkTransport,
    pub connected: bool,
    pub exchanges: u32,
    pub failures: u32,
    pub success_rate: f32,
    pub exchanges_per_sec: u32,
    pub bytes_tx: u64,
    pub bytes_rx: u64,
    pub reconnects: u32,
    pub round_trip: TimingWindowStats,
    pub slave_latency: TimingWindowStats,
    pub rx_duration: TimingWindowStats,
    pub pacing_state: PllState,
    pub pacing_period_us: u32,
    pub pacing_delay_us: u32,
    pub rtt_min_us: u32,
}

impl LinkStats {
    pub fn copy_from(&mut self, src: &Self) {
        *self = *src;
    }
}

pub fn compute_timing_stats(samples: &mut [u16]) -> TimingWindowStats {
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

/// Motor loop update-rate window (1 s flush, 10-entry histories).
pub struct LoopStatsWindow {
    last_window_us: Option<u64>,
    current_updates: u32,
    current_min_dt_ms: f32,
    current_max_dt_ms: f32,
    current_sum_dt_ms: f32,
    current_sum_sq_dt_ms: f32,
    update_history: LoopHistoryU32,
    position_history: LoopHistoryF32,
    latest: LoopStats,
    dirty: bool,
}

impl Default for LoopStatsWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl LoopStatsWindow {
    pub fn new() -> Self {
        Self {
            last_window_us: None,
            current_updates: 0,
            current_min_dt_ms: 0.0,
            current_max_dt_ms: 0.0,
            current_sum_dt_ms: 0.0,
            current_sum_sq_dt_ms: 0.0,
            update_history: LoopHistoryU32::default(),
            position_history: LoopHistoryF32::default(),
            latest: LoopStats::default(),
            dirty: false,
        }
    }

    pub fn reset_clock(&mut self, now_us: u64) {
        self.last_window_us = Some(now_us);
        self.current_updates = 0;
        self.current_sum_dt_ms = 0.0;
        self.current_sum_sq_dt_ms = 0.0;
    }

    pub fn record(&mut self, now_us: u64, dt_ms: f32, position: f32) {
        if self.current_updates == 0 {
            self.current_min_dt_ms = dt_ms;
            self.current_max_dt_ms = dt_ms;
        } else {
            if dt_ms < self.current_min_dt_ms {
                self.current_min_dt_ms = dt_ms;
            }
            if dt_ms > self.current_max_dt_ms {
                self.current_max_dt_ms = dt_ms;
            }
        }
        self.current_sum_dt_ms += dt_ms;
        self.current_sum_sq_dt_ms += dt_ms * dt_ms;
        self.current_updates = self.current_updates.saturating_add(1);

        if self.last_window_us.is_none() {
            self.last_window_us = Some(now_us);
        }
        if let Some(start) = self.last_window_us {
            if now_us.saturating_sub(start) >= WINDOW_US {
                self.flush_window(position);
                self.last_window_us = Some(now_us);
            }
        }
    }

    fn flush_window(&mut self, position: f32) {
        let n = self.current_updates as f32;
        let avg_dt_ms = if n > 0.0 {
            self.current_sum_dt_ms / n
        } else {
            0.0
        };
        let var = if n > 0.0 {
            (self.current_sum_sq_dt_ms / n) - (avg_dt_ms * avg_dt_ms)
        } else {
            0.0
        };
        let dt_mdev_ms = if var > 0.0 { libm::sqrtf(var) } else { 0.0 };

        self.update_history.push(self.current_updates);
        self.position_history.push(position);

        self.latest = LoopStats {
            ups: self.current_updates,
            dt_min_ms: self.current_min_dt_ms,
            dt_avg_ms: avg_dt_ms,
            dt_max_ms: self.current_max_dt_ms,
            dt_mdev_ms,
            update_history: self.update_history,
            position_history: self.position_history,
        };
        self.dirty = true;

        self.current_updates = 0;
        self.current_sum_dt_ms = 0.0;
        self.current_sum_sq_dt_ms = 0.0;
    }

    /// Returns the latest flushed window; may flush if the 1 s boundary elapsed.
    pub fn poll(&mut self, now_us: u64) -> LoopStats {
        self.flush_if_due(now_us);
        self.latest
    }

    /// Clone the flushed window only when a 1 s boundary just closed.
    pub fn take_flushed(&mut self) -> Option<LoopStats> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        Some(self.latest)
    }

    fn flush_if_due(&mut self, now_us: u64) {
        if self.current_updates > 0 {
            if let Some(start) = self.last_window_us {
                if now_us.saturating_sub(start) >= WINDOW_US {
                    let pos = self.position_history.last().unwrap_or(0.0);
                    self.flush_window(pos);
                    self.last_window_us = Some(now_us);
                }
            }
        }
    }
}

/// Bus / Modbus link timing window (1 s flush, percentile tables).
pub struct LinkStatsWindow {
    transport: LinkTransport,
    connected: bool,
    reconnects: u32,
    last_window_us: Option<u64>,
    window_successes: u32,
    window_failures: u32,
    round_trip_us: Vec<u16>,
    slave_latency_us: Vec<u16>,
    rx_duration_us: Vec<u16>,
    total_exchanges: u32,
    total_failures: u32,
    total_bytes_tx: u64,
    total_bytes_rx: u64,
    pacing_state: PllState,
    pacing_period_us: u32,
    pacing_delay_us: u32,
    pacing_rtt_min_us: u32,
    latest: LinkStats,
    dirty: bool,
}

impl Default for LinkStatsWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl LinkStatsWindow {
    pub fn new() -> Self {
        Self {
            transport: LinkTransport::None,
            connected: false,
            reconnects: 0,
            last_window_us: None,
            window_successes: 0,
            window_failures: 0,
            round_trip_us: Vec::with_capacity(LINK_SAMPLE_CAP),
            slave_latency_us: Vec::with_capacity(LINK_SAMPLE_CAP),
            rx_duration_us: Vec::with_capacity(LINK_SAMPLE_CAP),
            total_exchanges: 0,
            total_failures: 0,
            total_bytes_tx: 0,
            total_bytes_rx: 0,
            pacing_state: PllState::default(),
            pacing_period_us: 0,
            pacing_delay_us: 0,
            pacing_rtt_min_us: 0,
            latest: LinkStats::default(),
            dirty: false,
        }
    }

    pub fn set_link_info(&mut self, transport: LinkTransport) {
        self.transport = transport;
    }

    pub fn set_pacing(&mut self, info: &PllInfo) {
        self.pacing_state = info.state;
        self.pacing_period_us = info.period_us;
        self.pacing_delay_us = info.delay_us;
        self.pacing_rtt_min_us = info.rtt_min_us;
    }

    pub fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
    }

    pub fn record_reconnect(&mut self) {
        self.reconnects = self.reconnects.saturating_add(1);
    }

    pub fn record_success(
        &mut self,
        now_us: u64,
        round_trip_us: u16,
        slave_latency_us: Option<u16>,
        rx_duration_us: Option<u16>,
        tx_len: u32,
        rx_len: u32,
    ) {
        self.window_successes = self.window_successes.saturating_add(1);
        self.total_exchanges = self.total_exchanges.saturating_add(1);
        self.total_bytes_tx = self.total_bytes_tx.saturating_add(tx_len as u64);
        self.total_bytes_rx = self.total_bytes_rx.saturating_add(rx_len as u64);
        self.push_sample(round_trip_us, slave_latency_us, rx_duration_us);
        self.maybe_flush(now_us);
    }

    pub fn record_failure(&mut self, now_us: u64) {
        self.window_failures = self.window_failures.saturating_add(1);
        self.total_failures = self.total_failures.saturating_add(1);
        self.maybe_flush(now_us);
    }

    fn push_sample(
        &mut self,
        round_trip_us: u16,
        slave_latency_us: Option<u16>,
        rx_duration_us: Option<u16>,
    ) {
        if self.round_trip_us.len() < LINK_SAMPLE_CAP {
            self.round_trip_us.push(round_trip_us);
            if let Some(sl) = slave_latency_us {
                self.slave_latency_us.push(sl);
            }
            if let Some(rx) = rx_duration_us {
                self.rx_duration_us.push(rx);
            }
        }
    }

    fn maybe_flush(&mut self, now_us: u64) {
        if self.last_window_us.is_none() {
            self.last_window_us = Some(now_us);
        }
        if let Some(start) = self.last_window_us {
            if now_us.saturating_sub(start) >= WINDOW_US {
                self.flush_window();
                self.last_window_us = Some(now_us);
            }
        }
    }

    fn flush_window(&mut self) {
        let total = self.window_successes + self.window_failures;
        let success_rate = if total > 0 {
            (self.window_successes as f32 / total as f32) * 100.0
        } else {
            0.0
        };

        self.latest = LinkStats {
            transport: self.transport,
            connected: self.connected,
            exchanges: self.total_exchanges,
            failures: self.total_failures,
            success_rate,
            exchanges_per_sec: self.window_successes + self.window_failures,
            bytes_tx: self.total_bytes_tx,
            bytes_rx: self.total_bytes_rx,
            reconnects: self.reconnects,
            round_trip: compute_timing_stats(self.round_trip_us.as_mut_slice()),
            slave_latency: compute_timing_stats(self.slave_latency_us.as_mut_slice()),
            rx_duration: compute_timing_stats(self.rx_duration_us.as_mut_slice()),
            pacing_state: self.pacing_state,
            pacing_period_us: self.pacing_period_us,
            pacing_delay_us: self.pacing_delay_us,
            rtt_min_us: self.pacing_rtt_min_us,
        };
        self.dirty = true;

        self.window_successes = 0;
        self.window_failures = 0;
        self.round_trip_us.clear();
        self.slave_latency_us.clear();
        self.rx_duration_us.clear();
    }

    pub fn poll(&mut self, now_us: u64) -> LinkStats {
        self.flush_if_due(now_us);
        self.poll_live()
    }

    /// Clone the flushed window only when a 1 s boundary just closed.
    pub fn take_flushed(&mut self) -> Option<LinkStats> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        Some(self.poll_live())
    }

    pub fn total_exchanges(&self) -> u32 {
        self.total_exchanges
    }

    fn poll_live(&self) -> LinkStats {
        let mut out = self.latest;
        out.transport = self.transport;
        out.connected = self.connected;
        out.exchanges = self.total_exchanges;
        out.failures = self.total_failures;
        out.bytes_tx = self.total_bytes_tx;
        out.bytes_rx = self.total_bytes_rx;
        out.reconnects = self.reconnects;
        out.pacing_state = self.pacing_state;
        out.pacing_period_us = self.pacing_period_us;
        out.pacing_delay_us = self.pacing_delay_us;
        out.rtt_min_us = self.pacing_rtt_min_us;
        out
    }

    fn flush_if_due(&mut self, now_us: u64) {
        if self.window_successes > 0 || self.window_failures > 0 {
            if let Some(start) = self.last_window_us {
                if now_us.saturating_sub(start) >= WINDOW_US {
                    self.flush_window();
                    self.last_window_us = Some(now_us);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_stats_percentiles() {
        let mut samples: Vec<u16> = (1..=100).collect();
        let stats = compute_timing_stats(&mut samples);
        assert_eq!(stats.min, 1);
        assert_eq!(stats.max, 100);
        assert_eq!(stats.pct50, 51);
        assert_eq!(stats.mean, 50);
    }

    #[test]
    fn loop_window_flushes_after_one_second() {
        let mut w = LoopStatsWindow::new();
        w.record(0, 3.0, 0.5);
        w.record(500_000, 3.1, 0.51);
        let early = w.poll(500_000);
        assert_eq!(early.ups, 0);

        let flushed = w.poll(1_100_000);
        assert_eq!(flushed.ups, 2);
        assert!(flushed.dt_avg_ms > 0.0);
        assert_eq!(flushed.update_history.len(), 1);
    }

    #[test]
    fn link_window_success_rate() {
        let mut w = LinkStatsWindow::new();
        w.set_link_info(LinkTransport::Serial);
        w.record_success(0, 1000, Some(500), Some(200), 8, 8);
        w.record_failure(100);
        w.record_success(200, 1100, None, None, 8, 8);
        let stats = w.poll(1_100_000);
        assert_eq!(stats.exchanges, 2);
        assert_eq!(stats.failures, 1);
        assert!(stats.success_rate > 0.0);
        assert_eq!(stats.round_trip.min, 1000);
        assert_eq!(stats.transport, LinkTransport::Serial);
    }

    #[test]
    fn take_flushed_is_once_per_window() {
        let mut loop_w = LoopStatsWindow::new();
        loop_w.record(0, 3.0, 0.5);
        assert!(loop_w.take_flushed().is_none());
        loop_w.record(1_000_000, 3.0, 0.5);
        let first = loop_w.take_flushed().expect("window");
        assert_eq!(first.ups, 2);
        assert!(loop_w.take_flushed().is_none());

        let mut link_w = LinkStatsWindow::new();
        link_w.set_link_info(LinkTransport::Uart);
        link_w.record_success(0, 2000, None, None, 8, 8);
        assert!(link_w.take_flushed().is_none());
        link_w.record_success(1_000_000, 2100, None, None, 8, 8);
        let link = link_w.take_flushed().expect("link window");
        assert_eq!(link.transport, LinkTransport::Uart);
        assert_eq!(link.exchanges, 2);
        assert!(link_w.take_flushed().is_none());
    }

    #[test]
    fn copy_from_skips_identical() {
        let mut a = LoopStats {
            ups: 300,
            ..LoopStats::default()
        };
        a.update_history.push(300);
        let b = a;
        a.copy_from(&b);
        assert_eq!(a, b);
    }

    #[test]
    fn transport_wire_names() {
        assert_eq!(LinkTransport::UsbHost.as_str(), "usb-host");
        assert_eq!(LinkTransport::from_wire("rs485-ws"), LinkTransport::Rs485Ws);
        assert_eq!(PllState::ProbeFreq.as_str(), "probe-freq");
    }
}
