use std::io::{Read, Write as StdWrite};
use std::os::fd::FromRawFd;

use crate::context::AppContext;
use crate::motion::MotorControllerConfig;
use embedded_cli::cli::CliBuilder;
use embedded_cli::Command;
use embedded_io::ErrorType;
use embedded_io::Write as EmbeddedWrite;
use esp_idf_svc::hal::delay::FreeRtos;

#[derive(Command, Clone, Debug)]
#[command(help_title = "ossm-rust")]
enum BaseCommand<'a> {
    SetWifiSsid {
        /// WiFi SSID
        ssid: &'a str,
    },
    SetWifiPassword {
        /// WiFi password
        password: &'a str,
    },
    SetPinModbusTx {
        /// Modbus TX pin
        pin: u32,
    },
    SetPinModbusRx {
        /// Modbus RX pin
        pin: u32,
    },
    SetPinModbusDeRe {
        /// Modbus DE/RE pin
        pin: u32,
    },
    GetPinConfiguration,
    SetMotorConfig {
        /// Motor config in JSON format
        json: &'a str,
    },
    GetMotorConfig,
    Pause,
    Start,
    Reset,
    SetBpm {
        /// Motor BPM
        bpm: f32,
    },
    SetWave {
        /// Motor waveform: sine, thrust, spline
        wave: &'a str,
    },
    SetPausedPosition {
        /// Motor position when paused (0.0 to 1.0)
        position: f32,
    },
    SetDepth {
        /// Motor stroke depth (0.0 to 1.0)
        depth: f32,
    },
    SetDepthTop {
        /// Depth direction: true or false
        v: bool,
    },
    SetSharpness {
        /// Sharpness for thrust wave (0.01 to 0.99)
        sharpness: f32,
    },
    SetSplinePoints {
        /// Spline points separated by space (0.0 to 1.0)
        points: &'a str, // embedded-cli doesn't support Vec<f32> directly as argument, so we parse string manually
    },
    SetModbusTimeoutMs {
        /// Modbus per-read timeout in ms (0 = use default for baud rate)
        value: u32,
    },
    GetModbusTimeoutMs,
    SetModbusScanDelayUs {
        /// Modbus scan inter-probe delay in us (0 = use Modbus t3.5 only)
        value: u32,
    },
    GetModbusScanDelayUs,
}

struct StdoutAdapter(std::io::Stdout);

impl ErrorType for StdoutAdapter {
    type Error = std::io::Error;
}

impl EmbeddedWrite for StdoutAdapter {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let n = StdWrite::write(&mut self.0, buf)?;
        StdWrite::flush(&mut self.0)?;
        Ok(n)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        StdWrite::flush(&mut self.0)
    }
}

pub fn handle_stdin_command(app_context: AppContext) {
    let mut cli = CliBuilder::default()
        .writer(StdoutAdapter(std::io::stdout()))
        .command_buffer([0u8; 512])
        .history_buffer([0u8; 2048])
        .build()
        .unwrap();
    let mut processor = BaseCommand::processor(|_cli, command| {
        execute_command(command, &app_context);
        Ok(())
    });
    let mut buf = [0u8; 1];
    let mut stdin = unsafe { std::fs::File::from_raw_fd(0) };

    loop {
        match stdin.read(&mut buf) {
            Ok(1) => {
                let byte = if buf[0] == 0x7F { 0x08 } else { buf[0] };    // 0x7F is backspace, convert to 0x08 for embedded-cli
                if let Err(e) = cli.process_byte::<BaseCommand<'_>, _>(byte, &mut processor) {
                    log::error!("CLI processing error: {:?}", e);
                }
                std::io::stdout().flush().unwrap();
            }
            Ok(_) => {
                FreeRtos::delay_ms(10);
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::Interrupted
                {
                    FreeRtos::delay_ms(10);
                } else {
                    log::error!("stdin read error: {e}");
                    FreeRtos::delay_ms(100);
                }
            }
        }
    }
}

