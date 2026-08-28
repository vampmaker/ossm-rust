mod args;
mod engine_task;
mod http;
mod persist;
mod serial;

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
        bind = %args.bind_addr(),
        config = %args.config_path().display(),
        "starting ossm-std"
    );

    let persist = Persist::new(args.config_path());
    let saved = persist.load();

    let serial = if args.is_mock() {
        None
    } else if let Some(path) = args.serial.as_ref() {
        match serial::SerialPort::open(&path.to_string_lossy(), args.baud(), args.slave_id()) {
            Ok(p) => Some(p),
            Err(e) => {
                tracing::error!("open serial {}: {e}", path.display());
                None
            }
        }
    } else {
        None
    };

    let (cmd_tx, cmd_rx) = mpsc::channel(32);
    let (snap_tx, snap_rx) = watch::channel(ossm_core::StateResponse::default());
    let handle = EngineHandle {
        tx: cmd_tx,
        snap: snap_rx,
    };

    let opts = EngineTaskOpts {
        persist,
        saved,
        mock: args.is_mock() || serial.is_none(),
        no_homing: args.no_homing,
        serial,
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
    axum::serve(listener, app).await.expect("serve");
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
                no_homing: true,
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
