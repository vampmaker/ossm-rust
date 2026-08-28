use crate::command::Command;
use crate::engine::Engine;
use crate::error::CoreError;
use crate::motion::sine_evaluate;
use crate::MotorControllerConfig;

#[test]
fn sine_at_t0_is_midstroke() {
    let (y, _) = sine_evaluate(0.0, 36.0);
    assert!((y - 0.5).abs() < 1e-5, "y={y}");
}

#[test]
fn tick_clamps_dt_to_12ms_on_paused_source() {
    let mut engine = Engine::new(MotorControllerConfig::default());
    engine.apply(Command::SetPaused {
        paused: true,
        position: Some(1.0),
    });
    engine.reset_cycle_clock(1_000_000);
    let _ = engine.tick(1_000_000);
    let _ = engine.tick(1_000_000 + 20_000);
    let y = engine.snapshot().y;
    // PAUSE_SPEED = 0.3 /s * 12ms = 0.0036; unclamped 20ms would be 0.006
    assert!(
        (y - 0.0036).abs() < 1e-4,
        "y={y} (expected ~0.0036 from 12ms clamp)"
    );
}

#[test]
fn homing_complete_pauses_and_bumps_version() {
    let mut cfg = MotorControllerConfig::default();
    cfg.paused = false;
    let mut engine = Engine::new(cfg);
    let applied = engine
        .try_set_config(MotorControllerConfig::default())
        .unwrap();
    let before = applied.version;
    engine.apply(Command::HomingComplete {
        pos_min: 0.0,
        pos_max: 100.0,
        position: 50.0,
    });
    let snap = engine.snapshot();
    assert!(snap.config.paused);
    assert!(
        snap.config.version > before,
        "version {} should exceed {before}",
        snap.config.version
    );
}

#[test]
fn try_set_config_rejects_stale_version() {
    let mut engine = Engine::new(MotorControllerConfig::default());
    let first = engine
        .try_set_config(MotorControllerConfig::default())
        .unwrap();
    let mut next = MotorControllerConfig::default();
    next.version = first.version;
    let second = engine.try_set_config(next).unwrap();
    let mut stale = MotorControllerConfig::default();
    stale.version = first.version;
    assert!(stale.version > 0 && stale.version < second.version);
    let err = engine.try_set_config(stale).unwrap_err();
    assert_eq!(err, CoreError::StaleVersion);
}
