#![allow(unused_imports)]
#![allow(clippy::needless_borrows_for_generic_args)]

use alloc::string::String;

use serde::Serialize;

use bt_hci::controller::ExternalController;
use embassy_futures::select::{select, Either};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Instant, Timer};
use esp_radio::ble::controller::BleConnector;
use portable_atomic::{AtomicBool, AtomicU32, Ordering};
use trouble_host::prelude::*;

use crate::context::AppContext;
use crate::http_api::{PausedControl, RpcRequest};
use crate::motion::MotorControllerConfig;
use crate::rpc::{self, RpcAction};
use crate::storage::{NetworkConfiguration, PinConfiguration};

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 3;

/// Max payload bytes per BLE notification chunk (excluding the 2-byte header).
/// With a standard negotiated MTU of 247, ATT notification carries MTU-3 = 244 bytes.
/// We use 240 payload + 2 header = 242 bytes per notification, safely below 244.
const BLE_CHUNK_DATA: usize = 240;

#[gatt_service(uuid = "6e400001-b5a3-f393-e0a9-e50e24dcca9e")]
struct OssmService {
    #[characteristic(uuid = "6e400002-b5a3-f393-e0a9-e50e24dcca9e", read, write, notify)]
    config: heapless::Vec<u8, 512>,

    // Compact telemetry only (~150 B). Keep attribute <= ATT MTU so BlueZ
    // Read Request succeeds without Read Blob; avoids controller Unlikely Error.
    #[characteristic(uuid = "6e400003-b5a3-f393-e0a9-e50e24dcca9e", read, notify)]
    state: heapless::Vec<u8, 256>,

    #[characteristic(uuid = "6e400004-b5a3-f393-e0a9-e50e24dcca9e", write)]
    paused: heapless::Vec<u8, 128>,

    #[characteristic(uuid = "6e400005-b5a3-f393-e0a9-e50e24dcca9e", read, write)]
    pin_config: heapless::Vec<u8, 256>,

    #[characteristic(uuid = "6e400006-b5a3-f393-e0a9-e50e24dcca9e", write, notify)]
    rpc: heapless::Vec<u8, 512>,

    #[characteristic(uuid = "6e400007-b5a3-f393-e0a9-e50e24dcca9e", read, write)]
    net_config: heapless::Vec<u8, 256>,
}

#[gatt_server(connections_max = CONNECTIONS_MAX, mutex_type = CriticalSectionRawMutex, attribute_table_size = 64)]
struct OssmGattServer {
    ossm: OssmService,
}

fn to_vec<const N: usize>(data: &[u8]) -> heapless::Vec<u8, N> {
    let mut out = heapless::Vec::new();
    let _ = out.extend_from_slice(&data[..data.len().min(N)]);
    out
}

fn round_mult(v: f32, mult: f32) -> f32 {
    (v * mult) as i32 as f32 / mult
}

#[derive(Serialize)]
struct CompactConfig {
    bpm: f32,
    depth: f32,
    paused: bool,
    paused_position: f32,
}

#[derive(Serialize)]
struct CompactState {
    config: CompactConfig,
    y: f32,
    shaped_y: f32,
    position: f32,
    speed: f32,
    ups: u32,
    motor_connected: bool,
}

/// Build a compact JSON string of the state for GATT attribute reads.
/// Excludes large arrays (update_history, position_history) and redundant
/// config fields to fit within the 512-byte GATT attribute value limit.
fn format_compact_state(state: &crate::motion::StateResponse, out: &mut [u8]) -> Result<usize, ()> {
    // Keep a Web-Bluetooth-usable subset: include normalized y/shaped_y so the UI
    // can pause in-place and render the diagram without a full get-state RPC.
    let compact = CompactState {
        config: CompactConfig {
            bpm: round_mult(state.config.bpm, 10.0),
            depth: round_mult(state.config.depth, 100.0),
            paused: state.config.paused,
            paused_position: round_mult(state.config.paused_position, 1000.0),
        },
        y: round_mult(state.y, 1000.0),
        shaped_y: round_mult(state.shaped_y, 1000.0),
        position: round_mult(state.position, 1000.0),
        speed: round_mult(state.speed, 100.0),
        ups: state.ups,
        motor_connected: state.motor_connected,
    };
    serde_json_core::to_slice(&compact, out).map_err(|_| ())
}

