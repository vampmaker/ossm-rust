use std::time::{Duration, Instant};

use ossm_common::{LoopStatsWindow, Pll, PllConfig, Wake};
use ossm_core::paths::{self, PathError};
use ossm_core::rpc::RpcAction;
use ossm_core::{Command, CoreError, Engine, MotorControllerConfig, StateResponse};
use tokio::sync::{mpsc, oneshot, watch, Notify};

use crate::link_stats::{instant_to_mono, micros_now, mono_deadline, mono_us, SharedLinkStats};
use crate::persist::{persistent_motor_changed, Persist, SavedConfig};

pub enum EngineMsg {
    TrySetConfig {
        cfg: MotorControllerConfig,
        reply: oneshot::Sender<Result<MotorControllerConfig, CoreError>>,
    },
    SetPaused {
        paused: bool,
        position: Option<f32>,
        reply: oneshot::Sender<MotorControllerConfig>,
    },
    Rpc {
        req: Vec<u8>,
        reply: oneshot::Sender<(RpcAction, Vec<u8>)>,
    },
    PathGet {
        path: String,
        reply: oneshot::Sender<Result<String, PathError>>,
    },
    PathSet {
        path: String,
        value: String,
        reply: oneshot::Sender<Result<String, PathError>>,
    },
    Apply(Command),
}

#[derive(Clone)]
pub struct EngineHandle {
    pub tx: mpsc::Sender<EngineMsg>,
    pub snap: watch::Receiver<StateResponse>,
}

pub struct EngineTaskOpts {
    pub persist: Persist,
    pub saved: SavedConfig,
    pub mock: bool,
    pub serial: Option<crate::bus::ModbusBus>,
    pub link: SharedLinkStats,
}

pub async fn run_engine_task(
    mut rx: mpsc::Receiver<EngineMsg>,
    snap_tx: watch::Sender<StateResponse>,
    mut opts: EngineTaskOpts,
) {
    let mut engine = Engine::new(opts.saved.motor);

    let _ = snap_tx.send(*engine.snapshot());

    let mut write_bus = false;
    if opts.mock {
        opts.link.set_link_info(ossm_common::LinkTransport::Mock);
        engine.apply(Command::HomingComplete {
            pos_min: 0.0,
            pos_max: 100.0,
            position: 50.0,
        });
        engine.flush_snapshot();
    } else if let Some(port) = opts.serial.as_mut() {
        let mut enabled = false;
        for attempt in 1..=12 {
            match port.enable_modbus().await {
                Ok(()) => {
                    enabled = true;
                    break;
                }
                Err(e) => {
                    tracing::warn!("enable modbus ({attempt}/12): {e}");
                    tokio::time::sleep(Duration::from_millis(400)).await;
                }
            }
        }
        if !enabled {
            tracing::warn!("enable modbus: giving up after retries");
        }
        let home = tokio::select! {
            r = port.homing() => r,
            _ = pump_loop(&mut rx, &mut engine, &opts.persist, &snap_tx) => {
                Err("engine channel closed during homing".into())
            }
        };
        match home {
            Ok((pos_min, pos_max)) => {
                let position = port
                    .read_position_radians()
                    .await
                    .unwrap_or((pos_min + pos_max) / 2.0);
                engine.apply(Command::SetMotorConnected(true));
                engine.apply(Command::HomingComplete {
                    pos_min,
                    pos_max,
                    position,
                });
                engine.flush_snapshot();
                engine.reset_cycle_clock(micros_now());
                if let Err(e) = port.apply_run_gains().await {
                    tracing::warn!("run gains: {e}");
                }
                port.spawn_stream_worker();
                write_bus = true;
            }
            Err(e) => {
                tracing::error!("homing failed: {e}");
                engine.apply(Command::SetMotorConnected(false));
                opts.link.set_connected(false);
                engine.flush_snapshot();
            }
        }
    }

    let mut last_saved_motor = engine.snapshot().config;
    let mut last_persist = Instant::now();
    let _ = snap_tx.send(*engine.snapshot());

    run_motion_loop(
        &mut rx,
        &mut engine,
        &mut opts,
        &snap_tx,
        &mut last_saved_motor,
        &mut last_persist,
        write_bus,
    )
    .await;
}

