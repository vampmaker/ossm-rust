use core::f32::consts::PI;

/// Holding-register address for 32-bit position (two registers, low then high).
pub const REG_POSITION: u16 = 0x16;

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
}
