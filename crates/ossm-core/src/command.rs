use crate::state::{ModbusStats, MotionCommand};

pub enum Command {
    Motion(MotionCommand),
    SetPaused {
        paused: bool,
        position: Option<f32>,
    },
    HomingComplete {
        pos_min: f32,
        pos_max: f32,
        position: f32,
    },
    SetMotorConnected(bool),
    SetModbusStats(ModbusStats),
}

impl From<MotionCommand> for Command {
    fn from(cmd: MotionCommand) -> Self {
        Self::Motion(cmd)
    }
}
