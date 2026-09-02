use alloc::string::String;
use alloc::vec::Vec;
use core::net::{Ipv4Addr, SocketAddr};

use edge_http::io::server::Connection;
use edge_http::io::{Body, Error as HttpError};
use edge_http::{Method, DEFAULT_MAX_HEADERS_COUNT};
use edge_nal::{Close, TcpBind, TcpShutdown, TcpSplit};
use edge_nal_embassy::{Tcp, TcpBuffers, TcpSocket};
use edge_ws::io::{recv as ws_recv, send as ws_send};
use edge_ws::FrameType;
use embassy_executor::Spawner;
use embassy_futures::select::{select, Either as SelectEither};
use embassy_net::Stack;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embassy_sync::semaphore::{GreedySemaphore, Semaphore};
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};
use embedded_io_async::{Read, Write};
use serde::{Deserialize, Serialize};
use static_cell::StaticCell;

use crate::context::AppContext;
use crate::modbus_relay;
use crate::modbus_rtu::{self, InjectJunkMode};
use crate::motion::MotorControllerConfig;
use crate::rpc::{self, RpcAction};
use crate::storage::{NetworkConfiguration, PinConfiguration};

pub use ossm_core::{PausedControl, RpcRequest, SubscribeParams, WaypointsInput};

const APP_HTML_GZ: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/index.html.gz"));

const NO_GZIP_HTML: &[u8] = b"\
<!DOCTYPE html><html><head><meta charset=\"utf-8\">\
<title>OSSM</title></head><body style=\"font-family:sans-serif;text-align:center;padding:2em\">\
<h1>Browser Not Supported</h1>\
<p>This device serves a compressed interface that requires <code>Accept-Encoding: gzip</code>.</p>\
</body></html>";

pub const HTTP_BUFFER_SIZE: usize = 2048;
pub const TCP_BUFFER_SIZE: usize = 1024;
pub const WEB_TASK_POOL_SIZE: usize = 1;
const TCP_POOL_SIZE: usize = 10;
const Q_ACCEPTORS: usize = 4;
const WS_MAX: usize = 3;
const MODBUS_WS_MAX: usize = 2;
const MODBUS_TCP_POOL: usize = 2;
const MODBUS_TCP_BUFFER_SIZE: usize = 512;
const MAX_BODY_LEN: usize = 1024;

type HttpConn<'a> = Connection<'a, TcpSocket<'static>, DEFAULT_MAX_HEADERS_COUNT>;

struct SyncTcpSocket(TcpSocket<'static>);

// SAFETY: OSSM runs HTTP on a single Embassy executor thread/core.
unsafe impl Send for SyncTcpSocket {}
unsafe impl Sync for SyncTcpSocket {}

impl SyncTcpSocket {
    fn new(socket: TcpSocket<'_>) -> Self {
        // SAFETY: Embassy stack and TCP buffer pool live in StaticCell for the firmware lifetime.
        let socket = unsafe { core::mem::transmute::<TcpSocket<'_>, TcpSocket<'static>>(socket) };
        Self(socket)
    }

    fn into_inner(self) -> TcpSocket<'static> {
        self.0
    }
}

static TCP_BUFFERS: StaticCell<TcpBuffers<TCP_POOL_SIZE, TCP_BUFFER_SIZE, TCP_BUFFER_SIZE>> =
    StaticCell::new();
static TCP_FACTORY: StaticCell<Tcp<'static>> = StaticCell::new();
static MODBUS_TCP_BUFFERS: StaticCell<
    TcpBuffers<MODBUS_TCP_POOL, MODBUS_TCP_BUFFER_SIZE, MODBUS_TCP_BUFFER_SIZE>,
> = StaticCell::new();
static MODBUS_TCP_FACTORY: StaticCell<Tcp<'static>> = StaticCell::new();
static SOCKET_QUEUE: Channel<CriticalSectionRawMutex, (SyncTcpSocket, usize), Q_ACCEPTORS> =
    Channel::new();
static WS_SOCKET_CH: Channel<CriticalSectionRawMutex, (SyncTcpSocket, usize), WS_MAX> =
    Channel::new();
static MODBUS_WS_CH: Channel<CriticalSectionRawMutex, (SyncTcpSocket, usize), MODBUS_WS_MAX> =
    Channel::new();
static REST_GATE: Mutex<CriticalSectionRawMutex, ()> = Mutex::new(());
static WS_SLOTS: GreedySemaphore<CriticalSectionRawMutex> = GreedySemaphore::new(WS_MAX);
static MODBUS_WS_SLOTS: GreedySemaphore<CriticalSectionRawMutex> =
    GreedySemaphore::new(MODBUS_WS_MAX);
static ACCEPT_SIGNALS: [Signal<CriticalSectionRawMutex, ()>; Q_ACCEPTORS] =
    [const { Signal::new() }; Q_ACCEPTORS];
