use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::fmt::Write as _;

use crate::command::Command;
use crate::config::{MotorControllerConfig, NetworkConfiguration, PinConfiguration};
use crate::engine::Engine;
use crate::error::CoreError;

pub const PATHS_CATALOG: &str = "\
Config paths (get <path> / set <path> <value>):

pin — full PinConfiguration JSON
  pin.modbus_tx / modbus_rx / modbus_de_re     GPIO 0..48
  pin.modbus_timeout_ms                        0..1000 (0 = default ~10 ms)
  pin.modbus_rx_timeout_us                     0..200000 (0 = auto)
  pin.modbus_scan_delay_us                     0..200000 (0 = t3.5)
  pin.modbus_inter_frame_delay_us              0..200000 (0 = auto)
  pin.ble_enabled / modbus_debug               true|false (debug: reboot)
  pin.operating_mode                           servo|rtu_relay (reboot)

net — full NetworkConfiguration JSON
  net.wifi_enabled / dhcp_enabled              true|false
  net.ssid / password / hostname               string (quote if spaces)
  net.static_ip / static_mask / static_gateway / static_dns   IPv4

motor — full MotorControllerConfig JSON
  motor.bpm                                    > 0
  motor.depth                                  0.01..1
  motor.depth_top / reversed / paused / streaming   true|false
  motor.wave_func                              sine|thrust|spline
  motor.sharpness                              0.01..0.99
  motor.paused_position                        0..1
  motor.spline_points                          space-separated floats
  set motor {\"bpm\":36,...}                   bulk JSON (read-only: version)

inject — RAM fault injection (modbus_debug only)
  get inject                                   mode nbytes
  set inject <off|leading|trailing|both> <nbytes 0..64>

Actions: reset, get-state, get-status, reset-timestamp,
         set-waypoints <json>, append-waypoints <json>";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathError {
    InvalidPath,
    UnknownSection,
    UnknownKey,
    InvalidValue,
    ReadOnly,
    ScalarRequired,
    BulkJsonRequired,
    StaleVersion,
    RelayMode,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath => write!(f, "Invalid path"),
            Self::UnknownSection => write!(f, "Unknown section (try: pin, net, motor, inject)"),
            Self::UnknownKey => write!(f, "Unknown key"),
            Self::InvalidValue => write!(f, "Invalid value"),
            Self::ReadOnly => write!(f, "read-only"),
            Self::ScalarRequired => write!(f, "Use scalar path"),
            Self::BulkJsonRequired => write!(f, "Bulk motor set requires JSON"),
            Self::StaleVersion => write!(f, "Stale causal version"),
            Self::RelayMode => write!(f, "rtu_relay mode"),
        }
    }
}

pub fn parse_path(path: &str) -> Option<(&str, Option<&str>)> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    match path.split_once('.') {
        Some((section, key)) if !section.is_empty() && !key.is_empty() => {
            Some((section, Some(key)))
        }
        _ => Some((path, None)),
    }
}

