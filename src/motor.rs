use anyhow::Result;

#[allow(async_fn_in_trait)]
pub trait Motor: Send {
    async fn cycle(&mut self) -> Result<()>;
    async fn homing(&mut self) -> Result<()>;

    // read current position, unit: rad
    async fn read_position(&mut self) -> Result<f32>;

    // write position and speed, unit: rad, rad/s
    async fn write_position(&mut self, position: f32, speed: f32) -> Result<()>;

    // get min position, unit: rad
    fn pos_min(&self) -> f32;

    // get max position, unit: rad
    fn pos_max(&self) -> f32;

    // set max output power in range [0, 1]
    async fn set_max_power(&mut self, power: f32) -> Result<()>;

    // set max acceleration, unit: rad/s^2
    async fn set_acceleration(&mut self, acceleration: f32) -> Result<()>;

    async fn set_position_ring_ratio(&mut self, ratio: f32) -> Result<()>;
    async fn set_speed_ring_ratio(&mut self, ratio: f32) -> Result<()>;
}
