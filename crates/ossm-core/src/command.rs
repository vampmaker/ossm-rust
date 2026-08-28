use alloc::vec::Vec;

use crate::config::{MotorControllerConfig, NetworkConfiguration, PinConfiguration};
use crate::modbus::InjectJunkMode;
use crate::state::{ModbusStats, StreamWaypoint};

pub enum Command {
    SetConfig(MotorControllerConfig),
    AppendWaypoints(Vec<StreamWaypoint>),
    SetWaypoints {
        waypoints: Vec<StreamWaypoint>,
        reset_timestamp: bool,
    },
    ResetTimestamp,
    SetPaused {
        paused: bool,
        position: Option<f32>,
    },
    SetPin(PinConfiguration),
    SetNet(NetworkConfiguration),
    HomingComplete {
        pos_min: f32,
        pos_max: f32,
        position: f32,
    },
    SetMotorConnected(bool),
    SetModbusStats(ModbusStats),
    SetInject {
        mode: InjectJunkMode,
        nbytes: u8,
    },
}

impl From<crate::state::MotionCommand> for Command {
    fn from(cmd: crate::state::MotionCommand) -> Self {
        match cmd {
            crate::state::MotionCommand::SetConfig(c) => Self::SetConfig(c),
            crate::state::MotionCommand::AppendWaypoints(w) => Self::AppendWaypoints(w),
            crate::state::MotionCommand::SetWaypoints {
                waypoints,
                reset_timestamp,
            } => Self::SetWaypoints {
                waypoints,
                reset_timestamp,
            },
            crate::state::MotionCommand::ResetTimestamp => Self::ResetTimestamp,
        }
    }
}
