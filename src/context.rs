use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;

use crate::motion::MotorController;
use crate::storage::StorageManager;

pub type CsMutex<T> = Mutex<CriticalSectionRawMutex, T>;

#[derive(Clone, Copy)]
pub struct AppContext {
    pub storage: &'static CsMutex<StorageManager>,
    pub motor_controller: &'static CsMutex<Option<MotorController>>,
}
