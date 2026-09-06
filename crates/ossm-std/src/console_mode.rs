//! USB/UART CLI monitor (`--mode console`).

use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use regex::Regex;

use crate::esptool;
use crate::flash_port::FlashPort;

/// Same bytes as firmware `rs485::EXIT_MAGIC`.
pub const EXIT_MAGIC: [u8; 8] = [0xF0, 0x0F, b'O', b'S', b'S', b'M', 0x1B, b'q'];

const ACK_NEEDLES: &[&str] = &[
    "set to",
    "ok",
    "restart to apply",
    "saved",
    "enabled",
    "disabled",
    "resetting",
    "hostname set",
    "updated",
    "saving",
    "configuration",
    "got ip",
];

pub struct ConsoleOpts {
    pub send: Vec<String>,
    pub until: Option<String>,
    pub timeout: Duration,
    pub reset: bool,
    pub exit_rs485: bool,
    pub raw: bool,
}

pub fn looks_like_ack(line: &str) -> bool {
    let l = line.to_ascii_lowercase();
    if ACK_NEEDLES.iter().any(|n| l.contains(n)) {
        return true;
    }
    l.contains('{') && l.contains('}')
}

#[cfg(test)]
pub fn feed_exit_magic(matched: &mut u8, data: &[u8], out: &mut [u8]) -> (usize, bool) {
    const MAGIC: [u8; 8] = EXIT_MAGIC;
    let mut n = 0usize;
    let mut hit = false;
    for &b in data {
        let expect = MAGIC.get(*matched as usize).copied();
        if expect == Some(b) {
            *matched = matched.saturating_add(1);
            if *matched as usize >= MAGIC.len() {
                *matched = 0;
                hit = true;
            }
            continue;
        }
        if *matched > 0 {
            let prefix = &MAGIC[..*matched as usize];
            let copy = prefix.len().min(out.len().saturating_sub(n));
            out[n..n + copy].copy_from_slice(&prefix[..copy]);
            n += copy;
            *matched = 0;
        }
        if b == MAGIC[0] {
            *matched = 1;
        } else if n < out.len() {
            out[n] = b;
            n += 1;
        }
    }
    (n, hit)
}

fn read_text(port: &mut dyn FlashPort, timeout: Duration) -> io::Result<String> {
    let mut acc = Vec::new();
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 256];
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        let n = port.read(&mut buf, left.min(Duration::from_millis(80)))?;
        if n > 0 {
            acc.extend_from_slice(&buf[..n]);
            let s = String::from_utf8_lossy(&acc);
            print!("{}", String::from_utf8_lossy(&buf[..n]));
            let _ = io::stdout().flush();
            if s.contains('\n') {
                // keep gathering until idle-ish
                let idle = port.read(&mut buf, Duration::from_millis(40))?;
                if idle > 0 {
                    acc.extend_from_slice(&buf[..idle]);
                    print!("{}", String::from_utf8_lossy(&buf[..idle]));
                    continue;
                }
                if looks_like_ack(&s) {
                    break;
                }
            }
        }
    }
    Ok(String::from_utf8_lossy(&acc).into_owned())
}

fn wait_needle(port: &mut dyn FlashPort, needle: &Regex, timeout: Duration) -> io::Result<String> {
    let mut acc = String::new();
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 256];
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        let n = port.read(&mut buf, left.min(Duration::from_millis(200)))?;
        if n == 0 {
            continue;
        }
        let chunk = String::from_utf8_lossy(&buf[..n]);
        print!("{chunk}");
        let _ = io::stdout().flush();
        acc.push_str(&chunk);
        if needle.is_match(&acc) {
            return Ok(acc);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("timeout waiting for /{needle}/"),
    ))
}

