use serde::Serialize;

use crate::args::Args;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeMode {
    Servo,
    #[serde(rename = "rtu-relay")]
    RtuRelay,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RuntimeConfigResponse {
    pub mode: RuntimeMode,
    pub mock: bool,
    pub serial: Option<String>,
    pub baud: u32,
    pub slave_id: u8,
    pub bind: String,
    pub modbus_bind: String,
    pub relay_tcp: Option<String>,
    pub relay_ws: Option<String>,
    pub rs485_ws: Option<String>,
    pub config_path: String,
    pub static_dir: Option<String>,
    pub repl: bool,
    pub no_repl: bool,
    pub transport: String,
}

impl RuntimeConfigResponse {
    pub fn from_args(args: &Args) -> Self {
        let mode = if args.is_rtu_relay() {
            RuntimeMode::RtuRelay
        } else {
            RuntimeMode::Servo
        };
        let transport = describe_transport(args);
        Self {
            mode,
            mock: args.is_mock(),
            serial: args
                .serial
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            baud: args.baud(),
            slave_id: args.slave_id(),
            bind: args.bind_addr().to_string(),
            modbus_bind: args.modbus_bind_addr().to_string(),
            relay_tcp: args.relay_tcp.clone(),
            relay_ws: args.relay_ws.clone(),
            rs485_ws: args.rs485_ws.clone(),
            config_path: args.config_path().to_string_lossy().into_owned(),
            static_dir: args
                .static_dir
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            repl: args.repl,
            no_repl: args.no_repl,
            transport,
        }
    }
}

fn describe_transport(args: &Args) -> String {
    if args.is_mock() && !args.has_remote_bus() && args.serial.is_none() {
        return "mock".into();
    }
    if let Some(url) = &args.rs485_ws {
        return format!("rs485-ws:{url}");
    }
    if let Some(url) = &args.relay_ws {
        return format!("relay-ws:{url}");
    }
    if let Some(host) = &args.relay_tcp {
        return format!("relay-tcp:{host}");
    }
    if let Some(path) = &args.serial {
        return format!("serial:{}", path.display());
    }
    "none".into()
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::path::PathBuf;

    use super::*;
    use crate::args::{Args, StdMode};

    #[test]
    fn mock_transport_label() {
        let args = Args {
            mode: StdMode::Servo,
            serial: None,
            baud: None,
            image: None,
            flash_offset: 0,
            flash_size: "4mb".into(),
            keep_nvs: false,
            no_verify: false,
            force: false,
            usb_jtag: false,
            send: Vec::new(),
            until: None,
            timeout: None,
            reset: false,
            exit_rs485: false,
            raw: false,
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
        let cfg = RuntimeConfigResponse::from_args(&args);
        assert!(cfg.mock);
        assert_eq!(cfg.transport, "mock");
        assert_eq!(cfg.mode, RuntimeMode::Servo);
    }

    #[test]
    fn serial_transport_label() {
        let args = Args {
            mode: StdMode::Servo,
            serial: Some(PathBuf::from("/dev/ttyUSB0")),
            baud: Some(115200),
            image: None,
            flash_offset: 0,
            flash_size: "4mb".into(),
            keep_nvs: false,
            no_verify: false,
            force: false,
            usb_jtag: false,
            send: Vec::new(),
            until: None,
            timeout: None,
            reset: false,
            exit_rs485: false,
            raw: false,
            bind: Some("127.0.0.1:8080".parse::<SocketAddr>().unwrap()),
            modbus_bind: None,
            relay_tcp: None,
            relay_ws: None,
            rs485_ws: None,
            config: Some(PathBuf::from("cfg.json")),
            mock: false,
            slave_id: Some(1),
            static_dir: None,
            repl: false,
            no_repl: true,
        }
        .finalize();
        let cfg = RuntimeConfigResponse::from_args(&args);
        assert!(!cfg.mock);
        assert_eq!(cfg.serial.as_deref(), Some("/dev/ttyUSB0"));
        assert_eq!(cfg.transport, "serial:/dev/ttyUSB0");
        assert!(cfg.no_repl);
    }
}
