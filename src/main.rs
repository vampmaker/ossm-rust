
use std::sync::{Arc, Mutex};
use std::time;

use embassy_executor::Executor;
use embassy_time::Timer;
use static_cell::StaticCell;

use esp_idf_svc::hal::gpio::{AnyInputPin, AnyIOPin, AnyOutputPin};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::units::Hertz;
use esp_idf_svc::hal::uart;
use esp_idf_svc::hal::uart::UART1;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::io::vfs::MountedEventfs;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::timer::EspTaskTimerService;
use esp_idf_svc::wifi::{AsyncWifi, AuthMethod, ClientConfiguration, Configuration, EspWifi};
use esp_idf_svc::io::vfs::BlockingStdIo;
use esp_idf_svc::hal::usb_serial;
mod ble_api;
mod command;
mod context;
mod http_api;
mod motion;
mod motor;
mod motor_57aim30;
mod storage;

use command::handle_stdin_command;
use context::AppContext;
use motion::{MotorController, MotorControllerConfig};
use motor::Motor;
use motor_57aim30::{Modbus57AIM30Motor, ModbusRTUMaster};


const TARGET_BAUD_RATE: u32 = 115200;

static EXECUTOR: StaticCell<Executor> = StaticCell::new();

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
            std::thread::sleep(time::Duration::from_secs(10));
        }
    }
}