fn extract_ip(text: &str) -> Option<String> {
    let re = Regex::new(r"(?i)(?:sta ip:|got ip:|ip:)\s*([0-9]{1,3}(?:\.[0-9]{1,3}){3})").ok()?;
    re.captures(text)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

pub fn run(port: &mut dyn FlashPort, opts: ConsoleOpts) -> Result<(), String> {
    if opts.reset {
        let pid = port.usb_pid();
        esptool::hard_reset(port, pid).map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_millis(400));
    }
    if opts.exit_rs485 {
        port.write_all(&EXIT_MAGIC).map_err(|e| e.to_string())?;
        let re = Regex::new(r"shell\.operating_mode set to servo").map_err(|e| e.to_string())?;
        wait_needle(port, &re, opts.timeout).map_err(|e| e.to_string())?;
    }
    let mut acc = String::new();
    for cmd in &opts.send {
        eprintln!("> {cmd}");
        port.write_all(cmd.as_bytes()).map_err(|e| e.to_string())?;
        port.write_all(b"\r\n").map_err(|e| e.to_string())?;
        acc.push_str(&read_text(port, opts.timeout).map_err(|e| e.to_string())?);
    }
    if let Some(pat) = opts.until.as_deref() {
        let re = Regex::new(pat).map_err(|e| format!("--until regex: {e}"))?;
        if !re.is_match(&acc) {
            acc.push_str(&wait_needle(port, &re, opts.timeout).map_err(|e| e.to_string())?);
        }
        if let Some(ip) = extract_ip(&acc) {
            println!("DEVICE_IP={ip}");
        } else if !re.is_match(&acc) {
            return Err(format!("--until /{pat}/ not matched"));
        }
    }
    if opts.raw || (opts.send.is_empty() && opts.until.is_none() && !opts.exit_rs485) {
        raw_passthrough(port)?;
    }
    Ok(())
}

fn raw_passthrough(port: &mut dyn FlashPort) -> Result<(), String> {
    eprintln!("console: stdin -> serial (Ctrl+C to quit)");
    let mut stdin = io::stdin();
    let mut inbuf = [0u8; 256];
    // blocking stdin in a thread
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || loop {
        match stdin.read(&mut inbuf) {
            Ok(0) => break,
            Ok(n) => {
                if tx.send(inbuf[..n].to_vec()).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    });
    let mut out = [0u8; 256];
    loop {
        match rx.try_recv() {
            Ok(chunk) => port.write_all(&chunk).map_err(|e| e.to_string())?,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                let n = port
                    .read(&mut out, Duration::from_millis(50))
                    .map_err(|e| e.to_string())?;
                if n > 0 {
                    io::stdout()
                        .write_all(&out[..n])
                        .map_err(|e| e.to_string())?;
                }
                return Ok(());
            }
        }
        let n = port
            .read(&mut out, Duration::from_millis(50))
            .map_err(|e| e.to_string())?;
        if n > 0 {
            let mut stdout = io::stdout();
            stdout.write_all(&out[..n]).map_err(|e| e.to_string())?;
            let _ = stdout.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ack_json_and_set_to() {
        assert!(looks_like_ack(r#"{"modbus_tx":2}"#));
        assert!(looks_like_ack("pin.modbus_tx set to 2"));
        assert!(!looks_like_ack("waiting"));
    }

    #[test]
    fn magic_split_across_packets() {
        let mut st = 0u8;
        let mut out = [0u8; 32];
        let (n, hit) = feed_exit_magic(&mut st, &EXIT_MAGIC[..3], &mut out);
        assert_eq!(n, 0);
        assert!(!hit);
        let (n, hit) = feed_exit_magic(&mut st, &EXIT_MAGIC[3..], &mut out);
        assert_eq!(n, 0);
        assert!(hit);
        let (n, hit) = feed_exit_magic(&mut st, b"AB", &mut out);
        assert_eq!(&out[..n], b"AB");
        assert!(!hit);
    }

    #[test]
    fn magic_false_prefix_emitted() {
        let mut st = 0u8;
        let mut out = [0u8; 32];
        let mut data = EXIT_MAGIC[..4].to_vec();
        data.extend_from_slice(b"XYZ");
        let (n, hit) = feed_exit_magic(&mut st, &data, &mut out);
        assert!(!hit);
        assert_eq!(&out[..n], b"\xF0\x0FOSXYZ");
    }

    #[test]
    fn extract_sta_ip() {
        let s = "wifi: sta ip: 192.168.24.244\r\n";
        assert_eq!(extract_ip(s).as_deref(), Some("192.168.24.244"));
    }
}
