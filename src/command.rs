use std::io::{self, BufRead};
use std::sync::{Arc, Mutex};
use esp_idf_svc::hal::delay::FreeRtos;
use crate::storage::StorageManager;
use crate::motion::MotorControllerConfig;
use crate::context::AppContext;

pub fn handle_stdin_command(app_context: AppContext) {
    let stdin = io::stdin();
    loop {
        let mut handle = stdin.lock();
        let mut cmdline = String::new();
        match handle.read_line(&mut cmdline) {
            Ok(_) => {

            }
            Err(e) => {
                match e.kind() {
                    std::io::ErrorKind::WouldBlock
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::Interrupted => {
                        FreeRtos::delay_ms(10);
                        continue;
                    }
                    _ => {
                        log::info!("handle_uart_command: read from stdin failed: {e}");
                        continue;
                    }
                }
            }
        }

        let cmdline = cmdline.trim();

        log::info!("Command: {}", cmdline);

        // parse and execute command
        let parts = cmdline.splitn(2, ' ').collect::<Vec<&str>>();
        let command = parts[0];
        let args = if parts.len() > 1 { parts[1] } else { "" };

        match command {
            "set_wifi_ssid" => {
                app_context.storage_manager.lock().unwrap().set_ssid(args).unwrap();
                log::info!("SSID saved: {}, restart to apply", args);
            } ,
            "set_wifi_password" => {
                app_context.storage_manager.lock().unwrap().set_password(args).unwrap();
                log::info!("Password saved: {}, restart to apply", args);
            } ,
            "set_pin_modbus_tx" => {
                match args.parse::<u32>() {
                    Ok(pin) => {
                        let mut sm = app_context.storage_manager.lock().unwrap();
                        let mut config = sm.get_pin_configuration().unwrap_or_default();
                        config.modbus_tx = pin;
                        sm.set_pin_configuration(&config).unwrap();
                        log::info!("Modbus TX pin set to {}, restart to apply", pin);
                    }
                    Err(_) => log::error!("Invalid pin value: {}", args),
                }
            },
            "set_pin_modbus_rx" => {
                match args.parse::<u32>() {
                    Ok(pin) => {
                        let mut sm = app_context.storage_manager.lock().unwrap();
                        let mut config = sm.get_pin_configuration().unwrap_or_default();
                        config.modbus_rx = pin;
                        sm.set_pin_configuration(&config).unwrap();
                        log::info!("Modbus RX pin set to {}, restart to apply", pin);
                    }
                    Err(_) => log::error!("Invalid pin value: {}", args),
                }
            },
            "set_pin_modbus_de_re" => {
                match args.parse::<u32>() {
                    Ok(pin) => {
                        let mut sm = app_context.storage_manager.lock().unwrap();
                        let mut config = sm.get_pin_configuration().unwrap_or_default();
                        config.modbus_de_re = pin;
                        sm.set_pin_configuration(&config).unwrap();
                        log::info!("Modbus DE/RE pin set to {}, restart to apply", pin);
                    }
                    Err(_) => log::error!("Invalid pin value: {}", args),
                }
            },
            "get_pin_configuration" => {
                match app_context.storage_manager.lock().unwrap().get_pin_configuration() {
                    Ok(config) => {
                        let json = serde_json::to_string_pretty(&config).unwrap();
                        println!("{}", json);
                    }
                    Err(e) => {
                        log::error!("Failed to get pin config: {}", e);
                    }
                }
            },
            "set_motor_config" => {
                match serde_json::from_str::<MotorControllerConfig>(args) {
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
            } ,
            "get_motor_config" => {
                let mut mc_opt = app_context.motor_controller.lock().unwrap();
                if let Some(mc) = mc_opt.as_mut() {
                    let config = mc.get_config();
                    let json = serde_json::to_string_pretty(&config).unwrap();
                    println!("{}", json);
                } else {
                    log::error!("Motor controller not initialized");
                }
            },
            "pause" => {
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
            },
            "start" => {
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
            },
            "set_bpm" => {
                match args.parse::<f32>() {
                    Ok(bpm) => {
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
                    Err(_) => log::error!("Invalid BPM value: {}", args),
                }
            },
            "set_wave" => {
                if args == "sine" || args == "thrust" || args == "spline" {
                    let wave = args.to_string();
                    let mut mc_opt = app_context.motor_controller.lock().unwrap();
                    if let Some(mc) = mc_opt.as_mut() {
                        if let Err(e) = mc.update_config(|config| {
                            config.wave_func = wave;
                        }) {
                            log::error!("Failed to set motor config: {}", e);
                        } else {
                            log::info!("Wave function set to {}", args);
                        }
                    } else {
                        log::error!("Motor controller not initialized");
                    }
                } else {
                    log::error!("Invalid wave function: {}. Use 'sine' or 'thrust' or 'spline'", args);
                }
            },
            "set_paused_position" => {
                match args.parse::<f32>() {
                    Ok(pos) => {
                        let mut mc_opt = app_context.motor_controller.lock().unwrap();
                        if let Some(mc) = mc_opt.as_mut() {
                            if let Err(e) = mc.update_config(|config| {
                                config.paused_position = pos;
                            }) {
                                log::error!("Failed to set motor config: {}", e);
                            } else {
                                log::info!("Paused position set to {}", pos);
                            }
                        } else {
                            log::error!("Motor controller not initialized");
                        }
                    }
                    Err(_) => log::error!("Invalid paused position value: {}", args),
                }
            },
            "set_depth" => {
                match args.parse::<f32>() {
                    Ok(depth) => {
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
                    Err(_) => log::error!("Invalid depth value: {}", args),
                }
            },
            "set_depth_top" => {
                match args.parse::<bool>() {
                    Ok(v) => {
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
                    Err(_) => log::error!("Invalid boolean value: {}. Use 'true' or 'false'", args),
                }
            },
            "set_sharpness" => {
                match args.parse::<f32>() {
                    Ok(sharpness) => {
                        let mut mc_opt = app_context.motor_controller.lock().unwrap();
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
                    Err(_) => log::error!("Invalid sharpness value: {}", args),
                }
            },
            "help" => {
                log::info!("Available commands:");
                log::info!("  help                           - Show this help message");
                log::info!("  set_wifi_ssid <ssid>                - Set WiFi SSID");
                log::info!("  set_wifi_password <password>        - Set WiFi password");
                log::info!("  get_pin_configuration          - Get pin configuration in JSON format");
                log::info!("  set_pin_modbus_tx <pin>        - Set Modbus TX pin");
                log::info!("  set_pin_modbus_rx <pin>        - Set Modbus RX pin");
                log::info!("  set_pin_modbus_de_re <pin>     - Set Modbus DE/RE pin");
                log::info!("  get_motor_config               - Get motor config in JSON format");
                log::info!("  set_motor_config <json>        - Set motor config from a JSON string");
                log::info!("  pause                          - Pause the motor");
                log::info!("  start                          - Start the motor");
                log::info!("  set_bpm <bpm>                  - Set motor BPM");
                log::info!("  set_wave <sine|thrust|spline>         - Set motor waveform");
                log::info!("  set_paused_position <position> - Set motor position when paused (0.0 to 1.0)");
                log::info!("  set_depth <depth>              - Set motor stroke depth (0.0 to 1.0)");
                log::info!("  set_depth_top <true|false>     - Set depth direction");
                log::info!("  set_sharpness <sharpness>      - Set sharpness for thrust wave (0.01 to 0.99)");
                log::info!("  set_spline_points <p1> <p2> ... - Set points for spline wave (0.0 to 1.0)");
            },
            "set_spline_points" => {
                let points: Result<Vec<f32>, _> = args.split_whitespace().map(|s| s.parse::<f32>()).collect();
                match points {
                    Ok(points) => {
                        if points.is_empty() {
                            log::error!("Spline points cannot be empty");
                            return;
                        }
                        for &p in &points {
                            if !(0.0..=1.0).contains(&p) {
                                log::error!("Spline points must be between 0.0 and 1.0");
                                return;
                            }
                        }

                        let mut mc_opt = app_context.motor_controller.lock().unwrap();
                        if let Some(mc) = mc_opt.as_mut() {
                            if let Err(e) = mc.update_config(|config| {
                                config.spline_points = points.clone();
                            }) {
                                log::error!("Failed to set motor config: {}", e);
                            } else {
                                log::info!("Spline points set to {:?}", points);
                            }
                        } else {
                            log::error!("Motor controller not initialized");
                        }
                    }
                    Err(_) => log::error!("Invalid spline points value: {}", args),
                }
            },
            _ => {
                log::error!("Unknown command: {}", command);
                continue
            },
        }
    }
}