/// Update CHAR_STATE without awaiting. GattEvent handlers must never `.await`
/// before `accept()`/`send()` — yielding with an outstanding ATT request hangs
/// the controller (BlueZ sees Unlikely Error) and starves WiFi on the same executor.
fn set_compact_state(server: &OssmGattServer<'_>, state: &crate::motion::StateResponse) {
    let mut buf = [0u8; 256];
    if let Ok(len) = format_compact_state(state, &mut buf) {
        let _ = server.set(&server.ossm.state, &to_vec::<256>(&buf[..len]));
    }
}

/// Send a chunked notification on CHAR_STATE.
/// Each notification: [index: u8, total: u8, ...payload_fragment...]
async fn chunked_notify_state(
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    server: &OssmGattServer<'_>,
    data: &[u8],
) -> Result<(), trouble_host::Error> {
    let total = data.len().div_ceil(BLE_CHUNK_DATA).max(1) as u8;
    if data.is_empty() {
        let mut buf: heapless::Vec<u8, 256> = heapless::Vec::new();
        let _ = buf.push(0);
        let _ = buf.push(1);
        return server.ossm.state.notify(conn, &buf).await;
    }
    for (i, chunk) in data.chunks(BLE_CHUNK_DATA).enumerate() {
        let mut buf: heapless::Vec<u8, 256> = heapless::Vec::new();
        let _ = buf.push(i as u8);
        let _ = buf.push(total);
        let _ = buf.extend_from_slice(chunk);
        server.ossm.state.notify(conn, &buf).await?;
    }
    Ok(())
}

/// Send a chunked notification on CHAR_RPC.
async fn chunked_notify_rpc(
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    server: &OssmGattServer<'_>,
    data: &[u8],
) -> Result<(), trouble_host::Error> {
    let total = data.len().div_ceil(BLE_CHUNK_DATA).max(1) as u8;
    if data.is_empty() {
        let mut buf: heapless::Vec<u8, 512> = heapless::Vec::new();
        let _ = buf.push(0);
        let _ = buf.push(1);
        return server.ossm.rpc.notify(conn, &buf).await;
    }
    for (i, chunk) in data.chunks(BLE_CHUNK_DATA).enumerate() {
        let mut buf: heapless::Vec<u8, 512> = heapless::Vec::new();
        let _ = buf.push(i as u8);
        let _ = buf.push(total);
        let _ = buf.extend_from_slice(chunk);
        server.ossm.rpc.notify(conn, &buf).await?;
    }
    Ok(())
}

pub async fn run_ble_server(
    mut bt_peripheral: esp_hal::peripherals::BT<'static>,
    app_context: AppContext,
) {
    loop {
        log::info!("Initializing BLE stack & controller...");

        // Default controller task stack is 4 KiB — under WiFi coexistence +
        // connection setup the LLL path overflows it (Instruction access fault
        // at mepc=0x80000100). Give the controller more room.
        // HCI/ACL buffer knobs exist on NimBLE adapters (C6 etc.); BTDM (S3/C3)
        // uses a different Config without those fields.
        #[cfg(feature = "esp32c6")]
        let ble_config = esp_radio::ble::Config::default()
            .with_task_stack_size(10 * 1024)
            .with_max_connections(1)
            .with_hci_high_buffer_count(40)
            .with_acl_buf_count(32);
        #[cfg(feature = "esp32s3")]
        let ble_config = esp_radio::ble::Config::default()
            .with_task_stack_size(10 * 1024)
            .with_max_connections(1);
        let connector = match BleConnector::new(bt_peripheral, ble_config) {
            Ok(c) => c,
            Err(e) => {
                log::error!("BleConnector::new failed: {:?}, retrying in 2s...", e);
                Timer::after(Duration::from_secs(2)).await;
                bt_peripheral = unsafe { esp_hal::peripherals::BT::steal() };
                continue;
            }
        };
        let controller: ExternalController<_, 32> = ExternalController::new(connector);

        let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
            HostResources::new();
        let stack = trouble_host::new(controller, &mut resources)
            .set_random_address(Address::random([0x41, 0x42, 0x43, 0x44, 0x45, 0xC6]));
        let Host {
            mut peripheral,
            mut runner,
            ..
        } = stack.build();

        let server =
            match OssmGattServer::new_with_config(GapConfig::Peripheral(PeripheralConfig {
                name: "OSSM",
                appearance: &appearance::UNKNOWN,
            })) {
                Ok(s) => s,
                Err(e) => {
                    log::error!("GATT server init failed: {:?}, retrying in 2s...", e);
                    Timer::after(Duration::from_secs(2)).await;
                    bt_peripheral = unsafe { esp_hal::peripherals::BT::steal() };
                    continue;
                }
            };

        let config = app_context.storage.motor_config();
        let pin_config = app_context.storage.pin();
        let net_config = app_context.storage.net();
        let _ = crate::buffers::serialize_to_scratchpad(&config, |json| {
            let _ = server.set(&server.ossm.config, &to_vec::<512>(json.as_bytes()));
        })
        .await;
        let _ = crate::buffers::serialize_to_scratchpad(&pin_config, |json| {
            let _ = server.set(&server.ossm.pin_config, &to_vec::<256>(json.as_bytes()));
        })
        .await;
        let _ = crate::buffers::serialize_to_scratchpad(&net_config, |json| {
            let _ = server.set(&server.ossm.net_config, &to_vec::<256>(json.as_bytes()));
        })
        .await;

        let _ = embassy_futures::select::select(
            async {
                if let Err(e) = runner.run().await {
                    log::warn!(
                        "BLE runner error: {:?}, restarting BLE stack (no chip reset)...",
                        e
                    );
                } else {
                    log::warn!("BLE runner exited cleanly, restarting BLE stack...");
                }
            },
            serve_gatt(&server, &mut peripheral, app_context, &stack),
        )
        .await;

        // Recover in-process: WiFi/HTTP must keep running. Full chip reset was
        // killing the TCP stack and causing "connection refused" after BLE tests.
        log::warn!("BLE stack session ended; cooldown before re-init...");
        Timer::after(Duration::from_millis(500)).await;
        bt_peripheral = unsafe { esp_hal::peripherals::BT::steal() };
    }
}

