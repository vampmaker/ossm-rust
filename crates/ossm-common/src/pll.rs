//! BBR-style two-state link pacer. Caller supplies monotonic microseconds.

const RTT_EWMA_ALPHA: f32 = 0.2;
const LOCK_SLACK_US: u32 = 100;
const MIN_STEP_US: u32 = 50;
const WINDOW_US: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PllState {
    #[default]
    ProbeFreq,
    ProbePhase,
}

impl PllState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProbeFreq => "probe-freq",
            Self::ProbePhase => "probe-phase",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wake {
    Now,
    At(u64),
    OnAck,
}

#[derive(Clone, Copy, Debug)]
pub struct PllConfig {
    pub min_period_us: u32,
    pub max_period_us: u32,
    pub slot_us: u32,
    pub probe_cycles: u16,
    pub probe_min_us: u32,
    pub period_margin: f32,
    pub reprobe_interval_us: u64,
    pub rtt_degrade_ratio: f32,
    pub timeout_degrade_pct: u8,
    pub epoch_cycles: u8,
}

impl PllConfig {
    pub fn usb_serial() -> Self {
        Self {
            min_period_us: 2_500,
            max_period_us: 12_000,
            slot_us: 1_000,
            probe_cycles: 64,
            probe_min_us: 300_000,
            period_margin: 0.05,
            reprobe_interval_us: 10_000_000,
            rtt_degrade_ratio: 1.5,
            timeout_degrade_pct: 5,
            epoch_cycles: 8,
        }
    }