fn run_app() -> anyhow::Result<()> {
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;
    let peripherals = Peripherals::take()?;
    let p = peripherals.pins;

    // eventfd VFS is required by async-io (used by the edge-http server sockets)
    let eventfs = MountedEventfs::mount(5)?;
    std::mem::forget(eventfs);

    // setup stdin, note that USB serial pins are excluded from all_pins
    #[cfg(esp32c6)]
    let all_pins = Arc::new(Mutex::new(vec![
        Some(p.gpio0.into()), Some(p.gpio1.into()), Some(p.gpio2.into()),
        Some(p.gpio3.into()), Some(p.gpio4.into()), Some(p.gpio5.into()),
        Some(p.gpio6.into()), Some(p.gpio7.into()), Some(p.gpio8.into()),
        Some(p.gpio9.into()), Some(p.gpio10.into()), Some(p.gpio11.into()),
        None, None, Some(p.gpio14.into()), // 12, 13 used for USB serial
        Some(p.gpio15.into()), Some(p.gpio16.into()), Some(p.gpio17.into()),
        Some(p.gpio18.into()), Some(p.gpio19.into()), Some(p.gpio20.into()),
        Some(p.gpio21.into()), None, None, None, None, Some(p.gpio26.into()),
    ]));

    #[cfg(esp32s3)]
    let all_pins = Arc::new(Mutex::new(vec![
        Some(p.gpio0.into()),  Some(p.gpio1.into()),  Some(p.gpio2.into()),
        Some(p.gpio3.into()),  Some(p.gpio4.into()),  Some(p.gpio5.into()),
        Some(p.gpio6.into()),  Some(p.gpio7.into()),  Some(p.gpio8.into()),
        Some(p.gpio9.into()),  Some(p.gpio10.into()), Some(p.gpio11.into()),
        Some(p.gpio12.into()), Some(p.gpio13.into()), Some(p.gpio14.into()),
        Some(p.gpio15.into()), Some(p.gpio16.into()), Some(p.gpio17.into()),
        Some(p.gpio18.into()), None, None,             // 19, 20 used for USB serial
        Some(p.gpio21.into()), None, None, None, None, // 22-25 not exposed on most S3 modules
        None, None, None, None, None, None, None,      // 26-32 SPI flash
        Some(p.gpio33.into()), Some(p.gpio34.into()), Some(p.gpio35.into()),
        Some(p.gpio36.into()), Some(p.gpio37.into()), Some(p.gpio38.into()),
        Some(p.gpio39.into()), Some(p.gpio40.into()), Some(p.gpio41.into()),
        Some(p.gpio42.into()), Some(p.gpio43.into()), Some(p.gpio44.into()),
        Some(p.gpio45.into()), Some(p.gpio46.into()), Some(p.gpio47.into()),
        Some(p.gpio48.into()),
    ]));

    #[cfg(esp32c6)]
    let usb_serial = usb_serial::UsbSerialDriver::new(
        peripherals.usb_serial,
        p.gpio12,
        p.gpio13,
        &usb_serial::config::Config::default(),
    )?;
    #[cfg(esp32s3)]
    let usb_serial = usb_serial::UsbSerialDriver::new(
        peripherals.usb_serial,
        p.gpio19,
        p.gpio20,
        &usb_serial::config::Config::default(),
    )?;
    let blocking_io = BlockingStdIo::usb_serial(usb_serial)?;
    std::mem::forget(blocking_io);

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
        let builder = std::thread::Builder::new().name("stdin_command".to_string()).stack_size(32768);
        builder.spawn(move || handle_stdin_command(app_context)).unwrap();
    }

    // setup ble server
    let ble_enabled = app_context.storage_manager.lock().unwrap().get_pin_configuration().map(|c| c.ble_enabled).unwrap_or(true);
    if ble_enabled {
        let app_context = app_context.clone();
        let builder = std::thread::Builder::new().name("ble_server".to_string()).stack_size(16384);
        builder.spawn(move || {
            if let Err(e) = ble_api::run_ble_server(app_context) {
                log::error!("BLE server error: {}", e);
            }
        }).unwrap();
    } else {
        log::info!("BLE server disabled by configuration");
    }


    let net_conf = app_context.storage_manager.lock().unwrap().get_network_configuration().unwrap_or_default();

    let mut sta_conf = esp_idf_svc::netif::NetifConfiguration::wifi_default_client();
    if !net_conf.dhcp_enabled {
        let ip: std::net::Ipv4Addr = net_conf.static_ip.parse().unwrap_or_else(|_| std::net::Ipv4Addr::new(192, 168, 1, 100));
        let gateway: std::net::Ipv4Addr = net_conf.static_gateway.parse().unwrap_or_else(|_| std::net::Ipv4Addr::new(192, 168, 1, 1));
        
        let mask_val = if let Ok(val) = net_conf.static_mask.parse::<u8>() {
            val
        } else if let Ok(addr) = net_conf.static_mask.parse::<std::net::Ipv4Addr>() {
            u32::from_be_bytes(addr.octets()).leading_ones() as u8
        } else {
            24
        };
        let dns = net_conf.static_dns.parse::<std::net::Ipv4Addr>().ok();

        sta_conf.ip_configuration = Some(esp_idf_svc::ipv4::Configuration::Client(
            esp_idf_svc::ipv4::ClientConfiguration::Fixed(esp_idf_svc::ipv4::ClientSettings {
                ip,
                subnet: esp_idf_svc::ipv4::Subnet {
                    gateway,
                    mask: esp_idf_svc::ipv4::Mask(mask_val),
                },
                dns,
                secondary_dns: None,
            })
        ));
    } else {
        sta_conf.ip_configuration = Some(esp_idf_svc::ipv4::Configuration::Client(
            esp_idf_svc::ipv4::ClientConfiguration::DHCP(esp_idf_svc::ipv4::DHCPClientSettings {
                hostname: Some(heapless::String::try_from(net_conf.hostname.as_str()).unwrap_or_else(|_| heapless::String::try_from("ossm").unwrap())),
            })
        ));
    }

    let driver = esp_idf_svc::wifi::WifiDriver::new(peripherals.modem, sysloop.clone(), None)?;
    let sta_netif = esp_idf_svc::netif::EspNetif::new_with_conf(&sta_conf)?;
    
    if let Ok(c_hostname) = std::ffi::CString::new(net_conf.hostname.as_str()) {
        unsafe {
            use esp_idf_svc::handle::RawHandle;
            let _ = esp_idf_svc::sys::esp_netif_set_hostname(sta_netif.handle(), c_hostname.as_ptr());
        }
    }

    let wifi = EspWifi::wrap_all(
        driver,
        sta_netif,
        esp_idf_svc::netif::EspNetif::new(esp_idf_svc::netif::NetifStack::Ap)?,
    )?;
    let wifi = AsyncWifi::wrap(wifi, sysloop, EspTaskTimerService::new()?)?;

    // Run the Embassy executor on the main thread; it drives the network task,
    // background NVS saving task, and motor task cooperatively.
    let executor = EXECUTOR.init(Executor::new());

    #[cfg(esp32s3)]
    {
        let mut cfg = esp_idf_svc::hal::task::thread::ThreadSpawnConfiguration::get().unwrap_or_default();
        cfg.pin_to_core = Some(esp_idf_svc::hal::cpu::Core::Core1);
        cfg.priority = 15;
        cfg.stack_size = 32768;
        cfg.set().unwrap();

        let app_context_motor = app_context.clone();
        let uart1 = peripherals.uart1;
        std::thread::spawn(move || {
            log::info!("ESP32-S3: Motor loop running on dedicated Core 1 at FreeRTOS priority 15");
            if let Err(e) = esp_idf_svc::hal::task::block_on(run_motor(app_context_motor, uart1)) {
                log::error!("Motor task failed: {}", e);
            }
        });
    }

    #[cfg(esp32c6)]
    unsafe {
        esp_idf_svc::sys::vTaskPrioritySet(std::ptr::null_mut(), 15);
    }

    executor.run(|spawner| {
        spawner.spawn(net_task(wifi, app_context.clone()).unwrap());
        #[cfg(esp32c6)]
        spawner.spawn(motor_task(app_context.clone(), peripherals.uart1).unwrap());
        spawner.spawn(nvs_saver_task(app_context.clone()).unwrap());
    })
}