async fn run_motion_loop(
    rx: &mut mpsc::Receiver<EngineMsg>,
    engine: &mut Engine,
    opts: &mut EngineTaskOpts,
    snap_tx: &watch::Sender<StateResponse>,
    last_saved_motor: &mut MotorControllerConfig,
    last_persist: &mut Instant,
    write_bus: bool,
) {
    let cfg = opts
        .serial
        .as_ref()
        .map(|b| b.pll_config())
        .unwrap_or_else(PllConfig::network);
    let mut pll = Pll::new(cfg);
    let dummy_notify = std::sync::Arc::new(Notify::new());
    let mut last_pll_log = Instant::now();
    let mut loop_window = LoopStatsWindow::new();
    let mut last_tick_us = micros_now();
    loop_window.reset_clock(last_tick_us);

    loop {
        let notify = opts
            .serial
            .as_ref()
            .and_then(|b| b.ack_notify())
            .unwrap_or_else(|| dummy_notify.clone());
        let wake = if write_bus {
            pll.next_wake(mono_us())
        } else {
            Wake::At(mono_us().saturating_add(5_000))
        };
        let cap = Duration::from_micros(u64::from(
            pll.info().period_us.max(2_500).saturating_add(2_000),
        ));
        tokio::select! {
            msg = rx.recv() => {
                let Some(msg) = msg else { break; };
                handle_msg(engine, msg, &opts.persist);
            }
            _ = wait_wake(wake, &notify, cap) => {
                drain_acks(opts, &mut pll);
                let now = micros_now();
                let dt_ms = (now.saturating_sub(last_tick_us)) as f32 / 1000.0;
                last_tick_us = now;
                loop_window.record(now, dt_ms, engine.snapshot().position);
                if let Some(stats) = loop_window.take_flushed() {
                    engine.apply(Command::SetLoopStats(stats));
                }
                if let Some(stats) = opts.link.take_flushed() {
                    engine.apply(Command::SetLinkStats(stats));
                }

                let out = engine.tick(now);
                if write_bus && travel_homed(engine) {
                    if let Err(e) = stream_position(opts, &mut pll, out.position).await {
                        tracing::debug!("stream write: {e}");
                        engine.apply(Command::SetMotorConnected(false));
                    } else {
                        engine.apply(Command::SetMotorConnected(true));
                    }
                    drain_acks(opts, &mut pll);
                    opts.link.set_pacing(pll.info());
                    if last_pll_log.elapsed() >= Duration::from_secs(1) {
                        last_pll_log = Instant::now();
                        let snap = engine.snapshot();
                        let info = pll.info();
                        tracing::info!(
                            state = info.state.as_str(),
                            period_us = info.period_us,
                            delay_us = info.delay_us,
                            rtt_min_us = info.rtt_min_us,
                            rtt_ewma_us = info.rtt_ewma_us,
                            ups = snap.loop_stats.ups,
                            dt_avg_ms = snap.loop_stats.dt_avg_ms,
                            dt_max_ms = snap.loop_stats.dt_max_ms,
                            "motion"
                        );
                    }
                } else {
                    engine.apply(Command::SetMotorConnected(opts.mock));
                }
                after_tick(engine, opts, snap_tx, last_saved_motor, last_persist);
            }
        }
    }
}

async fn wait_wake(wake: Wake, notify: &Notify, cap: Duration) {
    match wake {
        Wake::Now => {}
        Wake::OnAck => {
            tokio::select! {
                _ = notify.notified() => {}
                _ = tokio::time::sleep(cap) => {}
            }
        }
        Wake::At(t) => {
            let now = Instant::now();
            let deadline = mono_deadline(t).min(now + cap);
            if deadline > now {
                let left = deadline.saturating_duration_since(now);
                if left > Duration::from_millis(2) {
                    tokio::time::sleep(left - Duration::from_millis(1)).await;
                }
                while Instant::now() < deadline {
                    std::hint::spin_loop();
                }
            }
        }
    }
}

fn drain_acks(opts: &EngineTaskOpts, pll: &mut Pll) {
    if let Some(bus) = opts.serial.as_ref() {
        if let Some(sample) = bus.take_ack_sample() {
            let t_tx = instant_to_mono(sample.t_tx);
            match sample.t_rx {
                Some(t_rx) => pll.on_ack(t_tx, instant_to_mono(t_rx)),
                None => pll.on_timeout(t_tx, mono_us()),
            }
        }
    }
}

