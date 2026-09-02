//! Firmware-only pin/net CLI path helpers (GPIO / WiFi). Motor paths stay in `ossm_core::paths`.

use alloc::format;
use alloc::string::String;

use ossm_core::paths::{parse_bool, PathError};

use crate::storage::{NetworkConfiguration, PinConfiguration};

pub const HW_PATHS_CATALOG: &str = "\
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

inject — RAM fault injection (modbus_debug only)
  get inject                                   mode nbytes
  set inject <off|leading|trailing|both> <nbytes 0..64>";

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

fn bool_str(v: bool) -> String {
    String::from(if v { "true" } else { "false" })
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
