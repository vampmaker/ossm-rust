extern crate alloc;

use alloc::vec;

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::boxed::Box;

use embassy_time::{self, Duration, Instant};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use heapless::spsc::{Producer, Consumer};

pub const COMMAND_QUEUE_SIZE: usize = 3;
pub type CommandProducer = Producer<'static, MotionCommand>;
pub type CommandConsumer = Consumer<'static, MotionCommand>;

// ===== Layer 1: Waveform Generator =====
// Generates y ∈ [0, 1] given time, handles BPM internally

trait WaveformGenerator: Send {
    fn evaluate(&self, time_offset_seconds: f32, bpm: f32) -> (f32, f32);
    
    // Find phase x ∈ [0, 1] that produces y ∈ [0, 1]
    fn find_x_for_y(&self, y: f32) -> f32;
}

struct SineWaveform;

impl WaveformGenerator for SineWaveform {
    fn evaluate(&self, time_offset_seconds: f32, bpm: f32) -> (f32, f32) {
        // period = 1 cycle, y ∈ [0, 1]
        let freq = bpm / 60.0;
        let phase_rads = 2.0 * core::f32::consts::PI * time_offset_seconds * freq;
        let y = libm::sinf(phase_rads) / 2.0 + 0.5;
        
        // speed = d/dt(y) = d/dt(sin(2π * freq * t) / 2 + 0.5)
        //       = cos(2π * freq * t) * (2π * freq) / 2
        //       = π * freq * cos(2π * freq * t)
        let speed = core::f32::consts::PI * freq * libm::cosf(phase_rads);
        (y, speed)
    }
    
    fn find_x_for_y(&self, y: f32) -> f32 {
        // y = sin(2πx) / 2 + 0.5
        // sin(2πx) = (y - 0.5) * 2
        // 2πx = asin((y - 0.5) * 2)
        // x = asin((y - 0.5) * 2) / (2π)
        let normalized = (y - 0.5) * 2.0;
        let clamped = normalized.clamp(-1.0, 1.0);
        let angle = libm::asinf(clamped);
        let x = angle / (2.0 * core::f32::consts::PI);
        // asin returns [-π/2, π/2], map to [0, 1]
        if x < 0.0 {
            x + 1.0
        } else {
            x
        }
    }
}

struct ThrustWaveform {
    sharpness: f32,
}

impl ThrustWaveform {
    fn new(sharpness: f32) -> Self {
        Self { sharpness }
    }
}

impl WaveformGenerator for ThrustWaveform {
    fn evaluate(&self, time_offset_seconds: f32, bpm: f32) -> (f32, f32) {
        let freq = bpm / 60.0;
        let cycles = time_offset_seconds * freq;
        let x = cycles % 1.0;
        
        // Sharpness controls the rise duration [0.01, 0.99]
        // Lower values = sharper thrust (faster rise)
        let rise_duration = self.sharpness.clamp(0.01, 0.99);
        
        // Smootherstep function and its derivative
        // s(t) = 6t^5 - 15t^4 + 10t^3
        // s'(t) = 30t^4 - 60t^3 + 30t^2 = 30 * t^2 * (t-1)^2
        let smootherstep = |t: f32| -> f32 {
            let t = t.clamp(0.0, 1.0);
            t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
        };
        let smootherstep_derivative = |t: f32| -> f32 {
            let t = t.clamp(0.0, 1.0);
            30.0 * t * t * (t - 1.0) * (t - 1.0)
        };
        
        let (y, dy_dx) = if x < rise_duration {
            // Rise phase
            let t = x / rise_duration;
            let y = smootherstep(t);
            let dy_dt_norm = smootherstep_derivative(t);
            let dy_dx = dy_dt_norm / rise_duration;
            (y, dy_dx)
        } else {
            // Fall phase
            let t = (x - rise_duration) / (1.0 - rise_duration);
            let y = 1.0 - smootherstep(t);
            let dy_dt_norm = smootherstep_derivative(t);
            let dy_dx = -dy_dt_norm / (1.0 - rise_duration);
            (y, dy_dx)
        };

        // speed = dy/d(time) = dy/dx * dx/d(time)
        // dx/d(time) = freq
        let speed = dy_dx * freq;
        (y, speed)
    }

    fn find_x_for_y(&self, y: f32) -> f32 {
        // Binary search to find x such that evaluate(x, 1.0) ≈ y
        let mut left = 0.0;
        let mut right = 1.0;
        let target_y = y.clamp(0.0, 1.0);
        
        for _ in 0..20 {  // 20 iterations should be enough precision
            let mid = (left + right) / 2.0;
            // Evaluate at 1 BPM means time_offset = mid * 60
            let (mid_y, _) = self.evaluate(mid * 60.0, 1.0);
            
            if (mid_y - target_y).abs() < 0.001 {
                return mid;
            }
            
            if mid_y < target_y {
                left = mid;
            } else {
                right = mid;
            }
        }
        
        (left + right) / 2.0
    }
}

