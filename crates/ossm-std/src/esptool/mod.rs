//! ESP ROM serial protocol (no flasher stub). Used by `--mode flash`.

mod reset;
mod slip;

use std::io::{self, Write};
use std::time::Duration;

use flate2::write::ZlibEncoder;
use flate2::Compression;
use md5::{Digest, Md5};

use crate::flash_port::FlashPort;

pub use reset::{hard_reset, USB_SERIAL_JTAG_PID};
pub use slip::{encode as slip_encode, Decoder as SlipDecoder};

pub const CHIP_DETECT_MAGIC_REG: u32 = 0x4000_1000;
pub const ESP32C6_MAGIC: u32 = 0x2CE0_806F;
pub const FLASH_WRITE_SIZE: usize = 0x400;
pub const CHECKSUM_INIT: u8 = 0xEF;

const OP_FLASH_BEGIN: u8 = 0x02;
const OP_FLASH_END: u8 = 0x04;
const OP_SYNC: u8 = 0x08;
const OP_READ_REG: u8 = 0x0A;
const OP_SPI_SET_PARAMS: u8 = 0x0B;
const OP_SPI_ATTACH: u8 = 0x0D;
const OP_FLASH_DEFL_BEGIN: u8 = 0x10;
const OP_FLASH_DEFL_DATA: u8 = 0x11;
const OP_FLASH_DEFL_END: u8 = 0x12;
const OP_FLASH_MD5: u8 = 0x13;

const SYNC_FRAME: [u8; 36] = {
    let mut f = [0x55u8; 36];
    f[0] = 0x07;
    f[1] = 0x07;
    f[2] = 0x12;
    f[3] = 0x20;
    f
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomResponse {
    pub op: u8,
    pub value: u32,
    pub data: Vec<u8>,
    pub status: u8,
    pub error: u8,
}

#[derive(Debug)]
pub struct FlashError(pub String);

impl std::fmt::Display for FlashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for FlashError {}

impl From<FlashError> for String {
    fn from(e: FlashError) -> Self {
        e.0
    }
}

impl From<io::Error> for FlashError {
    fn from(e: io::Error) -> Self {
        FlashError(e.to_string())
    }
}

pub struct FlashOpts {
    pub flash_size: u32,
    pub force_chip: bool,
    pub usb_jtag: bool,
    pub verify: bool,
}

pub fn parse_flash_size(s: &str) -> Result<u32, FlashError> {
    let t = s.trim().to_ascii_lowercase();
    let bytes = match t.as_str() {
        "2mb" | "2m" => 2 * 1024 * 1024,
        "4mb" | "4m" => 4 * 1024 * 1024,
        "8mb" | "8m" => 8 * 1024 * 1024,
        "16mb" | "16m" => 16 * 1024 * 1024,
        other => {
            if let Some(n) = other.strip_suffix("mb") {
                n.parse::<u32>()
                    .map_err(|_| FlashError(format!("bad --flash-size {s}")))?
                    * 1024
                    * 1024
            } else {
                return Err(FlashError(format!("bad --flash-size {s}")));
            }
        }
    };
    Ok(bytes)
}

pub fn checksum(data: &[u8], mut acc: u8) -> u8 {
    for &b in data {
        acc ^= b;
    }
    acc
}

pub fn build_command(op: u8, data: &[u8], data_checksum: u32) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(8 + data.len());
    pkt.push(0x00);
    pkt.push(op);
    pkt.extend_from_slice(&(data.len() as u16).to_le_bytes());
    pkt.extend_from_slice(&data_checksum.to_le_bytes());
    pkt.extend_from_slice(data);
    pkt
}

pub fn parse_response(frame: &[u8]) -> Result<RomResponse, FlashError> {
    if frame.len() < 8 {
        return Err(FlashError(format!("short ROM response ({})", frame.len())));
    }
    let status_len: usize = if frame.len() == 10 || frame.len() == 26 {
        2
    } else {
        4
    };
    if frame.len() < 8 + status_len.saturating_sub(4) {
        return Err(FlashError("truncated ROM status".into()));
    }
    let op = frame[1];
    let value = u32::from_le_bytes(frame[4..8].try_into().unwrap());
    let status = frame[frame.len() - status_len];
    let error = frame[frame.len() - status_len + 1];
    Ok(RomResponse {
        op,
        value,
        data: frame.to_vec(),
        status,
        error,
    })
}

