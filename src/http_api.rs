use core::fmt::{Debug, Display};

use edge_http::io::server::{Connection, Handler, Server};
use edge_http::io::Error;
use edge_http::ws::MAX_BASE64_KEY_RESPONSE_LEN;
use edge_http::Method;
use edge_nal::{TcpBind, TcpSplit};
use edge_ws::{FrameHeader, FrameType};
use embedded_io_async::{Read, Write};
use serde::{Deserialize, Serialize};

use crate::context::AppContext;
use crate::motion::{MotionCommand, MotorControllerConfig, StreamWaypoint};
use crate::storage::{NetworkConfiguration, PinConfiguration};

// Requests accepted on the /ws/command websocket. AppendWaypoints/SetWaypoints/ResetTimestamp map
// to MotionCommand; Status makes the device reply with a JSON status frame.
#[derive(Deserialize)]
pub struct WsMessage {
    #[serde(default)]
    pub id: Option<serde_json::Value>,
    #[serde(alias = "method")]
    pub cmd: String,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

impl WsMessage {
    pub fn parse_params<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        if let Some(ref params) = self.params {
            if !params.is_null() && (params.is_object() || params.is_array()) {
                return serde_json::from_value(params.clone());
            }
        }
        if let Some(config_val) = self.extra.get("config") {
            if !config_val.is_null() && config_val.is_object() {
                return serde_json::from_value(config_val.clone());
            }
        }
        serde_json::from_value(self.extra.clone())
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum WaypointsInput {
    List(Vec<StreamWaypoint>),
    Object {
        waypoints: Vec<StreamWaypoint>,
        #[serde(default, alias = "reset-timestamp", alias = "reset")]
        reset_timestamp: Option<bool>,
    },
    Single(StreamWaypoint),
}

impl WaypointsInput {
    pub fn into_parts(self) -> (Vec<StreamWaypoint>, bool) {
        match self {
            WaypointsInput::List(list) => (list, false),
            WaypointsInput::Object { waypoints, reset_timestamp } => (waypoints, reset_timestamp.unwrap_or(false)),
            WaypointsInput::Single(wp) => (vec![wp], false),
        }
    }
}

#[derive(Deserialize, Default)]
pub struct SubscribeParams {
    #[serde(default)]
    pub interval_ms: Option<u64>,
}

#[derive(Serialize)]
pub struct RpcResponse<'a, T: Serialize> {
    pub jsonrpc: &'static str,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<&'a T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError<'a>>,
}

#[derive(Serialize)]
pub struct RpcError<'a> {
    pub code: i32,
    pub message: &'a str,
}

#[derive(Serialize)]
pub struct WsNotification<'a, T: Serialize> {
    pub jsonrpc: &'static str,
    pub method: &'a str,
    pub cmd: &'a str,
    pub params: &'a T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<&'a T>,
}

async fn send_ws_text<S>(
    socket: &mut S,
    json: &str,
) -> Result<(), ()>
where
    S: Read + Write + TcpSplit,
{
    let reply = FrameHeader {
        frame_type: FrameType::Text(false),
        payload_len: json.len() as u64,
        mask_key: None,
    };
    if reply.send(&mut *socket).await.is_err()
        || reply.send_payload(&mut *socket, json.as_bytes()).await.is_err()
    {
        return Err(());
    }
    Ok(())
}

async fn send_rpc_result<S, T>(
    socket: &mut S,
    id: Option<&serde_json::Value>,
    result: &T,
) -> Result<(), ()>
where
    S: Read + Write + TcpSplit,
    T: Serialize,
{
    let resp = RpcResponse {
        jsonrpc: "2.0",
        id: id.cloned().unwrap_or(serde_json::Value::Null),
        result: Some(result),
        error: None,
    };
    let json = match serde_json::to_string(&resp) {
        Ok(j) => j,
        Err(_) => return Err(()),
    };
    send_ws_text(socket, &json).await
}

