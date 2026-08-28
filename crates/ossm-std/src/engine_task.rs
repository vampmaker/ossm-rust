use std::time::{Duration, Instant};

use ossm_core::modbus::{get_inject_junk, InjectJunkMode};
use ossm_core::rpc::RpcAction;
use ossm_core::{
    Command, CoreError, Engine, MotorControllerConfig, NetworkConfiguration, PinConfiguration,
    StateResponse,
};
use tokio::sync::{mpsc, oneshot, watch};

use crate::persist::{persistent_motor_changed, Persist, SavedConfig};
use crate::serial::SerialPort;

pub enum EngineMsg {
    TrySetConfig {
        cfg: MotorControllerConfig,
        reply: oneshot::Sender<Result<MotorControllerConfig, CoreError>>,
    },
    Rpc {
        req: Vec<u8>,
        reply: oneshot::Sender<(RpcAction, Vec<u8>)>,
    },
    SetPin(PinConfiguration),
    SetNet(NetworkConfiguration),
    GetPin {
        reply: oneshot::Sender<PinConfiguration>,
    },
    GetNet {
        reply: oneshot::Sender<NetworkConfiguration>,
    },
    SetInject {
        mode: InjectJunkMode,
        nbytes: u8,
    },
    GetInject {
        reply: oneshot::Sender<(InjectJunkMode, u8)>,
    },
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
    pub no_homing: bool,
    pub serial: Option<SerialPort>,
}

pub async fn run_engine_task(
    mut rx: mpsc::Receiver<EngineMsg>,
    snap_tx: watch::Sender<StateResponse>,
    mut opts: EngineTaskOpts,
) {
    let mut engine = Engine::with_pin_net(
        opts.saved.motor.clone(),
        opts.saved.pin.clone(),
        opts.saved.net.clone(),
    );

    if opts.no_homing || opts.mock {
        engine.apply(Command::HomingComplete {
            pos_min: 0.0,
            pos_max: 100.0,
            position: 50.0,
        });
        engine.flush_snapshot();
    }

    let mut last_saved_motor = engine.snapshot().config.clone();
    let mut last_persist = Instant::now();
    let mut interval = tokio::time::interval(Duration::from_millis(3));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let _ = snap_tx.send(engine.snapshot().clone());

    loop {
        tokio::select! {
            msg = rx.recv() => {
                let Some(msg) = msg else { break; };
                handle_msg(&mut engine, msg, &opts.persist);
            }
            _ = interval.tick() => {
                let now = micros_now();
                let out = engine.tick(now);
                if let Some(port) = opts.serial.as_mut() {
                    if let Err(e) = port.write_position_radians(out.position).await {
                        tracing::debug!("serial write: {e}");
                        engine.apply(Command::SetMotorConnected(false));
                    } else {
                        engine.apply(Command::SetMotorConnected(true));
                    }
                } else {
                    engine.apply(Command::SetMotorConnected(opts.mock));
                }
                let _ = snap_tx.send(engine.snapshot().clone());

                if last_persist.elapsed() >= Duration::from_secs(2) {
                    last_persist = Instant::now();
                    let cfg = engine.snapshot().config.clone();
                    if persistent_motor_changed(&last_saved_motor, &cfg) {
                        last_saved_motor = cfg.clone();
                        let saved = SavedConfig {
                            pin: engine.pin().clone(),
                            net: engine.net().clone(),
                            motor: cfg,
                        };
                        if let Err(e) = opts.persist.save(&saved) {
                            tracing::warn!("persist motor: {e}");
                        }
                    }
                }
            }
        }
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
        EngineMsg::GetPin { reply } => {
            let _ = reply.send(engine.pin().clone());
        }
        EngineMsg::GetNet { reply } => {
            let _ = reply.send(engine.net().clone());
        }
        EngineMsg::SetPin(pin) => {
            engine.apply(Command::SetPin(pin.clone()));
            let saved = SavedConfig {
                pin,
                net: engine.net().clone(),
                motor: engine.snapshot().config.clone(),
            };
            let _ = persist.save(&saved);
        }
        EngineMsg::SetNet(net) => {
            engine.apply(Command::SetNet(net.clone()));
            let saved = SavedConfig {
                pin: engine.pin().clone(),
                net,
                motor: engine.snapshot().config.clone(),
            };
            let _ = persist.save(&saved);
        }
        EngineMsg::SetInject { mode, nbytes } => {
            engine.apply(Command::SetInject { mode, nbytes });
        }
        EngineMsg::GetInject { reply } => {
            let _ = reply.send(get_inject_junk());
        }
    }
}

fn micros_now() -> ossm_core::Micros {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as ossm_core::Micros)
        .unwrap_or(0)
}