#[cfg(esp32c6)]
#[embassy_executor::task]
async fn motor_task(app_context: AppContext, uart_peripheral: UART1<'static>) {
    if let Err(e) = run_motor(app_context, uart_peripheral).await {
        log::error!("Motor task failed: {}", e);
    }
    log::info!("Motor task has returned, command interface remains available");
}

#[embassy_executor::task]
async fn nvs_saver_task(app_context: AppContext) {
    let mut last_saved_version = 0;
    loop {
        Timer::after_millis(500).await;
        let to_save = {
            let mut mc_opt = app_context.motor_controller.lock().unwrap();
            if let Some(mc) = mc_opt.as_mut() {
                let ver = mc.get_config_version();
                if ver != last_saved_version && last_saved_version != 0 {
                    last_saved_version = ver;
                    Some(mc.get_config())
                } else {
                    if last_saved_version == 0 {
                        last_saved_version = ver;
                    }
                    None
                }
            } else {
                None
            }
        };
        if let Some(config) = to_save {
            log::info!("Saving updated motor config to NVS via embassy task");
            if let Err(e) = app_context.storage_manager.lock().unwrap().set_motor_config(&config) {
                log::error!("Failed to save motor config to NVS: {}", e);
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(mut wifi: AsyncWifi<EspWifi<'static>>, app_context: AppContext) {
    if let Err(e) = connect_wifi(&mut wifi, app_context.storage_manager.clone()).await {
        log::error!("Failed to connect to wifi: {}", e);
    }

    // Run the HTTP server on a dedicated thread with a heap-allocated stack.
    // The edge-http server future is ~29KB and its poll chain is deep, far too
    // big for the main task stack shared with the Embassy executor. See the
    // comment in http_api::run_server for why the future itself is boxed
    // rather than kept on this stack.
    let builder = std::thread::Builder::new()
        .name("http_server".to_string())
        .stack_size(65536);
    builder
        .spawn(move || loop {
            if let Err(e) =
                esp_idf_svc::hal::task::block_on(http_api::run_server(app_context.clone()))
            {
                log::error!("HTTP server error: {}", e);
            }
            std::thread::sleep(time::Duration::from_millis(1000));
        })
        .unwrap();

    // Keep wifi alive for the lifetime of the program.
    loop {
        Timer::after_millis(60_000).await;
    }
}

async fn connect_wifi(
    wifi: &mut AsyncWifi<EspWifi<'static>>,
    storage_manager: Arc<Mutex<Box<storage::StorageManager>>>,
) -> anyhow::Result<()> {
    let (opt_ssid, opt_password) = {
        let storage_manager = storage_manager.lock().unwrap();
        (storage_manager.get_ssid(), storage_manager.get_password())
    };
    if let (Ok(saved_ssid), Ok(saved_password)) = (opt_ssid, opt_password) {
        if saved_ssid.is_empty() {
            log::info!("SSID is empty. Please set it via UART command: set-wifi-ssid <your_ssid> and set-wifi-password <your_password>");
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

            wifi.start().await?;
            log::info!(
                "WiFi connecting, SSID: {}, Password: {}",
                saved_ssid,
                saved_password
            );
            wifi.connect().await?;
            wifi.wait_netif_up().await?;
            
            let net_conf = storage_manager.lock().unwrap().get_network_configuration().unwrap_or_default();
            if let Ok(ip_info) = wifi.wifi().sta_netif().get_ip_info() {
                log::info!("WiFi connected. IP: {}, mDNS: http://{}.local", ip_info.ip, net_conf.hostname);
            } else {
                log::info!("WiFi connected. mDNS: http://{}.local", net_conf.hostname);
            }

            match esp_idf_svc::mdns::EspMdns::take() {
                Ok(mut mdns) => {
                    if let Err(e) = mdns.set_hostname(&net_conf.hostname) {
                        log::error!("Failed to set mDNS hostname: {}", e);
                    }
                    if let Err(e) = mdns.set_instance_name("OSSM Sex Machine") {
                        log::error!("Failed to set mDNS instance name: {}", e);
                    }
                    if let Err(e) = mdns.add_service(None, "_http", "_tcp", 80, &[("path", "/")]) {
                        log::error!("Failed to add mDNS service: {}", e);
                    }
                    log::info!("mDNS responder initialized for hostname: {}.local", net_conf.hostname);
                    std::mem::forget(mdns);
                }
                Err(e) => {
                    log::error!("Failed to initialize mDNS: {}", e);
                }
            }
        }
    } else {
        log::info!("WiFi SSID or password not set. Please set them via UART commands:\r\nset-wifi-ssid <your_ssid>\r\nset-wifi-password <your_password>");
    }
    Ok(())
}

async fn run_motor(app_context: AppContext, uart_peripheral: UART1<'static>) -> anyhow::Result<()> {
    let uart: uart::AsyncUartDriver<uart::UartDriver> = {
        let pin_config = app_context.storage_manager.lock().unwrap().get_pin_configuration().unwrap_or_default();

        let config = uart::config::Config::default()
            .baudrate(Hertz(TARGET_BAUD_RATE))
            .mode(uart::config::Mode::RS485HalfDuplex);

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
                uart::AsyncUartDriver::new(
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
                    ..Default::default()
                };
                app_context.storage_manager.lock().unwrap().set_pin_configuration(&new_pin_config)?;
                log::info!("Saved new pin configuration to NVS.");

                let tx: AnyOutputPin = tx.unwrap().into();
                let rx: AnyInputPin = rx.unwrap().into();
                let rts: AnyOutputPin = rts.unwrap().into();

                uart::AsyncUartDriver::new(
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

    let pin_config = app_context.storage_manager.lock().unwrap().get_pin_configuration().unwrap_or_default();
    let modbus = ModbusRTUMaster::new(uart, Option::<AnyOutputPin>::None, 1, pin_config.modbus_timeout_ms);

    let mut motor = Modbus57AIM30Motor::new(modbus, pin_config.modbus_scan_delay_us);

    let mut modbus_ok = false;
    for init_attempt in 1..=2 {
        if init_attempt > 1 {
            log::info!("Retrying motor initialization (attempt {}/2)...", init_attempt);
            Timer::after_millis(1000).await;
        }
        match motor.enable_modbus_communication().await {
            Ok(()) => { modbus_ok = true; break; }
            Err(e) => {
                log::info!("Failed to enable modbus (attempt {}/2), trying to scan and configure: {}", init_attempt, e);
                let mut scan_result = Err(anyhow::anyhow!("scan not attempted"));
                for attempt in 1..=3 {
                    match motor.modbus_scan().await {
                        Ok(result) => { scan_result = Ok(result); break; }
                        Err(e) => {
                            log::warn!("Scan attempt {}/3 failed: {}", attempt, e);
                            scan_result = Err(e);
                        }
                    }
                }
                match scan_result {
                    Ok(motor_scan_result) => {
                        log::info!("Motor device found, baud rate: {}, device id: {}", motor_scan_result.baud_rate, motor_scan_result.device_id);
                        if motor_scan_result.baud_rate != TARGET_BAUD_RATE {
                            motor.modbus_set_baud_rate(TARGET_BAUD_RATE).await.map_err(|e| anyhow::anyhow!("Failed to set baud rate to {}: {:?}", TARGET_BAUD_RATE, e))?;
                            log::info!("Motor baud rate set to {}, please power cycle the motor.", TARGET_BAUD_RATE);
                        }
                        modbus_ok = true;
                        break;
                    }
                    Err(e) => {
                        log::error!("Motor init attempt {}/2: scan failed: {}", init_attempt, e);
                    }
                }
            }
        }
    }
    if !modbus_ok {
        anyhow::bail!("Failed to establish modbus communication after retries. Please check connection to the motor.");
    }
    motor.enable_modbus_communication().await.map_err(|e| anyhow::anyhow!("Failed to enable modbus communication: {:?}", e))?;

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

    // Home the motor and set its parameters (the async I/O half of initialization)
    motor.homing().await.map_err(|e| anyhow::anyhow!("Failed to home motor: {:?}", e))?;
    log::info!("Motor homed, pos_min: {}, pos_max: {}", motor.pos_min(), motor.pos_max());

    motor.set_max_power(0.6).await?;
    motor.set_acceleration(4000.0).await?;
    motor.set_position_ring_ratio(3000.0).await?;
    motor.set_speed_ring_ratio(3000.0).await?;

    let current_position = motor.read_position().await?;

    // Create the controller (pure state/math) and sync it to the motor position
    let mut motor_controller = MotorController::new(motor_config);
    motor_controller
        .sync_to_position(motor.pos_min(), motor.pos_max(), current_position)
        .map_err(|e| anyhow::anyhow!("Failed to sync motor controller: {:?}", e))?;

    log::info!("Motor initialized, starting motor loop");
    *app_context.motor_controller.lock().unwrap() = Some(Box::new(motor_controller));

    let mut _update_counter = 0;
    let mut last_update_counter_reset = time::Instant::now();

    loop {
        // Compute the next position under a briefly-held lock (< 2 us, no I/O inside)
        let target = {
            let mut motor_controller_lock = app_context.motor_controller.lock().unwrap();
            motor_controller_lock.as_mut().map(|controller| controller.compute_cycle())
        };

        // Perform the motor I/O outside the lock; awaiting here yields to the
        // HTTP server and other tasks.
        match target {
            Some((position, speed)) => {
                if let Err(e) = motor.write_position(position, speed).await {
                    log::error!("Failed to write motor position: {}", e);
                }
                if let Err(e) = motor.cycle().await {
                    log::error!("Failed to cycle: {}", e);
                }
            }
            None => {
                log::error!("Motor controller lost, stopping motor loop");
                break;
            }
        }

        _update_counter += 1;
        if last_update_counter_reset.elapsed() >= time::Duration::from_secs(5) {
            if let Some(ref mut mc_lock) = *app_context.motor_controller.lock().unwrap() {
                let st = &mc_lock.last_loop_stats;
                if st.ups > 0 {
                    log::info!(
                        "Motor loop stats (1s): UPS={}, dt(ms) min/avg/max/mdev = {:.2} / {:.2} / {:.2} / {:.2}",
                        st.ups, st.min_dt_ms, st.avg_dt_ms, st.max_dt_ms, st.mdev_dt_ms
                    );
                }
            }
            last_update_counter_reset = time::Instant::now();
            _update_counter = 0;
        }
    }
    Ok(())
}
