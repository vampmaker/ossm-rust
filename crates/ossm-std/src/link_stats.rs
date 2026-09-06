//! Shared link telemetry for ossm-std (engine task + HTTP).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ossm_common::{LinkStats, LinkStatsWindow, LinkTransport, PllInfo};

#[derive(Clone, Default)]
pub struct SharedLinkStats(Arc<Mutex<LinkStatsWindow>>);

impl SharedLinkStats {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(LinkStatsWindow::new())))
    }

    pub fn set_link_info(&self, transport: LinkTransport) {
        if let Ok(mut w) = self.0.lock() {
            w.set_link_info(transport);
            w.set_connected(true);
        }
    }

    pub fn set_connected(&self, connected: bool) {
        if let Ok(mut w) = self.0.lock() {
            w.set_connected(connected);
        }
    }

    pub fn set_pacing(&self, info: PllInfo) {
        if let Ok(mut w) = self.0.lock() {
            w.set_pacing(&info);
        }
    }

    pub fn record_success(
        &self,
        now_us: u64,
        round_trip_us: u16,
        slave_latency_us: Option<u16>,
        rx_duration_us: Option<u16>,
        tx_len: u32,
        rx_len: u32,
    ) {
        if let Ok(mut w) = self.0.lock() {
            w.record_success(
                now_us,
                round_trip_us,
                slave_latency_us,
                rx_duration_us,
                tx_len,
                rx_len,
            );
        }
    }

    pub fn record_failure(&self, now_us: u64) {
        if let Ok(mut w) = self.0.lock() {
            w.record_failure(now_us);
        }
    }

    pub fn poll(&self, now_us: u64) -> LinkStats {
        self.0
            .lock()
            .map(|mut w| w.poll(now_us))
            .unwrap_or_default()
    }

    pub fn take_flushed(&self) -> Option<LinkStats> {
        self.0.lock().ok().and_then(|mut w| w.take_flushed())
    }
}

fn mono_origin() -> Instant {
    static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *ORIGIN.get_or_init(Instant::now)
}

pub fn mono_us() -> u64 {
    Instant::now()
        .saturating_duration_since(mono_origin())
        .as_micros() as u64
}

pub fn mono_deadline(us: u64) -> Instant {
    mono_origin() + Duration::from_micros(us)
}

pub fn instant_to_mono(t: Instant) -> u64 {
    t.saturating_duration_since(mono_origin()).as_micros() as u64
}

pub fn micros_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}
