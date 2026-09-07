//! Pluggable Modbus RTU bus: local serial, Modbus TCP, `/ws/modbus`, `/ws/rs485`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use ossm_common::PllConfig;
use ossm_core::modbus::{aim30, calc_crc16};
use rmodbus::{client::ModbusRequest, ModbusProto};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{client_async, WebSocketStream};

use crate::link_stats::{micros_now, SharedLinkStats};
use crate::modbus_rx::{expected_response_len, parse_fc03_u16, RxAccumulator};
use crate::precise_wait::{AckWaker, Waker};
use crate::serial::{encode_position, is_crc_miss, AckSample, LatestWins, SerialPort};

const RS485_TX: u8 = 0;
const RS485_RX: u8 = 1;
const RS485_CFG: u8 = 2;

#[allow(clippy::large_enum_variant)]
enum ModbusBusInner {
    Serial(SerialPort),
    Tcp(RelayTcp),
    ModbusWs(WsPipe),
    Rs485Ws(Rs485Ws),
    Stream(NetStream),
}

#[derive(Clone)]
struct NetStream {
    slave: u8,
    usb_serial_pll: bool,
    slot: Arc<LatestWins>,
    wakeup: Arc<Notify>,
    ack: Arc<Mutex<Option<AckSample>>>,
    ack_waker: Arc<AckWaker>,
    alive: Arc<AtomicBool>,
}

enum StreamInner {
    Net(NetStream),
    #[cfg(target_os = "linux")]
    UsbHost(crate::serial::UsbStreamParts),
}

/// Sync handle the motor thread uses after `spawn_stream_worker`.
pub struct StreamHandle {
    inner: StreamInner,
}

pub struct ModbusBus {
    pub link: SharedLinkStats,
    inner: ModbusBusInner,
}

pub struct RelayTcp {
    host: String,
    stream: Option<TcpStream>,
    tid: u16,
}

pub struct WsPipe {
    url: String,
    ws: Option<WebSocketStream<TcpStream>>,
}

pub struct Rs485Ws {
    url: String,
    baud: u32,
    ws: Option<WebSocketStream<TcpStream>>,
    rx: RxAccumulator,
    last_rx: Option<Instant>,
}

impl ModbusBus {
    pub fn serial(mut port: SerialPort, link: SharedLinkStats) -> Self {
        port.attach_link(link.clone());
        Self {
            link,
            inner: ModbusBusInner::Serial(port),
        }
    }

    pub async fn relay_tcp(host: &str, link: SharedLinkStats) -> Result<Self, String> {
        let mut tcp = RelayTcp {
            host: host.to_string(),
            stream: None,
            tid: 1,
        };
        tcp.connect().await?;
        Ok(Self {
            link,
            inner: ModbusBusInner::Tcp(tcp),
        })
    }

    pub async fn relay_ws(url: &str, link: SharedLinkStats) -> Result<Self, String> {
        let mut ws = WsPipe {
            url: url.to_string(),
            ws: None,
        };
        ws.connect().await?;
        Ok(Self {
            link,
            inner: ModbusBusInner::ModbusWs(ws),
        })
    }

    pub async fn rs485_ws(url: &str, baud: u32, link: SharedLinkStats) -> Result<Self, String> {
        let mut ws = Rs485Ws {
            url: url.to_string(),
            baud,
            ws: None,
            rx: RxAccumulator::new(),
            last_rx: None,
        };
        ws.connect().await?;
        Ok(Self {
            link,
            inner: ModbusBusInner::Rs485Ws(ws),
        })
    }