pub fn parse_bool(value: &str) -> Option<bool> {
    match value.trim() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

pub fn get(engine: &Engine, path: &str) -> Result<String, PathError> {
    let (section, key) = parse_path(path).ok_or(PathError::InvalidPath)?;
    match section {
        "pin" => match key {
            None => serde_json::to_string(engine.pin()).map_err(|_| PathError::InvalidValue),
            Some(k) => pin_field(engine.pin(), k),
        },
        "net" => match key {
            None => serde_json::to_string(engine.net()).map_err(|_| PathError::InvalidValue),
            Some(k) => net_field(engine.net(), k),
        },
        "motor" => match key {
            None => serde_json::to_string(&engine.snapshot().config)
                .map_err(|_| PathError::InvalidValue),
            Some(k) => motor_field(&engine.snapshot().config, k),
        },
        "inject" => Err(PathError::UnknownSection),
        _ => Err(PathError::UnknownSection),
    }
}

pub fn set(engine: &mut Engine, path: &str, value: &str) -> Result<String, PathError> {
    let (section, key) = parse_path(path).ok_or(PathError::InvalidPath)?;
    match section {
        "pin" => {
            let key = key.ok_or(PathError::ScalarRequired)?;
            let mut pin = engine.pin().clone();
            set_pin(&mut pin, key, value)?;
            engine.apply(Command::SetPin(pin));
            Ok(String::from(value))
        }
        "net" => {
            let key = key.ok_or(PathError::ScalarRequired)?;
            let mut net = engine.net().clone();
            set_net(&mut net, key, value)?;
            engine.apply(Command::SetNet(net));
            if key == "password" {
                Ok(String::from("(hidden)"))
            } else {
                Ok(String::from(value))
            }
        }
        "motor" => {
            let Some(key) = key else {
                return Err(PathError::BulkJsonRequired);
            };
            let mut cfg = engine.snapshot().config.clone();
            apply_motor_field(&mut cfg, key, value)?;
            match engine.try_set_config(cfg) {
                Ok(_) => Ok(String::from(value)),
                Err(CoreError::StaleVersion) => Err(PathError::StaleVersion),
                Err(CoreError::RelayMode) => Err(PathError::RelayMode),
                Err(_) => Err(PathError::InvalidValue),
            }
        }
        _ => Err(PathError::UnknownSection),
    }
}

pub fn pin_field(pin: &PinConfiguration, key: &str) -> Result<String, PathError> {
    match key {
        "modbus_tx" => Ok(format!("{}", pin.modbus_tx)),
        "modbus_rx" => Ok(format!("{}", pin.modbus_rx)),
        "modbus_de_re" => Ok(format!("{}", pin.modbus_de_re)),
        "modbus_timeout_ms" => Ok(format!("{}", pin.modbus_timeout_ms)),
        "modbus_rx_timeout_us" => Ok(format!("{}", pin.modbus_rx_timeout_us)),
        "modbus_scan_delay_us" => Ok(format!("{}", pin.modbus_scan_delay_us)),
        "modbus_inter_frame_delay_us" => Ok(format!("{}", pin.modbus_inter_frame_delay_us)),
        "ble_enabled" => Ok(bool_str(pin.ble_enabled)),
        "modbus_debug" => Ok(bool_str(pin.modbus_debug)),
        "operating_mode" => Ok(pin.operating_mode.clone()),
        _ => Err(PathError::UnknownKey),
    }
}

pub fn net_field(net: &NetworkConfiguration, key: &str) -> Result<String, PathError> {
    match key {
        "wifi_enabled" => Ok(bool_str(net.wifi_enabled)),
        "dhcp_enabled" => Ok(bool_str(net.dhcp_enabled)),
        "ssid" => Ok(net.ssid.clone()),
        "password" => Ok(String::from("(hidden)")),
        "hostname" => Ok(net.hostname.clone()),
        "static_ip" => Ok(net.static_ip.clone()),
        "static_mask" => Ok(net.static_mask.clone()),
        "static_gateway" => Ok(net.static_gateway.clone()),
        "static_dns" => Ok(net.static_dns.clone()),
        _ => Err(PathError::UnknownKey),
    }
}

pub fn motor_field(cfg: &MotorControllerConfig, key: &str) -> Result<String, PathError> {
    match key {
        "version" => Ok(format!("{}", cfg.version)),
        "bpm" => Ok(format!("{}", cfg.bpm)),
        "depth" => Ok(format!("{}", cfg.depth)),
        "depth_top" => Ok(bool_str(cfg.depth_top)),
        "reversed" => Ok(bool_str(cfg.reversed)),
        "paused" => Ok(bool_str(cfg.paused)),
        "streaming" => Ok(bool_str(cfg.streaming)),
        "wave_func" => Ok(cfg.wave_func.clone()),
        "sharpness" => Ok(format!("{}", cfg.sharpness)),
        "paused_position" => Ok(format!("{}", cfg.paused_position)),
        "spline_points" => Ok(format_spline(&cfg.spline_points)),
        _ => Err(PathError::UnknownKey),
    }
}

pub fn set_pin(pin: &mut PinConfiguration, key: &str, value: &str) -> Result<(), PathError> {
    match key {
        "modbus_tx" | "modbus_rx" | "modbus_de_re" => {
            let gpio = parse_gpio(value)?;
            match key {
                "modbus_tx" => pin.modbus_tx = gpio,
                "modbus_rx" => pin.modbus_rx = gpio,
                _ => pin.modbus_de_re = gpio,
            }
            Ok(())
        }
        "modbus_timeout_ms" => {
            let v = parse_u32_max(value, 1000)?;
            pin.modbus_timeout_ms = v;
            Ok(())
        }
        "modbus_rx_timeout_us" | "modbus_scan_delay_us" | "modbus_inter_frame_delay_us" => {
            let v = parse_u32_max(value, 200_000)?;
            match key {
                "modbus_rx_timeout_us" => pin.modbus_rx_timeout_us = v,
                "modbus_scan_delay_us" => pin.modbus_scan_delay_us = v,
                _ => pin.modbus_inter_frame_delay_us = v,
            }
            Ok(())
        }
        "ble_enabled" | "modbus_debug" => {
            let v = parse_bool(value).ok_or(PathError::InvalidValue)?;
            match key {
                "ble_enabled" => pin.ble_enabled = v,
                _ => pin.modbus_debug = v,
            }
            Ok(())
        }
        "operating_mode" => {
            if !matches!(value, "servo" | "rtu_relay") {
                return Err(PathError::InvalidValue);
            }
            pin.operating_mode = String::from(value);
            Ok(())
        }
        _ => Err(PathError::UnknownKey),
    }
}

pub fn set_net(net: &mut NetworkConfiguration, key: &str, value: &str) -> Result<(), PathError> {
    match key {
        "wifi_enabled" => {
            net.wifi_enabled = parse_bool(value).ok_or(PathError::InvalidValue)?;
            Ok(())
        }
        "dhcp_enabled" => {
            net.dhcp_enabled = parse_bool(value).ok_or(PathError::InvalidValue)?;
            Ok(())
        }
        "ssid" => {
            net.ssid = String::from(value);
            Ok(())
        }
        "password" => {
            net.password = String::from(value);
            Ok(())
        }
        "hostname" => {
            net.hostname = String::from(value);
            Ok(())
        }
        "static_ip" => {
            net.static_ip = String::from(value);
            Ok(())
        }
        "static_mask" => {
            net.static_mask = String::from(value);
            Ok(())
        }
        "static_gateway" => {
            net.static_gateway = String::from(value);
            Ok(())
        }
        "static_dns" => {
            net.static_dns = String::from(value);
            Ok(())
        }
        _ => Err(PathError::UnknownKey),
    }
}

pub fn apply_motor_field(
    cfg: &mut MotorControllerConfig,
    key: &str,
    value: &str,
) -> Result<(), PathError> {
    match key {
        "version" => Err(PathError::ReadOnly),
        "bpm" => {
            let v = value.parse::<f32>().map_err(|_| PathError::InvalidValue)?;
            if v <= 0.0 {
                return Err(PathError::InvalidValue);
            }
            cfg.bpm = v;
            Ok(())
        }
        "depth" => {
            let v = value.parse::<f32>().map_err(|_| PathError::InvalidValue)?;
            if !(0.01..=1.0).contains(&v) {
                return Err(PathError::InvalidValue);
            }
            cfg.depth = v;
            Ok(())
        }
        "depth_top" => {
            cfg.depth_top = parse_bool(value).ok_or(PathError::InvalidValue)?;
            Ok(())
        }
        "reversed" => {
            cfg.reversed = parse_bool(value).ok_or(PathError::InvalidValue)?;
            Ok(())
        }
        "paused" => {
            cfg.paused = parse_bool(value).ok_or(PathError::InvalidValue)?;
            Ok(())
        }
        "streaming" => {
            cfg.streaming = parse_bool(value).ok_or(PathError::InvalidValue)?;
            Ok(())
        }
        "wave_func" => {
            if !matches!(value, "sine" | "thrust" | "spline") {
                return Err(PathError::InvalidValue);
            }
            cfg.wave_func = String::from(value);
            Ok(())
        }
        "sharpness" => {
            let v = value.parse::<f32>().map_err(|_| PathError::InvalidValue)?;
            if !(0.01..=0.99).contains(&v) {
                return Err(PathError::InvalidValue);
            }
            cfg.sharpness = v;
            Ok(())
        }
        "paused_position" => {
            let v = value.parse::<f32>().map_err(|_| PathError::InvalidValue)?;
            if !(0.0..=1.0).contains(&v) {
                return Err(PathError::InvalidValue);
            }
            cfg.paused_position = v;
            Ok(())
        }
        "spline_points" => {
            let parsed: Result<Vec<f32>, _> =
                value.split_whitespace().map(|s| s.parse::<f32>()).collect();
            cfg.spline_points = parsed.map_err(|_| PathError::InvalidValue)?;
            Ok(())
        }
        _ => Err(PathError::UnknownKey),
    }
}

fn bool_str(v: bool) -> String {
    String::from(if v { "true" } else { "false" })
}

fn format_spline(pts: &[f32]) -> String {
    let mut s = String::new();
    for (i, p) in pts.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let _ = write!(s, "{}", p);
    }
    s
}

fn parse_gpio(value: &str) -> Result<u32, PathError> {
    let pin = value.parse::<u32>().map_err(|_| PathError::InvalidValue)?;
    if pin > 48 {
        return Err(PathError::InvalidValue);
    }
    Ok(pin)
}

fn parse_u32_max(value: &str, max: u32) -> Result<u32, PathError> {
    let v = value.parse::<u32>().map_err(|_| PathError::InvalidValue)?;
    if v > max {
        return Err(PathError::InvalidValue);
    }
    Ok(v)
}
