use alloc::boxed::Box;
use alloc::collections::VecDeque;

use crate::state::{MotionCommand, StreamStatus, StreamWaypoint};
use crate::time::{dt_seconds, seconds_to_micros, Micros};

use super::waveform::WaveformGenerator;

pub(crate) const PAUSE_SPEED: f32 = 0.3;
pub(crate) const TRANSITION_THRESHOLD: f32 = 0.01;

pub(crate) trait MotionSource: Send {
    fn update(&mut self, dt: f32, now: Micros) -> (f32, f32);
    fn follow(&mut self, y: f32, speed: f32, now: Micros);
    fn get_phase_info(&self, now: Micros) -> (f32, f32);
}

// ===== Motion Source Implementations =====

pub(crate) struct WaveformMotionSource {
    generator: Box<dyn WaveformGenerator>,
    bpm: f32,
    t0: Micros,
}

impl WaveformMotionSource {
    pub(crate) fn new(generator: Box<dyn WaveformGenerator>, bpm: f32) -> Self {
        Self {
            generator,
            bpm,
            t0: 0,
        }
    }
}

impl MotionSource for WaveformMotionSource {
    fn update(&mut self, _dt: f32, now: Micros) -> (f32, f32) {
        let elapsed = dt_seconds(self.t0, now);
        self.generator.evaluate(elapsed, self.bpm)
    }

    fn follow(&mut self, y: f32, _speed: f32, now: Micros) {
        // A periodic waveform cannot match an arbitrary velocity at a given
        // position, so only the phase is matched.
        let phase = self.generator.find_x_for_y(y);
        let time_offset = phase * 60.0 / self.bpm;
        self.t0 = now.saturating_sub(seconds_to_micros(time_offset));
    }

    fn get_phase_info(&self, now: Micros) -> (f32, f32) {
        let elapsed = dt_seconds(self.t0, now);
        let cycles = elapsed * self.bpm / 60.0;
        let x = cycles % 1.0;
        (elapsed, x)
    }
}

pub(crate) struct PausedMotionSource {
    current_y: f32,
    target_y: f32,
}

impl PausedMotionSource {
    pub(crate) fn new(current_y: f32, target_y: f32) -> Self {
        Self {
            current_y,
            target_y,
        }
    }

    /// Keep the same physical pose after a reverse rematch (`y := 1-y`).
    pub(crate) fn invert_y(&mut self) {
        self.current_y = 1.0 - self.current_y;
        self.target_y = 1.0 - self.target_y;
    }
}

impl MotionSource for PausedMotionSource {
    fn update(&mut self, dt: f32, _now: Micros) -> (f32, f32) {
        let diff = self.target_y - self.current_y;
        if diff.abs() < TRANSITION_THRESHOLD {
            self.current_y = self.target_y;
            (self.current_y, 0.0)
        } else {
            let step = PAUSE_SPEED * dt;
            let speed = if diff > 0.0 {
                self.current_y = (self.current_y + step).min(self.target_y);
                PAUSE_SPEED
            } else {
                self.current_y = (self.current_y - step).max(self.target_y);
                -PAUSE_SPEED
            };
            (self.current_y, speed)
        }
    }

    fn follow(&mut self, y: f32, _speed: f32, _now: Micros) {
        self.current_y = y;
    }

    fn get_phase_info(&self, _now: Micros) -> (f32, f32) {
        (0.0, 0.0)
    }
}

// ===== Streaming Motion Source =====
// Plays timestamped waypoints streamed from a client, interpolating between
// them with cubic Hermite segments on a logical stream clock.

struct Trajectory {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    duration: f32,
    elapsed: f32,
}

impl Trajectory {
    // Cubic polynomial: y(t) = at^3 + bt^2 + ct + d
    fn new(start_y: f32, start_speed: f32, end_y: f32, end_speed: f32, duration: f32) -> Self {
        if duration <= 1e-6 {
            // Instant move (avoid division by zero)
            return Self {
                a: 0.0,
                b: 0.0,
                c: 0.0,
                d: end_y,
                duration: 0.0,
                elapsed: 0.0,
            };
        }

        // Constraints:
        // y(0) = d = start_y
        // y'(0) = c = start_speed
        // y(T) = aT^3 + bT^2 + cT + d = end_y
        // y'(T) = 3aT^2 + 2bT + c = end_speed

        let d = start_y;
        let c = start_speed;

        let t = duration;
        let t2 = t * t;
        let _t3 = t2 * t;

        // a = (v0 + v1 - 2(y1 - y0)/T) / T^2
        // b = (3(y1 - y0)/T - 2v0 - v1) / T

        let dy = end_y - start_y;
        let a = (start_speed + end_speed - 2.0 * dy / t) / t2;
        let b = (3.0 * dy / t - 2.0 * start_speed - end_speed) / t;

        Self {
            a,
            b,
            c,
            d,
            duration,
            elapsed: 0.0,
        }
    }

