# 06: BLE GATT Server

## Scope

Replace `esp32-nimble` with `trouble-host` + `bt-hci` + `esp-radio` `BleConnector`. Redefine GATT service using trouble-host proc macros. Eliminate the `ble_worker` thread — trouble-host is fully async.

## Files to Modify

- `src/ble_api.rs` (complete rewrite)

---

## BLE Initialization (correct API)

The plan previously used `esp_radio::ble::new()` which does not exist. The correct API is `BleConnector::new()` with the shared radio controller:

```rust
use bt_hci::controller::ExternalController;
use esp_radio::ble::controller::BleConnector;
use trouble_host::prelude::*;

pub async fn run_ble_server(
    radio: &'static esp_radio::Controller<'static>,
    bt_peripheral: esp_hal::peripherals::BT,
    app_context: AppContext,
) {
    log::info!("Initializing BLE...");

    let connector = BleConnector::new(radio, bt_peripheral, Default::default())
        .expect("BleConnector::new failed");
    let controller: ExternalController<_, 20> = ExternalController::new(connector);

    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let (stack, mut peripheral, _, runner) = trouble_host::new(controller, &mut resources)
        .set_random_address(Default::default())
        .build();

    let server = OssmGattServer::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: "OSSM",
        appearance: &appearance::unknown::UNKNOWN,
    }))
    .expect("GATT server init");

    // Set initial characteristic values from NVS ...

    embassy_futures::join::join(
        runner.run(),
        serve_gatt(&server, &mut peripheral, app_context),
    )
    .await;
}
```

`radio` is the same `&'static esp_radio::Controller` created by `esp_radio::init()` in `main()` (see [02-entry-point](02-entry-point.md), [04-wifi-networking](04-wifi-networking.md)).

---

## GATT Service Definition

```rust
#[gatt_service(uuid = "6e400001-b5a3-f393-e0a9-e50e24dcca9e")]
struct OssmService {
    #[characteristic(uuid = "6e400002-b5a3-f393-e0a9-e50e24dcca9e", read, write, notify)]
    config: [u8; 512],

    #[characteristic(uuid = "6e400003-b5a3-f393-e0a9-e50e24dcca9e", read, notify)]
    state: [u8; 1024],

    #[characteristic(uuid = "6e400004-b5a3-f393-e0a9-e50e24dcca9e", write)]
    paused: [u8; 128],

    #[characteristic(uuid = "6e400005-b5a3-f393-e0a9-e50e24dcca9e", read, write)]
    pin_config: [u8; 256],

    #[characteristic(uuid = "6e400006-b5a3-f393-e0a9-e50e24dcca9e", write, notify)]
    rpc: [u8; 512],

    #[characteristic(uuid = "6e400007-b5a3-f393-e0a9-e50e24dcca9e", read, write)]
    net_config: [u8; 256],
}

#[gatt_server]
struct OssmGattServer {
    ossm: OssmService,
}
```

---

## RPC Handler (subscribe-state fix)

The RPC characteristic must handle `subscribe-state` / `unsubscribe-state` and update the telemetry push flags. Use shared `crate::rpc::dispatch_rpc()`:

```rust
} else if handle == rpc_handle.handle {
    if let Ok(request) = serde_json::from_slice::<WsMessage>(data) {
        match crate::rpc::dispatch_rpc(&request, app_context).await {
            RpcAction::Respond(json) => {
                let _ = rpc_handle.notify(conn, json.as_bytes(), true).await;
            }
            RpcAction::Subscribe { interval_ms } => {
                *push_interval_ms = interval_ms;
                *subscribed = true;
                // Send "subscribed" acknowledgement via RPC notify
                let ack = /* build RpcResponseJson with result "subscribed" */;
                let _ = rpc_handle.notify(conn, ack.as_bytes(), true).await;
            }
            RpcAction::Unsubscribe => {
                *subscribed = false;
                // Send "unsubscribed" acknowledgement
            }
            RpcAction::Restart => {
                // Send ack, then schedule restart
            }
        }
    }
}
```

Without wiring `subscribe-state` to `*subscribed`, `push_telemetry` never sends state notifications and `test_ble.py` fails.

---

## Telemetry Push

```rust
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

async fn push_telemetry(
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    server: &OssmGattServer<'_>,
    app_context: &AppContext,
    subscribed: &AtomicBool,
    interval_ms: &AtomicU64,
) {
    let state_handle = server.ossm.state;
    let mut next_push = embassy_time::Instant::now();

    loop {
        embassy_time::Timer::at(next_push).await;
        let interval = interval_ms.load(Ordering::Relaxed).max(20);
        next_push += embassy_time::Duration::from_millis(interval);

        if !subscribed.load(Ordering::Relaxed) {
            continue;
        }

        let mc = app_context.motor_controller.lock().await;
        if let Some(mc) = mc.as_ref() {
            if let Ok(json) = serde_json::to_string(&mc.get_current_state()) {
                let _ = state_handle.notify(conn, json.as_bytes(), true).await;
            }
        }
    }
}
```

---

## GATT Event Loop

```rust
async fn serve_gatt(
    server: &OssmGattServer,
    peripheral: &mut Peripheral<'_>,
    app_context: AppContext,
) {
    // ... advertise with service UUID ...

    loop {
        let conn = /* advertise().accept().with_attribute_server(server) */;
        let subscribed = AtomicBool::new(false);
        let push_interval_ms = AtomicU64::new(500);

        embassy_futures::select::select(
            handle_gatt_events(&conn, server, &app_context, &subscribed, &push_interval_ms),
            push_telemetry(&conn, server, &app_context, &subscribed, &push_interval_ms),
        )
        .await;
    }
}
```

In `handle_gatt_events`, always call `event.accept()?.send().await` to complete the ATT transaction.

---

## MTU Considerations

- Default ATT MTU is ~23 bytes (20 bytes payload). `StateResponse` JSON is typically 500+ bytes.
- After connection, negotiate higher MTU if trouble-host supports it.
- Large reads: serve `StateResponse` via the `state` characteristic **read** handler (blob), not notify.
- RPC responses exceeding MTU must be truncated or the client must read from dedicated characteristics (`CHAR_STATE`, `CHAR_CONFIG`).
- Known ESP32-C6 issue: MTU 251 can cause HCI errors; start with MTU 128 and increase after validation.

---

## Key Improvements over Current

| Aspect | Current (`esp32-nimble`) | Target (`trouble-host`) |
|---|---|---|
| Callback model | Sync C callbacks + mpsc + ble_worker thread | Fully async |
| ATT write collision (0x0E) | 15ms delay workaround | Not applicable |
| Thread overhead | ble_worker (4KB) + ble_server (16KB) | Zero extra threads |
| Telemetry push | `std::thread::sleep` | `embassy_time::Timer` |
| Radio sharing | Implicit (IDF modem) | Explicit `BleConnector::new(radio, ...)` with `coex` |
