#![no_std]

extern crate alloc;

pub mod command;
pub mod config;
pub mod engine;
pub mod error;
pub mod modbus;
pub mod motion;
pub mod paths;
pub mod rpc;
pub mod rpc_types;
pub mod state;
pub mod time;

pub use command::Command;
pub use config::MotorControllerConfig;
pub use engine::{CycleOutput, Engine};
pub use error::CoreError;
pub use motion::MotorController;
pub use rpc::{dispatch_rpc, RpcAction};
pub use rpc_types::{PausedControl, RpcRequest, SubscribeParams, WaypointsInput, WaypointsObject};
pub use state::{
    LoopStats, ModbusStats, MotionCommand, StateResponse, StreamStatus, StreamWaypoint,
    TimingWindowStats,
};
pub use time::Micros;

#[cfg(test)]
mod engine_tests;
#[cfg(test)]
mod rpc_tests;
