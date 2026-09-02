use super::source::TRANSITION_THRESHOLD;

// ===== Layer 2: Shaper =====
// Transforms y ∈ [0, 1] → y ∈ [0, 1] with depth, top/bottom anchor, and reversal.
//
// Invariant: `reversed` is an involution of the waveform map that preserves the
// current shaped position (controller rematches `y := 1-y` and snaps reverse).
// `depth` and `anchor` lerp so a window re-anchor never teleports.

#[derive(Clone)]
pub struct Shaper {
    target_depth: f32,
    current_depth: f32,
    /// 0 = bottom window [1-d, 1], 1 = top window [0, d]
    target_anchor: f32,
    current_anchor: f32,
    reversed: bool,
    transitioning: bool,
}

pub(crate) const TRANSITION_SPEED: f32 = 0.1; // Depth / anchor units per second

fn approach(current: f32, target: f32, step: f32) -> f32 {
    let diff = target - current;
    if diff.abs() <= step || diff.abs() < TRANSITION_THRESHOLD {
        target
    } else if diff > 0.0 {
        current + step
    } else {
        current - step
    }
}

fn anchor_from_top(depth_top: bool) -> f32 {
    if depth_top {
        1.0
    } else {
        0.0
    }
}

impl Shaper {
    pub fn new(depth: f32, depth_top: bool, reversed: bool) -> Self {
        let anchor = anchor_from_top(depth_top);
        Self {
            target_depth: depth,
            current_depth: depth,
            target_anchor: anchor,
            current_anchor: anchor,
            reversed,
            transitioning: false,
        }
    }

    pub fn set_params(&mut self, new_depth: f32, depth_top: bool, new_reversed: bool) {
        let new_anchor = anchor_from_top(depth_top);
        let depth_changed = (self.target_depth - new_depth).abs() > TRANSITION_THRESHOLD;
        let anchor_changed = (self.target_anchor - new_anchor).abs() > TRANSITION_THRESHOLD;

        if depth_changed || anchor_changed {
            self.transitioning = true;
        }

        self.target_depth = new_depth;
        self.target_anchor = new_anchor;
        // Reverse snaps; controller rematches waveform y so shaped_y is unchanged.
        self.reversed = new_reversed;
    }

    /// Inclusive window of shaped_y for the current depth/anchor.
    pub fn window(&self) -> (f32, f32) {
        let lo = (1.0 - self.current_depth) * (1.0 - self.current_anchor);
        (lo, lo + self.current_depth)
    }

    pub fn slew_toward(current: f32, target: f32, dt: f32) -> f32 {
        approach(current, target, TRANSITION_SPEED * dt)
    }

    fn apply_map(&self, y_in: f32, speed_in: f32) -> (f32, f32) {
        let y = if self.reversed { 1.0 - y_in } else { y_in };
        let speed = if self.reversed { -speed_in } else { speed_in };
        let offset = (1.0 - self.current_depth) * (1.0 - self.current_anchor);
        let shaped_y = y * self.current_depth + offset;
        let shaped_speed = speed * self.current_depth;
        (shaped_y, shaped_speed)
    }

    pub fn shape(&mut self, y_in: f32, speed_in: f32, dt: f32) -> (f32, f32) {
        if self.transitioning {
            self.current_depth =
                approach(self.current_depth, self.target_depth, TRANSITION_SPEED * dt);
            self.current_anchor = approach(
                self.current_anchor,
                self.target_anchor,
                TRANSITION_SPEED * dt,
            );
            let depth_done = (self.current_depth - self.target_depth).abs() < TRANSITION_THRESHOLD;
            let anchor_done =
                (self.current_anchor - self.target_anchor).abs() < TRANSITION_THRESHOLD;
            if depth_done && anchor_done {
                self.current_depth = self.target_depth;
                self.current_anchor = self.target_anchor;
                self.transitioning = false;
            }
        }

        self.apply_map(y_in, speed_in)
    }

    /// Inverse of [`Self::apply_map`]. `None` if depth is ~0 or `y_shaped` is
    /// outside the current window (before clamp).
    pub fn unshape(&self, y_shaped: f32) -> Option<f32> {
        if self.current_depth < TRANSITION_THRESHOLD {
            return None;
        }
        let (lo, hi) = self.window();
        if y_shaped < lo - TRANSITION_THRESHOLD || y_shaped > hi + TRANSITION_THRESHOLD {
            return None;
        }
        let y_rev = (y_shaped - lo) / self.current_depth;
        if !(0.0..=1.0).contains(&y_rev) {
            let clamped = y_rev.clamp(0.0, 1.0);
            if (clamped - y_rev).abs() > TRANSITION_THRESHOLD {
                return None;
            }
        }
        let y_rev = y_rev.clamp(0.0, 1.0);
        let y_in = if self.reversed { 1.0 - y_rev } else { y_rev };
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
        let y = y.clamp(0.0, 1.0);
        let pos_range = self.pos_max - self.pos_min;
        let position = y * pos_range + self.pos_min;
        let speed = speed_y * pos_range;
        (position, speed)
    }
}

#[cfg(test)]
mod tests {
    use super::PositionGenerator;

    #[test]
    fn generate_clamps_y_to_homed_bounds() {
        let gen = PositionGenerator::new(10.0, 20.0);
        let (lo, _) = gen.generate(-1.0, 0.0);
        let (hi, _) = gen.generate(2.0, 0.0);
        assert!((lo - 10.0).abs() < 1e-5, "lo={lo}");
        assert!((hi - 20.0).abs() < 1e-5, "hi={hi}");
        let (mid, _) = gen.generate(0.5, 0.0);
        assert!((mid - 15.0).abs() < 1e-5, "mid={mid}");
    }
}
