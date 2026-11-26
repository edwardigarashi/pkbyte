use pkbyte::{build, BuildMode, BuildOptions, Error, OpenOptions, PrepareOptions, Table};
use std::collections::BTreeSet;

#[path = "support/checksum.rs"]
mod checksum;
use checksum::refresh_crc;

fn ef_values() -> Vec<String> {
    // Every combination appears, so the varying byte positions have a
    // dense mixed-radix domain and gaps in the power-of-two representation.
    (0..3_usize.pow(6))
        .map(|mut ordinal| {
            let mut digits = [b'a'; 6];
            for digit in digits.iter_mut().rev() {
                *digit += (ordinal % 3) as u8;
                ordinal /= 3;
            }
            let accent = ["é", "ê", "ë"][usize::from(digits[0] - b'a')];
            let suffix = std::str::from_utf8(&digits[1..]).unwrap();
            format!("青🐱\0\nvalue/{accent}{suffix}")
        })
        .collect()
}

#[test]
fn complete_candidates_roundtrip_the_same_exact_set() {
    let mut values = ef_values();
    let ef_rows = values.len() as u64;
    let long = "世".repeat(2_000);
    let overflow_group = ["a".repeat(64), "z".repeat(64)];
    values.extend(
        [
            "",
            " \t ",
            "A\0B",
            "A\r\nB",
            "Élodie",
            "E\u{301}lodie",
            "尾🦀",
            "x",
        ]
        .map(str::to_owned),
    );
    values.push(long.clone());
    values.extend(overflow_group.clone());
    let expected: BTreeSet<_> = values.iter().cloned().collect();
    let mut open_options = OpenOptions::default();
    open_options.limits.max_value_bytes = long.len();
    let mut smallest_forced_candidate = usize::MAX;

    for mode in [
        BuildMode::Raw,
        BuildMode::Phrase,
        BuildMode::EliasFanoMixed,
        BuildMode::EliasFanoPowerOfTwo,
        BuildMode::Compact,
    ] {
        let options = BuildOptions {
            mode,
            max_value_bytes: long.len(),
            ..BuildOptions::default()
        };
        let packed = build(&values, &options).unwrap();
        assert_eq!(
            packed.bytes,
            build(values.iter().rev(), &options).unwrap().bytes,
            "{mode:?}"
        );
        if mode == BuildMode::Compact {
            assert_eq!(packed.bytes.len(), smallest_forced_candidate);
        } else {
            smallest_forced_candidate = smallest_forced_candidate.min(packed.bytes.len());
        }

        let table = Table::open_with_options(&packed.bytes, &open_options).unwrap();
        let stats = table.stats();
        assert_eq!(stats.ef_mixed_rows, packed.stats.ef_mixed_rows);
        assert_eq!(
            stats.ef_power_of_two_rows,
            packed.stats.ef_power_of_two_rows
        );
        assert_eq!(stats.ef_bytes, packed.stats.ef_bytes);
        let uses_ef = stats.ef_mixed_rows + stats.ef_power_of_two_rows > 0;
        assert_eq!(table.version(), if uses_ef { 3 } else { 2 });
        match mode {
            BuildMode::Raw => assert_eq!(stats.raw_rows, table.len()),
            BuildMode::Phrase => {
                assert!(stats.compressed_rows > 0);
                assert_eq!(stats.ef_bytes, 0);
            }
            BuildMode::EliasFanoMixed => {
                assert_eq!(stats.ef_mixed_rows, ef_rows);
                assert_eq!(stats.ef_power_of_two_rows, 0);
            }
            BuildMode::EliasFanoPowerOfTwo => {
                assert_eq!(stats.ef_mixed_rows, 0);
                assert_eq!(stats.ef_power_of_two_rows, ef_rows);
            }
            BuildMode::Compact => {}
        }

        table.validate_all(true).unwrap();
        let prepared = table.prepare(&PrepareOptions::default()).unwrap();
        let mut output = prepared.new_output_buffer().unwrap();
        let mut checked = Vec::with_capacity(table.max_value_bytes());
        let mut decoded = BTreeSet::new();
        for rank in 0..table.len() {
            let value = table.get_into(rank, &mut checked).unwrap();
            assert_eq!(prepared.get_into(rank, &mut output).unwrap(), value);
            assert!(decoded.insert(value.to_owned()), "{mode:?} rank {rank}");
            if matches!(
                mode,
                BuildMode::EliasFanoMixed | BuildMode::EliasFanoPowerOfTwo
            ) {
                if value == long || overflow_group.iter().any(|raw| raw == value) {
                    assert_eq!(table.get_raw(rank).unwrap(), Some(value));
                } else if value.starts_with("青🐱") {
                    assert_eq!(table.get_raw(rank).unwrap(), None);
                }
            }
        }
        assert_eq!(decoded, expected, "{mode:?}");
    }
}

