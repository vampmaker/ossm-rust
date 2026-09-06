//! USB-host serial (`/dev/bus/usb/…`) for Termux `termux-usb` and Linux usbfs.
//!
//! Android has no `/dev/ttyUSB*`. `termux-usb` grants a raw usbfs fd; a userspace
//! driver (CH340 / CDC / CP210x / FTDI / PL2303) speaks the chip.

use std::path::Path;

#[cfg(target_os = "linux")]
use std::fs::File;
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, RawFd};

#[cfg(target_os = "linux")]
use android_usb_serial::{
    from_raw_fd, open_port, DataBits, LineConfig, NusbTransport, Parity, StopBits, Transport,
};

#[cfg(target_os = "linux")]
use crate::serial::SerialPort;

pub const TERMUX_USB_PREFIX: &str = "termux-usb:";

/// True for `termux-usb:` URIs or Linux usbfs device nodes (`/dev/bus/usb/001/002`).
pub fn is_usb_bus_path(path: &Path) -> bool {
    if termux_usb_usbfs_path(path).is_some() {
        return true;
    }
    let s = path.to_string_lossy().replace('\\', "/");
    let bytes = s.as_bytes();
    bytes.windows(9).any(|w| w == b"/bus/usb/")
}

/// Device node inside a `termux-usb:` URI, if present.
pub fn termux_usb_usbfs_path(path: &Path) -> Option<std::path::PathBuf> {
    let s = path.to_string_lossy();
    s.strip_prefix(TERMUX_USB_PREFIX)
        .filter(|rest| !rest.is_empty())
        .map(std::path::PathBuf::from)
}

#[cfg(target_os = "linux")]
pub fn inherited_usb_fd() -> Option<RawFd> {
    let raw = std::env::var("TERMUX_USB_FD").ok()?;
    raw.parse::<i32>().ok().filter(|&fd| fd >= 0)
}

#[cfg(target_os = "linux")]
pub fn open_serial(path: &Path, baud: u32, slave: u8) -> Result<SerialPort, String> {
    let (fd, keepalive) = prepare_usb_fd(path)?;
    let (handle, _pid) = open_handle(fd, baud, true)?;
    SerialPort::from_usb_host(handle, slave, keepalive)
}

/// Open the usbfs CDC port without wrapping it as a Modbus RTU master.
#[cfg(target_os = "linux")]
pub fn open_raw(
    path: &Path,
    baud: u32,
    deassert_dtr_rts: bool,
) -> Result<(android_usb_serial::SerialPortHandle, Option<File>, u16), String> {
    let (fd, keepalive) = prepare_usb_fd(path)?;
    let (handle, pid) = open_handle(fd, baud, deassert_dtr_rts)?;
    Ok((handle, keepalive, pid))
}

#[cfg(target_os = "linux")]
fn prepare_usb_fd(path: &Path) -> Result<(RawFd, Option<File>), String> {
    if let Some(fd) = inherited_usb_fd() {
        return Ok((fd, None));
    }
    if let Some(dev) = termux_usb_usbfs_path(path) {
        eprintln!("usb: termux re-exec termux-usb ({})", dev.display());
        return reexec_termux_usb(&dev);
    }
    match File::options().read(true).write(true).open(path) {
        Ok(file) => {
            let fd = file.as_raw_fd();
            Ok((fd, Some(file)))
        }
        Err(e) => Err(format!(
            "open {}: {e} (use --serial {TERMUX_USB_PREFIX}{} for Termux permission re-exec)",
            path.display(),
            path.display()
        )),
    }
}

#[cfg(target_os = "linux")]
fn reexec_termux_usb(device: &Path) -> Result<(RawFd, Option<File>), String> {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;

    let argv: Vec<String> = std::env::args().collect();
    let dir = std::env::temp_dir();
    let json_path = dir.join("ossm-std-reexec.json");
    let runner_path = dir.join("ossm-std-reexec.py");
    std::fs::write(
        &json_path,
        serde_json::to_string(&argv).map_err(|e| format!("reexec json: {e}"))?,
    )
    .map_err(|e| format!("write {}: {e}", json_path.display()))?;
    let runner = format!(
        "#!{}/bin/python3\nimport json,os\nargv=json.load(open({}))\nos.execv(argv[0], argv)\n",
        std::env::var("PREFIX").unwrap_or_else(|_| "/data/data/com.termux/files/usr".into()),
        serde_json::to_string(&json_path.to_string_lossy()).unwrap()
    );
    std::fs::write(&runner_path, runner)
        .map_err(|e| format!("write {}: {e}", runner_path.display()))?;
    let mut perms = std::fs::metadata(&runner_path)
        .map_err(|e| format!("stat runner: {e}"))?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&runner_path, perms).map_err(|e| format!("chmod runner: {e}"))?;

    let _ = std::io::Write::flush(&mut std::io::stderr());
    let err = std::process::Command::new("termux-usb")
        .env("TERMUX_EXPORT_FD", "true")
        .args([
            "-r",
            "-E",
            "-e",
            runner_path.to_str().ok_or("runner path")?,
            &device.to_string_lossy(),
        ])
        .exec();
    Err(format!(
        "exec termux-usb failed ({err}). Install the Termux:API Android app and `pkg install termux-api`."
    ))
}

