use std::time;
use std::sync::{Arc, Mutex};

use serde::{Serialize, Deserialize};
use anyhow::Result;
use heapless::spsc::{Queue, Producer, Consumer};

use crate::motor::Motor;

const SPLINE_RESOLUTION: usize = 1500;
const COMMAND_QUEUE_SIZE: usize = 64;
type CommandProducer = Producer<'static, MotionCommand, COMMAND_QUEUE_SIZE>;

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
        let phase_rads = 2.0 * std::f32::consts::PI * time_offset_seconds * freq;
        let y = f32::sin(phase_rads) / 2.0 + 0.5;
        
        // speed = d/dt(y) = d/dt(sin(2π * freq * t) / 2 + 0.5)
        //       = cos(2π * freq * t) * (2π * freq) / 2
        //       = π * freq * cos(2π * freq * t)
        let speed = std::f32::consts::PI * freq * f32::cos(phase_rads);
        (y, speed)
    }
    
    fn find_x_for_y(&self, y: f32) -> f32 {
        // y = sin(2πx) / 2 + 0.5
        // sin(2πx) = (y - 0.5) * 2
        // 2πx = asin((y - 0.5) * 2)
        // x = asin((y - 0.5) * 2) / (2π)
        let normalized = (y - 0.5) * 2.0;
        let clamped = normalized.clamp(-1.0, 1.0);
        let angle = f32::asin(clamped);
        let x = angle / (2.0 * std::f32::consts::PI);
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
    resolution: usize,
    positions: Vec<f32>,
    speeds: Vec<f32>,
}

impl SplineWaveform {
    fn from_points(points: &[f32], resolution: usize) -> Result<Self> {
        let num_points = points.len();
        let mut positions = vec![0.0; resolution];
        let mut speeds = vec![0.0; resolution];

        if num_points == 0 {
            return Ok(Self {
                resolution,
                positions: vec![0.5; resolution], // Default to middle
                speeds: vec![0.0; resolution],
            });
        }
        if num_points == 1 {
            return Ok(Self {
                resolution,
                positions: vec![points[0]; resolution],
                speeds: vec![0.0; resolution],
            });
        }

        // Use Catmull-Rom splines to calculate tangents for cubic Hermite interpolation
        let mut tangents = Vec::with_capacity(num_points);
        for i in 0..num_points {
            let p_prev = points[(i + num_points - 1) % num_points];
            let p_next = points[(i + 1) % num_points];
            // Tangent dy/dx at point i
            tangents.push((p_next - p_prev) * num_points as f32 / 2.0);
        }
        
        let segment_width = 1.0 / num_points as f32;

        for i in 0..resolution {
            let x = i as f32 / (resolution as f32 - 1.0).max(1.0);
            
            let segment_index = (x / segment_width).floor() as usize;
            let segment_index = segment_index.min(num_points - 1);
            
            let p0_index = segment_index;
            let p1_index = (segment_index + 1) % num_points;

            let p0 = points[p0_index];
            let p1 = points[p1_index];
            let m0 = tangents[p0_index];
            let m1 = tangents[p1_index];
            
            // u is the interpolation factor within the segment, from 0 to 1
            let x_k = p0_index as f32 * segment_width;
            let u = if segment_width > 0.0 { (x - x_k) / segment_width } else { 0.0 };
            let u = u.clamp(0.0, 1.0);

            // The tangents (m0, m1) are dy/dx. For the Hermite spline, we need dy/du.
            // dy/du = dy/dx * dx/du = m * segment_width
            let m0_scaled = m0 * segment_width;
            let m1_scaled = m1 * segment_width;

            let u2 = u * u;
            let u3 = u2 * u;

            // Cubic Hermite spline formula
            let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
            let h10 = u3 - 2.0 * u2 + u;
            let h01 = -2.0 * u3 + 3.0 * u2;
            let h11 = u3 - u2;
            positions[i] = h00 * p0 + h10 * m0_scaled + h01 * p1 + h11 * m1_scaled;

            // Derivative of the spline w.r.t. u
            let dh00 = 6.0 * u2 - 6.0 * u;
            let dh10 = 3.0 * u2 - 4.0 * u + 1.0;
            let dh01 = -6.0 * u2 + 6.0 * u;
            let dh11 = 3.0 * u2 - 2.0 * u;
            let dy_du = dh00 * p0 + dh10 * m0_scaled + dh01 * p1 + dh11 * m1_scaled;

            // Convert speed from dy/du to dy/dx
            speeds[i] = if segment_width > 0.0 { dy_du / segment_width } else { 0.0 };
        }

        // Normalize positions to [0, 1] range and adjust speeds accordingly
        let mut min_pos = f32::MAX;
        let mut max_pos = f32::MIN;
        for &pos in &positions {
            if pos < min_pos { min_pos = pos; }
            if pos > max_pos { max_pos = pos; }
        }

        let range = max_pos - min_pos;
        if range > 1e-6 {
            let inv_range = 1.0 / range;
            for i in 0..resolution {
                positions[i] = (positions[i] - min_pos) * inv_range;
                speeds[i] *= inv_range;
            }
        } else {
            // All positions are the same, set to 0.5 and zero speed
            for i in 0..resolution {
                positions[i] = 0.5;
                speeds[i] = 0.0;
            }
        }
        Ok(Self { resolution, positions, speeds })
    }
}