pub fn md5_ascii_from_response(frame: &[u8]) -> Result<[u8; 16], FlashError> {
    if frame.len() >= 44 {
        let s = std::str::from_utf8(&frame[8..40]).map_err(|e| FlashError(e.to_string()))?;
        let mut out = [0u8; 16];
        for i in 0..16 {
            out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
                .map_err(|e| FlashError(format!("md5 hex: {e}")))?;
        }
        return Ok(out);
    }
    if frame.len() >= 26 {
        let mut out = [0u8; 16];
        out.copy_from_slice(&frame[8..24]);
        return Ok(out);
    }
    Err(FlashError(format!(
        "unexpected MD5 reply len {}",
        frame.len()
    )))
}

pub fn begin_params(
    size: u32,
    blocks: u32,
    block_size: u32,
    offset: u32,
    encrypted: bool,
) -> Vec<u8> {
    let mut d = Vec::with_capacity(20);
    d.extend_from_slice(&size.to_le_bytes());
    d.extend_from_slice(&blocks.to_le_bytes());
    d.extend_from_slice(&block_size.to_le_bytes());
    d.extend_from_slice(&offset.to_le_bytes());
    if encrypted {
        d.extend_from_slice(&0u32.to_le_bytes());
    } else {
        // C6 still takes the encrypted field (5 words). Always include it.
        d.extend_from_slice(&0u32.to_le_bytes());
    }
    d
}

pub fn data_params(payload: &[u8], sequence: u32) -> (Vec<u8>, u32) {
    let mut body = Vec::with_capacity(16 + payload.len());
    body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    body.extend_from_slice(&sequence.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(payload);
    let cs = checksum(payload, CHECKSUM_INIT) as u32;
    (body, cs)
}

pub fn spi_attach_rom() -> Vec<u8> {
    let mut d = vec![0u8; 8];
    d[..4].copy_from_slice(&0u32.to_le_bytes());
    d
}

pub fn spi_set_params(total_size: u32) -> Vec<u8> {
    let mut d = Vec::with_capacity(24);
    d.extend_from_slice(&0u32.to_le_bytes());
    d.extend_from_slice(&total_size.to_le_bytes());
    d.extend_from_slice(&(64 * 1024u32).to_le_bytes());
    d.extend_from_slice(&(4 * 1024u32).to_le_bytes());
    d.extend_from_slice(&256u32.to_le_bytes());
    d.extend_from_slice(&0xFFFFu32.to_le_bytes());
    d
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    pub name: String,
    pub offset: u32,
    pub size: u32,
    pub typ: u8,
    pub subtype: u8,
}

pub fn parse_partition_table(image: &[u8]) -> Vec<Partition> {
    let Some(table) = image.get(0x8000..) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 32 <= table.len() {
        let e = &table[off..off + 32];
        if e[0] != 0xAA || e[1] != 0x50 {
            break;
        }
        let typ = e[2];
        let subtype = e[3];
        let poff = u32::from_le_bytes(e[4..8].try_into().unwrap());
        let psz = u32::from_le_bytes(e[8..12].try_into().unwrap());
        let name = e[12..28]
            .iter()
            .copied()
            .take_while(|b| (0x20..=0x7e).contains(b))
            .map(|b| b as char)
            .collect();
        out.push(Partition {
            name,
            offset: poff,
            size: psz,
            typ,
            subtype,
        });
        off += 32;
    }
    out
}

pub fn keep_nvs_regions(image: &[u8]) -> Vec<(u32, Vec<u8>)> {
    let parts = parse_partition_table(image);
    let nvs = parts.iter().find(|p| p.name == "nvs");
    let factory = parts.iter().find(|p| p.name == "factory" || p.typ == 0);
    let nvs_off = nvs.map(|p| p.offset).unwrap_or(0x9000);
    let app_off = factory.map(|p| p.offset).unwrap_or(0x10000);
    let mut regions = Vec::new();
    let head_end = (nvs_off as usize).min(image.len());
    if head_end > 0 {
        regions.push((0, image[..head_end].to_vec()));
    }
    if (app_off as usize) < image.len() {
        regions.push((app_off, image[app_off as usize..].to_vec()));
    }
    regions
}

pub fn trim_trailing_ff(data: &[u8]) -> &[u8] {
    let mut end = data.len();
    while end > 0 && data[end - 1] == 0xFF {
        end -= 1;
    }
    &data[..end]
}

pub fn zlib_compress(data: &[u8]) -> Result<Vec<u8>, FlashError> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::best());
    enc.write_all(data).map_err(|e| FlashError(e.to_string()))?;
    enc.finish().map_err(|e| FlashError(e.to_string()))
}

