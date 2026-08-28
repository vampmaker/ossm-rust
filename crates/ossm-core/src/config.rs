use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

pub const OPERATING_MODE_SERVO: &str = "servo";
pub const OPERATING_MODE_RTU_RELAY: &str = "rtu_relay";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MotorControllerConfig {
    #[serde(default)]
    pub version: u32,
    pub bpm: f32,
    pub depth: f32,
    pub depth_top: bool,   // true = top [0, depth], false = bottom [1-depth, 1]
    pub reversed: bool,    // reverse waveform direction
    pub wave_func: String, // "sine", "thrust", or "spline"
    pub sharpness: f32,    // For thrust waveform: rise duration (0.01-0.99), higher = longer rise
    #[serde(default)]
    pub spline_points: Vec<f32>,
    pub paused: bool,
    pub paused_position: f32,
    #[serde(default)]
    pub streaming: bool,
}

impl MotorControllerConfig {
    /// Copy fields from `src`, reusing heap capacity where possible.
    pub fn copy_from(&mut self, src: &MotorControllerConfig) {
        self.version = src.version;
        self.bpm = src.bpm;
        self.depth = src.depth;
        self.depth_top = src.depth_top;
        self.reversed = src.reversed;
        self.wave_func.clone_from(&src.wave_func);
        self.sharpness = src.sharpness;
        self.spline_points.clone_from(&src.spline_points);
        self.paused = src.paused;
        self.paused_position = src.paused_position;
        self.streaming = src.streaming;
    }
}

impl Default for MotorControllerConfig {
    fn default() -> Self {
        Self {
            version: 0,
            bpm: 36.0,
            depth: 1.0,
            depth_top: false,
            reversed: false,
            wave_func: String::from("sine"),
            sharpness: 0.3,
            spline_points: vec![0.0, 1.0],
            paused: true,
            paused_position: 0.0,
            streaming: false,
        }
    }
}

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
    /// Diagnostic Modbus RX mode (5 ms deadline + MODBUS_DBG class/base64; 256 B DMA). Requires reboot.
    #[serde(default)]
    pub modbus_debug: bool,
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
        self.operating_mode
            .eq_ignore_ascii_case(OPERATING_MODE_RTU_RELAY)
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
            modbus_debug: false,
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
