use super::source::TRANSITION_THRESHOLD;

// ===== Layer 2: Shaper =====
// Transforms y ∈ [0, 1] → y ∈ [0, 1] with depth, direction, and reversal

#[derive(Clone, Copy, PartialEq)]
pub enum DepthDirection {
    Top,    // [0, depth]
    Bottom, // [1-depth, 1]
}

#[derive(Clone)]
pub struct Shaper {
    target_depth: f32,  // Target depth
    current_depth: f32, // Current depth (transitions smoothly to target)
    direction: DepthDirection,
    target_reversed: bool,
    current_reversal: f32, // 0.0 = normal, 1.0 = reversed (transitions smoothly)

    // Transition state
    pub(crate) transitioning: bool,
}

const TRANSITION_SPEED: f32 = 0.1; // Depth units per second
const REVERSAL_SPEED: f32 = 0.5; // Reversal units per second (faster)

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

    pub fn set_params(
        &mut self,
        new_depth: f32,
        new_direction: DepthDirection,
        new_reversed: bool,
    ) {
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
