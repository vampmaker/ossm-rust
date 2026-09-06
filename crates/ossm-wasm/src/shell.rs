//! Engine owner: RPC, pause, tick, mock home, persist dirty flag.

use ossm_common::{LinkStatsWindow, LoopStatsWindow, Pll, PllConfig, Wake};
use ossm_core::rpc::{
    build_state_notification_into, dispatch_rpc, write_restart_ack, write_subscribe_ack,
    write_unsubscribe_ack, RpcAction,
};
use ossm_core::{Command, CoreError, Engine, MotorControllerConfig, PausedControl, StateResponse};

pub fn persistent_motor_changed(a: &MotorControllerConfig, b: &MotorControllerConfig) -> bool {
    a.persistent_motor_changed(b)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellAction {
    Respond,
    Subscribe { interval_ms: u64 },
    Unsubscribe,
    Restart,
}

pub struct RpcReply {
    pub action: ShellAction,
    pub body: Vec<u8>,
}

pub struct Shell {
    engine: Engine,
    mock: bool,
    write_bus: bool,
    last_saved: MotorControllerConfig,
    last_persist_us: u64,
    subscribe_interval_ms: Option<u64>,
    last_notify_us: Option<u64>,
    loop_window: LoopStatsWindow,
    link_window: LinkStatsWindow,
    last_tick_us: u64,
    pll: Pll,
}

impl Shell {
    pub fn new(motor: MotorControllerConfig) -> Self {
        let engine = Engine::new(motor);
        let last_saved = engine.snapshot().config;
        Self {
            engine,
            mock: false,
            write_bus: false,
            last_saved,
            last_persist_us: 0,
            subscribe_interval_ms: None,
            last_notify_us: None,
            loop_window: LoopStatsWindow::new(),
            link_window: LinkStatsWindow::new(),
            last_tick_us: 0,
            pll: Pll::new(PllConfig::network()),
        }
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    pub fn snapshot(&self) -> &StateResponse {
        self.engine.snapshot()
    }

    pub fn snapshot_json(&self) -> String {
        serde_json::to_string(self.engine.snapshot()).unwrap_or_else(|_| "{}".into())
    }

    pub fn apply_mock_home(&mut self) {
        self.mock = true;
        self.write_bus = false;
        self.link_window
            .set_link_info(ossm_common::LinkTransport::Mock);
        self.engine.apply(Command::HomingComplete {
            pos_min: 0.0,
            pos_max: 100.0,
            position: 50.0,
        });
        self.engine.apply(Command::SetMotorConnected(true));
        self.engine.flush_snapshot();
    }

    pub fn apply_homing_complete(
        &mut self,
        pos_min: f32,
        pos_max: f32,
        position: f32,
        now_us: u64,
    ) {
        self.mock = false;
        self.write_bus = true;
        self.engine.apply(Command::SetMotorConnected(true));
        self.engine.apply(Command::HomingComplete {
            pos_min,
            pos_max,
            position,
        });
        self.engine.flush_snapshot();
        self.engine.reset_cycle_clock(now_us);
        self.loop_window.reset_clock(now_us);
        self.last_tick_us = now_us;
    }

    pub fn set_homing_failed(&mut self) {
        self.write_bus = false;
        self.engine.apply(Command::SetMotorConnected(false));
        self.engine.flush_snapshot();
    }

    pub fn travel_homed(&self) -> bool {
        let snap = self.engine.snapshot();
        snap.pos_min != snap.pos_max
    }

    pub fn write_bus(&self) -> bool {
        self.write_bus && self.travel_homed()
    }

    pub fn is_mock(&self) -> bool {
        self.mock
    }

    pub fn load_motor(&mut self, cfg: MotorControllerConfig) {
        self.engine = Engine::new(cfg);
        self.last_saved = self.engine.snapshot().config;
    }

    pub fn set_paused(
        &mut self,
        control: PausedControl,
    ) -> Result<MotorControllerConfig, CoreError> {
        let mut cfg = self.engine.snapshot().config;
        if let Some(p) = control.paused {
            cfg.paused = p;
        }
        let pos = control.position.or(control.paused_position);
        if let Some(p) = pos {
            cfg.paused_position = p;
        }
        if let Some(adj) = control.adjust {
            cfg.paused_position = (cfg.paused_position + adj).clamp(0.0, 1.0);
        }
        self.engine.apply(ossm_core::Command::SetPaused {
            paused: cfg.paused,
            position: Some(cfg.paused_position),
        });
        Ok(self.engine.snapshot().config)
    }

    pub fn rpc(&mut self, request: &[u8]) -> RpcReply {
        let mut out = vec![0u8; 8192];
        let (action, n) = dispatch_rpc(&mut self.engine, request, &mut out);
        match action {
            RpcAction::Respond => {
                out.truncate(n);
                RpcReply {
                    action: ShellAction::Respond,
                    body: out,
                }
            }
            RpcAction::Subscribe { interval_ms } => {
                self.subscribe_interval_ms = Some(interval_ms);
                self.last_notify_us = None;
                let id = rpc_id(request);
                let n = write_subscribe_ack(&id, interval_ms, &mut out).unwrap_or(0);
                out.truncate(n);
                RpcReply {
                    action: ShellAction::Subscribe { interval_ms },
                    body: out,
                }
            }
            RpcAction::Unsubscribe => {
                self.subscribe_interval_ms = None;
                self.last_notify_us = None;
                let id = rpc_id(request);
                let n = write_unsubscribe_ack(&id, &mut out).unwrap_or(0);
                out.truncate(n);
                RpcReply {
                    action: ShellAction::Unsubscribe,
                    body: out,
                }
            }
            RpcAction::Restart => {
                let id = rpc_id(request);
                let n = write_restart_ack(&id, &mut out).unwrap_or(0);
                out.truncate(n);
                RpcReply {
                    action: ShellAction::Restart,
                    body: out,
                }
            }
        }
    }

    pub fn set_link_info(&mut self, transport: &str, _endpoint: &str) {
        let t = ossm_common::LinkTransport::from_wire(transport);
        self.link_window.set_link_info(t);
        self.link_window.set_connected(true);
        let cfg = if t == ossm_common::LinkTransport::Serial
            || t == ossm_common::LinkTransport::UsbHost
        {
            PllConfig::usb_serial()
        } else {
            PllConfig::network()
        };
        self.pll = Pll::new(cfg);
    }

    pub fn pll_reset(&mut self) {
        self.pll.reset();
    }

    pub fn next_wake_us(&self, now_us: u64) -> i64 {
        match self.pll.next_wake(now_us) {
            Wake::Now => 0,
            Wake::At(t) => t.saturating_sub(now_us) as i64,
            Wake::OnAck => -1,
        }
    }

    pub fn on_tx(&mut self, t_tx_us: u64) {
        self.pll.on_tx(t_tx_us);
    }

    pub fn on_ack(&mut self, t_tx_us: u64, t_rx_us: u64) {
        self.pll.on_ack(t_tx_us, t_rx_us);
        self.link_window.set_pacing(&self.pll.info());
    }

    pub fn on_timeout(&mut self, t_tx_us: u64, now_us: u64) {
        self.pll.on_timeout(t_tx_us, now_us);
        self.link_window.set_pacing(&self.pll.info());
    }

    pub fn record_bus_sample(
        &mut self,
        rtt_us: u16,
        ok: bool,
        tx_len: u32,
        rx_len: u32,
        now_us: u64,
    ) {
        if ok {
            self.link_window
                .record_success(now_us, rtt_us, None, None, tx_len, rx_len);
        } else {
            self.link_window.record_failure(now_us);
        }
    }

    pub fn link_stats_json(&mut self, now_us: u64) -> String {
        serde_json::to_string(&ossm_core::LinkStatsSer(&self.link_window.poll(now_us)))
            .unwrap_or_else(|_| "{}".into())
    }

    pub fn tick(&mut self, now_us: u64) -> f32 {
        let dt_ms = if self.last_tick_us == 0 {
            0.0
        } else {
            (now_us.saturating_sub(self.last_tick_us)) as f32 / 1000.0
        };
        self.last_tick_us = now_us;
        self.loop_window
            .record(now_us, dt_ms, self.engine.snapshot().position);
        if let Some(stats) = self.loop_window.take_flushed() {
            self.engine.apply(Command::SetLoopStats(stats));
        }
        if let Some(stats) = self.link_window.take_flushed() {
            self.engine.apply(Command::SetLinkStats(stats));
        }
        let out = self.engine.tick(now_us);
        if self.mock {
            self.engine.apply(Command::SetMotorConnected(true));
        }
        out.position
    }

    pub fn set_motor_connected(&mut self, connected: bool) {
        self.engine.apply(Command::SetMotorConnected(connected));
    }

    pub fn should_notify(&mut self, now_us: u64) -> bool {
        let Some(interval_ms) = self.subscribe_interval_ms else {
            return false;
        };
        let interval_us = interval_ms.saturating_mul(1000);
        let due = match self.last_notify_us {
            None => true,
            Some(last) => now_us.saturating_sub(last) >= interval_us,
        };
        if due {
            self.last_notify_us = Some(now_us);
            true
        } else {
            false
        }
    }

    pub fn state_notification_json(&self) -> Option<String> {
        let mut out = vec![0u8; 8192];
        let n = build_state_notification_into(self.engine.snapshot(), &mut out)?;
        String::from_utf8(out[..n].to_vec()).ok()
    }

    /// Motor JSON to persist when dirty and ≥ 2 s since last check.
    pub fn take_persist(&mut self, now_us: u64) -> Option<MotorControllerConfig> {
        if now_us.saturating_sub(self.last_persist_us) < 2_000_000 {
            return None;
        }
        self.last_persist_us = now_us;
        let cfg = self.engine.snapshot().config;
        if persistent_motor_changed(&self.last_saved, &cfg) {
            self.last_saved = cfg;
            Some(cfg)
        } else {
            None
        }
    }
}

fn rpc_id(request: &[u8]) -> serde_json::Value {
    serde_json::from_slice::<serde_json::Value>(request)
        .ok()
        .and_then(|v| v.get("id").cloned())
        .unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_home_virtual_range() {
        let mut shell = Shell::new(MotorControllerConfig::default());
        shell.apply_mock_home();
        let snap = shell.snapshot();
        assert_eq!(snap.pos_min, 0.0);
        assert_eq!(snap.pos_max, 100.0);
        assert!(snap.motor_connected);
        assert!(snap.config.paused);
    }

    #[test]
    fn set_config_bumps_version_and_stale_rejects() {
        let mut shell = Shell::new(MotorControllerConfig::default());
        let mut cfg = shell.snapshot().config;
        cfg.bpm = 40.0;
        cfg.version = 0;
        let applied = shell.engine_mut().try_set_config(cfg).unwrap();
        assert_eq!(applied.bpm, 40.0);
        assert_eq!(applied.version, 1);

        cfg.version = 1;
        cfg.bpm = 41.0;
        let applied = shell.engine_mut().try_set_config(cfg).unwrap();
        assert_eq!(applied.version, 2);

        cfg.version = 1;
        cfg.bpm = 12.0;
        let err = shell.engine_mut().try_set_config(cfg).unwrap_err();
        assert_eq!(err, CoreError::StaleVersion);
    }

    #[test]
    fn rpc_get_state_and_subscribe() {
        let mut shell = Shell::new(MotorControllerConfig::default());
        shell.apply_mock_home();
        let reply = shell.rpc(br#"{"jsonrpc":"2.0","id":1,"method":"get-state"}"#);
        assert_eq!(reply.action, ShellAction::Respond);
        let v: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
        assert!(v["result"]["motor_connected"].as_bool().unwrap());

        let sub = shell.rpc(
            br#"{"jsonrpc":"2.0","id":2,"method":"subscribe-state","params":{"interval_ms":33}}"#,
        );
        assert!(matches!(
            sub.action,
            ShellAction::Subscribe { interval_ms: 33 }
        ));
        assert!(shell.should_notify(0));
        assert!(!shell.should_notify(1_000));
        assert!(shell.should_notify(33_000));
    }

    #[test]
    fn persist_ignores_pause_only() {
        let mut shell = Shell::new(MotorControllerConfig::default());
        let _ = shell.take_persist(3_000_000);
        let mut cfg = shell.snapshot().config;
        cfg.paused = false;
        cfg.paused_position = 0.4;
        let _ = shell.engine_mut().try_set_config(cfg).unwrap();
        assert!(shell.take_persist(6_000_000).is_none());
    }

    #[test]
    fn pll_probe_freq_then_phase_sleeps() {
        let mut shell = Shell::new(MotorControllerConfig::default());
        shell.set_link_info("rs485-ws", "rs485");
        let mut t = 0u64;
        for _ in 0..200 {
            let sleep = shell.next_wake_us(t);
            if sleep > 0 {
                t += sleep as u64;
            }
            shell.on_tx(t);
            let rx = t + 2_000;
            shell.on_ack(t, rx);
            t = rx;
        }
        assert!(
            shell.next_wake_us(t) > 0,
            "probe-phase should request a sleep, got {}",
            shell.next_wake_us(t)
        );
    }
}
