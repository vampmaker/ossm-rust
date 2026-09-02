//! ossm-std RTU relay server: Modbus TCP `:502` + `/ws/modbus`.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tower_http::cors::{Any, CorsLayer};

use crate::bus::ModbusBus;
use ossm_core::modbus::{calc_crc16, verify_rtu_crc};

#[derive(Clone)]
struct RelayState {
    bus: Arc<Mutex<ModbusBus>>,
}

pub async fn run_relay_servers(
    bus: ModbusBus,
    http_bind: SocketAddr,
    modbus_bind: SocketAddr,
) -> Result<(), String> {
    let bus = Arc::new(Mutex::new(bus));
    let http_state = RelayState { bus: bus.clone() };
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);
    let app = Router::new()
        .route("/ws/modbus", get(ws_upgrade))
        .route("/health", get(|| async { "ok" }))
        .with_state(http_state)
        .layer(cors);

    let http_listener = TcpListener::bind(http_bind)
        .await
        .map_err(|e| format!("http bind {http_bind}: {e}"))?;
    tracing::info!("rtu-relay HTTP/WS on http://{http_bind}  (/ws/modbus)");

    let tcp_bus = bus.clone();
    tokio::spawn(async move {
        if let Err(e) = run_modbus_tcp(tcp_bus, modbus_bind).await {
            tracing::error!("modbus tcp server: {e}");
        }
    });

    axum::serve(http_listener, app)
        .await
        .map_err(|e| format!("http serve: {e}"))
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(state): State<RelayState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws_session(socket, state))
}

async fn ws_session(mut socket: WebSocket, state: RelayState) {
    while let Some(Ok(msg)) = socket.recv().await {
        let data = match msg {
            Message::Binary(b) => b,
            Message::Text(t) => t.as_bytes().to_vec().into(),
            Message::Close(_) => break,
            _ => continue,
        };
        if !verify_rtu_crc(&data) {
            tracing::warn!("relay ws bad crc");
            break;
        }
        let resp = {
            let mut bus = state.bus.lock().await;
            match bus.exchange_rtu(&data).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("relay ws exchange: {e}");
                    break;
                }
            }
        };
        if socket.send(Message::Binary(resp.into())).await.is_err() {
            break;
        }
    }
}

async fn run_modbus_tcp(bus: Arc<Mutex<ModbusBus>>, bind: SocketAddr) -> Result<(), String> {
    let listener = TcpListener::bind(bind)
        .await
        .map_err(|e| format!("modbus bind {bind}: {e}"))?;
    tracing::info!("Modbus TCP relay listening on {bind}");
    loop {
        let (mut socket, _) = listener
            .accept()
            .await
            .map_err(|e| format!("modbus accept: {e}"))?;
        let bus = bus.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_modbus_tcp_client(&mut socket, bus).await {
                tracing::debug!("modbus tcp client: {e}");
            }
        });
    }
}

async fn handle_modbus_tcp_client(
    socket: &mut tokio::net::TcpStream,
    bus: Arc<Mutex<ModbusBus>>,
) -> Result<(), String> {
    let mut hdr = [0u8; 7];
    loop {
        if socket.read_exact(&mut hdr).await.is_err() {
            return Ok(());
        }
        let tid = u16::from_be_bytes([hdr[0], hdr[1]]);
        let proto = u16::from_be_bytes([hdr[2], hdr[3]]);
        let length = u16::from_be_bytes([hdr[4], hdr[5]]) as usize;
        let unit_id = hdr[6];
        if proto != 0 || !(2..=253).contains(&length) {
            return Ok(());
        }
        let pdu_len = length - 1;
        let mut pdu = vec![0u8; pdu_len];
        socket
            .read_exact(&mut pdu)
            .await
            .map_err(|e| e.to_string())?;

        let mut rtu = Vec::with_capacity(pdu_len + 3);
        rtu.push(unit_id);
        rtu.extend_from_slice(&pdu);
        let crc = calc_crc16(&rtu);
        rtu.extend_from_slice(&crc.to_le_bytes());

        let resp = {
            let mut bus = bus.lock().await;
            bus.exchange_rtu(&rtu).await
        };
        match resp {
            Ok(rtu_resp) if rtu_resp.len() >= 3 => {
                let resp_pdu = &rtu_resp[1..rtu_resp.len() - 2];
                let resp_len = 1 + resp_pdu.len();
                let mut out = Vec::with_capacity(7 + resp_pdu.len());
                out.extend_from_slice(&tid.to_be_bytes());
                out.extend_from_slice(&0u16.to_be_bytes());
                out.extend_from_slice(&(resp_len as u16).to_be_bytes());
                out.push(unit_id);
                out.extend_from_slice(resp_pdu);
                socket.write_all(&out).await.map_err(|e| e.to_string())?;
            }
            Ok(_) | Err(_) => {
                let fc = pdu.first().copied().unwrap_or(0);
                let exc = [fc | 0x80, 0x0A];
                let mut out = [0u8; 9];
                out[0..2].copy_from_slice(&tid.to_be_bytes());
                out[4..6].copy_from_slice(&3u16.to_be_bytes());
                out[6] = unit_id;
                out[7..9].copy_from_slice(&exc);
                let _ = socket.write_all(&out).await;
                return Ok(());
            }
        }
    }
}
