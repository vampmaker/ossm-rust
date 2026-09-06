use heapless::spsc::{Consumer, Producer};

pub use ossm_core::{LinkStats, LoopStats, MotionCommand, MotorControllerConfig, StateResponse};

pub const COMMAND_QUEUE_SIZE: usize = 3;
pub type CommandProducer = Producer<'static, MotionCommand>;
pub type CommandConsumer = Consumer<'static, MotionCommand>;