    pub async fn exchange_rtu(&mut self, frame: &[u8]) -> Result<Vec<u8>, String> {
        let t0 = Instant::now();
        let result = match &mut self.inner {
            ModbusBusInner::Serial(p) => p.exchange_rtu(frame).await,
            ModbusBusInner::Tcp(t) => t.exchange_rtu(frame).await,
            ModbusBusInner::ModbusWs(w) => w.exchange_rtu(frame).await,
            ModbusBusInner::Rs485Ws(w) => w.exchange_rtu(frame).await,
            ModbusBusInner::Stream(_) => Err("stream worker owns the bus".into()),
        };
        let now_us = micros_now();
        let rtt_us = t0.elapsed().as_micros().min(u16::MAX as u128) as u16;
        match &result {
            Ok(rsp) => self.link.record_success(
                now_us,
                rtt_us,
                None,
                None,
                frame.len() as u32,
                rsp.len() as u32,
            ),
            Err(_e) => self.link.record_failure(now_us),
        }
        result
    }

    pub async fn write_position_radians_strict(&mut self, position: f32) -> Result<(), String> {
        let counts = aim30::write_counts_for_radians(position);
        let regs = aim30::pack_position_i32(counts);
        self.write_holdings(aim30::REG_POSITION, &regs).await
    }

    pub async fn read_position_radians(&mut self) -> Result<f32, String> {
        let regs = self.read_holdings(aim30::REG_POSITION, 2).await?;
        if regs.len() < 2 {
            return Err("short position read".into());
        }
        let counts = aim30::unpack_position_i32([regs[0], regs[1]]);
        Ok(aim30::counts_to_radians(counts))
    }

    pub async fn enable_modbus(&mut self) -> Result<(), String> {
        self.write_holding(aim30::REG_ENABLE, aim30::ENABLE_MODBUS)
            .await
    }

    pub async fn set_max_power(&mut self, power: f32) -> Result<(), String> {
        self.write_holding(aim30::REG_POWER, aim30::encode_max_power(power))
            .await
    }

    pub async fn set_acceleration(&mut self, acceleration: f32) -> Result<(), String> {
        self.write_holding(aim30::REG_ACCEL, aim30::encode_acceleration(acceleration))
            .await
    }

    pub async fn set_position_ring_ratio(&mut self, ratio: u16) -> Result<(), String> {
        self.write_holding(aim30::REG_POS_RING, ratio).await
    }

    pub async fn set_speed_ring_ratio(&mut self, ratio: u16) -> Result<(), String> {
        self.write_holding(aim30::REG_SPEED_RING, ratio).await
    }

    pub async fn reset_position(&mut self) -> Result<(), String> {
        let regs = aim30::pack_position_i32(0);
        self.write_holdings(aim30::REG_POSITION, &regs).await
    }