#[test]
fn ef_modes_fall_back_to_v2_for_groups_that_cannot_shrink() {
    let values = ["", "x", "y", "世"];
    let raw = build(
        values,
        &BuildOptions {
            mode: BuildMode::Raw,
            ..BuildOptions::default()
        },
    )
    .unwrap();
    for mode in [BuildMode::EliasFanoMixed, BuildMode::EliasFanoPowerOfTwo] {
        let packed = build(
            values,
            &BuildOptions {
                mode,
                ..BuildOptions::default()
            },
        )
        .unwrap();
        assert_eq!(packed.bytes, raw.bytes);
        assert_eq!(packed.stats.ef_bytes, 0);
        let table = Table::open(&packed.bytes).unwrap();
        assert_eq!(table.version(), 2);
        assert_eq!(table.stats().raw_rows, values.len() as u64);
        table.validate_all(true).unwrap();
    }
}

#[test]
fn generated_subsets_preserve_exact_values_and_counts() {
    for seed in 1..=12u32 {
        let radix = 2 + seed % 4;
        let mut state = seed;
        let mut expected = BTreeSet::from([String::new()]);
        for ordinal in 0..radix.pow(6) {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            if state >> 30 == 0 {
                let mut suffix = [b'a'; 6];
                let mut value = ordinal;
                for digit in suffix.iter_mut().rev() {
                    *digit += (value % radix) as u8;
                    value /= radix;
                }
                let prefix = if ordinal % 2 == 0 { "🐱\0" } else { "青\n" };
                expected.insert(format!(
                    "{prefix}{}{suffix}",
                    "固定".repeat(seed as usize % 3),
                    suffix = std::str::from_utf8(&suffix).unwrap()
                ));
                if expected.len() == 193 {
                    break;
                }
            }
        }
        for mode in [BuildMode::EliasFanoMixed, BuildMode::EliasFanoPowerOfTwo] {
            let packed = build(
                &expected,
                &BuildOptions {
                    mode,
                    ..BuildOptions::default()
                },
            )
            .unwrap();
            let table = Table::open(&packed.bytes).unwrap();
            let report = table.validate_all(true).unwrap();
            assert_eq!(report.row_count, expected.len() as u64);
            let actual = (0..table.len())
                .map(|rank| table.get(rank).unwrap())
                .collect::<BTreeSet<_>>();
            assert_eq!(actual, expected, "{mode:?}, seed {seed}");
        }
    }
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

fn set_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn set_u64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

struct EfBlock {
    directory: usize,
    start: usize,
    alphabets: usize,
    lows: usize,
    highs: usize,
    checkpoints: usize,
}

fn single_ef_block(mode: BuildMode, codec: u8) -> (Vec<u8>, EfBlock) {
    let packed = build(
        ef_values(),
        &BuildOptions {
            mode,
            ..BuildOptions::default()
        },
    )
    .unwrap();
    let bytes = packed.bytes;
    assert_eq!(u16_at(&bytes, 8), 3);
    assert_eq!(u32_at(&bytes, 40), 1);
    assert_eq!(u32_at(&bytes, 44), 0);
    let directory = 64;
    assert_eq!(bytes[directory], codec);
    let row_count = u64_at(&bytes, directory + 8) as usize;
    let start = directory + 16;
    let alphabets = start + 24;
    let lows = alphabets + u32_at(&bytes, start + 12) as usize;
    let highs = lows + (row_count * usize::from(bytes[start + 20])).div_ceil(64) * 8;
    let checkpoints = highs + (u32_at(&bytes, start + 16) as usize).div_ceil(64) * 8;
    assert_eq!(checkpoints + row_count.div_ceil(64) * 4, bytes.len());
    assert_eq!(u32_at(&bytes, start + 8) as usize, bytes.len() - start);
    assert_eq!(packed.stats.ef_bytes, bytes.len() - start);
    let table = Table::open(&bytes).unwrap();
    table.validate_all(true).unwrap();
    (
        bytes,
        EfBlock {
            directory,
            start,
            alphabets,
            lows,
            highs,
            checkpoints,
        },
    )
}

fn assert_invalid_structure(mut bytes: Vec<u8>, case: &str) {
    refresh_crc(&mut bytes);
    assert!(Table::open(&bytes).is_err(), "{case}");
    assert!(Table::open_structural(&bytes).is_err(), "{case}");
}

#[test]
fn duplicate_across_raw_and_ef_groups_fails_full_validation() {
    let (original, block) = single_ef_block(BuildMode::EliasFanoMixed, 2);
    let duplicate = Table::open(&original).unwrap().get(0).unwrap();
    let width = duplicate.len();
    let mut bytes = original[..64].to_vec();
    set_u64(&mut bytes, 16, u64_at(&original, 16) + 1);
    set_u64(&mut bytes, 24, u64_at(&original, 24) + width as u64);
    set_u32(&mut bytes, 40, 2);
    set_u64(&mut bytes, 48, u64_at(&original, 48) + width as u64);
    bytes.extend([0, 0, 0, 0]);
    bytes.extend((width as u32).to_le_bytes());
    bytes.extend(1u64.to_le_bytes());
    bytes.extend_from_slice(&original[block.directory..block.start]);
    bytes.extend_from_slice(duplicate.as_bytes());
    bytes.extend_from_slice(&original[block.start..]);
    refresh_crc(&mut bytes);

    let table = Table::open(&bytes).unwrap();
    assert_eq!(table.get_raw(0).unwrap(), Some(duplicate.as_str()));
    assert_eq!(table.get(1).unwrap(), duplicate);
    table.validate_all(false).unwrap();
    assert!(matches!(
        table.validate_all(true),
        Err(Error::DuplicateValue(value)) if value == duplicate
    ));
    assert!(matches!(
        table.prepare(&PrepareOptions::default()),
        Err(Error::DuplicateValue(value)) if value == duplicate
    ));
}

#[test]
fn ef_codec_ids_are_rejected_under_older_versions() {
    for (mode, codec) in [
        (BuildMode::EliasFanoMixed, 2),
        (BuildMode::EliasFanoPowerOfTwo, 3),
    ] {
        let (original, _) = single_ef_block(mode, codec);
        for version in [1_u16, 2] {
            let mut bytes = original.clone();
            bytes[8..10].copy_from_slice(&version.to_le_bytes());
            assert_invalid_structure(bytes, "EF codec under an older format version");
        }
    }
}

#[test]
fn ef_reader_applies_checked_file_and_decoded_limits() {
    let (bytes, _) = single_ef_block(BuildMode::EliasFanoMixed, 2);
    let table = Table::open(&bytes).unwrap();
    let stats = table.stats();
    let limits = [
        pkbyte::Limits {
            max_file_bytes: bytes.len() - 1,
            ..pkbyte::Limits::default()
        },
        pkbyte::Limits {
            max_rows: table.len() - 1,
            ..pkbyte::Limits::default()
        },
        pkbyte::Limits {
            max_value_bytes: table.max_value_bytes() - 1,
            ..pkbyte::Limits::default()
        },
        pkbyte::Limits {
            max_decoded_bytes: stats.raw_value_bytes - 1,
            ..pkbyte::Limits::default()
        },
        pkbyte::Limits {
            max_groups: 0,
            ..pkbyte::Limits::default()
        },
    ];
    for limits in limits {
        let options = OpenOptions {
            limits,
            ..OpenOptions::default()
        };
        assert!(matches!(
            Table::open_with_options(&bytes, &options),
            Err(Error::LimitExceeded(_))
        ));
    }
}

#[test]
fn malformed_ef_structure_is_rejected_even_with_a_valid_crc() {
    let (original, block) = single_ef_block(BuildMode::EliasFanoMixed, 2);
    for (case, offset, value) in [
        ("zero decoded width", block.directory + 4, 0),
        ("undersized block", block.start + 8, 23),
        ("oversized block", block.start + 8, u32::MAX),
        ("zero alphabet bytes", block.start + 12, 0),
        ("oversized alphabet section", block.start + 12, u32::MAX),
        ("empty high stream", block.start + 16, 0),
        ("oversized high stream", block.start + 16, u32::MAX),
        (
            "checkpoint outside high stream",
            block.checkpoints,
            u32::MAX,
        ),
        (
            "checkpoint at the wrong bit",
            block.checkpoints + 4,
            u32_at(&original, block.checkpoints + 4) + 1,
        ),
    ] {
        let mut bytes = original.clone();
        set_u32(&mut bytes, offset, value);
        assert_invalid_structure(bytes, case);
    }
    for (case, offset, value) in [
        ("unknown codec", block.directory, 255),
        ("directory reserved byte", block.directory + 2, 1),
        ("unsupported low width", block.start + 20, 64),
        ("EF reserved byte", block.start + 23, 1),
    ] {
        let mut bytes = original.clone();
        bytes[offset] = value;
        assert_invalid_structure(bytes, case);
    }
    for size in [0_u16, 257] {
        let mut bytes = original.clone();
        bytes[block.alphabets..block.alphabets + 2].copy_from_slice(&size.to_le_bytes());
        assert_invalid_structure(bytes, "invalid alphabet cardinality");
    }

    assert_eq!(original[block.alphabets + 2..block.alphabets + 5], *b"abc");
    let mut repeated = original.clone();
    repeated[block.alphabets + 2] = b'b';
    assert_invalid_structure(repeated, "repeated alphabet byte");
    let mut unsorted = original.clone();
    unsorted.swap(block.alphabets + 2, block.alphabets + 3);
    assert_invalid_structure(unsorted, "unsorted alphabet");

    let mut wrong_population = original.clone();
    wrong_population[block.highs] &= !1;
    assert_invalid_structure(wrong_population, "missing high-stream bit");

    let high_bits = u32_at(&original, block.start + 16) as usize;
    assert_ne!(high_bits % 64, 0);
    let mut high_padding = original.clone();
    high_padding[block.highs + high_bits / 8] |= 1 << (high_bits % 8);
    assert_invalid_structure(high_padding, "nonzero high padding");

    let mut invalid_base = original.clone();
    set_u64(&mut invalid_base, block.start, u64::MAX);
    assert_invalid_structure(invalid_base, "overflowing base and delta");

    let mut huge_count = original;
    set_u64(&mut huge_count, block.directory + 8, u64::MAX);
    assert_invalid_structure(huge_count, "oversized EF group count");
}

#[test]
fn power_of_two_low_stream_requires_zero_padding_and_strict_integers() {
    let (original, block) = single_ef_block(BuildMode::EliasFanoPowerOfTwo, 3);
    assert_eq!(original[block.start + 20], 1);
    let low_bits = u64_at(&original, block.directory + 8) as usize;
    assert_ne!(low_bits % 64, 0);
    let mut low_padding = original.clone();
    low_padding[block.lows + low_bits / 8] |= 1 << (low_bits % 8);
    assert_invalid_structure(low_padding, "nonzero low padding");

    // Integers zero and one have the same high part. Clearing the low bit
    // of the second integer makes it equal to the first.
    assert_eq!(original[block.lows] & 0b11, 0b10);
    let mut duplicate = original;
    duplicate[block.lows] &= !0b10;
    assert_invalid_structure(duplicate, "duplicate EF integer");
}

#[test]
fn forbidden_power_of_two_digits_fail_retrieval_and_preparation() {
    let (mut bytes, block) = single_ef_block(BuildMode::EliasFanoPowerOfTwo, 3);
    assert_eq!(bytes[block.start + 20], 1);
    assert_eq!(bytes[block.lows] & 0b111, 0b010);
    // The alphabets have three symbols but radix four. Turn the third
    // integer from two into three, while preserving strict ordering.
    bytes[block.lows] |= 0b100;
    refresh_crc(&mut bytes);
    let table = Table::open(&bytes).unwrap();
    assert!(table.get(1).is_ok());
    assert!(matches!(table.get(2), Err(Error::InvalidFormat(_))));
    assert!(table.get(3).is_ok());
    assert!(table.validate_all(true).is_err());
    assert!(table.prepare(&PrepareOptions::default()).is_err());
}

#[test]
fn ef_utf8_errors_fail_retrieval_and_preparation() {
    let (mut bytes, block) = single_ef_block(BuildMode::EliasFanoMixed, 2);
    // This preserves the ascending three-byte alphabet and changes only
    // the strings using its last symbol.
    bytes[block.alphabets + 4] = 0xff;
    refresh_crc(&mut bytes);
    let table = Table::open(&bytes).unwrap();
    assert!(table.get(1).is_ok());
    assert!(matches!(table.get(2), Err(Error::InvalidUtf8)));
    assert!(matches!(table.validate_all(true), Err(Error::InvalidUtf8)));
    assert!(matches!(
        table.prepare(&PrepareOptions::default()),
        Err(Error::InvalidUtf8)
    ));
}

fn mixed_block_with_initial_gap(gap: u32) -> Vec<u8> {
    const ROWS: u32 = 4096;
    const WIDTH: u32 = 3;
    const BLOCK_START: usize = 80;
    let mut alphabets = Vec::new();
    for size in [32_u16, 32, 8] {
        alphabets.extend_from_slice(&size.to_le_bytes());
        alphabets.extend(b' '..b' ' + size as u8);
    }
    let high_bits = gap + 2 * (ROWS - 1) + 1;
    let mut highs = vec![0; high_bits.div_ceil(64) as usize * 8];
    let mut checkpoints = Vec::new();
    for rank in 0..ROWS {
        let position = if rank == 0 { 0 } else { gap + 2 * rank };
        highs[position as usize / 8] |= 1 << (position % 8);
        if rank % 64 == 0 {
            checkpoints.extend_from_slice(&position.to_le_bytes());
        }
    }

    let mut bytes = vec![0; BLOCK_START + 24];
    bytes[..8].copy_from_slice(b"NAMPACK\0");
    bytes[8..10].copy_from_slice(&3_u16.to_le_bytes());
    set_u32(&mut bytes, 12, 64);
    set_u64(&mut bytes, 16, u64::from(ROWS));
    set_u64(&mut bytes, 24, u64::from(ROWS * WIDTH));
    set_u32(&mut bytes, 32, WIDTH);
    set_u32(&mut bytes, 40, 1);
    bytes[64] = 2;
    set_u32(&mut bytes, 68, WIDTH);
    set_u64(&mut bytes, 72, u64::from(ROWS));
    set_u32(&mut bytes, BLOCK_START + 12, alphabets.len() as u32);
    set_u32(&mut bytes, BLOCK_START + 16, high_bits);
    bytes.extend(alphabets);
    bytes.extend(highs);
    bytes.extend(checkpoints);
    let block_size = bytes.len() - BLOCK_START;
    set_u64(&mut bytes, 48, block_size as u64);
    set_u32(&mut bytes, BLOCK_START + 8, block_size as u32);
    refresh_crc(&mut bytes);
    bytes
}

#[test]
fn ef_checkpoint_span_is_bounded_for_external_files() {
    const GAP_AT_LIMIT: u32 = 2048 - 2 * 63;
    let bounded = mixed_block_with_initial_gap(GAP_AT_LIMIT);
    let table = Table::open(&bounded).unwrap();
    table.validate_all(true).unwrap();
    let prepared = table.prepare(&PrepareOptions::default()).unwrap();
    let mut output = prepared.new_output_buffer().unwrap();
    assert_eq!(
        prepared.get_into(63, &mut output).unwrap(),
        table.get(63).unwrap()
    );

    let overwide = mixed_block_with_initial_gap(GAP_AT_LIMIT + 1);
    assert_invalid_structure(overwide, "excessive span after one checkpoint");
}