impl WaveformGenerator for SplineWaveform {
    fn evaluate(&self, time_offset_seconds: f32, bpm: f32) -> (f32, f32) {
        let freq = bpm / 60.0;
        let cycles = time_offset_seconds * freq;
        let x = cycles % 1.0;
        
        // Linear interpolation
        let float_index = x * (self.resolution as f32 - 1.0);
        let index1 = float_index.floor() as usize;
        let index2 = (index1 + 1).min(self.resolution - 1);

        if index1 >= self.resolution -1 {
            let y = self.positions[self.resolution - 1];
            let dy_dx = self.speeds[self.resolution - 1];
            let speed = dy_dx * freq;
            return (y, speed);
        }
        
        let t = float_index - index1 as f32;
        
        let y1 = self.positions[index1];
        let y2 = self.positions[index2];
        let y = y1 + t * (y2 - y1);
        
        let s1 = self.speeds[index1];
        let s2 = self.speeds[index2];
        let dy_dx = s1 + t * (s2 - s1);
        
        let speed = dy_dx * freq;
        (y, speed)
    }

    fn find_x_for_y(&self, y: f32) -> f32 {
        let mut best_index = 0;
        let mut min_diff = f32::MAX;
        
        for i in 0..self.resolution {
            let diff = (self.positions[i] - y).abs();
            if diff < min_diff {
                min_diff = diff;
                best_index = i;
            }
        }
        
        best_index as f32 / (self.resolution - 1) as f32
    }
}

// ===== Motion Commands =====

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum MotionCommand {
    Move { target: f32, duration_ms: f32, final_speed: f32 },
    SetMaxSpeed(f32),
    SetMaxAccel(f32),
}

// ===== Motion Source Trait =====
pub trait MotionSource: Send {
    // Returns (pos, speed) where pos is normalized [0, 1]
    fn update(&mut self, dt: f32) -> (f32, f32);
    
    // Adjust internal state to match y, used to avoid jumps when switching sources
    fn follow(&mut self, y: f32);

    // Get current phase info (t, x) for status reporting
    fn get_phase_info(&self) -> (f32, f32);
}

// ===== Motion Source Implementations =====

struct WaveformMotionSource {
    generator: Box<dyn WaveformGenerator>,
    bpm: f32,
    t0: time::Instant,
}

impl WaveformMotionSource {
    fn new(generator: Box<dyn WaveformGenerator>, bpm: f32) -> Self {
        Self {
            generator,
            bpm,
            t0: time::Instant::now(),
        }
    }
}

impl MotionSource for WaveformMotionSource {
    fn update(&mut self, _dt: f32) -> (f32, f32) {
        let now = time::Instant::now();
        let elapsed = now.duration_since(self.t0).as_secs_f32();
        self.generator.evaluate(elapsed, self.bpm)
    }

    fn follow(&mut self, y: f32) {
        let phase = self.generator.find_x_for_y(y);
        let time_offset = phase * 60.0 / self.bpm;
        self.t0 = time::Instant::now() - time::Duration::from_secs_f32(time_offset);
    }
    