    pub async fn wait_stable_position(&mut self, timeout: Duration) -> Result<f32, String> {
        let start = Instant::now();
        let mut position = self.read_position_radians().await?;
        tokio::time::sleep(Duration::from_millis(100)).await;
        while start.elapsed() < timeout {
            let new_position = self.read_position_radians().await?;
            if (new_position - position).abs() < 0.002 {
                return Ok(new_position);
            }
            position = new_position;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err("stable position timeout".into())
    }

    pub async fn homing(&mut self) -> Result<(f32, f32), String> {
        tracing::info!("Homing motor");
        let mut power_ok = false;
        for attempt in 1..=8 {
            match self.set_max_power(aim30::POWER_HOMING).await {
                Ok(()) => {
                    power_ok = true;
                    break;
                }
                Err(e) => {
                    tracing::warn!("set_max_power POWER_HOMING ({attempt}/8): {e}");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
        if !power_ok {
            return Err("set_max_power POWER_HOMING failed after retries".into());
        }
        let want = aim30::encode_max_power(aim30::POWER_HOMING);
        let regs = self.read_holdings(aim30::REG_POWER, 1).await?;
        let got = regs.first().copied();
        if got != Some(want) {
            return Err(format!(
                "REG_POWER readback {got:?} want {want} (POWER_HOMING)"
            ));
        }
        tracing::info!("REG_POWER={want} after POWER_HOMING");
        self.set_acceleration(aim30::ACCEL_HOMING).await?;
        self.reset_position().await?;

        tracing::info!("Writing position to -100.0");
        self.write_position_radians_strict(-100.0).await?;
        tokio::time::sleep(Duration::from_millis(5000)).await;
        let pos_min = self
            .wait_stable_position(Duration::from_millis(5000))
            .await?
            + 0.1;
        tracing::info!("pos_min: {pos_min}");

        tracing::info!("Writing position to 100.0");
        self.write_position_radians_strict(100.0).await?;
        tokio::time::sleep(Duration::from_millis(5000)).await;
        let pos_max = self
            .wait_stable_position(Duration::from_millis(5000))
            .await?
            - 0.1;
        tracing::info!("pos_max: {pos_max}");

        self.write_position_radians_strict((pos_min + pos_max) / 2.0)
            .await?;
        tokio::time::sleep(Duration::from_millis(5000)).await;
        self.wait_stable_position(Duration::from_millis(5000))
            .await?;
        Ok((pos_min, pos_max))
    }

    pub async fn apply_run_gains(&mut self) -> Result<(), String> {
        self.set_max_power(aim30::POWER_RUN).await?;
        self.set_acceleration(aim30::ACCEL_RUN).await?;
        self.set_position_ring_ratio(aim30::RING_RATIO_RUN).await?;
        self.set_speed_ring_ratio(aim30::RING_RATIO_RUN).await
    }

    async fn write_holding(&mut self, addr: u16, value: u16) -> Result<(), String> {
        let slave = self.slave();
        let mut request = ModbusRequest::new(slave, ModbusProto::Rtu);
        let mut request_buf = fixedvec::alloc_stack!([u8; 256]);
        let mut frame_buf = fixedvec::FixedVec::new(&mut request_buf);
        request
            .generate_set_holding(addr, value, &mut frame_buf)
            .map_err(|e| format!("modbus encode: {e:?}"))?;
        let tx = frame_buf.as_slice().to_vec();
        let rsp = self.exchange_rtu(&tx).await?;
        request
            .parse_ok(&rsp)
            .map_err(|e| format!("modbus parse: {e:?}"))
    }

    async fn write_holdings(&mut self, addr: u16, values: &[u16]) -> Result<(), String> {
        let slave = self.slave();
        let mut request = ModbusRequest::new(slave, ModbusProto::Rtu);
        let mut request_buf = fixedvec::alloc_stack!([u8; 256]);
        let mut frame_buf = fixedvec::FixedVec::new(&mut request_buf);
        request
            .generate_set_holdings_bulk(addr, values, &mut frame_buf)
            .map_err(|e| format!("modbus encode: {e:?}"))?;
        let tx = frame_buf.as_slice().to_vec();
        let rsp = self.exchange_rtu(&tx).await?;
        request
            .parse_ok(&rsp)
            .map_err(|e| format!("modbus parse: {e:?}"))
    }

    async fn read_holdings(&mut self, addr: u16, count: u16) -> Result<Vec<u16>, String> {
        let slave = self.slave();
        let mut request = ModbusRequest::new(slave, ModbusProto::Rtu);
        let mut request_buf = fixedvec::alloc_stack!([u8; 256]);
        let mut frame_buf = fixedvec::FixedVec::new(&mut request_buf);
        request
            .generate_get_holdings(addr, count, &mut frame_buf)
            .map_err(|e| format!("modbus encode: {e:?}"))?;
        let tx = frame_buf.as_slice().to_vec();
        let rsp = self.exchange_rtu(&tx).await?;
        parse_fc03_u16(&rsp).ok_or_else(|| "modbus parse: fc03".into())
    }

    fn slave(&self) -> u8 {
        match &self.inner {
            ModbusBusInner::Serial(p) => p.slave_id(),
            ModbusBusInner::Stream(s) => s.slave,
            ModbusBusInner::Tcp(_) | ModbusBusInner::ModbusWs(_) | ModbusBusInner::Rs485Ws(_) => 1,
        }
    }

    pub fn as_serial(&self) -> Option<&SerialPort> {
        match &self.inner {
            ModbusBusInner::Serial(p) => Some(p),
            _ => None,
        }
    }

    pub fn is_usb_host(&self) -> bool {
        self.as_serial().is_some_and(|p| p.is_usb_host())
    }

    /// After homing: move the bus into a tokio task so engine ticks do not await RTT.
    /// USB-host already has its own thread; this is a no-op there.
    pub fn spawn_stream_worker(&mut self) {
        if self.is_usb_host() {
            return;
        }
        if matches!(self.inner, ModbusBusInner::Stream(_)) {
            return;
        }
        let usb_serial_pll = matches!(self.inner, ModbusBusInner::Serial(_));
        let slave = self.slave();
        let stream = NetStream {
            slave,
            usb_serial_pll,
            slot: Arc::new(LatestWins::new()),
            wakeup: Arc::new(Notify::new()),
            ack: Arc::new(Mutex::new(None)),
            ack_waker: Arc::new(AckWaker::new()),
            alive: Arc::new(AtomicBool::new(true)),
        };
        let worker = stream.clone();
        let link = self.link.clone();
        let inner = std::mem::replace(&mut self.inner, ModbusBusInner::Stream(stream));
        tokio::spawn(async move {
            let mut bus = ModbusBus { link, inner };
            net_stream_loop(&mut bus, worker).await;
        });
    }

    pub fn stream_handle(&self) -> Option<StreamHandle> {
        match &self.inner {
            ModbusBusInner::Stream(s) => Some(StreamHandle {
                inner: StreamInner::Net(s.clone()),
            }),
            ModbusBusInner::Serial(p) => {
                #[cfg(target_os = "linux")]
                {
                    p.usb_stream_parts().map(|parts| StreamHandle {
                        inner: StreamInner::UsbHost(parts),
                    })
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = p;
                    None
                }
            }
            _ => None,
        }
    }
}

impl StreamHandle {
    pub fn set_ack_waker(&self, waker: Waker) {
        match &self.inner {
            StreamInner::Net(s) => s.ack_waker.set(waker),
            #[cfg(target_os = "linux")]
            StreamInner::UsbHost(p) => p.ack_waker.set(waker),
        }
    }

    pub fn submit_position(&self, position: f32) -> Result<(), String> {
        match &self.inner {
            StreamInner::Net(s) => {
                if !s.alive.load(Ordering::Relaxed) {
                    return Err("stream worker ended".into());
                }
                let tx = encode_position(s.slave, position)?;
                s.slot.submit(tx);
                s.wakeup.notify_one();
                Ok(())
            }
            #[cfg(target_os = "linux")]
            StreamInner::UsbHost(p) => {
                if !p.alive.load(Ordering::Relaxed) {
                    return Err("usb host thread ended".into());
                }
                p.stream.submit(encode_position(p.slave, position)?);
                Ok(())
            }
        }
    }

    pub fn take_ack_sample(&self) -> Option<AckSample> {
        match &self.inner {
            StreamInner::Net(s) => s.ack.lock().ok().and_then(|mut g| g.take()),
            #[cfg(target_os = "linux")]
            StreamInner::UsbHost(p) => p.ack.lock().ok().and_then(|mut g| g.take()),
        }
    }

    pub fn alive(&self) -> bool {
        match &self.inner {
            StreamInner::Net(s) => s.alive.load(Ordering::Relaxed),
            #[cfg(target_os = "linux")]
            StreamInner::UsbHost(p) => p.alive.load(Ordering::Relaxed),
        }
    }

    pub fn pll_config(&self) -> PllConfig {
        match &self.inner {
            StreamInner::Net(s) if s.usb_serial_pll => PllConfig::usb_serial(),
            StreamInner::Net(_) => PllConfig::network(),
            #[cfg(target_os = "linux")]
            StreamInner::UsbHost(_) => PllConfig::usb_serial(),
        }
    }
}

async fn net_stream_loop(bus: &mut ModbusBus, stream: NetStream) {
    loop {
        if !stream.alive.load(Ordering::Relaxed) {
            break;
        }
        let wait = stream.wakeup.notified();
        let Some(frame) = stream.slot.take() else {
            wait.await;
            continue;
        };
        drop(wait);
        let t_tx = Instant::now();
        let t_rx = match bus.exchange_rtu(&frame).await {
            Ok(_) => Some(Instant::now()),
            Err(e) if is_crc_miss(&e) => None,
            Err(e) => {
                tracing::debug!("stream worker: {e}");
                stream.alive.store(false, Ordering::Relaxed);
                publish_net_ack(&stream, t_tx, None);
                break;
            }
        };
        publish_net_ack(&stream, t_tx, t_rx);
    }
}

fn publish_net_ack(stream: &NetStream, t_tx: Instant, t_rx: Option<Instant>) {
    if let Ok(mut g) = stream.ack.lock() {
        *g = Some(AckSample { t_tx, t_rx });
    }
    stream.ack_waker.notify();
}

impl RelayTcp {
    async fn connect(&mut self) -> Result<(), String> {
        let stream = TcpStream::connect(&self.host)
            .await
            .map_err(|e| format!("relay-tcp connect {}: {e}", self.host))?;
        self.stream = Some(stream);
        Ok(())
    }

    async fn exchange_rtu(&mut self, frame: &[u8]) -> Result<Vec<u8>, String> {
        if frame.len() < 4 {
            return Err("short rtu".into());
        }
        let unit = frame[0];
        let pdu = &frame[1..frame.len() - 2];
        self.tid = self.tid.wrapping_add(1);
        if self.tid == 0 {
            self.tid = 1;
        }
        let len = 1 + pdu.len();
        let mut mbap = Vec::with_capacity(7 + pdu.len());
        mbap.extend_from_slice(&self.tid.to_be_bytes());
        mbap.extend_from_slice(&0u16.to_be_bytes());
        mbap.extend_from_slice(&(len as u16).to_be_bytes());
        mbap.push(unit);
        mbap.extend_from_slice(pdu);

        let stream = self.stream.as_mut().ok_or("tcp closed")?;
        stream
            .write_all(&mbap)
            .await
            .map_err(|e| format!("tcp write: {e}"))?;
        let mut hdr = [0u8; 7];
        stream
            .read_exact(&mut hdr)
            .await
            .map_err(|e| format!("tcp hdr: {e}"))?;
        let r_len = u16::from_be_bytes([hdr[4], hdr[5]]) as usize;
        if !(2..=253).contains(&r_len) {
            return Err(format!("bad mbap len {r_len}"));
        }
        let mut body = vec![0u8; r_len - 1];
        stream
            .read_exact(&mut body)
            .await
            .map_err(|e| format!("tcp body: {e}"))?;
        let mut rtu = Vec::with_capacity(1 + body.len() + 2);
        rtu.push(hdr[6]);
        rtu.extend_from_slice(&body);
        let crc = calc_crc16(&rtu);
        rtu.extend_from_slice(&crc.to_le_bytes());
        Ok(rtu)
    }
}

impl WsPipe {
    async fn connect(&mut self) -> Result<(), String> {
        self.ws = Some(connect_ws(&self.url).await?);
        Ok(())
    }

    async fn exchange_rtu(&mut self, frame: &[u8]) -> Result<Vec<u8>, String> {
        let ws = self.ws.as_mut().ok_or("ws closed")?;
        ws.send(Message::Binary(frame.to_vec().into()))
            .await
            .map_err(|e| format!("ws send: {e}"))?;
        match tokio::time::timeout(Duration::from_millis(800), ws.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => Ok(b.to_vec()),
            Ok(Some(Ok(Message::Text(t)))) => Ok(t.as_bytes().to_vec()),
            Ok(Some(Ok(_))) => Err("unexpected ws frame".into()),
            Ok(Some(Err(e))) => Err(format!("ws recv: {e}")),
            Ok(None) => Err("ws closed".into()),
            Err(_) => Err("ws timeout".into()),
        }
    }
}

impl Rs485Ws {
    async fn connect(&mut self) -> Result<(), String> {
        self.ws = Some(connect_ws(&self.url).await?);
        self.send_cfg(self.baud).await?;
        Ok(())
    }

    async fn send_cfg(&mut self, baud: u32) -> Result<(), String> {
        let json = format!("{{\"baud\":{baud}}}");
        let pkt = pack_rs485(RS485_CFG, json.as_bytes());
        let ws = self.ws.as_mut().ok_or("ws closed")?;
        ws.send(Message::Binary(pkt.into()))
            .await
            .map_err(|e| format!("rs485 cfg: {e}"))
    }

    async fn exchange_rtu(&mut self, frame: &[u8]) -> Result<Vec<u8>, String> {
        let expected = expected_response_len(frame);
        let req_fc = frame.get(1).copied().unwrap_or(0x10);
        let slave = frame.first().copied().unwrap_or(1);
        self.rx.clear();

        {
            let ws = self.ws.as_mut().ok_or("ws closed")?;
            let pkt = pack_rs485(RS485_TX, frame);
            ws.send(Message::Binary(pkt.into()))
                .await
                .map_err(|e| format!("rs485 tx: {e}"))?;
        }

        let deadline = Instant::now() + Duration::from_millis(800);
        loop {
            if let Some(out) = self.rx.extract(slave, expected, req_fc) {
                let out = out.to_vec();
                self.last_rx = Some(Instant::now());
                return Ok(out);
            }
            let remain = deadline.saturating_duration_since(Instant::now());
            if remain.is_zero() {
                return Err(format!("no CRC-valid response ({} bytes)", self.rx.len()));
            }
            let ws = self.ws.as_mut().ok_or("ws closed")?;
            match tokio::time::timeout(remain, ws.next()).await {
                Err(_) => {
                    return Err(format!("no CRC-valid response ({} bytes)", self.rx.len()));
                }
                Ok(None) => return Err("ws closed".into()),
                Ok(Some(Err(e))) => return Err(format!("ws recv: {e}")),
                Ok(Some(Ok(Message::Binary(b)))) => {
                    if let Some((typ, payload)) = unpack_rs485(&b) {
                        if typ == RS485_RX {
                            self.rx.push(&payload);
                            self.last_rx = Some(Instant::now());
                        }
                    }
                }
                Ok(Some(Ok(_))) => {}
            }
        }
    }
}

fn pack_rs485(typ: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(3 + payload.len());
    v.push(typ);
    v.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    v.extend_from_slice(payload);
    v
}

fn unpack_rs485(frame: &[u8]) -> Option<(u8, Vec<u8>)> {
    if frame.len() < 3 {
        return None;
    }
    let typ = frame[0];
    let len = u16::from_le_bytes([frame[1], frame[2]]) as usize;
    if frame.len() < 3 + len {
        return None;
    }
    Some((typ, frame[3..3 + len].to_vec()))
}

async fn connect_ws(url: &str) -> Result<WebSocketStream<TcpStream>, String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("ws url: {e}"))?;
    if parsed.scheme() != "ws" {
        return Err("only ws:// is supported".into());
    }
    let host = parsed.host_str().ok_or("ws host")?;
    let port = parsed.port_or_known_default().unwrap_or(80);
    let addr = format!("{host}:{port}");
    let tcp = TcpStream::connect(&addr)
        .await
        .map_err(|e| format!("ws tcp {addr}: {e}"))?;
    let req = url
        .into_client_request()
        .map_err(|e| format!("ws request: {e}"))?;
    let (ws, _) = client_async(req, tcp)
        .await
        .map_err(|e| format!("ws handshake: {e}"))?;
    Ok(ws)
}
