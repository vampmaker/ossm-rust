use std::thread::sleep;
use std::time::Duration;

use crate::flash_port::FlashPort;

pub const USB_SERIAL_JTAG_PID: u16 = 0x1001;

pub fn usb_jtag_serial_reset(port: &mut dyn FlashPort) -> std::io::Result<()> {
    port.set_rts(false)?;
    port.set_dtr(false)?;
    sleep(Duration::from_millis(100));
    port.set_rts(false)?;
    port.set_dtr(true)?;
    sleep(Duration::from_millis(100));
    port.set_rts(true)?;
    port.set_dtr(false)?;
    port.set_rts(true)?;
    sleep(Duration::from_millis(100));
    port.set_rts(false)?;
    port.set_dtr(false)?;
    Ok(())
}

pub fn classic_reset(port: &mut dyn FlashPort, extra_delay: bool) -> std::io::Result<()> {
    let delay = if extra_delay { 500 } else { 50 };
    port.set_rts(false)?;
    port.set_dtr(false)?;
    port.set_rts(true)?;
    port.set_dtr(true)?;
    port.set_rts(true)?;
    port.set_dtr(false)?;
    sleep(Duration::from_millis(100));
    port.set_rts(false)?;
    port.set_dtr(true)?;
    sleep(Duration::from_millis(delay));
    port.set_rts(false)?;
    port.set_dtr(false)?;
    Ok(())
}

/// Leave the bootloader and run user code (USB Serial/JTAG variant when pid is 0x1001).
pub fn hard_reset(port: &mut dyn FlashPort, pid: Option<u16>) -> std::io::Result<()> {
    sleep(Duration::from_millis(100));
    if pid == Some(USB_SERIAL_JTAG_PID) {
        port.set_dtr(false)?;
        sleep(Duration::from_millis(100));
        port.set_rts(true)?;
        port.set_dtr(false)?;
        port.set_rts(true)?;
        sleep(Duration::from_millis(100));
        port.set_rts(false)?;
    } else {
        port.set_rts(true)?;
        sleep(Duration::from_millis(100));
        port.set_rts(false)?;
    }
    Ok(())
}
