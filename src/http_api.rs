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
use crate::motion::{MotionCommand, MotorControllerConfig};
use crate::storage::PinConfiguration;

#[derive(Serialize, Deserialize)]
pub struct PausedControl {
    pub paused: Option<bool>,              // Set paused state
    pub position: Option<f32>,             // Set absolute position
    pub adjust: Option<f32>,               // Adjust position relatively (positive or negative)
}

const APP_HTML: &[u8] = include_bytes!("../frontend/dist/index.html");

// Bodies must be fully buffered because serde_json needs a contiguous slice;
// all POST payloads are small JSON objects, so 1 KiB is plenty. Note this
// array is held across await points, so it is part of each handler future.
const MAX_BODY_LEN: usize = 1024;
// Keep this low: the whole HTTP server future (including one handler future per
// task) lives inside net_task, which Embassy allocates statically in .bss.
const HANDLER_TASKS: usize = 2;
const HTTP_BUF_SIZE: usize = 2048;

const CORS_ORIGIN: (&str, &str) = ("Access-Control-Allow-Origin", "*");

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
                conn.initiate_response(200, Some("OK"), &[CORS_ORIGIN, ("Content-Type", "text/html")])
                    .await?;
                conn.write_all(APP_HTML).await?;
            }

            (Method::Options, "/config") => cors_preflight(conn, "GET, POST, OPTIONS").await?,
            (Method::Options, "/paused") => cors_preflight(conn, "POST, OPTIONS").await?,
            (Method::Options, "/state") => cors_preflight(conn, "GET, OPTIONS").await?,
            (Method::Options, "/pin-config") => cors_preflight(conn, "GET, POST, OPTIONS").await?,
            (Method::Options, "/restart") => cors_preflight(conn, "POST, OPTIONS").await?,

            (Method::Get, "/config") => {
                let json = {
                    let mut mc_opt = self.app_context.motor_controller.lock().unwrap();
                    mc_opt.as_mut().map(|mc| serde_json::to_string(&mc.get_config()).unwrap())
                };
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
                        let json = {
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
                                Some(serde_json::to_string(&config).unwrap())
                            } else {
                                None
                            }
                        };
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
                let json = {
                    let mut mc_opt = self.app_context.motor_controller.lock().unwrap();
                    mc_opt.as_mut().map(|mc| serde_json::to_string(&mc.get_current_state()).unwrap())
                };
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
        let mut frame_data = [0u8; 1024];

        loop {
            let header = match FrameHeader::recv(&mut *socket).await {
                Ok(header) => header,
                Err(e) => {
                    log::info!("WebSocket closed: {:?}", e);
                    return Ok(());
                }
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
                    if let Ok(cmd) = serde_json::from_slice::<MotionCommand>(payload) {
                        if command_tx.lock().unwrap().enqueue(cmd).is_err() {
                            log::error!("Failed to enqueue command");
                            return Ok(());
                        }
                    } else {
                        log::error!("Failed to parse command: {}", String::from_utf8_lossy(payload));
                        return Ok(());
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
    conn.initiate_response(200, Some("OK"), &[CORS_ORIGIN, ("Content-Type", "application/json")])
        .await?;
    conn.write_all(json.as_bytes()).await
}

async fn respond_unavailable<T, const N: usize>(
    conn: &mut Connection<'_, T, N>,
) -> Result<(), Error<T::Error>>
where
    T: Read + Write + TcpSplit,
{
    conn.initiate_response(503, Some("Service Unavailable"), &[CORS_ORIGIN]).await?;
    conn.write_all("Motor controller not initialized".as_bytes()).await
}

async fn respond_bad_request<T, const N: usize>(
    conn: &mut Connection<'_, T, N>,
    message: &str,
) -> Result<(), Error<T::Error>>
where
    T: Read + Write + TcpSplit,
{
    conn.initiate_response(400, Some("Bad Request"), &[CORS_ORIGIN]).await?;
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
