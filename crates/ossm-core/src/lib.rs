#![no_std]

extern crate alloc;

pub mod command;
pub mod config;
pub mod engine;
pub mod error;
pub mod event;
pub mod modbus;
pub mod motion;
pub mod paths;
pub mod rpc;
pub mod rpc_types;
pub mod state;
pub mod time;

pub use command::Command;
pub use config::{
    MotorControllerConfig, NetworkConfiguration, PinConfiguration, OPERATING_MODE_RTU_RELAY,
    OPERATING_MODE_SERVO,
};
pub use engine::Engine;
pub use error::CoreError;
pub use event::CycleOutput;
pub use motion::MotorController;
pub use rpc::{dispatch_rpc, RpcAction};
pub use rpc_types::{PausedControl, SubscribeParams, WaypointsInput, WaypointsObject, WsMessage};
pub use state::{
    LoopStats, ModbusStats, MotionCommand, StateResponse, StreamStatus, StreamWaypoint,
    TimingWindowStats,
};
pub use time::Micros;

#[cfg(test)]
mod engine_tests;
#[cfg(test)]
mod rpc_tests;
