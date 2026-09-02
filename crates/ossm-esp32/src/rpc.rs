use alloc::string::String;

use serde::Serialize;

use crate::context::AppContext;
use crate::http_api::{RpcRequest, SubscribeParams, WaypointsInput};
use crate::motion::{MotionCommand, MotorControllerConfig};
use crate::storage::NetworkConfiguration;

pub enum RpcAction {
    Respond(String),
    Subscribe { interval_ms: u64 },
    Unsubscribe,
    Restart,
}

fn bytes_to_string(buf: &[u8]) -> Option<String> {
    core::str::from_utf8(buf).ok().map(String::from)
}

const INTERNAL_JSON: &str =
    "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32603,\"message\":\"Internal error\"}}";

async fn err_response(id: &serde_json::Value, code: i32, message: &'static str) -> RpcAction {
    let encoded = crate::buffers::try_with_scratchpad(|buf| {
        ossm_core::rpc::write_error(id, code, message, buf).and_then(|n| bytes_to_string(&buf[..n]))
    })
    .unwrap_or(None);
    RpcAction::Respond(encoded.unwrap_or_else(|| String::from(INTERNAL_JSON)))
}

async fn respond<T: Serialize + ?Sized>(id: &serde_json::Value, result: &T) -> RpcAction {
    let encoded = crate::buffers::try_with_scratchpad(|buf| {
        ossm_core::rpc::write_result(id, result, buf).and_then(|n| bytes_to_string(&buf[..n]))
    });

    match encoded {
        Some(Some(s)) => RpcAction::Respond(s),
        Some(None) => err_response(id, -32603, "Serialization failed").await,
        None => {
            let encoded_async = crate::buffers::with_scratchpad(|buf| {
                ossm_core::rpc::write_result(id, result, buf)
                    .and_then(|n| bytes_to_string(&buf[..n]))
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

pub async fn dispatch_rpc(request: &RpcRequest, app_context: AppContext) -> RpcAction {
    let id = request.id.clone().unwrap_or(serde_json::Value::Null);
    let blocked = !app_context.storage.pin().is_servo();

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
            if blocked {
                return err_response(&id, -32001, "not servo mode").await;
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
            if blocked {
                return err_response(&id, -32001, "not servo mode").await;
            }
            let params = request
                .parse_params::<SubscribeParams>()
                .unwrap_or_default();
            let interval = params.interval_ms.unwrap_or(33).clamp(20, 60000);
            RpcAction::Subscribe {
                interval_ms: interval,
            }
        }
        "unsubscribe-state" | "unsubscribe_state" => RpcAction::Unsubscribe,
        "append-waypoints" | "append_waypoints" => {
            if blocked {
                return err_response(&id, -32001, "not servo mode").await;
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
            if blocked {
                return err_response(&id, -32001, "not servo mode").await;
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
            if blocked {
                return err_response(&id, -32001, "not servo mode").await;
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

pub fn build_state_notification_into(app_context: AppContext, out: &mut [u8]) -> Option<usize> {
    let state = app_context.load_snapshot();
    ossm_core::rpc::build_state_notification_into(state.as_ref(), out)
}

fn ack_from_scratchpad(
    write: impl FnOnce(&mut [u8]) -> Option<usize>,
    fallback: impl FnOnce() -> String,
) -> String {
    crate::buffers::try_with_scratchpad(|buf| {
        write(buf)
            .and_then(|n| core::str::from_utf8(&buf[..n]).ok().map(String::from))
            .unwrap_or_default()
    })
    .filter(|s| !s.is_empty())
    .unwrap_or_else(fallback)
}

pub fn subscribe_ack(id: serde_json::Value, interval_ms: u64) -> String {
    ack_from_scratchpad(
        |buf| ossm_core::rpc::write_subscribe_ack(&id, interval_ms, buf),
        || {
            alloc::format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"subscribed\":true,\"interval_ms\":{}}}}}",
                id,
                interval_ms
            )
        },
    )
}

pub fn unsubscribe_ack(id: serde_json::Value) -> String {
    ack_from_scratchpad(
        |buf| ossm_core::rpc::write_unsubscribe_ack(&id, buf),
        || {
            alloc::format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":\"unsubscribed\"}}",
                id
            )
        },
    )
}

pub fn restart_ack(id: serde_json::Value) -> String {
    ack_from_scratchpad(
        |buf| ossm_core::rpc::write_restart_ack(&id, buf),
        || {
            alloc::format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":\"restarting\"}}",
                id
            )
        },
    )
}
