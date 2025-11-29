use anyhow::Result;

pub trait Motor: Send {
    fn cycle(&mut self) -> Result<()>;
    fn homing(&mut self) -> Result<()>;
    fn read_position(&mut self) -> Result<i32>;
    fn write_position(&mut self, position: i32, speed: f32) -> Result<()>;
    fn pos_min(&self) -> i32;
    fn pos_max(&self) -> i32;
    fn set_max_power(&mut self, power: u16) -> Result<()>;
    fn set_acceleration(&mut self, acceleration: u16) -> Result<()>;
    fn set_position_ring_ratio(&mut self, ratio: u16) -> Result<()>;
    fn set_speed_ring_ratio(&mut self, ratio: u16) -> Result<()>;
}