/// Signalled once the HTTP listener is bound and accept loops are about to run.
static HTTP_READY: Signal<CriticalSectionRawMutex, ()> = Signal::new();

fn is_relay_mode(ctx: AppContext) -> bool {
    modbus_relay::is_active() || ctx.storage.pin().is_rtu_relay()
}

fn normalize_path(path: &str) -> &str {
    path.split('?').next().unwrap_or(path)
}

fn header_value<'a, const N: usize>(
    headers: &'a edge_http::RequestHeaders<'a, N>,
    name: &str,
) -> Option<&'a str> {
    headers
        .headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v)
}

fn accepts_gzip<const N: usize>(headers: &edge_http::RequestHeaders<'_, N>) -> bool {
    header_value(headers, "accept-encoding").is_some_and(|v| v.contains("gzip"))
}

async fn write_all_conn(
    conn: &mut HttpConn<'_>,
    data: &[u8],
) -> Result<(), HttpError<edge_nal_embassy::TcpError>> {
    let mut offset = 0;
    while offset < data.len() {
        let written = conn.write(&data[offset..]).await?;
        if written == 0 {
            break;
        }
        offset += written;
    }
    conn.flush().await?;
    Ok(())
}

async fn send_json(
    conn: &mut HttpConn<'_>,
    status: u16,
    reason: &'static str,
    body: &str,
) -> Result<(), HttpError<edge_nal_embassy::TcpError>> {
    conn.initiate_response(
        status,
        Some(reason),
        &[
            ("Content-Type", "application/json"),
            ("Access-Control-Allow-Origin", "*"),
            ("Connection", "close"),
        ],
    )
    .await?;
    write_all_conn(conn, body.as_bytes()).await
}

async fn send_json_obj<T: serde::Serialize>(
    conn: &mut HttpConn<'_>,
    status: u16,
    reason: &'static str,
    obj: &T,
) -> Result<(), HttpError<edge_nal_embassy::TcpError>> {
    let mut lease = crate::buffers::NET_BUFFER_POOL.acquire().await;
    if let Ok(len) = serde_json_core::to_slice(obj, &mut *lease) {
        conn.initiate_response(
            status,
            Some(reason),
            &[
                ("Content-Type", "application/json"),
                ("Access-Control-Allow-Origin", "*"),
                ("Connection", "close"),
            ],
        )
        .await?;
        write_all_conn(conn, &lease[..len]).await
    } else {
        send_json(conn, 500, "Internal Server Error", "Serialization failed").await
    }
}

async fn cors_preflight(
    conn: &mut HttpConn<'_>,
    methods: &'static str,
) -> Result<(), HttpError<edge_nal_embassy::TcpError>> {
    conn.initiate_response(
        200,
        Some("OK"),
        &[
            ("Access-Control-Allow-Origin", "*"),
            ("Access-Control-Allow-Methods", methods),
            ("Access-Control-Allow-Headers", "Content-Type"),
            ("Connection", "close"),
        ],
    )
    .await
}

async fn read_body_limited(
    body: &mut Body<'_, TcpSocket<'static>>,
    max_len: usize,
) -> Result<Vec<u8>, HttpError<edge_nal_embassy::TcpError>> {
    let mut out = Vec::new();
    let mut scratch = [0u8; 256];
    loop {
        let read = body.read(&mut scratch).await?;
        if read == 0 {
            break;
        }
        if out.len() + read > max_len {
            return Err(HttpError::TooLongBody);
        }
        out.extend_from_slice(&scratch[..read]);
    }
    Ok(out)
}

async fn finish_connection(conn: &mut HttpConn<'_>) {
    let needs_close = conn.needs_close();
    let _ = conn.complete().await;
    if needs_close {
        if let Ok(socket) = conn.unbind() {
            let _ = socket.close(Close::Both).await;
        }
    }
}

async fn serve_html(conn: &mut HttpConn<'_>) -> Result<(), HttpError<edge_nal_embassy::TcpError>> {
    let headers = conn.headers()?;
    if accepts_gzip(headers) {
        conn.initiate_response(
            200,
            Some("OK"),
            &[
                ("Content-Type", "text/html"),
                ("Content-Encoding", "gzip"),
                ("Access-Control-Allow-Origin", "*"),
            ],
        )
        .await?;
        write_all_conn(conn, APP_HTML_GZ).await
    } else {
        conn.initiate_response(
            200,
            Some("OK"),
            &[
                ("Content-Type", "text/html"),
                ("Access-Control-Allow-Origin", "*"),
                ("Connection", "close"),
            ],
        )
        .await?;
        write_all_conn(conn, NO_GZIP_HTML).await
    }
}