    fn update(&mut self, dt: f32) -> Option<(f32, f32)> {
        self.elapsed += dt;
        if self.elapsed >= self.duration {
            // Finished
            return None;
        }

        let t = self.elapsed;
        let t2 = t * t;
        let t3 = t2 * t;

        let y = self.a * t3 + self.b * t2 + self.c * t + self.d;
        let speed = 3.0 * self.a * t2 + 2.0 * self.b * t + self.c;

        Some((y, speed))
    }

    fn end_state(&self) -> (f32, f32) {
        let t = self.duration;
        let t2 = t * t;
        let t3 = t2 * t;
        let y = self.a * t3 + self.b * t2 + self.c * t + self.d;
        let speed = 3.0 * self.a * t2 + 2.0 * self.b * t + self.c;
        (y, speed)
    }
}

#[derive(Clone, Copy)]
struct Waypoint {
    t: f32,           // Playback time on the stream clock (seconds)
    pos: f32,         // y in [0, 1]
    vel: Option<f32>, // y-units/s; estimated from neighbors when absent
}

const MAX_WINDOW: usize = 128;
// Speed used to size the catch-up trajectory toward a newly anchored stream
const CATCHUP_SPEED: f32 = 0.5; // y-units/s
const CATCHUP_MIN_DURATION: f32 = 0.2;
const CATCHUP_MAX_DURATION: f32 = 2.0;
const UNDERRUN_DECEL: f32 = 20.0; // y-units/s^2

enum StreamSample {
    Interpolated(f32, f32),
    BeforeFirst,
    Exhausted,
}

pub(crate) struct StreamingMotionSource {
    // Logical clock: advances only while streaming is the active mode, so
    // pausing the device naturally freezes playback.
    stream_time: f32,
    // Maps sender timestamps to the stream clock: (sender ts ms, playback t)
    epoch: Option<(u64, f32)>,
    // Waypoints sorted by playback time, spanning both sides of stream_time
    // (up to 2 past entries are retained for tangent estimation).
    window: VecDeque<Waypoint>,

    current_y: f32,
    current_speed: f32,
    underrun: bool,

    // Blend from the current state into the stream (mode entry, underrun
    // recovery, reset) with matched position and velocity.
    catchup: Option<Trajectory>,
}

impl StreamingMotionSource {
    pub(crate) fn new(start_y: f32) -> Self {
        Self {
            stream_time: 0.0,
            epoch: None,
            window: VecDeque::new(),
            current_y: start_y,
            current_speed: 0.0,
            underrun: false,
            catchup: None,
        }
    }

    pub(crate) fn status(&self) -> StreamStatus {
        StreamStatus {
            buffered: self.window.len(),
            stream_time: self.stream_time,
            underrun: self.underrun,
        }
    }

    pub(crate) fn apply_stream_command(&mut self, cmd: MotionCommand) {
        match cmd {
            MotionCommand::ResetTimestamp => {
                self.window.clear();
                self.epoch = None;
                self.catchup = None;
                self.stream_time = 0.0;
                self.underrun = false;
            }
            MotionCommand::SetWaypoints {
                waypoints,
                reset_timestamp,
            } => {
                self.window.clear();
                if reset_timestamp {
                    self.epoch = None;
                    self.catchup = None;
                    self.stream_time = 0.0;
                    self.underrun = false;
                }
                for wp in waypoints {
                    self.ingest_waypoint(wp);
                }
            }
            MotionCommand::AppendWaypoints(waypoints) => {
                for wp in waypoints {
                    self.ingest_waypoint(wp);
                }
            }
            MotionCommand::SetConfig(_) => {
                // Applied by MotorController::apply_motion; unreachable here.
            }
        }
    }

    fn ingest_waypoint(&mut self, wp: StreamWaypoint) {
        let ts = wp.ts;
        let pos = wp.pos.clamp(0.0, 1.0);
        let vel = wp.vel;
        let t = match self.epoch {
            Some((epoch_ts, epoch_t)) => {
                if ts < epoch_ts {
                    return; // Stale: predates the current epoch
                }
                epoch_t + (ts - epoch_ts) as f32 / 1000.0
            }
            None => {
                // Anchor the stream: schedule the first waypoint far
                // enough in the future to reach it smoothly.
                let duration = ((pos - self.current_y).abs() / CATCHUP_SPEED)
                    .clamp(CATCHUP_MIN_DURATION, CATCHUP_MAX_DURATION);
                let t0 = self.stream_time + duration;
                self.epoch = Some((ts, t0));
                self.underrun = false;
                t0
            }
        };
        if self.window.len() >= MAX_WINDOW {
            log::warn!("Stream window full, dropping waypoint ts={}", ts);
            return;
        }
        // Insert sorted by playback time (commands normally arrive in order)
        let idx = self
            .window
            .iter()
            .rposition(|w| w.t <= t)
            .map_or(0, |i| i + 1);
        self.window.insert(idx, Waypoint { t, pos, vel });
    }

    // Drop past waypoints, keeping the 2 most recent ones behind stream_time
    // so segment tangents can still be estimated.
    fn evict_old(&mut self) {
        while self.window.len() > 2 && self.window[2].t < self.stream_time {
            self.window.pop_front();
        }
    }

