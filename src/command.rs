use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

use embassy_sync::blocking_mutex::{raw::CriticalSectionRawMutex, Mutex as BlockingMutex};
use embassy_sync::channel::Channel;
use embedded_cli::cli::CliBuilder;
use embedded_cli::Command;
use embedded_io::ErrorType;
use embedded_io::Write as EmbeddedWrite;

use crate::console;
use crate::context::AppContext;
use crate::http_api::WaypointsInput;
use crate::motion::{MotionCommand, MotorControllerConfig};

static CLI_CHANNEL: Channel<CriticalSectionRawMutex, CliCommand, 4> = Channel::new();
static CLI_OUTPUT: BlockingMutex<CriticalSectionRawMutex, Vec<u8>> = BlockingMutex::new(Vec::new());

#[derive(Clone)]
enum CliCommand {
    Get(String),
    Set { path: String, value: String },
    Paths,
    Reset,
    GetState,
    GetStatus,
    ResetTimestamp,
    SetWaypoints(String),
    AppendWaypoints(String),
}

/// nmcli-style configuration CLI.
#[derive(Command, Clone, Debug)]
#[command(help_title = "ossm-rust — get/set config paths (pin, net, motor, inject)")]
enum BaseCommand<'a> {
    /// Get config section JSON or scalar: get <path>
    /// Sections: pin, net, motor, inject | Keys: pin.modbus_tx, net.ssid, motor.bpm, ...
    /// Type help paths for the full path/value catalog.
    Get { path: &'a str },
    /// Set config: set <path> <value>  (quote strings with spaces)
    /// Bulk motor JSON: set motor {"bpm":36,...}
    /// Inject: set inject <off|leading|trailing|both> <nbytes 0..64>
    Set { path: &'a str, value: &'a str },
    /// Print config path catalog (types, enums, ranges)
    Paths,
    Reset,
    GetState,
    GetStatus,
    ResetTimestamp,
    SetWaypoints { json: &'a str },
    AppendWaypoints { json: &'a str },
}

struct CliOutputWriter;

impl ErrorType for CliOutputWriter {
    type Error = core::convert::Infallible;
}

impl EmbeddedWrite for CliOutputWriter {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        unsafe {
            CLI_OUTPUT.lock_mut(|out| {
                out.extend_from_slice(buf);
            });
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn queue_command(cmd: CliCommand) {
    let _ = CLI_CHANNEL.try_send(cmd);
}

fn flush_cli_output() {
    let pending = unsafe { CLI_OUTPUT.lock_mut(core::mem::take) };
    if pending.is_empty() {
        return;
    }
    console::write_bytes(&pending);
}

fn base_to_cli(command: BaseCommand<'_>) -> Option<CliCommand> {
    Some(match command {
        BaseCommand::Get { path } if !path.is_empty() => CliCommand::Get(String::from(path)),
        BaseCommand::Set { path, value } if !path.is_empty() && !value.is_empty() => {
            CliCommand::Set {
                path: String::from(path),
                value: String::from(value),
            }
        }
        BaseCommand::Paths => CliCommand::Paths,
        BaseCommand::Reset => CliCommand::Reset,
        BaseCommand::GetState => CliCommand::GetState,
        BaseCommand::GetStatus => CliCommand::GetStatus,
        BaseCommand::ResetTimestamp => CliCommand::ResetTimestamp,
        BaseCommand::SetWaypoints { json } => CliCommand::SetWaypoints(String::from(json)),
        BaseCommand::AppendWaypoints { json } => CliCommand::AppendWaypoints(String::from(json)),
        _ => return None,
    })
}

fn parse_path(path: &str) -> Option<(&str, Option<&str>)> {
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

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

fn log_set(path: &str, value: &str) {
    log::info!("{} set to {}", path, value);
}

fn log_get(path: &str, value: &str) {
    log::info!("{}: {}", path, value);
}

fn print_paths_catalog() {
    const CATALOG: &str = "\
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
    for line in CATALOG.split('\n') {
        console::write_line(line);
    }
}

/// CLI consumer: bytes from `console` IN channel, replies via OUT / `log::*`.
pub async fn handle_cli(app_context: AppContext) {
    let mut cli = CliBuilder::default()
        .writer(CliOutputWriter)
        .command_buffer([0u8; 2048])
        .history_buffer([0u8; 2048])
        .build()
        .unwrap();

    let mut processor = BaseCommand::processor(|_cli, command| {
        if let Some(cmd) = base_to_cli(command) {
            queue_command(cmd);
        } else {
            log::error!("Invalid command arguments");
        }
        Ok(())
    });

    loop {
        while let Ok(cmd) = CLI_CHANNEL.try_receive() {
            execute_command(cmd, app_context).await;
            flush_cli_output();
        }

        let byte = console::read_byte().await;
        let _ = cli.process_byte::<BaseCommand<'_>, _>(byte, &mut processor);
        flush_cli_output();
        while let Ok(cmd) = CLI_CHANNEL.try_receive() {
            execute_command(cmd, app_context).await;
            flush_cli_output();
        }
    }
}

async fn execute_command(command: CliCommand, app_context: AppContext) {
    match command {
        CliCommand::Get(path) => execute_get(app_context, path.as_str()).await,
        CliCommand::Set { path, value } => execute_set(app_context, path.as_str(), value.as_str()).await,
        CliCommand::Paths => print_paths_catalog(),
        CliCommand::Reset => {
            log::info!("Restarting device...");
            esp_hal::system::software_reset();
        }
        CliCommand::GetState => {
            let state = app_context.load_snapshot();
            let _ = crate::buffers::serialize_to_scratchpad(state.as_ref(), |json| {
                console::write_line(json);
            })
            .await;
        }
        CliCommand::GetStatus => {
            let state = app_context.load_snapshot();
            let config = state.config.clone();
            let st = serde_json::json!({ "state": state.as_ref(), "config": config });
            let _ = crate::buffers::serialize_to_scratchpad(&st, |json| {
                console::write_line(json);
            })
            .await;
        }
        CliCommand::ResetTimestamp => enqueue(app_context, MotionCommand::ResetTimestamp).await,
        CliCommand::SetWaypoints(json) => {
            if let Ok(input) = serde_json::from_str::<WaypointsInput>(&json) {
                let (waypoints, reset_timestamp) = input.into_parts();
                enqueue(
                    app_context,
                    MotionCommand::SetWaypoints {
                        waypoints,
                        reset_timestamp,
                    },
                )
                .await;
            }
        }
        CliCommand::AppendWaypoints(json) => {
            if let Ok(input) = serde_json::from_str::<WaypointsInput>(&json) {
                let (waypoints, _) = input.into_parts();
                enqueue(app_context, MotionCommand::AppendWaypoints(waypoints)).await;
            }
        }
    }
}

async fn execute_get(app_context: AppContext, path: &str) {
    let Some((section, key)) = parse_path(path) else {
        log::error!("Invalid path: {}", path);
        return;
    };

    match section {
        "pin" => match key {
            None => {
                let config = app_context.storage.pin();
                let _ = crate::buffers::serialize_to_scratchpad(&config, |json| {
                    console::write_line(json);
                })
                .await;
            }
            Some("modbus_tx") => {
                let v = app_context.storage.pin().modbus_tx;
                log_get("pin.modbus_tx", &format!("{}", v));
            }
            Some("modbus_rx") => {
                let v = app_context.storage.pin().modbus_rx;
                log_get("pin.modbus_rx", &format!("{}", v));
            }
            Some("modbus_de_re") => {
                let v = app_context.storage.pin().modbus_de_re;
                log_get("pin.modbus_de_re", &format!("{}", v));
            }
            Some("modbus_timeout_ms") => {
                let v = app_context.storage.pin().modbus_timeout_ms;
                log_get("pin.modbus_timeout_ms", &format!("{}", v));
            }
            Some("modbus_rx_timeout_us") => {
                let v = app_context.storage.pin().modbus_rx_timeout_us;
                log_get("pin.modbus_rx_timeout_us", &format!("{}", v));
            }
            Some("modbus_scan_delay_us") => {
                let v = app_context.storage.pin().modbus_scan_delay_us;
                log_get("pin.modbus_scan_delay_us", &format!("{}", v));
            }
            Some("modbus_inter_frame_delay_us") => {
                let v = app_context.storage.pin().modbus_inter_frame_delay_us;
                log_get("pin.modbus_inter_frame_delay_us", &format!("{}", v));
            }
            Some("ble_enabled") => {
                let v = app_context.storage.pin().ble_enabled;
                log_get("pin.ble_enabled", if v { "true" } else { "false" });
            }
            Some("modbus_debug") => {
                let v = app_context.storage.pin().modbus_debug;
                log_get("pin.modbus_debug", if v { "true" } else { "false" });
            }
            Some("operating_mode") => {
                let v = &app_context.storage.pin().operating_mode;
                log_get("pin.operating_mode", v.as_str());
            }
            Some(k) => log::error!("Unknown pin key: {}", k),
        },
        "net" => match key {
            None => {
                let config = app_context.storage.net();
                let _ = crate::buffers::serialize_to_scratchpad(&config, |json| {
                    console::write_line(json);
                })
                .await;
            }
            Some("wifi_enabled") => {
                let v = app_context.storage.net().wifi_enabled;
                log_get("net.wifi_enabled", if v { "true" } else { "false" });
            }
            Some("ssid") => {
                let v = &app_context.storage.net().ssid;
                log_get("net.ssid", v.as_str());
            }
            Some("password") => {
                log_get("net.password", "(hidden)");
            }
            Some("hostname") => {
                let v = &app_context.storage.net().hostname;
                log_get("net.hostname", v.as_str());
            }
            Some("dhcp_enabled") => {
                let v = app_context.storage.net().dhcp_enabled;
                log_get("net.dhcp_enabled", if v { "true" } else { "false" });
            }
            Some("static_ip") => {
                let v = &app_context.storage.net().static_ip;
                log_get("net.static_ip", v.as_str());
            }
            Some("static_mask") => {
                let v = &app_context.storage.net().static_mask;
                log_get("net.static_mask", v.as_str());
            }
            Some("static_gateway") => {
                let v = &app_context.storage.net().static_gateway;
                log_get("net.static_gateway", v.as_str());
            }
            Some("static_dns") => {
                let v = &app_context.storage.net().static_dns;
                log_get("net.static_dns", v.as_str());
            }
            Some(k) => log::error!("Unknown net key: {}", k),
        },
        "motor" => match key {
            None => {
                let config = app_context.load_snapshot().config.clone();
                let _ = crate::buffers::serialize_to_scratchpad(&config, |json| {
                    console::write_line(json);
                })
                .await;
            }
            Some("version") => {
                let v = app_context.load_snapshot().config.version;
                log_get("motor.version", &format!("{}", v));
            }
            Some("bpm") => {
                let v = app_context.load_snapshot().config.bpm;
                log_get("motor.bpm", &format!("{}", v));
            }
            Some("depth") => {
                let v = app_context.load_snapshot().config.depth;
                log_get("motor.depth", &format!("{}", v));
            }
            Some("depth_top") => {
                let v = app_context.load_snapshot().config.depth_top;
                log_get("motor.depth_top", if v { "true" } else { "false" });
            }
            Some("reversed") => {
                let v = app_context.load_snapshot().config.reversed;
                log_get("motor.reversed", if v { "true" } else { "false" });
            }
            Some("wave_func") => {
                let v = &app_context.load_snapshot().config.wave_func;
                log_get("motor.wave_func", v.as_str());
            }
            Some("sharpness") => {
                let v = app_context.load_snapshot().config.sharpness;
                log_get("motor.sharpness", &format!("{}", v));
            }
            Some("paused") => {
                let v = app_context.load_snapshot().config.paused;
                log_get("motor.paused", if v { "true" } else { "false" });
            }
            Some("paused_position") => {
                let v = app_context.load_snapshot().config.paused_position;
                log_get("motor.paused_position", &format!("{}", v));
            }
            Some("streaming") => {
                let v = app_context.load_snapshot().config.streaming;
                log_get("motor.streaming", if v { "true" } else { "false" });
            }
            Some("spline_points") => {
                let pts = &app_context.load_snapshot().config.spline_points;
                let mut s = String::new();
                for (i, p) in pts.iter().enumerate() {
                    if i > 0 {
                        s.push(' ');
                    }
                    let _ = write!(s, "{}", p);
                }
                log_get("motor.spline_points", s.as_str());
            }
            Some(k) => log::error!("Unknown motor key: {}", k),
        },
        "inject" => {
            if key.is_some() {
                log::error!("Use: get inject");
                return;
            }
            let (mode, nbytes) = crate::modbus_rtu::get_inject_junk();
            let msg = format!("{} {}", mode.as_str(), nbytes);
            log_get("inject", msg.as_str());
            console::write_line(&format!("inject: {}", msg));
        }
        _ => log::error!("Unknown section: {} (try: pin, net, motor, inject)", section),
    }
}

async fn execute_set(app_context: AppContext, path: &str, value: &str) {
    let Some((section, key)) = parse_path(path) else {
        log::error!("Invalid path: {}", path);
        return;
    };

    match section {
        "pin" => set_pin(app_context, key, value).await,
        "net" => set_net(app_context, key, value).await,
        "motor" => set_motor(app_context, key, value).await,
        "inject" => set_inject(value),
        _ => log::error!("Unknown section: {} (try: pin, net, motor, inject)", section),
    }
}

async fn set_pin(app_context: AppContext, key: Option<&str>, value: &str) {
    let Some(key) = key else {
        log::error!("Use scalar path, e.g. set pin.modbus_tx 2");
        return;
    };
    let full_path = format!("pin.{}", key);
    let mut config = app_context.storage.pin();

    match key {
        "modbus_tx" | "modbus_rx" | "modbus_de_re" => {
            let Ok(pin) = value.parse::<u32>() else {
                log::error!("Invalid GPIO: {}", value);
                return;
            };
            if pin > 48 {
                log::error!("GPIO out of range 0..48");
                return;
            }
            match key {
                "modbus_tx" => config.modbus_tx = pin,
                "modbus_rx" => config.modbus_rx = pin,
                _ => config.modbus_de_re = pin,
            }
            app_context.storage.set_pin(config);
            log_set(full_path.as_str(), value);
        }
        "modbus_timeout_ms" => {
            let Ok(v) = value.parse::<u32>() else {
                return;
            };
            if v > 1000 {
                return;
            }
            config.modbus_timeout_ms = v;
            app_context.storage.set_pin(config);
            log_set(full_path.as_str(), value);
        }
        "modbus_rx_timeout_us" | "modbus_scan_delay_us" | "modbus_inter_frame_delay_us" => {
            let Ok(v) = value.parse::<u32>() else {
                return;
            };
            if v > 200_000 {
                return;
            }
            match key {
                "modbus_rx_timeout_us" => config.modbus_rx_timeout_us = v,
                "modbus_scan_delay_us" => config.modbus_scan_delay_us = v,
                _ => config.modbus_inter_frame_delay_us = v,
            }
            app_context.storage.set_pin(config);
            log_set(full_path.as_str(), value);
        }
        "ble_enabled" | "modbus_debug" => {
            let Some(v) = parse_bool(value) else {
                return;
            };
            match key {
                "ble_enabled" => config.ble_enabled = v,
                _ => config.modbus_debug = v,
            }
            app_context.storage.set_pin(config);
            log_set(full_path.as_str(), value);
        }
        "operating_mode" => {
            if !matches!(value, "servo" | "rtu_relay") {
                log::error!("operating_mode must be servo or rtu_relay");
                return;
            }
            config.operating_mode = String::from(value);
            app_context.storage.set_pin(config);
            log_set(full_path.as_str(), value);
        }
        _ => log::error!("Unknown pin key: {}", key),
    }
}

async fn set_net(app_context: AppContext, key: Option<&str>, value: &str) {
    if key.is_none() {
        log::error!("Use scalar path, e.g. set net.ssid \"MyNetwork\"");
        return;
    }
    let key = key.unwrap();
    let full_path = format!("net.{}", key);

    match key {
        "wifi_enabled" | "dhcp_enabled" => {
            let Some(v) = parse_bool(value) else {
                return;
            };
            let mut config = app_context.storage.net();
            match key {
                "wifi_enabled" => config.wifi_enabled = v,
                _ => config.dhcp_enabled = v,
            }
            app_context.storage.set_net(config);
            log_set(full_path.as_str(), value);
        }
        "ssid" => {
            app_context.storage.set_ssid(value);
            log_set(full_path.as_str(), value);
        }
        "password" => {
            app_context.storage.set_password(value);
            log_set(full_path.as_str(), "(hidden)");
        }
        "hostname" => {
            let mut config = app_context.storage.net();
            config.hostname = String::from(value);
            app_context.storage.set_net(config);
            log_set(full_path.as_str(), value);
        }
        "static_ip" | "static_mask" | "static_gateway" | "static_dns" => {
            let mut config = app_context.storage.net();
            match key {
                "static_ip" => config.static_ip = String::from(value),
                "static_mask" => config.static_mask = String::from(value),
                "static_gateway" => config.static_gateway = String::from(value),
                _ => config.static_dns = String::from(value),
            }
            app_context.storage.set_net(config);
            log_set(full_path.as_str(), value);
        }
        _ => log::error!("Unknown net key: {}", key),
    }
}

async fn set_motor(app_context: AppContext, key: Option<&str>, value: &str) {
    if key.is_none() {
        if !value.starts_with('{') {
            log::error!("Bulk motor set requires JSON: set motor {{...}}");
            return;
        }
        if let Ok(config) = serde_json::from_str::<MotorControllerConfig>(value) {
            if app_context.try_enqueue_config(config).await.is_ok() {
                log_set("motor", value);
            }
        } else {
            log::error!("Invalid motor JSON");
        }
        return;
    }

    let key = key.unwrap();
    if key == "version" {
        log::error!("motor.version is read-only");
        return;
    }
    let full_path = format!("motor.{}", key);

    match key {
        "bpm" => {
            let Ok(v) = value.parse::<f32>() else {
                return;
            };
            if v <= 0.0 {
                return;
            }
            update_config(app_context, |c| c.bpm = v).await;
            log_set(full_path.as_str(), value);
        }
        "depth" => {
            let Ok(v) = value.parse::<f32>() else {
                return;
            };
            if !(0.01..=1.0).contains(&v) {
                return;
            }
            update_config(app_context, |c| c.depth = v).await;
            log_set(full_path.as_str(), value);
        }
        "depth_top" | "reversed" | "paused" | "streaming" => {
            let Some(v) = parse_bool(value) else {
                return;
            };
            update_config(app_context, |c| match key {
                "depth_top" => c.depth_top = v,
                "reversed" => c.reversed = v,
                "paused" => c.paused = v,
                _ => c.streaming = v,
            })
            .await;
            log_set(full_path.as_str(), value);
        }
        "wave_func" => {
            if !matches!(value, "sine" | "thrust" | "spline") {
                log::error!("wave_func must be sine, thrust, or spline");
                return;
            }
            update_config(app_context, |c| c.wave_func = String::from(value)).await;
            log_set(full_path.as_str(), value);
        }
        "sharpness" => {
            let Ok(v) = value.parse::<f32>() else {
                return;
            };
            if !(0.01..=0.99).contains(&v) {
                return;
            }
            update_config(app_context, |c| c.sharpness = v).await;
            log_set(full_path.as_str(), value);
        }
        "paused_position" => {
            let Ok(v) = value.parse::<f32>() else {
                return;
            };
            if !(0.0..=1.0).contains(&v) {
                return;
            }
            update_config(app_context, |c| c.paused_position = v).await;
            log_set(full_path.as_str(), value);
        }
        "spline_points" => {
            let parsed: Result<Vec<f32>, _> = value
                .split_whitespace()
                .map(|s| s.parse::<f32>())
                .collect();
            let Ok(points) = parsed else {
                return;
            };
            update_config(app_context, |c| c.spline_points = points).await;
            log_set(full_path.as_str(), value);
        }
        _ => log::error!("Unknown motor key: {}", key),
    }
}

fn set_inject(value: &str) {
    use crate::modbus_rtu::InjectJunkMode;

    let mut parts = value.split_whitespace();
    let mode_str = parts.next().unwrap_or("");
    let nbytes_str = parts.next().unwrap_or("0");
    let Ok(nbytes) = nbytes_str.parse::<u32>() else {
        log::error!("Invalid inject nbytes");
        return;
    };
    if nbytes > 64 {
        log::error!("inject nbytes must be 0..64");
        return;
    }
    let mode = match mode_str {
        "off" => InjectJunkMode::Off,
        "leading" => InjectJunkMode::Leading,
        "trailing" => InjectJunkMode::Trailing,
        "both" => InjectJunkMode::Both,
        _ => {
            log::error!("inject mode must be off, leading, trailing, or both");
            return;
        }
    };
    crate::modbus_rtu::set_inject_junk(mode, nbytes as u8);
    let msg = format!("{} {}", mode.as_str(), nbytes);
    log_set("inject", msg.as_str());
    console::write_line(&format!("inject set to {}", msg));
}

async fn update_config(app_context: AppContext, f: impl FnOnce(&mut MotorControllerConfig)) {
    let mut config = app_context.load_snapshot().config.clone();
    f(&mut config);
    let _ = app_context.try_enqueue_config(config).await;
}

async fn enqueue(app_context: AppContext, cmd: MotionCommand) {
    let _ = app_context.enqueue_motion(cmd).await;
    log::info!("ok");
}
