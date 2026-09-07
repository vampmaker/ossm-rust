use std::time::Duration;

use ossm_core::paths::PathError;
use ossm_core::rpc::RpcAction;
use ossm_core::{Command, CoreError, Engine, MotorControllerConfig, StateResponse};
use tokio::sync::{mpsc, oneshot, watch};

use crate::link_stats::SharedLinkStats;
use crate::motor_thread::{self, MotorThreadOpts};
use crate::persist::{Persist, SavedConfig};
use crate::precise_wait::{Waiter, Waker};

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
    pub wake: Waker,
    pub snap: watch::Receiver<StateResponse>,
}

impl EngineHandle {
    pub fn send(&self, msg: EngineMsg) -> Result<(), EngineMsg> {
        self.tx.try_send(msg).map_err(|e| match e {
            mpsc::error::TrySendError::Full(m) | mpsc::error::TrySendError::Closed(m) => m,
        })?;
        self.wake.notify();
        Ok(())
    }
}

pub struct EngineTaskOpts {
    pub persist: Persist,
    pub saved: SavedConfig,
    pub mock: bool,
    pub serial: Option<crate::bus::ModbusBus>,
    pub link: SharedLinkStats,
    pub wake: Waker,
    pub waiter: Waiter,
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
                engine.reset_cycle_clock(crate::link_stats::micros_now());
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

    let _ = snap_tx.send(*engine.snapshot());
    let stream = opts.serial.as_ref().and_then(|b| b.stream_handle());
    let thread_opts = MotorThreadOpts {
        engine,
        rx,
        snap_tx,
        persist: opts.persist,
        link: opts.link,
        stream,
        serial: opts.serial,
        write_bus,
        mock: opts.mock,
        wake: opts.wake,
        waiter: opts.waiter,
    };
    if let Err(e) = std::thread::Builder::new()
        .name("ossm-motor".into())
        .spawn(move || motor_thread::run(thread_opts))
    {
        tracing::error!("spawn ossm-motor: {e}");
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
        motor_thread::handle_msg(engine, msg, persist);
        let _ = snap_tx.send(*engine.snapshot());
    }
}
