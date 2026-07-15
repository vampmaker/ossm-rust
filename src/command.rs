use alloc::string::String;
use alloc::vec::Vec;

use embedded_cli::cli::CliBuilder;
use embedded_cli::Command;
use embedded_io::ErrorType;
use embedded_io::Write as EmbeddedWrite;
use embassy_sync::blocking_mutex::{raw::CriticalSectionRawMutex, Mutex as BlockingMutex};
use embassy_sync::channel::Channel;

use crate::console;
use crate::context::AppContext;
use crate::http_api::WaypointsInput;
use crate::motion::{MotionCommand, MotorControllerConfig};

static CLI_CHANNEL: Channel<CriticalSectionRawMutex, CliCommand, 4> = Channel::new();
static CLI_OUTPUT: BlockingMutex<CriticalSectionRawMutex, Vec<u8>> = BlockingMutex::new(Vec::new());

#[derive(Clone)]
enum CliCommand {
    SetWifiSsid(String),
    SetWifiPassword(String),
    SetPinModbusTx(u32),
    SetPinModbusRx(u32),
    SetPinModbusDeRe(u32),
    GetPinConfiguration,
    SetMotorConfig(String),
    GetMotorConfig,
    Pause,
    Start,
    Reset,
    SetBpm(f32),
    SetWave(String),
    SetPausedPosition(f32),
    SetDepth(f32),
    SetDepthTop(bool),
    SetSharpness(f32),
    SetSplinePoints(Vec<f32>),
    SetModbusTimeoutMs(u32),
    GetModbusTimeoutMs,
    SetModbusRxTimeoutUs(u32),
    GetModbusRxTimeoutUs,
    SetModbusScanDelayUs(u32),
    GetModbusScanDelayUs,
    SetModbusInterFrameDelayUs(u32),
    GetModbusInterFrameDelayUs,
    SetBleEnabled(bool),
    GetBleEnabled,
    SetWifiEnabled(bool),
    GetWifiEnabled,
    SetHostname(String),
    SetDhcpEnabled(bool),
    SetStaticIp(String),
    SetStaticMask(String),
    SetStaticGateway(String),
    SetStaticDns(String),
    GetNetworkConfig,
    GetState,
    GetStatus,
    ResetTimestamp,
    SetWaypoints(String),
    AppendWaypoints(String),
}