async fn send_rpc_error<S>(
    socket: &mut S,
    id: Option<&serde_json::Value>,
    code: i32,
    message: &str,
) -> Result<(), ()>
where
    S: Read + Write + TcpSplit,
{
    let resp = RpcResponse::<()> {
        jsonrpc: "2.0",
        id: id.cloned().unwrap_or(serde_json::Value::Null),
        result: None,
        error: Some(RpcError { code, message }),
    };
    let json = match serde_json::to_string(&resp) {
        Ok(j) => j,
        Err(_) => return Err(()),
    };
    send_ws_text(socket, &json).await
}

async fn send_ws_notification<S, T>(
    socket: &mut S,
    method: &str,
    params: &T,
) -> Result<(), ()>
where
    S: Read + Write + TcpSplit,
    T: Serialize,
{
    let notif = WsNotification {
        jsonrpc: "2.0",
        method,
        cmd: method,
        params,
        state: None,
    };
    let json = match serde_json::to_string(&notif) {
        Ok(j) => j,
        Err(_) => return Err(()),
    };
    send_ws_text(socket, &json).await
}

#[derive(Serialize, Deserialize)]
pub struct PausedControl {
    pub paused: Option<bool>,              // Set paused state
    pub position: Option<f32>,             // Set absolute position
    pub adjust: Option<f32>,               // Adjust position relatively (positive or negative)
}

const APP_HTML_GZ: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/index.html.gz"));

const NO_GZIP_HTML: &[u8] = b"\
<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
<title>OSSM</title></head><body style=\"font-family:sans-serif;text-align:center;padding:2em\">\
<h1>Browser Not Supported</h1>\
<p>This device serves a compressed interface that requires <code>Accept-Encoding: gzip</code>.</p>\
<p>Please use a modern browser (Chrome, Firefox, Safari, Edge).</p>\
<p>If using curl, use <code>curl --compressed</code>.</p>\
</body></html>";

// Bodies must be fully buffered because serde_json needs a contiguous slice;
// all POST payloads are small JSON objects, so 1 KiB is plenty. Note this
// array is held across await points, so it is part of each handler future.
const MAX_BODY_LEN: usize = 1024;
// Keep this low: the whole HTTP server future (including one handler future per
// task) lives inside net_task, which Embassy allocates statically in .bss.
const HANDLER_TASKS: usize = 2;
const HTTP_BUF_SIZE: usize = 2048;

const CORS_ORIGIN: (&str, &str) = ("Access-Control-Allow-Origin", "*");
const CONNECTION_CLOSE: (&str, &str) = ("Connection", "close");

type HttpServer = Server<HANDLER_TASKS, HTTP_BUF_SIZE>;

pub async fn run_server(app_context: AppContext) -> anyhow::Result<()> {
    let addr = "0.0.0.0:80";
    log::info!("Starting HTTP server on {}", addr);

    let acceptor = edge_nal_std::Stack::new().bind(addr.parse().unwrap()).await?;

    // The server run future is ~29KB (handler futures embed per-task buffers).
    // Box::pin it: keeping it on the stack was tried and overflows even a
    // 128KB thread stack, because at low opt levels every by-value move
    // (into block_on, into its pinned slot) leaves another ~29KB dead copy in
    // a live stack frame, on top of the deep poll call chain. Boxing pays one
    // transient stack copy during construction, then the stack is free for
    // polling, so total memory is lower (64KB stack + ~29KB heap).
    let mut server: Box<HttpServer> = Box::new(HttpServer::new());
    Box::pin(server.run(Some(50_000), acceptor, HttpHandler { app_context })).await?;

    Ok(())
}

struct HttpHandler {
    app_context: AppContext,
}

impl Handler for HttpHandler {
    type Error<E>
        = Error<E>
    where
        E: Debug;

