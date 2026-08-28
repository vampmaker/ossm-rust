use heapless::spsc::{Consumer, Producer};

pub use ossm_core::{
    LoopStats, ModbusStats, MotionCommand, MotorControllerConfig, StateResponse, TimingWindowStats,
};

pub const COMMAND_QUEUE_SIZE: usize = 3;
pub type CommandProducer = Producer<'static, MotionCommand>;
pub type CommandConsumer = Consumer<'static, MotionCommand>;
