use crate::command::Command;
use crate::config::{MotorControllerConfig, NetworkConfiguration, PinConfiguration};
use crate::error::{CoreError, Result};
use crate::event::CycleOutput;
use crate::modbus;
use crate::motion::MotorController;
use crate::state::{MotionCommand, StateResponse};
use crate::time::Micros;

pub struct Engine {
    motion: MotorController,
    pin: PinConfiguration,
    net: NetworkConfiguration,
}

impl Engine {
    pub fn new(motor: MotorControllerConfig) -> Self {
        Self {
            motion: MotorController::new(motor),
            pin: PinConfiguration::default(),
            net: NetworkConfiguration::default(),
        }
    }

    pub fn with_pin_net(
        motor: MotorControllerConfig,
        pin: PinConfiguration,
        net: NetworkConfiguration,
    ) -> Self {
        Self {
            motion: MotorController::new(motor),
            pin,
            net,
        }
    }

    pub fn apply(&mut self, cmd: Command) {
        if self.pin.is_rtu_relay() {
            match &cmd {
                Command::SetConfig(_)
                | Command::AppendWaypoints(_)
                | Command::SetWaypoints { .. }
                | Command::ResetTimestamp
                | Command::SetPaused { .. }
                | Command::HomingComplete { .. } => {
                    return;
                }
                _ => {}
            }
        }

        match cmd {
            Command::SetConfig(cfg) => {
                self.motion.set_config(cfg);
                self.motion.flush_snapshot();
            }
            Command::AppendWaypoints(waypoints) => {
                self.motion
                    .apply_motion(MotionCommand::AppendWaypoints(waypoints));
            }
            Command::SetWaypoints {
                waypoints,
                reset_timestamp,
            } => {
                self.motion.apply_motion(MotionCommand::SetWaypoints {
                    waypoints,
                    reset_timestamp,
                });
            }
            Command::ResetTimestamp => {
                self.motion.apply_motion(MotionCommand::ResetTimestamp);
            }
            Command::SetPaused { paused, position } => {
                let mut cfg = self.motion.snapshot().config.clone();
                cfg.paused = paused;
                if let Some(p) = position {
                    cfg.paused_position = p;
                }
                self.motion.set_config(cfg);
                self.motion.flush_snapshot();
            }
            Command::SetPin(pin) => {
                let mut pin = pin;
                pin.normalize_operating_mode();
                self.pin = pin;
            }
            Command::SetNet(net) => {
                self.net = net;
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
            Command::SetModbusStats(stats) => {
                self.motion.set_modbus_stats(stats);
            }
            Command::SetInject { mode, nbytes } => {
                modbus::set_inject_junk(mode, nbytes);
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

    pub fn last_loop_stats(&self) -> crate::state::LoopStats {
        self.motion.last_loop_stats()
    }

    pub fn flush_snapshot(&mut self) {
        self.motion.flush_snapshot();
    }

    pub fn reset_cycle_clock(&mut self, now: Micros) {
        self.motion.reset_cycle_clock(now);
    }

    pub fn pin(&self) -> &PinConfiguration {
        &self.pin
    }

    pub fn net(&self) -> &NetworkConfiguration {
        &self.net
    }

    pub fn is_rtu_relay(&self) -> bool {
        self.pin.is_rtu_relay()
    }

    pub fn try_set_config(
        &mut self,
        mut cfg: MotorControllerConfig,
    ) -> Result<MotorControllerConfig> {
        if self.pin.is_rtu_relay() {
            return Err(CoreError::RelayMode);
        }
        let current_version = self.motion.snapshot().config.version;
        if cfg.version > 0 && cfg.version < current_version {
            return Err(CoreError::StaleVersion);
        }
        if cfg.version == 0 || cfg.version == current_version {
            cfg.version = current_version.wrapping_add(1);
        }
        self.motion.set_config(cfg.clone());
        self.motion.flush_snapshot();
        Ok(self.motion.snapshot().config.clone())
    }
}
