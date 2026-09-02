use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use ossm_core::rpc::RpcAction;
use ossm_core::{CoreError, MotorControllerConfig, PausedControl, RpcRequest, StateResponse};
use serde::Serialize;
use tokio::sync::oneshot;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;

use crate::engine_task::{EngineHandle, EngineMsg};

#[derive(Clone)]
pub struct AppState {
    pub engine: EngineHandle,
    pub static_dir: PathBuf,
    pub restart: Arc<dyn Fn() + Send + Sync>,
}

pub fn router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);
    let static_dir = state.static_dir.clone();
    Router::new()
        .route("/", get(index))
        .route("/config", get(get_config).post(post_config))
        .route("/state", get(get_state))
        .route("/paused", post(post_paused))
        .route("/restart", post(post_restart))
        .route("/ws/command", get(ws_upgrade))
        .fallback_service(ServeDir::new(static_dir))
        .layer(cors)
        .with_state(state)
}

async fn index(State(state): State<AppState>) -> Response {
    let path = state.static_dir.join("index.html");
    match tokio::fs::read_to_string(path).await {
        Ok(html) => Html(html).into_response(),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "frontend/dist/index.html missing — run npm run build in frontend/",
        )
            .into_response(),
    }
}

fn json_with_cors<T: Serialize>(status: StatusCode, body: T) -> Response {
    let bytes = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
    (
        status,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
            (header::CONNECTION, "close"),
        ],
        bytes,
    )
        .into_response()
}

async fn get_config(State(state): State<AppState>) -> Response {
    let snap = state.engine.snap.borrow().clone();
    json_with_cors(StatusCode::OK, snap.config)
}

async fn post_config(State(state): State<AppState>, body: axum::body::Bytes) -> Response {
    if body.len() > 1024 {
        return json_with_cors(StatusCode::BAD_REQUEST, "Request too large");
    }
    let Ok(config) = serde_json::from_slice::<MotorControllerConfig>(&body) else {
        return json_with_cors(StatusCode::BAD_REQUEST, "Bad Request");
    };
    match try_set(&state, config).await {
        Ok(applied) => json_with_cors(StatusCode::OK, applied),
        Err(CoreError::StaleVersion) => {
            json_with_cors(StatusCode::CONFLICT, "Stale causal version")
        }
        Err(_) => json_with_cors(StatusCode::BAD_REQUEST, "Invalid config"),
    }
}

async fn get_state(State(state): State<AppState>) -> Json<StateResponse> {
    Json(state.engine.snap.borrow().clone())
}

async fn post_paused(State(state): State<AppState>, body: axum::body::Bytes) -> Response {
    let Ok(control) = serde_json::from_slice::<PausedControl>(&body) else {
        return json_with_cors(StatusCode::BAD_REQUEST, "Bad Request");
    };
    let mut config = state.engine.snap.borrow().config.clone();
    if let Some(paused) = control.paused {
        config.paused = paused;
    }
    if let Some(position) = control.position.or(control.paused_position) {
        config.paused_position = position.clamp(0.0, 1.0);
    }
    if let Some(adjust) = control.adjust {
        config.paused_position = (config.paused_position + adjust).clamp(0.0, 1.0);
    }
    match try_set(&state, config).await {
        Ok(applied) => json_with_cors(StatusCode::OK, applied),
        Err(CoreError::StaleVersion) => {
            json_with_cors(StatusCode::CONFLICT, "Stale causal version")
        }
        Err(_) => json_with_cors(StatusCode::SERVICE_UNAVAILABLE, "Command queue full"),
    }
}

async fn post_restart(State(state): State<AppState>) -> Response {
    let restart = state.restart.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        restart();
    });
    json_with_cors(StatusCode::OK, serde_json::json!({"ok": true}))
}

async fn try_set(
    state: &AppState,
    cfg: MotorControllerConfig,
) -> Result<MotorControllerConfig, CoreError> {
    let (tx, rx) = oneshot::channel();
    state
        .engine
        .tx
        .send(EngineMsg::TrySetConfig { cfg, reply: tx })
        .await
        .map_err(|_| CoreError::InvalidConfig)?;
    rx.await.map_err(|_| CoreError::InvalidConfig)?
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| run_ws(socket, state))
}

async fn run_ws(mut socket: WebSocket, state: AppState) {
    let mut subscribed = false;
    let mut interval_ms = 33u64;
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(interval_ms));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let Some(Ok(msg)) = incoming else { break; };
                let data = match msg {
                    Message::Text(t) => t.as_bytes().to_vec(),
                    Message::Binary(b) => b.to_vec(),
                    Message::Ping(p) => {
                        let _ = socket.send(Message::Pong(p)).await;
                        continue;
                    }
                    Message::Close(_) => break,
                    _ => continue,
                };
                let parsed = match serde_json::from_slice::<RpcRequest>(&data) {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                let id = parsed.id.clone().unwrap_or(serde_json::Value::Null);
                let (tx, rx) = oneshot::channel();
                if state
                    .engine
                    .tx
                    .send(EngineMsg::Rpc { req: data, reply: tx })
                    .await
                    .is_err()
                {
                    break;
                }
                let Ok((action, out)) = rx.await else { break; };
                match action {
                    RpcAction::Respond => {
                        if !out.is_empty() {
                            let text = String::from_utf8_lossy(&out).into_owned();
                            if socket.send(Message::Text(text.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    RpcAction::Subscribe { interval_ms: ms } => {
                        interval_ms = ms;
                        subscribed = true;
                        ticker = tokio::time::interval(std::time::Duration::from_millis(interval_ms));
                        let mut buf = [0u8; 512];
                        if let Some(n) =
                            ossm_core::rpc::write_subscribe_ack(&id, interval_ms, &mut buf)
                        {
                            let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                            let _ = socket.send(Message::Text(text.into())).await;
                        }
                    }
                    RpcAction::Unsubscribe => {
                        subscribed = false;
                        let mut buf = [0u8; 256];
                        if let Some(n) = ossm_core::rpc::write_unsubscribe_ack(&id, &mut buf) {
                            let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                            let _ = socket.send(Message::Text(text.into())).await;
                        }
                    }
                    RpcAction::Restart => {
                        let mut buf = [0u8; 256];
                        if let Some(n) = ossm_core::rpc::write_restart_ack(&id, &mut buf) {
                            let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                            let _ = socket.send(Message::Text(text.into())).await;
                        }
                        let restart = state.restart.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            restart();
                        });
                    }
                }
            }
            _ = ticker.tick(), if subscribed => {
                let snap = state.engine.snap.borrow().clone();
                let mut buf = vec![0u8; 4096];
                if let Some(n) = ossm_core::rpc::build_state_notification_into(&snap, &mut buf) {
                    let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
            }
        }
    }
}