async fn get_config(
    conn: &mut HttpConn<'_>,
    ctx: AppContext,
) -> Result<(), HttpError<edge_nal_embassy::TcpError>> {
    let config = ctx.load_snapshot().config.clone();
    send_json_obj(conn, 200, "OK", &config).await
}

async fn post_config(
    conn: &mut HttpConn<'_>,
    ctx: AppContext,
    body: Vec<u8>,
) -> Result<(), HttpError<edge_nal_embassy::TcpError>> {
    if is_relay_mode(ctx) {
        return send_json(conn, 409, "Conflict", "rtu_relay mode").await;
    }
    if body.len() > 1024 {
        return send_json(conn, 400, "Bad Request", "Request too large").await;
    }
    let config_res = serde_json::from_slice::<MotorControllerConfig>(&body);

    match config_res {
        Ok(config) => match ctx.try_enqueue_config(config).await {
            Ok(applied) => send_json_obj(conn, 200, "OK", &applied).await,
            Err("Stale causal version") => {
                send_json(conn, 409, "Conflict", "Stale causal version").await
            }
            Err(msg) => send_json(conn, 503, "Service Unavailable", msg).await,
        },
        Err(_) => send_json(conn, 400, "Bad Request", "Bad Request").await,
    }
}

async fn get_state(
    conn: &mut HttpConn<'_>,
    ctx: AppContext,
) -> Result<(), HttpError<edge_nal_embassy::TcpError>> {
    let state = ctx.load_snapshot();
    send_json_obj(conn, 200, "OK", state.as_ref()).await
}

async fn post_paused(
    conn: &mut HttpConn<'_>,
    ctx: AppContext,
    body: Vec<u8>,
) -> Result<(), HttpError<edge_nal_embassy::TcpError>> {
    if is_relay_mode(ctx) {
        return send_json(conn, 409, "Conflict", "rtu_relay mode").await;
    }
    if body.len() > MAX_BODY_LEN {
        return send_json(conn, 400, "Bad Request", "Request too large").await;
    }
    match serde_json::from_slice::<PausedControl>(&body) {
        Ok(control) => {
            let mut config = ctx.load_snapshot().config.clone();
            if let Some(paused) = control.paused {
                config.paused = paused;
            }
            if let Some(position) = control.position.or(control.paused_position) {
                config.paused_position = position.clamp(0.0, 1.0);
            }
            if let Some(adjust) = control.adjust {
                config.paused_position = (config.paused_position + adjust).clamp(0.0, 1.0);
            }
            match ctx.try_enqueue_config(config).await {
                Ok(applied) => send_json_obj(conn, 200, "OK", &applied).await,
                Err("Stale causal version") => {
                    send_json(conn, 409, "Conflict", "Stale causal version").await
                }
                Err(msg) => send_json(conn, 503, "Service Unavailable", msg).await,
            }
        }
        Err(_) => send_json(conn, 400, "Bad Request", "Bad Request").await,
    }
}

async fn get_pin_config(
    conn: &mut HttpConn<'_>,
    ctx: AppContext,
) -> Result<(), HttpError<edge_nal_embassy::TcpError>> {
    let config = ctx.storage.pin();
    send_json_obj(conn, 200, "OK", &config).await
}

async fn post_pin_config(
    conn: &mut HttpConn<'_>,
    ctx: AppContext,
    body: Vec<u8>,
) -> Result<(), HttpError<edge_nal_embassy::TcpError>> {
    if body.len() > MAX_BODY_LEN {
        return send_json(conn, 400, "Bad Request", "Request too large").await;
    }
    match serde_json::from_slice::<PinConfiguration>(&body) {
        Ok(config) => {
            if config.modbus_timeout_ms > 1000
                || config.modbus_scan_delay_us > 200_000
                || config.modbus_inter_frame_delay_us > 200_000
            {
                return send_json(conn, 400, "Bad Request", "Invalid pin config").await;
            }
            ctx.storage.set_pin(config.clone());
            send_json_obj(conn, 200, "OK", &config).await
        }
        Err(_) => send_json(conn, 400, "Bad Request", "Bad Request").await,
    }
}

async fn get_network_config(
    conn: &mut HttpConn<'_>,
    ctx: AppContext,
) -> Result<(), HttpError<edge_nal_embassy::TcpError>> {
    let config = ctx.storage.net();
    send_json_obj(conn, 200, "OK", &config).await
}

async fn post_network_config(
    conn: &mut HttpConn<'_>,
    ctx: AppContext,
    body: Vec<u8>,
) -> Result<(), HttpError<edge_nal_embassy::TcpError>> {
    if body.len() > MAX_BODY_LEN {
        return send_json(conn, 400, "Bad Request", "Request too large").await;
    }
    match serde_json::from_slice::<NetworkConfiguration>(&body) {
        Ok(config) => {
            ctx.storage.set_net(config.clone());
            send_json_obj(conn, 200, "OK", &config).await
        }
        Err(_) => send_json(conn, 400, "Bad Request", "Bad Request").await,
    }
}

