// Use a bit-at-a-time CRC so malformed fixtures reach structural or semantic
// checks without relying on the implementation under test.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

pub fn refresh_crc(bytes: &mut [u8]) {
    bytes[56..60].fill(0);
    let value = crc32(bytes);
    bytes[56..60].copy_from_slice(&value.to_le_bytes());
}
