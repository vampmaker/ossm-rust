//! Firmware Modbus RTU: re-export shared CRC/resync helpers and own RAM inject knobs.

pub use ossm_core::modbus::*;

use core::sync::atomic::Ordering;

use portable_atomic::AtomicU8;

/// Debug-only RX capture fault injection (RAM; cleared on reboot).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum InjectJunkMode {
    Off = 0,
    Leading = 1,
    Trailing = 2,
    Both = 3,
}

impl InjectJunkMode {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Leading,
            2 => Self::Trailing,
            3 => Self::Both,
            _ => Self::Off,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Leading => "leading",
            Self::Trailing => "trailing",
            Self::Both => "both",
        }
    }
}

static INJECT_MODE: AtomicU8 = AtomicU8::new(0);
static INJECT_NBYTES: AtomicU8 = AtomicU8::new(0);

pub fn set_inject_junk(mode: InjectJunkMode, nbytes: u8) {
    INJECT_MODE.store(mode as u8, Ordering::Relaxed);
    INJECT_NBYTES.store(nbytes, Ordering::Relaxed);
}

pub fn get_inject_junk() -> (InjectJunkMode, u8) {
    (
        InjectJunkMode::from_u8(INJECT_MODE.load(Ordering::Relaxed)),
        INJECT_NBYTES.load(Ordering::Relaxed),
    )
}

fn junk_byte(i: usize) -> u8 {
    // Non-zero pattern so resync is exercised (not only skip_leading_zeros).
    0xFFu8.wrapping_sub((i % 7) as u8)
}

/// Shift/append junk into a DMA capture buffer (debug fault injection).
pub fn apply_capture_inject(capture: &mut [u8], rx_len: usize) -> usize {
    let mode = InjectJunkMode::from_u8(INJECT_MODE.load(Ordering::Relaxed));
    let n = INJECT_NBYTES.load(Ordering::Relaxed) as usize;
    if mode == InjectJunkMode::Off || n == 0 || rx_len == 0 {
        return rx_len;
    }
    let cap = capture.len();
    match mode {
        InjectJunkMode::Off => rx_len,
        InjectJunkMode::Leading => {
            let shift = n.min(cap.saturating_sub(rx_len));
            if shift == 0 {
                return rx_len;
            }
            capture.copy_within(0..rx_len, shift);
            for (i, slot) in capture[..shift].iter_mut().enumerate() {
                *slot = junk_byte(i);
            }
            rx_len + shift
        }
        InjectJunkMode::Trailing => {
            let append = n.min(cap.saturating_sub(rx_len));
            for i in 0..append {
                capture[rx_len + i] = junk_byte(i);
            }
            rx_len + append
        }
        InjectJunkMode::Both => {
            let shift = n.min(cap.saturating_sub(rx_len).min(rx_len.max(1)));
            let after_lead = if shift > 0 {
                capture.copy_within(0..rx_len, shift);
                for (i, slot) in capture[..shift].iter_mut().enumerate() {
                    *slot = junk_byte(i);
                }
                rx_len + shift
            } else {
                rx_len
            };
            let append = n.min(cap.saturating_sub(after_lead));
            for i in 0..append {
                capture[after_lead + i] = junk_byte(shift + i);
            }
            after_lead + append
        }
    }
}
