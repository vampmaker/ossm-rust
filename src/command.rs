use alloc::string::String;
use alloc::vec::Vec;

use embedded_cli::cli::CliBuilder;
use embedded_cli::Command;
use embedded_io::ErrorType;
use embedded_io::Write as EmbeddedWrite;
use embedded_io_async::Read as AsyncRead;
use embassy_sync::blocking_mutex::{raw::CriticalSectionRawMutex, Mutex as BlockingMutex};
use embassy_sync::channel::Channel;
use embassy_time::Timer;
use esp_hal::usb_serial_jtag::UsbSerialJtag;
use esp_println::println;

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

async fn flush_cli_output(usb: &mut UsbSerialJtag<'_, esp_hal::Async>) {
    let pending = unsafe {
        CLI_OUTPUT.lock_mut(core::mem::take)
    };
    if !pending.is_empty() {
        let _ = embedded_io::Write::write(usb, &pending);
    }
}

pub async fn handle_cli(usb_peripheral: esp_hal::peripherals::USB_DEVICE<'static>, app_context: AppContext) {
    let mut usb = UsbSerialJtag::new(usb_peripheral).into_async();

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

    let mut buf = [0u8; 1];
    loop {
        while let Ok(cmd) = CLI_CHANNEL.try_receive() {
            execute_command(cmd, app_context).await;
            flush_cli_output(&mut usb).await;
        }

        match AsyncRead::read(&mut usb, &mut buf).await {
            Ok(1) => {
                let byte = if buf[0] == 0x7F { 0x08 } else { buf[0] };
                let _ = cli.process_byte::<BaseCommand<'_>, _>(byte, &mut processor);
                flush_cli_output(&mut usb).await;
                while let Ok(cmd) = CLI_CHANNEL.try_receive() {
                    execute_command(cmd, app_context).await;
                    flush_cli_output(&mut usb).await;
                }
            }
            Ok(_) | Err(_) => {
                Timer::after_millis(10).await;
            }
        }
    }
}

