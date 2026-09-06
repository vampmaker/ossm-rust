//! Exclusive UART1 bus tokens: UART1 + UHCI0 + DMA_CH0 + Modbus GPIOs.
//!
//! Boot takes those peripherals plus leftover `GPIO*` into this bank. Each
//! UART1 role checks them out instead of `steal()`. Console UART0 pins stay
//! out of the bank.

use alloc::vec::Vec;

use esp_hal::gpio::{AnyPin, Pin};
use esp_hal::peripherals::{DMA_CH0, UART1, UHCI0};

use crate::error::{FirmwareError, Result};

const MAX_CHECKED_OUT: usize = 4;

/// Owner of UART1 bus peripherals. Only the UART1 owner task mutates this.
pub struct PeripheralBank {
    uart1: UART1<'static>,
    uhci0: UHCI0<'static>,
    dma_ch0: DMA_CH0<'static>,
    uart_in_use: bool,
    pins: Vec<AnyPin<'static>>,
    pins_in_use: heapless::Vec<u8, MAX_CHECKED_OUT>,
}

impl PeripheralBank {
    pub fn new(uart1: UART1<'static>, uhci0: UHCI0<'static>, dma_ch0: DMA_CH0<'static>) -> Self {
        Self {
            uart1,
            uhci0,
            dma_ch0,
            uart_in_use: false,
            pins: Vec::new(),
            pins_in_use: heapless::Vec::new(),
        }
    }

    pub fn insert_pin(&mut self, pin: impl Into<AnyPin<'static>>) {
        let pin = pin.into();
        let n = pin.number();
        if self.pins.iter().any(|p| p.number() == n) {
            return;
        }
        self.pins.push(pin);
    }

    pub fn take_uart(&mut self) -> Result<(UART1<'static>, UHCI0<'static>, DMA_CH0<'static>)> {
        if self.uart_in_use {
            return Err(FirmwareError::Uart("uart1 in use"));
        }
        self.uart_in_use = true;
        // SAFETY: `uart_in_use` forbids a second checkout. The clones are the
        // only driver-facing tokens until `release_uart`. The UART1 owner
        // is the sole caller. Drivers consume the clones and drop them when
        // the role ends; the originals stay here for the next role.
        Ok(unsafe {
            (
                self.uart1.clone_unchecked(),
                self.uhci0.clone_unchecked(),
                self.dma_ch0.clone_unchecked(),
            )
        })
    }

    pub fn release_uart(&mut self) {
        self.uart_in_use = false;
    }

    /// Check out a pin for the current UART1 role.
    pub fn take_pin(&mut self, number: u8) -> Result<AnyPin<'static>> {
        if self.pins_in_use.contains(&number) {
            return Err(FirmwareError::PinUnavailable);
        }
        let pin = self
            .pins
            .iter()
            .find(|p| p.number() == number)
            .ok_or(FirmwareError::PinUnavailable)?;
        self.pins_in_use
            .push(number)
            .map_err(|_| FirmwareError::PinUnavailable)?;
        // SAFETY: same exclusive-checkout rule as `take_uart`.
        Ok(unsafe { pin.clone_unchecked() })
    }

    pub fn release_pin(&mut self, number: u8) {
        if let Some(i) = self.pins_in_use.iter().position(|&n| n == number) {
            let _ = self.pins_in_use.swap_remove(i);
        }
    }

    pub fn release_role(&mut self, tx: u8, rx: u8, de: u8) {
        self.release_uart();
        self.release_pin(tx);
        self.release_pin(rx);
        self.release_pin(de);
    }
}

/// Insert leftover GPIOs (not console UART0).
#[macro_export]
macro_rules! peripheral_bank_insert_pins {
    ($bank:expr, $p:expr, $($gpio:ident),+ $(,)?) => {
        $($bank.insert_pin($p.$gpio);)+
    };
}
