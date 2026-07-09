use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use esp32_nimble::{uuid128, BLEAdvertisementData, BLEDevice, NimbleProperties, BLECharacteristic};
use esp32_nimble::utilities::mutex::Mutex as NimbleMutex;
use crate::context::AppContext;
use crate::http_api::{PausedControl, RpcError, RpcResponse, SubscribeParams, WaypointsInput, WsMessage};
use crate::motion::{MotionCommand, MotorControllerConfig};
use crate::storage::{NetworkConfiguration, PinConfiguration};

enum BleJob {
    NotifyChar(Arc<NimbleMutex<BLECharacteristic>>, Vec<u8>),
    UpdateChar(Arc<NimbleMutex<BLECharacteristic>>, Vec<u8>),
}

pub fn run_ble_server(app_context: AppContext) -> anyhow::Result<()> {
    log::info!("Initializing BLE device...");
    let ble_device = BLEDevice::take();
    let server = ble_device.get_server();

    let (tx, rx) = mpsc::channel::<BleJob>();
    std::thread::Builder::new()
        .name("ble_worker".to_string())
        .stack_size(4096)
        .spawn(move || {
            while let Ok(job) = rx.recv() {
                std::thread::sleep(Duration::from_millis(15));
                match job {
                    BleJob::NotifyChar(handle, bytes) => {
                        handle.lock().set_value(&bytes);
                        handle.lock().notify();
                    }
                    BleJob::UpdateChar(handle, bytes) => {
                        handle.lock().set_value(&bytes);
                    }
                }
            }
        })?;

    // Create OSSM GATT Service
    let service = server.create_service(uuid128!("6e400001-b5a3-f393-e0a9-e50e24dcca9e"));

    // 1. Config Characteristic (Read / Write / Notify)
    let config_char = service.lock().create_characteristic(
        uuid128!("6e400002-b5a3-f393-e0a9-e50e24dcca9e"),
        NimbleProperties::READ | NimbleProperties::WRITE | NimbleProperties::NOTIFY,
    );

    // 2. State Characteristic (Read / Notify)
    let state_char = service.lock().create_characteristic(
        uuid128!("6e400003-b5a3-f393-e0a9-e50e24dcca9e"),
        NimbleProperties::READ | NimbleProperties::NOTIFY,
    );

    // 3. Paused Control Characteristic (Write)
    let paused_char = service.lock().create_characteristic(
        uuid128!("6e400004-b5a3-f393-e0a9-e50e24dcca9e"),
        NimbleProperties::WRITE,
    );

    // 4. Pin Config Characteristic (Read / Write)
    let pin_config_char = service.lock().create_characteristic(
        uuid128!("6e400005-b5a3-f393-e0a9-e50e24dcca9e"),
        NimbleProperties::READ | NimbleProperties::WRITE,
    );

    // 5. RPC Command Characteristic (Write / Notify)
    let rpc_char = service.lock().create_characteristic(
        uuid128!("6e400006-b5a3-f393-e0a9-e50e24dcca9e"),
        NimbleProperties::WRITE | NimbleProperties::NOTIFY,
    );

    // 6. Network Config Characteristic (Read / Write)
    let net_config_char = service.lock().create_characteristic(
        uuid128!("6e400007-b5a3-f393-e0a9-e50e24dcca9e"),
        NimbleProperties::READ | NimbleProperties::WRITE,
    );

    // Set initial values from NVS / MotorController
    {
        let sm = app_context.storage_manager.lock().unwrap();
        if let Ok(config) = sm.get_motor_config() {
            if let Ok(json) = serde_json::to_string(&config) {
                config_char.lock().set_value(json.as_bytes());
            }
        }
        if let Ok(pin_config) = sm.get_pin_configuration() {
            if let Ok(json) = serde_json::to_string(&pin_config) {
                pin_config_char.lock().set_value(json.as_bytes());
            }
        }
        if let Ok(net_config) = sm.get_network_configuration() {
            if let Ok(json) = serde_json::to_string(&net_config) {
                net_config_char.lock().set_value(json.as_bytes());
            }
        }
    }

    // Attach write callback for Config
    {
        let app_context_clone = app_context.clone();
        let config_char_clone = config_char.clone();
        let tx = tx.clone();
        config_char.lock().on_write(move |args| {
            let data = args.recv_data();
            if let Ok(config) = serde_json::from_slice::<MotorControllerConfig>(data) {
                log::info!("BLE write config: bpm={}", config.bpm);
                let applied = {
                    let mut mc_opt = app_context_clone.motor_controller.lock().unwrap();
                    if let Some(mc) = mc_opt.as_mut() {
                        mc.set_config(config.clone()).is_ok()
                    } else {
                        false
                    }
                };
                if applied {
                    if let Ok(json) = serde_json::to_string(&config) {
                        let _ = tx.send(BleJob::NotifyChar(config_char_clone.clone(), json.into_bytes()));
                    }
                }
            } else {
                log::error!("BLE failed to parse MotorControllerConfig");
            }
        });
    }

    // Attach write callback for Paused Control
    {
        let app_context_clone = app_context.clone();
        let config_char_clone = config_char.clone();
        let tx = tx.clone();
        paused_char.lock().on_write(move |args| {
            let data = args.recv_data();
            if let Ok(control) = serde_json::from_slice::<PausedControl>(data) {
                let json_opt = {
                    let mut mc_opt = app_context_clone.motor_controller.lock().unwrap();
                    if let Some(mc) = mc_opt.as_mut() {
                        let mut config = mc.get_config();
                        if let Some(paused) = control.paused {
                            config.paused = paused;
                        }
                        if let Some(position) = control.position {
                            config.paused_position = position.clamp(0.0, 1.0);
                        }
                        if let Some(adjust) = control.adjust {
                            config.paused_position = (config.paused_position + adjust).clamp(0.0, 1.0);
                        }
                        let _ = mc.set_config(config.clone());
                        serde_json::to_string(&config).ok()
                    } else {
                        None
                    }
                };
                if let Some(json) = json_opt {
                    let _ = tx.send(BleJob::NotifyChar(config_char_clone.clone(), json.into_bytes()));
                }
            } else {
                log::error!("BLE failed to parse PausedControl");
            }
        });
    }

    // Attach write callback for Pin Config
    {
        let app_context_clone = app_context.clone();
        let pin_config_char_clone = pin_config_char.clone();
        let tx = tx.clone();
        pin_config_char.lock().on_write(move |args| {
            let data = args.recv_data();
            if let Ok(config) = serde_json::from_slice::<PinConfiguration>(data) {
                if config.modbus_timeout_ms <= 1000
                    && config.modbus_scan_delay_us <= 200000
                    && app_context_clone.storage_manager.lock().unwrap().set_pin_configuration(&config).is_ok()
                {
                    if let Ok(json) = serde_json::to_string(&config) {
                        let _ = tx.send(BleJob::UpdateChar(pin_config_char_clone.clone(), json.into_bytes()));
                    }
                }
            } else {
                log::error!("BLE failed to parse PinConfiguration");
            }
        });
    }

    // Attach write callback for Network Config
    {
        let app_context_clone = app_context.clone();
        let net_config_char_clone = net_config_char.clone();
        let tx = tx.clone();
        net_config_char.lock().on_write(move |args| {
            let data = args.recv_data();
            if let Ok(config) = serde_json::from_slice::<NetworkConfiguration>(data) {
                if app_context_clone.storage_manager.lock().unwrap().set_network_configuration(&config).is_ok() {
                    if let Ok(json) = serde_json::to_string(&config) {
                        let _ = tx.send(BleJob::UpdateChar(net_config_char_clone.clone(), json.into_bytes()));
                    }
                }
            } else {
                log::error!("BLE failed to parse NetworkConfiguration");
            }
        });
    }

    // State notification management
    let subscribed = Arc::new(Mutex::new(false));
    let interval_ms = Arc::new(Mutex::new(500u64));

    // Attach write callback for RPC Commands
    {
        let app_context_clone = app_context.clone();
        let rpc_char_clone = rpc_char.clone();
        let subscribed_state = subscribed.clone();
        let push_interval = interval_ms.clone();
        let tx = tx.clone();
        rpc_char.lock().on_write(move |args| {
            let data = args.recv_data();
            if let Ok(request) = serde_json::from_slice::<WsMessage>(data) {
                let mut response_bytes: Option<Vec<u8>> = None;
                match request.cmd.as_str() {
                    "ping" => {
                        let resp = RpcResponse {
                            jsonrpc: "2.0",
                            id: request.id.clone().unwrap_or(serde_json::Value::Null),
                            result: Some(&"pong"),
                            error: None,
                        };
                        response_bytes = serde_json::to_vec(&resp).ok();
                    }
                    "status" => {
                        let status = {
                            let mc_opt = app_context_clone.motor_controller.lock().unwrap();
                            mc_opt.as_ref().map(|mc| mc.get_stream_status())
                        };
                        if let Some(st) = status {
                            let resp = RpcResponse {
                                jsonrpc: "2.0",
                                id: request.id.clone().unwrap_or(serde_json::Value::Null),
                                result: Some(&st),
                                error: None,
                            };
                            response_bytes = serde_json::to_vec(&resp).ok();
                        } else {
                            let err = RpcError { code: -32603, message: "Motor controller not initialized" };
                            let resp: RpcResponse<'_, ()> = RpcResponse {
                                jsonrpc: "2.0",
                                id: request.id.clone().unwrap_or(serde_json::Value::Null),
                                result: None,
                                error: Some(err),
                            };
                            response_bytes = serde_json::to_vec(&resp).ok();
                        }
                    }
                    "get_state" | "get-state" => {
                        let state = {
                            let mut mc_opt = app_context_clone.motor_controller.lock().unwrap();
                            mc_opt.as_mut().map(|mc| mc.get_current_state())
                        };
                        if let Some(state) = state {
                            let resp = RpcResponse {
                                jsonrpc: "2.0",
                                id: request.id.clone().unwrap_or(serde_json::Value::Null),
                                result: Some(&state),
                                error: None,
                            };
                            response_bytes = serde_json::to_vec(&resp).ok();
                        }
                    }
                    "set_config" | "set-config" => {
                        if let Ok(config) = request.parse_params::<MotorControllerConfig>() {
                            let applied = {
                                let mut mc_opt = app_context_clone.motor_controller.lock().unwrap();
                                if let Some(mc) = mc_opt.as_mut() {
                                    mc.set_config(config.clone()).is_ok()
                                } else {
                                    false
                                }
                            };
                            if applied {
                                let resp = RpcResponse {
                                    jsonrpc: "2.0",
                                    id: request.id.clone().unwrap_or(serde_json::Value::Null),
                                    result: Some(&config),
                                    error: None,
                                };
                                response_bytes = serde_json::to_vec(&resp).ok();
                            }
                        }
                    }
                    "subscribe_state" | "subscribe-state" => {
                        let params = request.parse_params::<SubscribeParams>().unwrap_or_default();
                        let interval = params.interval_ms.unwrap_or(33).clamp(20, 60000);
                        *push_interval.lock().unwrap() = interval;
                        *subscribed_state.lock().unwrap() = true;
                        let resp = RpcResponse {
                            jsonrpc: "2.0",
                            id: request.id.clone().unwrap_or(serde_json::Value::Null),
                            result: Some(&"subscribed"),
                            error: None,
                        };
                        response_bytes = serde_json::to_vec(&resp).ok();
                    }
                    "unsubscribe_state" | "unsubscribe-state" => {
                        *subscribed_state.lock().unwrap() = false;
                        let resp = RpcResponse {
                            jsonrpc: "2.0",
                            id: request.id.clone().unwrap_or(serde_json::Value::Null),
                            result: Some(&"unsubscribed"),
                            error: None,
                        };
                        response_bytes = serde_json::to_vec(&resp).ok();
                    }
                    "get_network_config" | "get-network-config" => {
                        let config = app_context_clone.storage_manager.lock().unwrap().get_network_configuration().unwrap_or_default();
                        let resp = RpcResponse {
                            jsonrpc: "2.0",
                            id: request.id.clone().unwrap_or(serde_json::Value::Null),
                            result: Some(&config),
                            error: None,
                        };
                        response_bytes = serde_json::to_vec(&resp).ok();
                    }
                    "set_network_config" | "set-network-config" => {
                        if let Ok(config) = request.parse_params::<NetworkConfiguration>() {
                            if app_context_clone.storage_manager.lock().unwrap().set_network_configuration(&config).is_ok() {
                                let resp = RpcResponse {
                                    jsonrpc: "2.0",
                                    id: request.id.clone().unwrap_or(serde_json::Value::Null),
                                    result: Some(&config),
                                    error: None,
                                };
                                response_bytes = serde_json::to_vec(&resp).ok();
                            }
                        }
                    }
                    "reset_timestamp" | "reset-timestamp" => {
                        let command_tx = {
                            let mut mc_opt = app_context_clone.motor_controller.lock().unwrap();
                            mc_opt.as_mut().map(|mc| mc.get_command_sender())
                        };
                        if let Some(command_tx) = command_tx {
                            let _ = command_tx.lock().unwrap().enqueue(MotionCommand::ResetTimestamp);
                            let resp = RpcResponse {
                                jsonrpc: "2.0",
                                id: request.id.clone().unwrap_or(serde_json::Value::Null),
                                result: Some(&"ok"),
                                error: None,
                            };
                            response_bytes = serde_json::to_vec(&resp).ok();
                        }
                    }
                    "append-waypoints" | "append_waypoints" => {
                        if let Ok(input) = request.parse_params::<WaypointsInput>() {
                            let (waypoints, _) = input.into_parts();
                            let command_tx = {
                                let mut mc_opt = app_context_clone.motor_controller.lock().unwrap();
                                mc_opt.as_mut().map(|mc| mc.get_command_sender())
                            };
                            if let Some(command_tx) = command_tx {
                                let _ = command_tx.lock().unwrap().enqueue(MotionCommand::AppendWaypoints(waypoints));
                                let resp = RpcResponse {
                                    jsonrpc: "2.0",
                                    id: request.id.clone().unwrap_or(serde_json::Value::Null),
                                    result: Some(&"ok"),
                                    error: None,
                                };
                                response_bytes = serde_json::to_vec(&resp).ok();
                            }
                        }
                    }
                    "set-waypoints" | "set_waypoints" => {
                        if let Ok(input) = request.parse_params::<WaypointsInput>() {
                            let (waypoints, reset_timestamp) = input.into_parts();
                            let command_tx = {
                                let mut mc_opt = app_context_clone.motor_controller.lock().unwrap();
                                mc_opt.as_mut().map(|mc| mc.get_command_sender())
                            };
                            if let Some(command_tx) = command_tx {
                                let _ = command_tx.lock().unwrap().enqueue(MotionCommand::SetWaypoints { waypoints, reset_timestamp });
                                let resp = RpcResponse {
                                    jsonrpc: "2.0",
                                    id: request.id.clone().unwrap_or(serde_json::Value::Null),
                                    result: Some(&"ok"),
                                    error: None,
                                };
                                response_bytes = serde_json::to_vec(&resp).ok();
                            }
                        }
                    }
                    "restart" => {
                        let resp = RpcResponse {
                            jsonrpc: "2.0",
                            id: request.id.clone().unwrap_or(serde_json::Value::Null),
                            result: Some(&"restarting"),
                            error: None,
                        };
                        response_bytes = serde_json::to_vec(&resp).ok();
                        std::thread::spawn(|| {
                            std::thread::sleep(Duration::from_millis(100));
                            esp_idf_svc::hal::reset::restart();
                        });
                    }
                    _ => {
                        let err = RpcError { code: -32601, message: "Method not found" };
                        let resp: RpcResponse<'_, ()> = RpcResponse {
                            jsonrpc: "2.0",
                            id: request.id.clone().unwrap_or(serde_json::Value::Null),
                            result: None,
                            error: Some(err),
                        };
                        response_bytes = serde_json::to_vec(&resp).ok();
                    }
                }
                if let Some(bytes) = response_bytes {
                    let _ = tx.send(BleJob::NotifyChar(rpc_char_clone.clone(), bytes));
                }
            }
        });
    }

    log::info!("Starting BLE advertising...");
    let ble_advertising = ble_device.get_advertising();
    let mut ad_data = BLEAdvertisementData::new();
    ad_data.name("OSSM");
    ad_data.add_service_uuid(uuid128!("6e400001-b5a3-f393-e0a9-e50e24dcca9e"));
    ble_advertising.lock().set_data(&mut ad_data)?;
    ble_advertising.lock().start()?;
    log::info!("BLE advertising started successfully.");

    // Periodic telemetry loop
    let mut last_notify = std::time::Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(50));
        let (state_opt, config_opt) = {
            let mut mc_opt = app_context.motor_controller.lock().unwrap();
            if let Some(mc) = mc_opt.as_mut() {
                (Some(mc.get_current_state()), Some(mc.get_config()))
            } else {
                (None, None)
            }
        };
        let state_json = state_opt.and_then(|s| serde_json::to_string(&s).ok());
        let config_json = config_opt.and_then(|c| serde_json::to_string(&c).ok());

        if let Some(config_str) = config_json {
            config_char.lock().set_value(config_str.as_bytes());
        }

        if let Some(state_str) = state_json {
            let is_sub = *subscribed.lock().unwrap();
            let interval = *interval_ms.lock().unwrap();
            if is_sub && last_notify.elapsed() >= Duration::from_millis(interval) {
                last_notify = std::time::Instant::now();
                let _ = tx.send(BleJob::NotifyChar(state_char.clone(), state_str.into_bytes()));
            } else {
                state_char.lock().set_value(state_str.as_bytes());
            }
        }
    }
}
