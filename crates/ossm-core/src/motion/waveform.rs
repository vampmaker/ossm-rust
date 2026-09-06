use crate::config::MotorControllerConfig;

pub(crate) trait WaveformGenerator: Send {
    fn evaluate(&self, time_offset_seconds: f32, bpm: f32) -> (f32, f32);

    // Find phase x ∈ [0, 1] that produces y ∈ [0, 1]
    fn find_x_for_y(&self, y: f32) -> f32;
}

pub(crate) struct SineWaveform;

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

pub(crate) struct ThrustWaveform {
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

        for _ in 0..20 {
            // 20 iterations should be enough precision
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

pub(crate) struct SplineWaveform {
    points: [f32; crate::config::SPLINE_POINTS_CAP],
    tangents: [f32; crate::config::SPLINE_POINTS_CAP],
    len: u8,
    min_pos: f32,
    inv_range: f32,
}

impl SplineWaveform {
    fn points_slice(&self) -> &[f32] {
        &self.points[..self.len as usize]
    }

    fn tangents_slice(&self) -> &[f32] {
        &self.tangents[..self.len as usize]
    }

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

        let dy_dx = if segment_width > 0.0 {
            dy_du / segment_width
        } else {
            0.0
        };
        (pos, dy_dx)
    }

    fn from_points(src: &[f32]) -> Self {
        let cap = crate::config::SPLINE_POINTS_CAP;
        let mut points = [0.0f32; crate::config::SPLINE_POINTS_CAP];
        if src.is_empty() {
            points[0] = 0.5;
            return Self {
                points,
                tangents: [0.0; crate::config::SPLINE_POINTS_CAP],
                len: 1,
                min_pos: 0.5,
                inv_range: 0.0,
            };
        }
        let n = src.len().min(cap);
        points[..n].copy_from_slice(&src[..n]);
        if n == 1 {
            return Self {
                points,
                tangents: [0.0; crate::config::SPLINE_POINTS_CAP],
                len: 1,
                min_pos: points[0],
                inv_range: 0.0,
            };
        }

        let mut tangents = [0.0f32; crate::config::SPLINE_POINTS_CAP];
        for i in 0..n {
            let p_prev = src[(i + n - 1) % n];
            let p_next = src[(i + 1) % n];
            tangents[i] = (p_next - p_prev) * n as f32 / 2.0;
        }

        let mut min_pos = f32::MAX;
        let mut max_pos = f32::MIN;
        let num_samples = (n * 20).max(100);
        for i in 0..=num_samples {
            let x = (i as f32 / num_samples as f32).min(0.999999);
            let (pos, _) = Self::eval_raw(&points[..n], &tangents[..n], x);
            if pos < min_pos {
                min_pos = pos;
            }
            if pos > max_pos {
                max_pos = pos;
            }
        }

        let range = max_pos - min_pos;
        let (min_pos, inv_range) = if range > 1e-6 {
            (min_pos, 1.0 / range)
        } else {
            (0.5, 0.0)
        };

        Self {
            points,
            tangents,
            len: n as u8,
            min_pos,
            inv_range,
        }
    }
}

impl WaveformGenerator for SplineWaveform {
    fn evaluate(&self, time_offset_seconds: f32, bpm: f32) -> (f32, f32) {
        let freq = bpm / 60.0;
        let cycles = time_offset_seconds * freq;
        let x = cycles % 1.0;

        let (raw_pos, raw_dy_dx) = Self::eval_raw(self.points_slice(), self.tangents_slice(), x);

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

pub(crate) enum WaveformKind {
    Sine(SineWaveform),
    Thrust(ThrustWaveform),
    Spline(SplineWaveform),
}

impl WaveformKind {
    pub(crate) fn evaluate(&self, time_offset_seconds: f32, bpm: f32) -> (f32, f32) {
        match self {
            Self::Sine(w) => w.evaluate(time_offset_seconds, bpm),
            Self::Thrust(w) => w.evaluate(time_offset_seconds, bpm),
            Self::Spline(w) => w.evaluate(time_offset_seconds, bpm),
        }
    }

    pub(crate) fn find_x_for_y(&self, y: f32) -> f32 {
        match self {
            Self::Sine(w) => w.find_x_for_y(y),
            Self::Thrust(w) => w.find_x_for_y(y),
            Self::Spline(w) => w.find_x_for_y(y),
        }
    }
}

pub(crate) fn create_waveform_generator(config: &MotorControllerConfig) -> WaveformKind {
    match config.wave_func {
        crate::config::WaveFunc::Sine => WaveformKind::Sine(SineWaveform),
        crate::config::WaveFunc::Thrust => {
            WaveformKind::Thrust(ThrustWaveform::new(config.sharpness))
        }
        crate::config::WaveFunc::Spline => {
            WaveformKind::Spline(SplineWaveform::from_points(config.spline_points.as_slice()))
        }
    }
}

/// Host-test helper: sine waveform at time `t` seconds.
pub fn sine_evaluate(time_offset_seconds: f32, bpm: f32) -> (f32, f32) {
    SineWaveform.evaluate(time_offset_seconds, bpm)
}
