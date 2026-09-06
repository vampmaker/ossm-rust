//! Shared `no_std` + `alloc` helpers for OSSM shells: telemetry windows and link pacing.

#![no_std]

extern crate alloc;

pub mod pll;
pub mod telemetry;

pub use pll::{Pll, PllConfig, PllInfo, PllState, Wake};
pub use telemetry::*;
