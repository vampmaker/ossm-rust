use alloc::vec::Vec;

use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};

use crate::config::MotorControllerConfig;
pub use ossm_common::{LinkStats, LinkTransport, LoopStats, TimingWindowStats};

fn ser_timing<S: Serializer>(t: &TimingWindowStats, s: S) -> Result<S::Ok, S::Error> {
    let mut st = s.serialize_struct("TimingWindowStats", 9)?;
    st.serialize_field("min", &t.min)?;
    st.serialize_field("max", &t.max)?;
    st.serialize_field("pct5", &t.pct5)?;
    st.serialize_field("pct10", &t.pct10)?;
    st.serialize_field("pct50", &t.pct50)?;
    st.serialize_field("pct90", &t.pct90)?;
    st.serialize_field("pct95", &t.pct95)?;
    st.serialize_field("mean", &t.mean)?;
    st.serialize_field("mdev", &t.mdev)?;
    st.end()
}

struct LoopStatsSer<'a>(&'a LoopStats);

impl Serialize for LoopStatsSer<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let s = self.0;
        let mut st = serializer.serialize_struct("LoopStats", 7)?;
        st.serialize_field("ups", &s.ups)?;
        st.serialize_field("dt_min_ms", &s.dt_min_ms)?;
        st.serialize_field("dt_avg_ms", &s.dt_avg_ms)?;
        st.serialize_field("dt_max_ms", &s.dt_max_ms)?;
        st.serialize_field("dt_mdev_ms", &s.dt_mdev_ms)?;
        st.serialize_field("update_history", s.update_history.as_slice())?;
        st.serialize_field("position_history", s.position_history.as_slice())?;
        st.end()
    }
}

/// `/state` / `/link-stats` JSON for a common `LinkStats` snapshot.
pub struct LinkStatsSer<'a>(pub &'a LinkStats);

impl Serialize for LinkStatsSer<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let s = self.0;
        let mut st = serializer.serialize_struct("LinkStats", 16)?;
        st.serialize_field("transport", s.transport.as_str())?;
        st.serialize_field("connected", &s.connected)?;
        st.serialize_field("exchanges", &s.exchanges)?;
        st.serialize_field("failures", &s.failures)?;
        st.serialize_field("success_rate", &s.success_rate)?;
        st.serialize_field("exchanges_per_sec", &s.exchanges_per_sec)?;
        st.serialize_field("bytes_tx", &s.bytes_tx)?;
        st.serialize_field("bytes_rx", &s.bytes_rx)?;
        st.serialize_field("reconnects", &s.reconnects)?;
        st.serialize_field("round_trip", &TimingSer(&s.round_trip))?;
        st.serialize_field("slave_latency", &TimingSer(&s.slave_latency))?;
        st.serialize_field("rx_duration", &TimingSer(&s.rx_duration))?;
        st.serialize_field("pacing_state", s.pacing_state.as_str())?;
        st.serialize_field("pacing_period_us", &s.pacing_period_us)?;
        st.serialize_field("pacing_delay_us", &s.pacing_delay_us)?;
        st.serialize_field("rtt_min_us", &s.rtt_min_us)?;
        st.end()
    }
}

struct TimingSer<'a>(&'a TimingWindowStats);

impl Serialize for TimingSer<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ser_timing(self.0, serializer)
    }
}

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
    SetPaused {
        paused: bool,
        position: Option<f32>,
    },
}

#[derive(Serialize, Clone, Copy, Default, PartialEq)]
pub struct StreamStatus {
    pub buffered: usize,
    pub stream_time: f32,
    pub underrun: bool,
}

/// Serial CLI `get-status` wrapper. `config` is also nested in `state`; kept at
/// the top level so `ossm.py` serial status can `_track_version(res["config"])`.
#[derive(Serialize)]
pub struct CliStatusDump<'a> {
    pub state: &'a StateResponse,
    pub config: &'a MotorControllerConfig,
}

#[derive(Clone, Copy)]
pub struct StateResponse {
    pub config: MotorControllerConfig,
    pub t: f32,               // Time offset in seconds
    pub x: f32,               // Phase [0, 1]
    pub y: f32,               // Waveform output [0, 1]
    pub shaped_y: f32,        // After shaping [0, 1]
    pub position: f32,        // Motor position
    pub speed: f32,           // Motor speed
    pub stream: StreamStatus, // Streaming source status
    pub pos_min: f32,         // Minimum motor position limit
    pub pos_max: f32,         // Maximum motor position limit
    pub motor_connected: bool,
    pub loop_stats: LoopStats,
    pub link_stats: LinkStats,
    /// Bumped when `loop_stats` / `link_stats` are replaced. Not on the wire.
    pub(crate) stats_gen: u32,
}

impl Serialize for StateResponse {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut st = serializer.serialize_struct("StateResponse", 13)?;
        st.serialize_field("config", &self.config)?;
        st.serialize_field("t", &self.t)?;
        st.serialize_field("x", &self.x)?;
        st.serialize_field("y", &self.y)?;
        st.serialize_field("shaped_y", &self.shaped_y)?;
        st.serialize_field("position", &self.position)?;
        st.serialize_field("speed", &self.speed)?;
        st.serialize_field("stream", &self.stream)?;
        st.serialize_field("pos_min", &self.pos_min)?;
        st.serialize_field("pos_max", &self.pos_max)?;
        st.serialize_field("motor_connected", &self.motor_connected)?;
        st.serialize_field("loop_stats", &LoopStatsSer(&self.loop_stats))?;
        st.serialize_field("link_stats", &LinkStatsSer(&self.link_stats))?;
        st.end()
    }
}

impl StateResponse {
    pub fn copy_from(&mut self, src: &StateResponse) {
        *self = *src;
    }

    /// Compact BLE / low-MTU JSON (no histories, spline, or link stats).
    pub fn write_compact(&self, out: &mut [u8]) -> Option<usize> {
        #[derive(Serialize)]
        struct CompactConfig {
            bpm: f32,
            depth: f32,
            paused: bool,
            paused_position: f32,
        }
        #[derive(Serialize)]
        struct CompactState {
            config: CompactConfig,
            y: f32,
            shaped_y: f32,
            position: f32,
            speed: f32,
            ups: u32,
            motor_connected: bool,
        }
        let round_mult = |v: f32, mult: f32| libm::roundf(v * mult) / mult;
        let compact = CompactState {
            config: CompactConfig {
                bpm: round_mult(self.config.bpm, 10.0),
                depth: round_mult(self.config.depth, 100.0),
                paused: self.config.paused,
                paused_position: round_mult(self.config.paused_position, 1000.0),
            },
            y: round_mult(self.y, 1000.0),
            shaped_y: round_mult(self.shaped_y, 1000.0),
            position: round_mult(self.position, 1000.0),
            speed: round_mult(self.speed, 100.0),
            ups: self.loop_stats.ups,
            motor_connected: self.motor_connected,
        };
        serde_json_core::to_slice(&compact, out).ok()
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
            pos_min: 0.0,
            pos_max: 0.0,
            motor_connected: false,
            loop_stats: LoopStats::default(),
            link_stats: LinkStats::default(),
            stats_gen: 0,
        }
    }
}
