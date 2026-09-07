//! Dedicated OS thread that owns `Engine` and paces the motor loop.

use std::time::{Duration, Instant};

use ossm_common::{LoopStatsWindow, Pll, PllConfig, Wake};
use ossm_core::paths::{self, PathError};
use ossm_core::{Command, CoreError, Engine, MotorControllerConfig, StateResponse};
use tokio::sync::{mpsc, watch};

use crate::bus::StreamHandle;
use crate::engine_task::EngineMsg;
use crate::link_stats::{instant_to_mono, micros_now, mono_deadline, mono_us, SharedLinkStats};
use crate::persist::{persistent_motor_changed, Persist};
use crate::precise_wait::{Waiter, Waker};

pub struct MotorThreadOpts {
    pub engine: Engine,
    pub rx: mpsc::Receiver<EngineMsg>,
    pub snap_tx: watch::Sender<StateResponse>,
    pub persist: Persist,
    pub link: SharedLinkStats,
    pub stream: Option<StreamHandle>,
    /// Keeps USB-host writer channels alive after `stream_handle()` is cloned.
    #[allow(dead_code)]
    pub serial: Option<crate::bus::ModbusBus>,
    pub write_bus: bool,
    pub mock: bool,
    pub wake: Waker,
    pub waiter: Waiter,
}

pub fn run(mut opts: MotorThreadOpts) {
    #[cfg(target_os = "linux")]
    tighten_timer_slack();
    #[cfg(windows)]
    raise_thread_priority();

    tracing::info!(waiter = opts.waiter.describe(), "motor thread started");

    if let Some(stream) = opts.stream.as_ref() {
        stream.set_ack_waker(opts.wake.clone());
    }

    let cfg = opts
        .stream
        .as_ref()
        .map(|s| s.pll_config())
        .unwrap_or_else(PllConfig::network);
    let mut pll = Pll::new(cfg);
    let mut last_pll_log = Instant::now();
    let mut loop_window = LoopStatsWindow::new();
    let mut last_tick_us = micros_now();
    loop_window.reset_clock(last_tick_us);
    let mut last_saved_motor = opts.engine.snapshot().config;
    let mut last_persist = Instant::now();

    loop {
        match drain_msgs(&mut opts) {
            Drain::Disconnected => break,
            Drain::HadWork => continue,
            Drain::Idle => {}
        }

        drain_acks(opts.stream.as_ref(), &mut pll);

        let now = mono_us();
        let wake = if opts.write_bus {
            pll.next_wake(now)
        } else {
            Wake::At(now.saturating_add(5_000))
        };
        let cap_deadline = mono_deadline(now.saturating_add(u64::from(
            pll.info().period_us.max(2_500).saturating_add(2_000),
        )));

        match wake {
            Wake::Now => {}
            Wake::At(t) => {
                let deadline = mono_deadline(t).min(cap_deadline);
                if opts.waiter.sleep_until(deadline) {
                    continue;
                }
            }
            Wake::OnAck => {
                if opts.waiter.wait_until(Some(cap_deadline)) {
                    continue;
                }
            }
        }

        tick_once(
            &mut opts,
            &mut pll,
            &mut loop_window,
            &mut last_tick_us,
            &mut last_pll_log,
            &mut last_saved_motor,
            &mut last_persist,
        );
    }
}

enum Drain {
    Idle,
    HadWork,
    Disconnected,
}

fn drain_msgs(opts: &mut MotorThreadOpts) -> Drain {
    let mut any = false;
    loop {
        match opts.rx.try_recv() {
            Ok(msg) => {
                handle_msg(&mut opts.engine, msg, &opts.persist);
                any = true;
            }
            Err(mpsc::error::TryRecvError::Empty) => {
                return if any { Drain::HadWork } else { Drain::Idle };
            }
            Err(mpsc::error::TryRecvError::Disconnected) => return Drain::Disconnected,
        }
    }
}

fn tick_once(
    opts: &mut MotorThreadOpts,
    pll: &mut Pll,
    loop_window: &mut LoopStatsWindow,
    last_tick_us: &mut u64,
    last_pll_log: &mut Instant,
    last_saved_motor: &mut MotorControllerConfig,
    last_persist: &mut Instant,
) {
    drain_acks(opts.stream.as_ref(), pll);
    let now = micros_now();
    let dt_ms = (now.saturating_sub(*last_tick_us)) as f32 / 1000.0;
    *last_tick_us = now;
    loop_window.record(now, dt_ms, opts.engine.snapshot().position);
    if let Some(stats) = loop_window.take_flushed() {
        opts.engine.apply(Command::SetLoopStats(stats));
    }
    if let Some(stats) = opts.link.take_flushed() {
        opts.engine.apply(Command::SetLinkStats(stats));
    }

    let out = opts.engine.tick(now);
    if opts.write_bus && travel_homed(&opts.engine) {
        if let Err(e) = stream_position(opts.stream.as_ref(), pll, out.position) {
            tracing::debug!("stream write: {e}");
            opts.engine.apply(Command::SetMotorConnected(false));
        } else {
            opts.engine.apply(Command::SetMotorConnected(true));
        }
        drain_acks(opts.stream.as_ref(), pll);
        opts.link.set_pacing(pll.info());
        if last_pll_log.elapsed() >= Duration::from_secs(1) {
            *last_pll_log = Instant::now();
            let snap = opts.engine.snapshot();
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
        opts.engine.apply(Command::SetMotorConnected(opts.mock));
    }
    after_tick(
        &mut opts.engine,
        &opts.persist,
        &opts.snap_tx,
        last_saved_motor,
        last_persist,
    );
}

fn drain_acks(stream: Option<&StreamHandle>, pll: &mut Pll) {
    let Some(stream) = stream else {
        return;
    };
    if let Some(sample) = stream.take_ack_sample() {
        let t_tx = instant_to_mono(sample.t_tx);
        match sample.t_rx {
            Some(t_rx) => pll.on_ack(t_tx, instant_to_mono(t_rx)),
            None => pll.on_timeout(t_tx, mono_us()),
        }
    }
}

fn stream_position(
    stream: Option<&StreamHandle>,
    pll: &mut Pll,
    position: f32,
) -> Result<(), String> {
    let Some(stream) = stream else {
        return Ok(());
    };
    if !stream.alive() {
        return Err("stream worker ended".into());
    }
    stream.submit_position(position)?;
    pll.on_tx(mono_us());
    Ok(())
}

fn travel_homed(engine: &Engine) -> bool {
    let snap = engine.snapshot();
    snap.pos_min != snap.pos_max
}

fn after_tick(
    engine: &mut Engine,
    persist: &Persist,
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
            let persist = persist.clone();
            std::thread::spawn(move || {
                if let Err(e) = persist.save_motor(cfg) {
                    tracing::warn!("persist motor: {e}");
                }
            });
        }
    }
}

pub(crate) fn handle_msg(engine: &mut Engine, msg: EngineMsg, persist: &Persist) {
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

#[cfg(target_os = "linux")]
fn tighten_timer_slack() {
    let rc = unsafe { libc::prctl(libc::PR_SET_TIMERSLACK, 10_000u64) };
    if rc != 0 {
        tracing::debug!("PR_SET_TIMERSLACK: {rc}");
    }
}

#[cfg(windows)]
fn raise_thread_priority() {
    unsafe {
        let _ = windows::Win32::System::Threading::SetThreadPriority(
            windows::Win32::System::Threading::GetCurrentThread(),
            windows::Win32::System::Threading::THREAD_PRIORITY_HIGHEST,
        );
    }
}
