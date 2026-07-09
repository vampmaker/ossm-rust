# 05: HTTP Server & WebSocket (JSON-RPC)

## Scope

Replace `edge-http` + `edge-nal-std` + `edge-ws` with `picoserve` running as Embassy tasks over `embassy-net` TCP sockets. Port all REST endpoints, CORS preflight, and the WebSocket JSON-RPC 2.0 handler. Extract shared RPC dispatch into `src/rpc.rs`.

## Files to Modify

- `src/http_api.rs` (complete rewrite of server setup and routing)
- New: `src/rpc.rs` (shared JSON-RPC dispatch for HTTP WS and BLE)

---

## Target Architecture

- `picoserve` with axum-inspired routing
- Runs as Embassy tasks (pool of 2) on the thread-mode executor
- `picoserve` built-in WebSocket support (`ws` feature)
- `embassy-net` TCP sockets
- `AppContext` captured in router closures; WebSocket uses `WebSocketCallbackWithState` or a static

---

## Server Setup

```rust
use embassy_net::Stack;
use embassy_time::Duration;
use picoserve::routing::{get, post, options};
use static_cell::StaticCell;

use crate::context::AppContext;

pub const WEB_TASK_POOL_SIZE: usize = 2;

pub async fn run_server(
    stack: &'static Stack<'static>,
    app_context: AppContext,
    spawner: &embassy_executor::Spawner,
) {
    let ctx = app_context;
    let app = picoserve::make_static!(
        impl picoserve::routing::PathRouter,
        picoserve::Router::new()
            .route("/", get(move |req| serve_html(req)))
            // REST
            .route("/config", get(move || get_config(ctx)).post(move |body| post_config(ctx, body)))
            .route("/state", get(move || get_state(ctx)))
            .route("/paused", post(move |body| post_paused(ctx, body)))
            .route("/pin-config", get(move || get_pin_config(ctx)).post(move |body| post_pin_config(ctx, body)))
            .route("/network-config", get(move || get_network_config(ctx)).post(move |body| post_network_config(ctx, body)))
            .route("/restart", post(move || post_restart(ctx)))
            // CORS preflight (required by frontend dev server)
            .route("/config", options(|| cors_preflight("GET, POST, OPTIONS")))
            .route("/paused", options(|| cors_preflight("POST, OPTIONS")))
            .route("/state", options(|| cors_preflight("GET, OPTIONS")))
            .route("/pin-config", options(|| cors_preflight("GET, POST, OPTIONS")))
            .route("/network-config", options(|| cors_preflight("GET, POST, OPTIONS")))
            .route("/restart", options(|| cors_preflight("POST, OPTIONS")))
            // WebSocket
            .route("/ws/command", get(move |upgrade: picoserve::response::ws::WebSocketUpgrade|
                upgrade.on_upgrade(WsCommandHandler { context: ctx })))
    );

    let config = picoserve::make_static!(
        picoserve::Config,
        picoserve::Config::new(picoserve::Timeouts {
            start_read_request: Some(Duration::from_secs(5)),
            read_request: Some(Duration::from_secs(1)),
            write: Some(Duration::from_secs(1)),
        })
    );

    for id in 0..WEB_TASK_POOL_SIZE {
        spawner.must_spawn(web_task(id, stack, app, config));
    }
}

#[embassy_executor::task(pool_size = WEB_TASK_POOL_SIZE)]
async fn web_task(/* ... */) -> ! {
    // Per-task TCP/HTTP buffers; listen_and_serve on port 80
}
```

---

## Response Helpers

All REST responses must include `Connection: close` (same reason as current `edge-http` — prevents task slot exhaustion when browsers hold keep-alive connections):

```rust
fn json_response(json: &str) -> impl picoserve::response::IntoResponse {
    picoserve::response::Response::new(picoserve::response::StatusCode::OK, json)
        .with_header("Content-Type", "application/json")
        .with_header("Access-Control-Allow-Origin", "*")
        .with_header("Connection", "close")
}

fn cors_preflight(allow_methods: &str) -> impl picoserve::response::IntoResponse {
    picoserve::response::Response::empty(picoserve::response::StatusCode::OK)
        .with_header("Access-Control-Allow-Origin", "*")
        .with_header("Access-Control-Allow-Methods", allow_methods)
        .with_header("Access-Control-Allow-Headers", "Content-Type")
        .with_header("Connection", "close")
}
```

---

## Shared RPC Dispatch (`src/rpc.rs`)

Extract command matching from `http_api.rs` so both WebSocket and BLE reuse it:

