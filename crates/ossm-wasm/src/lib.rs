//! Browser shell for OSSM: Engine + AIM30 RTU math. I/O is a JS `exchange` callback.

mod rtu;
mod shell;

pub use rtu::{
    apply_run_gains, enable_modbus, expected_response_len, homing, parse_fc03_u16,
    position_write_frame, read_position_radians, write_position_radians, DEFAULT_SLAVE,
    GENERIC_TIMEOUT_MS, STREAM_TIMEOUT_MS,
};
pub use shell::{persistent_motor_changed, RpcReply, Shell, ShellAction};

#[cfg(target_arch = "wasm32")]
mod wasm_api;
#[cfg(target_arch = "wasm32")]
pub use wasm_api::OssmShell;

#[cfg(test)]
mod rtu_tests;
