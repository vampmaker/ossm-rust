use alloc::string::String;

use serde::Serialize;

use crate::context::AppContext;
use crate::http_api::{SubscribeParams, WaypointsInput, WsMessage};
use crate::motion::{MotionCommand, MotorControllerConfig};
use crate::storage::NetworkConfiguration;

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

pub enum RpcAction {
    Respond(String),
    Subscribe { interval_ms: u64 },
    Unsubscribe,
    Restart,
}

fn ok_response(id: serde_json::Value, result: serde_json::Value) -> RpcAction {
    let resp = RpcResponseJson {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    };
    RpcAction::Respond(serde_json::to_string(&resp).unwrap_or_default())
}

fn err_response(id: serde_json::Value, code: i32, message: &'static str) -> RpcAction {
    let resp = RpcResponseJson {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(RpcErrorJson { code, message }),
    };
    RpcAction::Respond(serde_json::to_string(&resp).unwrap_or_default())
}

pub async fn dispatch_rpc(request: &WsMessage, app_context: AppContext) -> RpcAction {
    let id = request.id.clone().unwrap_or(serde_json::Value::Null);

    match request.cmd.as_str() {
        "ping" => ok_response(id, serde_json::Value::String(String::from("pong"))),
        "status" => {
            let status = {
                let mc_opt = app_context.motor_controller.lock().await;
                mc_opt.as_ref().map(|mc| mc.get_stream_status())
            };
            match status {
                Some(st) => {
                    let resp = RpcResponseJson {
                        jsonrpc: "2.0",
                        id,
                        result: serde_json::to_value(st).ok(),
                        error: None,
                    };
                    RpcAction::Respond(serde_json::to_string(&resp).unwrap_or_default())
                }
                None => err_response(id, -32603, "Motor controller not initialized"),
            }
        }
        "get-state" | "get_state" => {
            let state = {
                let mut mc_opt = app_context.motor_controller.lock().await;
                mc_opt.as_mut().map(|mc| mc.get_current_state())
            };
            match state {
                Some(state) => {
                    let resp = RpcResponseJson {
                        jsonrpc: "2.0",
                        id,
                        result: serde_json::to_value(&state).ok(),
                        error: None,
                    };
                    RpcAction::Respond(serde_json::to_string(&resp).unwrap_or_default())
                }
                None => err_response(id, -32603, "Motor controller not initialized"),
            }
        }
        "set-config" | "set_config" => {
            if let Ok(config) = request.parse_params::<MotorControllerConfig>() {
                let applied = {
                    let mut mc_opt = app_context.motor_controller.lock().await;
                    if let Some(mc) = mc_opt.as_mut() {
                        mc.set_config(config.clone()).is_ok()
                    } else {
                        false
                    }
                };
                if applied {
                    let resp = RpcResponseJson {
                        jsonrpc: "2.0",
                        id,
                        result: serde_json::to_value(&config).ok(),
                        error: None,
                    };
                    return RpcAction::Respond(
                        serde_json::to_string(&resp).unwrap_or_default(),
                    );
                }
                return err_response(id, -32603, "Failed to apply config");
            }
            err_response(id, -32602, "Invalid params")
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
                if let Some(mc) = app_context.motor_controller.lock().await.as_ref() {
                    let sender = mc.get_command_sender();
                    if sender
                        .lock()
                        .await
                        .enqueue(MotionCommand::AppendWaypoints(waypoints))
                        .is_ok()
                    {
                        return ok_response(id, serde_json::Value::String(String::from("ok")));
                    }
                }
            }
            err_response(id, -32602, "Invalid params")
        }
        "set-waypoints" | "set_waypoints" => {
            if let Ok(input) = request.parse_params::<WaypointsInput>() {
                let (waypoints, reset_timestamp) = input.into_parts();
                if let Some(mc) = app_context.motor_controller.lock().await.as_ref() {
                    let sender = mc.get_command_sender();
                    let mut queue = sender.lock().await;
                    if reset_timestamp {
                        let _ = queue.enqueue(MotionCommand::ResetTimestamp);
                    }
                    if queue
                        .enqueue(MotionCommand::SetWaypoints {
                            waypoints,
                            reset_timestamp,
                        })
                        .is_ok()
                    {
                        return ok_response(id, serde_json::Value::String(String::from("ok")));
                    }
                }
            }
            err_response(id, -32602, "Invalid params")
        }
        "reset-timestamp" | "reset_timestamp" => {
            if let Some(mc) = app_context.motor_controller.lock().await.as_ref() {
                let _ = mc
                    .get_command_sender()
                    .lock()
                    .await
                    .enqueue(MotionCommand::ResetTimestamp);
            }
            ok_response(id, serde_json::Value::String(String::from("ok")))
        }
        "get-network-config" | "get_network_config" => {
            let conf = app_context
                .storage
                .lock()
                .await
                .get_network_configuration()
                .unwrap_or_default();
            let resp = RpcResponseJson {
                jsonrpc: "2.0",
                id,
                result: serde_json::to_value(&conf).ok(),
                error: None,
            };
            RpcAction::Respond(serde_json::to_string(&resp).unwrap_or_default())
        }
        "set-network-config" | "set_network_config" => {
            if let Ok(conf) = request.parse_params::<NetworkConfiguration>() {
                let _ = app_context
                    .storage
                    .lock()
                    .await
                    .set_network_configuration(&conf);
                let resp = RpcResponseJson {
                    jsonrpc: "2.0",
                    id,
                    result: serde_json::to_value(&conf).ok(),
                    error: None,
                };
                return RpcAction::Respond(serde_json::to_string(&resp).unwrap_or_default());
            }
            err_response(id, -32602, "Invalid params")
        }
        "restart" => RpcAction::Restart,
        _ => err_response(id, -32601, "Method not found"),
    }
}

pub async fn build_state_notification(
    app_context: AppContext,
) -> Option<String> {
    let state = {
        let mut mc_opt = app_context.motor_controller.lock().await;
        mc_opt.as_mut().map(|mc| mc.get_current_state())
    };
    state.and_then(|s| {
        let notif = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "state",
            "cmd": "state",
            "params": s,
            "state": s,
        });
        serde_json::to_string(&notif).ok()
    })
}

pub fn subscribe_ack(id: serde_json::Value, interval_ms: u64) -> String {
    let resp = RpcResponseJson {
        jsonrpc: "2.0",
        id,
        result: Some(serde_json::json!({
            "subscribed": true,
            "interval_ms": interval_ms,
        })),
        error: None,
    };
    serde_json::to_string(&resp).unwrap_or_default()
}

pub fn unsubscribe_ack(id: serde_json::Value) -> String {
    let resp = RpcResponseJson {
        jsonrpc: "2.0",
        id,
        result: Some(serde_json::Value::String(String::from("unsubscribed"))),
        error: None,
    };
    serde_json::to_string(&resp).unwrap_or_default()
}

pub fn restart_ack(id: serde_json::Value) -> String {
    let resp = RpcResponseJson {
        jsonrpc: "2.0",
        id,
        result: Some(serde_json::Value::String(String::from("restarting"))),
        error: None,
    };
    serde_json::to_string(&resp).unwrap_or_default()
}