    async fn handle<T, const N: usize>(
        &self,
        _task_id: impl Display + Copy,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), Self::Error<T::Error>>
    where
        T: Read + Write + TcpSplit,
    {
        let headers = conn.headers()?;
        let method = headers.method;
        let path = headers.path;

        match (method, path) {
            (Method::Get, "/") => {
                let accept_encoding = headers.headers.get("Accept-Encoding").unwrap_or("");
                if accept_encoding.contains("gzip") {
                    conn.initiate_response(
                        200,
                        Some("OK"),
                        &[
                            CORS_ORIGIN,
                            ("Content-Type", "text/html"),
                            ("Content-Encoding", "gzip"),
                        ],
                    )
                    .await?;
                    conn.write_all(APP_HTML_GZ).await?;
                } else {
                    conn.initiate_response(200, Some("OK"), &[CORS_ORIGIN, ("Content-Type", "text/html")])
                        .await?;
                    conn.write_all(NO_GZIP_HTML).await?;
                }
            }

            (Method::Options, "/config") => cors_preflight(conn, "GET, POST, OPTIONS").await?,
            (Method::Options, "/paused") => cors_preflight(conn, "POST, OPTIONS").await?,
            (Method::Options, "/state") => cors_preflight(conn, "GET, OPTIONS").await?,
            (Method::Options, "/pin-config") => cors_preflight(conn, "GET, POST, OPTIONS").await?,
            (Method::Options, "/network-config") => cors_preflight(conn, "GET, POST, OPTIONS").await?,
            (Method::Options, "/restart") => cors_preflight(conn, "POST, OPTIONS").await?,

            (Method::Get, "/config") => {
                let config = {
                    let mc_opt = self.app_context.motor_controller.lock().unwrap();
                    mc_opt.as_ref().map(|mc| mc.get_config())
                };
                let json = config.map(|c| serde_json::to_string(&c).unwrap());
                match json {
                    Some(json) => respond_json(conn, &json).await?,
                    None => respond_unavailable(conn).await?,
                }
            }

            (Method::Post, "/config") => {
                let mut body = [0u8; MAX_BODY_LEN];
                let Some(len) = read_body(conn, &mut body).await? else {
                    respond_bad_request(conn, "Request too large").await?;
                    return Ok(());
                };
                match serde_json::from_slice::<MotorControllerConfig>(&body[..len]) {
                    Ok(config) => {
                        let json = serde_json::to_string(&config).unwrap();
                        let applied = {
                            let mut mc_opt = self.app_context.motor_controller.lock().unwrap();
                            if let Some(mc) = mc_opt.as_mut() {
                                mc.set_config(config).unwrap();
                                true
                            } else {
                                false
                            }
                        };
                        if applied {
                            respond_json(conn, &json).await?;
                        } else {
                            respond_unavailable(conn).await?;
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to parse config: {}", e);
                        respond_bad_request(conn, "Bad Request").await?;
                    }
                }
            }

            (Method::Post, "/paused") => {
                let mut body = [0u8; MAX_BODY_LEN];
                let Some(len) = read_body(conn, &mut body).await? else {
                    respond_bad_request(conn, "Request too large").await?;
                    return Ok(());
                };
                match serde_json::from_slice::<PausedControl>(&body[..len]) {
                    Ok(control) => {
                        let updated_cfg = {
                            let mut mc_opt = self.app_context.motor_controller.lock().unwrap();
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

                                mc.set_config(config.clone()).unwrap();
                                Some(config)
                            } else {
                                None
                            }
                        };
                        let json = updated_cfg.map(|c| serde_json::to_string(&c).unwrap());
                        match json {
                            Some(json) => respond_json(conn, &json).await?,
                            None => respond_unavailable(conn).await?,
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to parse paused control: {}", e);
                        respond_bad_request(conn, "Bad Request").await?;
                    }
                }
            }

            (Method::Get, "/state") => {
                let state = {
                    let mut mc_opt = self.app_context.motor_controller.lock().unwrap();
                    mc_opt.as_mut().map(|mc| mc.get_current_state())
                };
                let json = state.map(|s| serde_json::to_string(&s).unwrap());
                match json {
                    Some(json) => respond_json(conn, &json).await?,
                    None => respond_unavailable(conn).await?,
                }
            }

            (Method::Get, "/pin-config") => {
                let config = self.app_context.storage_manager.lock().unwrap().get_pin_configuration().unwrap_or_default();
                let json = serde_json::to_string(&config).unwrap();
                respond_json(conn, &json).await?;
            }

            (Method::Post, "/pin-config") => {
                let mut body = [0u8; MAX_BODY_LEN];
                let Some(len) = read_body(conn, &mut body).await? else {
                    respond_bad_request(conn, "Request too large").await?;
                    return Ok(());
                };
                match serde_json::from_slice::<PinConfiguration>(&body[..len]) {
                    Ok(config) => {
                        if config.modbus_timeout_ms > 1000 {
                            respond_bad_request(conn, "modbus_timeout_ms must be 0..=1000").await?;
                            return Ok(());
                        }
                        if config.modbus_scan_delay_us > 200000 {
                            respond_bad_request(conn, "modbus_scan_delay_us must be 0..=200000").await?;
                            return Ok(());
                        }
                        let result = self.app_context.storage_manager.lock().unwrap().set_pin_configuration(&config);
                        match result {
                            Ok(()) => {
                                let json = serde_json::to_string(&config).unwrap();
                                respond_json(conn, &json).await?;
                            }
                            Err(e) => {
                                log::error!("Failed to save pin config: {}", e);
                                conn.initiate_response(500, Some("Internal Server Error"), &[CORS_ORIGIN]).await?;
                                conn.write_all("Failed to save pin config".as_bytes()).await?;
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to parse pin config: {}", e);
                        respond_bad_request(conn, "Bad Request").await?;
                    }
                }
            }

            (Method::Get, "/network-config") => {
                let config = self.app_context.storage_manager.lock().unwrap().get_network_configuration().unwrap_or_default();
                let json = serde_json::to_string(&config).unwrap();
                respond_json(conn, &json).await?;
            }

            (Method::Post, "/network-config") => {
                let mut body = [0u8; MAX_BODY_LEN];
                let Some(len) = read_body(conn, &mut body).await? else {
                    respond_bad_request(conn, "Request too large").await?;
                    return Ok(());
                };
                match serde_json::from_slice::<NetworkConfiguration>(&body[..len]) {
                    Ok(config) => {
                        let result = self.app_context.storage_manager.lock().unwrap().set_network_configuration(&config);
                        match result {
                            Ok(()) => {
                                let json = serde_json::to_string(&config).unwrap();
                                respond_json(conn, &json).await?;
                            }
                            Err(e) => {
                                log::error!("Failed to save network config: {}", e);
                                conn.initiate_response(500, Some("Internal Server Error"), &[CORS_ORIGIN]).await?;
                                conn.write_all("Failed to save network config".as_bytes()).await?;
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to parse network config: {}", e);
                        respond_bad_request(conn, "Bad Request").await?;
                    }
                }
            }

            (Method::Post, "/restart") => {
                conn.initiate_response(200, Some("OK"), &[CORS_ORIGIN]).await?;
                conn.write_all("Restarting device".as_bytes()).await?;
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    esp_idf_svc::hal::reset::restart();
                });
            }

            (Method::Get, "/ws/command") => {
                self.handle_ws_command(conn).await?;
            }

            _ => {
                conn.initiate_response(404, Some("Not Found"), &[CORS_ORIGIN]).await?;
            }
        }

        Ok(())
    }
}

impl HttpHandler {
    async fn handle_ws_command<T, const N: usize>(
        &self,
        conn: &mut Connection<'_, T, N>,
    ) -> Result<(), Error<T::Error>>
    where
        T: Read + Write + TcpSplit,
    {
        if !conn.is_ws_upgrade_request()? {
            conn.initiate_response(400, Some("Bad Request"), &[CORS_ORIGIN]).await?;
            conn.write_all("Expected WebSocket upgrade request".as_bytes()).await?;
            return Ok(());
        }

        let command_tx = {
            let mut mc_opt = self.app_context.motor_controller.lock().unwrap();
            mc_opt.as_mut().map(|mc| mc.get_command_sender())
        };
        let Some(command_tx) = command_tx else {
            log::error!("Motor controller not initialized");
            conn.initiate_response(503, Some("Service Unavailable"), &[CORS_ORIGIN]).await?;
            return Ok(());
        };

        let mut upgrade_buf = [0u8; MAX_BASE64_KEY_RESPONSE_LEN];
        conn.initiate_ws_upgrade_response(&mut upgrade_buf).await?;
        conn.complete().await?;

        let socket = conn.unbind()?;
        let mut frame_data = [0u8; 2048];
        let mut subscribed = false;
        let mut push_interval_ms = 500u64;
        let mut next_push = embassy_time::Instant::now() + embassy_time::Duration::from_millis(push_interval_ms);

        loop {
            if subscribed {
                let now = embassy_time::Instant::now();
                if now >= next_push {
                    next_push = now + embassy_time::Duration::from_millis(push_interval_ms);
                    let state = {
                        let mut mc_opt = self.app_context.motor_controller.lock().unwrap();
                        mc_opt.as_mut().map(|mc| mc.get_current_state())
                    };
                    if let Some(state) = state {
                        if send_ws_notification(&mut *socket, "state", &state).await.is_err() {
                            return Ok(());
                        }
                    }
                    continue;
                }
            }

            let header_res = if subscribed {
                match embassy_futures::select::select(
                    FrameHeader::recv(&mut *socket),
                    embassy_time::Timer::at(next_push),
                )
                .await
                {
                    embassy_futures::select::Either::First(res) => Some(res),
                    embassy_futures::select::Either::Second(_) => None,
                }
            } else {
                Some(FrameHeader::recv(&mut *socket).await)
            };

            let header = match header_res {
                Some(Ok(header)) => header,
                Some(Err(e)) => {
                    log::info!("WebSocket closed: {:?}", e);
                    return Ok(());
                }
                None => continue,
            };

            let payload = match header.recv_payload(&mut *socket, &mut frame_data).await {
                Ok(payload) => payload,
                Err(e) => {
                    log::info!("WebSocket payload error: {:?}", e);
                    return Ok(());
                }
            };

            match header.frame_type {
                FrameType::Text(false) | FrameType::Binary(false) => {
                    let request = match serde_json::from_slice::<WsMessage>(payload) {
                        Ok(request) => request,
                        Err(e) => {
                            log::error!("Failed to parse command ({}): {}", e, String::from_utf8_lossy(payload));
                            let _ = send_rpc_error(&mut *socket, None, -32700, "Parse error").await;
                            continue;
                        }
                    };

                    match request.cmd.as_str() {
                        "append-waypoints" | "append_waypoints" => {
                            match request.parse_params::<WaypointsInput>() {
                                Ok(input) => {
                                    let (waypoints, _) = input.into_parts();
                                    let cmd = MotionCommand::AppendWaypoints(waypoints);
                                    if command_tx.lock().unwrap().enqueue(cmd).is_err() {
                                        log::warn!("Command queue full, dropping command");
                                    }
                                    if request.id.is_some()
                                        && send_rpc_result(&mut *socket, request.id.as_ref(), &"ok").await.is_err()
                                    {
                                        return Ok(());
                                    }
                                }
                                Err(e) => {
                                    log::error!("Failed to parse append-waypoints params: {}", e);
                                    if request.id.is_some()
                                        && send_rpc_error(&mut *socket, request.id.as_ref(), -32602, "Invalid params")
                                            .await
                                            .is_err()
                                    {
                                        return Ok(());
                                    }
                                }
                            }
                        }
                        "set-waypoints" | "set_waypoints" => {
                            match request.parse_params::<WaypointsInput>() {
                                Ok(input) => {
                                    let (waypoints, reset_timestamp) = input.into_parts();
                                    let cmd = MotionCommand::SetWaypoints { waypoints, reset_timestamp };
                                    if command_tx.lock().unwrap().enqueue(cmd).is_err() {
                                        log::warn!("Command queue full, dropping command");
                                    }
                                    if request.id.is_some()
                                        && send_rpc_result(&mut *socket, request.id.as_ref(), &"ok").await.is_err()
                                    {
                                        return Ok(());
                                    }
                                }
                                Err(e) => {
                                    log::error!("Failed to parse set-waypoints params: {}", e);
                                    if request.id.is_some()
                                        && send_rpc_error(&mut *socket, request.id.as_ref(), -32602, "Invalid params")
                                            .await
                                            .is_err()
                                    {
                                        return Ok(());
                                    }
                                }
                            }
                        }
                        "reset_timestamp" | "reset-timestamp" => {
                            let cmd = MotionCommand::ResetTimestamp;
                            if command_tx.lock().unwrap().enqueue(cmd).is_err() {
                                log::warn!("Command queue full, dropping command");
                            }
                            if request.id.is_some()
                                && send_rpc_result(&mut *socket, request.id.as_ref(), &"ok").await.is_err()
                            {
                                return Ok(());
                            }
                        }
                        "status" => {
                            let status = {
                                let mc_opt = self.app_context.motor_controller.lock().unwrap();
                                mc_opt.as_ref().map(|mc| mc.get_stream_status())
                            };
                            let Some(status) = status else {
                                log::error!("Motor controller not initialized");
                                if request.id.is_some() {
                                    let _ = send_rpc_error(&mut *socket, request.id.as_ref(), -32603, "Motor controller not initialized").await;
                                }
                                continue;
                            };
                            if request.id.is_some() {
                                if send_rpc_result(&mut *socket, request.id.as_ref(), &status).await.is_err() {
                                    return Ok(());
                                }
                            } else {
                                let json = serde_json::to_string(&status).unwrap();
                                if send_ws_text(&mut *socket, &json).await.is_err() {
                                    return Ok(());
                                }
                            }
                        }
                        "ping" => {
                            if send_rpc_result(&mut *socket, request.id.as_ref(), &"pong").await.is_err() {
                                return Ok(());
                            }
                        }
                        "get_state" | "get-state" => {
                            let state = {
                                let mut mc_opt = self.app_context.motor_controller.lock().unwrap();
                                mc_opt.as_mut().map(|mc| mc.get_current_state())
                            };
                            if let Some(state) = state {
                                if send_rpc_result(&mut *socket, request.id.as_ref(), &state).await.is_err() {
                                    return Ok(());
                                }
                            } else if send_rpc_error(&mut *socket, request.id.as_ref(), -32603, "Motor controller not initialized").await.is_err() {
                                return Ok(());
                            }
                        }
                        "set_config" | "set-config" => {
                            match request.parse_params::<MotorControllerConfig>() {
                                Ok(config) => {
                                    let applied = {
                                        let mut mc_opt = self.app_context.motor_controller.lock().unwrap();
                                        if let Some(mc) = mc_opt.as_mut() {
                                            mc.set_config(config.clone()).is_ok()
                                        } else {
                                            false
                                        }
                                    };
                                    if applied {
                                        if send_rpc_result(&mut *socket, request.id.as_ref(), &config).await.is_err() {
                                            return Ok(());
                                        }
                                    } else if send_rpc_error(&mut *socket, request.id.as_ref(), -32603, "Failed to apply config").await.is_err() {
                                        return Ok(());
                                    }
                                }
                                Err(e) => {
                                    log::error!("Failed to parse config for set_config: {}", e);
                                    if send_rpc_error(&mut *socket, request.id.as_ref(), -32602, "Invalid params").await.is_err() {
                                        return Ok(());
                                    }
                                }
                            }
                        }
                        "subscribe_state" | "subscribe-state" => {
                            let params = request.parse_params::<SubscribeParams>().unwrap_or_default();
                            let interval = params.interval_ms.unwrap_or(33).clamp(20, 60000);
                            push_interval_ms = interval;
                            subscribed = true;
                            next_push = embassy_time::Instant::now() + embassy_time::Duration::from_millis(push_interval_ms);
                            if send_rpc_result(&mut *socket, request.id.as_ref(), &"subscribed").await.is_err() {
                                return Ok(());
                            }
                        }
                        "unsubscribe_state" | "unsubscribe-state" => {
                            subscribed = false;
                            if send_rpc_result(&mut *socket, request.id.as_ref(), &"unsubscribed").await.is_err() {
                                return Ok(());
                            }
                        }
                        "get_network_config" | "get-network-config" => {
                            let config = self.app_context.storage_manager.lock().unwrap().get_network_configuration().unwrap_or_default();
                            if send_rpc_result(&mut *socket, request.id.as_ref(), &config).await.is_err() {
                                return Ok(());
                            }
                        }
                        "set_network_config" | "set-network-config" => {
                            match request.parse_params::<NetworkConfiguration>() {
                                Ok(config) => {
                                    if self.app_context.storage_manager.lock().unwrap().set_network_configuration(&config).is_ok() {
                                        if send_rpc_result(&mut *socket, request.id.as_ref(), &config).await.is_err() {
                                            return Ok(());
                                        }
                                    } else if send_rpc_error(&mut *socket, request.id.as_ref(), -32603, "Failed to apply config").await.is_err() {
                                        return Ok(());
                                    }
                                }
                                Err(e) => {
                                    log::error!("Failed to parse config for set-network-config: {}", e);
                                    if send_rpc_error(&mut *socket, request.id.as_ref(), -32602, "Invalid params").await.is_err() {
                                        return Ok(());
                                    }
                                }
                            }
                        }
                        other => {
                            log::warn!("Unknown WS command: {}", other);
                            if send_rpc_error(&mut *socket, request.id.as_ref(), -32601, "Method not found").await.is_err() {
                                return Ok(());
                            }
                        }
                    }
                }
                FrameType::Ping => {
                    let pong = FrameHeader {
                        frame_type: FrameType::Pong,
                        payload_len: payload.len() as u64,
                        mask_key: None,
                    };
                    if pong.send(&mut *socket).await.is_err()
                        || pong.send_payload(&mut *socket, payload).await.is_err()
                    {
                        return Ok(());
                    }
                }
                FrameType::Close => {
                    log::info!("WebSocket close requested");
                    return Ok(());
                }
                _ => (),
            }
        }
    }
}

async fn cors_preflight<T, const N: usize>(
    conn: &mut Connection<'_, T, N>,
    methods: &str,
) -> Result<(), Error<T::Error>>
where
    T: Read + Write + TcpSplit,
{
    conn.initiate_response(
        200,
        Some("OK"),
        &[
            CORS_ORIGIN,
            ("Access-Control-Allow-Methods", methods),
            ("Access-Control-Allow-Headers", "*"),
        ],
    )
    .await
}

async fn respond_json<T, const N: usize>(
    conn: &mut Connection<'_, T, N>,
    json: &str,
) -> Result<(), Error<T::Error>>
where
    T: Read + Write + TcpSplit,
{
    conn.initiate_response(
        200,
        Some("OK"),
        &[CORS_ORIGIN, CONNECTION_CLOSE, ("Content-Type", "application/json")],
    )
    .await?;
    conn.write_all(json.as_bytes()).await
}

async fn respond_unavailable<T, const N: usize>(
    conn: &mut Connection<'_, T, N>,
) -> Result<(), Error<T::Error>>
where
    T: Read + Write + TcpSplit,
{
    conn.initiate_response(503, Some("Service Unavailable"), &[CORS_ORIGIN, CONNECTION_CLOSE])
        .await?;
    conn.write_all("Motor controller not initialized".as_bytes()).await
}

async fn respond_bad_request<T, const N: usize>(
    conn: &mut Connection<'_, T, N>,
    message: &str,
) -> Result<(), Error<T::Error>>
where
    T: Read + Write + TcpSplit,
{
    conn.initiate_response(400, Some("Bad Request"), &[CORS_ORIGIN, CONNECTION_CLOSE])
        .await?;
    conn.write_all(message.as_bytes()).await
}

// Reads the request body into `buf`. Returns `None` if the body exceeds the buffer.
async fn read_body<T, const N: usize>(
    conn: &mut Connection<'_, T, N>,
    buf: &mut [u8],
) -> Result<Option<usize>, Error<T::Error>>
where
    T: Read + Write + TcpSplit,
{
    let mut len = 0;
    loop {
        if len == buf.len() {
            // Check whether there is more data than fits in the buffer
            let mut probe = [0u8; 1];
            let n = conn.read(&mut probe).await?;
            if n == 0 {
                return Ok(Some(len));
            }
            return Ok(None);
        }
        let n = conn.read(&mut buf[len..]).await?;
        if n == 0 {
            return Ok(Some(len));
        }
        len += n;
    }
}

#[cfg(test)]
#[allow(unused_imports, dead_code)]
mod tests {
    use super::{RpcResponse, SubscribeParams, WaypointsInput, WsMessage};

    #[test]
    fn test_ws_message_append_waypoints_jsonrpc_style() {
        let json = r#"{"jsonrpc":"2.0","method":"append-waypoints","params":[{"ts":100,"pos":0.5,"vel":0.1},{"ts":150,"pos":0.7}],"id":1}"#;
        let msg: WsMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.cmd, "append-waypoints");
        assert_eq!(msg.id, Some(serde_json::Value::from(1)));

        let input: WaypointsInput = msg.parse_params().unwrap();
        let (waypoints, reset) = input.into_parts();
        assert_eq!(waypoints.len(), 2);
        assert!(!reset);
        assert_eq!(waypoints[0].ts, 100);
        assert_eq!(waypoints[0].pos, 0.5);
        assert_eq!(waypoints[0].vel, Some(0.1));
        assert_eq!(waypoints[1].ts, 150);
        assert_eq!(waypoints[1].pos, 0.7);
        assert_eq!(waypoints[1].vel, None);
    }

    #[test]
    fn test_ws_message_set_waypoints_flat_style() {
        let json = r#"{"cmd":"set-waypoints","waypoints":[{"ts":200,"pos":0.8}]}"#;
        let msg: WsMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.cmd, "set-waypoints");
        assert_eq!(msg.id, None);

        let input: WaypointsInput = msg.parse_params().unwrap();
        let (waypoints, reset) = input.into_parts();
        assert_eq!(waypoints.len(), 1);
        assert!(!reset);
        assert_eq!(waypoints[0].ts, 200);
        assert_eq!(waypoints[0].pos, 0.8);
        assert_eq!(waypoints[0].vel, None);
    }

    #[test]
    fn test_ws_message_set_waypoints_with_reset_timestamp() {
        let json = r#"{"jsonrpc":"2.0","method":"set-waypoints","params":{"waypoints":[{"ts":300,"pos":0.1}],"reset-timestamp":true},"id":1}"#;
        let msg: WsMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.cmd, "set-waypoints");

        let input: WaypointsInput = msg.parse_params().unwrap();
        let (waypoints, reset) = input.into_parts();
        assert_eq!(waypoints.len(), 1);
        assert!(reset);
        assert_eq!(waypoints[0].ts, 300);
    }

    #[test]
    fn test_ws_message_subscribe_params() {
        let json1 = r#"{"method":"subscribe-state","params":{"interval_ms":250},"id":"sub1"}"#;
        let msg1: WsMessage = serde_json::from_str(json1).unwrap();
        assert_eq!(msg1.cmd, "subscribe-state");
        let p1: SubscribeParams = msg1.parse_params().unwrap();
        assert_eq!(p1.interval_ms, Some(250));

        let json2 = r#"{"cmd":"subscribe_state"}"#;
        let msg2: WsMessage = serde_json::from_str(json2).unwrap();
        let p2: SubscribeParams = msg2.parse_params().unwrap_or_default();
        assert_eq!(p2.interval_ms, None);
    }

    #[test]
    fn test_rpc_response_serialization() {
        let resp = RpcResponse {
            jsonrpc: "2.0",
            id: serde_json::Value::from(1),
            result: Some(&"pong"),
            error: None,
        };
        let serialized = serde_json::to_string(&resp).unwrap();
        assert_eq!(serialized, r#"{"jsonrpc":"2.0","id":1,"result":"pong"}"#);
    }
}

