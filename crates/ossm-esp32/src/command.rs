use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use embassy_sync::blocking_mutex::{raw::CriticalSectionRawMutex, Mutex as BlockingMutex};
use embassy_sync::channel::Channel;
use embedded_cli::cli::CliBuilder;
use embedded_cli::Command;
use embedded_io::ErrorType;
use embedded_io::Write as EmbeddedWrite;

use ossm_core::paths::{self, PathError};

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
    Get {
        path: &'a str,
    },
    /// Set config: set <path> <value>  (quote strings with spaces)
    /// Bulk motor JSON: set motor {"bpm":36,...}
    /// Inject: set inject <off|leading|trailing|both> <nbytes 0..64>
    Set {
        path: &'a str,
        value: &'a str,
    },
    /// Print config path catalog (types, enums, ranges)
    Paths,
    Reset,
    GetState,
    GetStatus,
    ResetTimestamp,
    SetWaypoints {
        json: &'a str,
    },
    AppendWaypoints {
        json: &'a str,
    },
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

fn log_set(path: &str, value: &str) {
    log::info!("{} set to {}", path, value);
    console::write_line(&format!("{} set to {}", path, value));
}

fn log_get(path: &str, value: &str) {
    log::info!("{}: {}", path, value);
    console::write_line(&format!("{}: {}", path, value));
}

fn print_paths_catalog() {
    for line in paths::PATHS_CATALOG.split('\n') {
        console::write_line(line);
    }
}

fn log_path_err(path: &str, err: PathError) {
    match err {
        PathError::UnknownKey => log::error!("Unknown key: {}", path),
        PathError::UnknownSection => log::error!("{}", err),
        PathError::ReadOnly => log::error!("motor.version is read-only"),
        PathError::ScalarRequired => log::error!("Use scalar path, e.g. set pin.modbus_tx 2"),
        PathError::BulkJsonRequired => {
            log::error!("Bulk motor set requires JSON: set motor {{...}}")
        }
        PathError::InvalidValue => {}
        _ => log::error!("{}: {}", path, err),
    }
}

/// CLI consumer: bytes from `console` IN channel, replies via `write_line` / `log::*`.
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
        CliCommand::Set { path, value } => {
            execute_set(app_context, path.as_str(), value.as_str()).await
        }
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
    let Some((section, key)) = paths::parse_path(path) else {
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
            Some(k) => match paths::pin_field(&app_context.storage.pin(), k) {
                Ok(v) => log_get(&format!("pin.{}", k), v.as_str()),
                Err(e) => log_path_err(&format!("pin.{}", k), e),
            },
        },
        "net" => match key {
            None => {
                let config = app_context.storage.net();
                let _ = crate::buffers::serialize_to_scratchpad(&config, |json| {
                    console::write_line(json);
                })
                .await;
            }
            Some(k) => match paths::net_field(&app_context.storage.net(), k) {
                Ok(v) => log_get(&format!("net.{}", k), v.as_str()),
                Err(e) => log_path_err(&format!("net.{}", k), e),
            },
        },
        "motor" => match key {
            None => {
                let config = app_context.load_snapshot().config.clone();
                let _ = crate::buffers::serialize_to_scratchpad(&config, |json| {
                    console::write_line(json);
                })
                .await;
            }
            Some(k) => {
                let config = app_context.load_snapshot().config.clone();
                match paths::motor_field(&config, k) {
                    Ok(v) => log_get(&format!("motor.{}", k), v.as_str()),
                    Err(e) => log_path_err(&format!("motor.{}", k), e),
                }
            }
        },
        "inject" => {
            if key.is_some() {
                log::error!("Use: get inject");
                return;
            }
            let (mode, nbytes) = crate::modbus_rtu::get_inject_junk();
            let msg = format!("{} {}", mode.as_str(), nbytes);
            log_get("inject", msg.as_str());
        }
        _ => log::error!(
            "Unknown section: {} (try: pin, net, motor, inject)",
            section
        ),
    }
}

async fn execute_set(app_context: AppContext, path: &str, value: &str) {
    let Some((section, key)) = paths::parse_path(path) else {
        log::error!("Invalid path: {}", path);
        return;
    };

    match section {
        "pin" => set_pin(app_context, key, value),
        "net" => set_net(app_context, key, value),
        "motor" => set_motor(app_context, key, value).await,
        "inject" => set_inject(value),
        _ => log::error!(
            "Unknown section: {} (try: pin, net, motor, inject)",
            section
        ),
    }
}

fn set_pin(app_context: AppContext, key: Option<&str>, value: &str) {
    let Some(key) = key else {
        log_path_err("pin", PathError::ScalarRequired);
        return;
    };
    let full_path = format!("pin.{}", key);
    let mut config = app_context.storage.pin();
    match paths::set_pin(&mut config, key, value) {
        Ok(()) => {
            app_context.storage.set_pin(config);
            log_set(full_path.as_str(), value);
        }
        Err(e) => log_path_err(full_path.as_str(), e),
    }
}

fn set_net(app_context: AppContext, key: Option<&str>, value: &str) {
    let Some(key) = key else {
        log::error!("Use scalar path, e.g. set net.ssid \"MyNetwork\"");
        return;
    };
    let full_path = format!("net.{}", key);
    let mut config = app_context.storage.net();
    match paths::set_net(&mut config, key, value) {
        Ok(()) => {
            match key {
                "ssid" => app_context.storage.set_ssid(value),
                "password" => {
                    app_context.storage.set_password(value);
                    log_set(full_path.as_str(), "(hidden)");
                    return;
                }
                _ => app_context.storage.set_net(config),
            }
            log_set(full_path.as_str(), value);
        }
        Err(e) => log_path_err(full_path.as_str(), e),
    }
}

async fn set_motor(app_context: AppContext, key: Option<&str>, value: &str) {
    if key.is_none() {
        if !value.starts_with('{') {
            log_path_err("motor", PathError::BulkJsonRequired);
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
    let full_path = format!("motor.{}", key);
    let mut config = app_context.load_snapshot().config.clone();
    match paths::apply_motor_field(&mut config, key, value) {
        Ok(()) => {
            let _ = app_context.try_enqueue_config(config).await;
            log_set(full_path.as_str(), value);
        }
        Err(e) => log_path_err(full_path.as_str(), e),
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
}

async fn enqueue(app_context: AppContext, cmd: MotionCommand) {
    let _ = app_context.enqueue_motion(cmd).await;
    log::info!("ok");
}
