use std::time::{Duration, Instant};

use ossm_core::paths::{self, PathError};
use ossm_core::rpc::RpcAction;
use ossm_core::{Command, CoreError, Engine, MotorControllerConfig, StateResponse};
use tokio::sync::{mpsc, oneshot, watch};

use crate::bus::ModbusBus;
use crate::persist::{persistent_motor_changed, Persist, SavedConfig};
use crate::usb_pll::FramePll;

pub enum EngineMsg {
    TrySetConfig {
        cfg: MotorControllerConfig,
        reply: oneshot::Sender<Result<MotorControllerConfig, CoreError>>,
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
}

pub async fn run_engine_task(
    mut rx: mpsc::Receiver<EngineMsg>,
    snap_tx: watch::Sender<StateResponse>,
    mut opts: EngineTaskOpts,
) {
    let mut engine = Engine::new(opts.saved.motor.clone());

    let _ = snap_tx.send(engine.snapshot().clone());

    let mut write_bus = false;
    if opts.mock {
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
                write_bus = true;
            }
            Err(e) => {
                tracing::error!("homing failed: {e}");
                engine.apply(Command::SetMotorConnected(false));
                engine.flush_snapshot();
            }
        }
    }

    let mut last_saved_motor = engine.snapshot().config.clone();
    let mut last_persist = Instant::now();
    let _ = snap_tx.send(engine.snapshot().clone());

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
    let mut pll = FramePll::new();
    let use_pll = write_bus && matches!(&opts.serial, Some(ModbusBus::Serial(_)));
    let mut last_pll_log = Instant::now();
    loop {
        tokio::select! {
            msg = rx.recv() => {
                let Some(msg) = msg else { break; };
                handle_msg(engine, msg, &opts.persist);
            }
            _ = wait_next_tx(use_pll, &mut pll) => {
                let now = micros_now();
                let out = engine.tick(now);
                if write_bus && travel_homed(engine) {
                    if use_pll {
                        let Some(port) = opts.serial.as_mut().and_then(|b| b.as_serial_mut()) else {
                            break;
                        };
                        match port.write_position_radians(out.position).await {
                            Ok(timing) => {
                                engine.apply(Command::SetMotorConnected(true));
                                pll.on_done(timing.t_tx, timing.t_rx);
                            }
                            Err(e) => {
                                tracing::debug!("serial write: {e}");
                                engine.apply(Command::SetMotorConnected(false));
                                pll.on_done(Instant::now(), None);
                            }
                        }
                        if last_pll_log.elapsed() >= Duration::from_secs(1) {
                            last_pll_log = Instant::now();
                            let snap = engine.snapshot();
                            tracing::info!(
                                psi = pll.psi_us(),
                                phase_mag = pll.phase_mag(),
                                period = pll.period_us(),
                                e = pll.last_err_us(),
                                i = pll.integral_us(),
                                write_lead = pll.write_lead_us(),
                                rtt_ewma = pll.rtt_ewma_us(),
                                ups = snap.ups,
                                dt_avg_ms = snap.dt_avg_ms,
                                dt_max_ms = snap.dt_max_ms,
                                "pll"
                            );
                        }
                    } else if let Some(port) = opts.serial.as_mut() {
                        if let Err(e) = port.write_position_radians(out.position).await {
                            tracing::debug!("bus write: {e}");
                            engine.apply(Command::SetMotorConnected(false));
                        } else {
                            engine.apply(Command::SetMotorConnected(true));
                        }
                    }
                } else {
                    engine.apply(Command::SetMotorConnected(opts.mock));
                }
                after_tick(engine, opts, snap_tx, last_saved_motor, last_persist);
            }
        }
    }
}

async fn wait_next_tx(use_pll: bool, pll: &mut FramePll) {
    if use_pll {
        pll.wait_tx().await;
    } else {
        tokio::task::yield_now().await;
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
    let _ = snap_tx.send(engine.snapshot().clone());

    if last_persist.elapsed() >= Duration::from_secs(2) {
        *last_persist = Instant::now();
        let cfg = engine.snapshot().config.clone();
        if persistent_motor_changed(last_saved_motor, &cfg) {
            *last_saved_motor = cfg.clone();
            let saved = SavedConfig { motor: cfg };
            if let Err(e) = opts.persist.save(&saved) {
                tracing::warn!("persist motor: {e}");
            }
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
        let _ = snap_tx.send(engine.snapshot().clone());
    }
}

fn handle_msg(engine: &mut Engine, msg: EngineMsg, persist: &Persist) {
    match msg {
        EngineMsg::TrySetConfig { cfg, reply } => {
            let _ = reply.send(engine.try_set_config(cfg));
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
        let saved = SavedConfig {
            motor: engine.snapshot().config.clone(),
        };
        let _ = persist.save(&saved);
        return Ok(String::from(value));
    }
    let shown = paths::set(engine, path, value)?;
    if section == "motor" {
        let saved = SavedConfig {
            motor: engine.snapshot().config.clone(),
        };
        let _ = persist.save(&saved);
    }
    Ok(shown)
}

fn micros_now() -> ossm_core::Micros {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as ossm_core::Micros)
        .unwrap_or(0)
}
