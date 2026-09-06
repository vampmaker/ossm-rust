//! Firmware-only shell CLI path helpers (GPIO / WiFi). Motor paths stay in `ossm_core::paths`.

use alloc::format;
use alloc::string::String;

use ossm_core::paths::{parse_bool, PathError};

use crate::storage::ShellConfig;

pub const HW_PATHS_CATALOG: &str = "\
shell — full ShellConfig JSON
  shell.modbus_tx / modbus_rx / modbus_de_re   GPIO 0..48
  shell.modbus_timeout_ms                      0..1000 (0 = default ~10 ms)
  shell.modbus_rx_timeout_us                   0..200000 (0 = auto)
  shell.modbus_scan_delay_us                   0..200000 (0 = t3.5)
  shell.modbus_inter_frame_delay_us            0..200000 (0 = auto)
  shell.ble_enabled / modbus_debug             true|false (debug: reboot)
  shell.operating_mode                         servo|rtu_relay|rs485
  shell.modbus_baud                            1200..3000000 (default 115200)
  shell.wifi_enabled / dhcp_enabled            true|false
  shell.ssid / password / hostname             string (quote if spaces)
  shell.static_ip / static_mask / static_gateway / static_dns   IPv4
  aliases: pin.* and net.* (ACK is shell.<key>)

inject — RAM fault injection (modbus_debug only)
  get inject                                   mode nbytes
  set inject <off|leading|trailing|both> <nbytes 0..64>";

pub fn shell_field(shell: &ShellConfig, key: &str) -> Result<String, PathError> {
    match key {
        "modbus_tx" => Ok(format!("{}", shell.modbus_tx)),
        "modbus_rx" => Ok(format!("{}", shell.modbus_rx)),
        "modbus_de_re" => Ok(format!("{}", shell.modbus_de_re)),
        "modbus_timeout_ms" => Ok(format!("{}", shell.modbus_timeout_ms)),
        "modbus_rx_timeout_us" => Ok(format!("{}", shell.modbus_rx_timeout_us)),
        "modbus_scan_delay_us" => Ok(format!("{}", shell.modbus_scan_delay_us)),
        "modbus_inter_frame_delay_us" => Ok(format!("{}", shell.modbus_inter_frame_delay_us)),
        "ble_enabled" => Ok(bool_str(shell.ble_enabled)),
        "modbus_debug" => Ok(bool_str(shell.modbus_debug)),
        "operating_mode" => Ok(shell.operating_mode.clone()),
        "modbus_baud" => Ok(format!("{}", shell.modbus_baud)),
        "wifi_enabled" => Ok(bool_str(shell.wifi_enabled)),
        "dhcp_enabled" => Ok(bool_str(shell.dhcp_enabled)),
        "ssid" => Ok(shell.ssid.clone()),
        "password" => Ok(String::from("(hidden)")),
        "hostname" => Ok(shell.hostname.clone()),
        "static_ip" => Ok(shell.static_ip.clone()),
        "static_mask" => Ok(shell.static_mask.clone()),
        "static_gateway" => Ok(shell.static_gateway.clone()),
        "static_dns" => Ok(shell.static_dns.clone()),
        _ => Err(PathError::UnknownKey),
    }
}

pub fn set_shell(shell: &mut ShellConfig, key: &str, value: &str) -> Result<(), PathError> {
    match key {
        "modbus_tx" | "modbus_rx" | "modbus_de_re" => {
            let gpio = parse_gpio(value)?;
            match key {
                "modbus_tx" => shell.modbus_tx = gpio,
                "modbus_rx" => shell.modbus_rx = gpio,
                _ => shell.modbus_de_re = gpio,
            }
            Ok(())
        }
        "modbus_timeout_ms" => {
            shell.modbus_timeout_ms = parse_u32_max(value, 1000)?;
            Ok(())
        }
        "modbus_rx_timeout_us" | "modbus_scan_delay_us" | "modbus_inter_frame_delay_us" => {
            let v = parse_u32_max(value, 200_000)?;
            match key {
                "modbus_rx_timeout_us" => shell.modbus_rx_timeout_us = v,
                "modbus_scan_delay_us" => shell.modbus_scan_delay_us = v,
                _ => shell.modbus_inter_frame_delay_us = v,
            }
            Ok(())
        }
        "ble_enabled" | "modbus_debug" => {
            let v = parse_bool(value).ok_or(PathError::InvalidValue)?;
            match key {
                "ble_enabled" => shell.ble_enabled = v,
                _ => shell.modbus_debug = v,
            }
            Ok(())
        }
        "operating_mode" => {
            if crate::storage::parse_operating_mode(value).is_none() {
                return Err(PathError::InvalidValue);
            }
            shell.operating_mode = String::from(value);
            Ok(())
        }
        "modbus_baud" => {
            let v = parse_u32_max(value, 3_000_000)?;
            if v != 0 && v < 1200 {
                return Err(PathError::InvalidValue);
            }
            shell.modbus_baud = if v == 0 { 115_200 } else { v };
            Ok(())
        }
        "wifi_enabled" => {
            shell.wifi_enabled = parse_bool(value).ok_or(PathError::InvalidValue)?;
            Ok(())
        }
        "dhcp_enabled" => {
            shell.dhcp_enabled = parse_bool(value).ok_or(PathError::InvalidValue)?;
            Ok(())
        }
        "ssid" => {
            shell.ssid = String::from(value);
            Ok(())
        }
        "password" => {
            shell.password = String::from(value);
            Ok(())
        }
        "hostname" => {
            shell.hostname = String::from(value);
            Ok(())
        }
        "static_ip" => {
            shell.static_ip = String::from(value);
            Ok(())
        }
        "static_mask" => {
            shell.static_mask = String::from(value);
            Ok(())
        }
        "static_gateway" => {
            shell.static_gateway = String::from(value);
            Ok(())
        }
        "static_dns" => {
            shell.static_dns = String::from(value);
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
