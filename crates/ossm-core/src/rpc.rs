use serde::Serialize;

use crate::command::Command;
use crate::config::{MotorControllerConfig, NetworkConfiguration};
use crate::engine::Engine;
use crate::error::CoreError;
use crate::rpc_types::{SubscribeParams, WaypointsInput, WsMessage};
use crate::state::StateResponse;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcAction {
    Respond,
    Subscribe { interval_ms: u64 },
    Unsubscribe,
    Restart,
}

const FALLBACK_INTERNAL: &[u8] =
    b"{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32603,\"message\":\"Internal error\"}}";

pub fn write_result<T: Serialize + ?Sized>(
    id: &serde_json::Value,
    result: &T,
    out: &mut [u8],
) -> Option<usize> {
    let resp = RpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    };
    serde_json_core::to_slice(&resp, out).ok()
}

pub fn write_error(
    id: &serde_json::Value,
    code: i32,
    message: &'static str,
    out: &mut [u8],
) -> Option<usize> {
    let resp: RpcResponse<'_, ()> = RpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(RpcErrorJson { code, message }),
    };
    serde_json_core::to_slice(&resp, out).ok()
}

fn write_or_fallback(
    id: &serde_json::Value,
    code: i32,
    message: &'static str,
    out: &mut [u8],
) -> usize {
    if let Some(n) = write_error(id, code, message, out) {
        n
    } else {
        copy_fallback(out)
    }
}

fn copy_fallback(out: &mut [u8]) -> usize {
    let n = FALLBACK_INTERNAL.len().min(out.len());
    out[..n].copy_from_slice(&FALLBACK_INTERNAL[..n]);
    n
}

#[derive(Serialize)]
struct StateNotification<'a> {
    jsonrpc: &'static str,
    method: &'static str,
    cmd: &'static str,
    params: &'a StateResponse,
    state: &'a StateResponse,
}

pub fn build_state_notification_into(state: &StateResponse, out: &mut [u8]) -> Option<usize> {
    let notif = StateNotification {
        jsonrpc: "2.0",
        method: "state",
        cmd: "state",
        params: state,
        state,
    };
    serde_json_core::to_slice(&notif, out).ok()
}

#[derive(Serialize)]
struct SubscribeAckPayload {
    subscribed: bool,
    interval_ms: u64,
}

pub fn write_subscribe_ack(
    id: &serde_json::Value,
    interval_ms: u64,
    out: &mut [u8],
) -> Option<usize> {
    let payload = SubscribeAckPayload {
        subscribed: true,
        interval_ms,
    };
    write_result(id, &payload, out)
}

pub fn write_unsubscribe_ack(id: &serde_json::Value, out: &mut [u8]) -> Option<usize> {
    write_result(id, "unsubscribed", out)
}

pub fn write_restart_ack(id: &serde_json::Value, out: &mut [u8]) -> Option<usize> {
    write_result(id, "restarting", out)
}

/// Sync JSON-RPC dispatcher against an [`Engine`]. Writes a JSON-RPC object into `out`
/// when the action is [`RpcAction::Respond`].
pub fn dispatch_rpc(engine: &mut Engine, request: &[u8], out: &mut [u8]) -> (RpcAction, usize) {
    let msg: WsMessage = match serde_json::from_slice(request) {
        Ok(m) => m,
        Err(_) => {
            let n = write_or_fallback(&serde_json::Value::Null, -32700, "Parse error", out);
            return (RpcAction::Respond, n);
        }
    };
    dispatch_parsed(engine, &msg, out)
}

pub fn dispatch_parsed(
    engine: &mut Engine,
    request: &WsMessage,
    out: &mut [u8],
) -> (RpcAction, usize) {
    let id = request.id.clone().unwrap_or(serde_json::Value::Null);
    let relay = engine.is_rtu_relay();

    match request.cmd.as_str() {
        "ping" => respond(&id, "pong", out),
        "status" => respond(&id, &engine.snapshot().stream, out),
        "get-state" | "get_state" => respond(&id, engine.snapshot(), out),
        "set-config" | "set_config" => {
            if relay {
                return error(&id, -32001, "rtu_relay mode", out);
            }
            match request.parse_params::<MotorControllerConfig>() {
                Ok(config) => match engine.try_set_config(config) {
                    Ok(applied) => respond(&id, &applied, out),
                    Err(CoreError::StaleVersion) => error(&id, -32001, "Stale causal version", out),
                    Err(CoreError::RelayMode) => error(&id, -32001, "rtu_relay mode", out),
                    Err(_) => error(&id, -32603, "Invalid config", out),
                },
                Err(_) => error(&id, -32602, "Invalid params", out),
            }
        }
        "subscribe-state" | "subscribe_state" => {
            if relay {
                return error(&id, -32001, "rtu_relay mode", out);
            }
            let params = request
                .parse_params::<SubscribeParams>()
                .unwrap_or_default();
            let interval_ms = params.interval_ms.unwrap_or(33).clamp(20, 60000);
            (RpcAction::Subscribe { interval_ms }, 0)
        }
        "unsubscribe-state" | "unsubscribe_state" => (RpcAction::Unsubscribe, 0),
        "append-waypoints" | "append_waypoints" => {
            if relay {
                return error(&id, -32001, "rtu_relay mode", out);
            }
            match request.parse_params::<WaypointsInput>() {
                Ok(input) => {
                    let (waypoints, _) = input.into_parts();
                    engine.apply(Command::AppendWaypoints(waypoints));
                    respond(&id, "ok", out)
                }
                Err(_) => error(&id, -32602, "Invalid params", out),
            }
        }
        "set-waypoints" | "set_waypoints" => {
            if relay {
                return error(&id, -32001, "rtu_relay mode", out);
            }
            match request.parse_params::<WaypointsInput>() {
                Ok(input) => {
                    let (waypoints, reset_timestamp) = input.into_parts();
                    engine.apply(Command::SetWaypoints {
                        waypoints,
                        reset_timestamp,
                    });
                    respond(&id, "ok", out)
                }
                Err(_) => error(&id, -32602, "Invalid params", out),
            }
        }
        "reset-timestamp" | "reset_timestamp" => {
            if relay {
                return error(&id, -32001, "rtu_relay mode", out);
            }
            engine.apply(Command::ResetTimestamp);
            respond(&id, "ok", out)
        }
        "get-network-config" | "get_network_config" => respond(&id, engine.net(), out),
        "set-network-config" | "set_network_config" => {
            match request.parse_params::<NetworkConfiguration>() {
                Ok(conf) => {
                    engine.apply(Command::SetNet(conf.clone()));
                    respond(&id, &conf, out)
                }
                Err(_) => error(&id, -32602, "Invalid params", out),
            }
        }
        "restart" => (RpcAction::Restart, 0),
        _ => error(&id, -32601, "Method not found", out),
    }
}

fn respond<T: Serialize + ?Sized>(
    id: &serde_json::Value,
    result: &T,
    out: &mut [u8],
) -> (RpcAction, usize) {
    let n = write_result(id, result, out).unwrap_or_else(|| copy_fallback(out));
    (RpcAction::Respond, n)
}

fn error(
    id: &serde_json::Value,
    code: i32,
    message: &'static str,
    out: &mut [u8],
) -> (RpcAction, usize) {
    (
        RpcAction::Respond,
        write_or_fallback(id, code, message, out),
    )
}
