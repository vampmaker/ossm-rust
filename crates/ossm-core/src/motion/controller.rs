use alloc::vec::Vec;

use crate::config::MotorControllerConfig;
use crate::state::{LoopStats, ModbusStats, MotionCommand, StateResponse};
use crate::time::{dt_seconds, Micros};

use super::shaper::{DepthDirection, PositionGenerator, Shaper};
use super::source::{
    MotionSource, PausedMotionSource, StreamingMotionSource, WaveformMotionSource,
};
use super::waveform::create_waveform_generator;

#[derive(PartialEq, Clone, Copy)]
enum MotionMode {
    Waveform,
    Paused,
    Streaming,
}

pub struct MotorController {
    waveform_source: WaveformMotionSource,
    paused_source: PausedMotionSource,
    streaming_source: StreamingMotionSource,
    active_mode: MotionMode,
    shaper: Shaper,
    position_gen: PositionGenerator,
    config: MotorControllerConfig,
    config_version: u32,
    last_cycle: Micros,
    last_now: Micros,
    last_y: f32,
    last_speed: f32,
    last_window_time: Micros,
    current_window_updates: u32,
    current_min_dt_ms: f32,
    current_max_dt_ms: f32,
    current_sum_dt_ms: f32,
    current_sum_sq_dt_ms: f32,
    last_loop_stats: LoopStats,
    motor_connected: bool,
    modbus_stats: ModbusStats,
    update_history: Vec<u32>,
    position_history: Vec<f32>,
    snapshot: StateResponse,
    snapshot_config_version: u32,
}

