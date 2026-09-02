mod args;
mod bus;
mod cli;
mod engine_task;
mod http;
mod modbus_rx;
mod persist;
mod relay_server;
mod serial;
mod usb_pll;

use std::sync::Arc;

use args::Args;
use engine_task::{run_engine_task, EngineHandle, EngineTaskOpts};
use persist::Persist;
use tokio::sync::{mpsc, watch};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
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

    let persist = Persist::new(args.config_path());
    let saved = persist.load();

    let bus = open_bus(&args).await;

    if args.is_rtu_relay() {
        let Some(bus) = bus else {
            tracing::error!(
                "--mode rtu-relay requires --serial, --relay-tcp, --relay-ws, or --rs485-ws"
            );
            std::process::exit(2);
        };
        if let Err(e) =
            relay_server::run_relay_servers(bus, args.bind_addr(), args.modbus_bind_addr()).await
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
    };
    tokio::spawn(run_engine_task(cmd_rx, snap_tx, opts));

    let state = http::AppState {
        engine: handle,
        static_dir: args
            .static_dir
            .clone()
            .unwrap_or_else(args::default_static_dir),
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

async fn open_bus(args: &Args) -> Option<bus::ModbusBus> {
    if args.is_mock() && !args.has_remote_bus() && args.serial.is_none() {
        return None;
    }
    if let Some(url) = args.rs485_ws.as_ref() {
        return match bus::ModbusBus::rs485_ws(url, args.baud()).await {
            Ok(b) => Some(b),
            Err(e) => {
                tracing::error!("open --rs485-ws {url}: {e}");
                None
            }
        };
    }
    if let Some(url) = args.relay_ws.as_ref() {
        return match bus::ModbusBus::relay_ws(url).await {
            Ok(b) => Some(b),
            Err(e) => {
                tracing::error!("open --relay-ws {url}: {e}");
                None
            }
        };
    }
    if let Some(host) = args.relay_tcp.as_ref() {
        return match bus::ModbusBus::relay_tcp(host).await {
            Ok(b) => Some(b),
            Err(e) => {
                tracing::error!("open --relay-tcp {host}: {e}");
                None
            }
        };
    }
    if let Some(path) = args.serial.as_ref() {
        return match serial::SerialPort::open(&path.to_string_lossy(), args.baud(), args.slave_id())
        {
            Ok(p) => Some(bus::ModbusBus::serial(p)),
            Err(e) => {
                tracing::error!("open serial {}: {e}", path.display());
                None
            }
        };
    }
    None
}

#[cfg(test)]
mod http_tests {
    use super::*;
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
            },
        ));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        http::AppState {
            engine: handle,
            static_dir: dir,
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
