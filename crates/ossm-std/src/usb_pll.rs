//! f32 PID lock of TX cadence to the host USB 1 ms poll.
//!
//! The host polls Full-Speed interrupt/bulk endpoints on a 1 ms grid, but that
//! grid's phase versus `Instant` is unknown. `t_rx` is userspace CRC-complete
//! (SOF + kernel + tokio), not a SOF timestamp — do not treat it as 1 ms-aligned.

use std::f32::consts::PI;
use std::time::{Duration, Instant};

const USB_FRAME_US: f32 = 1000.0;
const MIN_PERIOD_US: f32 = 2_000.0;
const MAX_PERIOD_US: f32 = 8_000.0;
const EWMA_ALPHA: f32 = 0.2;
const KP: f32 = 0.35;
const KI: f32 = 0.04;
const KD: f32 = 0.08;
const I_CLAMP: f32 = 800.0;
const TARGET_ERR_US: f32 = -300.0;
const WRITE_LEAD_INIT_US: f32 = 250.0;
const WRITE_LEAD_MIN_US: f32 = 50.0;
const WRITE_LEAD_MAX_US: f32 = 2_000.0;
const PHASE_LOCK_MAG: f32 = 0.45;

pub struct FramePll {
    origin: Instant,
    rtt_ewma_us: f32,
    period_us: f32,
    integral_us: f32,
    prev_err_us: f32,
    last_err_us: f32,
    write_lead_us: f32,
    phase_cos: f32,
    phase_sin: f32,
    phase_ready: bool,
    last_wake: Option<Instant>,
    aim_avail: Option<Instant>,
    next_tx: Option<Instant>,
    cached_u: f32,
}

impl Default for FramePll {
    fn default() -> Self {
        Self::new()
    }
}

