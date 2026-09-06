//! Non-realtime Modbus RTU RX: accumulate USB-serial chunks, extract by sliding CRC.

use ossm_core::modbus::{classify_rtu_prefix, find_modbus_frame, find_modbus_response, RtuPrefix};

const CAP: usize = 256;

pub struct RxAccumulator {
    buf: [u8; CAP],
    len: usize,
}

impl Default for RxAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl RxAccumulator {
    pub fn new() -> Self {
        Self {
            buf: [0u8; CAP],
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Append `data`. If it would overflow, drop the current capture then append.
    pub fn push(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        if self.len + data.len() > CAP {
            self.len = 0;
        }
        if data.len() > CAP {
            return;
        }
        self.buf[self.len..self.len + data.len()].copy_from_slice(data);
        self.len += data.len();
    }

    /// Next CRC-valid RTU frame from the stream. Keeps leftover bytes after the frame.
    /// Waits (`None`) when a header guess is short. A junk prefix that wants more
    /// bytes must not hide a CRC-valid frame that already sits later in the buffer
    /// (FC06 ACK after a 5-byte USB fragment is the homing case).
    pub fn pop_frame(&mut self) -> Option<Vec<u8>> {
        loop {
            if let Some((off, frame_len)) = find_modbus_frame(&self.buf[..self.len]) {
                if off > 0 {
                    self.drain_prefix(off);
                }
                let frame = self.buf[..frame_len].to_vec();
                self.drain_prefix(frame_len);
                return Some(frame);
            }
            match classify_rtu_prefix(&self.buf[..self.len]) {
                RtuPrefix::Invalid => {
                    if self.len == 0 {
                        return None;
                    }
                    self.drain_prefix(1);
                }
                _ => return None,
            }
        }
    }

    /// First CRC-valid response for `slave` with length `expected` (or exception 5)
    /// whose function code is `req_fc` or `req_fc | 0x80`. Consumes through the
    /// frame and drops trailing bytes.
    pub fn extract(&mut self, slave: u8, expected: usize, req_fc: u8) -> Option<&[u8]> {
        loop {
            let (off, frame_len) = find_modbus_response(&self.buf[..self.len], slave, expected)?;
            let fc = self.buf[off + 1];
            if fc == req_fc || fc == (req_fc | 0x80) {
                if off > 0 {
                    self.buf.copy_within(off..off + frame_len, 0);
                }
                self.len = frame_len;
                return Some(&self.buf[..self.len]);
            }
            self.drain_prefix(off + 1);
        }
    }

    fn drain_prefix(&mut self, n: usize) {
        let n = n.min(self.len);
        if n == 0 {
            return;
        }
        self.buf.copy_within(n..self.len, 0);
        self.len -= n;
    }
}

/// Parse FC03 holding-register payload (big-endian u16s). `frame` is a CRC-valid RTU response.
pub fn parse_fc03_u16(frame: &[u8]) -> Option<Vec<u16>> {
    if frame.len() < 5 || frame[1] != 0x03 {
        return None;
    }
    let n = frame[2] as usize;
    if !n.is_multiple_of(2) || frame.len() < 3 + n + 2 {
        return None;
    }
    let mut out = Vec::with_capacity(n / 2);
    let data = &frame[3..3 + n];
    for chunk in data.chunks_exact(2) {
        out.push(u16::from_be_bytes([chunk[0], chunk[1]]));
    }
    Some(out)
}

/// Expected RTU response length from a request (exception frames are still 5).
pub fn expected_response_len(req: &[u8]) -> usize {
    if req.len() >= 6 {
        match req[1] {
            0x03 | 0x04 => {
                let qty = u16::from_be_bytes([req[4], req[5]]) as usize;
                5 + qty * 2
            }
            0x06 | 0x10 => 8,
            _ => 8,
        }
    } else {
        8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ossm_core::modbus::calc_crc16;

    fn with_crc(payload: &[u8]) -> Vec<u8> {
        let crc = calc_crc16(payload);
        let mut v = payload.to_vec();
        v.push((crc & 0xff) as u8);
        v.push((crc >> 8) as u8);
        v
    }

    fn fc16_ack() -> Vec<u8> {
        // slave=1 FC=16 addr=0x0016 qty=2
        with_crc(&[0x01, 0x10, 0x00, 0x16, 0x00, 0x02])
    }

    fn fc16_request() -> Vec<u8> {
        with_crc(&[
            0x01, 0x10, 0x00, 0x16, 0x00, 0x02, 0x04, 0x00, 0x01, 0x00, 0x00,
        ])
    }

    fn fc06_ack() -> Vec<u8> {
        with_crc(&[0x01, 0x06, 0x00, 0x18, 0x00, 0x3c])
    }

    fn exception_ack() -> Vec<u8> {
        with_crc(&[0x01, 0x90, 0x02])
    }

    #[test]
    fn expected_len_fc16() {
        let req = fc16_request();
        assert_eq!(expected_response_len(&req), 8);
    }

    #[test]
    fn expected_len_fc03() {
        let req = with_crc(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x02]);
        assert_eq!(expected_response_len(&req), 9);
    }

    #[test]
    fn split_ack_1_plus_7() {
        let ack = fc16_ack();
        let mut rx = RxAccumulator::new();
        rx.push(&ack[..1]);
        assert!(rx.extract(1, 8, 0x10).is_none());
        rx.push(&ack[1..]);
        let got = rx.extract(1, 8, 0x10).unwrap();
        assert_eq!(got, ack.as_slice());
        assert_eq!(rx.len(), 8);
    }

    #[test]
    fn split_ack_4_plus_4() {
        let ack = fc16_ack();
        let mut rx = RxAccumulator::new();
        rx.push(&ack[..4]);
        assert!(rx.extract(1, 8, 0x10).is_none());
        rx.push(&ack[4..]);
        assert_eq!(rx.extract(1, 8, 0x10).unwrap(), ack.as_slice());
    }

    #[test]
    fn split_ack_byte_at_a_time() {
        let ack = fc16_ack();
        let mut rx = RxAccumulator::new();
        for (i, b) in ack.iter().enumerate() {
            rx.push(&[*b]);
            if i + 1 < ack.len() {
                assert!(rx.extract(1, 8, 0x10).is_none());
            }
        }
        assert_eq!(rx.extract(1, 8, 0x10).unwrap(), ack.as_slice());
    }

    #[test]
    fn leading_junk_resync() {
        let ack = fc16_ack();
        let mut raw = vec![0xAA, 0xBB, 0xCC];
        raw.extend_from_slice(&ack);
        let mut rx = RxAccumulator::new();
        rx.push(&raw);
        assert_eq!(rx.extract(1, 8, 0x10).unwrap(), ack.as_slice());
        assert_eq!(rx.len(), 8);
    }

    #[test]
    fn trailing_junk_dropped() {
        let ack = fc16_ack();
        let mut raw = ack.clone();
        raw.extend_from_slice(&[0xFF, 0xFE]);
        let mut rx = RxAccumulator::new();
        rx.push(&raw);
        assert_eq!(rx.extract(1, 8, 0x10).unwrap(), ack.as_slice());
        assert_eq!(rx.len(), 8);
    }

    #[test]
    fn fc03_two_registers() {
        let frame = with_crc(&[0x01, 0x03, 0x04, 0x00, 0x10, 0x00, 0x20]);
        let mut rx = RxAccumulator::new();
        rx.push(&frame);
        let got = rx.extract(1, 9, 0x03).unwrap();
        assert_eq!(got, frame.as_slice());
        assert_eq!(parse_fc03_u16(got), Some(vec![0x0010, 0x0020]));
    }

    #[test]
    fn exception_frame() {
        let ex = exception_ack();
        let mut rx = RxAccumulator::new();
        rx.push(&ex);
        let got = rx.extract(1, 8, 0x10).unwrap();
        assert_eq!(got, ex.as_slice());
        assert_eq!(got.len(), 5);
    }

    #[test]
    fn overflow_clears_then_accepts_new() {
        let mut rx = RxAccumulator::new();
        rx.push(&[0x11; 200]);
        rx.push(&[0x22; 100]);
        assert!(rx.len() <= CAP);
        let ack = fc16_ack();
        rx.push(&ack);
        // 100 leftover + 8 may still be in buf; extract should still find the ack
        // if overflow reset: only the ack. Either way CRC scan should hit.
        assert_eq!(rx.extract(1, 8, 0x10).unwrap(), ack.as_slice());
    }

    #[test]
    fn overflow_drop_when_chunk_does_not_fit() {
        let mut rx = RxAccumulator::new();
        rx.push(&[0x11; 200]);
        assert_eq!(rx.len(), 200);
        rx.push(&[0x22; 200]);
        assert_eq!(rx.len(), 200);
        assert_eq!(&rx.buf[..1], &[0x22]);
        rx.clear();
        assert_eq!(rx.len(), 0);
        rx.push(&[0x33; 300]);
        assert_eq!(rx.len(), 0);
    }

    #[test]
    fn pop_frame_split_ack_emits_when_crc_completes() {
        let ack = fc16_ack();
        let mut rx = RxAccumulator::new();
        rx.push(&ack[..4]);
        assert!(rx.pop_frame().is_none());
        rx.push(&ack[4..]);
        assert_eq!(rx.pop_frame().as_deref(), Some(ack.as_slice()));
        assert_eq!(rx.len(), 0);
    }

    #[test]
    fn pop_frame_keeps_leftover() {
        let ack = fc16_ack();
        let mut raw = ack.clone();
        raw.extend_from_slice(&[0xAA, 0xBB]);
        let mut rx = RxAccumulator::new();
        rx.push(&raw);
        assert_eq!(rx.pop_frame().as_deref(), Some(ack.as_slice()));
        assert_eq!(rx.len(), 2);
        assert_eq!(&rx.buf[..2], &[0xAA, 0xBB]);
    }

    #[test]
    fn pop_frame_two_in_one_push() {
        let a = fc16_ack();
        let b = exception_ack();
        let mut raw = a.clone();
        raw.extend_from_slice(&b);
        let mut rx = RxAccumulator::new();
        rx.push(&raw);
        assert_eq!(rx.pop_frame().as_deref(), Some(a.as_slice()));
        assert_eq!(rx.pop_frame().as_deref(), Some(b.as_slice()));
        assert_eq!(rx.len(), 0);
    }

    #[test]
    fn pop_frame_short_header_waits() {
        let mut rx = RxAccumulator::new();
        rx.push(&[0x01]);
        assert!(rx.pop_frame().is_none());
        assert_eq!(rx.len(), 1);
    }

    #[test]
    fn pop_frame_fc06_behind_fc01_leftover() {
        let ack = fc06_ack();
        let mut rx = RxAccumulator::new();
        rx.push(&[0xc0, 0x00, 0x01, 0x48, 0x0a]);
        assert!(rx.pop_frame().is_none());
        rx.push(&ack);
        assert_eq!(rx.pop_frame().as_deref(), Some(ack.as_slice()));
        assert_eq!(rx.len(), 0);
    }
}