    fn get_phase_info(&self) -> (f32, f32) {
        let now = time::Instant::now();
        let elapsed = now.duration_since(self.t0).as_secs_f32();
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

    fn follow(&mut self, y: f32) {
        self.current_y = y;
    }
    
    fn get_phase_info(&self) -> (f32, f32) {
        (0.0, 0.0)
    }
}

// ===== Streaming Motion Source =====
// Handles queued moves and respects constraints

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

struct StreamingMotionSource {
    consumer: Consumer<'static, MotionCommand, COMMAND_QUEUE_SIZE>,
    current_y: f32,
    current_speed: f32,
    max_speed: f32,
    max_accel: f32,
    
    active_trajectory: Option<Trajectory>,
}

impl StreamingMotionSource {
    fn new(start_y: f32, consumer: Consumer<'static, MotionCommand, COMMAND_QUEUE_SIZE>) -> Self {
        Self {
            consumer,
            current_y: start_y,
            current_speed: 0.0,
            max_speed: 10.0, // Default arbitrary limit
            max_accel: 20.0, // Default arbitrary limit
            active_trajectory: None,
        }
    }
}

impl MotionSource for StreamingMotionSource {
    fn update(&mut self, dt: f32) -> (f32, f32) {
        // Check if we need to start a new command
        if self.active_trajectory.is_none() {
            if let Some(cmd) = self.consumer.dequeue() {
                match cmd {
                    MotionCommand::Move { target, duration_ms, final_speed } => {
                        let duration = duration_ms / 1000.0;
                        // Create trajectory from current state to target
                        // We clamp target to [0, 1]
                        let target_clamped = target.clamp(0.0, 1.0);
                        self.active_trajectory = Some(Trajectory::new(
                            self.current_y, 
                            self.current_speed, 
                            target_clamped, 
                            final_speed, 
                            duration
                        ));
                    }
                    MotionCommand::SetMaxSpeed(s) => self.max_speed = s,
                    MotionCommand::SetMaxAccel(a) => self.max_accel = a,
                }
            }
        }
        
        // Update active trajectory
        if let Some(traj) = &mut self.active_trajectory {
            if let Some((y, speed)) = traj.update(dt) {
                self.current_y = y.clamp(0.0, 1.0);
                self.current_speed = speed;
            } else {
                // Trajectory finished
                let (end_y, end_speed) = traj.end_state();
                self.current_y = end_y.clamp(0.0, 1.0);
                self.current_speed = end_speed;
                self.active_trajectory = None;
            }
        } else {
            // Idle state: decelerate to zero
            if self.current_speed.abs() > 1e-4 {
                let decel = self.max_accel * dt;
                if self.current_speed > 0.0 {
                    self.current_speed = (self.current_speed - decel).max(0.0);
                } else {
                    self.current_speed = (self.current_speed + decel).min(0.0);
                }
                // Update position based on average speed? Or current speed?
                // Simple integration: pos += speed * dt
                self.current_y += self.current_speed * dt;
                self.current_y = self.current_y.clamp(0.0, 1.0);
            } else {
                self.current_speed = 0.0;
            }
        }
        
        (self.current_y, self.current_speed)
    }

    fn follow(&mut self, y: f32) {
        // Drain queue?
        while self.consumer.dequeue().is_some() {}
        self.active_trajectory = None;
        self.current_y = y;
        self.current_speed = 0.0; // Reset speed as we don't know prior speed
    }
    
