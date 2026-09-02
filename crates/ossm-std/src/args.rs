use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, ValueEnum};

/// Desktop OSSM HTTP/WS shell. Flags override environment; environment overrides defaults.
///
/// `.env` in the current directory is loaded first (missing file is ok).
/// Existing device names (`DEVICE_PORT`, `DEVICE_BAUD`) are accepted as aliases.
#[derive(Parser, Debug, Clone)]
#[command(name = "ossm-std", version, about)]
pub struct Args {
    /// Process role: servo (Engine is RTU master) or rtu-relay (serve :502 + /ws/modbus)
    #[arg(long, value_enum, default_value_t = StdMode::Servo)]
    pub mode: StdMode,

    /// RS-485 / USB-UART device (env: OSSM_SERIAL, DEVICE_PORT).
    /// Requires automatic DE/RE direction control; TX-loopback adapters are not supported.
    #[arg(long)]
    pub serial: Option<PathBuf>,

    /// Serial baud (env: OSSM_BAUD, DEVICE_BAUD; default 115200)
    #[arg(long)]
    pub baud: Option<u32>,

    /// HTTP bind address (env: OSSM_BIND; default 127.0.0.1:8080)
    #[arg(long)]
    pub bind: Option<SocketAddr>,

    /// Modbus TCP bind when `--mode rtu-relay` (env: OSSM_MODBUS_BIND; default 127.0.0.1:502)
    #[arg(long)]
    pub modbus_bind: Option<SocketAddr>,

    /// Modbus TCP client `HOST:502` (firmware or another ossm-std relay)
    #[arg(long)]
    pub relay_tcp: Option<String>,

    /// Binary RTU WebSocket `ws://HOST/ws/modbus`
    #[arg(long)]
    pub relay_ws: Option<String>,

    /// Raw RS-485 WebSocket `ws://HOST/ws/rs485`
    #[arg(long)]
    pub rs485_ws: Option<String>,

    /// Atomic `{pin,net,motor}` JSON path (env: OSSM_CONFIG; default ossm-config.json)
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Skip UART; tick Engine and publish `/state` (env: OSSM_MOCK=1)
    #[arg(long, default_value_t = false)]
    pub mock: bool,

    /// Modbus slave id (env: OSSM_SLAVE_ID; default 1)
    #[arg(long)]
    pub slave_id: Option<u8>,

    /// Directory containing `index.html` (env: OSSM_STATIC_DIR)
    #[arg(long)]
    pub static_dir: Option<PathBuf>,

    /// Force stdin REPL even when stdin is not a TTY (env: OSSM_REPL=1)
    #[arg(long, default_value_t = false)]
    pub repl: bool,

    /// Disable stdin REPL even on a TTY (env: OSSM_NO_REPL=1)
    #[arg(long, default_value_t = false)]
    pub no_repl: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum StdMode {
    #[default]
    Servo,
    #[value(name = "rtu-relay")]
    RtuRelay,
}

impl Args {
    pub fn parse_from_env() -> Self {
        let _ = dotenvy::from_filename(".env");
        Self::parse().finalize()
    }

    pub fn finalize(mut self) -> Self {
        if self.serial.is_none() {
            if let Some(v) = env_first(&["OSSM_SERIAL", "DEVICE_PORT"]) {
                self.serial = Some(PathBuf::from(v));
            }
        }
        if self.baud.is_none() {
            if let Some(v) = env_first(&["OSSM_BAUD", "DEVICE_BAUD"]) {
                self.baud = v.parse().ok();
            }
        }
        if self.bind.is_none() {
            if let Some(v) = env_first(&["OSSM_BIND"]) {
                self.bind = v.parse().ok();
            }
        }
        if self.config.is_none() {
            if let Some(v) = env_first(&["OSSM_CONFIG"]) {
                self.config = Some(PathBuf::from(v));
            }
        }
        if self.modbus_bind.is_none() {
            if let Some(v) = env_first(&["OSSM_MODBUS_BIND"]) {
                self.modbus_bind = v.parse().ok();
            }
        }
        if self.relay_tcp.is_none() {
            if let Some(v) = env_first(&["OSSM_RELAY_TCP"]) {
                self.relay_tcp = Some(v);
            }
        }
        if self.relay_ws.is_none() {
            if let Some(v) = env_first(&["OSSM_RELAY_WS"]) {
                self.relay_ws = Some(v);
            }
        }
        if self.rs485_ws.is_none() {
            if let Some(v) = env_first(&["OSSM_RS485_WS"]) {
                self.rs485_ws = Some(v);
            }
        }
        if let Some(v) = env_first(&["OSSM_MODE"]) {
            match v.to_ascii_lowercase().as_str() {
                "rtu-relay" | "rtu_relay" => self.mode = StdMode::RtuRelay,
                "servo" => self.mode = StdMode::Servo,
                _ => {}
            }
        }
        if env_truthy("OSSM_MOCK") {
            self.mock = true;
        }
        if self.slave_id.is_none() {
            if let Some(v) = env_first(&["OSSM_SLAVE_ID"]) {
                self.slave_id = v.parse().ok();
            }
        }
        if self.static_dir.is_none() {
            if let Some(v) = env_first(&["OSSM_STATIC_DIR"]) {
                self.static_dir = Some(PathBuf::from(v));
            }
        }
        if env_truthy("OSSM_REPL") {
            self.repl = true;
        }
        if env_truthy("OSSM_NO_REPL") {
            self.no_repl = true;
        }

        if self.baud.is_none() {
            self.baud = Some(115200);
        }
        if self.bind.is_none() {
            self.bind = Some(default_bind());
        }
        if self.config.is_none() {
            self.config = Some(PathBuf::from("ossm-config.json"));
        }
        if self.slave_id.is_none() {
            self.slave_id = Some(1);
        }
        if self.static_dir.is_none() {
            self.static_dir = Some(default_static_dir());
        }
        if self.modbus_bind.is_none() {
            self.modbus_bind = Some(default_modbus_bind());
        }
        if self.serial.is_none()
            && self.relay_tcp.is_none()
            && self.relay_ws.is_none()
            && self.rs485_ws.is_none()
        {
            self.mock = true;
        }
        self
    }

