use alloc::string::String;

use esp_nvs::{Key, Nvs};
use esp_storage::FlashStorage;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::error::{FirmwareError, Result};
use crate::motion::MotorControllerConfig;

const NVS_PARTITION_OFFSET: usize = 0x9000;
const NVS_PARTITION_SIZE: usize = 0x6000;
const NVS_NAMESPACE: Key = Key::from_str("ossm");

pub struct StorageManager {
    nvs: Nvs<FlashStorage<'static>>,
}

pub const OPERATING_MODE_SERVO: &str = "servo";
pub const OPERATING_MODE_RTU_RELAY: &str = "rtu_relay";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PinConfiguration {
    pub modbus_tx: u32,
    pub modbus_rx: u32,
    pub modbus_de_re: u32,
    #[serde(default)]
    pub modbus_timeout_ms: u32,
    #[serde(default)]
    pub modbus_rx_timeout_us: u32,
    #[serde(default)]
    pub modbus_scan_delay_us: u32,
    #[serde(default)]
    pub modbus_inter_frame_delay_us: u32,
    #[serde(default = "default_ble_enabled")]
    pub ble_enabled: bool,
    #[serde(default = "default_operating_mode")]
    pub operating_mode: String,
}

fn default_ble_enabled() -> bool {
    true
}

fn default_operating_mode() -> String {
    String::from(OPERATING_MODE_SERVO)
}

impl PinConfiguration {
    pub fn is_rtu_relay(&self) -> bool {
        self.operating_mode.eq_ignore_ascii_case(OPERATING_MODE_RTU_RELAY)
    }

    pub fn normalize_operating_mode(&mut self) {
        if self.is_rtu_relay() {
            self.operating_mode = String::from(OPERATING_MODE_RTU_RELAY);
        } else {
            self.operating_mode = String::from(OPERATING_MODE_SERVO);
        }
    }
}

impl Default for PinConfiguration {
    fn default() -> Self {
        Self {
            modbus_tx: 18,
            modbus_rx: 19,
            modbus_de_re: 20,
            modbus_timeout_ms: 0,
            modbus_rx_timeout_us: 0,
            modbus_scan_delay_us: 0,
            modbus_inter_frame_delay_us: 0,
            ble_enabled: true,
            operating_mode: default_operating_mode(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkConfiguration {
    #[serde(default = "default_wifi_enabled")]
    pub wifi_enabled: bool,
    #[serde(default)]
    pub ssid: String,
    #[serde(default)]
    pub password: String,
    #[serde(default = "default_hostname")]
    pub hostname: String,
    #[serde(default = "default_dhcp_enabled")]
    pub dhcp_enabled: bool,
    #[serde(default = "default_static_ip")]
    pub static_ip: String,
    #[serde(default = "default_static_mask")]
    pub static_mask: String,
    #[serde(default = "default_static_gateway")]
    pub static_gateway: String,
    #[serde(default = "default_static_dns")]
    pub static_dns: String,
}

fn default_wifi_enabled() -> bool {
    true
}

fn default_hostname() -> String {
    String::from("ossm")
}

fn default_dhcp_enabled() -> bool {
    true
}

fn default_static_ip() -> String {
    String::from("192.168.1.100")
}

fn default_static_mask() -> String {
    String::from("255.255.255.0")
}

fn default_static_gateway() -> String {
    String::from("192.168.1.1")
}

fn default_static_dns() -> String {
    String::from("8.8.8.8")
}

impl Default for NetworkConfiguration {
    fn default() -> Self {
        Self {
            wifi_enabled: default_wifi_enabled(),
            ssid: String::new(),
            password: String::new(),
            hostname: default_hostname(),
            dhcp_enabled: default_dhcp_enabled(),
            static_ip: default_static_ip(),
            static_mask: default_static_mask(),
            static_gateway: default_static_gateway(),
            static_dns: default_static_dns(),
        }
    }
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