fn read_frames(
    port: &mut dyn FlashPort,
    decoder: &mut SlipDecoder,
    timeout: Duration,
    need: usize,
) -> Result<Vec<Vec<u8>>, FlashError> {
    let deadline = std::time::Instant::now() + timeout;
    let mut frames = Vec::new();
    let mut buf = [0u8; 512];
    while frames.len() < need && std::time::Instant::now() < deadline {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let n = port.read(&mut buf, left.min(Duration::from_millis(50)))?;
        if n == 0 {
            continue;
        }
        decoder.push(&buf[..n], &mut frames);
    }
    Ok(frames)
}

fn command(
    port: &mut dyn FlashPort,
    decoder: &mut SlipDecoder,
    op: u8,
    data: &[u8],
    data_checksum: u32,
    timeout: Duration,
) -> Result<RomResponse, FlashError> {
    let _ = port.clear_input();
    decoder.reset();
    let pkt = build_command(op, data, data_checksum);
    port.write_all(&slip_encode(&pkt))?;
    let frames = read_frames(port, decoder, timeout, 1)?;
    let Some(frame) = frames.into_iter().next() else {
        return Err(FlashError(format!("no response for op {op:#x}")));
    };
    let rsp = parse_response(&frame)?;
    if rsp.status != 0 {
        return Err(FlashError(format!(
            "ROM op {op:#x} failed status={} error={}",
            rsp.status, rsp.error
        )));
    }
    Ok(rsp)
}

fn sync(port: &mut dyn FlashPort, decoder: &mut SlipDecoder) -> Result<(), FlashError> {
    let _ = port.clear_input();
    decoder.reset();
    let pkt = build_command(OP_SYNC, &SYNC_FRAME, 0);
    port.write_all(&slip_encode(&pkt))?;
    let frames = read_frames(port, decoder, Duration::from_millis(200), 1)?;
    for frame in frames {
        if let Ok(rsp) = parse_response(&frame) {
            if rsp.op == OP_SYNC && rsp.status == 0 {
                let _ = read_frames(port, decoder, Duration::from_millis(50), 8);
                return Ok(());
            }
        }
    }
    Err(FlashError("SYNC failed".into()))
}

pub fn connect(port: &mut dyn FlashPort, opts: &FlashOpts) -> Result<u32, FlashError> {
    let pid = port.usb_pid();
    let use_jtag = opts.usb_jtag || pid == Some(USB_SERIAL_JTAG_PID);
    let mut last = FlashError("connect failed".into());
    for extra in [false, true] {
        if use_jtag {
            reset::usb_jtag_serial_reset(port)?;
        } else {
            reset::classic_reset(port, extra)?;
        }
        std::thread::sleep(Duration::from_millis(50));
        let mut drain = [0u8; 256];
        let n = port.read(&mut drain, Duration::from_millis(200))?;
        if n > 0 {
            let log = String::from_utf8_lossy(&drain[..n]);
            eprint!("{log}");
        }
        let mut decoder = SlipDecoder::new();
        for _ in 0..5 {
            match sync(port, &mut decoder) {
                Ok(()) => {
                    let magic = command(
                        port,
                        &mut decoder,
                        OP_READ_REG,
                        &CHIP_DETECT_MAGIC_REG.to_le_bytes(),
                        0,
                        Duration::from_secs(1),
                    )?
                    .value;
                    if magic != ESP32C6_MAGIC && !opts.force_chip {
                        return Err(FlashError(format!(
                            "chip magic {magic:#010x} is not ESP32-C6 ({ESP32C6_MAGIC:#010x}); pass --force"
                        )));
                    }
                    command(
                        port,
                        &mut decoder,
                        OP_SPI_ATTACH,
                        &spi_attach_rom(),
                        0,
                        Duration::from_secs(1),
                    )?;
                    command(
                        port,
                        &mut decoder,
                        OP_SPI_SET_PARAMS,
                        &spi_set_params(opts.flash_size),
                        0,
                        Duration::from_secs(1),
                    )?;
                    return Ok(magic);
                }
                Err(e) => last = e,
            }
        }
        if use_jtag {
            break;
        }
    }
    Err(last)
}

fn erase_timeout(size: u32) -> Duration {
    let mb = (size as f64 / 1_000_000.0).max(0.1);
    Duration::from_secs((30.0 * mb).ceil() as u64).max(Duration::from_secs(15))
}