#[cfg(test)]
pub(crate) fn posix_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    if s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-_./:@+=,%".contains(&b))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

#[cfg(target_os = "linux")]
fn open_handle(
    fd: RawFd,
    baud: u32,
    deassert_dtr_rts: bool,
) -> Result<(android_usb_serial::SerialPortHandle, u16), String> {
    let device = from_raw_fd(fd).map_err(|e| format!("usb from_raw_fd: {e}"))?;
    let desc = device.device_descriptor();
    let pid = desc.product_id();
    let transport = std::sync::Arc::new(
        NusbTransport::from_device(device).map_err(|e| format!("usb transport: {e}"))?,
    ) as std::sync::Arc<dyn Transport>;
    let mut port = open_port(transport, 0).map_err(|e| format!("usb open_port: {e}"))?;
    port.set_line_config(LineConfig {
        baud_rate: baud,
        data_bits: DataBits::Eight,
        parity: Parity::None,
        stop_bits: StopBits::One,
    })
    .map_err(|e| format!("usb line config: {e}"))?;
    if deassert_dtr_rts {
        let _ = port.set_dtr(false);
        let _ = port.set_rts(false);
    }
    port.start_reader()
        .map_err(|e| format!("usb start_reader: {e}"))?;
    Ok((port, pid))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn detects_usbfs_path() {
        assert!(is_usb_bus_path(Path::new("/dev/bus/usb/001/002")));
        assert!(is_usb_bus_path(Path::new("/dev/bus/usb/002/047")));
        assert!(is_usb_bus_path(Path::new(
            "termux-usb:/dev/bus/usb/001/005"
        )));
        assert!(!is_usb_bus_path(Path::new("/dev/ttyUSB0")));
        assert!(!is_usb_bus_path(Path::new("/dev/ttyACM0")));
        assert!(!is_usb_bus_path(Path::new("COM3")));
        assert_eq!(
            termux_usb_usbfs_path(Path::new("termux-usb:/dev/bus/usb/001/005"))
                .as_deref()
                .and_then(Path::to_str),
            Some("/dev/bus/usb/001/005")
        );
        assert!(termux_usb_usbfs_path(Path::new("/dev/bus/usb/001/005")).is_none());
    }

    #[test]
    fn posix_quote_safe_and_spaces() {
        assert_eq!(posix_quote("/opt/ossm-std"), "/opt/ossm-std");
        assert_eq!(posix_quote("a b"), "'a b'");
        assert_eq!(posix_quote("it's"), "'it'\\''s'");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn inherited_fd_parses_termux_env() {
        let saved = std::env::var("TERMUX_USB_FD").ok();
        std::env::set_var("TERMUX_USB_FD", "7");
        let got = inherited_usb_fd();
        match saved {
            Some(v) => std::env::set_var("TERMUX_USB_FD", v),
            None => std::env::remove_var("TERMUX_USB_FD"),
        }
        assert_eq!(got, Some(7));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn inherited_fd_skips_reexec() {
        let saved = std::env::var("TERMUX_USB_FD").ok();
        std::env::set_var("TERMUX_USB_FD", "3");
        let path = Path::new("/dev/bus/usb/001/002");
        let result = prepare_usb_fd(path);
        match saved {
            Some(v) => std::env::set_var("TERMUX_USB_FD", v),
            None => std::env::remove_var("TERMUX_USB_FD"),
        }
        let (fd, keepalive) = result.expect("inherited fd");
        assert_eq!(fd, 3);
        assert!(keepalive.is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fake_cdc_write_roundtrip() {
        use android_usb_serial::{open_port, FakeTransport, Transport};
        use std::sync::Arc;

        let fake = FakeTransport::cdc_single_iface();
        let transport: Arc<dyn Transport> = Arc::new(fake.clone());
        let mut port = open_port(transport, 0).expect("open fake");
        port.write(b"PING").expect("write");
        assert_eq!(fake.take_tx(), b"PING");
    }
}