    // Velocity at waypoint i: explicit if provided, else a Catmull-Rom style
    // finite difference over the neighbors (one-sided at the head/tail).
    fn tangent_at(&self, i: usize) -> f32 {
        let w = &self.window;
        if let Some(v) = w[i].vel {
            return v;
        }
        let n = w.len();
        if n < 2 {
            return 0.0;
        }
        let (a, b) = if i == 0 {
            (w[0], w[1])
        } else if i == n - 1 {
            (w[n - 2], w[n - 1])
        } else {
            (w[i - 1], w[i + 1])
        };
        let dt = b.t - a.t;
        if dt > 1e-4 {
            (b.pos - a.pos) / dt
        } else {
            0.0
        }
    }

    fn sample(&self, t: f32) -> StreamSample {
        let n = self.window.len();
        if n == 0 {
            return StreamSample::Exhausted;
        }
        if t < self.window[0].t {
            return StreamSample::BeforeFirst;
        }
        // Find segment [i, i+1] bracketing t
        let mut i = 0;
        while i + 1 < n && self.window[i + 1].t < t {
            i += 1;
        }
        if i + 1 >= n {
            return StreamSample::Exhausted;
        }

        let w1 = self.window[i];
        let w2 = self.window[i + 1];
        let h = (w2.t - w1.t).max(1e-4);
        let u = ((t - w1.t) / h).clamp(0.0, 1.0);
        // Tangents are dy/dt; scale by segment duration for the unit-domain Hermite basis
        let m1 = self.tangent_at(i) * h;
        let m2 = self.tangent_at(i + 1) * h;

        let u2 = u * u;
        let u3 = u2 * u;
        let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
        let h10 = u3 - 2.0 * u2 + u;
        let h01 = -2.0 * u3 + 3.0 * u2;
        let h11 = u3 - u2;
        let y = h00 * w1.pos + h10 * m1 + h01 * w2.pos + h11 * m2;

        let dh00 = 6.0 * u2 - 6.0 * u;
        let dh10 = 3.0 * u2 - 4.0 * u + 1.0;
        let dh01 = -6.0 * u2 + 6.0 * u;
        let dh11 = 3.0 * u2 - 2.0 * u;
        let dy_du = dh00 * w1.pos + dh10 * m1 + dh01 * w2.pos + dh11 * m2;

        StreamSample::Interpolated(y, dy_du / h)
    }

    fn decelerate(&mut self, dt: f32) {
        if self.current_speed.abs() > 1e-4 {
            let decel = UNDERRUN_DECEL * dt;
            if self.current_speed > 0.0 {
                self.current_speed = (self.current_speed - decel).max(0.0);
            } else {
                self.current_speed = (self.current_speed + decel).min(0.0);
            }
            self.current_y = (self.current_y + self.current_speed * dt).clamp(0.0, 1.0);
        } else {
            self.current_speed = 0.0;
        }
    }
}

impl MotionSource for StreamingMotionSource {
    fn update(&mut self, dt: f32, _now: Micros) -> (f32, f32) {
        self.evict_old();

        // Start a catch-up trajectory toward the first upcoming waypoint if
        // playback has not reached the stream yet (mode entry / re-anchor).
        if self.catchup.is_none() {
            if let Some(first) = self.window.front() {
                if self.stream_time < first.t {
                    let duration = first.t - self.stream_time;
                    let target_vel = self.tangent_at(0);
                    self.catchup = Some(Trajectory::new(
                        self.current_y,
                        self.current_speed,
                        first.pos,
                        target_vel,
                        duration,
                    ));
                }
            }
        }

        self.stream_time += dt;

        if let Some(traj) = &mut self.catchup {
            match traj.update(dt) {
                Some((y, speed)) => {
                    self.current_y = y.clamp(0.0, 1.0);
                    self.current_speed = speed;
                    return (self.current_y, self.current_speed);
                }
                None => {
                    let (end_y, end_speed) = traj.end_state();
                    self.current_y = end_y.clamp(0.0, 1.0);
                    self.current_speed = end_speed;
                    self.catchup = None;
                    // Fall through to interpolation for the current time
                }
            }
        }

        match self.sample(self.stream_time) {
            StreamSample::Interpolated(y, speed) => {
                self.underrun = false;
                self.current_y = y.clamp(0.0, 1.0);
                self.current_speed = speed;
            }
            StreamSample::BeforeFirst => {
                // Waiting for the catch-up to be (re)built next cycle; hold
                self.decelerate(dt);
            }
            StreamSample::Exhausted => {
                // Ran past the last waypoint: drop the epoch so the next
                // waypoint re-anchors the clock, then decelerate and hold.
                if !self.underrun {
                    self.underrun = true;
                    self.epoch = None;
                    self.window.clear();
                }
                self.decelerate(dt);
            }
        }

        (self.current_y, self.current_speed)
    }

    fn follow(&mut self, y: f32, speed: f32, _now: Micros) {
        self.window.clear();
        self.epoch = None;
        self.catchup = None;
        self.underrun = false;
        self.current_y = y;
        self.current_speed = speed;
    }

    fn get_phase_info(&self, _now: Micros) -> (f32, f32) {
        (self.stream_time, 0.0) // No phase concept in streaming
    }
}