async fn post_restart(
    conn: &mut HttpConn<'_>,
) -> Result<(), HttpError<edge_nal_embassy::TcpError>> {
    let spawner = unsafe { embassy_executor::Spawner::for_current_executor().await };
    spawner.spawn(delayed_reset_task().unwrap());
    send_json(conn, 200, "OK", "{\"ok\":true}").await
}

#[derive(Deserialize)]
struct ModbusInjectBody {
    mode: alloc::string::String,
    #[serde(default)]
    nbytes: u8,
}

#[derive(Serialize)]
struct ModbusInjectResponse {
    mode: &'static str,
    nbytes: u8,
}

async fn get_modbus_inject(
    conn: &mut HttpConn<'_>,
    ctx: AppContext,
) -> Result<(), HttpError<edge_nal_embassy::TcpError>> {
    if !ctx.storage.pin().modbus_debug {
        return send_json(conn, 409, "Conflict", "modbus_debug disabled").await;
    }
    let (mode, nbytes) = modbus_rtu::get_inject_junk();
    send_json_obj(
        conn,
        200,
        "OK",
        &ModbusInjectResponse {
            mode: mode.as_str(),
            nbytes,
        },
    )
    .await
}

async fn post_modbus_inject(
    conn: &mut HttpConn<'_>,
    ctx: AppContext,
    body: Vec<u8>,
) -> Result<(), HttpError<edge_nal_embassy::TcpError>> {
    if !ctx.storage.pin().modbus_debug {
        return send_json(conn, 409, "Conflict", "modbus_debug disabled").await;
    }
    if body.len() > MAX_BODY_LEN {
        return send_json(conn, 400, "Bad Request", "Request too large").await;
    }
    match serde_json::from_slice::<ModbusInjectBody>(&body) {
        Ok(req) => {
            let mode = match req.mode.as_str() {
                "off" => InjectJunkMode::Off,
                "leading" => InjectJunkMode::Leading,
                "trailing" => InjectJunkMode::Trailing,
                "both" => InjectJunkMode::Both,
                _ => return send_json(conn, 400, "Bad Request", "bad mode").await,
            };
            if req.nbytes > 64 {
                return send_json(conn, 400, "Bad Request", "nbytes too large").await;
            }
            modbus_rtu::set_inject_junk(mode, req.nbytes);
            send_json_obj(
                conn,
                200,
                "OK",
                &ModbusInjectResponse {
                    mode: mode.as_str(),
                    nbytes: req.nbytes,
                },
            )
            .await
        }
        Err(_) => send_json(conn, 400, "Bad Request", "Bad Request").await,
    }
}

#[embassy_executor::task]
async fn delayed_reset_task() {
    Timer::after(Duration::from_millis(100)).await;
    esp_hal::system::software_reset();
}

async fn handle_rest(
    conn: &mut HttpConn<'_>,
    ctx: AppContext,
) -> Result<(), HttpError<edge_nal_embassy::TcpError>> {
    let (method, path) = {
        let headers = conn.headers()?;
        (headers.method, normalize_path(headers.path))
    };

    match (method, path) {
        (Method::Get, "/") => serve_html(conn).await,
        (Method::Get, "/config") => get_config(conn, ctx).await,
        (Method::Post, "/config") => {
            let (_, body) = conn.split();
            let payload = read_body_limited(body, MAX_BODY_LEN).await?;
            let _gate = REST_GATE.lock().await;
            post_config(conn, ctx, payload).await
        }
        (Method::Options, "/config") => cors_preflight(conn, "GET, POST, OPTIONS").await,
        (Method::Get, "/state") => get_state(conn, ctx).await,
        (Method::Options, "/state") => cors_preflight(conn, "GET, OPTIONS").await,
        (Method::Post, "/paused") => {
            let (_, body) = conn.split();
            let payload = read_body_limited(body, MAX_BODY_LEN).await?;
            let _gate = REST_GATE.lock().await;
            post_paused(conn, ctx, payload).await
        }
        (Method::Options, "/paused") => cors_preflight(conn, "POST, OPTIONS").await,
        (Method::Get, "/pin-config") => get_pin_config(conn, ctx).await,
        (Method::Post, "/pin-config") => {
            let (_, body) = conn.split();
            let payload = read_body_limited(body, MAX_BODY_LEN).await?;
            let _gate = REST_GATE.lock().await;
            post_pin_config(conn, ctx, payload).await
        }
        (Method::Options, "/pin-config") => cors_preflight(conn, "GET, POST, OPTIONS").await,
        (Method::Get, "/network-config") => get_network_config(conn, ctx).await,
        (Method::Post, "/network-config") => {
            let (_, body) = conn.split();
            let payload = read_body_limited(body, MAX_BODY_LEN).await?;
            let _gate = REST_GATE.lock().await;
            post_network_config(conn, ctx, payload).await
        }
        (Method::Options, "/network-config") => cors_preflight(conn, "GET, POST, OPTIONS").await,
        (Method::Post, "/restart") => post_restart(conn).await,
        (Method::Options, "/restart") => cors_preflight(conn, "POST, OPTIONS").await,
        (Method::Get, "/modbus-inject") => get_modbus_inject(conn, ctx).await,
        (Method::Post, "/modbus-inject") => {
            let (_, body) = conn.split();
            let payload = read_body_limited(body, MAX_BODY_LEN).await?;
            post_modbus_inject(conn, ctx, payload).await
        }
        (Method::Options, "/modbus-inject") => cors_preflight(conn, "GET, POST, OPTIONS").await,
        _ => {
            conn.initiate_response(404, Some("Not Found"), &[("Connection", "close")])
                .await
        }
    }
}

