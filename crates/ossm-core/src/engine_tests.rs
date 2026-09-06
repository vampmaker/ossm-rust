use crate::command::Command;
use crate::engine::Engine;
use crate::error::CoreError;
use crate::motion::sine_evaluate;
use crate::state::StateResponse;
use crate::MotorControllerConfig;

fn homed_paused_at(depth: f32, waveform_y: f32, t0: u64) -> Engine {
    let cfg = MotorControllerConfig {
        paused: true,
        depth,
        depth_top: false,
        reversed: false,
        paused_position: waveform_y,
        ..MotorControllerConfig::default()
    };
    let mut engine = Engine::new(cfg);
    engine.reset_cycle_clock(t0);
    let shaped = waveform_y * depth + (1.0 - depth);
    engine.apply(Command::HomingComplete {
        pos_min: 0.0,
        pos_max: 100.0,
        position: shaped * 100.0,
    });
    let _ = engine.tick(t0);
    engine
}

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
    let cfg = MotorControllerConfig {
        paused: false,
        ..MotorControllerConfig::default()
    };
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
    let next = MotorControllerConfig {
        version: first.version,
        ..MotorControllerConfig::default()
    };
    let second = engine.try_set_config(next).unwrap();
    let stale = MotorControllerConfig {
        version: first.version,
        ..MotorControllerConfig::default()
    };
    assert!(stale.version > 0 && stale.version < second.version);
    let err = engine.try_set_config(stale).unwrap_err();
    assert_eq!(err, CoreError::StaleVersion);
}

#[test]
fn reverse_toggle_holds_position_at_stroke_end() {
    let t0 = 1_000_000;
    let mut engine = homed_paused_at(1.0, 0.0, t0);
    let before_pos = engine.snapshot().position;
    let before_shaped = engine.snapshot().shaped_y;
    let mut cfg = engine.snapshot().config;
    cfg.reversed = true;
    engine.try_set_config(cfg).unwrap();
    let _ = engine.tick(t0 + 3_000);
    let snap = engine.snapshot();
    assert!(
        (snap.position - before_pos).abs() < 1e-2,
        "position jumped {} -> {}",
        before_pos,
        snap.position
    );
    assert!(
        (snap.shaped_y - before_shaped).abs() < 1e-3,
        "shaped_y jumped {} -> {}",
        before_shaped,
        snap.shaped_y
    );
}

#[test]
fn reverse_toggle_holds_position_at_midstroke_and_partial_depth() {
    let t0 = 1_000_000;
    for (depth, y) in [(1.0, 0.5), (0.5, 0.0), (0.5, 0.4)] {
        let mut engine = homed_paused_at(depth, y, t0);
        let before = engine.snapshot().position;
        let mut cfg = engine.snapshot().config;
        cfg.reversed = true;
        engine.try_set_config(cfg).unwrap();
        let _ = engine.tick(t0 + 3_000);
        let after = engine.snapshot().position;
        assert!(
            (after - before).abs() < 1e-2,
            "depth={depth} y={y}: position {before} -> {after}"
        );
    }
}

#[test]
fn depth_top_toggle_does_not_teleport() {
    let t0 = 1_000_000;
    let mut engine = homed_paused_at(0.5, 0.4, t0);
    let before = engine.snapshot().position;
    let mut cfg = engine.snapshot().config;
    cfg.depth_top = true;
    engine.try_set_config(cfg).unwrap();
    let _ = engine.tick(t0 + 3_000);
    let after = engine.snapshot().position;
    // Old snap: |1-depth|=0.5 of stroke → 50 units. Lerp step is 0.1/s * 3ms.
    assert!(
        (after - before).abs() < 1.0,
        "depth_top teleported position {before} -> {after}"
    );
}