struct SplineWaveform {
    points: Vec<f32>,
    tangents: Vec<f32>,
    min_pos: f32,
    inv_range: f32,
}

impl SplineWaveform {
    fn eval_raw(points: &[f32], tangents: &[f32], x: f32) -> (f32, f32) {
        let num_points = points.len();
        if num_points == 0 {
            return (0.5, 0.0);
        }
        if num_points == 1 {
            return (points[0], 0.0);
        }

        let segment_width = 1.0 / num_points as f32;
        let segment_index = (libm::floorf(x / segment_width) as usize).min(num_points - 1);

        let p0_index = segment_index;
        let p1_index = (segment_index + 1) % num_points;

        let p0 = points[p0_index];
        let p1 = points[p1_index];
        let m0 = tangents[p0_index];
        let m1 = tangents[p1_index];

        let x_k = p0_index as f32 * segment_width;
        let u = if segment_width > 0.0 {
            ((x - x_k) / segment_width).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let m0_scaled = m0 * segment_width;
        let m1_scaled = m1 * segment_width;

        let u2 = u * u;
        let u3 = u2 * u;

        // Cubic Hermite spline formula
        let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
        let h10 = u3 - 2.0 * u2 + u;
        let h01 = -2.0 * u3 + 3.0 * u2;
        let h11 = u3 - u2;
        let pos = h00 * p0 + h10 * m0_scaled + h01 * p1 + h11 * m1_scaled;

        // Derivative of the spline w.r.t. u
        let dh00 = 6.0 * u2 - 6.0 * u;
        let dh10 = 3.0 * u2 - 4.0 * u + 1.0;
        let dh01 = -6.0 * u2 + 6.0 * u;
        let dh11 = 3.0 * u2 - 2.0 * u;
        let dy_du = dh00 * p0 + dh10 * m0_scaled + dh01 * p1 + dh11 * m1_scaled;

        let dy_dx = if segment_width > 0.0 { dy_du / segment_width } else { 0.0 };
        (pos, dy_dx)
    }

    fn from_points(points: &[f32]) -> Result<Self> {
        let num_points = points.len();

        if num_points == 0 {
            return Ok(Self {
                points: alloc::vec![0.5],
                tangents: alloc::vec![0.0],
                min_pos: 0.5,
                inv_range: 0.0,
            });
        }
        if num_points == 1 {
            return Ok(Self {
                points: alloc::vec![points[0]],
                tangents: alloc::vec![0.0],
                min_pos: points[0],
                inv_range: 0.0,
            });
        }

        // Use Catmull-Rom splines to calculate tangents for cubic Hermite interpolation
        let mut tangents = alloc::vec::Vec::with_capacity(num_points);
        for i in 0..num_points {
            let p_prev = points[(i + num_points - 1) % num_points];
            let p_next = points[(i + 1) % num_points];
            // Tangent dy/dx at point i
            tangents.push((p_next - p_prev) * num_points as f32 / 2.0);
        }

        // Find min and max positions over the spline to normalize to [0, 1]
        let mut min_pos = f32::MAX;
        let mut max_pos = f32::MIN;
        let num_samples = (num_points * 20).max(100);
        for i in 0..=num_samples {
            let x = (i as f32 / num_samples as f32).min(0.999999);
            let (pos, _) = Self::eval_raw(points, &tangents, x);
            if pos < min_pos { min_pos = pos; }
            if pos > max_pos { max_pos = pos; }
        }

        let range = max_pos - min_pos;
        let (min_pos, inv_range) = if range > 1e-6 {
            (min_pos, 1.0 / range)
        } else {
            (0.5, 0.0)
        };

        Ok(Self {
            points: points.to_vec(),
            tangents,
            min_pos,
            inv_range,
        })
    }
}

impl WaveformGenerator for SplineWaveform {
    fn evaluate(&self, time_offset_seconds: f32, bpm: f32) -> (f32, f32) {
        let freq = bpm / 60.0;
        let cycles = time_offset_seconds * freq;
        let x = cycles % 1.0;

        let (raw_pos, raw_dy_dx) = Self::eval_raw(&self.points, &self.tangents, x);

        let pos = if self.inv_range > 0.0 {
            (raw_pos - self.min_pos) * self.inv_range
        } else {
            0.5
        };
        let dy_dx = raw_dy_dx * self.inv_range;
        let speed = dy_dx * freq;
        (pos, speed)
    }

