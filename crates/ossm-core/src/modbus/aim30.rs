use core::f32::consts::PI;

/// Holding-register address for 32-bit position (two registers, low then high).
pub const REG_POSITION: u16 = 0x16;
/// Communication / enable register.
pub const REG_ENABLE: u16 = 0x00;
/// Acceleration.
pub const REG_ACCEL: u16 = 0x03;
/// Speed-loop ratio.
pub const REG_SPEED_RING: u16 = 0x05;
/// Position-loop ratio.
pub const REG_POS_RING: u16 = 0x07;
/// Peak current / max power.
pub const REG_POWER: u16 = 0x18;

pub const ENABLE_MODBUS: u16 = 1;
pub const RING_RATIO_RUN: u16 = 3000;
pub const POWER_HOMING: f32 = 0.1;
pub const POWER_RUN: f32 = 0.6;
pub const ACCEL_HOMING: f32 = 1000.0;
pub const ACCEL_RUN: f32 = 4000.0;

pub fn pack_position_i32(position: i32) -> [u16; 2] {
    [position as u16, (position >> 16) as u16]
}

pub fn unpack_position_i32(regs: [u16; 2]) -> i32 {
    (regs[1] as i32) << 16 | (regs[0] as i32)
}

pub fn counts_to_radians(counts: i32) -> f32 {
    counts as f32 / 32768.0 * 2.0 * PI
}

pub fn radians_to_counts(position: f32) -> i32 {
    (position / (2.0 * PI) * 32768.0) as i32
}

/// Firmware avoids writing raw 0 (drive quirk); map that to 1 count.
pub fn write_counts_for_radians(position: f32) -> i32 {
    let counts = radians_to_counts(position);
    if counts == 0 {
        1
    } else {
        counts
    }
}

/// FC16 write of `values` starting at `addr`. Returns the frame length.
pub fn pack_fc16(slave: u8, addr: u16, values: &[u16], out: &mut [u8]) -> Option<usize> {
    let n = values.len();
    let payload = 7 + n * 2;
    let need = payload + 2;
    if out.len() < need || n > 255 {
        return None;
    }
    out[0] = slave;
    out[1] = 0x10;
    out[2] = (addr >> 8) as u8;
    out[3] = addr as u8;
    out[4] = (n >> 8) as u8;
    out[5] = n as u8;
    out[6] = (n * 2) as u8;
    for (i, v) in values.iter().enumerate() {
        out[7 + i * 2] = (*v >> 8) as u8;
        out[8 + i * 2] = *v as u8;
    }
    let crc = super::calc_crc16(&out[..payload]);
    out[payload] = crc as u8;
    out[payload + 1] = (crc >> 8) as u8;
    Some(need)
}

/// 13-byte FC16 write of the AIM30 32-bit position (2 holding registers).
pub fn pack_position_fc16(slave: u8, position: i32) -> [u8; 13] {
    let regs = pack_position_i32(position);
    let mut frame = [0u8; 13];
    pack_fc16(slave, REG_POSITION, &regs, &mut frame).expect("13-byte FC16");
    frame
}

/// Matches firmware `(power * 60.0) as u16 * 10`.
pub fn encode_max_power(power: f32) -> u16 {
    (power * 60.0) as u16 * 10
}

/// Matches firmware `(acceleration * 60.0 / (2π)) as u16`.
pub fn encode_acceleration(acceleration: f32) -> u16 {
    (acceleration * 60.0 / (2.0 * PI)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_roundtrip() {
        let raw = pack_position_i32(-123456);
        assert_eq!(unpack_position_i32(raw), -123456);
    }

    #[test]
    fn write_counts_skips_zero() {
        assert_eq!(write_counts_for_radians(0.0), 1);
    }

    #[test]
    fn pack_position_fc16_layout_and_crc() {
        let frame = pack_position_fc16(1, 0x1234_5678);
        assert_eq!(frame[0], 1);
        assert_eq!(frame[1], 0x10);
        assert_eq!(&frame[2..7], &[0x00, 0x16, 0x00, 0x02, 0x04]);
        assert_eq!(&frame[7..11], &[0x56, 0x78, 0x12, 0x34]);
        assert!(crate::modbus::verify_rtu_crc(&frame));
    }

    #[test]
    fn encode_power_homing_and_run() {
        assert_eq!(encode_max_power(POWER_HOMING), 60);
        assert_eq!(encode_max_power(POWER_RUN), 360);
    }

    #[test]
    fn encode_accel_is_truncated_u16() {
        assert_eq!(
            encode_acceleration(ACCEL_HOMING),
            (1000.0 * 60.0 / (2.0 * PI)) as u16
        );
        assert_eq!(
            encode_acceleration(ACCEL_RUN),
            (4000.0 * 60.0 / (2.0 * PI)) as u16
        );
    }
}
