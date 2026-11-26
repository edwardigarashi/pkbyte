#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;
use pkbyte_fuzz::{exercise_reader, refresh_crc, FILE_BYTES};

fuzz_target!(|input: &[u8]| {
    let input = &input[..input.len().min(FILE_BYTES)];
    exercise_reader(input);
    // Let mutations of a valid file reach semantic and prepared decoding
    // even when they damaged the stored checksum.
    if input.len() >= 64 {
        let mut repaired = input.to_vec();
        refresh_crc(&mut repaired);
        if repaired != input {
            exercise_reader(&repaired);
        }
    }
});