async fn serve_gatt<C: Controller>(
    server: &OssmGattServer<'_>,
    peripheral: &mut Peripheral<'_, C, DefaultPacketPool>,
    app_context: AppContext,
    stack: &Stack<'_, C, DefaultPacketPool>,
) {
    // Put BOTH CompleteLocalName and the 128-bit service UUID in the primary
    // advertising PDU so Web Bluetooth name filters and services filters match
    // without depending on an active-scan response (Flags+Name+UUID128 = 27B).
    let service_uuid: [u8; 16] = [
        0x9e, 0xca, 0xdc, 0x24, 0x0e, 0xe5, 0xa9, 0xe0, 0x93, 0xf3, 0xa3, 0xb5, 0x01, 0x00, 0x40,
        0x6e,
    ];
    let mut adv_data = [0u8; 31];
    let adv_len = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteLocalName(b"OSSM"),
            AdStructure::ServiceUuids128(&[service_uuid]),
        ],
        &mut adv_data,
    )
    .unwrap_or(0);

    // Scan response keeps the service UUID for scanners that prefer scan RSPs.
    let mut scan_data = [0u8; 31];
    let scan_len = AdStructure::encode_slice(
        &[AdStructure::ServiceUuids128(&[service_uuid])],
        &mut scan_data,
    )
    .unwrap_or(0);

    let adv_params = AdvertisementParameters {
        // Relaxed intervals reduce WiFi coexistence contention (AGENTS.md).
        interval_min: Duration::from_millis(250),
        interval_max: Duration::from_millis(500),
        ..Default::default()
    };

    loop {
        log::info!("BLE advertising...");
        let acceptor = match peripheral
            .advertise(
                &adv_params,
                Advertisement::ConnectableScannableUndirected {
                    adv_data: &adv_data[..adv_len],
                    scan_data: &scan_data[..scan_len],
                },
            )
            .await
        {
            Ok(a) => a,
            Err(e) => {
                log::error!("BLE advertise failed: {:?}", e);
                Timer::after(Duration::from_millis(500)).await;
                continue;
            }
        };

        let conn = match acceptor.accept().await {
            Ok(c) => c,
            Err(e) => {
                log::error!("BLE accept failed: {:?}", e);
                // Cool down so the host can refresh its BlueZ device cache /
                // advertising state before we race another accept.
                Timer::after(Duration::from_millis(350)).await;
                continue;
            }
        };

        let conn = match conn.with_attribute_server(&server.server) {
            Ok(c) => c,
            Err(e) => {
                log::error!("BLE GATT attach failed: {:?}", e);
                Timer::after(Duration::from_millis(350)).await;
                continue;
            }
        };

        log::info!("BLE connected");

        {
            let state = app_context.load_snapshot();
            set_compact_state(server, state.as_ref());
            // Prefer scratchpad (heap-free); fall back to truncating stack encode.
            let _ = crate::buffers::try_with_scratchpad(|buf| {
                if let Ok(len) = serde_json_core::to_slice(&state.config, buf) {
                    let _ = server.set(&server.ossm.config, &to_vec::<512>(&buf[..len]));
                }
            });
            let pin_config = app_context.storage.pin();
            let net_config = app_context.storage.net();
            if let Ok(json) = serde_json_core::to_string::<_, 512>(&pin_config) {
                let _ = server.set(&server.ossm.pin_config, &to_vec::<256>(json.as_bytes()));
            }
            if let Ok(json) = serde_json_core::to_string::<_, 512>(&net_config) {
                let _ = server.set(&server.ossm.net_config, &to_vec::<256>(json.as_bytes()));
            }
        }

        let subscribed = AtomicBool::new(false);
        let push_interval_ms = AtomicU32::new(100);

        select(
            handle_gatt_events(
                &conn,
                server,
                app_context,
                &subscribed,
                &push_interval_ms,
                stack,
            ),
            push_telemetry(&conn, server, app_context, &subscribed, &push_interval_ms),
        )
        .await;

        log::info!("BLE session ended, cooldown before next advertisement...");
        Timer::after(Duration::from_millis(500)).await;
    }
}

