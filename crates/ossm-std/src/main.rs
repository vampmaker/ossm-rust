mod args;
mod bus;
mod cli;
mod console_mode;
mod engine_task;
mod esptool;
mod flash_port;
mod http;
mod link_stats;
mod modbus_rx;
mod persist;
mod relay_server;
mod runtime_config;
mod serial;
mod usb_host;

use std::sync::Arc;

use args::Args;
use engine_task::{run_engine_task, EngineHandle, EngineTaskOpts};
use link_stats::SharedLinkStats;
use persist::Persist;
use runtime_config::RuntimeConfigResponse;
use tokio::sync::{mpsc, watch};
use tracing_subscriber::EnvFilter;

#[cfg(target_os = "linux")]
fn tighten_timer_slack() {
    // Default slack is often 4–10 ms on Android; that becomes dt_max on a 5 ms grid.
    let rc = unsafe { libc::prctl(libc::PR_SET_TIMERSLACK, 10_000u64) };
    if rc != 0 {
        tracing::debug!("PR_SET_TIMERSLACK: {rc}");
    }
}

#[tokio::main]
async fn main() {
    #[cfg(target_os = "linux")]
    tighten_timer_slack();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap()))
        .init();

    let args = Args::parse_from_env();
    tracing::info!(
        mock = args.is_mock(),
        mode = ?args.mode,
        bind = %args.bind_addr(),
        config = %args.config_path().display(),
        "starting ossm-std"
    );

    if args.is_flash() || args.is_console() {
        let args = args.clone();
        match tokio::task::spawn_blocking(move || run_tool_mode(&args)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                tracing::error!("{e}");
                std::process::exit(1);
            }
            Err(e) => {
                tracing::error!("tool task: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let persist = Persist::new(args.config_path());
    let saved = persist.load();

    let link = SharedLinkStats::new();
    let bus = open_bus(&args, &link).await;

    if args.is_rtu_relay() {
        let Some(bus) = bus else {
            tracing::error!(
                "--mode rtu-relay requires --serial, --relay-tcp, --relay-ws, or --rs485-ws"
            );
            std::process::exit(2);
        };
        if let Err(e) =
            relay_server::run_relay_servers(bus, link, args.bind_addr(), args.modbus_bind_addr())
                .await
        {
            tracing::error!("{e}");
            std::process::exit(1);
        }
        return;
    }

    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    let (snap_tx, snap_rx) = watch::channel(ossm_core::StateResponse::default());
    let handle = EngineHandle {
        tx: cmd_tx,
        snap: snap_rx,
    };

    let opts = EngineTaskOpts {
        persist,
        saved,
        mock: args.is_mock() || bus.is_none(),
        serial: bus,
        link: link.clone(),
    };
    tokio::spawn(run_engine_task(cmd_rx, snap_tx, opts));

    let state = http::AppState {
        engine: handle,
        static_dir: args.static_dir.clone(),
        runtime: RuntimeConfigResponse::from_args(&args),
        link,
        restart: Arc::new(|| std::process::exit(0)),
    };
    let repl_engine = state.engine.clone();
    let repl_restart = state.restart.clone();
    let app = http::router(state);
    let listener = tokio::net::TcpListener::bind(args.bind_addr())
        .await
        .expect("bind");
    tracing::info!("listening on http://{}", args.bind_addr());
    tracing::info!(
        "point scripts at DEVICE_IP=127.0.0.1:{} (or http://{})",
        args.bind_addr().port(),
        args.bind_addr()
    );
    let http = axum::serve(listener, app);
    if args.want_repl() {
        tracing::info!("stdin REPL enabled (help, get/set, paths)");
        tokio::select! {
            r = http => r.expect("serve"),
            _ = cli::run(repl_engine, repl_restart) => {}
        }
    } else {
        http.await.expect("serve");
    }
}

async fn open_bus(args: &Args, link: &SharedLinkStats) -> Option<bus::ModbusBus> {
    if args.is_mock() && !args.has_remote_bus() && args.serial.is_none() {
        link.set_link_info(ossm_common::LinkTransport::Mock);
        return None;
    }
    if let Some(url) = args.rs485_ws.as_ref() {
        link.set_link_info(ossm_common::LinkTransport::Rs485Ws);
        return match bus::ModbusBus::rs485_ws(url, args.baud(), link.clone()).await {
            Ok(b) => Some(b),
            Err(e) => {
                tracing::error!("open --rs485-ws {url}: {e}");
                None
            }
        };
    }
    if let Some(url) = args.relay_ws.as_ref() {
        link.set_link_info(ossm_common::LinkTransport::ModbusWs);
        return match bus::ModbusBus::relay_ws(url, link.clone()).await {
            Ok(b) => Some(b),
            Err(e) => {
                tracing::error!("open --relay-ws {url}: {e}");
                None
            }
        };
    }
    if let Some(host) = args.relay_tcp.as_ref() {
        link.set_link_info(ossm_common::LinkTransport::RelayTcp);
        return match bus::ModbusBus::relay_tcp(host, link.clone()).await {
            Ok(b) => Some(b),
            Err(e) => {
                tracing::error!("open --relay-tcp {host}: {e}");
                None
            }
        };
    }
    if let Some(path) = args.serial.as_ref() {
        if usb_host::is_usb_bus_path(path) {
            #[cfg(target_os = "linux")]
            {
                return match usb_host::open_serial(path, args.baud(), args.slave_id()) {
                    Ok(p) => {
                        link.set_link_info(ossm_common::LinkTransport::UsbHost);
                        Some(bus::ModbusBus::serial(p, link.clone()))
                    }
                    Err(e) => {
                        tracing::error!("open usb host {}: {e}", path.display());
                        None
                    }
                };
            }
            #[cfg(not(target_os = "linux"))]
            {
                tracing::error!(
                    "USB host serial ({}) is Linux/Android only (Termux termux-usb / usbfs)",
                    path.display()
                );
                return None;
            }
        }
        return match serial::SerialPort::open(&path.to_string_lossy(), args.baud(), args.slave_id())
        {
            Ok(p) => {
                link.set_link_info(ossm_common::LinkTransport::Serial);
                Some(bus::ModbusBus::serial(p, link.clone()))
            }
            Err(e) => {
                tracing::error!("open serial {}: {e}", path.display());
                None
            }
        };
    }
    None
}

fn run_tool_mode(args: &Args) -> Result<(), String> {
    let Some(path) = args.serial.as_ref() else {
        return Err("--mode flash/console requires --serial".into());
    };
    let deassert = args.is_console() && !args.reset;
    let assume_jtag = args.usb_jtag;
    let mut port = flash_port::open_flash_port(path, args.baud(), deassert, assume_jtag)?;
    if args.is_flash() {
        let image_path = args
            .image
            .clone()
            .ok_or_else(|| "--mode flash requires --image".to_string())?;
        let image = std::fs::read(&image_path)
            .map_err(|e| format!("read {}: {e}", image_path.display()))?;
        let flash_size = esptool::parse_flash_size(&args.flash_size)?;
        let opts = esptool::FlashOpts {
            flash_size,
            force_chip: args.force,
            usb_jtag: args.usb_jtag,
            verify: !args.no_verify,
        };
        esptool::flash_image(
            port.as_mut(),
            &image,
            args.flash_offset,
            args.keep_nvs,
            &opts,
        )?;
        return Ok(());
    }
    let opts = console_mode::ConsoleOpts {
        send: args.send.clone(),
        until: args.until.clone(),
        timeout: std::time::Duration::from_secs(args.timeout_secs()),
        reset: args.reset,
        exit_rs485: args.exit_rs485,
        raw: args.raw,
    };
    console_mode::run(port.as_mut(), opts)
}

#[cfg(test)]
mod http_tests {
    use super::*;
    use crate::args::{Args, StdMode};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use ossm_core::rpc::RpcAction;
    use tower::ServiceExt;

    async fn mock_app() -> http::AppState {
        let dir = std::env::temp_dir().join(format!(
            "ossm-std-http-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let persist = Persist::new(dir.join("config.json"));
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let (snap_tx, snap_rx) = watch::channel(ossm_core::StateResponse::default());
        let handle = EngineHandle {
            tx: cmd_tx,
            snap: snap_rx,
        };
        tokio::spawn(run_engine_task(
            cmd_rx,
            snap_tx,
            EngineTaskOpts {
                persist,
                saved: persist::SavedConfig::default(),
                mock: true,
                serial: None,
                link: SharedLinkStats::new(),
            },
        ));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let cfg_path = dir.join("config.json");
        let args = Args {
            mode: StdMode::Servo,
            serial: None,
            baud: None,
            image: None,
            flash_offset: 0,
            flash_size: "4mb".into(),
            keep_nvs: false,
            no_verify: false,
            force: false,
            usb_jtag: false,
            send: Vec::new(),
            until: None,
            timeout: None,
            reset: false,
            exit_rs485: false,
            raw: false,
            bind: Some("127.0.0.1:8080".parse().unwrap()),
            modbus_bind: None,
            relay_tcp: None,
            relay_ws: None,
            rs485_ws: None,
            config: Some(cfg_path),
            mock: true,
            slave_id: None,
            static_dir: Some(dir.clone()),
            repl: false,
            no_repl: true,
        }
        .finalize();
        http::AppState {
            engine: handle,
            static_dir: Some(dir),
            runtime: RuntimeConfigResponse::from_args(&args),
            link: SharedLinkStats::new(),
            restart: Arc::new(|| {}),
        }
    }

    #[tokio::test]
    async fn get_state_after_mock_tick() {
        let state = mock_app().await;
        let app = http::router(state);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/state")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), 64 * 1024)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(v.get("config").is_some());
        assert!(v.get("paused").is_none() || v["config"]["paused"] == true);
        assert_eq!(v["pos_max"], 100.0);
    }

    #[tokio::test]
    async fn get_state_sends_cors_star() {
        let state = mock_app().await;
        let app = http::router(state);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/state")
                    .header("Origin", "http://example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()
                .get("access-control-allow-origin")
                .and_then(|v| v.to_str().ok()),
            Some("*")
        );
    }

    #[tokio::test]
    async fn options_config_preflight_cors() {
        let state = mock_app().await;
        let app = http::router(state);
        let res = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/config")
                    .header("Origin", "http://example.com")
                    .header("Access-Control-Request-Method", "POST")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(res.status().is_success());
        assert_eq!(
            res.headers()
                .get("access-control-allow-origin")
                .and_then(|v| v.to_str().ok()),
            Some("*")
        );
    }

    #[tokio::test]
    async fn get_index_serves_embedded_html() {
        let mut state = mock_app().await;
        state.static_dir = None;
        let app = http::router(state);
        let res = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let ct = res
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(ct.contains("text/html"), "content-type={ct}");
        let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let html = String::from_utf8_lossy(&bytes);
        assert!(html.to_ascii_lowercase().contains("<html"), "{html}");
        assert!(
            html.contains("speed-slider") && !html.contains("webui-std not built"),
            "embedded HTML should be the built webui-std (run npm run build -w webui-std)"
        );
        assert!(
            !html.contains("id=\"ble-enabled\"") && !html.contains("id=\"wifi-ssid\""),
            "bundled HTML must not include firmware WiFi/BLE settings"
        );
    }

    #[tokio::test]
    async fn get_runtime_config_read_only() {
        let state = mock_app().await;
        let app = http::router(state);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/runtime-config")
                    .header("Origin", "http://example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()
                .get("access-control-allow-origin")
                .and_then(|v| v.to_str().ok()),
            Some("*")
        );
        let bytes = axum::body::to_bytes(res.into_body(), 64 * 1024)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["mode"], "servo");
        assert_eq!(v["mock"], true);
        assert!(v.get("transport").is_some());
        assert!(v.get("bind").is_some());
    }

    #[tokio::test]
    async fn firmware_only_shell_routes_are_404() {
        for path in ["/shell-config", "/pin-config", "/network-config"] {
            let app = http::router(mock_app().await);
            let res = app
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[tokio::test]
    async fn rpc_ping_pong() {
        let state = mock_app().await;
        let (tx, rx) = tokio::sync::oneshot::channel();
        state
            .engine
            .tx
            .send(engine_task::EngineMsg::Rpc {
                req: br#"{"jsonrpc":"2.0","method":"ping","id":1}"#.to_vec(),
                reply: tx,
            })
            .await
            .unwrap();
        let (action, body) = rx.await.unwrap();
        assert_eq!(action, RpcAction::Respond);
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["result"], "pong");
    }
}