#[derive(Command, Clone, Debug)]
#[command(help_title = "ossm-rust")]
enum BaseCommand<'a> {
    SetWifiSsid { ssid: &'a str },
    SetWifiPassword { password: &'a str },
    SetPinModbusTx { pin: u32 },
    SetPinModbusRx { pin: u32 },
    SetPinModbusDeRe { pin: u32 },
    GetPinConfiguration,
    SetMotorConfig { json: &'a str },
    GetMotorConfig,
    Pause,
    Start,
    Reset,
    SetBpm { bpm: f32 },
    SetWave { wave: &'a str },
    SetPausedPosition { position: f32 },
    SetDepth { depth: f32 },
    SetDepthTop { v: bool },
    SetSharpness { sharpness: f32 },
    SetSplinePoints { points: &'a str },
    SetModbusTimeoutMs { value: u32 },
    GetModbusTimeoutMs,
    SetModbusRxTimeoutUs { value: u32 },
    GetModbusRxTimeoutUs,
    SetModbusScanDelayUs { value: u32 },
    GetModbusScanDelayUs,
    SetModbusInterFrameDelayUs { value: u32 },
    GetModbusInterFrameDelayUs,
    SetBleEnabled { value: bool },
    GetBleEnabled,
    SetWifiEnabled { value: bool },
    GetWifiEnabled,
    SetHostname { hostname: &'a str },
    SetDhcpEnabled { value: bool },
    SetStaticIp { ip: &'a str },
    SetStaticMask { mask: &'a str },
    SetStaticGateway { gateway: &'a str },
    SetStaticDns { dns: &'a str },
    GetNetworkConfig,
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
    // Pass through as-is (echo chars + CLI newlines); do not wrap each flush in `\r\n`.
    console::write_bytes(&pending);
}

fn base_to_cli(command: BaseCommand<'_>) -> Option<CliCommand> {
    Some(match command {
        BaseCommand::SetWifiSsid { ssid } => CliCommand::SetWifiSsid(String::from(ssid)),
        BaseCommand::SetWifiPassword { password } => {
            CliCommand::SetWifiPassword(String::from(password))
        }
        BaseCommand::SetPinModbusTx { pin } => CliCommand::SetPinModbusTx(pin),
        BaseCommand::SetPinModbusRx { pin } => CliCommand::SetPinModbusRx(pin),
        BaseCommand::SetPinModbusDeRe { pin } => CliCommand::SetPinModbusDeRe(pin),
        BaseCommand::GetPinConfiguration => CliCommand::GetPinConfiguration,
        BaseCommand::SetMotorConfig { json } => CliCommand::SetMotorConfig(String::from(json)),
        BaseCommand::GetMotorConfig => CliCommand::GetMotorConfig,
        BaseCommand::Pause => CliCommand::Pause,
        BaseCommand::Start => CliCommand::Start,
        BaseCommand::Reset => CliCommand::Reset,
        BaseCommand::SetBpm { bpm } if bpm > 0.0 => CliCommand::SetBpm(bpm),
        BaseCommand::SetWave { wave } if matches!(wave, "sine" | "thrust" | "spline") => {
            CliCommand::SetWave(String::from(wave))
        }
        BaseCommand::SetPausedPosition { position }
            if position > 0.0 && position < 1.0 =>
        {
            CliCommand::SetPausedPosition(position)
        }
        BaseCommand::SetDepth { depth } if depth > 0.01 && depth < 1.0 => CliCommand::SetDepth(depth),
        BaseCommand::SetDepthTop { v } => CliCommand::SetDepthTop(v),
        BaseCommand::SetSharpness { sharpness }
            if (0.01..1.0).contains(&sharpness) =>
        {
            CliCommand::SetSharpness(sharpness)
        }
        BaseCommand::SetSplinePoints { points } => {
            let parsed: Result<Vec<f32>, _> =
                points.split_whitespace().map(|s| s.parse::<f32>()).collect();
            CliCommand::SetSplinePoints(parsed.ok()?)
        }
        BaseCommand::SetModbusTimeoutMs { value } if value <= 1000 => {
            CliCommand::SetModbusTimeoutMs(value)
        }
        BaseCommand::GetModbusTimeoutMs => CliCommand::GetModbusTimeoutMs,
        BaseCommand::SetModbusRxTimeoutUs { value } if value <= 200_000 => {
            CliCommand::SetModbusRxTimeoutUs(value)
        }
        BaseCommand::GetModbusRxTimeoutUs => CliCommand::GetModbusRxTimeoutUs,
        BaseCommand::SetModbusScanDelayUs { value } if value <= 200_000 => {
            CliCommand::SetModbusScanDelayUs(value)
        }
        BaseCommand::GetModbusScanDelayUs => CliCommand::GetModbusScanDelayUs,
        BaseCommand::SetModbusInterFrameDelayUs { value } if value <= 200_000 => {
            CliCommand::SetModbusInterFrameDelayUs(value)
        }
        BaseCommand::GetModbusInterFrameDelayUs => CliCommand::GetModbusInterFrameDelayUs,
        BaseCommand::SetBleEnabled { value } => CliCommand::SetBleEnabled(value),
        BaseCommand::GetBleEnabled => CliCommand::GetBleEnabled,
        BaseCommand::SetWifiEnabled { value } => CliCommand::SetWifiEnabled(value),
        BaseCommand::GetWifiEnabled => CliCommand::GetWifiEnabled,
        BaseCommand::SetHostname { hostname } => CliCommand::SetHostname(String::from(hostname)),
        BaseCommand::SetDhcpEnabled { value } => CliCommand::SetDhcpEnabled(value),
        BaseCommand::SetStaticIp { ip } => CliCommand::SetStaticIp(String::from(ip)),
        BaseCommand::SetStaticMask { mask } => CliCommand::SetStaticMask(String::from(mask)),
        BaseCommand::SetStaticGateway { gateway } => {
            CliCommand::SetStaticGateway(String::from(gateway))
        }
        BaseCommand::SetStaticDns { dns } => CliCommand::SetStaticDns(String::from(dns)),
        BaseCommand::GetNetworkConfig => CliCommand::GetNetworkConfig,
        BaseCommand::GetState => CliCommand::GetState,
        BaseCommand::GetStatus => CliCommand::GetStatus,
        BaseCommand::ResetTimestamp => CliCommand::ResetTimestamp,
        BaseCommand::SetWaypoints { json } => CliCommand::SetWaypoints(String::from(json)),
        BaseCommand::AppendWaypoints { json } => CliCommand::AppendWaypoints(String::from(json)),
        _ => return None,
    })
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
        CliCommand::SetWifiSsid(ssid) => {
            app_context.storage.set_ssid(ssid.as_str());
            log::info!("SSID saved: {}, restart to apply", ssid);
        }
        CliCommand::SetWifiPassword(password) => {
            app_context.storage.set_password(password.as_str());
            log::info!("Password saved, restart to apply");
        }
        CliCommand::SetPinModbusTx(pin) => {
            let mut config = app_context.storage.pin();
            config.modbus_tx = pin;
            app_context.storage.set_pin(config);
            log::info!("Modbus TX pin set to {}, restart to apply", pin);
        }
        CliCommand::SetPinModbusRx(pin) => {
            let mut config = app_context.storage.pin();
            config.modbus_rx = pin;
            app_context.storage.set_pin(config);
            log::info!("Modbus RX pin set to {}, restart to apply", pin);
        }
        CliCommand::SetPinModbusDeRe(pin) => {
            let mut config = app_context.storage.pin();
            config.modbus_de_re = pin;
            app_context.storage.set_pin(config);
            log::info!("Modbus DE/RE pin set to {}, restart to apply", pin);
        }
        CliCommand::GetPinConfiguration => {
            let config = app_context.storage.pin();
            let _ = crate::buffers::serialize_to_scratchpad(&config, |json| {
                console::write_line(json);
            })
            .await;
        }
        CliCommand::SetMotorConfig(json) => {
            if let Ok(config) = serde_json::from_str::<MotorControllerConfig>(&json) {
                let _ = app_context
                    .enqueue_motion(MotionCommand::SetConfig(config))
                    .await;
                log::info!("Motor config updated");
            }
        }
        CliCommand::GetMotorConfig => {
            let config = app_context.load_snapshot().config.clone();
            let _ = crate::buffers::serialize_to_scratchpad(&config, |json| {
                console::write_line(json);
            })
            .await;
        }
        CliCommand::Pause => update_config(app_context, |c| c.paused = true).await,
        CliCommand::Start => update_config(app_context, |c| c.paused = false).await,
        CliCommand::Reset => {
            log::info!("Restarting device...");
            esp_hal::system::software_reset();
        }
        CliCommand::SetBpm(bpm) => update_config(app_context, |c| c.bpm = bpm).await,
        CliCommand::SetWave(wave) => update_config(app_context, |c| c.wave_func = wave).await,
        CliCommand::SetPausedPosition(position) => {
            update_config(app_context, |c| c.paused_position = position).await;
        }
        CliCommand::SetDepth(depth) => update_config(app_context, |c| c.depth = depth).await,
        CliCommand::SetDepthTop(v) => update_config(app_context, |c| c.depth_top = v).await,
        CliCommand::SetSharpness(sharpness) => {
            update_config(app_context, |c| c.sharpness = sharpness).await;
        }
        CliCommand::SetSplinePoints(points_vec) => {
            update_config(app_context, |c| c.spline_points = points_vec).await;
        }
        CliCommand::SetModbusTimeoutMs(value) => {
            let mut config = app_context.storage.pin();
            config.modbus_timeout_ms = value;
            app_context.storage.set_pin(config);
        }
        CliCommand::GetModbusTimeoutMs => {
            let config = app_context.storage.pin();
            log::info!("modbus_timeout_ms: {}", config.modbus_timeout_ms);
        }
        CliCommand::SetModbusRxTimeoutUs(value) => {
            let mut config = app_context.storage.pin();
            config.modbus_rx_timeout_us = value;
            app_context.storage.set_pin(config);
        }
        CliCommand::GetModbusRxTimeoutUs => {
            let config = app_context.storage.pin();
            log::info!("modbus_rx_timeout_us: {}", config.modbus_rx_timeout_us);
        }
        CliCommand::SetModbusScanDelayUs(value) => {
            let mut config = app_context.storage.pin();
            config.modbus_scan_delay_us = value;
            app_context.storage.set_pin(config);
        }
        CliCommand::GetModbusScanDelayUs => {
            let config = app_context.storage.pin();
            log::info!("modbus_scan_delay_us: {}", config.modbus_scan_delay_us);
        }
        CliCommand::SetModbusInterFrameDelayUs(value) => {
            let mut config = app_context.storage.pin();
            config.modbus_inter_frame_delay_us = value;
            app_context.storage.set_pin(config);
        }
        CliCommand::GetModbusInterFrameDelayUs => {
            let config = app_context.storage.pin();
            log::info!(
                "modbus_inter_frame_delay_us: {}",
                config.modbus_inter_frame_delay_us
            );
        }
        CliCommand::SetBleEnabled(value) => {
            let mut config = app_context.storage.pin();
            config.ble_enabled = value;
            app_context.storage.set_pin(config);
        }
        CliCommand::GetBleEnabled => {
            let config = app_context.storage.pin();
            log::info!("ble_enabled: {}", config.ble_enabled);
        }
        CliCommand::SetWifiEnabled(value) => {
            let mut config = app_context.storage.net();
            config.wifi_enabled = value;
            app_context.storage.set_net(config);
        }
        CliCommand::GetWifiEnabled => {
            let config = app_context.storage.net();
            log::info!("wifi_enabled: {}", config.wifi_enabled);
        }
        CliCommand::SetHostname(hostname) => {
            let mut config = app_context.storage.net();
            config.hostname = hostname;
            app_context.storage.set_net(config);
        }
        CliCommand::SetDhcpEnabled(value) => {
            let mut config = app_context.storage.net();
            config.dhcp_enabled = value;
            app_context.storage.set_net(config);
        }
        CliCommand::SetStaticIp(ip) => set_net_field(app_context, |c| c.static_ip = ip).await,
        CliCommand::SetStaticMask(mask) => set_net_field(app_context, |c| c.static_mask = mask).await,
        CliCommand::SetStaticGateway(gateway) => {
            set_net_field(app_context, |c| c.static_gateway = gateway).await;
        }
        CliCommand::SetStaticDns(dns) => set_net_field(app_context, |c| c.static_dns = dns).await,
        CliCommand::GetNetworkConfig => {
            let config = app_context.storage.net();
            let _ = crate::buffers::serialize_to_scratchpad(&config, |json| {
                console::write_line(json);
            })
            .await;
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

async fn update_config(app_context: AppContext, f: impl FnOnce(&mut MotorControllerConfig)) {
    let mut config = app_context.load_snapshot().config.clone();
    f(&mut config);
    let _ = app_context
        .enqueue_motion(MotionCommand::SetConfig(config))
        .await;
}

async fn set_net_field(app_context: AppContext, f: impl FnOnce(&mut crate::storage::NetworkConfiguration)) {
    let mut config = app_context.storage.net();
    f(&mut config);
    app_context.storage.set_net(config);
}

async fn enqueue(app_context: AppContext, cmd: MotionCommand) {
    let _ = app_context.enqueue_motion(cmd).await;
    log::info!("ok");
}
