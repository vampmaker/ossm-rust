//! SLIP framing used by the ESP ROM serial protocol.

pub const END: u8 = 0xC0;
pub const ESC: u8 = 0xDB;
pub const ESC_END: u8 = 0xDC;
pub const ESC_ESC: u8 = 0xDD;

pub fn encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 4);
    out.push(END);
    for &b in data {
        match b {
            END => out.extend_from_slice(&[ESC, ESC_END]),
            ESC => out.extend_from_slice(&[ESC, ESC_ESC]),
            _ => out.push(b),
        }
    }
    out.push(END);
    out
}

#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
    esc: bool,
    in_frame: bool,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.buf.clear();
        self.esc = false;
        self.in_frame = false;
    }

    /// Push bytes. Completed frames are appended to `frames`.
    pub fn push(&mut self, data: &[u8], frames: &mut Vec<Vec<u8>>) {
        for &b in data {
            if !self.in_frame {
                if b == END {
                    self.in_frame = true;
                    self.esc = false;
                    self.buf.clear();
                }
                continue;
            }
            if self.esc {
                self.esc = false;
                match b {
                    ESC_END => self.buf.push(END),
                    ESC_ESC => self.buf.push(ESC),
                    END => {
                        if !self.buf.is_empty() {
                            frames.push(std::mem::take(&mut self.buf));
                        }
                        self.in_frame = true;
                    }
                    _ => self.buf.push(b),
                }
                continue;
            }
            match b {
                ESC => self.esc = true,
                END => {
                    if !self.buf.is_empty() {
                        frames.push(std::mem::take(&mut self.buf));
                    }
                    self.in_frame = true;
                }
                _ => self.buf.push(b),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_with_escapes() {
        let payload = vec![0x00, END, 0x08, ESC, 0x55];
        let wire = encode(&payload);
        let mut dec = Decoder::new();
        let mut frames = Vec::new();
        dec.push(&wire, &mut frames);
        assert_eq!(frames, vec![payload]);
    }

    #[test]
    fn split_across_chunks() {
        let payload = b"hello".to_vec();
        let wire = encode(&payload);
        let mut dec = Decoder::new();
        let mut frames = Vec::new();
        dec.push(&wire[..3], &mut frames);
        assert!(frames.is_empty());
        dec.push(&wire[3..], &mut frames);
        assert_eq!(frames, vec![payload]);
    }
}