impl MotorController {
    pub fn new(config: MotorControllerConfig) -> Self {
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
        let position_gen = PositionGenerator::new(0.0, 0.0);
        let default_config = config.clone();
        Self {
            waveform_source,
            paused_source,
            streaming_source,
            active_mode,
            shaper,
            position_gen,
            config: default_config.clone(),
            config_version: 0,
            last_cycle: 0,
            last_now: 0,
            last_y: config.paused_position,
            last_speed: 0.0,
            last_window_time: 0,
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

    pub fn last_loop_stats(&self) -> LoopStats {
        self.last_loop_stats
    }

    pub fn apply_motion(&mut self, cmd: MotionCommand) {
        match cmd {
            MotionCommand::SetConfig(config) => {
                self.set_config(config);
            }
            stream_cmd => {
                self.streaming_source.apply_stream_command(stream_cmd);
            }
        }
    }

    fn refresh_snapshot(&mut self, position: f32, speed: f32, shaped_y: f32, y_wave: f32) {
        if self.snapshot_config_version != self.config_version {
            self.snapshot.config = self.config.clone();
            self.snapshot_config_version = self.config_version;
        }

        let (t, x) = self.active_source().get_phase_info(self.last_now);
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

    pub fn sync_to_position(&mut self, pos_min: f32, pos_max: f32, position: f32) {
        self.position_gen = PositionGenerator::new(pos_min, pos_max);

        let pos_normalized = (position - pos_min) / (pos_max - pos_min);
        let now = self.last_now;

        match self.shaper.unshape(pos_normalized) {
            Some(waveform_y) => {
                log::info!("Syncing waveform to current position (y={})", waveform_y);
                self.active_source_mut().follow(waveform_y, 0.0, now);
                self.last_y = waveform_y;
                self.last_speed = 0.0;
            }
            None => {
                log::info!("Current position is outside depth range, starting transition");
                self.shaper.transitioning = true;
                self.active_source_mut().follow(0.5, 0.0, now);
                self.last_y = 0.5;
                self.last_speed = 0.0;
            }
        }

        let mut config = self.config.clone();
        config.paused = true;
        config.paused_position = self.last_y;
        self.set_config(config);
    }

    fn commit_config(&mut self, mut config: MotorControllerConfig) {
        if config.version > self.config_version {
            self.config_version = config.version;
        } else {
            self.config_version = self.config_version.wrapping_add(1);
            config.version = self.config_version;
        }
        self.config = config;
    }

    pub fn set_config(&mut self, config: MotorControllerConfig) {
        let wave_changed = self.config.wave_func != config.wave_func
            || self.config.spline_points != config.spline_points;
        let sharpness_changed = (self.config.sharpness - config.sharpness).abs() > 0.001;
        let bpm_changed = (self.config.bpm - config.bpm).abs() > 0.001;
        let spline_changed = self.config.spline_points != config.spline_points;

        let direction = if config.depth_top {
            DepthDirection::Top
        } else {
            DepthDirection::Bottom
        };
        self.shaper
            .set_params(config.depth, direction, config.reversed);

        let target_mode = if config.paused {
            MotionMode::Paused
        } else if config.streaming {
            MotionMode::Streaming
        } else {
            MotionMode::Waveform
        };

        let now = self.last_now;

        if target_mode == MotionMode::Paused {
            if self.active_mode != MotionMode::Paused {
                self.paused_source = PausedMotionSource::new(self.last_y, config.paused_position);
            } else if (self.config.paused_position - config.paused_position).abs() > 0.001 {
                self.paused_source = PausedMotionSource::new(self.last_y, config.paused_position);
            }
        }

        if target_mode == MotionMode::Streaming && self.active_mode != MotionMode::Streaming {
            self.streaming_source
                .follow(self.last_y, self.last_speed, now);
        }

        if target_mode == MotionMode::Waveform {
            let need_recreate = self.active_mode != MotionMode::Waveform
                || wave_changed
                || sharpness_changed
                || bpm_changed
                || spline_changed;

            if need_recreate {
                let generator = create_waveform_generator(&config);
                let mut new_wf = WaveformMotionSource::new(generator, config.bpm);
                new_wf.follow(self.last_y, self.last_speed, now);
                self.waveform_source = new_wf;
            }
        }

        self.active_mode = target_mode;
        self.commit_config(config);
    }

    pub fn set_motor_connected(&mut self, connected: bool) {
        self.motor_connected = connected;
    }

    pub fn set_modbus_stats(&mut self, stats: ModbusStats) {
        self.modbus_stats = stats;
    }

    pub fn flush_snapshot(&mut self) {
        self.refresh_snapshot(
            self.snapshot.position,
            self.snapshot.speed,
            self.snapshot.shaped_y,
            self.snapshot.y,
        );
    }

    pub fn reset_cycle_clock(&mut self, now: Micros) {
        self.last_cycle = now;
        self.last_now = now;
        self.current_window_updates = 0;
        self.current_sum_dt_ms = 0.0;
        self.current_sum_sq_dt_ms = 0.0;
        self.last_window_time = now;
    }

    pub fn tick(&mut self, now: Micros) -> (f32, f32) {
        self.last_now = now;
        let dt = dt_seconds(self.last_cycle, now);
        self.last_cycle = now;

        let dt_ms = dt * 1000.0;
        if self.current_window_updates == 0 {
            self.current_min_dt_ms = dt_ms;
            self.current_max_dt_ms = dt_ms;
        } else {
            if dt_ms < self.current_min_dt_ms {
                self.current_min_dt_ms = dt_ms;
            }
            if dt_ms > self.current_max_dt_ms {
                self.current_max_dt_ms = dt_ms;
            }
        }
        self.current_sum_dt_ms += dt_ms;
        self.current_sum_sq_dt_ms += dt_ms * dt_ms;

        let dt_clamped = dt.min(0.012);

        let (y_wave, speed_wave) = self.active_source_mut().update(dt_clamped, now);
        self.last_y = y_wave;
        self.last_speed = speed_wave;

        let (shaped_y, shaped_speed) = self.shaper.shape(y_wave, speed_wave, dt_clamped);
        let (position, speed) = self.position_gen.generate(shaped_y, shaped_speed);

        self.current_window_updates += 1;
        if dt_seconds(self.last_window_time, now) >= 1.0 {
            let n = self.current_window_updates as f32;
            let avg_dt_ms = if n > 0.0 {
                self.current_sum_dt_ms / n
            } else {
                0.0
            };
            let var = if n > 0.0 {
                (self.current_sum_sq_dt_ms / n) - (avg_dt_ms * avg_dt_ms)
            } else {
                0.0
            };
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
