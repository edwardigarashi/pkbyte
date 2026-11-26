#![forbid(unsafe_code)]

use pkbyte::{Error, Limits, OpenOptions, PrepareOptions, SamplingMethod, Table};

pub const FILE_BYTES: usize = 16 * 1024;
pub const ROWS: usize = 128;
pub const INPUT_VALUE_BYTES: usize = 96;
pub const VALUE_BYTES: usize = INPUT_VALUE_BYTES * 3;

pub fn open_options(verify_checksum: bool) -> OpenOptions {
    OpenOptions {
        verify_checksum,
        limits: Limits {
            max_file_bytes: FILE_BYTES,
            max_rows: ROWS as u64,
            max_value_bytes: VALUE_BYTES,
            max_groups: 128,
            max_decoded_bytes: 12 * 1024,
        },
    }
}

/// Independent, bit-at-a-time IEEE CRC-32 of a pkbyte file.
pub fn refresh_crc(bytes: &mut [u8]) {
    let Some(checksum) = bytes.get_mut(56..60) else {
        return;
    };
    checksum.fill(0);
    let mut crc = !0u32;
    for &byte in bytes.iter() {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let odd = crc & 1;
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(odd));
        }
    }
    bytes[56..60].copy_from_slice(&(!crc).to_le_bytes());
}

pub fn exercise_reader(input: &[u8]) {
    let Ok(table) = Table::open_with_options(input, &open_options(false)) else {
        assert!(Table::open_with_options(input, &open_options(true)).is_err());
        return;
    };
    assert_eq!(
        Table::open_with_options(input, &open_options(true)).is_ok(),
        table.verify_checksum().is_ok()
    );

    let mut checked = Vec::with_capacity(table.max_value_bytes());
    assert!(matches!(
        table.get_into(table.len(), &mut checked),
        Err(Error::OutOfBounds { .. })
    ));
    let seed = input
        .iter()
        .rev()
        .take(8)
        .fold(0u64, |value, &byte| value.rotate_left(8) ^ u64::from(byte));
    if !table.is_empty() {
        for rank in [0, table.len() - 1, seed % table.len()] {
            if let Ok(Some(raw)) = table.get_raw(rank) {
                assert_eq!(table.get_into(rank, &mut checked).unwrap(), raw);
            } else {
                let _ = table.get_into(rank, &mut checked);
            }
        }
    }
    let full_ok = table.validate_all(true).is_ok();
    let prepared = table.prepare(&PrepareOptions {
        sampling: SamplingMethod::Modulo,
        max_hint_bytes: ((seed as usize) % 256) * 4,
    });
    assert_eq!(prepared.is_ok(), full_ok);
    let Ok(prepared) = prepared else {
        return;
    };
    let mut output = prepared.new_output_buffer().unwrap();
    assert_eq!(prepared.len(), table.len());
    if table.is_empty() {
        assert!(matches!(
            prepared.sample_into(&mut || panic!("empty table consumed RNG"), &mut output),
            Err(Error::EmptyTable)
        ));
        return;
    }
    for rank in [0, table.len() - 1, seed % table.len()] {
        assert_eq!(
            prepared.get_into(rank, &mut output).unwrap(),
            table.get_into(rank, &mut checked).unwrap()
        );
    }
    let mut first_checked = [seed, u64::MAX].into_iter();
    let mut first_prepared = first_checked.clone();
    assert_eq!(
        table
            .sample_into(&mut || first_checked.next().unwrap(), &mut checked)
            .unwrap(),
        prepared
            .sample_into(&mut || first_prepared.next().unwrap(), &mut output)
            .unwrap()
    );
}
