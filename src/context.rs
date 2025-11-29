use crate::motion::MotorController;
use crate::storage::StorageManager;
use esp_idf_svc::hal::gpio::AnyIOPin;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct AppContext {
    pub storage_manager: Arc<Mutex<Box<StorageManager>>>,
    pub motor_controller: Arc<Mutex<Option<Box<MotorController<'static>>>>>,
    pub all_pins: Arc<Mutex<Vec<Option<AnyIOPin>>>>,
}