impl FramePll {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
            rtt_ewma_us: 3_000.0,
            period_us: 3_000.0,
            integral_us: 0.0,
            prev_err_us: 0.0,
            last_err_us: 0.0,
            write_lead_us: WRITE_LEAD_INIT_US,
            phase_cos: 0.0,
            phase_sin: 0.0,
            phase_ready: false,
            last_wake: None,
            aim_avail: None,
            next_tx: None,
            cached_u: 0.0,
        }
    }

    pub fn period_us(&self) -> f32 {
        self.period_us
    }

    pub fn psi_us(&self) -> f32 {
        if !self.phase_ready {
            return 0.0;
        }
        let ang = self.phase_sin.atan2(self.phase_cos);
        (ang / (2.0 * PI) * USB_FRAME_US).rem_euclid(USB_FRAME_US)
    }

    pub fn last_err_us(&self) -> f32 {
        self.last_err_us
    }

    pub fn integral_us(&self) -> f32 {
        self.integral_us
    }

    pub fn write_lead_us(&self) -> f32 {
        self.write_lead_us
    }

    pub fn rtt_ewma_us(&self) -> f32 {
        self.rtt_ewma_us
    }

    pub fn phase_mag(&self) -> f32 {
        self.phase_cos.hypot(self.phase_sin)
    }

    /// Sleep until the next TX slot. Overrun (now past `next_tx`) returns immediately.
    pub async fn wait_tx(&mut self) {
        let Some(deadline) = self.next_tx else {
            tokio::task::yield_now().await;
            self.last_wake = Some(Instant::now());
            return;
        };
        let now = Instant::now();
        if deadline > now {
            tokio::time::sleep(deadline.saturating_duration_since(now)).await;
        }
        self.last_wake = Some(Instant::now());
    }

    pub fn on_done(&mut self, t_send: Instant, t_rx: Option<Instant>) {
        self.update_write_lead(t_send);

        if let Some(t_rx) = t_rx {
            let rtt = signed_us(t_rx, t_send);
            if rtt > 0.0 && rtt.is_finite() {
                self.rtt_ewma_us = EWMA_ALPHA * rtt + (1.0 - EWMA_ALPHA) * self.rtt_ewma_us;
                self.period_us = self.rtt_ewma_us.clamp(MIN_PERIOD_US, MAX_PERIOD_US);
            }
            self.observe_phase(t_rx);
            if self.phase_mag() >= PHASE_LOCK_MAG {
                self.update_pid(t_send);
                let t_avail = self.next_poll_at_or_after(t_rx);
                self.aim_avail = Some(t_avail);
                self.next_tx = Some(self.wake_for(t_avail));
            } else {
                self.aim_avail = None;
                self.next_tx = Some(t_rx);
            }
        } else {
            self.aim_avail = None;
            self.next_tx = Some(Instant::now());
        }
    }

    fn update_write_lead(&mut self, t_send: Instant) {
        let Some(wake) = self.last_wake else {
            return;
        };
        let lead = signed_us(t_send, wake);
        if lead <= 0.0 || !lead.is_finite() {
            return;
        }
        self.write_lead_us = (EWMA_ALPHA * lead + (1.0 - EWMA_ALPHA) * self.write_lead_us)
            .clamp(WRITE_LEAD_MIN_US, WRITE_LEAD_MAX_US);
    }

    fn update_pid(&mut self, t_send: Instant) {
        let t_avail = match self.aim_avail {
            Some(t) => t,
            None => self.nearest_poll(t_send),
        };
        let e = signed_us(t_send, t_avail);
        self.last_err_us = e;
        let e_ctrl = e - TARGET_ERR_US;
        self.integral_us = (self.integral_us + KI * e_ctrl).clamp(-I_CLAMP, I_CLAMP);
        let deriv = e_ctrl - self.prev_err_us;
        self.prev_err_us = e_ctrl;
        self.cached_u = KP * e_ctrl + self.integral_us + KD * deriv;
    }

    fn wake_for(&self, t_avail: Instant) -> Instant {
        offset_us(t_avail, -(self.write_lead_us + self.cached_u))
    }

    fn observe_phase(&mut self, t: Instant) {
        let frac = self.frac_us(t);
        let theta = frac / USB_FRAME_US * 2.0 * PI;
        let (s, c) = theta.sin_cos();
        if !self.phase_ready {
            self.phase_cos = c;
            self.phase_sin = s;
            self.phase_ready = true;
            return;
        }
        self.phase_cos = EWMA_ALPHA * c + (1.0 - EWMA_ALPHA) * self.phase_cos;
        self.phase_sin = EWMA_ALPHA * s + (1.0 - EWMA_ALPHA) * self.phase_sin;
    }

    fn frac_us(&self, t: Instant) -> f32 {
        (self.elapsed_us(t) as f32).rem_euclid(USB_FRAME_US)
    }

    fn elapsed_us(&self, t: Instant) -> f64 {
        t.saturating_duration_since(self.origin).as_nanos() as f64 / 1_000.0
    }

    fn instant_from_elapsed_us(&self, us: f64) -> Instant {
        if us <= 0.0 {
            self.origin
        } else {
            self.origin + Duration::from_nanos((us * 1_000.0) as u64)
        }
    }

    fn next_poll_at_or_after(&self, t: Instant) -> Instant {
        let psi = self.psi_us() as f64;
        let elapsed = self.elapsed_us(t);
        let frame_start = (elapsed / USB_FRAME_US as f64).floor() * USB_FRAME_US as f64;
        let mut candidate = frame_start + psi;
        if candidate + 0.5 < elapsed {
            candidate += USB_FRAME_US as f64;
        }
        self.instant_from_elapsed_us(candidate)
    }

    fn nearest_poll(&self, t: Instant) -> Instant {
        let next = self.next_poll_at_or_after(t);
        let prev = offset_us(next, -USB_FRAME_US);
        let d_next = signed_us(next, t).abs();
        let d_prev = signed_us(t, prev).abs();
        if d_prev < d_next {
            prev
        } else {
            next
        }
    }
}

fn signed_us(later: Instant, earlier: Instant) -> f32 {
    if later >= earlier {
        later.duration_since(earlier).as_nanos() as f32 / 1_000.0
    } else {
        -(earlier.duration_since(later).as_nanos() as f32 / 1_000.0)
    }
}

