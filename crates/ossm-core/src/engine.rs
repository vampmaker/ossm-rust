use crate::command::Command;
use crate::config::MotorControllerConfig;
use crate::error::{CoreError, Result};
use crate::motion::MotorController;
use crate::state::{MotionCommand, StateResponse};
use crate::time::Micros;

pub struct CycleOutput {
    pub position: f32,
    pub speed: f32,
}

pub struct Engine {
    motion: MotorController,
}

impl Engine {
    pub fn new(motor: MotorControllerConfig) -> Self {
        Self {
            motion: MotorController::new(motor),
        }
    }

    pub fn apply(&mut self, cmd: Command) {
        match cmd {
            Command::Motion(m) => {
                let flush = matches!(&m, MotionCommand::SetConfig(_));
                self.motion.apply_motion(m);
                if flush {
                    self.motion.flush_snapshot();
                }
            }
            Command::SetPaused { paused, position } => {
                let mut cfg = self.motion.snapshot().config;
                cfg.paused = paused;
                if let Some(p) = position {
                    cfg.paused_position = p;
                }
                self.motion.set_config(cfg);
                self.motion.flush_snapshot();
            }
            Command::HomingComplete {
                pos_min,
                pos_max,
                position,
            } => {
                self.motion.sync_to_position(pos_min, pos_max, position);
                self.motion.flush_snapshot();
            }
            Command::SetMotorConnected(connected) => {
                self.motion.set_motor_connected(connected);
            }
            Command::SetLoopStats(stats) => {
                self.motion.set_loop_stats(stats);
            }
            Command::SetLinkStats(stats) => {
                self.motion.set_link_stats(stats);
            }
        }
    }

    pub fn tick(&mut self, now: Micros) -> CycleOutput {
        let (position, speed) = self.motion.tick(now);
        CycleOutput { position, speed }
    }

    pub fn snapshot(&self) -> &StateResponse {
        self.motion.snapshot()
    }

    pub fn flush_snapshot(&mut self) {
        self.motion.flush_snapshot();
    }

    pub fn reset_cycle_clock(&mut self, now: Micros) {
        self.motion.reset_cycle_clock(now);
    }

    pub fn try_set_config(
        &mut self,
        mut cfg: MotorControllerConfig,
    ) -> Result<MotorControllerConfig> {
        let current_version = self.motion.snapshot().config.version;
        if cfg.version > 0 && cfg.version < current_version {
            return Err(CoreError::StaleVersion);
        }
        if cfg.version == 0 || cfg.version == current_version {
            cfg.version = current_version.wrapping_add(1);
        }
        self.motion.set_config(cfg);
        self.motion.flush_snapshot();
        Ok(self.motion.snapshot().config)
    }
}
