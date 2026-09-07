//! Raw serial used by `--mode flash` and `--mode console`.
//!
//! Not the Modbus RTU bus: DTR/RTS stay controllable and RX is a byte stream.

use std::io;
use std::path::Path;
use std::time::Duration;

pub trait FlashPort: Send {
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()>;
    /// `Ok(0)` means the timeout elapsed with no bytes.
    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize>;
    fn set_dtr(&mut self, on: bool) -> io::Result<()>;
    fn set_rts(&mut self, on: bool) -> io::Result<()>;
    #[allow(dead_code)]
    fn set_baud(&mut self, baud: u32) -> io::Result<()>;
    fn clear_input(&mut self) -> io::Result<()>;
    fn usb_pid(&self) -> Option<u16>;
}

pub fn open_flash_port(
    path: &Path,
    baud: u32,
    deassert_dtr_rts: bool,
    assume_usb_jtag: bool,
) -> Result<Box<dyn FlashPort>, String> {
    if crate::usb_host::is_usb_bus_path(path) {
        #[cfg(target_os = "linux")]
        {
            let (handle, keepalive, pid) = crate::usb_host::open_raw(path, baud, deassert_dtr_rts)?;
            return Ok(Box::new(UsbHostFlashPort {
                handle,
                _keepalive: keepalive,
                pid,
            }));
        }
        #[cfg(not(target_os = "linux"))]
        {
            return Err(format!(
                "USB host serial ({}) is Linux/Android only",
                path.display()
            ));
        }
    }
    open_tty(path, baud, deassert_dtr_rts, assume_usb_jtag)
}

fn open_tty(
    path: &Path,
    baud: u32,
    deassert_dtr_rts: bool,
    assume_usb_jtag: bool,
) -> Result<Box<dyn FlashPort>, String> {
    // Linux cdc-acm still asserts DTR in the kernel on activate; `dtr_on_open(false)`
    // only clears it after that pulse. USB-Serial/JTAG may reset once on first open.
    let mut builder =
        serialport::new(path.to_string_lossy().as_ref(), baud).timeout(Duration::from_millis(1));
    if deassert_dtr_rts {
        builder = builder.dtr_on_open(false);
    }
    let mut port = builder
        .open()
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    if deassert_dtr_rts {
        let _ = serialport::SerialPort::write_data_terminal_ready(port.as_mut(), false);
        let _ = serialport::SerialPort::write_request_to_send(port.as_mut(), false);
    }
    let pid = tty_usb_pid(path).or(if assume_usb_jtag { Some(0x1001) } else { None });
    Ok(Box::new(TtyFlashPort { port, pid }))
}

fn tty_usb_pid(path: &Path) -> Option<u16> {
    let name = path.file_name()?.to_str()?;
    let candidates = [
        format!("/sys/class/tty/{name}/device/../idProduct"),
        format!("/sys/class/tty/{name}/device/idProduct"),
    ];
    for p in candidates {
        if let Ok(s) = std::fs::read_to_string(&p) {
            if let Ok(v) = u16::from_str_radix(s.trim(), 16) {
                return Some(v);
            }
        }
    }
    None
}

struct TtyFlashPort {
    port: Box<dyn serialport::SerialPort>,
    pid: Option<u16>,
}

impl FlashPort for TtyFlashPort {
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        use std::io::Write;
        self.port.write_all(buf)?;
        self.port.flush()
    }

    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        use std::io::Read;
        self.port
            .set_timeout(timeout)
            .map_err(|e| io::Error::other(e.to_string()))?;
        match self.port.read(buf) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == io::ErrorKind::TimedOut => Ok(0),
            Err(e) => Err(e),
        }
    }

    fn set_dtr(&mut self, on: bool) -> io::Result<()> {
        self.port
            .write_data_terminal_ready(on)
            .map_err(|e| io::Error::other(e.to_string()))
    }

    fn set_rts(&mut self, on: bool) -> io::Result<()> {
        self.port
            .write_request_to_send(on)
            .map_err(|e| io::Error::other(e.to_string()))
    }

    fn set_baud(&mut self, baud: u32) -> io::Result<()> {
        self.port
            .set_baud_rate(baud)
            .map_err(|e| io::Error::other(e.to_string()))
    }

    fn clear_input(&mut self) -> io::Result<()> {
        self.port
            .clear(serialport::ClearBuffer::Input)
            .map_err(|e| io::Error::other(e.to_string()))
    }

    fn usb_pid(&self) -> Option<u16> {
        self.pid
    }
}

#[cfg(target_os = "linux")]
struct UsbHostFlashPort {
    handle: android_usb_serial::SerialPortHandle,
    _keepalive: Option<std::fs::File>,
    pid: u16,
}

#[cfg(target_os = "linux")]
impl FlashPort for UsbHostFlashPort {
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        let mut off = 0;
        while off < buf.len() {
            let n = self
                .handle
                .write(&buf[off..])
                .map_err(|e| io::Error::other(e.to_string()))?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::WriteZero, "usb write 0"));
            }
            off += n;
        }
        Ok(())
    }

    fn read(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match self.handle.try_read(buf) {
                Ok(n) if n > 0 => return Ok(n),
                Ok(_) => {}
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("TimedOut") || msg.contains("WouldBlock") {
                        // keep polling
                    } else {
                        return Err(io::Error::other(msg));
                    }
                }
            }
            if std::time::Instant::now() >= deadline {
                return Ok(0);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn set_dtr(&mut self, on: bool) -> io::Result<()> {
        self.handle
            .set_dtr(on)
            .map_err(|e| io::Error::other(e.to_string()))
    }

    fn set_rts(&mut self, on: bool) -> io::Result<()> {
        self.handle
            .set_rts(on)
            .map_err(|e| io::Error::other(e.to_string()))
    }

    fn set_baud(&mut self, baud: u32) -> io::Result<()> {
        self.handle
            .set_line_config(android_usb_serial::LineConfig {
                baud_rate: baud,
                data_bits: android_usb_serial::DataBits::Eight,
                parity: android_usb_serial::Parity::None,
                stop_bits: android_usb_serial::StopBits::One,
            })
            .map_err(|e| io::Error::other(e.to_string()))
    }

    fn clear_input(&mut self) -> io::Result<()> {
        let mut dump = [0u8; 512];
        loop {
            match self.handle.try_read(&mut dump) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
        Ok(())
    }

    fn usb_pid(&self) -> Option<u16> {
        Some(self.pid)
    }
}
