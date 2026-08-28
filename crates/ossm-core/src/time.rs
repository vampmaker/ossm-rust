//! Monotonic microsecond timestamps supplied by the device shell.

pub type Micros = u64;

/// Elapsed seconds between two timestamps. Saturating; never negative.
pub fn dt_seconds(prev: Micros, now: Micros) -> f32 {
    now.saturating_sub(prev) as f32 / 1_000_000.0
}

pub fn seconds_to_micros(seconds: f32) -> Micros {
    if seconds <= 0.0 {
        0
    } else {
        (seconds * 1_000_000.0) as Micros
    }
}
