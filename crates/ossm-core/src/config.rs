use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MotorControllerConfig {
    #[serde(default)]
    pub version: u32,
    pub bpm: f32,
    pub depth: f32,
    pub depth_top: bool,   // true = top [0, depth], false = bottom [1-depth, 1]
    pub reversed: bool,    // reverse waveform direction
    pub wave_func: String, // "sine", "thrust", or "spline"
    pub sharpness: f32,    // For thrust waveform: rise duration (0.01-0.99), higher = longer rise
    #[serde(default)]
    pub spline_points: Vec<f32>,
    pub paused: bool,
    pub paused_position: f32,
    #[serde(default)]
    pub streaming: bool,
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
}

impl Default for MotorControllerConfig {
    fn default() -> Self {
        Self {
            version: 0,
            bpm: 36.0,
            depth: 1.0,
            depth_top: false,
            reversed: false,
            wave_func: String::from("sine"),
            sharpness: 0.3,
            spline_points: vec![0.0, 1.0],
            paused: true,
            paused_position: 0.0,
            streaming: false,
        }
    }
}