pub fn write_region(
    port: &mut dyn FlashPort,
    offset: u32,
    data: &[u8],
    verify: bool,
) -> Result<(), FlashError> {
    let trimmed = trim_trailing_ff(data);
    if trimmed.is_empty() {
        return Ok(());
    }
    let compressed = zlib_compress(trimmed)?;
    let blocks = compressed.len().div_ceil(FLASH_WRITE_SIZE) as u32;
    let erase_size = trimmed.len() as u32;
    eprintln!(
        "flash {offset:#x} uncompressed={} compressed={} blocks={blocks}",
        trimmed.len(),
        compressed.len()
    );
    let mut decoder = SlipDecoder::new();
    let begin = begin_params(erase_size, blocks, FLASH_WRITE_SIZE as u32, offset, true);
    command(
        port,
        &mut decoder,
        OP_FLASH_DEFL_BEGIN,
        &begin,
        0,
        erase_timeout(erase_size),
    )?;
    for (seq, chunk) in compressed.chunks(FLASH_WRITE_SIZE).enumerate() {
        let (body, cs) = data_params(chunk, seq as u32);
        command(
            port,
            &mut decoder,
            OP_FLASH_DEFL_DATA,
            &body,
            cs,
            Duration::from_secs(5),
        )?;
        if seq % 32 == 0 || seq + 1 == blocks as usize {
            eprint!("\r  block {}/{blocks}", seq + 1);
            let _ = std::io::stderr().flush();
        }
    }
    eprintln!();
    if verify {
        let mut md5 = Md5::new();
        md5.update(trimmed);
        let local: [u8; 16] = md5.finalize().into();
        let mut md5_req = [0u8; 16];
        md5_req[..4].copy_from_slice(&offset.to_le_bytes());
        md5_req[4..8].copy_from_slice(&(trimmed.len() as u32).to_le_bytes());
        let rsp = command(
            port,
            &mut decoder,
            OP_FLASH_MD5,
            &md5_req,
            0,
            erase_timeout(erase_size),
        )?;
        let remote = md5_ascii_from_response(&rsp.data)?;
        if remote != local {
            return Err(FlashError(format!(
                "MD5 mismatch local={} remote={}",
                hex16(&local),
                hex16(&remote)
            )));
        }
        eprintln!("MD5 ok {}", hex16(&local));
    }
    let _ = command(
        port,
        &mut decoder,
        OP_FLASH_DEFL_END,
        &[1u8],
        0,
        Duration::from_secs(3),
    );
    let _ = OP_FLASH_BEGIN;
    let _ = OP_FLASH_END;
    Ok(())
}

