//! wasm-bindgen surface. Built only for `wasm32`.

use js_sys::{Function, Promise, Uint8Array};
use ossm_core::{MotorControllerConfig, PausedControl};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use crate::rtu::{
    apply_run_gains, enable_modbus, homing, position_write_frame, read_position_radians,
    write_position_radians, DEFAULT_SLAVE, GENERIC_TIMEOUT_MS, STREAM_TIMEOUT_MS,
};
use crate::shell::{Shell, ShellAction};

fn js_err(msg: impl ToString) -> JsValue {
    JsValue::from_str(&msg.to_string())
}

fn now_us_js() -> u64 {
    let global = js_sys::global();
    let performance = js_sys::Reflect::get(&global, &JsValue::from_str("performance"))
        .ok()
        .filter(|v| !v.is_undefined());
    if let Some(perf) = performance {
        if let Ok(now_fn) = js_sys::Reflect::get(&perf, &JsValue::from_str("now")) {
            if let Ok(func) = now_fn.dyn_into::<Function>() {
                if let Ok(ms) = func.call0(&perf) {
                    if let Some(v) = ms.as_f64() {
                        return (v * 1000.0) as u64;
                    }
                }
            }
        }
    }
    0
}

async fn sleep_ms(ms: u32) {
    let promise = Promise::new(&mut |resolve, _reject| {
        let global = js_sys::global();
        let set_timeout = match js_sys::Reflect::get(&global, &JsValue::from_str("setTimeout")) {
            Ok(v) => v,
            Err(_) => return,
        };
        let Ok(func) = set_timeout.dyn_into::<Function>() else {
            return;
        };
        let _ = func.call2(&global, &resolve, &JsValue::from(ms));
    });
    let _ = JsFuture::from(promise).await;
}

async fn js_exchange(f: &Function, req: &[u8], timeout_ms: u32) -> Result<Vec<u8>, String> {
    let arr = Uint8Array::from(req);
    let result = f
        .call2(&JsValue::NULL, &arr, &JsValue::from(timeout_ms))
        .map_err(|e| format!("exchange call: {e:?}"))?;
    let promise = Promise::from(result);
    let val = JsFuture::from(promise)
        .await
        .map_err(|e| format!("exchange: {e:?}"))?;
    let out = Uint8Array::new(&val);
    let mut buf = vec![0u8; out.length() as usize];
    out.copy_to(&mut buf);
    Ok(buf)
}

#[wasm_bindgen]
pub struct OssmShell {
    inner: Shell,
    exchange: Option<Function>,
    last_action: String,
}

#[wasm_bindgen]
impl OssmShell {
    #[wasm_bindgen(constructor)]
    pub fn new() -> OssmShell {
        console_error_panic_hook::set_once();
        OssmShell {
            inner: Shell::new(MotorControllerConfig::default()),
            exchange: None,
            last_action: "respond".into(),
        }
    }

    #[wasm_bindgen(js_name = loadMotorJson)]
    pub fn load_motor_json(&mut self, json: &str) -> Result<(), JsValue> {
        let cfg: MotorControllerConfig =
            serde_json::from_str(json).map_err(|e| js_err(format!("motor json: {e}")))?;
        self.inner.load_motor(cfg);
        Ok(())
    }

    /// JS `async (req: Uint8Array, timeoutMs: number) => Uint8Array`
    #[wasm_bindgen(js_name = setExchange)]
    pub fn set_exchange(&mut self, f: Function) {
        self.exchange = Some(f);
    }

    #[wasm_bindgen(js_name = clearExchange)]
    pub fn clear_exchange(&mut self) {
        self.exchange = None;
        self.inner.set_homing_failed();
    }

    #[wasm_bindgen(js_name = applyMockHome)]
    pub fn apply_mock_home(&mut self) {
        self.exchange = None;
        self.inner.apply_mock_home();
    }

    #[wasm_bindgen]
    pub async fn home(&mut self, now_us: u64) -> Result<(), JsValue> {
        let Some(f) = self.exchange.clone() else {
            return Err(js_err("no RTU exchange"));
        };
        let mut exchange = |req: Vec<u8>, timeout_ms: u32| {
            let f = f.clone();
            async move { js_exchange(&f, &req, timeout_ms).await }
        };
        let mut sleep = |ms: u32| async move { sleep_ms(ms).await };

        for attempt in 1..=12 {
            match enable_modbus(&mut exchange, DEFAULT_SLAVE).await {
                Ok(()) => break,
                Err(e) => {
                    log::warn!("enable modbus ({attempt}/12): {e}");
                    if attempt == 12 {
                        self.inner.set_homing_failed();
                        return Err(js_err(e));
                    }
                    sleep_ms(400).await;
                }
            }
        }

        match homing(&mut exchange, &mut sleep, DEFAULT_SLAVE).await {
            Ok((pos_min, pos_max)) => {
                let position = read_position_radians(&mut exchange, DEFAULT_SLAVE)
                    .await
                    .unwrap_or((pos_min + pos_max) / 2.0);
                if let Err(e) = apply_run_gains(&mut exchange, DEFAULT_SLAVE).await {
                    log::warn!("run gains: {e}");
                }
                let now = match now_us_js() {
                    0 => now_us,
                    n => n,
                };
                self.inner
                    .apply_homing_complete(pos_min, pos_max, position, now);
                Ok(())
            }
            Err(e) => {
                self.inner.set_homing_failed();
                Err(js_err(e))
            }
        }
    }