fn offset_us(t: Instant, us: f32) -> Instant {
    if !us.is_finite() {
        return t;
    }
    if us >= 0.0 {
        t + Duration::from_nanos((us * 1_000.0) as u64)
    } else {
        t.checked_sub(Duration::from_nanos(((-us) * 1_000.0) as u64))
            .unwrap_or(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_phase_ewma_converges_near_zero_not_mid() {
        let mut pll = FramePll::new();
        let origin = pll.origin;
        for i in 0..80 {
            pll.observe_phase(origin + Duration::from_micros(i * 2_000 + 900));
            pll.observe_phase(origin + Duration::from_micros(i * 2_000 + 2_100));
        }
        let psi = pll.psi_us();
        assert!(
            psi < 80.0 || psi > 920.0,
            "wrapped 900/100 samples should sit near 0, got {psi}"
        );
        assert!(
            (psi - 500.0).abs() > 200.0,
            "must not average wrap to 500, got {psi}"
        );
    }

    #[test]
    fn late_t_send_pulls_next_wake_earlier() {
        let mut on_time = seeded_phase_pll();
        let mut late = seeded_phase_pll();
        let origin = on_time.origin;
        let t_avail = on_time.next_poll_at_or_after(origin + Duration::from_millis(40));
        on_time.aim_avail = Some(t_avail);
        late.aim_avail = Some(t_avail);
        let t_rx = t_avail + Duration::from_micros(5_000);

        on_time.on_done(t_avail, Some(t_rx));
        late.on_done(t_avail + Duration::from_micros(400), Some(t_rx));

        let early_wake = on_time.next_tx.expect("on-time next");
        let late_wake = late.next_tx.expect("late next");
        assert!(
            late_wake < early_wake,
            "late t_send should pull wake earlier ({late_wake:?} vs {early_wake:?})"
        );
    }

    #[test]
    fn period_tracks_rtt_without_integer_ceil() {
        let mut pll = FramePll::new();
        let t_send = pll.origin + Duration::from_millis(10);
        let t_rx = t_send + Duration::from_micros(2_200);
        pll.on_done(t_send, Some(t_rx));
        let period = pll.period_us();
        assert!(
            (period - 2_840.0).abs() < 2.0,
            "0.2*2200 + 0.8*3000 = 2840, not 3000; got {period}"
        );
        assert_ne!(period, 3_000.0);
    }

    #[tokio::test]
    async fn overrun_does_not_sleep() {
        let mut pll = FramePll::new();
        let past = Instant::now() - Duration::from_millis(5);
        pll.next_tx = Some(past);
        let start = Instant::now();
        pll.wait_tx().await;
        assert!(start.elapsed() < Duration::from_millis(2));
    }

    #[test]
    fn integral_clamps() {
        let mut pll = seeded_phase_pll();
        let origin = pll.origin;
        let t_avail = origin + Duration::from_millis(50);
        pll.aim_avail = Some(t_avail);
        for _ in 0..200 {
            pll.update_pid(t_avail + Duration::from_millis(10));
        }
        assert!(pll.integral_us() <= I_CLAMP);
        assert!(pll.integral_us() >= -I_CLAMP);
        assert_eq!(pll.integral_us(), I_CLAMP);
    }

    #[test]
    fn crc_miss_schedules_immediate_next_tx() {
        let mut pll = FramePll::new();
        let t_send = Instant::now();
        pll.on_done(t_send, None);
        let next = pll.next_tx.expect("next");
        assert!(next <= Instant::now() + Duration::from_millis(1));
        let delay = next.saturating_duration_since(t_send);
        assert!(
            delay < Duration::from_millis(2),
            "CRC miss must not wait period, delay={delay:?}"
        );
    }

    fn seeded_phase_pll() -> FramePll {
        let mut pll = FramePll::new();
        let origin = pll.origin;
        for i in 1..=40 {
            pll.observe_phase(origin + Duration::from_millis(i));
        }
        pll.last_wake = Some(origin);
        pll
    }
}