    fn find_x_for_y(&self, y: f32) -> f32 {
        let mut left = 0.0;
        let mut right = 1.0;
        let target_y = y.clamp(0.0, 1.0);

        for _ in 0..20 {
            let mid = (left + right) / 2.0;
            let (mid_y, _) = self.evaluate(mid * 60.0, 1.0);

            if (mid_y - target_y).abs() < 0.001 {
                return mid;
            }

            if mid_y < target_y {
                left = mid;
            } else {
                right = mid;
            }
        }

        (left + right) / 2.0
    }
}

// ===== Motion Commands =====

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct StreamWaypoint {
    pub ts: u64,
    pub pos: f32,
    #[serde(default)]
    pub vel: Option<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum MotionCommand {
    AppendWaypoints(Vec<StreamWaypoint>),
    SetWaypoints { waypoints: Vec<StreamWaypoint>, reset_timestamp: bool },
    // Drop buffered waypoints and the clock epoch; the next waypoint re-anchors
    ResetTimestamp,
    /// Apply a full motor config (sole cross-task write path into MotorController).
    SetConfig(MotorControllerConfig),
}

// ===== Motion Source Trait =====
pub trait MotionSource: Send {
    // Returns (pos, speed) where pos is normalized [0, 1]
    fn update(&mut self, dt: f32) -> (f32, f32);
    
    // Adjust internal state to match (y, speed), used to avoid jumps when switching sources
    fn follow(&mut self, y: f32, speed: f32);

    // Get current phase info (t, x) for status reporting
    fn get_phase_info(&self) -> (f32, f32);
}

// ===== Motion Source Implementations =====

struct WaveformMotionSource {
    generator: Box<dyn WaveformGenerator>,
    bpm: f32,
    t0: Instant,
}

impl WaveformMotionSource {
    fn new(generator: Box<dyn WaveformGenerator>, bpm: f32) -> Self {
        Self {
            generator,
            bpm,
            t0: Instant::now(),
        }
    }
}

impl MotionSource for WaveformMotionSource {
    fn update(&mut self, _dt: f32) -> (f32, f32) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.t0).as_micros() as f32 / 1_000_000.0;
        self.generator.evaluate(elapsed, self.bpm)
    }

    fn follow(&mut self, y: f32, _speed: f32) {
        // A periodic waveform cannot match an arbitrary velocity at a given
        // position, so only the phase is matched.
        let phase = self.generator.find_x_for_y(y);
        let time_offset = phase * 60.0 / self.bpm;
        self.t0 = Instant::now()
            - Duration::from_micros((time_offset * 1_000_000.0) as u64);
    }
    
    fn get_phase_info(&self) -> (f32, f32) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.t0).as_micros() as f32 / 1_000_000.0;
        let cycles = elapsed * self.bpm / 60.0;
        let x = cycles % 1.0;
        (elapsed, x)
    }
}

struct PausedMotionSource {
    current_y: f32,
    target_y: f32,
}

impl PausedMotionSource {
    fn new(current_y: f32, target_y: f32) -> Self {
        Self { current_y, target_y }
    }
}

