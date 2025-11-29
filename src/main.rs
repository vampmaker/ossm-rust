
use std::sync::{Arc, Mutex};
use std::time;

use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio::{AnyInputPin, AnyIOPin, AnyOutputPin};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::prelude::*;
use esp_idf_svc::hal::uart;
use esp_idf_svc::hal::uart::UART1;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{AuthMethod, ClientConfiguration, Configuration, EspWifi};
use esp_idf_svc::io::vfs::BlockingStdIo;
use esp_idf_svc::hal::usb_serial;
use esp_idf_svc::http::server::EspHttpServer;

mod command;
mod context;
mod http_api;
mod motion;
mod motor;
mod motor_57aim30;
mod motor_pwm;
mod storage;

use command::handle_stdin_command;
use context::AppContext;
use motion::{MotorController, MotorControllerConfig};
use motor_57aim30::{Modbus57AIM30Motor, ModbusRTUMaster};


const TARGET_BAUD_RATE: u32 = 115200;


fn main() {
    // It is necessary to call this function once. Otherwise, some patches to the runtime
    // implemented by esp-idf-sys might not link properly. See https://github.com/esp-rs/esp-idf-template/issues/71
    esp_idf_svc::sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("Hello, world!");

    if let Err(e) = run_app() {
        log::error!("App error: {}", e);
        loop {
            log::info!("System halted. Restarting in 10 seconds...");
            FreeRtos::delay_ms(10000);
        }
    }
}

fn run_app() -> anyhow::Result<()> {
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let peripherals = Peripherals::take()?;
    let p = peripherals.pins;

    // setup stdin, note that pins are consumed here
    let all_pins = Arc::new(Mutex::new(vec![
        Some(p.gpio0.into()), Some(p.gpio1.into()), Some(p.gpio2.into()),
        Some(p.gpio3.into()), Some(p.gpio4.into()), Some(p.gpio5.into()),
        Some(p.gpio6.into()), Some(p.gpio7.into()), Some(p.gpio8.into()),
        Some(p.gpio9.into()), Some(p.gpio10.into()), Some(p.gpio11.into()),
        None, None, Some(p.gpio14.into()), // 12, 13 used for stdin
        Some(p.gpio15.into()), Some(p.gpio16.into()), Some(p.gpio17.into()),
        Some(p.gpio18.into()), Some(p.gpio19.into()), Some(p.gpio20.into()),
        Some(p.gpio21.into()), None, None, None, None, Some(p.gpio26.into()),
    ]));

    let usb_serial = usb_serial::UsbSerialDriver::new(
        peripherals.usb_serial,
        p.gpio12,
        p.gpio13,
        &usb_serial::config::Config::default(),
    )?;
    let _blocking_io = BlockingStdIo::usb_serial(usb_serial)?;

    // setup storage manager
    let storage_manager = Arc::new(Mutex::new(Box::new(storage::StorageManager::new(nvs))));

    let app_context = AppContext {
        storage_manager: storage_manager.clone(),
        motor_controller: Arc::new(Mutex::new(None)),
        all_pins,
    };

    // setup stdin command handler
    {
        let app_context = app_context.clone();
        std::thread::spawn(move || handle_stdin_command(app_context));
    }

    // setup wifi
    let mut wifi = EspWifi::new(
        peripherals.modem,
        sysloop.clone(),
        None,
    )?;
    if let Err(e) = connect_wifi(&mut wifi, storage_manager.clone()) {
        log::error!("Failed to connect to wifi: {}", e);
    }

    // setup http api
    let mut server = EspHttpServer::new(&Default::default())?;
    http_api::register_handlers(&mut server, app_context.clone());

    if let Err(e) = run_motor(app_context, peripherals.uart1) {
        log::error!("Motor task failed: {}", e);
    }

    loop {
        FreeRtos::delay_ms(1000);
    }
}