fn execute_command(command: BaseCommand<'_>, app_context: &AppContext) {
    match command {
        BaseCommand::SetWifiSsid { ssid } => {
            app_context.storage_manager.lock().unwrap().set_ssid(&ssid).unwrap();
            log::info!("SSID saved: {}, restart to apply", ssid);
        }
        BaseCommand::SetWifiPassword { password } => {
            app_context.storage_manager.lock().unwrap().set_password(&password).unwrap();
            log::info!("Password saved: {}, restart to apply", password);
        }
        BaseCommand::SetPinModbusTx { pin } => {
            let mut sm = app_context.storage_manager.lock().unwrap();
            let mut config = sm.get_pin_configuration().unwrap_or_default();
            config.modbus_tx = pin;
            sm.set_pin_configuration(&config).unwrap();
            log::info!("Modbus TX pin set to {}, restart to apply", pin);
        }
        BaseCommand::SetPinModbusRx { pin } => {
            let mut sm = app_context.storage_manager.lock().unwrap();
            let mut config = sm.get_pin_configuration().unwrap_or_default();
            config.modbus_rx = pin;
            sm.set_pin_configuration(&config).unwrap();
            log::info!("Modbus RX pin set to {}, restart to apply", pin);
        }
        BaseCommand::SetPinModbusDeRe { pin } => {
            let mut sm = app_context.storage_manager.lock().unwrap();
            let mut config = sm.get_pin_configuration().unwrap_or_default();
            config.modbus_de_re = pin;
            sm.set_pin_configuration(&config).unwrap();
            log::info!("Modbus DE/RE pin set to {}, restart to apply", pin);
        }
        BaseCommand::GetPinConfiguration => {
            match app_context.storage_manager.lock().unwrap().get_pin_configuration() {
                Ok(config) => {
                    let json = serde_json::to_string_pretty(&config).unwrap();
                    println!("{}", json);
                }
                Err(e) => {
                    log::error!("Failed to get pin config: {}", e);
                }
            }
        }
        BaseCommand::SetMotorConfig { json } => {
            match serde_json::from_str::<MotorControllerConfig>(json) {
                Ok(config) => {
                    let mut mc_opt = app_context.motor_controller.lock().unwrap();
                    if let Some(mc) = mc_opt.as_mut() {
                        mc.set_config(config).unwrap();
                        log::info!("Motor config updated");
                    } else {
                        log::error!("Motor controller not initialized");
                    }
                }
                Err(e) => {
                    log::error!("Failed to parse motor config: {}", e);
                }
            }
        }
        BaseCommand::GetMotorConfig => {
            let mut mc_opt = app_context.motor_controller.lock().unwrap();
            if let Some(mc) = mc_opt.as_mut() {
                let config = mc.get_config();
                let json = serde_json::to_string_pretty(&config).unwrap();
                println!("{}", json);
            } else {
                log::error!("Motor controller not initialized");
            }
        }
        BaseCommand::Pause => {
            let mut mc_opt = app_context.motor_controller.lock().unwrap();
            if let Some(mc) = mc_opt.as_mut() {
                if let Err(e) = mc.update_config(|config| {
                    config.paused = true;
                }) {
                    log::error!("Failed to set motor config: {}", e);
                } else {
                    log::info!("Motor paused");
                }
            } else {
                log::error!("Motor controller not initialized");
            }
        }
        BaseCommand::Start => {
            let mut mc_opt = app_context.motor_controller.lock().unwrap();
            if let Some(mc) = mc_opt.as_mut() {
                if let Err(e) = mc.update_config(|config| {
                    config.paused = false;
                }) {
                    log::error!("Failed to set motor config: {}", e);
                } else {
                    log::info!("Motor started");
                }
            } else {
                log::error!("Motor controller not initialized");
            }
        }
        BaseCommand::Reset => {
            log::info!("Restarting device...");
            FreeRtos::delay_ms(100);
            esp_idf_svc::hal::reset::restart();
        }
        BaseCommand::SetBpm { bpm } => {
            if bpm <= 0.0 {
                log::error!("BPM must be greater than 0.0");
                return
            }
            let mut mc_opt = app_context.motor_controller.lock().unwrap();
            if let Some(mc) = mc_opt.as_mut() {
                if let Err(e) = mc.update_config(|config| {
                    config.bpm = bpm;
                }) {
                    log::error!("Failed to set motor config: {}", e);
                } else {
                    log::info!("BPM set to {}", bpm);
                }
            } else {
                log::error!("Motor controller not initialized");
            }
        }
        BaseCommand::SetWave { wave } => {
            if wave == "sine" || wave == "thrust" || wave == "spline" {
                let mut mc_opt = app_context.motor_controller.lock().unwrap();
                if let Some(mc) = mc_opt.as_mut() {
                    if let Err(e) = mc.update_config(|config| {
                        config.wave_func = wave.to_string();
                    }) {
                        log::error!("Failed to set motor config: {}", e);
                    } else {
                        log::info!("Wave function set to {}", wave);
                    }
                } else {
                    log::error!("Motor controller not initialized");
                }
            } else {
                log::error!("Invalid wave function: {}. Use 'sine' or 'thrust' or 'spline'", wave);
            }
        }
        BaseCommand::SetPausedPosition { position } => {
            if position <= 0.0 || position >= 1.0 {
                log::error!("Paused position must be between 0.0 and 1.0");
                return
            }
             let mut mc_opt = app_context.motor_controller.lock().unwrap();
            if let Some(mc) = mc_opt.as_mut() {
                if let Err(e) = mc.update_config(|config| {
                    config.paused_position = position;
                }) {
                    log::error!("Failed to set motor config: {}", e);
                } else {
                    log::info!("Paused position set to {}", position);
                }
            } else {
                log::error!("Motor controller not initialized");
            }
        }
        BaseCommand::SetDepth { depth } => {
            if depth <= 0.01 || depth >= 1.0 {
                log::error!("Depth must be between 0.01 and 1.0");
                return
            }
             let mut mc_opt = app_context.motor_controller.lock().unwrap();
            if let Some(mc) = mc_opt.as_mut() {
                if let Err(e) = mc.update_config(|config| {
                    config.depth = depth;
                }) {
                    log::error!("Failed to set motor config: {}", e);
                } else {
                    log::info!("Depth set to {}", depth);
                }
            } else {
                log::error!("Motor controller not initialized");
            }
        }
        BaseCommand::SetDepthTop { v } => {
            let mut mc_opt = app_context.motor_controller.lock().unwrap();
            if let Some(mc) = mc_opt.as_mut() {
                if let Err(e) = mc.update_config(|config| {
                    config.depth_top = v;
                }) {
                    log::error!("Failed to set motor config: {}", e);
                } else {
                    log::info!("Depth top set to {}", v);
                }
            } else {
                log::error!("Motor controller not initialized");
            }
        }
        BaseCommand::SetSharpness { sharpness } => {
             let mut mc_opt = app_context.motor_controller.lock().unwrap();
             if sharpness < 0.01 || sharpness >= 1.0 {
                log::error!("Sharpness must be between 0.01 and 0.99");
                return
             }
            if let Some(mc) = mc_opt.as_mut() {
                if let Err(e) = mc.update_config(|config| {
                    config.sharpness = sharpness;
                }) {
                    log::error!("Failed to set motor config: {}", e);
                } else {
                    log::info!("Sharpness set to {}", sharpness);
                }
            } else {
                log::error!("Motor controller not initialized");
            }
        }
        BaseCommand::SetSplinePoints { points } => {
             let points_parsed: Result<Vec<f32>, _> = points.split_whitespace().map(|s| s.parse::<f32>()).collect();
            match points_parsed {
                Ok(points_vec) => {
                    if points_vec.is_empty() {
                        log::error!("Spline points cannot be empty");
                        return;
                    }
                    for &p in &points_vec {
                        if !(0.0..=1.0).contains(&p) {
                            log::error!("Spline points must be between 0.0 and 1.0");
                            return;
                        }
                    }

                    let mut mc_opt = app_context.motor_controller.lock().unwrap();
                    if let Some(mc) = mc_opt.as_mut() {
                        if let Err(e) = mc.update_config(|config| {
                            config.spline_points = points_vec.clone();
                        }) {
                            log::error!("Failed to set motor config: {}", e);
                        } else {
                            log::info!("Spline points set to {:?}", points_vec);
                        }
                    } else {
                        log::error!("Motor controller not initialized");
                    }
                }
                Err(_) => log::error!("Invalid spline points value: {}", points),
            }
        }
        BaseCommand::SetModbusTimeoutMs { value } => {
            if value > 1000 {
                log::error!("modbus_timeout_ms must be 0..=1000 (0 = use default)");
                return;
            }
            let mut sm = app_context.storage_manager.lock().unwrap();
            let mut config = sm.get_pin_configuration().unwrap_or_default();
            config.modbus_timeout_ms = value;
            sm.set_pin_configuration(&config).unwrap();
            if value == 0 {
                log::info!("modbus_timeout_ms set to 0 (use default for baud rate), restart to apply");
            } else {
                log::info!("modbus_timeout_ms set to {}ms, restart to apply", value);
            }
        }
        BaseCommand::GetModbusTimeoutMs => {
            let config = app_context.storage_manager.lock().unwrap().get_pin_configuration().unwrap_or_default();
            if config.modbus_timeout_ms == 0 {
                println!("modbus_timeout_ms: 0 (using default for baud rate)");
            } else {
                println!("modbus_timeout_ms: {}ms", config.modbus_timeout_ms);
            }
        }
        BaseCommand::SetModbusScanDelayUs { value } => {
            if value > 200000 {
                log::error!("modbus_scan_delay_us must be 0..=200000 (0 = use Modbus t3.5 only)");
                return;
            }
            let mut sm = app_context.storage_manager.lock().unwrap();
            let mut config = sm.get_pin_configuration().unwrap_or_default();
            config.modbus_scan_delay_us = value;
            sm.set_pin_configuration(&config).unwrap();
            if value == 0 {
                log::info!("modbus_scan_delay_us set to 0 (use Modbus t3.5 only), restart to apply");
            } else {
                log::info!("modbus_scan_delay_us set to {}us, restart to apply", value);
            }
        }
        BaseCommand::GetModbusScanDelayUs => {
            let config = app_context.storage_manager.lock().unwrap().get_pin_configuration().unwrap_or_default();
            if config.modbus_scan_delay_us == 0 {
                println!("modbus_scan_delay_us: 0 (using Modbus t3.5 only)");
            } else {
                println!("modbus_scan_delay_us: {}us", config.modbus_scan_delay_us);
            }
        }
    }
}