#[test]
fn homing_at_mid_with_partial_depth_stays_at_mid() {
    let t0 = 1_000_000;
    let cfg = MotorControllerConfig {
        paused: true,
        depth: 0.6,
        depth_top: false,
        ..MotorControllerConfig::default()
    };
    let mut engine = Engine::new(cfg);
    engine.reset_cycle_clock(t0);
    engine.apply(Command::HomingComplete {
        pos_min: 0.0,
        pos_max: 100.0,
        position: 50.0,
    });
    let out = engine.tick(t0);
    assert!(
        (out.position - 50.0).abs() < 1e-2,
        "homing jumped to {} (old follow(0.5) would be 70)",
        out.position
    );
}

#[test]
fn homing_outside_depth_window_holds_pose() {
    let t0 = 1_000_000;
    let cfg = MotorControllerConfig {
        paused: true,
        depth: 0.3,
        depth_top: false,
        ..MotorControllerConfig::default()
    };
    let mut engine = Engine::new(cfg);
    engine.reset_cycle_clock(t0);
    engine.apply(Command::HomingComplete {
        pos_min: 0.0,
        pos_max: 100.0,
        position: 50.0,
    });
    let first = engine.tick(t0);
    assert!(
        (first.position - 50.0).abs() < 1.0,
        "outside-window homing teleported to {}",
        first.position
    );
    let second = engine.tick(t0 + 3_000);
    assert!(
        (second.position - first.position).abs() < 1.0,
        "hold slew jumped {} -> {}",
        first.position,
        second.position
    );
}

#[test]
fn config_copy_from_skips_equal_heap_fields() {
    let mut dst = MotorControllerConfig::default();
    let src = MotorControllerConfig::default();
    dst.copy_from(&src);
    assert_eq!(dst, src);
    dst.bpm = 40.0;
    dst.copy_from(&src);
    assert_eq!(dst.bpm, 36.0);
    assert_eq!(dst.wave_func, crate::config::WaveFunc::Sine);
    dst.wave_func = crate::config::WaveFunc::Spline;
    dst.copy_from(&src);
    assert_eq!(dst.wave_func, crate::config::WaveFunc::Sine);
}

#[test]
fn state_copy_from_copies_all_fields() {
    let mut dst = StateResponse::default();
    let mut src = StateResponse {
        position: 12.5,
        speed: 3.0,
        stats_gen: 1,
        ..StateResponse::default()
    };
    src.config.bpm = 99.0;
    src.config.version = 1;
    src.loop_stats.ups = 321;
    dst.copy_from(&src);
    assert_eq!(dst.position, 12.5);
    assert_eq!(dst.speed, 3.0);
    assert_eq!(dst.config.bpm, 99.0);
    assert_eq!(dst.loop_stats.ups, 321);
}

#[test]
fn waveform_fold_keeps_sine_phase_across_many_cycles() {
    let t0 = 1_000_000u64;
    let mut short = homed_paused_at(1.0, 0.5, t0);
    let mut cfg = short.snapshot().config;
    cfg.paused = false;
    short.try_set_config(cfg).unwrap();
    let period_us = crate::time::seconds_to_micros(60.0 / short.snapshot().config.bpm);

    let mut long = homed_paused_at(1.0, 0.5, t0);
    let mut cfg = long.snapshot().config;
    cfg.paused = false;
    long.try_set_config(cfg).unwrap();

    let _ = short.tick(t0);
    let _ = short.tick(t0 + 3_000);
    let _ = long.tick(t0);
    let _ = long.tick(t0 + 10 * period_us + 3_000);

    let sy = short.snapshot().y;
    let ly = long.snapshot().y;
    assert!(
        (sy - ly).abs() < 1e-4,
        "waveform y drifted across fold: {sy} vs {ly}"
    );
    let st = short.snapshot().t;
    let lt = long.snapshot().t;
    assert!(
        st < 60.0 / short.snapshot().config.bpm + 0.05,
        "short t should be intra-cycle, got {st}"
    );
    assert!(
        lt < 60.0 / long.snapshot().config.bpm + 0.05,
        "folded t should be intra-cycle, got {lt}"
    );
}
