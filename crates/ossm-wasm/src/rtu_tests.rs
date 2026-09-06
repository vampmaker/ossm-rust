use ossm_core::modbus::{aim30, calc_crc16};

use crate::rtu::{
    enable_modbus, expected_response_len, homing, parse_fc03_u16, position_write_frame,
    DEFAULT_SLAVE,
};

fn with_crc(payload: &[u8]) -> Vec<u8> {
    let crc = calc_crc16(payload);
    let mut v = payload.to_vec();
    v.push((crc & 0xff) as u8);
    v.push((crc >> 8) as u8);
    v
}

/// Mock drive: ACKs writes, reports a latched position.
#[derive(Default)]
struct MockDrive {
    power: u16,
    enable: u16,
    accel: u16,
    pos_ring: u16,
    speed_ring: u16,
    position: i32,
}

impl MockDrive {
    fn exchange(&mut self, req: &[u8]) -> Result<Vec<u8>, String> {
        if req.len() < 4 {
            return Err("short".into());
        }
        let fc = req[1];
        match fc {
            0x06 => {
                let addr = u16::from_be_bytes([req[2], req[3]]);
                let val = u16::from_be_bytes([req[4], req[5]]);
                match addr {
                    aim30::REG_ENABLE => self.enable = val,
                    aim30::REG_POWER => self.power = val,
                    aim30::REG_ACCEL => self.accel = val,
                    aim30::REG_POS_RING => self.pos_ring = val,
                    aim30::REG_SPEED_RING => self.speed_ring = val,
                    _ => {}
                }
                Ok(req.to_vec())
            }
            0x10 => {
                let addr = u16::from_be_bytes([req[2], req[3]]);
                if addr == aim30::REG_POSITION && req.len() >= 11 {
                    let lo = u16::from_be_bytes([req[7], req[8]]);
                    let hi = u16::from_be_bytes([req[9], req[10]]);
                    self.position = aim30::unpack_position_i32([lo, hi]);
                }
                Ok(with_crc(&[req[0], 0x10, req[2], req[3], req[4], req[5]]))
            }
            0x03 => {
                let addr = u16::from_be_bytes([req[2], req[3]]);
                let qty = u16::from_be_bytes([req[4], req[5]]) as usize;
                let mut data = Vec::new();
                for i in 0..qty {
                    let a = addr + i as u16;
                    let val = match a {
                        aim30::REG_POWER => self.power,
                        aim30::REG_ENABLE => self.enable,
                        aim30::REG_ACCEL => self.accel,
                        aim30::REG_POS_RING => self.pos_ring,
                        aim30::REG_SPEED_RING => self.speed_ring,
                        aim30::REG_POSITION => self.position as u16,
                        x if x == aim30::REG_POSITION + 1 => (self.position >> 16) as u16,
                        _ => 0,
                    };
                    data.extend_from_slice(&val.to_be_bytes());
                }
                let mut payload = vec![req[0], 0x03, (qty * 2) as u8];
                payload.extend_from_slice(&data);
                Ok(with_crc(&payload))
            }
            _ => Err(format!("fc {fc:#x}")),
        }
    }
}

#[test]
fn expected_len_fc16_and_fc03() {
    let pos = position_write_frame(DEFAULT_SLAVE, 1.0).unwrap();
    assert_eq!(expected_response_len(&pos), 8);
    let fc03 = with_crc(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x02]);
    assert_eq!(expected_response_len(&fc03), 9);
}

#[test]
fn parse_fc03_regs() {
    let frame = with_crc(&[0x01, 0x03, 0x04, 0x00, 0x01, 0x00, 0x02]);
    assert_eq!(parse_fc03_u16(&frame), Some(vec![1, 2]));
}

#[test]
fn mock_homing_completes() {
    let mut drive = MockDrive::default();
    let mut exchange = |req: Vec<u8>, _timeout: u32| {
        let rsp = drive.exchange(&req);
        async move { rsp }
    };
    let mut sleep = |_ms: u32| async {};
    let (pos_min, pos_max) =
        pollster::block_on(homing(&mut exchange, &mut sleep, DEFAULT_SLAVE)).expect("homing");
    assert!(pos_max > pos_min);
}

#[test]
fn enable_modbus_writes_reg() {
    let mut drive = MockDrive::default();
    let mut exchange = |req: Vec<u8>, _timeout: u32| {
        let rsp = drive.exchange(&req);
        async move { rsp }
    };
    pollster::block_on(enable_modbus(&mut exchange, DEFAULT_SLAVE)).unwrap();
    assert_eq!(drive.enable, aim30::ENABLE_MODBUS);
}
