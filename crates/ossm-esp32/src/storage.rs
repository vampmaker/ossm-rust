use alloc::string::String;

use esp_nvs::{Key, Nvs};
use esp_storage::FlashStorage;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::{FirmwareError, Result};
use crate::motion::MotorControllerConfig;

pub use ossm_core::{NetworkConfiguration, PinConfiguration};

const NVS_PARTITION_OFFSET: usize = 0x9000;
const NVS_PARTITION_SIZE: usize = 0x6000;
const NVS_NAMESPACE: Key = Key::from_str("ossm");

pub struct StorageManager {
    nvs: Nvs<FlashStorage<'static>>,
}

impl StorageManager {
    pub fn new(flash: esp_hal::peripherals::FLASH<'static>) -> Self {
        let storage = FlashStorage::new(flash);
        let nvs = Nvs::new(NVS_PARTITION_OFFSET, NVS_PARTITION_SIZE, storage)
            .expect("Failed to initialize NVS");
        Self { nvs }
    }

    fn key(name: &str) -> Key {
        Key::from_str(name)
    }

    fn get_string(&mut self, key: &str) -> Result<String> {
        self.nvs
            .get(&NVS_NAMESPACE, &Self::key(key))
            .map_err(|_| FirmwareError::Storage("read failed"))
    }

    fn set_string(&mut self, key: &str, value: &str) -> Result<()> {
        self.nvs
            .set(&NVS_NAMESPACE, &Self::key(key), value)
            .map_err(|_| FirmwareError::Storage("write failed"))
    }

    fn set_json<T: Serialize>(&mut self, key: &str, value: &T) -> Result<()> {
        let json = serde_json::to_string(value).map_err(|_| FirmwareError::Json)?;
        self.set_string(key, &json)
    }

    fn get_json<T: DeserializeOwned>(&mut self, key: &str) -> Result<T> {
        let string = self.get_string(key)?;
        serde_json::from_str(&string).map_err(|_| FirmwareError::Json)
    }

    pub fn set_ssid(&mut self, ssid: &str) -> Result<()> {
        self.set_string("ssid", ssid)
    }

    pub fn get_ssid(&mut self) -> Result<String> {
        self.get_string("ssid")
    }

    pub fn set_password(&mut self, password: &str) -> Result<()> {
        self.set_string("password", password)
    }

    pub fn get_password(&mut self) -> Result<String> {
        self.get_string("password")
    }

    pub fn set_motor_config(&mut self, config: &MotorControllerConfig) -> Result<()> {
        let mut config = config.clone();
        config.depth = config.depth.clamp(0.0, 1.0);
        config.bpm = config.bpm.clamp(1.0, 500.0);
        config.sharpness = config.sharpness.clamp(0.0, 1.0);
        config.paused_position = config.paused_position.clamp(0.0, 1.0);
        self.set_json("motor_config", &config)
    }

    pub fn get_motor_config(&mut self) -> Result<MotorControllerConfig> {
        self.get_json("motor_config")
    }

    pub fn set_pin_configuration(&mut self, config: &PinConfiguration) -> Result<()> {
        let mut config = config.clone();
        config.normalize_operating_mode();
        self.set_json("pin_conf", &config)
    }

    pub fn get_pin_configuration(&mut self) -> Result<PinConfiguration> {
        self.get_json("pin_conf")
    }

    pub fn set_network_configuration(&mut self, config: &NetworkConfiguration) -> Result<()> {
        if !config.ssid.is_empty() {
            let _ = self.set_ssid(&config.ssid);
        }
        if !config.password.is_empty() {
            let _ = self.set_password(&config.password);
        }
        self.set_json("net_conf", config)
    }

    pub fn get_network_configuration(&mut self) -> Result<NetworkConfiguration> {
        let mut config = self
            .get_json::<NetworkConfiguration>("net_conf")
            .unwrap_or_default();
        if config.ssid.is_empty() {
            if let Ok(ssid) = self.get_ssid() {
                config.ssid = ssid;
            }
        }
        if config.password.is_empty() {
            if let Ok(password) = self.get_password() {
                config.password = password;
            }
        }
        Ok(config)
    }
}