impl MotionSource for PausedMotionSource {
    fn update(&mut self, dt: f32) -> (f32, f32) {
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

    fn follow(&mut self, y: f32, _speed: f32) {
        self.current_y = y;
    }
    
    fn get_phase_info(&self) -> (f32, f32) {
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
            return Self { a: 0.0, b: 0.0, c: 0.0, d: end_y, duration: 0.0, elapsed: 0.0 };
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
        
        Self { a, b, c, d, duration, elapsed: 0.0 }
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

#[derive(Serialize, Clone, Copy, Default)]
pub struct StreamStatus {
    pub buffered: usize,
    pub stream_time: f32,
    pub underrun: bool,
}

const MAX_WINDOW: usize = 128;
// Speed used to size the catch-up trajectory toward a newly anchored stream
const CATCHUP_SPEED: f32 = 0.5;      // y-units/s
const CATCHUP_MIN_DURATION: f32 = 0.2;
const CATCHUP_MAX_DURATION: f32 = 2.0;
const UNDERRUN_DECEL: f32 = 20.0;    // y-units/s^2

enum StreamSample {
    Interpolated(f32, f32),
    BeforeFirst,
    Exhausted,
}

struct StreamingMotionSource {
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
    fn new(start_y: f32) -> Self {
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

    pub fn status(&self) -> StreamStatus {
        StreamStatus {
            buffered: self.window.len(),
            stream_time: self.stream_time,
            underrun: self.underrun,
        }
    }

    fn apply_stream_command(&mut self, cmd: MotionCommand) {
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
                // Applied by MotorController::drain_commands; unreachable here.
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
        let idx = self.window.iter().rposition(|w| w.t <= t).map_or(0, |i| i + 1);
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
        if dt > 1e-4 { (b.pos - a.pos) / dt } else { 0.0 }
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
    fn update(&mut self, dt: f32) -> (f32, f32) {
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

    fn follow(&mut self, y: f32, speed: f32) {
        self.window.clear();
        self.epoch = None;
        self.catchup = None;
        self.underrun = false;
        self.current_y = y;
        self.current_speed = speed;
    }
    
    fn get_phase_info(&self) -> (f32, f32) {
        (self.stream_time, 0.0) // No phase concept in streaming
    }
}



// ===== Layer 2: Shaper =====
// Transforms y ∈ [0, 1] → y ∈ [0, 1] with depth, direction, and reversal

#[derive(Clone, Copy, PartialEq)]
pub enum DepthDirection {
    Top,    // [0, depth]
    Bottom, // [1-depth, 1]
}

#[derive(Clone)]
pub struct Shaper {
    target_depth: f32,       // Target depth
    current_depth: f32,      // Current depth (transitions smoothly to target)
    direction: DepthDirection,
    target_reversed: bool,
    current_reversal: f32,   // 0.0 = normal, 1.0 = reversed (transitions smoothly)
    
    // Transition state
    transitioning: bool,
}

const TRANSITION_SPEED: f32 = 0.1;  // Depth units per second
const REVERSAL_SPEED: f32 = 0.5;    // Reversal units per second (faster)
const PAUSE_SPEED: f32 = 0.3;       // Pause position transition speed (y units per second)
const TRANSITION_THRESHOLD: f32 = 0.01;

impl Shaper {
    pub fn new(depth: f32, direction: DepthDirection, reversed: bool) -> Self {
        Self {
            target_depth: depth,
            current_depth: depth,
            direction,
            target_reversed: reversed,
            current_reversal: if reversed { 1.0 } else { 0.0 },
            transitioning: true,
        }
    }
    
    pub fn set_params(&mut self, new_depth: f32, new_direction: DepthDirection, new_reversed: bool) {
        // Check if depth or reversal changed significantly
        let depth_changed = (self.target_depth - new_depth).abs() > TRANSITION_THRESHOLD;
        let reversal_changed = self.target_reversed != new_reversed;
        
        if depth_changed || reversal_changed {
            self.transitioning = true;
        }
        
        // Update target parameters
        self.target_depth = new_depth;
        self.direction = new_direction;
        self.target_reversed = new_reversed;
    }
    
    pub fn shape(&mut self, y_in: f32, speed_in: f32, dt: f32) -> (f32, f32) {
        // Update transitions if needed
        if self.transitioning {
            let mut depth_done = false;
            let mut reversal_done = false;
            
            // Update depth
            let depth_diff = self.target_depth - self.current_depth;
            if depth_diff.abs() < TRANSITION_THRESHOLD {
                self.current_depth = self.target_depth;
                depth_done = true;
            } else {
                let step = TRANSITION_SPEED * dt;
                if depth_diff > 0.0 {
                    self.current_depth = (self.current_depth + step).min(self.target_depth);
                } else {
                    self.current_depth = (self.current_depth - step).max(self.target_depth);
                }
            }
            
            // Update reversal
            let target_reversal = if self.target_reversed { 1.0 } else { 0.0 };
            let reversal_diff = target_reversal - self.current_reversal;
            if reversal_diff.abs() < TRANSITION_THRESHOLD {
                self.current_reversal = target_reversal;
                reversal_done = true;
            } else {
                let step = REVERSAL_SPEED * dt;
                if reversal_diff > 0.0 {
                    self.current_reversal = (self.current_reversal + step).min(target_reversal);
                } else {
                    self.current_reversal = (self.current_reversal - step).max(target_reversal);
                }
            }
            
            // Clear transitioning flag when both are done
            if depth_done && reversal_done {
                self.transitioning = false;
            }
        }
        
        // Apply smooth reversal: lerp between y_in and (1 - y_in)
        let r = self.current_reversal;
        let y = y_in * (1.0 - r) + (1.0 - y_in) * r;
        // Simplified: y = y_in * (1 - 2r) + r
        
        // Chain rule for speed: dy/dt = (∂y/∂y_in) * (dy_in/dt)
        // ∂y/∂y_in = 1 - 2r
        let speed = speed_in * (1.0 - 2.0 * r);
        
        // Then apply depth and direction
        match self.direction {
            DepthDirection::Top => {
                // Map [0, 1] → [0, current_depth]
                let shaped_y = y * self.current_depth;
                let shaped_speed = speed * self.current_depth;
                (shaped_y, shaped_speed)
            }
            DepthDirection::Bottom => {
                // Map [0, 1] → [1-current_depth, 1]
                let shaped_y = y * self.current_depth + (1.0 - self.current_depth);
                let shaped_speed = speed * self.current_depth;
                (shaped_y, shaped_speed)
            }
        }
    }
    
    // Reverse the shaping transformation to get unshaped y from shaped y
    // Returns None if currently transitioning or if reversal makes inversion ambiguous
    pub fn unshape(&self, y_shaped: f32) -> Option<f32> {
        // Can't reliably unshape during transitions
        if self.transitioning {
            return None;
        }
        
        // First, reverse depth and direction transformation
        let y_after_reversal = match self.direction {
            DepthDirection::Top => {
                // shaped = y * current_depth
                // y = shaped / current_depth
                if self.current_depth < TRANSITION_THRESHOLD {
                    return None; // Can't divide by near-zero depth
                }
                y_shaped / self.current_depth
            }
            DepthDirection::Bottom => {
                // shaped = y * current_depth + (1 - current_depth)
                // y = (shaped - (1 - current_depth)) / current_depth
                if self.current_depth < TRANSITION_THRESHOLD {
                    return None; // Can't divide by near-zero depth
                }
                (y_shaped - (1.0 - self.current_depth)) / self.current_depth
            }
        };
        
        // Then, reverse the reversal transformation
        // Forward: y = y_in * (1 - r) + (1 - y_in) * r
        // Simplify: y = y_in * (1 - 2r) + r
        // Solve for y_in: y_in = (y - r) / (1 - 2r)
        let r = self.current_reversal;
        let denominator = 1.0 - 2.0 * r;
        
        // When r ≈ 0.5, the transformation loses information (everything maps to 0.5)
        if denominator.abs() < TRANSITION_THRESHOLD {
            return None;
        }
        
        let y_in = (y_after_reversal - r) / denominator;
        
        // Clamp to valid range
        Some(y_in.clamp(0.0, 1.0))
    }
}

// ===== Layer 3: Position Generator =====
// Maps y ∈ [0, 1] to motor position

pub struct PositionGenerator {
    pub pos_min: f32,
    pub pos_max: f32,
}

impl PositionGenerator {
    pub fn new(pos_min: f32, pos_max: f32) -> Self {
        Self { pos_min, pos_max }
    }
    
    pub fn generate(&self, y: f32, speed_y: f32) -> (f32, f32) {
        let pos_range = self.pos_max - self.pos_min;
        let position = y * pos_range + self.pos_min;
        let speed = speed_y * pos_range;
        (position, speed)
    }
}

fn create_waveform_generator(config: &MotorControllerConfig) -> alloc::boxed::Box<dyn WaveformGenerator> {
    match config.wave_func.as_str() {
        "sine" => alloc::boxed::Box::new(SineWaveform),
        "thrust" => alloc::boxed::Box::new(ThrustWaveform::new(config.sharpness)),
        "spline" => {
            match SplineWaveform::from_points(&config.spline_points) {
                Ok(wf) => alloc::boxed::Box::new(wf),
                Err(e) => {
                    log::error!("Error creating spline waveform: {}. Falling back to sine wave.", e);
                    alloc::boxed::Box::new(SineWaveform)
                }
            }
        },
        _ => alloc::boxed::Box::new(SineWaveform),
    }
}

#[derive(PartialEq, Clone, Copy)]
enum MotionMode {
    Waveform,
    Paused,
    Streaming,
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug, Default, PartialEq)]
pub struct LoopStats {
    pub ups: u32,
    pub min_dt_ms: f32,
    pub max_dt_ms: f32,
    pub avg_dt_ms: f32,
    pub mdev_dt_ms: f32,
}

pub struct MotorController {
    // Store concrete structs
    waveform_source: WaveformMotionSource,
    paused_source: PausedMotionSource,
    streaming_source: StreamingMotionSource,

    command_consumer: CommandConsumer,

    active_mode: MotionMode,

    shaper: Shaper,
    position_gen: PositionGenerator,
    config: MotorControllerConfig,
    config_version: u32,
    last_cycle: Instant,
    
    // Internal state
    last_y: f32,
    last_speed: f32,

    // Telemetry history (10s capacity, 1s window)
    last_window_time: Instant,
    current_window_updates: u32,
    current_min_dt_ms: f32,
    current_max_dt_ms: f32,
    current_sum_dt_ms: f32,
    current_sum_sq_dt_ms: f32,
    pub last_loop_stats: LoopStats,
    pub motor_connected: bool,
    pub modbus_stats: ModbusStats,
    update_history: Vec<u32>,
    position_history: Vec<f32>,
    snapshot: StateResponse,
    snapshot_config_version: u32,
}

impl MotorController {
    pub fn new(config: MotorControllerConfig, command_consumer: CommandConsumer) -> Self {
        let generator = create_waveform_generator(&config);
        let waveform_source = WaveformMotionSource::new(generator, config.bpm);
        
        let paused_source = PausedMotionSource::new(config.paused_position, config.paused_position);
        let streaming_source = StreamingMotionSource::new(config.paused_position);
        
        let active_mode = if config.paused {
            MotionMode::Paused
        } else if config.streaming {
            MotionMode::Streaming
        } else {
            MotionMode::Waveform
        };
        
        let direction = if config.depth_top {
            DepthDirection::Top
        } else {
            DepthDirection::Bottom
        };
        
        let shaper = Shaper::new(config.depth, direction, config.reversed);
        let position_gen = PositionGenerator::new(0.0, 0.0); // Will be updated after homing
        
        let now = Instant::now();
        let default_config = config.clone();
        Self {
            waveform_source,
            paused_source,
            streaming_source,
            command_consumer,
            active_mode,
            shaper,
            position_gen,
            config: default_config.clone(),
            config_version: 0,
            last_cycle: now,
            last_y: config.paused_position, // Reasonable default
            last_speed: 0.0,
            last_window_time: now,
            current_window_updates: 0,
            current_min_dt_ms: 0.0,
            current_max_dt_ms: 0.0,
            current_sum_dt_ms: 0.0,
            current_sum_sq_dt_ms: 0.0,
            last_loop_stats: LoopStats::default(),
            motor_connected: false,
            modbus_stats: ModbusStats::default(),
            update_history: Vec::with_capacity(10),
            position_history: Vec::with_capacity(10),
            snapshot: StateResponse {
                config: default_config,
                ..StateResponse::default()
            },
            snapshot_config_version: 0,
        }
    }

    pub fn snapshot(&self) -> &StateResponse {
        &self.snapshot
    }

    fn refresh_snapshot(&mut self, position: f32, speed: f32, shaped_y: f32, y_wave: f32) {
        if self.snapshot_config_version != self.config_version {
            self.snapshot.config = self.config.clone();
            self.snapshot_config_version = self.config_version;
        }

        let (t, x) = self.active_source().get_phase_info();
        self.snapshot.t = t;
        self.snapshot.x = x;
        self.snapshot.y = y_wave;
        self.snapshot.shaped_y = shaped_y;
        self.snapshot.position = position;
        self.snapshot.speed = speed;
        self.snapshot.stream = self.streaming_source.status();
        self.snapshot.pos_min = self.position_gen.pos_min;
        self.snapshot.pos_max = self.position_gen.pos_max;
        self.snapshot.motor_connected = self.motor_connected;
        self.snapshot.modbus_stats = self.modbus_stats;
        self.snapshot.ups = self.last_loop_stats.ups;
        self.snapshot.dt_min_ms = self.last_loop_stats.min_dt_ms;
        self.snapshot.dt_max_ms = self.last_loop_stats.max_dt_ms;
        self.snapshot.dt_avg_ms = self.last_loop_stats.avg_dt_ms;
        self.snapshot.dt_mdev_ms = self.last_loop_stats.mdev_dt_ms;
    }

    fn drain_commands(&mut self) {
        while let Some(cmd) = self.command_consumer.dequeue() {
            match cmd {
                MotionCommand::SetConfig(config) => {
                    let _ = self.set_config(config);
                }
                stream_cmd => {
                    self.streaming_source.apply_stream_command(stream_cmd);
                }
            }
        }
    }

    fn active_source(&self) -> &dyn MotionSource {
        match self.active_mode {
            MotionMode::Waveform => &self.waveform_source,
            MotionMode::Paused => &self.paused_source,
            MotionMode::Streaming => &self.streaming_source,
        }
    }

    fn active_source_mut(&mut self) -> &mut dyn MotionSource {
        match self.active_mode {
            MotionMode::Waveform => &mut self.waveform_source,
            MotionMode::Paused => &mut self.paused_source,
            MotionMode::Streaming => &mut self.streaming_source,
        }
    }

    // Sync internal state to the motor's homed range and current position.
    // The actual homing and motor parameter setup happen in the (async) motor task,
    // which then calls this to align the motion sources with the physical position.
    pub fn sync_to_position(&mut self, pos_min: f32, pos_max: f32, position: f32) -> Result<()> {
        // Update position generator with actual range
        self.position_gen = PositionGenerator::new(pos_min, pos_max);

        // pause the motor and set pause position to current position
        let pos_normalized = (position - pos_min) / (pos_max - pos_min);
        
        // Try to unshape the current position to get the waveform y
        match self.shaper.unshape(pos_normalized) {
            Some(waveform_y) => {
                // Position is within current depth range, sync source to match
                log::info!("Syncing waveform to current position (y={})", waveform_y);
                self.active_source_mut().follow(waveform_y, 0.0);
                self.last_y = waveform_y;
                self.last_speed = 0.0; // Approximation
            }
            None => {
                // Position is outside current depth range, trigger transition
                log::info!("Current position is outside depth range, starting transition");
                
                // Set transitioning flag so shaper will move to target depth
                self.shaper.transitioning = true;
                
                // Start source at a default position (middle)
                self.active_source_mut().follow(0.5, 0.0);
                self.last_y = 0.5;
                self.last_speed = 0.0;
            }
        }
        
        // Enforce pause after homing via set_config so config_version always advances.
        let mut config = self.config.clone();
        config.paused = true;
        config.paused_position = self.last_y;
        self.set_config(config)
    }

    /// Sole post-init assignment for `self.config` — always bumps the version so
    /// `refresh_snapshot` publishes the change.
    fn commit_config(&mut self, mut config: MotorControllerConfig) {
        if config.version > self.config_version {
            self.config_version = config.version;
        } else {
            self.config_version = self.config_version.wrapping_add(1);
            config.version = self.config_version;
        }
        self.config = config;
    }

    pub fn set_config(&mut self, config: MotorControllerConfig) -> Result<()> {
        let wave_changed = self.config.wave_func != config.wave_func || self.config.spline_points != config.spline_points;
        let sharpness_changed = (self.config.sharpness - config.sharpness).abs() > 0.001;
        let bpm_changed = (self.config.bpm - config.bpm).abs() > 0.001;
        let spline_changed = self.config.spline_points != config.spline_points;
        
        // Update shaper
        let direction = if config.depth_top {
            DepthDirection::Top
        } else {
            DepthDirection::Bottom
        };
        self.shaper.set_params(config.depth, direction, config.reversed);

        // Determine target mode
        let target_mode = if config.paused {
            MotionMode::Paused
        } else if config.streaming {
            MotionMode::Streaming
        } else {
            MotionMode::Waveform
        };
        
        // 1. Handle Paused Source
        if target_mode == MotionMode::Paused {
            if self.active_mode != MotionMode::Paused {
                // Switching TO paused
                self.paused_source = PausedMotionSource::new(self.last_y, config.paused_position);
            } else {
                // Staying in paused
                if (self.config.paused_position - config.paused_position).abs() > 0.001 {
                    self.paused_source = PausedMotionSource::new(self.last_y, config.paused_position);
                }
            }
        }

        // 2. Handle Streaming Source
        if target_mode == MotionMode::Streaming && self.active_mode != MotionMode::Streaming {
            // Seed with the current position AND velocity; the catch-up
            // trajectory then blends into the stream without discontinuity.
            self.streaming_source.follow(self.last_y, self.last_speed);
        }

        // 3. Handle Waveform Source
        if target_mode == MotionMode::Waveform {
             let need_recreate = self.active_mode != MotionMode::Waveform || // Switching to it
                                 wave_changed || sharpness_changed || bpm_changed || spline_changed;
             
             if need_recreate {
                 let generator = create_waveform_generator(&config);
                 let mut new_wf = WaveformMotionSource::new(generator, config.bpm);
                 new_wf.follow(self.last_y, self.last_speed);
                 self.waveform_source = new_wf;
             }
        }

        self.active_mode = target_mode;
        self.commit_config(config);
        
        Ok(())
    }

    pub fn set_motor_connected(&mut self, connected: bool) {
        self.motor_connected = connected;
    }

    pub fn flush_snapshot(&mut self) {
        self.refresh_snapshot(
            self.snapshot.position,
            self.snapshot.speed,
            self.snapshot.shaped_y,
            self.snapshot.y,
        );
    }

    /// Reset the cycle clock so the next `compute_cycle` does not treat a long
    /// init/homing gap as an enormous `dt` / telemetry max.
    pub fn reset_cycle_clock(&mut self) {
        self.last_cycle = Instant::now();
        self.current_window_updates = 0;
        self.current_sum_dt_ms = 0.0;
        self.current_sum_sq_dt_ms = 0.0;
        self.last_window_time = Instant::now();
    }

    // Pure computation step: advances motion state and returns the (position, speed)
    // that should be written to the motor. The caller (motor task) performs the
    // actual (async) motor I/O.
    pub fn compute_cycle(&mut self) -> (f32, f32) {
        self.drain_commands();

        let now = Instant::now();
        let dt = now.duration_since(self.last_cycle).as_micros() as f32 / 1_000_000.0;
        self.last_cycle = now;

        let dt_ms = dt * 1000.0;
        if self.current_window_updates == 0 {
            self.current_min_dt_ms = dt_ms;
            self.current_max_dt_ms = dt_ms;
        } else {
            if dt_ms < self.current_min_dt_ms { self.current_min_dt_ms = dt_ms; }
            if dt_ms > self.current_max_dt_ms { self.current_max_dt_ms = dt_ms; }
        }
        self.current_sum_dt_ms += dt_ms;
        self.current_sum_sq_dt_ms += dt_ms * dt_ms;

        // Clamp step dt to at most 12ms so temporary CPU scheduling delays
        // never cause sudden start/stop position jumps on the servo motor.
        let dt_clamped = dt.min(0.012);

        // Layer 1: Motion Source
        let (y_wave, speed_wave) = self.active_source_mut().update(dt_clamped);
        self.last_y = y_wave;
        self.last_speed = speed_wave;
        
        // Layer 2: Apply shaping (with smooth transitions)
        let (shaped_y, shaped_speed) = self.shaper.shape(y_wave, speed_wave, dt_clamped);
        
        // Layer 3: Convert to position
        let (position, speed) = self.position_gen.generate(shaped_y, shaped_speed);

        self.current_window_updates += 1;
        if now.duration_since(self.last_window_time).as_micros() as f32 / 1_000_000.0 >= 1.0 {
            let n = self.current_window_updates as f32;
            let avg_dt_ms = if n > 0.0 { self.current_sum_dt_ms / n } else { 0.0 };
            let var = if n > 0.0 { (self.current_sum_sq_dt_ms / n) - (avg_dt_ms * avg_dt_ms) } else { 0.0 };
            let mdev_dt_ms = if var > 0.0 { libm::sqrtf(var) } else { 0.0 };

            self.last_loop_stats = LoopStats {
                ups: self.current_window_updates,
                min_dt_ms: self.current_min_dt_ms,
                max_dt_ms: self.current_max_dt_ms,
                avg_dt_ms,
                mdev_dt_ms,
            };

            if self.update_history.len() >= 10 {
                self.update_history.remove(0);
            }
            self.update_history.push(self.current_window_updates);

            if self.position_history.len() >= 10 {
                self.position_history.remove(0);
            }
            self.position_history.push(position);
            self.snapshot.update_history = self.update_history.clone();
            self.snapshot.position_history = self.position_history.clone();

            self.current_window_updates = 0;
            self.current_sum_dt_ms = 0.0;
            self.current_sum_sq_dt_ms = 0.0;
            self.last_window_time = now;
        }

        self.refresh_snapshot(position, speed, shaped_y, y_wave);

        (position, speed)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MotorControllerConfig {
    #[serde(default)]
    pub version: u32,
    pub bpm: f32,
    pub depth: f32,
    pub depth_top: bool,     // true = top [0, depth], false = bottom [1-depth, 1]
    pub reversed: bool,      // reverse waveform direction
    pub wave_func: String,   // "sine", "thrust", or "spline"
    pub sharpness: f32,      // For thrust waveform: rise duration (0.01-0.99), higher = longer rise
    #[serde(default)]
    pub spline_points: Vec<f32>,
    pub paused: bool,
    pub paused_position: f32,
    #[serde(default)]
    pub streaming: bool,
}

#[derive(Serialize, Clone, Copy, Default, PartialEq)]
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

#[derive(Serialize, Clone, Copy, Default, PartialEq)]
pub struct ModbusStats {
    pub successful_requests: u32,
    pub failed_requests: u32,
    pub success_rate: f32,
    pub round_trip: TimingWindowStats,
    pub slave_latency: TimingWindowStats,
    pub rx_duration: TimingWindowStats,
}

#[derive(Serialize, Clone)]
pub struct StateResponse {
    pub config: MotorControllerConfig,
    pub t: f32,              // Time offset in seconds
    pub x: f32,              // Phase [0, 1]
    pub y: f32,              // Waveform output [0, 1]
    pub shaped_y: f32,       // After shaping [0, 1]
    pub position: f32,       // Motor position
    pub speed: f32,          // Motor speed
    pub stream: StreamStatus, // Streaming source status
    pub update_history: Vec<u32>,   // Historical motor position update count per second (10s, 1s window)
    pub position_history: Vec<f32>, // Historical motor position sampled per second (10s, 1s window)
    pub pos_min: f32,        // Minimum motor position limit
    pub pos_max: f32,        // Maximum motor position limit
    pub ups: u32,
    pub dt_min_ms: f32,
    pub dt_max_ms: f32,
    pub dt_avg_ms: f32,
    pub dt_mdev_ms: f32,
    pub motor_connected: bool,
    pub modbus_stats: ModbusStats,
}

impl StateResponse {
    /// Copy fields from `src`, reusing heap capacity where possible.
    pub fn copy_from(&mut self, src: &StateResponse) {
        self.config.copy_from(&src.config);
        self.t = src.t;
        self.x = src.x;
        self.y = src.y;
        self.shaped_y = src.shaped_y;
        self.position = src.position;
        self.speed = src.speed;
        self.stream = src.stream;
        self.update_history.clone_from(&src.update_history);
        self.position_history.clone_from(&src.position_history);
        self.pos_min = src.pos_min;
        self.pos_max = src.pos_max;
        self.ups = src.ups;
        self.dt_min_ms = src.dt_min_ms;
        self.dt_max_ms = src.dt_max_ms;
        self.dt_avg_ms = src.dt_avg_ms;
        self.dt_mdev_ms = src.dt_mdev_ms;
        self.motor_connected = src.motor_connected;
        self.modbus_stats = src.modbus_stats;
    }
}

impl Default for StateResponse {
    fn default() -> Self {
        Self {
            config: MotorControllerConfig::default(),
            t: 0.0,
            x: 0.0,
            y: 0.0,
            shaped_y: 0.0,
            position: 0.0,
            speed: 0.0,
            stream: StreamStatus::default(),
            update_history: Vec::new(),
            position_history: Vec::new(),
            pos_min: 0.0,
            pos_max: 0.0,
            ups: 0,
            dt_min_ms: 0.0,
            dt_max_ms: 0.0,
            dt_avg_ms: 0.0,
            dt_mdev_ms: 0.0,
            motor_connected: false,
            modbus_stats: ModbusStats::default(),
        }
    }
}

impl MotorControllerConfig {
    /// Copy fields from `src`, reusing heap capacity where possible.
    pub fn copy_from(&mut self, src: &MotorControllerConfig) {
        self.version = src.version;
        self.bpm = src.bpm;
        self.depth = src.depth;
        self.depth_top = src.depth_top;
        self.reversed = src.reversed;
        self.wave_func.clone_from(&src.wave_func);
        self.sharpness = src.sharpness;
        self.spline_points.clone_from(&src.spline_points);
        self.paused = src.paused;
        self.paused_position = src.paused_position;
        self.streaming = src.streaming;
    }

    pub fn default() -> Self {
        Self {
            version: 0,
            bpm: 36.0,
            depth: 1.0,
            depth_top: false,
            reversed: false,
            wave_func: String::from("sine"),
            sharpness: 0.3,
            spline_points: vec![0.0, 1.0], // Default to a sawtooth wave
            paused: true,
            paused_position: 0.0,
            streaming: false,
        }
    }
}