fn connect_wifi(
    wifi: &mut EspWifi,
    storage_manager: Arc<Mutex<Box<storage::StorageManager>>>,
) -> anyhow::Result<()> {
    let (opt_ssid, opt_password) = {
        let storage_manager = storage_manager.lock().unwrap();
        (storage_manager.get_ssid(), storage_manager.get_password())
    };
    if let (Ok(saved_ssid), Ok(saved_password)) = (opt_ssid, opt_password) {
        if saved_ssid.is_empty() {
            log::info!("SSID is empty. Please set it via UART command: set_ssid <your_ssid>");
        } else {
            let mut ssid = heapless::String::<32>::new();
            ssid.push_str(&saved_ssid)
                .map_err(|_| anyhow::anyhow!("SSID is too long"))?;
            let mut password = heapless::String::<64>::new();
            password
                .push_str(&saved_password)
                .map_err(|_| anyhow::anyhow!("Password is too long"))?;

            let wifi_configuration = Configuration::Client(ClientConfiguration {
                ssid,
                password,
                auth_method: AuthMethod::WPA2Personal,
                ..Default::default()
            });
            wifi.set_configuration(&wifi_configuration)?;

            wifi.start()?;
            wifi.connect()?;
            log::info!(
                "WiFi connecting, SSID: {}, Password: {}",
                saved_ssid,
                saved_password
            );
            while !wifi.is_up()? {
                FreeRtos::delay_ms(1);
            }
            log::info!("WiFi connected.");
        }
    } else {
        log::info!("WiFi SSID or password not set. Please set them via UART commands:\r\nset_ssid <your_ssid>\r\nset_password <your_password>");
    }
    Ok(())
}