async fn execute_command(command: CliCommand, app_context: AppContext) {
    match command {
        CliCommand::SetWifiSsid(ssid) => {
            let _ = app_context.storage.lock().await.set_ssid(&ssid);
            log::info!("SSID saved: {}, restart to apply", ssid);
        }
        CliCommand::SetWifiPassword(password) => {
            let _ = app_context.storage.lock().await.set_password(&password);
            log::info!("Password saved, restart to apply");
        }
        CliCommand::SetPinModbusTx(pin) => {
            let mut sm = app_context.storage.lock().await;
            let mut config = sm.get_pin_configuration().unwrap_or_default();
            config.modbus_tx = pin;
            let _ = sm.set_pin_configuration(&config);
            log::info!("Modbus TX pin set to {}, restart to apply", pin);
        }
        CliCommand::SetPinModbusRx(pin) => {
            let mut sm = app_context.storage.lock().await;
            let mut config = sm.get_pin_configuration().unwrap_or_default();
            config.modbus_rx = pin;
            let _ = sm.set_pin_configuration(&config);
            log::info!("Modbus RX pin set to {}, restart to apply", pin);
        }
        CliCommand::SetPinModbusDeRe(pin) => {
            let mut sm = app_context.storage.lock().await;
            let mut config = sm.get_pin_configuration().unwrap_or_default();
            config.modbus_de_re = pin;
            let _ = sm.set_pin_configuration(&config);
            log::info!("Modbus DE/RE pin set to {}, restart to apply", pin);
        }
        CliCommand::GetPinConfiguration => {
            if let Ok(config) = app_context.storage.lock().await.get_pin_configuration() {
                let _ = crate::buffers::serialize_to_scratchpad(&config, |json| println!("{}", json));
            }
        }
        CliCommand::SetMotorConfig(json) => {
            if let Ok(config) = serde_json::from_str::<MotorControllerConfig>(&json) {
                let mut mc_opt = app_context.motor_controller.lock().await;
                if let Some(mc) = mc_opt.as_mut() {
                    let _ = mc.set_config(config);
                }
                log::info!("Motor config updated");
            }
        }
        CliCommand::GetMotorConfig => {
            let config = {
                let mc_opt = app_context.motor_controller.lock().await;
                mc_opt.as_ref().map(|mc| mc.get_config())
            };
            if let Some(config) = config {
                let _ = crate::buffers::serialize_to_scratchpad(&config, |json| println!("{}", json));
            }
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
            let mut sm = app_context.storage.lock().await;
            let mut config = sm.get_pin_configuration().unwrap_or_default();
            config.modbus_timeout_ms = value;
            let _ = sm.set_pin_configuration(&config);
        }
        CliCommand::GetModbusTimeoutMs => {
            let config = app_context
                .storage
                .lock()
                .await
                .get_pin_configuration()
                .unwrap_or_default();
            println!("modbus_timeout_ms: {}", config.modbus_timeout_ms);
        }
        CliCommand::SetModbusRxTimeoutUs(value) => {
            let mut sm = app_context.storage.lock().await;
            let mut config = sm.get_pin_configuration().unwrap_or_default();
            config.modbus_rx_timeout_us = value;
            let _ = sm.set_pin_configuration(&config);
        }
        CliCommand::GetModbusRxTimeoutUs => {
            let config = app_context
                .storage
                .lock()
                .await
                .get_pin_configuration()
                .unwrap_or_default();
            println!("modbus_rx_timeout_us: {}", config.modbus_rx_timeout_us);
        }
        CliCommand::SetModbusScanDelayUs(value) => {
            let mut sm = app_context.storage.lock().await;
            let mut config = sm.get_pin_configuration().unwrap_or_default();
            config.modbus_scan_delay_us = value;
            let _ = sm.set_pin_configuration(&config);
        }
        CliCommand::GetModbusScanDelayUs => {
            let config = app_context
                .storage
                .lock()
                .await
                .get_pin_configuration()
                .unwrap_or_default();
            println!("modbus_scan_delay_us: {}", config.modbus_scan_delay_us);
        }
        CliCommand::SetModbusInterFrameDelayUs(value) => {
            let mut sm = app_context.storage.lock().await;
            let mut config = sm.get_pin_configuration().unwrap_or_default();
            config.modbus_inter_frame_delay_us = value;
            let _ = sm.set_pin_configuration(&config);
        }
        CliCommand::GetModbusInterFrameDelayUs => {
            let config = app_context
                .storage
                .lock()
                .await
                .get_pin_configuration()
                .unwrap_or_default();
            println!("modbus_inter_frame_delay_us: {}", config.modbus_inter_frame_delay_us);
        }
        CliCommand::SetBleEnabled(value) => {
            let mut sm = app_context.storage.lock().await;
            let mut config = sm.get_pin_configuration().unwrap_or_default();
            config.ble_enabled = value;
            let _ = sm.set_pin_configuration(&config);
        }
        CliCommand::GetBleEnabled => {
            let config = app_context
                .storage
                .lock()
                .await
                .get_pin_configuration()
                .unwrap_or_default();
            println!("ble_enabled: {}", config.ble_enabled);
        }
        CliCommand::SetWifiEnabled(value) => {
            let mut sm = app_context.storage.lock().await;
            let mut config = sm.get_network_configuration().unwrap_or_default();
            config.wifi_enabled = value;
            let _ = sm.set_network_configuration(&config);
        }
        CliCommand::GetWifiEnabled => {
            let config = app_context
                .storage
                .lock()
                .await
                .get_network_configuration()
                .unwrap_or_default();
            println!("wifi_enabled: {}", config.wifi_enabled);
        }
        CliCommand::SetHostname(hostname) => {
            let mut sm = app_context.storage.lock().await;
            let mut config = sm.get_network_configuration().unwrap_or_default();
            config.hostname = hostname;
            let _ = sm.set_network_configuration(&config);
        }
        CliCommand::SetDhcpEnabled(value) => {
            let mut sm = app_context.storage.lock().await;
            let mut config = sm.get_network_configuration().unwrap_or_default();
            config.dhcp_enabled = value;
            let _ = sm.set_network_configuration(&config);
        }
        CliCommand::SetStaticIp(ip) => set_net_field(app_context, |c| c.static_ip = ip).await,
        CliCommand::SetStaticMask(mask) => set_net_field(app_context, |c| c.static_mask = mask).await,
        CliCommand::SetStaticGateway(gateway) => {
            set_net_field(app_context, |c| c.static_gateway = gateway).await;
        }
        CliCommand::SetStaticDns(dns) => set_net_field(app_context, |c| c.static_dns = dns).await,
        CliCommand::GetNetworkConfig => {
            if let Ok(config) = app_context.storage.lock().await.get_network_configuration() {
                let _ = crate::buffers::serialize_to_scratchpad(&config, |json| println!("{}", json));
            }
        }
        CliCommand::GetState => {
            let state = {
                let mut mc_opt = app_context.motor_controller.lock().await;
                mc_opt.as_mut().map(|mc| mc.get_current_state())
            };
            if let Some(state) = state {
                let _ = crate::buffers::serialize_to_scratchpad(&state, |json| println!("{}", json));
            }
        }
        CliCommand::GetStatus => {
            let data = {
                let mut mc_opt = app_context.motor_controller.lock().await;
                mc_opt.as_mut().map(|mc| (mc.get_current_state(), mc.get_config()))
            };
            if let Some((state, config)) = data {
                let st = serde_json::json!({ "state": state, "config": config });
                let _ = crate::buffers::serialize_to_scratchpad(&st, |json| println!("{}", json));
            }
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
    let mut mc_opt = app_context.motor_controller.lock().await;
    if let Some(mc) = mc_opt.as_mut() {
        let _ = mc.update_config(f);
    }
}

async fn set_net_field(app_context: AppContext, f: impl FnOnce(&mut crate::storage::NetworkConfiguration)) {
    let mut sm = app_context.storage.lock().await;
    let mut config = sm.get_network_configuration().unwrap_or_default();
    f(&mut config);
    let _ = sm.set_network_configuration(&config);
}

async fn enqueue(app_context: AppContext, cmd: MotionCommand) {
    if let Some(mc) = app_context.motor_controller.lock().await.as_ref() {
        let _ = mc.get_command_sender().lock().await.enqueue(cmd);
    }
    println!("ok");
}