fn hex16(b: &[u8; 16]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn flash_image(
    port: &mut dyn FlashPort,
    image: &[u8],
    offset: u32,
    keep_nvs: bool,
    opts: &FlashOpts,
) -> Result<(), FlashError> {
    connect(port, opts)?;
    let regions = if keep_nvs && offset == 0 {
        keep_nvs_regions(image)
    } else {
        vec![(offset, image.to_vec())]
    };
    for (off, data) in regions {
        write_region(port, off, &data, opts.verify)?;
    }
    let pid = port.usb_pid();
    reset::hard_reset(port, pid)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_header_layout() {
        let pkt = build_command(OP_SYNC, &SYNC_FRAME, 0);
        assert_eq!(pkt[0], 0);
        assert_eq!(pkt[1], OP_SYNC);
        assert_eq!(u16::from_le_bytes(pkt[2..4].try_into().unwrap()), 36);
        assert_eq!(&pkt[8..], &SYNC_FRAME);
    }

    #[test]
    fn parse_rom_status_12() {
        let mut f = vec![0x01, OP_READ_REG, 4, 0, 0x6F, 0x80, 0xE0, 0x2C, 0, 0, 0, 0];
        f[4..8].copy_from_slice(&ESP32C6_MAGIC.to_le_bytes());
        let r = parse_response(&f).unwrap();
        assert_eq!(r.value, ESP32C6_MAGIC);
        assert_eq!(r.status, 0);
    }

    #[test]
    fn defl_begin_five_words() {
        let p = begin_params(0x1000, 4, 0x400, 0x10000, true);
        assert_eq!(p.len(), 20);
        assert_eq!(u32::from_le_bytes(p[0..4].try_into().unwrap()), 0x1000);
        assert_eq!(u32::from_le_bytes(p[12..16].try_into().unwrap()), 0x10000);
    }

    #[test]
    fn data_checksum_xor() {
        let payload = [1u8, 2, 3, 4];
        let (_body, cs) = data_params(&payload, 0);
        assert_eq!(cs as u8, checksum(&payload, CHECKSUM_INIT));
    }

    #[test]
    fn md5_ascii_parse() {
        let mut f = vec![0u8; 44];
        f[0] = 1;
        f[1] = OP_FLASH_MD5;
        let hex = b"0123456789abcdef0123456789abcdef";
        f[8..40].copy_from_slice(hex);
        let d = md5_ascii_from_response(&f).unwrap();
        assert_eq!(d[0], 0x01);
        assert_eq!(d[15], 0xef);
    }

    #[test]
    fn partition_split_skips_nvs() {
        let mut img = vec![0xFFu8; 0x20000];
        img[0] = 0xE9;
        img[0x8000..0x8002].copy_from_slice(&[0xAA, 0x50]);
        img[0x8002] = 1;
        img[0x8003] = 2;
        img[0x8004..0x8008].copy_from_slice(&0x9000u32.to_le_bytes());
        img[0x8008..0x800C].copy_from_slice(&0x6000u32.to_le_bytes());
        img[0x800C..0x800F].copy_from_slice(b"nvs");
        let e2 = 0x8000 + 32;
        img[e2..e2 + 2].copy_from_slice(&[0xAA, 0x50]);
        img[e2 + 2] = 1;
        img[e2 + 3] = 1;
        img[e2 + 4..e2 + 8].copy_from_slice(&0xF000u32.to_le_bytes());
        img[e2 + 8..e2 + 12].copy_from_slice(&0x1000u32.to_le_bytes());
        img[e2 + 12..e2 + 20].copy_from_slice(b"phy_init");
        let e3 = e2 + 32;
        img[e3..e3 + 2].copy_from_slice(&[0xAA, 0x50]);
        img[e3 + 2] = 0;
        img[e3 + 4..e3 + 8].copy_from_slice(&0x10000u32.to_le_bytes());
        img[e3 + 8..e3 + 12].copy_from_slice(&0x10000u32.to_le_bytes());
        img[e3 + 12..e3 + 19].copy_from_slice(b"factory");
        img[0x10000] = 0xE9;
        let parts = parse_partition_table(&img);
        assert_eq!(parts[0].name, "nvs");
        let regions = keep_nvs_regions(&img);
        assert_eq!(regions[0].0, 0);
        assert_eq!(regions[0].1.len(), 0x9000);
        assert_eq!(regions[1].0, 0x10000);
    }

    #[test]
    fn flash_size_parse() {
        assert_eq!(parse_flash_size("4mb").unwrap(), 4 * 1024 * 1024);
    }

    struct FakePort {
        rx: Vec<u8>,
        tx: Vec<u8>,
    }

    impl FlashPort for FakePort {
        fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
            self.tx.extend_from_slice(buf);
            Ok(())
        }
        fn read(&mut self, buf: &mut [u8], _timeout: Duration) -> io::Result<usize> {
            let n = self.rx.len().min(buf.len());
            buf[..n].copy_from_slice(&self.rx[..n]);
            self.rx.drain(..n);
            Ok(n)
        }
        fn set_dtr(&mut self, _: bool) -> io::Result<()> {
            Ok(())
        }
        fn set_rts(&mut self, _: bool) -> io::Result<()> {
            Ok(())
        }
        fn set_baud(&mut self, _: u32) -> io::Result<()> {
            Ok(())
        }
        fn clear_input(&mut self) -> io::Result<()> {
            Ok(())
        }
        fn usb_pid(&self) -> Option<u16> {
            Some(USB_SERIAL_JTAG_PID)
        }
    }

    fn slip_ok(op: u8, value: u32) -> Vec<u8> {
        let mut inner = vec![0x01, op, 4, 0];
        inner.extend_from_slice(&value.to_le_bytes());
        inner.extend_from_slice(&[0, 0, 0, 0]);
        slip_encode(&inner)
    }

    #[test]
    fn fake_port_sync_decode() {
        let mut port = FakePort {
            rx: slip_ok(OP_SYNC, 0),
            tx: Vec::new(),
        };
        let mut dec = SlipDecoder::new();
        sync(&mut port, &mut dec).unwrap();
        assert!(!port.tx.is_empty());
        assert_eq!(port.tx[0], 0xC0);
    }
}
