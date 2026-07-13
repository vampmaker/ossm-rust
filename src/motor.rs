use crate::error::Result;

#[allow(async_fn_in_trait)]
pub trait Motor {
    async fn cycle(&mut self) -> Result<()>;
    async fn homing(&mut self) -> Result<()>;
    async fn read_position(&mut self) -> Result<f32>;
    async fn write_position(&mut self, position: f32, speed: f32) -> Result<()>;
    fn pos_min(&self) -> f32;
    fn pos_max(&self) -> f32;
    async fn set_max_power(&mut self, power: f32) -> Result<()>;
    async fn set_acceleration(&mut self, acceleration: f32) -> Result<()>;
    async fn set_position_ring_ratio(&mut self, ratio: f32) -> Result<()>;
    async fn set_speed_ring_ratio(&mut self, ratio: f32) -> Result<()>;
}
