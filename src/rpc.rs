use alloc::string::String;

use serde::Serialize;

use crate::context::AppContext;
use crate::http_api::{SubscribeParams, WaypointsInput, WsMessage};
use crate::modbus_relay;
use crate::motion::{MotionCommand, MotorControllerConfig};
use crate::storage::NetworkConfiguration;

#[derive(Serialize)]
struct RpcResponse<'a, T: ?Sized> {
    jsonrpc: &'static str,
    id: &'a serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<&'a T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcErrorJson>,
}

#[derive(Serialize)]
pub struct RpcErrorJson {
    pub code: i32,
    pub message: &'static str,
}

pub enum RpcAction {
    Respond(String),
    Subscribe { interval_ms: u64 },
    Unsubscribe,
    Restart,
}

async fn err_response(id: &serde_json::Value, code: i32, message: &'static str) -> RpcAction {
    let resp: RpcResponse<'_, ()> = RpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(RpcErrorJson { code, message }),
    };
    let encoded = crate::buffers::try_with_scratchpad(|buf| {
        if let Ok(len) = serde_json_core::to_slice(&resp, buf) {
            if let Ok(s) = core::str::from_utf8(&buf[..len]) {
                return Some(String::from(s));
            }
        }
        None
    })
    .unwrap_or(None);
    RpcAction::Respond(encoded.unwrap_or_else(|| {
        String::from("{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32603,\"message\":\"Internal error\"}}")
    }))
}

async fn respond<T: Serialize + ?Sized>(id: &serde_json::Value, result: &T) -> RpcAction {
    let resp = RpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    };
    
    let encoded = crate::buffers::try_with_scratchpad(|buf| {
        if let Ok(len) = serde_json_core::to_slice(&resp, buf) {
            if let Ok(s) = core::str::from_utf8(&buf[..len]) {
                return Some(String::from(s));
            }
        }
        None
    });

    match encoded {
        Some(Some(s)) => RpcAction::Respond(s),
        Some(None) => err_response(id, -32603, "Serialization failed").await,
        None => {
            // Scratchpad was locked, fallback to async wait
            let encoded_async = crate::buffers::with_scratchpad(|buf| {
                if let Ok(len) = serde_json_core::to_slice(&resp, buf) {
                    if let Ok(s) = core::str::from_utf8(&buf[..len]) {
                        return Some(String::from(s));
                    }
                }
                None
            })
            .await;
            
            if let Some(s) = encoded_async {
                RpcAction::Respond(s)
            } else {
                err_response(id, -32603, "Serialization failed").await
            }
        }
    }
}

pub async fn dispatch_rpc(request: &WsMessage, app_context: AppContext) -> RpcAction {
    let id = request.id.clone().unwrap_or(serde_json::Value::Null);
    let relay = modbus_relay::is_active() || app_context.storage.pin().is_rtu_relay();

    match request.cmd.as_str() {
        "ping" => respond(&id, "pong").await,
        "status" => {
            let status = app_context.load_snapshot().stream;
            respond(&id, &status).await
        }
        "get-state" | "get_state" => {
            let state = app_context.load_snapshot();
            respond(&id, &*state).await
        }
        "set-config" | "set_config" => {
            if relay {
                return err_response(&id, -32001, "rtu_relay mode").await;
            }
            if let Ok(config) = request.parse_params::<MotorControllerConfig>() {
                match app_context.try_enqueue_config(config).await {
                    Ok(applied) => return respond(&id, &applied).await,
                    Err("Stale causal version") => {
                        return err_response(&id, -32001, "Stale causal version").await
                    }
                    Err(msg) => return err_response(&id, -32603, msg).await,
                }
            }
            err_response(&id, -32602, "Invalid params").await
        }
        "subscribe-state" | "subscribe_state" => {
            if relay {
                return err_response(&id, -32001, "rtu_relay mode").await;
            }
            let params = request.parse_params::<SubscribeParams>().unwrap_or_default();
            let interval = params.interval_ms.unwrap_or(33).clamp(20, 60000);
            RpcAction::Subscribe { interval_ms: interval }
        }
        "unsubscribe-state" | "unsubscribe_state" => RpcAction::Unsubscribe,
        "append-waypoints" | "append_waypoints" => {
            if relay {
                return err_response(&id, -32001, "rtu_relay mode").await;
            }
            if let Ok(input) = request.parse_params::<WaypointsInput>() {
                let (waypoints, _) = input.into_parts();
                if app_context
                    .enqueue_motion(MotionCommand::AppendWaypoints(waypoints))
                    .await
                {
                    return respond(&id, "ok").await;
                }
            }
            err_response(&id, -32602, "Invalid params").await
        }
        "set-waypoints" | "set_waypoints" => {
            if relay {
                return err_response(&id, -32001, "rtu_relay mode").await;
            }
            if let Ok(input) = request.parse_params::<WaypointsInput>() {
                let (waypoints, reset_timestamp) = input.into_parts();
                if app_context
                    .enqueue_motion(MotionCommand::SetWaypoints {
                        waypoints,
                        reset_timestamp,
                    })
                    .await
                {
                    return respond(&id, "ok").await;
                }
            }
            err_response(&id, -32602, "Invalid params").await
        }
        "reset-timestamp" | "reset_timestamp" => {
            if relay {
                return err_response(&id, -32001, "rtu_relay mode").await;
            }
            let _ = app_context
                .enqueue_motion(MotionCommand::ResetTimestamp)
                .await;
            respond(&id, "ok").await
        }
        "get-network-config" | "get_network_config" => {
            let conf = app_context.storage.net();
            respond(&id, &conf).await
        }
        "set-network-config" | "set_network_config" => {
            if let Ok(conf) = request.parse_params::<NetworkConfiguration>() {
                app_context.storage.set_net(conf.clone());
                return respond(&id, &conf).await;
            }
            err_response(&id, -32602, "Invalid params").await
        }
        "restart" => RpcAction::Restart,
        _ => err_response(&id, -32601, "Method not found").await,
    }
}

