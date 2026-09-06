use alloc::string::String;

use esp_nvs::{Key, Nvs};
use esp_storage::FlashStorage;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{FirmwareError, Result};
use crate::motion::MotorControllerConfig;

pub const OPERATING_MODE_SERVO: &str = "servo";
pub const OPERATING_MODE_RTU_RELAY: &str = "rtu_relay";
pub const OPERATING_MODE_RS485: &str = "rs485";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperatingMode {
    Servo,
    RtuRelay,
    Rs485,
}

/// GPIO / UART / BLE slice for `CHAR_PIN_CONFIG` (ATT size).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
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
    #[serde(default = "default_modbus_baud")]
    pub modbus_baud: u32,
    #[serde(default = "default_ble_enabled")]
    pub ble_enabled: bool,
    #[serde(default)]
    pub modbus_debug: bool,
    #[serde(default = "default_operating_mode")]
    pub operating_mode: String,
}

/// Full firmware shell persist / `GET /shell-config` body.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ShellConfig {
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
    #[serde(default = "default_modbus_baud")]
    pub modbus_baud: u32,
    #[serde(default = "default_ble_enabled")]
    pub ble_enabled: bool,
    #[serde(default)]
    pub modbus_debug: bool,
    #[serde(default = "default_operating_mode")]
    pub operating_mode: String,
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

fn default_ble_enabled() -> bool {
    true
}

fn default_operating_mode() -> String {
    String::from(OPERATING_MODE_SERVO)
}

fn default_modbus_baud() -> u32 {
    115_200
}

pub fn parse_operating_mode(value: &str) -> Option<OperatingMode> {
    if value.eq_ignore_ascii_case(OPERATING_MODE_SERVO) {
        Some(OperatingMode::Servo)
    } else if value.eq_ignore_ascii_case(OPERATING_MODE_RTU_RELAY)
        || value.eq_ignore_ascii_case("rtu-relay")
    {
        Some(OperatingMode::RtuRelay)
    } else if value.eq_ignore_ascii_case(OPERATING_MODE_RS485) {
        Some(OperatingMode::Rs485)
    } else {
        None
    }
}

impl ShellConfig {
    pub fn mode(&self) -> OperatingMode {
        parse_operating_mode(&self.operating_mode).unwrap_or(OperatingMode::Servo)
    }

    pub fn normalize_operating_mode(&mut self) {
        self.operating_mode = String::from(match self.mode() {
            OperatingMode::Servo => OPERATING_MODE_SERVO,
            OperatingMode::RtuRelay => OPERATING_MODE_RTU_RELAY,
            OperatingMode::Rs485 => OPERATING_MODE_RS485,
        });
        if self.modbus_baud == 0 {
            self.modbus_baud = default_modbus_baud();
        }
    }

    pub fn uart_needs_restart(&self, other: &Self) -> bool {
        self.mode() != other.mode()
            || self.modbus_tx != other.modbus_tx
            || self.modbus_rx != other.modbus_rx
            || self.modbus_de_re != other.modbus_de_re
            || self.modbus_timeout_ms != other.modbus_timeout_ms
            || self.modbus_rx_timeout_us != other.modbus_rx_timeout_us
            || self.modbus_scan_delay_us != other.modbus_scan_delay_us
            || self.modbus_inter_frame_delay_us != other.modbus_inter_frame_delay_us
            || self.modbus_baud != other.modbus_baud
    }

    pub fn pin_slice(&self) -> PinConfiguration {
        PinConfiguration {
            modbus_tx: self.modbus_tx,
            modbus_rx: self.modbus_rx,
            modbus_de_re: self.modbus_de_re,
            modbus_timeout_ms: self.modbus_timeout_ms,
            modbus_rx_timeout_us: self.modbus_rx_timeout_us,
            modbus_scan_delay_us: self.modbus_scan_delay_us,
            modbus_inter_frame_delay_us: self.modbus_inter_frame_delay_us,
            modbus_baud: self.modbus_baud,
            ble_enabled: self.ble_enabled,
            modbus_debug: self.modbus_debug,
            operating_mode: self.operating_mode.clone(),
        }
    }

