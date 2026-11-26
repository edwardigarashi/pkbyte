// The wire signature predates the pkbyte crate and CLI names.
pub(crate) const MAGIC: &[u8; 8] = b"NAMPACK\0";
pub(crate) const VERSION_V1: u16 = 1;
pub(crate) const VERSION_V2: u16 = 2;
pub(crate) const VERSION_V3: u16 = 3;
pub(crate) const HEADER_LEN: usize = 64;
pub(crate) const DIR_ENTRY_LEN: usize = 16;
pub(crate) const MAX_SYMBOL_BYTES: usize = 8;
pub(crate) const MAX_SYMBOLS: usize = 256;
pub(crate) const RAW: u8 = 0;
pub(crate) const PHRASES: u8 = 1;

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut j = 0;
        while j < 8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            j += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}
const CRC_TABLE: [u32; 256] = crc_table();

/// IEEE CRC-32 of the entire file with its checksum field zeroed.
pub(crate) fn checksum(bytes: &[u8]) -> u32 {
    let mut c = !0u32;
    for (i, &value) in bytes.iter().enumerate() {
        let byte = if (56..60).contains(&i) { 0 } else { value };
        c = CRC_TABLE[((c ^ u32::from(byte)) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}