fn extract_unbound_socket(conn: HttpConn<'_>) -> TcpSocket<'static> {
    match conn {
        Connection::Unbound(socket) => socket,
        _ => unreachable!(),
    }
}

async fn process_connection(socket: TcpSocket<'static>, acceptor_id: usize, ctx: AppContext) {
    let mut http_buf = [0u8; HTTP_BUFFER_SIZE];
    let mut conn = match Connection::new(&mut http_buf, socket).await {
        Ok(conn) => conn,
        Err(_) => return,
    };

    let ws_path = conn.headers().ok().map(|h| normalize_path(h.path));
    let is_ws = conn.is_ws_upgrade_request().unwrap_or(false);
    let is_command_ws = is_ws && ws_path == Some("/ws/command");
    let is_modbus_ws = is_ws && ws_path == Some("/ws/modbus");

    if is_modbus_ws {
        if !is_relay_mode(ctx) {
            let _ = conn
                .initiate_response(
                    403,
                    Some("Forbidden"),
                    &[("Connection", "close"), ("Content-Type", "text/plain")],
                )
                .await;
            finish_connection(&mut conn).await;
            return;
        }
        if MODBUS_WS_SLOTS.try_acquire(1).is_none() {
            let _ = conn
                .initiate_response(
                    503,
                    Some("Service Unavailable"),
                    &[("Connection", "close"), ("Content-Type", "text/plain")],
                )
                .await;
            finish_connection(&mut conn).await;
            return;
        }

        let mut key_buf = [0u8; edge_http::ws::MAX_BASE64_KEY_RESPONSE_LEN];
        if conn
            .initiate_ws_upgrade_response(&mut key_buf)
            .await
            .is_err()
        {
            MODBUS_WS_SLOTS.release(1);
            finish_connection(&mut conn).await;
            return;
        }
        if conn.complete().await.is_err() {
            MODBUS_WS_SLOTS.release(1);
            finish_connection(&mut conn).await;
            return;
        }
        let _ = conn.unbind();
        let socket = extract_unbound_socket(conn);
        match MODBUS_WS_CH.try_send((SyncTcpSocket::new(socket), acceptor_id)) {
            Ok(()) => {}
            Err(embassy_sync::channel::TrySendError::Full((wrapped, _))) => {
                MODBUS_WS_SLOTS.release(1);
                let mut socket = wrapped.into_inner();
                let _ = socket.close(Close::Both).await;
            }
        }
        return;
    }

    if is_command_ws {
        if WS_SLOTS.try_acquire(1).is_none() {
            let _ = conn
                .initiate_response(
                    503,
                    Some("Service Unavailable"),
                    &[("Connection", "close"), ("Content-Type", "text/plain")],
                )
                .await;
            finish_connection(&mut conn).await;
            return;
        }

        let mut key_buf = [0u8; edge_http::ws::MAX_BASE64_KEY_RESPONSE_LEN];
        if conn
            .initiate_ws_upgrade_response(&mut key_buf)
            .await
            .is_err()
        {
            WS_SLOTS.release(1);
            finish_connection(&mut conn).await;
            return;
        }
        if conn.complete().await.is_err() {
            WS_SLOTS.release(1);
            finish_connection(&mut conn).await;
            return;
        }
        let _ = conn.unbind();
        let socket = extract_unbound_socket(conn);
        match WS_SOCKET_CH.try_send((SyncTcpSocket::new(socket), acceptor_id)) {
            Ok(()) => {}
            Err(embassy_sync::channel::TrySendError::Full((wrapped, _))) => {
                WS_SLOTS.release(1);
                let mut socket = wrapped.into_inner();
                let _ = socket.close(Close::Both).await;
            }
        }
        return;
    }

    // REST handlers that mutate state take REST_GATE themselves. Holding the
    // gate across the entire request (including slow gzip HTML / TCP writes)
    // starved acceptors and made port 80 look dead under burst load.
    if handle_rest(&mut conn, ctx).await.is_err() {
        let _ = conn.complete_err("INTERNAL ERROR").await;
    }
    finish_connection(&mut conn).await;
}

enum WsTxMsg {
    Text(String),
    Pong(String),
    Subscribe {
        id: serde_json::Value,
        interval_ms: u64,
    },
    Unsubscribe {
        id: serde_json::Value,
    },
    Restart {
        id: serde_json::Value,
    },
    Close,
}

async fn run_ws_session(mut socket: TcpSocket<'static>, ctx: AppContext) {
    let (mut rx, mut tx) = socket.split();
    let tx_ch = Channel::<CriticalSectionRawMutex, WsTxMsg, 2>::new();

    let rx_task = async {
        let mut buf = [0u8; HTTP_BUFFER_SIZE];
        loop {
            match ws_recv(&mut rx, &mut buf).await {
                Ok((frame_type, len)) => match frame_type {
                    FrameType::Text(_) | FrameType::Binary(_) => {
                        let payload = &buf[..len];
                        let request = match serde_json::from_slice::<RpcRequest>(payload) {
                            Ok(request) => request,
                            Err(_) => continue,
                        };

                        let id = request.id.clone().unwrap_or(serde_json::Value::Null);
                        match rpc::dispatch_rpc(&request, ctx).await {
                            RpcAction::Respond(json) => {
                                let _ = tx_ch.send(WsTxMsg::Text(json)).await;
                            }
                            RpcAction::Subscribe { interval_ms } => {
                                let _ = tx_ch.send(WsTxMsg::Subscribe { id, interval_ms }).await;
                            }
                            RpcAction::Unsubscribe => {
                                let _ = tx_ch.send(WsTxMsg::Unsubscribe { id }).await;
                            }
                            RpcAction::Restart => {
                                let _ = tx_ch.send(WsTxMsg::Restart { id }).await;
                            }
                        }
                    }
                    FrameType::Ping => {
                        let pong_str = core::str::from_utf8(&buf[..len])
                            .map(String::from)
                            .unwrap_or_default();
                        let _ = tx_ch.send(WsTxMsg::Pong(pong_str)).await;
                    }
                    FrameType::Close => {
                        let _ = tx_ch.send(WsTxMsg::Close).await;
                        break;
                    }
                    _ => {}
                },
                Err(_) => {
                    let _ = tx_ch.send(WsTxMsg::Close).await;
                    break;
                }
            }
        }
    };

    let tx_task = async {
        let mut subscribed = false;
        let mut push_interval_ms = 500u64;
        let mut next_push = Instant::now() + Duration::from_millis(push_interval_ms);

        loop {
            let timer_fut = async {
                if subscribed {
                    Timer::at(next_push).await;
                } else {
                    core::future::pending::<()>().await;
                }
            };

            match select(tx_ch.receive(), timer_fut).await {
                SelectEither::First(msg) => match msg {
                    WsTxMsg::Text(json) => {
                        if ws_send(&mut tx, FrameType::Text(false), None, json.as_bytes())
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    WsTxMsg::Pong(pong_str) => {
                        if ws_send(&mut tx, FrameType::Pong, None, pong_str.as_bytes())
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    WsTxMsg::Subscribe { id, interval_ms } => {
                        push_interval_ms = interval_ms;
                        subscribed = true;
                        next_push = Instant::now() + Duration::from_millis(push_interval_ms);
                        let ack = rpc::subscribe_ack(id, interval_ms);
                        if ws_send(&mut tx, FrameType::Text(false), None, ack.as_bytes())
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    WsTxMsg::Unsubscribe { id } => {
                        subscribed = false;
                        let ack = rpc::unsubscribe_ack(id);
                        if ws_send(&mut tx, FrameType::Text(false), None, ack.as_bytes())
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    WsTxMsg::Restart { id } => {
                        let ack = rpc::restart_ack(id);
                        let _ =
                            ws_send(&mut tx, FrameType::Text(false), None, ack.as_bytes()).await;
                        Timer::after(Duration::from_millis(100)).await;
                        esp_hal::system::software_reset();
                    }
                    WsTxMsg::Close => break,
                },
                SelectEither::Second(()) => {
                    next_push += Duration::from_millis(push_interval_ms);
                    let mut lease = crate::buffers::NET_BUFFER_POOL.acquire().await;
                    if let Some(len) = rpc::build_state_notification_into(ctx, &mut *lease) {
                        if ws_send(&mut tx, FrameType::Text(false), None, &lease[..len])
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        }
    };

    select(rx_task, tx_task).await;
    // Do not call close(Both).await here: that re-enters TCP read/flush after the
    // peer or a WiFi blip (common when BLE starts) may have already torn down the
    // smoltcp slot — observed as Load access fault (mtval=0) in SocketSet::get_mut.
    // Dropping the edge TcpSocket removes the handle without another await.
    drop(socket);
}

#[embassy_executor::task(pool_size = 3)]
async fn ws_session_task(ctx: AppContext) {
    loop {
        let (wrapped, _acceptor_id) = WS_SOCKET_CH.receive().await;
        run_ws_session(wrapped.into_inner(), ctx).await;
        WS_SLOTS.release(1);
    }
}

async fn run_modbus_ws_session(mut socket: TcpSocket<'static>) {
    let (mut rx, mut tx) = socket.split();
    let mut buf = [0u8; HTTP_BUFFER_SIZE];
    while let Ok((frame_type, len)) = ws_recv(&mut rx, &mut buf).await {
        match frame_type {
            FrameType::Binary(_) | FrameType::Text(_) => {
                let req = &buf[..len];
                match modbus_relay::exchange_rtu(req).await {
                    Ok(resp) => {
                        if ws_send(&mut tx, FrameType::Binary(false), None, &resp)
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(e) => {
                        log::warn!("Modbus WS exchange failed: {}", e);
                        // Close on bus errors so the client can retry/reconnect.
                        break;
                    }
                }
            }
            FrameType::Ping
                if ws_send(&mut tx, FrameType::Pong, None, &buf[..len])
                    .await
                    .is_err() =>
            {
                break;
            }
            FrameType::Close => break,
            _ => {}
        }
    }
    drop(socket);
}

#[embassy_executor::task(pool_size = 2)]
async fn modbus_ws_session_task() {
    loop {
        let (wrapped, _acceptor_id) = MODBUS_WS_CH.receive().await;
        run_modbus_ws_session(wrapped.into_inner()).await;
        MODBUS_WS_SLOTS.release(1);
    }
}

static MODBUS_TCP_SOCKET_CH: Channel<CriticalSectionRawMutex, SyncTcpSocket, MODBUS_TCP_POOL> =
    Channel::new();

async fn read_exact(socket: &mut TcpSocket<'static>, buf: &mut [u8]) -> bool {
    let mut off = 0;
    while off < buf.len() {
        match socket.read(&mut buf[off..]).await {
            Ok(0) => return false,
            Ok(n) => off += n,
            Err(_) => return false,
        }
    }
    true
}

async fn handle_modbus_tcp_client(mut socket: TcpSocket<'static>) {
    let mut hdr = [0u8; 7];
    loop {
        if !read_exact(&mut socket, &mut hdr).await {
            break;
        }
        let tid = u16::from_be_bytes([hdr[0], hdr[1]]);
        let proto = u16::from_be_bytes([hdr[2], hdr[3]]);
        let length = u16::from_be_bytes([hdr[4], hdr[5]]) as usize;
        let unit_id = hdr[6];
        if proto != 0 || !(2..=253).contains(&length) {
            break;
        }
        let pdu_len = length - 1;
        let mut pdu = [0u8; 256];
        if !read_exact(&mut socket, &mut pdu[..pdu_len]).await {
            break;
        }

        match modbus_relay::exchange_modbus_tcp(unit_id, &pdu[..pdu_len]).await {
            Ok(resp_pdu) => {
                let resp_len = 1 + resp_pdu.len();
                let mut out = [0u8; 260];
                out[0..2].copy_from_slice(&tid.to_be_bytes());
                out[2..4].copy_from_slice(&0u16.to_be_bytes());
                out[4..6].copy_from_slice(&(resp_len as u16).to_be_bytes());
                out[6] = unit_id;
                out[7..7 + resp_pdu.len()].copy_from_slice(&resp_pdu);
                if Write::write_all(&mut socket, &out[..7 + resp_pdu.len()])
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(e) => {
                log::warn!("Modbus TCP exchange failed: {}", e);
                let fc = if pdu_len > 0 { pdu[0] } else { 0x00 };
                let exc = [fc | 0x80, 0x0A];
                let mut out = [0u8; 9];
                out[0..2].copy_from_slice(&tid.to_be_bytes());
                out[2..4].copy_from_slice(&0u16.to_be_bytes());
                out[4..6].copy_from_slice(&3u16.to_be_bytes());
                out[6] = unit_id;
                out[7..9].copy_from_slice(&exc);
                let _ = Write::write_all(&mut socket, &out).await;
                break;
            }
        }
    }
    let _ = socket.close(Close::Both).await;
}

#[embassy_executor::task(pool_size = 2)]
async fn modbus_tcp_session_task() {
    loop {
        let wrapped = MODBUS_TCP_SOCKET_CH.receive().await;
        handle_modbus_tcp_client(wrapped.into_inner()).await;
    }
}

async fn modbus_tcp_server_main(stack: &'static Stack<'static>) {
    let spawner = unsafe { embassy_executor::Spawner::for_current_executor().await };
    for _ in 0..MODBUS_TCP_POOL {
        spawner.spawn(modbus_tcp_session_task().unwrap());
    }

    let buffers = MODBUS_TCP_BUFFERS.init(TcpBuffers::new());
    let tcp = MODBUS_TCP_FACTORY.init(Tcp::new(*stack, buffers));
    let acceptor = loop {
        match tcp
            .bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 502)))
            .await
        {
            Ok(a) => break a,
            Err(e) => {
                log::error!("Modbus TCP :502 bind failed: {:?}, retrying...", e);
                Timer::after(Duration::from_secs(1)).await;
            }
        }
    };
    log::info!("Modbus TCP relay listening on :502");

    use edge_nal::TcpAccept;
    loop {
        match acceptor.accept().await {
            Ok((_, socket)) => {
                let wrapped = SyncTcpSocket::new(socket);
                if MODBUS_TCP_SOCKET_CH.try_send(wrapped).is_err() {
                    // Pool busy — drop connection.
                }
            }
            Err(e) => {
                log::warn!("Modbus TCP accept error: {:?}", e);
                Timer::after(Duration::from_millis(50)).await;
            }
        }
    }
}

#[embassy_executor::task]
async fn modbus_tcp_server_task(stack: &'static Stack<'static>) {
    modbus_tcp_server_main(stack).await;
}

async fn acceptor_loop(acceptor_id: usize, acceptor: edge_nal_embassy::TcpAccept<'static>) {
    use edge_nal::TcpAccept;
    loop {
        ACCEPT_SIGNALS[acceptor_id].wait().await;
        match acceptor.accept().await {
            Ok((_, socket)) => {
                SOCKET_QUEUE
                    .send((SyncTcpSocket::new(socket), acceptor_id))
                    .await;
            }
            Err(e) => {
                log::warn!("HTTP acceptor {} error: {:?}", acceptor_id, e);
                ACCEPT_SIGNALS[acceptor_id].signal(());
            }
        }
    }
}

async fn dispatcher_loop(ctx: AppContext) {
    loop {
        let (wrapped, acceptor_id) = SOCKET_QUEUE.receive().await;
        let socket = wrapped.into_inner();
        process_connection(socket, acceptor_id, ctx).await;
        ACCEPT_SIGNALS[acceptor_id].signal(());
    }
}

async fn http_server_main(stack: &'static Stack<'static>, ctx: AppContext) {
    let buffers = TCP_BUFFERS.init(TcpBuffers::new());
    let tcp = TCP_FACTORY.init(Tcp::new(*stack, buffers));
    let acceptor: edge_nal_embassy::TcpAccept<'static> = loop {
        match tcp
            .bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 80)))
            .await
        {
            Ok(acceptor) => {
                #[allow(clippy::missing_transmute_annotations)]
                break unsafe { core::mem::transmute(acceptor) };
            }
            Err(e) => {
                log::error!("HTTP bind failed: {:?}, retrying in 1s...", e);
                Timer::after(Duration::from_secs(1)).await;
            }
        }
    };

    log::info!(
        "HTTP edge server: acceptors={}, ws_max={}, tcp_pool={}, dispatcher_tasks={}",
        Q_ACCEPTORS,
        WS_MAX,
        TCP_POOL_SIZE,
        WEB_TASK_POOL_SIZE
    );

    for signal in ACCEPT_SIGNALS.iter() {
        signal.signal(());
    }
    HTTP_READY.signal(());

    embassy_futures::join::join(
        embassy_futures::join::join4(
            acceptor_loop(0, acceptor),
            acceptor_loop(1, acceptor),
            acceptor_loop(2, acceptor),
            acceptor_loop(3, acceptor),
        ),
        dispatcher_loop(ctx),
    )
    .await;
}

#[embassy_executor::task]
async fn http_server_task(stack: &'static Stack<'static>, app_context: AppContext) {
    http_server_main(stack, app_context).await;
}

pub async fn run_server(
    stack: &'static Stack<'static>,
    app_context: AppContext,
    spawner: &Spawner,
    rtu_relay: bool,
) {
    for _ in 0..WS_MAX {
        spawner.spawn(ws_session_task(app_context).unwrap());
    }
    if rtu_relay {
        for _ in 0..MODBUS_WS_MAX {
            spawner.spawn(modbus_ws_session_task().unwrap());
        }
        spawner.spawn(modbus_tcp_server_task(stack).unwrap());
    }
    spawner.spawn(http_server_task(stack, app_context).unwrap());
    // Wait until bind + accept priming completes so BLE is not started while the
    // HTTP socket set is still mid-setup (and so the log message is accurate).
    HTTP_READY.wait().await;
}