    pub fn net_slice(&self) -> NetworkConfiguration {
        NetworkConfiguration {
            wifi_enabled: self.wifi_enabled,
            ssid: self.ssid.clone(),
            password: self.password.clone(),
            hostname: self.hostname.clone(),
            dhcp_enabled: self.dhcp_enabled,
            static_ip: self.static_ip.clone(),
            static_mask: self.static_mask.clone(),
            static_gateway: self.static_gateway.clone(),
            static_dns: self.static_dns.clone(),
        }
    }

    pub fn apply_pin(&mut self, pin: PinConfiguration) {
        self.modbus_tx = pin.modbus_tx;
        self.modbus_rx = pin.modbus_rx;
        self.modbus_de_re = pin.modbus_de_re;
        self.modbus_timeout_ms = pin.modbus_timeout_ms;
        self.modbus_rx_timeout_us = pin.modbus_rx_timeout_us;
        self.modbus_scan_delay_us = pin.modbus_scan_delay_us;
        self.modbus_inter_frame_delay_us = pin.modbus_inter_frame_delay_us;
        self.modbus_baud = pin.modbus_baud;
        self.ble_enabled = pin.ble_enabled;
        self.modbus_debug = pin.modbus_debug;
        self.operating_mode = pin.operating_mode;
        self.normalize_operating_mode();
    }

    pub fn apply_net(&mut self, net: NetworkConfiguration) {
        self.wifi_enabled = net.wifi_enabled;
        self.ssid = net.ssid;
        self.password = net.password;
        self.hostname = net.hostname;
        self.dhcp_enabled = net.dhcp_enabled;
        self.static_ip = net.static_ip;
        self.static_mask = net.static_mask;
        self.static_gateway = net.static_gateway;
        self.static_dns = net.static_dns;
    }
}

impl Default for ShellConfig {
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
            modbus_debug: false,
            modbus_baud: default_modbus_baud(),
            operating_mode: default_operating_mode(),
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

impl Default for PinConfiguration {
    fn default() -> Self {
        ShellConfig::default().pin_slice()
    }
}

impl PinConfiguration {
    pub fn mode(&self) -> OperatingMode {
        parse_operating_mode(&self.operating_mode).unwrap_or(OperatingMode::Servo)
    }

    pub fn is_servo(&self) -> bool {
        self.mode() == OperatingMode::Servo
    }

    pub fn is_rtu_relay(&self) -> bool {
        self.mode() == OperatingMode::RtuRelay
    }

    pub fn is_rs485(&self) -> bool {
        self.mode() == OperatingMode::Rs485
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
        let mut config = *config;
        config.depth = config.depth.clamp(0.0, 1.0);
        config.bpm = config.bpm.clamp(1.0, 500.0);
        config.sharpness = config.sharpness.clamp(0.0, 1.0);
        config.paused_position = config.paused_position.clamp(0.0, 1.0);
        self.set_json("motor_config", &config)
    }

    pub fn get_motor_config(&mut self) -> Result<MotorControllerConfig> {
        self.get_json("motor_config")
    }

    pub fn set_shell_configuration(&mut self, config: &ShellConfig) -> Result<()> {
        let mut config = config.clone();
        config.normalize_operating_mode();
        if !config.ssid.is_empty() {
            let _ = self.set_ssid(&config.ssid);
        }
        if !config.password.is_empty() {
            let _ = self.set_password(&config.password);
        }
        self.set_json("shell_conf", &config)
    }

    pub fn get_shell_configuration(&mut self) -> Result<ShellConfig> {
        if let Ok(mut config) = self.get_json::<ShellConfig>("shell_conf") {
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
            config.normalize_operating_mode();
            return Ok(config);
        }
        let mut config = ShellConfig::default();
        if let Ok(pin) = self.get_json::<PinConfiguration>("pin_conf") {
            config.apply_pin(pin);
        }
        if let Ok(net) = self.get_json::<NetworkConfiguration>("net_conf") {
            config.apply_net(net);
        } else {
            if let Ok(ssid) = self.get_ssid() {
                config.ssid = ssid;
            }
            if let Ok(password) = self.get_password() {
                config.password = password;
            }
        }
        config.normalize_operating_mode();
        Ok(config)
    }
}