```rust
use crate::context::AppContext;
use crate::http_api::WsMessage;
use crate::motion::{MotionCommand, MotorControllerConfig};
use serde::Serialize;

#[derive(Serialize)]
pub struct RpcResponseJson {
    pub jsonrpc: &'static str,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcErrorJson>,
}

#[derive(Serialize)]
pub struct RpcErrorJson {
    pub code: i32,
    pub message: &'static str,
}

/// Outcome of dispatching a JSON-RPC command.
pub enum RpcAction {
    /// Send this JSON string as the RPC response.
    Respond(alloc::string::String),
    /// Begin periodic state push at this interval.
    Subscribe { interval_ms: u64 },
    /// Stop periodic state push.
    Unsubscribe,
    /// Schedule device restart.
    Restart,
}

pub async fn dispatch_rpc(request: &WsMessage, app_context: &AppContext) -> RpcAction {
    let id = request.id.clone().unwrap_or(serde_json::Value::Null);

    // Note: request.cmd matches either "cmd" or "method" via #[serde(alias = "method")]
    match request.cmd.as_str() {
        "ping" => {
            let resp = RpcResponseJson {
                jsonrpc: "2.0", id,
                result: Some(serde_json::Value::String("pong".into())),
                error: None,
            };
            RpcAction::Respond(serde_json::to_string(&resp).unwrap_or_default())
        }
        "get-state" | "get_state" => {
            let state = {
                let mut mc_opt = app_context.motor_controller.lock().await;
                mc_opt.as_mut().map(|mc| mc.get_current_state())
            };
            if let Some(state) = state {
                let resp = RpcResponseJson {
                    jsonrpc: "2.0", id,
                    result: serde_json::to_value(&state).ok(),
                    error: None,
                };
                RpcAction::Respond(serde_json::to_string(&resp).unwrap_or_default())
            } else {
                let resp = RpcResponseJson {
                    jsonrpc: "2.0", id, result: None,
                    error: Some(RpcErrorJson { code: -32603, message: "Motor controller not initialized" }),
                };
                RpcAction::Respond(serde_json::to_string(&resp).unwrap_or_default())
            }
        }
        "set-config" | "set_config" => {
            if let Ok(config) = request.parse_params::<MotorControllerConfig>() {
                let mut mc_opt = app_context.motor_controller.lock().await;
                if let Some(mc) = mc_opt.as_mut() {
                    let _ = mc.set_config(config);
                    let resp = RpcResponseJson {
                        jsonrpc: "2.0", id,
                        result: Some(serde_json::Value::String("ok".into())),
                        error: None,
                    };
                    return RpcAction::Respond(serde_json::to_string(&resp).unwrap_or_default());
                }
            }
            let resp = RpcResponseJson {
                jsonrpc: "2.0", id, result: None,
                error: Some(RpcErrorJson { code: -32602, message: "Invalid params" }),
            };
            RpcAction::Respond(serde_json::to_string(&resp).unwrap_or_default())
        }
        "subscribe-state" | "subscribe_state" => {
            let params = request.parse_params::<SubscribeParams>().unwrap_or_default();
            let interval = params.interval_ms.unwrap_or(33).clamp(20, 60000);
            RpcAction::Subscribe { interval_ms: interval }
        }
        "unsubscribe-state" | "unsubscribe_state" => RpcAction::Unsubscribe,
        "append-waypoints" | "append_waypoints" => {
            if let Ok(input) = request.parse_params::<WaypointsInput>() {
                let (waypoints, _) = input.into_parts();
                let cmd = MotionCommand::AppendWaypoints(waypoints);
                if let Some(mc) = app_context.motor_controller.lock().await.as_ref() {
                    let sender = mc.get_command_sender();
                    let _ = sender.lock().await.enqueue(cmd);
                    let resp = RpcResponseJson {
                        jsonrpc: "2.0", id,
                        result: Some(serde_json::Value::String("ok".into())),
                        error: None,
                    };
                    return RpcAction::Respond(serde_json::to_string(&resp).unwrap_or_default());
                }
            }
            let resp = RpcResponseJson {
                jsonrpc: "2.0", id, result: None,
                error: Some(RpcErrorJson { code: -32602, message: "Invalid params" }),
            };
            RpcAction::Respond(serde_json::to_string(&resp).unwrap_or_default())
        }
        "set-waypoints" | "set_waypoints" => {
            if let Ok(input) = request.parse_params::<WaypointsInput>() {
                let (waypoints, reset) = input.into_parts();
                if let Some(mc) = app_context.motor_controller.lock().await.as_ref() {
                    let sender = mc.get_command_sender();
                    let mut queue = sender.lock().await;
                    if reset {
                        let _ = queue.enqueue(MotionCommand::ResetTimestamp);
                    }
                    let _ = queue.enqueue(MotionCommand::SetWaypoints(waypoints));
                    let resp = RpcResponseJson {
                        jsonrpc: "2.0", id,
                        result: Some(serde_json::Value::String("ok".into())),
                        error: None,
                    };
                    return RpcAction::Respond(serde_json::to_string(&resp).unwrap_or_default());
                }
            }
            let resp = RpcResponseJson {
                jsonrpc: "2.0", id, result: None,
                error: Some(RpcErrorJson { code: -32602, message: "Invalid params" }),
            };
            RpcAction::Respond(serde_json::to_string(&resp).unwrap_or_default())
        }
        "reset-timestamp" | "reset_timestamp" => {
            if let Some(mc) = app_context.motor_controller.lock().await.as_ref() {
                let _ = mc.get_command_sender().lock().await.enqueue(MotionCommand::ResetTimestamp);
            }
            let resp = RpcResponseJson {
                jsonrpc: "2.0", id,
                result: Some(serde_json::Value::String("ok".into())),
                error: None,
            };
            RpcAction::Respond(serde_json::to_string(&resp).unwrap_or_default())
        }
        "get-network-config" | "get_network_config" => {
            let conf = app_context.storage.lock().await.get_network_configuration().unwrap_or_default();
            let resp = RpcResponseJson {
                jsonrpc: "2.0", id,
                result: serde_json::to_value(&conf).ok(),
                error: None,
            };
            RpcAction::Respond(serde_json::to_string(&resp).unwrap_or_default())
        }
        "set-network-config" | "set_network_config" => {
            if let Ok(conf) = request.parse_params::<NetworkConfiguration>() {
                let _ = app_context.storage.lock().await.set_network_configuration(&conf);
                let resp = RpcResponseJson {
                    jsonrpc: "2.0", id,
                    result: serde_json::to_value(&conf).ok(),
                    error: None,
                };
                return RpcAction::Respond(serde_json::to_string(&resp).unwrap_or_default());
            }
            let resp = RpcResponseJson {
                jsonrpc: "2.0", id, result: None,
                error: Some(RpcErrorJson { code: -32602, message: "Invalid params" }),
            };
            RpcAction::Respond(serde_json::to_string(&resp).unwrap_or_default())
        }
        "restart" => RpcAction::Restart,
        _ => {
            let resp = RpcResponseJson {
                jsonrpc: "2.0", id, result: None,
                error: Some(RpcErrorJson { code: -32601, message: "Method not found" }),
            };
            RpcAction::Respond(serde_json::to_string(&resp).unwrap_or_default())
        }
    }
}
```

