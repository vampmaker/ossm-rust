use crate::config::MotorControllerConfig;
use crate::engine::Engine;
use crate::paths;
use crate::rpc::{self, RpcAction};

fn rpc_json(v: &[u8]) -> serde_json::Value {
    serde_json::from_slice(v).expect("rpc json")
}

#[test]
fn ping_returns_pong() {
    let mut engine = Engine::new(MotorControllerConfig::default());
    let mut out = [0u8; 512];
    let req = br#"{"jsonrpc":"2.0","method":"ping","id":1}"#;
    let (action, n) = rpc::dispatch_rpc(&mut engine, req, &mut out);
    assert_eq!(action, RpcAction::Respond);
    let v = rpc_json(&out[..n]);
    assert_eq!(v["result"], "pong");
    assert_eq!(v["id"], 1);
}

#[test]
fn set_config_rejects_stale_version() {
    let mut engine = Engine::new(MotorControllerConfig::default());
    let first = engine
        .try_set_config(MotorControllerConfig::default())
        .unwrap();
    let next = MotorControllerConfig {
        version: first.version,
        ..MotorControllerConfig::default()
    };
    let second = engine.try_set_config(next).unwrap();
    assert!(second.version > first.version);

    let req = alloc::format!(
        r#"{{"jsonrpc":"2.0","method":"set-config","id":2,"params":{{"version":{},"bpm":36.0,"depth":1.0,"depth_top":false,"reversed":false,"wave_func":"sine","sharpness":0.3,"paused":true,"paused_position":0.0,"streaming":false}}}}"#,
        first.version
    );
    let mut out = [0u8; 512];
    let (action, n) = rpc::dispatch_rpc(&mut engine, req.as_bytes(), &mut out);
    assert_eq!(action, RpcAction::Respond);
    let v = rpc_json(&out[..n]);
    assert_eq!(v["error"]["code"], -32001);
    assert_eq!(v["error"]["message"], "Stale causal version");
}

#[test]
fn path_set_get_motor_bpm() {
    let mut engine = Engine::new(MotorControllerConfig::default());
    paths::set(&mut engine, "motor.bpm", "42").unwrap();
    assert_eq!(paths::get(&engine, "motor.bpm").unwrap(), "42");
}

#[test]
fn state_notification_serializes_params_once() {
    let engine = Engine::new(MotorControllerConfig::default());
    let mut out = [0u8; 4096];
    let n = rpc::build_state_notification_into(engine.snapshot(), &mut out)
        .expect("default snapshot fits 4 KiB");
    let v = rpc_json(&out[..n]);
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["method"], "state");
    assert_eq!(v["cmd"], "state");
    assert!(v["params"]["config"].is_object(), "params.config required");
    assert!(
        v.get("state").is_none(),
        "duplicate top-level state must be gone"
    );

    let mut fat = *engine.snapshot();
    fat.config.spline_points = crate::config::SplinePoints::from_slice(&[0.0; 16]);
    for i in 0..10u32 {
        fat.loop_stats.update_history.push(i);
        fat.loop_stats.position_history.push(i as f32);
    }
    fat.link_stats.transport = ossm_common::LinkTransport::Uart;
    let n_fat =
        rpc::build_state_notification_into(&fat, &mut out).expect("fat snapshot fits 4 KiB");
    let v_fat = rpc_json(&out[..n_fat]);
    assert!(v_fat.get("state").is_none());
    assert_eq!(
        v_fat["params"]["config"]["spline_points"]
            .as_array()
            .unwrap()
            .len(),
        16
    );
}
