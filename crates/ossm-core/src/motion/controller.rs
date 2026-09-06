use crate::config::MotorControllerConfig;
use crate::state::{MotionCommand, StateResponse};
use crate::time::{dt_seconds, Micros};
use ossm_common::{LinkStats, LoopStats};

use super::shaper::{PositionGenerator, Shaper};
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
    motor_connected: bool,
    snapshot: StateResponse,
    snapshot_config_version: u32,
    /// When homed pose is outside the depth window, command this shaped_y
    /// (slewed toward the window) instead of `follow(0.5)`.
    held_shaped: Option<f32>,
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

        let shaper = Shaper::new(config.depth, config.depth_top, config.reversed);
        let position_gen = PositionGenerator::new(0.0, 0.0);
        Self {
            waveform_source,
            paused_source,
            streaming_source,
            active_mode,
            shaper,
            position_gen,
            config,
            config_version: 0,
            last_cycle: 0,
            last_now: 0,
            last_y: config.paused_position,
            last_speed: 0.0,
            motor_connected: false,
            snapshot: StateResponse {
                config,
                ..StateResponse::default()
            },
            snapshot_config_version: 0,
            held_shaped: None,
        }
    }

    pub fn snapshot(&self) -> &StateResponse {
        &self.snapshot
    }

    pub fn apply_motion(&mut self, cmd: MotionCommand) {
        match cmd {
            MotionCommand::SetConfig(config) => {
                self.set_config(config);
            }
            MotionCommand::SetPaused { paused, position } => {
                let mut cfg = self.config;
                cfg.paused = paused;
                if let Some(p) = position {
                    cfg.paused_position = p;
                }
                self.set_config(cfg);
            }
            stream_cmd => {
                self.streaming_source.apply_stream_command(stream_cmd);
            }
        }
    }

    fn refresh_snapshot(&mut self, position: f32, speed: f32, shaped_y: f32, y_wave: f32) {
        if self.snapshot_config_version != self.config_version {
            self.snapshot.config.copy_from(&self.config);
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

        let pos_normalized = if (pos_max - pos_min).abs() < f32::EPSILON {
            0.5
        } else {
            ((position - pos_min) / (pos_max - pos_min)).clamp(0.0, 1.0)
        };
        let now = self.last_now;

        match self.shaper.unshape(pos_normalized) {
            Some(waveform_y) => {
                log::info!("Syncing waveform to current position (y={})", waveform_y);
                self.active_source_mut().follow(waveform_y, 0.0, now);
                self.last_y = waveform_y;
                self.last_speed = 0.0;
                self.held_shaped = None;
            }
            None => {
                log::info!("Current position is outside depth range, holding pose");
                self.held_shaped = Some(pos_normalized);
                self.last_speed = 0.0;
            }
        }

        let mut config = self.config;
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

    pub fn set_config(&mut self, mut config: MotorControllerConfig) {
        let wave_changed = self.config.wave_func != config.wave_func
            || self.config.spline_points != config.spline_points;
        let sharpness_changed = (self.config.sharpness - config.sharpness).abs() > 0.001;
        let bpm_changed = (self.config.bpm - config.bpm).abs() > 0.001;
        let spline_changed = self.config.spline_points != config.spline_points;
        let reverse_changed = self.config.reversed != config.reversed;

        let now = self.last_now;
        if reverse_changed {
            self.last_y = 1.0 - self.last_y;
            self.last_speed = -self.last_speed;
            config.paused_position = 1.0 - config.paused_position;
            self.paused_source.invert_y();
            self.waveform_source
                .follow(self.last_y, self.last_speed, now);
            self.streaming_source
                .follow(self.last_y, self.last_speed, now);
        }

        self.shaper
            .set_params(config.depth, config.depth_top, config.reversed);

        let target_mode = if config.paused {
            MotionMode::Paused
        } else if config.streaming {
            MotionMode::Streaming
        } else {
            MotionMode::Waveform
        };

        if target_mode == MotionMode::Paused
            && (self.active_mode != MotionMode::Paused
                || (self.config.paused_position - config.paused_position).abs() > 0.001)
        {
            self.paused_source = PausedMotionSource::new(self.last_y, config.paused_position);
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
        if self.motor_connected == connected {
            return;
        }
        self.motor_connected = connected;
    }

    pub fn set_loop_stats(&mut self, stats: LoopStats) {
        self.snapshot.loop_stats = stats;
        self.snapshot.stats_gen = self.snapshot.stats_gen.wrapping_add(1);
    }

    pub fn set_link_stats(&mut self, stats: LinkStats) {
        self.snapshot.link_stats = stats;
        self.snapshot.stats_gen = self.snapshot.stats_gen.wrapping_add(1);
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
    }

    pub fn tick(&mut self, now: Micros) -> (f32, f32) {
        self.last_now = now;
        let dt = dt_seconds(self.last_cycle, now);
        self.last_cycle = now;

        let dt_clamped = dt.min(0.012);

        let (y_wave, speed_wave) = self.active_source_mut().update(dt_clamped, now);

        let (shaped_y, shaped_speed, y_out) = if let Some(hold) = self.held_shaped {
            // Advance depth/anchor lerp without using the source sample for pose.
            let _ = self.shaper.shape(self.last_y, 0.0, dt_clamped);
            let (lo, hi) = self.shaper.window();
            if let Some(waveform_y) = self.shaper.unshape(hold) {
                self.active_source_mut().follow(waveform_y, 0.0, now);
                self.last_y = waveform_y;
                self.last_speed = 0.0;
                self.held_shaped = None;
                if self.active_mode == MotionMode::Paused {
                    self.config.paused_position = waveform_y;
                    self.paused_source = PausedMotionSource::new(waveform_y, waveform_y);
                }
                let (s, sp) = self.shaper.shape(waveform_y, 0.0, 0.0);
                (s, sp, waveform_y)
            } else {
                let target = hold.clamp(lo, hi);
                let new_hold = Shaper::slew_toward(hold, target, dt_clamped);
                self.held_shaped = Some(new_hold);
                self.last_speed = 0.0;
                (new_hold, 0.0, self.last_y)
            }
        } else {
            self.last_y = y_wave;
            self.last_speed = speed_wave;
            let (s, sp) = self.shaper.shape(y_wave, speed_wave, dt_clamped);
            (s, sp, y_wave)
        };

        let (position, speed) = self.position_gen.generate(shaped_y, shaped_speed);

        self.refresh_snapshot(position, speed, shaped_y, y_out);
        (position, speed)
    }
}