Waypoint commands enqueue via `MotorController::get_command_sender()` (see [09-shared-state](09-shared-state.md)).

---

## WebSocket Handler

```rust
struct WsCommandHandler {
    context: AppContext,
}

impl picoserve::response::ws::WebSocketCallback for WsCommandHandler {
    async fn run<R: embedded_io_async::Read, W: embedded_io_async::Write>(
        self,
        mut rx: picoserve::response::ws::SocketRx<R>,
        mut tx: picoserve::response::ws::SocketTx<W>,
    ) -> Result<(), W::Error> {
        let mut subscribed = false;
        let mut push_interval_ms = 500u64;
        let mut next_push = embassy_time::Instant::now();
        let mut buf = [0u8; 1024];

        loop {
            if subscribed && embassy_time::Instant::now() >= next_push {
                next_push += embassy_time::Duration::from_millis(push_interval_ms);
                // Push state notification JSON (same format as current http_api.rs)
                if let Some(json) = build_state_notification(&self.context).await {
                    let _ = tx.send_text(&json).await;
                }
            }

            let msg = embassy_futures::select::select(
                rx.next_message(&mut buf),
                embassy_time::Timer::at(next_push),
            ).await;

            // Parse JSON-RPC, call dispatch_rpc(), handle RpcAction
            // Subscribe/Unsubscribe update local flags
            // Respond sends JSON via tx.send_text()
        }
    }
}
```

---

## Buffer Sizing Requirement for Large JSON Responses

Because `StateResponse` contains arrays (`update_history`, `position_history`) and nested configuration objects totaling ~1 KB of JSON, `picoserve` HTTP and WebSocket TX/RX buffers must be sized to at least **2048 bytes** (`[u8; 2048]`). Using default small buffers (512 or 1024 bytes) will cause JSON serialization panics or frame truncation when `/state` or WebSocket `get-state` is invoked:

```rust
const HTTP_BUFFER_SIZE: usize = 2048;
type AppRouter = picoserve::Router<AppRouterPath, AppState>;
```

---

## Endpoints to Port (checklist)

All routes from current `http_api.rs` must be ported:

| Method | Path | Handler |
|---|---|---|
| GET | `/` | Serve gzipped `index.html` |
| GET/POST | `/config` | Motor config read/write |
| GET | `/state` | Current `StateResponse` |
| POST | `/paused` | Pause/resume control |
| GET/POST | `/pin-config` | Pin configuration |
| GET/POST | `/network-config` | Network configuration |
| POST | `/restart` | Delayed `software_reset()` |
| OPTIONS | `/*` | CORS preflight for all above |
| GET (WS) | `/ws/command` | JSON-RPC 2.0 WebSocket |

---

## Key Differences from Current

| Aspect | Current (`edge-http`) | Target (`picoserve`) |
|---|---|---|
| Thread model | Dedicated 64KB OS thread | Embassy tasks (pool of 2) |
| TCP binding | `edge-nal-std` + eventfd | `embassy-net` TCP sockets |
| WebSocket | `edge-ws` manual framing | `picoserve` built-in WS |
| RPC dispatch | Inline in `http_api.rs` | Shared `src/rpc.rs` |
| CORS | Manual OPTIONS match arms | `.route(..., options(...))` |
| Connection lifecycle | `Connection: close` headers | Same — explicit per response |
