use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use anyhow::Result;
use crate::motion::MotorControllerConfig;

pub struct StorageManager {
    nvs: EspNvs<NvsDefault>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PinConfiguration {
    pub modbus_tx: u32,
    pub modbus_rx: u32,
    pub modbus_de_re: u32,
}

impl Default for PinConfiguration {
    fn default() -> Self {
        Self {
            modbus_tx: 18,
            modbus_rx: 19,
            modbus_de_re: 20,
        }
    }
}

impl StorageManager {
    pub fn new(nvs_partition: EspDefaultNvsPartition) -> Self {
        let nvs = EspNvs::new(nvs_partition, "ossm", true).unwrap();
        Self { nvs }
    }

    fn get_string(&self, key: &str) -> Result<String> {
        let mut buf = vec![0u8; 1024];
        let str_value = self.nvs.get_str(key, &mut buf).map_err(|e| anyhow::anyhow!("Failed to get string by key {}: {}", key, e))?;
        match str_value {
            Some(s) => {
                Ok(s.to_string())
            }
            None => {
                Err(anyhow::anyhow!("String value not found by key: {}", key))
            }
        }
    }

    fn set_json<T: Serialize>(&mut self, key: &str, value: &T) -> Result<()> {
        let json = serde_json::to_string(value)?;
        self.nvs.set_str(key, &json)?;
        Ok(())
    }

    fn get_json<T: DeserializeOwned>(&self, key: &str) -> Result<T> {
        let string = self.get_string(key)?;
        serde_json::from_str(&string).map_err(|e| anyhow::anyhow!("Failed to get JSON by key {}: {}", key, e))
    }

    pub fn set_ssid(&mut self, ssid: &str) -> Result<()> {
        self.nvs.set_str("ssid", ssid)?;
        Ok(())
    }

    pub fn get_ssid(&self) -> Result<String> {
        let mut buf = [0u8; 32];
        self.nvs.get_str("ssid", &mut buf)?;
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let ssid = core::str::from_utf8(&buf[..end]).map_err(|e| anyhow::anyhow!("Failed to get SSID: {}", e))?;
        Ok(ssid.to_string())
    }

    pub fn set_password(&mut self, password: &str) -> Result<()> {
        self.nvs.set_str("password", password)?;
        Ok(())
    }

    pub fn get_password(&self) -> Result<String> {
        let mut buf = [0u8; 64];
        self.nvs.get_str("password", &mut buf)?;
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let password = core::str::from_utf8(&buf[..end]).map_err(|e| anyhow::anyhow!("Failed to get Password: {}", e))?;
        Ok(password.to_string())
    }

    pub fn set_motor_config(&mut self, config: &MotorControllerConfig) -> Result<()> {
        let config = {
            let mut config = config.clone();
            config.depth = config.depth.clamp(0.0, 1.0);
            config.bpm = config.bpm.clamp(1.0, 500.0);
            config.sharpness = config.sharpness.clamp(0.0, 1.0);
            config.paused_position = config.paused_position.clamp(0.0, 1.0);
            config
        };

        self.set_json("motor_config", &config)?;
        Ok(())
    }

    pub fn get_motor_config(&self) -> Result<MotorControllerConfig> {
        self.get_json("motor_config")
    }

    pub fn set_pin_configuration(&mut self, config: &PinConfiguration) -> Result<()> {
        self.set_json("pin_configuration", &config)?;
        Ok(())
    }

    pub fn get_pin_configuration(&self) -> Result<PinConfiguration> {
        self.get_json("pin_configuration")
    }
}