async fn handle_gatt_events<C: Controller>(
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    server: &OssmGattServer<'_>,
    app_context: AppContext,
    subscribed: &AtomicBool,
    push_interval_ms: &AtomicU32,
    stack: &Stack<'_, C, DefaultPacketPool>,
) {
    loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => {
                log::info!("BLE disconnected: {:?}", reason);
                break;
            }
            GattConnectionEvent::Gatt { event } => match event {
                GattEvent::Write(write) => {
                    let data = write.data().to_vec();
                    let handle = write.handle();

                    if let Ok(reply) = write.accept() {
                        reply.send().await;
                    }

                    if handle == server.ossm.config.handle {
                        if crate::modbus_relay::is_active() {
                            // Motion config writes are disabled in RTU relay mode.
                        } else if let Ok(config) =
                            serde_json::from_slice::<MotorControllerConfig>(&data)
                        {
                            if let Ok(applied) = app_context.try_enqueue_config(config).await {
                                let _ = crate::buffers::try_with_scratchpad(|buf| {
                                    if let Ok(len) = serde_json_core::to_slice(&applied, buf) {
                                        let _ = server
                                            .set(&server.ossm.config, &to_vec::<512>(&buf[..len]));
                                    }
                                });
                            }
                        }
                    } else if handle == server.ossm.paused.handle {
                        if crate::modbus_relay::is_active() {
                            // Pause control disabled in RTU relay mode.
                        } else if let Ok(control) = serde_json::from_slice::<PausedControl>(&data) {
                            let mut config = app_context.load_snapshot().config.clone();
                            if let Some(paused) = control.paused {
                                config.paused = paused;
                            }
                            // Accept both "position" (canonical) and "paused_position"
                            // (config field name) for client compatibility.
                            let park = control.position.or(control.paused_position);
                            if let Some(position) = park {
                                config.paused_position = position.clamp(0.0, 1.0);
                            }
                            if let Some(adjust) = control.adjust {
                                config.paused_position =
                                    (config.paused_position + adjust).clamp(0.0, 1.0);
                            }
                            if let Ok(applied) = app_context.try_enqueue_config(config).await {
                                let _ = crate::buffers::try_with_scratchpad(|buf| {
                                    if let Ok(len) = serde_json_core::to_slice(&applied, buf) {
                                        let _ = server
                                            .set(&server.ossm.config, &to_vec::<512>(&buf[..len]));
                                    }
                                });
                            }
                        }
                    } else if handle == server.ossm.pin_config.handle {
                        if let Ok(config) = serde_json::from_slice::<PinConfiguration>(&data) {
                            if config.modbus_timeout_ms <= 1000
                                && config.modbus_scan_delay_us <= 200_000
                                && config.modbus_inter_frame_delay_us <= 200_000
                                && crate::storage::parse_operating_mode(&config.operating_mode)
                                    .is_some()
                            {
                                app_context.storage.set_pin(config.clone());
                                if let Ok(json) = serde_json_core::to_string::<_, 512>(&config) {
                                    let _ = server.set(
                                        &server.ossm.pin_config,
                                        &to_vec::<256>(json.as_bytes()),
                                    );
                                }
                            }
                        }
                    } else if handle == server.ossm.net_config.handle {
                        if let Ok(config) = serde_json::from_slice::<NetworkConfiguration>(&data) {
                            app_context.storage.set_net(config.clone());
                            if let Ok(json) = serde_json_core::to_string::<_, 512>(&config) {
                                let _ = server
                                    .set(&server.ossm.net_config, &to_vec::<256>(json.as_bytes()));
                            }
                        }
                    } else if handle == server.ossm.rpc.handle {
                        if let Ok(request) = serde_json::from_slice::<RpcRequest>(&data) {
                            let id = request.id.clone().unwrap_or(serde_json::Value::Null);
                            match rpc::dispatch_rpc(&request, app_context).await {
                                RpcAction::Respond(json) => {
                                    let _ = chunked_notify_rpc(conn, server, json.as_bytes()).await;
                                }
                                RpcAction::Subscribe { interval_ms } => {
                                    push_interval_ms.store(
                                        interval_ms.min(u32::MAX as u64) as u32,
                                        Ordering::Relaxed,
                                    );
                                    subscribed.store(true, Ordering::Relaxed);
                                    log::info!("BLE subscribe-state: interval={}ms", interval_ms);
                                    let ack = rpc::subscribe_ack(id, interval_ms);
                                    let _ = chunked_notify_rpc(conn, server, ack.as_bytes()).await;
                                }
                                RpcAction::Unsubscribe => {
                                    subscribed.store(false, Ordering::Relaxed);
                                    let ack = rpc::unsubscribe_ack(id);
                                    let _ = chunked_notify_rpc(conn, server, ack.as_bytes()).await;
                                }
                                RpcAction::Restart => {
                                    let ack = rpc::restart_ack(id);
                                    let _ = chunked_notify_rpc(conn, server, ack.as_bytes()).await;
                                    Timer::after(Duration::from_millis(100)).await;
                                    esp_hal::system::software_reset();
                                }
                            }
                        }
                    }
                }
                GattEvent::Read(read) => {
                    // Reply with cached attribute values only. Mutating the
                    // attribute table mid-Read (especially during BlueZ service
                    // discovery) races the controller and has been observed to
                    // kill WiFi. Values are refreshed on connect/write/notify.
                    if let Ok(reply) = read.accept() {
                        reply.send().await;
                    }
                }
                _ => {}
            },
            GattConnectionEvent::RequestConnectionParams(req) => {
                log::info!("BLE connection params requested: {:?}", req.params());
                if let Err(e) = req.accept(None, stack).await {
                    log::warn!("BLE accept connection params failed: {:?}", e);
                }
            }
            GattConnectionEvent::PhyUpdated { tx_phy, rx_phy } => {
                log::info!("BLE PHY updated: tx={:?}, rx={:?}", tx_phy, rx_phy);
            }
            GattConnectionEvent::ConnectionParamsUpdated {
                conn_interval,
                peripheral_latency,
                supervision_timeout,
            } => {
                log::info!(
                    "BLE conn params updated: interval={:?}, latency={}, timeout={:?}",
                    conn_interval,
                    peripheral_latency,
                    supervision_timeout
                );
            }
            _ => {}
        }
    }
}