    pub fn is_mock(&self) -> bool {
        self.mock
            || (self.serial.is_none()
                && self.relay_tcp.is_none()
                && self.relay_ws.is_none()
                && self.rs485_ws.is_none())
    }

    pub fn is_rtu_relay(&self) -> bool {
        self.mode == StdMode::RtuRelay
    }

    pub fn has_remote_bus(&self) -> bool {
        self.relay_tcp.is_some() || self.relay_ws.is_some() || self.rs485_ws.is_some()
    }

    /// Interactive stdin CLI. Default: on when stdin is a TTY.
    pub fn want_repl(&self) -> bool {
        if self.no_repl {
            false
        } else if self.repl {
            true
        } else {
            std::io::IsTerminal::is_terminal(&std::io::stdin())
        }
    }

    pub fn baud(&self) -> u32 {
        self.baud.unwrap_or(115200)
    }

    pub fn bind_addr(&self) -> SocketAddr {
        self.bind.unwrap_or_else(default_bind)
    }

    pub fn config_path(&self) -> PathBuf {
        self.config
            .clone()
            .unwrap_or_else(|| PathBuf::from("ossm-config.json"))
    }

    pub fn slave_id(&self) -> u8 {
        self.slave_id.unwrap_or(1)
    }

    pub fn modbus_bind_addr(&self) -> SocketAddr {
        self.modbus_bind.unwrap_or_else(default_modbus_bind)
    }
}

fn default_bind() -> SocketAddr {
    "127.0.0.1:8080".parse().unwrap()
}

fn default_modbus_bind() -> SocketAddr {
    "127.0.0.1:502".parse().unwrap()
}

pub fn default_static_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../frontend/dist")
}

fn env_first(keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Ok(raw) = std::env::var(key) {
            let v = strip_env(&raw);
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

fn env_truthy(key: &str) -> bool {
    env_first(&[key])
        .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

fn strip_env(raw: &str) -> String {
    raw.trim().trim_matches('"').trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_serial_implies_mock() {
        let saved_port = std::env::var("DEVICE_PORT").ok();
        let saved_serial = std::env::var("OSSM_SERIAL").ok();
        std::env::remove_var("DEVICE_PORT");
        std::env::remove_var("OSSM_SERIAL");

        let args = Args {
            mode: StdMode::Servo,
            serial: None,
            baud: None,
            bind: None,
            modbus_bind: None,
            relay_tcp: None,
            relay_ws: None,
            rs485_ws: None,
            config: None,
            mock: false,
            slave_id: None,
            static_dir: None,
            repl: false,
            no_repl: false,
        }
        .finalize();

        match saved_port {
            Some(v) => std::env::set_var("DEVICE_PORT", v),
            None => std::env::remove_var("DEVICE_PORT"),
        }
        match saved_serial {
            Some(v) => std::env::set_var("OSSM_SERIAL", v),
            None => std::env::remove_var("OSSM_SERIAL"),
        }

        assert!(args.is_mock());
        assert_eq!(args.baud(), 115200);
        assert_eq!(args.bind_addr(), default_bind());
    }
}