fn run_motor(app_context: AppContext, uart_peripheral: UART1) -> anyhow::Result<()> {
    let motor_controller_result = (|| -> anyhow::Result<MotorController<'static>> {
        let uart: uart::UartDriver = {
            let pin_config = app_context.storage_manager.lock().unwrap().get_pin_configuration().unwrap_or_default();
    
            let config = uart::config::Config::default()
                .baudrate(Hertz(TARGET_BAUD_RATE))
                .mode(uart::config::Mode::RS485HalfDuplex);    // the driver software will control rts pin, which is connected to the rs485 transceiver's DE/~RE pin
    
            let mut all_pins = app_context.all_pins.lock().unwrap();
            let tx_pin_num = pin_config.modbus_tx as usize;
            let rx_pin_num = pin_config.modbus_rx as usize;
            let rts_pin_num = pin_config.modbus_de_re as usize;
    
            let tx = all_pins.get_mut(tx_pin_num).and_then(|p| p.take());
            let rx = all_pins.get_mut(rx_pin_num).and_then(|p| p.take());
            let rts = all_pins.get_mut(rts_pin_num).and_then(|p| p.take());
    
            match (tx, rx, rts) {
                (Some(tx), Some(rx), Some(rts)) => {
                    log::info!("Using configured pins for UART: tx={}, rx={}, rts={}", tx_pin_num, rx_pin_num, rts_pin_num);
                    uart::UartDriver::new(
                        uart_peripheral,
                        <AnyIOPin as Into<AnyOutputPin>>::into(tx),
                        <AnyIOPin as Into<AnyInputPin>>::into(rx),
                        Option::<AnyIOPin>::None,
                        Some(<AnyIOPin as Into<AnyOutputPin>>::into(rts)),
                        &config,
                    )?
                }
                _ => {
                    log::warn!("Failed to get configured pins, searching for available pins.");
    
                    let mut tx_pin_num = 0;
                    let mut rx_pin_num = 0;
                    let mut rts_pin_num = 0;
    
                    let tx = all_pins.iter_mut().enumerate().find_map(|(i, p)| if p.is_some() { tx_pin_num = i; p.take() } else { None });
                    let rx = all_pins.iter_mut().enumerate().find_map(|(i, p)| if p.is_some() { rx_pin_num = i; p.take() } else { None });
                    let rts = all_pins.iter_mut().enumerate().find_map(|(i, p)| if p.is_some() { rts_pin_num = i; p.take() } else { None });
    
                    if tx.is_none() || rx.is_none() || rts.is_none() {
                        anyhow::bail!("Not enough available pins for UART.");
                    }
    
                    log::info!("Found available pins for UART: tx={}, rx={}, rts={}", tx_pin_num, rx_pin_num, rts_pin_num);
    
                    let new_pin_config = storage::PinConfiguration {
                        modbus_tx: tx_pin_num as u32,
                        modbus_rx: rx_pin_num as u32,
                        modbus_de_re: rts_pin_num as u32,
                    };
                    app_context.storage_manager.lock().unwrap().set_pin_configuration(&new_pin_config)?;
                    log::info!("Saved new pin configuration to NVS.");
    
                    let tx: AnyOutputPin = tx.unwrap().into();
                    let rx: AnyInputPin = rx.unwrap().into();
                    let rts: AnyOutputPin = rts.unwrap().into();
    
                    uart::UartDriver::new(
                        uart_peripheral,
                        tx,
                        rx,
                        Option::<AnyIOPin>::None,
                        Some(rts),
                        &config,
                    )?
                }
            }
        };

        let modbus = ModbusRTUMaster::new(uart, Option::<AnyOutputPin>::None, 1);

        let mut motor = Modbus57AIM30Motor::new(modbus);
        if let Err(e) = motor.enable_modbus_communication() {
            log::info!("Failed to enable modbus, trying to scan and configure: {}", e);
            let motor_scan_result = motor.modbus_scan().map_err(|e| anyhow::anyhow!("Failed to scan motor device. Please check connection to the motor. {:?}", e))?;
            log::info!("Motor device found, baud rate: {}, device id: {}", motor_scan_result.baud_rate, motor_scan_result.device_id);
            if motor_scan_result.baud_rate != TARGET_BAUD_RATE {
                motor.modbus_set_baud_rate(TARGET_BAUD_RATE).map_err(|e| anyhow::anyhow!("Failed to set baud rate to {}: {:?}", TARGET_BAUD_RATE, e))?;
                log::info!("Motor baud rate set to {}, please power cycle the motor.", TARGET_BAUD_RATE);
            }
        }
        motor.enable_modbus_communication().map_err(|e| anyhow::anyhow!("Failed to enable modbus communication: {:?}", e))?;

        let motor_config = {
            let sm = app_context.storage_manager.lock().unwrap();
            sm.get_motor_config()
        };

        let motor_config = match motor_config {
            Ok(config) => {
                log::info!("Loaded motor config from NVS");
                config
            }
            Err(_) => {
                log::info!("No motor config found in NVS, using default");
                let default_config = MotorControllerConfig::default();
                app_context.storage_manager.lock().unwrap().set_motor_config(&default_config)?;
                default_config
            }
        };

        let mut motor_controller = MotorController::new(Box::new(motor), motor_config);
        motor_controller.init_motor().map_err(|e| anyhow::anyhow!("Failed to init motor: {:?}", e))?;
        Ok(motor_controller)
    })();

    match motor_controller_result {
        Ok(mc) => {
            log::info!("Motor initialized, starting motor loop");
            *app_context.motor_controller.lock().unwrap() = Some(Box::new(mc));

            let mut last_config_check = time::Instant::now();
            let mut last_saved_config_version = app_context.motor_controller.lock().unwrap().as_ref().map_or(0, |mc| mc.get_config_version());
            let mut update_counter = 0;
            let mut last_update_counter_reset = time::Instant::now();

            loop {
                {
                    let mut motor_controller_lock = app_context.motor_controller.lock().unwrap();
                    if let Some(controller) = motor_controller_lock.as_mut() {
                        if last_config_check.elapsed() > time::Duration::from_millis(200) {
                            last_config_check = time::Instant::now();
                            let current_version = controller.get_config_version();
                            if current_version != last_saved_config_version {
                                let config = controller.get_config();
                                log::info!("Config updated, saving to NVS");
                                if let Err(e) = app_context.storage_manager.lock().unwrap().set_motor_config(&config) {
                                    log::error!("Failed to save motor config: {}", e);
                                } else {
                                    last_saved_config_version = current_version;
                                }
                            }
                        }
            
                        if let Err(e) = controller.cycle() {
                            log::error!("Failed to cycle: {}", e);
                        }
                    } else {
                        log::error!("Motor controller lost, stopping motor loop");
                        break;
                    }
                }
        
                update_counter += 1;
                if last_update_counter_reset.elapsed() > time::Duration::from_secs(60) {
                    log::info!("Motor task update per second: {}", update_counter as f64 / 60.0);
                    last_update_counter_reset = time::Instant::now();
                    update_counter = 0;
                }
            }
        },
        Err(e) => {
            log::error!("Failed to initialize motor: {}. Motor task will not run.", e);
            return Err(e);
        }
    }
    Ok(())
}