async fn push_telemetry(
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    server: &OssmGattServer<'_>,
    app_context: AppContext,
    subscribed: &AtomicBool,
    interval_ms: &AtomicU32,
) {
    let mut consecutive_errors = 0u32;
    loop {
        if !subscribed.load(Ordering::Relaxed) {
            Timer::after(Duration::from_millis(100)).await;
            continue;
        }

        let interval = interval_ms.load(Ordering::Relaxed).clamp(25, 5000) as u64;
        Timer::after(Duration::from_millis(interval)).await;

        if !subscribed.load(Ordering::Relaxed) {
            continue;
        }

        let state = app_context.load_snapshot();

        let mut lease = crate::buffers::NET_BUFFER_POOL.acquire().await;
        if let Ok(len) = format_compact_state(state.as_ref(), &mut *lease) {
            match chunked_notify_state(conn, server, &lease[..len]).await {
                Ok(_) => {
                    consecutive_errors = 0;
                }
                Err(trouble_host::Error::Disconnected)
                | Err(trouble_host::Error::ChannelClosed) => {
                    log::info!("BLE client disconnected during telemetry push");
                    break;
                }
                Err(e) => {
                    consecutive_errors += 1;
                    if consecutive_errors % 10 == 1 {
                        log::warn!(
                            "BLE state notify failed ({}/10): {:?}",
                            consecutive_errors,
                            e
                        );
                    }
                    Timer::after(Duration::from_millis(15)).await;
                }
            }
        }
    }
}
