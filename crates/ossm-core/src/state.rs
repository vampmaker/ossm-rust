use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::config::MotorControllerConfig;

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
    SetWaypoints {
        waypoints: Vec<StreamWaypoint>,
        reset_timestamp: bool,
    },
    // Drop buffered waypoints and the clock epoch; the next waypoint re-anchors
    ResetTimestamp,
    /// Apply a full motor config (sole cross-task write path into MotorController).
    SetConfig(MotorControllerConfig),
}

#[derive(Serialize, Clone, Copy, Default)]
pub struct StreamStatus {
    pub buffered: usize,
    pub stream_time: f32,
    pub underrun: bool,
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug, Default, PartialEq)]
pub struct LoopStats {
    pub ups: u32,
    pub min_dt_ms: f32,
    pub max_dt_ms: f32,
    pub avg_dt_ms: f32,
    pub mdev_dt_ms: f32,
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
    pub t: f32,                     // Time offset in seconds
    pub x: f32,                     // Phase [0, 1]
    pub y: f32,                     // Waveform output [0, 1]
    pub shaped_y: f32,              // After shaping [0, 1]
    pub position: f32,              // Motor position
    pub speed: f32,                 // Motor speed
    pub stream: StreamStatus,       // Streaming source status
    pub update_history: Vec<u32>, // Historical motor position update count per second (10s, 1s window)
    pub position_history: Vec<f32>, // Historical motor position sampled per second (10s, 1s window)
    pub pos_min: f32,             // Minimum motor position limit
    pub pos_max: f32,             // Maximum motor position limit
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