    fn get_phase_info(&self) -> (f32, f32) {
        (0.0, 0.0) // No phase concept in streaming
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
    pos_min: f32,
    pos_max: f32,
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

fn create_waveform_generator(config: &MotorControllerConfig) -> Box<dyn WaveformGenerator> {
    match config.wave_func.as_str() {
        "sine" => Box::new(SineWaveform),
        "thrust" => Box::new(ThrustWaveform::new(config.sharpness)),
        "spline" => {
            match SplineWaveform::from_points(&config.spline_points, SPLINE_RESOLUTION) {
                Ok(wf) => Box::new(wf),
                Err(e) => {
                    log::error!("Error creating spline waveform: {}. Falling back to sine wave.", e);
                    Box::new(SineWaveform)
                }
            }
        },
        _ => Box::new(SineWaveform),
    }
}

#[derive(PartialEq, Clone, Copy)]
enum MotionMode {
    Waveform,
    Paused,
    Streaming,
}

pub struct MotorController<'a> {
    motor: Box<dyn Motor + Send + 'a>,
    // Store concrete structs
    waveform_source: WaveformMotionSource,
    paused_source: PausedMotionSource,
    streaming_source: StreamingMotionSource,
    
    command_producer: Arc<Mutex<CommandProducer>>,

    active_mode: MotionMode,

    shaper: Shaper,
    position_gen: PositionGenerator,
    config: MotorControllerConfig,
    config_version: u32,
    last_cycle: time::Instant,
    
    // Internal state
    last_y: f32,
    last_speed: f32,
}

impl<'a> MotorController<'a> {
    pub fn new(motor: Box<dyn Motor + Send + 'a>, config: MotorControllerConfig) -> Self {
        let generator = create_waveform_generator(&config);
        let waveform_source = WaveformMotionSource::new(generator, config.bpm);
        
        let paused_source = PausedMotionSource::new(config.paused_position, config.paused_position);
        
        // Initialize Queue
        let queue: &'static mut Queue<MotionCommand, COMMAND_QUEUE_SIZE> = Box::leak(Box::new(Queue::new()));
        let (producer, consumer) = queue.split();
        let command_producer = Arc::new(Mutex::new(producer));

        let streaming_source = StreamingMotionSource::new(config.paused_position, consumer);
        
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
        
        let now = time::Instant::now();
        Self {
            motor,
            waveform_source,
            paused_source,
            streaming_source,
            command_producer,
            active_mode,
            shaper,
            position_gen,
            config: config.clone(),
            config_version: 0,
            last_cycle: now,
            last_y: config.paused_position, // Reasonable default
            last_speed: 0.0,
        }
    }

    pub fn get_command_sender(&self) -> Arc<Mutex<CommandProducer>> {
        self.command_producer.clone()
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

    pub fn init_motor(&mut self) -> Result<(), anyhow::Error> {
        self.motor.homing()?;
        log::info!("Motor homed, pos_min: {}, pos_max: {}", self.motor.pos_min(), self.motor.pos_max());

        // Update position generator with actual range
        self.position_gen = PositionGenerator::new(self.motor.pos_min(), self.motor.pos_max());

        // set motor parameters
        self.motor.set_max_power(0.6)?;
        self.motor.set_acceleration(4000.0)?;
        self.motor.set_position_ring_ratio(3000.0)?;
        self.motor.set_speed_ring_ratio(3000.0)?;

        // pause the motor and set pause position to current position

        // Read current motor position and sync source
        let position = self.motor.read_position()?;
        let pos_normalized = (position - self.motor.pos_min()) / (self.motor.pos_max() - self.motor.pos_min());
        
        // Try to unshape the current position to get the waveform y
        match self.shaper.unshape(pos_normalized) {
            Some(waveform_y) => {
                // Position is within current depth range, sync source to match
                println!("Syncing waveform to current position (y={})", waveform_y);
                self.active_source_mut().follow(waveform_y);
                self.last_y = waveform_y;
                self.last_speed = 0.0; // Approximation
            }
            None => {
                // Position is outside current depth range, trigger transition
                println!("Current position is outside depth range, starting transition");
                
                // Set transitioning flag so shaper will move to target depth
                self.shaper.transitioning = true;
                
                // Start source at a default position (middle)
                self.active_source_mut().follow(0.5);
                self.last_y = 0.5;
                self.last_speed = 0.0;
            }
        }
        
        // Enforce pause state as per previous logic (initially paused)
        self.config.paused = true;
        self.config.paused_position = self.last_y;
        
        self.active_mode = MotionMode::Paused;
        // Ensure paused source is synced
        self.paused_source.follow(self.last_y);
        self.paused_source = PausedMotionSource::new(self.last_y, self.config.paused_position);

        Ok(())
    }

    pub fn set_config(&mut self, config: MotorControllerConfig) -> Result<(), anyhow::Error> {
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
                self.paused_source.follow(self.last_y);
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
            self.streaming_source.follow(self.last_y);
        } else if target_mode != MotionMode::Streaming && self.active_mode == MotionMode::Streaming {
            // Switching away from streaming - maybe drain queue? or keep it
            // self.streaming_source.clear(); // Not strictly necessary
        }

        // 3. Handle Waveform Source
        if target_mode == MotionMode::Waveform {
             let need_recreate = self.active_mode != MotionMode::Waveform || // Switching to it
                                 wave_changed || sharpness_changed || bpm_changed || spline_changed;
             
             if need_recreate {
                 let generator = create_waveform_generator(&config);
                 let mut new_wf = WaveformMotionSource::new(generator, config.bpm);
                 new_wf.follow(self.last_y); 
                 self.waveform_source = new_wf;
             }
        }

        self.active_mode = target_mode;
        
        // Update config
        self.config = config.clone();
        self.config_version += 1;
        
        Ok(())
    }

