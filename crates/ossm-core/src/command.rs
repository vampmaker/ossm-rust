use ossm_common::{LinkStats, LoopStats};

use crate::state::MotionCommand;

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
    SetLoopStats(LoopStats),
    SetLinkStats(LinkStats),
}

impl From<MotionCommand> for Command {
    fn from(cmd: MotionCommand) -> Self {
        match cmd {
            MotionCommand::SetPaused { paused, position } => Self::SetPaused { paused, position },
            other => Self::Motion(other),
        }
    }
}
