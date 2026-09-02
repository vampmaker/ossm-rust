use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;

/// Desktop OSSM HTTP/WS shell. Flags override environment; environment overrides defaults.
///
/// `.env` in the current directory is loaded first (missing file is ok).
/// Existing device names (`DEVICE_PORT`, `DEVICE_BAUD`) are accepted as aliases.
#[derive(Parser, Debug, Clone)]
#[command(name = "ossm-std", version, about)]
pub struct Args {
    /// RS-485 / USB-UART device (env: OSSM_SERIAL, DEVICE_PORT)
    #[arg(long)]
    pub serial: Option<PathBuf>,

    /// Serial baud (env: OSSM_BAUD, DEVICE_BAUD; default 115200)
    #[arg(long)]
    pub baud: Option<u32>,

    /// HTTP bind address (env: OSSM_BIND; default 127.0.0.1:8080)
    #[arg(long)]
    pub bind: Option<SocketAddr>,

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

    /// Skip 57AIM30 travel homing (env: OSSM_NO_HOMING=1)
    #[arg(long, default_value_t = false)]
    pub no_homing: bool,
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
        if env_truthy("OSSM_NO_HOMING") {
            self.no_homing = true;
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
        if self.serial.is_none() {
            self.mock = true;
        }
        if self.mock {
            self.no_homing = true;
        }
        self
    }

    pub fn is_mock(&self) -> bool {
        self.mock || self.serial.is_none()
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
}

fn default_bind() -> SocketAddr {
    "127.0.0.1:8080".parse().unwrap()
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
            serial: None,
            baud: None,
            bind: None,
            config: None,
            mock: false,
            slave_id: None,
            static_dir: None,
            no_homing: false,
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
        assert!(args.no_homing);
        assert_eq!(args.baud(), 115200);
        assert_eq!(args.bind_addr(), default_bind());
    }
}