    /// JSON-RPC 2.0 (motion catalog). Returns the JSON-RPC response body.
    pub fn rpc(&mut self, request: &str) -> String {
        let reply = self.inner.rpc(request.as_bytes());
        self.last_action = match reply.action {
            ShellAction::Respond => "respond".into(),
            ShellAction::Subscribe { .. } => "subscribe".into(),
            ShellAction::Unsubscribe => "unsubscribe".into(),
            ShellAction::Restart => "restart".into(),
        };
        String::from_utf8(reply.body).unwrap_or_else(|_| {
            "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32603,\"message\":\"Internal error\"}}"
                .into()
        })
    }

    #[wasm_bindgen(js_name = lastAction)]
    pub fn last_action(&self) -> String {
        self.last_action.clone()
    }

    pub fn paused(&mut self, json: &str) -> Result<String, JsValue> {
        let control: PausedControl =
            serde_json::from_str(json).map_err(|e| js_err(format!("paused json: {e}")))?;
        let applied = self
            .inner
            .set_paused(control)
            .map_err(|e| js_err(e.to_string()))?;
        serde_json::to_string(&applied).map_err(|e| js_err(e.to_string()))
    }

    #[wasm_bindgen(js_name = setLinkInfo)]
    pub fn set_link_info(&mut self, transport: &str, endpoint: &str) {
        self.inner.set_link_info(transport, endpoint);
    }

    #[wasm_bindgen(js_name = pllReset)]
    pub fn pll_reset(&mut self) {
        self.inner.pll_reset();
    }

    #[wasm_bindgen(js_name = nextWakeUs)]
    pub fn next_wake_us(&self, now_us: u64) -> i64 {
        self.inner.next_wake_us(now_us)
    }

    #[wasm_bindgen(js_name = recordBusSample)]
    pub fn record_bus_sample(
        &mut self,
        rtt_us: u16,
        ok: bool,
        tx_len: u32,
        rx_len: u32,
        now_us: u64,
    ) {
        self.inner
            .record_bus_sample(rtt_us, ok, tx_len, rx_len, now_us);
    }

    #[wasm_bindgen(js_name = linkStatsJson)]
    pub fn link_stats_json(&mut self, now_us: u64) -> String {
        self.inner.link_stats_json(now_us)
    }

    pub fn tick(&mut self, now_us: u64) -> f32 {
        self.inner.tick(now_us)
    }

    #[wasm_bindgen(js_name = writeBus)]
    pub fn write_bus(&self) -> bool {
        self.inner.write_bus()
    }

    #[wasm_bindgen(js_name = positionFrame)]
    pub fn position_frame(&self, position: f32) -> Result<Vec<u8>, JsValue> {
        position_write_frame(DEFAULT_SLAVE, position).map_err(js_err)
    }

    #[wasm_bindgen(js_name = streamTimeoutMs)]
    pub fn stream_timeout_ms(&self) -> u32 {
        STREAM_TIMEOUT_MS
    }

    #[wasm_bindgen(js_name = genericTimeoutMs)]
    pub fn generic_timeout_ms(&self) -> u32 {
        GENERIC_TIMEOUT_MS
    }

    #[wasm_bindgen(js_name = snapshotJson)]
    pub fn snapshot_json(&self) -> String {
        self.inner.snapshot_json()
    }

    #[wasm_bindgen(js_name = shouldNotify)]
    pub fn should_notify(&mut self, now_us: u64) -> bool {
        self.inner.should_notify(now_us)
    }

    #[wasm_bindgen(js_name = stateNotificationJson)]
    pub fn state_notification_json(&self) -> Option<String> {
        self.inner.state_notification_json()
    }

    #[wasm_bindgen(js_name = takePersistJson)]
    pub fn take_persist_json(&mut self, now_us: u64) -> Option<String> {
        self.inner
            .take_persist(now_us)
            .and_then(|cfg| serde_json::to_string(&cfg).ok())
    }

    /// Streaming position write (CRC miss is not fatal).
    #[wasm_bindgen(js_name = writePosition)]
    pub async fn write_position(&mut self, position: f32) -> Result<(), JsValue> {
        let Some(f) = self.exchange.clone() else {
            return Ok(());
        };
        let mut exchange = |req: Vec<u8>, timeout_ms: u32| {
            let f = f.clone();
            async move { js_exchange(&f, &req, timeout_ms).await }
        };
        let t_tx = now_us_js();
        self.inner.on_tx(t_tx);
        match write_position_radians(
            &mut exchange,
            DEFAULT_SLAVE,
            position,
            STREAM_TIMEOUT_MS,
            false,
        )
        .await
        {
            Ok(()) => {
                self.inner.on_ack(t_tx, now_us_js());
                self.inner.set_motor_connected(true);
                Ok(())
            }
            Err(e) => {
                self.inner.on_timeout(t_tx, now_us_js());
                log::warn!("stream write: {e}");
                Ok(())
            }
        }
    }
}

impl Default for OssmShell {
    fn default() -> Self {
        Self::new()
    }
}
