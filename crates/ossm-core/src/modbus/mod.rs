//! Modbus RTU helpers shared by the motor master and relay bridge.

pub mod aim30;

use rmodbus::{guess_response_frame_len, ModbusProto};

pub fn calc_crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xffff;
    for &b in data {
        crc ^= u16::from(b);
        for _ in 0..8 {
            if (crc & 0x0001) != 0 {
                crc = (crc >> 1) ^ 0xA001;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

pub fn verify_rtu_crc(frame: &[u8]) -> bool {
    if frame.len() < 4 {
        return false;
    }
    let n = frame.len();
    let expected = u16::from_le_bytes([frame[n - 2], frame[n - 1]]);
    calc_crc16(&frame[..n - 2]) == expected
}

pub fn skip_leading_zeros(data: &[u8]) -> &[u8] {
    let mut i = 0;
    while i < data.len() && data[i] == 0 {
        i += 1;
    }
    &data[i..]
}

/// Result of inspecting the *prefix* of an RTU byte stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RtuPrefix {
    /// CRC-valid response of this length starts at offset 0.
    Complete(usize),
    /// Header length guess succeeded but not enough bytes yet — wait.
    Incomplete,
    /// First byte cannot start a valid frame — drop it and rescan.
    Invalid,
}

fn response_header_len(func: u8) -> usize {
    if func < 0x80 && (1..=4).contains(&func) {
        3
    } else {
        2
    }
}

/// Classify the front of an unbounded RTU RX stream (no `expected` length gate).
pub fn classify_rtu_prefix(rx: &[u8]) -> RtuPrefix {
    if rx.len() < 2 {
        return RtuPrefix::Incomplete;
    }
    let need = response_header_len(rx[1]);
    if rx.len() < need {
        return RtuPrefix::Incomplete;
    }
    let hdr_len = rx.len().min(3);
    let frame_len = match guess_response_frame_len(&rx[..hdr_len], ModbusProto::Rtu) {
        Ok(n) => n as usize,
        Err(_) => return RtuPrefix::Invalid,
    };
    if frame_len < 4 {
        return RtuPrefix::Invalid;
    }
    if rx.len() < frame_len {
        return RtuPrefix::Incomplete;
    }
    if verify_rtu_crc(&rx[..frame_len]) {
        RtuPrefix::Complete(frame_len)
    } else {
        RtuPrefix::Invalid
    }
}

/// First CRC-valid RTU response in `rx`, any guessed length. Slides on a short
/// header guess so a later CRC-valid frame is not hidden behind junk.
pub fn find_modbus_frame(rx: &[u8]) -> Option<(usize, usize)> {
    let mut offset = 0;
    while offset < rx.len() {
        match classify_rtu_prefix(&rx[offset..]) {
            RtuPrefix::Complete(len) => return Some((offset, len)),
            RtuPrefix::Incomplete | RtuPrefix::Invalid => offset += 1,
        }
    }
    None
}

/// Scan `rx` for a CRC-valid Modbus RTU response for `slave` with length `expected` or exception (5).
pub fn find_modbus_response(rx: &[u8], slave: u8, expected: usize) -> Option<(usize, usize)> {
    if rx.len() < 4 {
        return None;
    }
    let max_off = rx.len().saturating_sub(4);
    for offset in 0..=max_off {
        if rx[offset] != slave {
            continue;
        }
        let avail = rx.len() - offset;
        let hdr_len = avail.min(3);
        let frame_len =
            match guess_response_frame_len(&rx[offset..offset + hdr_len], ModbusProto::Rtu) {
                Ok(n) => n as usize,
                Err(_) => continue,
            };
        if frame_len < 4 || offset + frame_len > rx.len() {
            continue;
        }
        if frame_len != expected && frame_len != 5 {
            continue;
        }
        let frame = &rx[offset..offset + frame_len];
        if verify_rtu_crc(frame) {
            return Some((offset, frame_len));
        }
    }
    None
}

/// Classify a recovered frame relative to the raw capture length.
pub fn classify_recovered(rx_len: usize, offset: usize, frame_len: usize) -> &'static str {
    if offset > 0 {
        "long_resync"
    } else if rx_len > frame_len {
        "long"
    } else {
        "exact"
    }
}

/// Label a failed scan (no CRC-valid frame found).
pub fn classify_modbus_rx_miss(rx: &[u8], req: &[u8], expected: usize) -> &'static str {
    if rx.is_empty() {
        return "empty";
    }
    if rx.iter().all(|&b| b == 0) {
        return "empty";
    }
    let leading_zeros = rx.iter().take_while(|&&b| b == 0).count();
    if leading_zeros > 0 && leading_zeros == rx.len() {
        return "leading_zero";
    }
    let slave = req.first().copied().unwrap_or(1);
    if skip_leading_zeros(rx).first() != Some(&slave) {
        return "leading_junk";
    }
    if rx.len() < expected {
        return "short";
    }
    "parse_fail"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn append_crc(frame: &mut [u8], len: usize) {
        let crc = calc_crc16(&frame[..len]);
        frame[len] = (crc & 0xFF) as u8;
        frame[len + 1] = (crc >> 8) as u8;
    }

    #[test]
    fn find_exact_read_holding() {
        // slave=1 FC=3 byte_count=4 + 4 data bytes + CRC = 9 bytes
        let mut raw = [0u8; 16];
        raw[..7].copy_from_slice(&[0x01, 0x03, 0x04, 0x00, 0x10, 0x00, 0x20]);
        append_crc(&mut raw, 7);
        let found = find_modbus_response(&raw[..9], 1, 9);
        assert_eq!(found, Some((0, 9)));
        assert_eq!(classify_recovered(9, 0, 9), "exact");
    }

    #[test]
    fn find_trailing_junk() {
        let mut raw = [0u8; 16];
        raw[..7].copy_from_slice(&[0x01, 0x03, 0x04, 0x00, 0x10, 0x00, 0x20]);
        append_crc(&mut raw, 7);
        raw[9] = 0xFF;
        raw[10] = 0xFE;
        let found = find_modbus_response(&raw[..11], 1, 9);
        assert_eq!(found, Some((0, 9)));
        assert_eq!(classify_recovered(11, 0, 9), "long");
    }

    #[test]
    fn find_leading_junk_resync() {
        // Match scripts/test_modbus_resync.py: junk prefix + CRC over payload only.
        let mut raw = [0u8; 20];
        raw[0] = 0xAA;
        raw[1] = 0xBB;
        raw[2] = 0xCC;
        raw[3..10].copy_from_slice(&[0x01, 0x03, 0x04, 0x00, 0x10, 0x00, 0x20]);
        let crc = calc_crc16(&raw[3..10]);
        raw[10] = (crc & 0xFF) as u8;
        raw[11] = (crc >> 8) as u8;
        let found = find_modbus_response(&raw[..12], 1, 9);
        assert_eq!(found, Some((3, 9)));
        assert_eq!(classify_recovered(12, 3, 9), "long_resync");
    }

    #[test]
    fn reject_bad_crc() {
        let raw = [0x01, 0x03, 0x04, 0x00, 0x10, 0x00, 0x20, 0x00, 0x00];
        assert!(find_modbus_response(&raw, 1, 9).is_none());
    }

    #[test]
    fn prefix_fc16_incomplete_then_complete() {
        let mut ack = [0u8; 8];
        ack[..6].copy_from_slice(&[0x01, 0x10, 0x00, 0x16, 0x00, 0x02]);
        append_crc(&mut ack, 6);
        assert_eq!(classify_rtu_prefix(&ack[..4]), RtuPrefix::Incomplete);
        assert_eq!(classify_rtu_prefix(&ack), RtuPrefix::Complete(8));
        assert_eq!(find_modbus_frame(&ack), Some((0, 8)));
    }

    #[test]
    fn prefix_invalid_slides_to_frame() {
        let mut raw = [0u8; 12];
        raw[0] = 0xAA;
        raw[1] = 0x00;
        raw[2..8].copy_from_slice(&[0x01, 0x10, 0x00, 0x16, 0x00, 0x02]);
        let crc = calc_crc16(&raw[2..8]);
        raw[8] = (crc & 0xFF) as u8;
        raw[9] = (crc >> 8) as u8;
        assert_eq!(classify_rtu_prefix(&raw[..2]), RtuPrefix::Invalid);
        assert_eq!(find_modbus_frame(&raw[..10]), Some((2, 8)));
    }

    #[test]
    fn find_fc06_behind_fc01_header_guess() {
        let mut raw = [0u8; 12];
        raw[..4].copy_from_slice(&[0x00, 0x01, 0x48, 0x0a]);
        raw[4..10].copy_from_slice(&[0x01, 0x06, 0x00, 0x18, 0x00, 0x3c]);
        append_crc(&mut raw[4..], 6);
        assert_eq!(classify_rtu_prefix(&raw), RtuPrefix::Incomplete);
        assert_eq!(find_modbus_frame(&raw), Some((4, 8)));
    }
}