async fn stream_position(
    opts: &mut EngineTaskOpts,
    pll: &mut Pll,
    position: f32,
) -> Result<(), String> {
    let Some(bus) = opts.serial.as_mut() else {
        return Ok(());
    };
    if bus.has_async_stream() {
        if !bus.stream_worker_alive() {
            return Err("stream worker ended".into());
        }
        bus.submit_position(position)?;
        pll.on_tx(mono_us());
        return Ok(());
    }
    match bus.write_position_radians(position).await {
        Ok(t) => {
            pll.on_tx(t.t_tx_us);
            match t.t_rx_us {
                Some(rx) => pll.on_ack(t.t_tx_us, rx),
                None => pll.on_timeout(t.t_tx_us, mono_us()),
            }
            Ok(())
        }
        Err(e) => {
            pll.on_timeout(mono_us(), mono_us());
            opts.link.record_failure(micros_now());
            Err(e)
        }
    }
}

fn travel_homed(engine: &Engine) -> bool {
    let snap = engine.snapshot();
    snap.pos_min != snap.pos_max
}

fn after_tick(
    engine: &mut Engine,
    opts: &EngineTaskOpts,
    snap_tx: &watch::Sender<StateResponse>,
    last_saved_motor: &mut MotorControllerConfig,
    last_persist: &mut Instant,
) {
    let _ = snap_tx.send(*engine.snapshot());

    if last_persist.elapsed() >= Duration::from_secs(2) {
        *last_persist = Instant::now();
        let cfg = engine.snapshot().config;
        if persistent_motor_changed(last_saved_motor, &cfg) {
            *last_saved_motor = cfg;
            let persist = opts.persist.clone();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = persist.save_motor(cfg) {
                    tracing::warn!("persist motor: {e}");
                }
            });
        }
    }
}

async fn pump_loop(
    rx: &mut mpsc::Receiver<EngineMsg>,
    engine: &mut Engine,
    persist: &Persist,
    snap_tx: &watch::Sender<StateResponse>,
) {
    loop {
        let Some(msg) = rx.recv().await else {
            return;
        };
        handle_msg(engine, msg, persist);
        let _ = snap_tx.send(*engine.snapshot());
    }
}

fn handle_msg(engine: &mut Engine, msg: EngineMsg, persist: &Persist) {
    match msg {
        EngineMsg::TrySetConfig { cfg, reply } => {
            let _ = reply.send(engine.try_set_config(cfg));
        }
        EngineMsg::SetPaused {
            paused,
            position,
            reply,
        } => {
            engine.apply(Command::SetPaused { paused, position });
            let _ = reply.send(engine.snapshot().config);
        }
        EngineMsg::Rpc { req, reply } => {
            let mut out = vec![0u8; 4096];
            let (action, n) = ossm_core::rpc::dispatch_rpc(engine, &req, &mut out);
            out.truncate(n);
            let _ = reply.send((action, out));
        }
        EngineMsg::PathGet { path, reply } => {
            let _ = reply.send(paths::get(engine, &path));
        }
        EngineMsg::PathSet { path, value, reply } => {
            let result = path_set(engine, persist, &path, &value);
            let _ = reply.send(result);
        }
        EngineMsg::Apply(cmd) => {
            engine.apply(cmd);
        }
    }
}

fn path_set(
    engine: &mut Engine,
    persist: &Persist,
    path: &str,
    value: &str,
) -> Result<String, PathError> {
    let (section, key) = paths::parse_path(path).ok_or(PathError::InvalidPath)?;
    if section == "motor" && key.is_none() {
        let cfg: MotorControllerConfig =
            serde_json::from_str(value).map_err(|_| PathError::InvalidValue)?;
        engine.try_set_config(cfg).map_err(|e| match e {
            CoreError::StaleVersion => PathError::StaleVersion,
            _ => PathError::InvalidValue,
        })?;
        let _ = persist.save_motor(engine.snapshot().config);
        return Ok(String::from(value));
    }
    let shown = paths::set(engine, path, value)?;
    if section == "motor" {
        let _ = persist.save_motor(engine.snapshot().config);
    }
    Ok(shown)
}