#[derive(Serialize)]
struct StateNotification<'a> {
    jsonrpc: &'static str,
    method: &'static str,
    cmd: &'static str,
    params: &'a crate::motion::StateResponse,
    state: &'a crate::motion::StateResponse,
}

pub fn build_state_notification_into(
    app_context: AppContext,
    out: &mut [u8],
) -> Option<usize> {
    let state = app_context.load_snapshot();

    let notif = StateNotification {
        jsonrpc: "2.0",
        method: "state",
        cmd: "state",
        params: state.as_ref(),
        state: state.as_ref(),
    };

    serde_json_core::to_slice(&notif, out).ok()
}

#[derive(Serialize)]
struct SubscribeAckPayload {
    subscribed: bool,
    interval_ms: u64,
}

pub fn subscribe_ack(id: serde_json::Value, interval_ms: u64) -> String {
    let payload = SubscribeAckPayload {
        subscribed: true,
        interval_ms,
    };
    let resp = RpcResponse {
        jsonrpc: "2.0",
        id: &id,
        result: Some(&payload),
        error: None,
    };
    crate::buffers::try_with_scratchpad(|buf| {
        let len = serde_json_core::to_slice(&resp, buf).unwrap_or(0);
        core::str::from_utf8(&buf[..len])
            .map(String::from)
            .unwrap_or_default()
    })
    .unwrap_or_else(|| alloc::format!("{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"subscribed\":true,\"interval_ms\":{}}}}}", id, interval_ms))
}

pub fn unsubscribe_ack(id: serde_json::Value) -> String {
    let resp = RpcResponse {
        jsonrpc: "2.0",
        id: &id,
        result: Some("unsubscribed"),
        error: None,
    };
    crate::buffers::try_with_scratchpad(|buf| {
        let len = serde_json_core::to_slice(&resp, buf).unwrap_or(0);
        core::str::from_utf8(&buf[..len])
            .map(String::from)
            .unwrap_or_default()
    })
    .unwrap_or_else(|| alloc::format!("{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":\"unsubscribed\"}}", id))
}

pub fn restart_ack(id: serde_json::Value) -> String {
    let resp = RpcResponse {
        jsonrpc: "2.0",
        id: &id,
        result: Some("restarting"),
        error: None,
    };
    crate::buffers::try_with_scratchpad(|buf| {
        let len = serde_json_core::to_slice(&resp, buf).unwrap_or(0);
        core::str::from_utf8(&buf[..len])
            .map(String::from)
            .unwrap_or_default()
    })
    .unwrap_or_else(|| alloc::format!("{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":\"restarting\"}}", id))
}