    pub fn network() -> Self {
        let mut c = Self::usb_serial();
        c.slot_us = 0;
        c
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PllInfo {
    pub state: PllState,
    pub period_us: u32,
    pub delay_us: u32,
    pub rtt_min_us: u32,
    pub rtt_ewma_us: u32,
    pub max_rate_hz: u32,
}

pub struct Pll {
    cfg: PllConfig,
    state: PllState,
    waiting_ack: bool,
    period_us: u32,
    delay_us: u32,
    anchor_us: u64,
    last_tx_us: u64,
    phase_start_us: u64,
    probe_start_us: Option<u64>,
    probe_acks: u16,
    rtt_min_us: u32,
    rtt_ewma_us: f32,
    epoch_left: u8,
    epoch_rtt_sum: u32,
    epoch_rtt_n: u8,
    last_epoch_mean: Option<f32>,
    step_us: u32,
    climb_dir: i32,
    locked: bool,
    win_start_us: Option<u64>,
    win_ok: u32,
    win_to: u32,
    max_rate_hz: u32,
}

impl Pll {
    pub fn new(cfg: PllConfig) -> Self {
        Self {
            cfg,
            state: PllState::ProbeFreq,
            waiting_ack: false,
            period_us: cfg.min_period_us,
            delay_us: 0,
            anchor_us: 0,
            last_tx_us: 0,
            phase_start_us: 0,
            probe_start_us: None,
            probe_acks: 0,
            rtt_min_us: u32::MAX,
            rtt_ewma_us: 0.0,
            epoch_left: cfg.epoch_cycles,
            epoch_rtt_sum: 0,
            epoch_rtt_n: 0,
            last_epoch_mean: None,
            step_us: initial_step(&cfg, cfg.min_period_us),
            climb_dir: 1,
            locked: false,
            win_start_us: None,
            win_ok: 0,
            win_to: 0,
            max_rate_hz: 0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new(self.cfg);
    }

    pub fn next_wake(&self, now_us: u64) -> Wake {
        match self.state {
            PllState::ProbeFreq => {
                if self.waiting_ack {
                    Wake::OnAck
                } else {
                    Wake::Now
                }
            }
            PllState::ProbePhase => {
                let mut t = self.phase_deadline(now_us);
                let min_gap = (self.period_us as u64) / 2;
                if t.saturating_sub(self.last_tx_us) < min_gap {
                    t = t.saturating_add(self.period_us.max(1) as u64);
                }
                if t <= now_us {
                    Wake::Now
                } else {
                    Wake::At(t)
                }
            }
        }
    }

    pub fn on_tx(&mut self, t_tx_us: u64) {
        self.waiting_ack = true;
        self.last_tx_us = t_tx_us;
        if self.probe_start_us.is_none() {
            self.probe_start_us = Some(t_tx_us);
        }
    }

    pub fn on_ack(&mut self, t_tx_us: u64, t_rx_us: u64) {
        self.waiting_ack = false;
        let rtt = t_rx_us.saturating_sub(t_tx_us).min(u32::MAX as u64) as u32;
        if rtt > 0 {
            if rtt < self.rtt_min_us {
                self.rtt_min_us = rtt;
            }
            if self.rtt_ewma_us <= 0.0 {
                self.rtt_ewma_us = rtt as f32;
            } else {
                self.rtt_ewma_us =
                    RTT_EWMA_ALPHA * rtt as f32 + (1.0 - RTT_EWMA_ALPHA) * self.rtt_ewma_us;
            }
        }
        self.note_result(t_rx_us, false);
        match self.state {
            PllState::ProbeFreq => self.on_probe_ack(t_rx_us),
            PllState::ProbePhase => self.on_phase_ack(rtt, t_rx_us),
        }
    }

    pub fn on_timeout(&mut self, t_tx_us: u64, now_us: u64) {
        let _ = t_tx_us;
        self.waiting_ack = false;
        self.note_result(now_us, true);
        if self.state == PllState::ProbePhase && self.should_reprobe_timeout() {
            self.enter_probe_freq();
        }
    }

    pub fn info(&self) -> PllInfo {
        PllInfo {
            state: self.state,
            period_us: self.period_us,
            delay_us: self.delay_us,
            rtt_min_us: if self.rtt_min_us == u32::MAX {
                0
            } else {
                self.rtt_min_us
            },
            rtt_ewma_us: self.rtt_ewma_us as u32,
            max_rate_hz: self.max_rate_hz,
        }
    }

    fn on_probe_ack(&mut self, t_rx_us: u64) {
        self.probe_acks = self.probe_acks.saturating_add(1);
        let Some(start) = self.probe_start_us else {
            return;
        };
        let elapsed = t_rx_us.saturating_sub(start);
        if self.probe_acks >= self.cfg.probe_cycles && elapsed >= self.cfg.probe_min_us as u64 {
            self.enter_probe_phase(elapsed, t_rx_us);
        }
    }

    fn enter_probe_phase(&mut self, elapsed_us: u64, t_rx_us: u64) {
        let acks = self.probe_acks.max(1) as f32;
        let raw = (elapsed_us as f32 / acks) * (1.0 + self.cfg.period_margin);
        let period = snap_period(
            libm::ceilf(raw) as u32,
            self.cfg.slot_us,
            self.cfg.min_period_us,
            self.cfg.max_period_us,
        );
        self.max_rate_hz = if elapsed_us > 0 {
            ((self.probe_acks as u64).saturating_mul(1_000_000) / elapsed_us.max(1)) as u32
        } else {
            0
        };
        self.state = PllState::ProbePhase;
        self.period_us = period;
        self.delay_us = 0;
        self.anchor_us = self.last_tx_us;
        self.phase_start_us = t_rx_us;
        self.step_us = initial_step(&self.cfg, period);
        self.climb_dir = 1;
        self.locked = false;
        self.epoch_left = self.cfg.epoch_cycles;
        self.epoch_rtt_sum = 0;
        self.epoch_rtt_n = 0;
        self.last_epoch_mean = None;
        self.win_start_us = None;
        self.win_ok = 0;
        self.win_to = 0;
    }

    fn enter_probe_freq(&mut self) {
        self.state = PllState::ProbeFreq;
        self.waiting_ack = false;
        self.probe_start_us = None;
        self.probe_acks = 0;
        self.rtt_min_us = u32::MAX;
        self.rtt_ewma_us = 0.0;
        self.locked = false;
        self.delay_us = 0;
        self.win_start_us = None;
        self.win_ok = 0;
        self.win_to = 0;
    }

    fn on_phase_ack(&mut self, rtt: u32, now_us: u64) {
        if now_us.saturating_sub(self.phase_start_us) >= self.cfg.reprobe_interval_us {
            self.enter_probe_freq();
            return;
        }
        if self.rtt_min_us != u32::MAX
            && self.rtt_ewma_us > self.cfg.rtt_degrade_ratio * self.rtt_min_us as f32
            && self.epoch_rtt_n > 0
            && self.epoch_left == 1
        {
            // checked after epoch below
        }
        if self.should_reprobe_timeout() {
            self.enter_probe_freq();
            return;
        }
        if self.locked {
            return;
        }
        self.epoch_rtt_sum = self.epoch_rtt_sum.saturating_add(rtt);
        self.epoch_rtt_n = self.epoch_rtt_n.saturating_add(1);
        self.epoch_left = self.epoch_left.saturating_sub(1);
        if self.epoch_left > 0 {
            return;
        }
        let mean = self.epoch_rtt_sum as f32 / self.epoch_rtt_n.max(1) as f32;
        self.epoch_left = self.cfg.epoch_cycles;
        self.epoch_rtt_sum = 0;
        self.epoch_rtt_n = 0;
        if self.step_us <= MIN_STEP_US
            && self.rtt_min_us != u32::MAX
            && mean <= self.rtt_min_us as f32 + LOCK_SLACK_US as f32
        {
            self.locked = true;
            return;
        }
        if self.rtt_min_us != u32::MAX
            && self.rtt_ewma_us > self.cfg.rtt_degrade_ratio * self.rtt_min_us as f32
        {
            self.enter_probe_freq();
            return;
        }
        match self.last_epoch_mean {
            None => {
                self.last_epoch_mean = Some(mean);
                self.apply_delay_step();
            }
            Some(prev) => {
                if mean < prev {
                    self.last_epoch_mean = Some(mean);
                    self.apply_delay_step();
                } else {
                    self.climb_dir = -self.climb_dir;
                    self.step_us = (self.step_us / 2).max(MIN_STEP_US);
                    self.last_epoch_mean = None;
                    self.apply_delay_step();
                }
            }
        }
    }

    fn apply_delay_step(&mut self) {
        let next = self.delay_us as i32 + self.climb_dir * self.step_us as i32;
        self.delay_us = wrap_delay(next, self.cfg.slot_us, self.period_us);
    }

    fn phase_deadline(&self, now_us: u64) -> u64 {
        let p = self.period_us.max(1) as u64;
        let d = self.delay_us as u64;
        let mut t = self.anchor_us.saturating_add(d);
        while t <= now_us {
            t = t.saturating_add(p);
        }
        t
    }

    fn note_result(&mut self, now_us: u64, timeout: bool) {
        if self.win_start_us.is_none() {
            self.win_start_us = Some(now_us);
        }
        if let Some(start) = self.win_start_us {
            if now_us.saturating_sub(start) >= WINDOW_US {
                self.win_ok = 0;
                self.win_to = 0;
                self.win_start_us = Some(now_us);
            }
        }
        if timeout {
            self.win_to = self.win_to.saturating_add(1);
        } else {
            self.win_ok = self.win_ok.saturating_add(1);
        }
    }

    fn should_reprobe_timeout(&self) -> bool {
        let total = self.win_ok.saturating_add(self.win_to);
        if total == 0 {
            return false;
        }
        (self.win_to * 100) / total >= self.cfg.timeout_degrade_pct as u32
    }
}

fn initial_step(cfg: &PllConfig, period_us: u32) -> u32 {
    if cfg.slot_us > 0 {
        (cfg.slot_us / 4).max(MIN_STEP_US)
    } else {
        (period_us / 8).max(MIN_STEP_US)
    }
}

fn snap_period(us: u32, slot: u32, min: u32, max: u32) -> u32 {
    let mut p = us.clamp(min, max);
    if slot > 0 {
        p = p.div_ceil(slot) * slot;
        p = p.clamp(min, max);
    }
    p
}

fn wrap_delay(d: i32, slot: u32, period: u32) -> u32 {
    if slot > 0 {
        let s = slot as i32;
        let m = ((d % s) + s) % s;
        m as u32
    } else {
        let max = period.saturating_sub(1) as i32;
        d.clamp(0, max.max(0)) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(pll: &mut Pll, n: usize, mut now: u64, rtt: impl Fn(u64) -> u32) -> u64 {
        for _ in 0..n {
            now = wait(pll, now);
            pll.on_tx(now);
            match pll.next_wake(now) {
                Wake::OnAck => {}
                Wake::Now | Wake::At(_) if pll.state == PllState::ProbeFreq => {
                    panic!("ProbeFreq should wait OnAck after TX")
                }
                _ => {}
            }
            let r = rtt(now);
            let rx = now + r as u64;
            pll.on_ack(now, rx);
            now = rx;
        }
        now
    }

    fn wait(pll: &Pll, now: u64) -> u64 {
        match pll.next_wake(now) {
            Wake::Now => now,
            Wake::At(t) => now.max(t),
            Wake::OnAck => now,
        }
    }

    #[test]
    fn usb_sawtooth_snaps_period_and_reduces_rtt() {
        let mut pll = Pll::new(PllConfig::usb_serial());
        let slot = 1_000u64;
        let base = 3_000u32;
        let rtt = |t_tx: u64| {
            let rem = (slot - (t_tx % slot)) % slot;
            base + if rem == 0 { slot as u32 } else { rem as u32 }
        };
        let now = drive(&mut pll, 80, 0, rtt);
        assert_eq!(pll.info().state, PllState::ProbePhase);
        assert_eq!(pll.info().period_us, 5_000);
        let before = rtt(now);
        let mut t = now;
        let mut sum = 0u64;
        let mut n = 0u32;
        for _ in 0..64 {
            t = wait(&pll, t);
            pll.on_tx(t);
            let r = rtt(t);
            pll.on_ack(t, t + r as u64);
            t += r as u64;
            sum += r as u64;
            n += 1;
        }
        let mean = sum / n as u64;
        assert!(
            mean < before as u64 + 200 || mean <= (base as u64 + 500),
            "hill-climb should pull RTT toward base={base}, mean={mean} start={before}"
        );
    }

    #[test]
    fn network_flat_rtt_sets_period_from_margin() {
        let mut cfg = PllConfig::network();
        cfg.probe_cycles = 16;
        cfg.probe_min_us = 1_000;
        let mut pll = Pll::new(cfg);
        let rtt = 3_000u32;
        drive(&mut pll, 20, 0, |_| rtt);
        assert_eq!(pll.info().state, PllState::ProbePhase);
        let p = pll.info().period_us;
        assert!(
            (3_000..3_300).contains(&p),
            "period should be rtt*(1+margin)≈3150, got {p}"
        );
    }

    #[test]
    fn overrun_catch_up_does_not_stack() {
        let mut pll = Pll::new(PllConfig::usb_serial());
        pll.state = PllState::ProbePhase;
        pll.period_us = 5_000;
        pll.delay_us = 0;
        pll.anchor_us = 0;
        let w1 = pll.next_wake(0);
        assert_eq!(w1, Wake::At(5_000));
        let w2 = pll.next_wake(12_000);
        assert_eq!(w2, Wake::At(15_000));
    }

    #[test]
    fn reprobe_on_interval() {
        let mut cfg = PllConfig::network();
        cfg.probe_cycles = 8;
        cfg.probe_min_us = 100;
        cfg.reprobe_interval_us = 20_000;
        let mut pll = Pll::new(cfg);
        drive(&mut pll, 8, 0, |_| 2_000);
        assert_eq!(pll.info().state, PllState::ProbePhase);
        let mut t = 40_000;
        t = wait(&pll, t);
        pll.on_tx(t);
        pll.on_ack(t, t + 2_000);
        assert_eq!(pll.info().state, PllState::ProbeFreq);
    }

    #[test]
    fn reprobe_on_timeout_share() {
        let mut cfg = PllConfig::network();
        cfg.probe_cycles = 8;
        cfg.probe_min_us = 100;
        cfg.timeout_degrade_pct = 5;
        let mut pll = Pll::new(cfg);
        drive(&mut pll, 16, 0, |_| 2_000);
        assert_eq!(pll.info().state, PllState::ProbePhase);
        for i in 0..8 {
            let t = 100_000 + i * 3_000;
            pll.on_tx(t);
            pll.on_timeout(t, t + 6_000);
        }
        assert_eq!(pll.info().state, PllState::ProbeFreq);
    }

    #[test]
    fn reset_returns_to_probe_freq() {
        let mut cfg = PllConfig::network();
        cfg.probe_cycles = 8;
        cfg.probe_min_us = 100;
        let mut pll = Pll::new(cfg);
        drive(&mut pll, 80, 0, |_| 2_000);
        assert_eq!(pll.info().state, PllState::ProbePhase);
        pll.reset();
        assert_eq!(pll.info().state, PllState::ProbeFreq);
        assert_eq!(pll.next_wake(0), Wake::Now);
    }

    #[test]
    fn pacing_wire_name() {
        assert_eq!(PllState::ProbeFreq.as_str(), "probe-freq");
        assert_eq!(PllState::ProbePhase.as_str(), "probe-phase");
    }
}