    pub fn update_config(&mut self, f: impl FnOnce(&mut MotorControllerConfig)) -> Result<(), anyhow::Error> {
        let mut config = self.config.clone();
        f(&mut config);
        self.set_config(config)
    }

    pub fn get_config(&self) -> MotorControllerConfig {
        self.config.clone()
    }

    pub fn get_config_version(&self) -> u32 {
        self.config_version
    }

    pub fn get_current_state(&self) -> StateResponse {
        let (t, x) = self.active_source().get_phase_info();
        
        // Get current output without advancing state (assumes query is instantaneous or uses stored last_y)
        // Since we don't have a peek method, we can use last_y and assume speed 0 for query?
        // Or we can just return last computed values. 
        // We track last_y. But we don't track last_speed. 
        // Ideally we should separate update from query.
        // For now, let's use last_y and 0 speed or re-evaluate if it was a waveform?
        // But we can't easily re-evaluate without mutable access or knowing the type.
        // Let's rely on last_y and recalculate shaping.
        
        let y_wave = self.last_y;
        let speed_wave = self.last_speed;
        
        // Calculate shaped y
        let (shaped_y, shaped_speed) = {
            let mut temp_shaper = self.shaper.clone();
            // We pass speed_wave because we cached it
            temp_shaper.shape(y_wave, speed_wave, 0.0) 
        };
        
        // Calculate position
        let (position, speed) = self.position_gen.generate(shaped_y, shaped_speed);
        
        StateResponse {
            config: self.get_config(),
            t,
            x,
            y: y_wave,
            shaped_y,
            position,
            speed,
        }
    }

    pub fn cycle(&mut self) -> Result<(), anyhow::Error> {
        let now = time::Instant::now();
        let dt = now.duration_since(self.last_cycle).as_secs_f32();
        self.last_cycle = now;
        
        // Layer 1: Motion Source
        let (y_wave, speed_wave) = self.active_source_mut().update(dt);
        self.last_y = y_wave;
        self.last_speed = speed_wave;
        
        // Layer 2: Apply shaping (with smooth transitions)
        let (shaped_y, shaped_speed) = self.shaper.shape(y_wave, speed_wave, dt);
        
        // Layer 3: Convert to position and write
        let (position, speed) = self.position_gen.generate(shaped_y, shaped_speed);
        self.motor.write_position(position, speed)?;

        self.motor.cycle()?;
        
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct MotorControllerConfig {
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

#[derive(Serialize)]
pub struct StateResponse {
    pub config: MotorControllerConfig,
    pub t: f32,              // Time offset in seconds
    pub x: f32,              // Phase [0, 1]
    pub y: f32,              // Waveform output [0, 1]
    pub shaped_y: f32,       // After shaping [0, 1]
    pub position: f32,       // Motor position
    pub speed: f32,          // Motor speed
}

impl MotorControllerConfig {
    pub fn default() -> Self {
        Self {
            bpm: 36.0,
            depth: 1.0,
            depth_top: false,
            reversed: false,
            wave_func: "sine".to_string(),
            sharpness: 0.3,
            spline_points: vec![0.0, 1.0], // Default to a sawtooth wave
            paused: false,
            paused_position: 0.0,
            streaming: false,
        }
    }
}
