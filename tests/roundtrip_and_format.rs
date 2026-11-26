use pkbyte::{
    build, uniform_below, BuildMode, BuildOptions, DuplicatePolicy, Error, Limits, OpenOptions,
    Table,
};
use std::collections::BTreeSet;

#[path = "support/checksum.rs"]
mod checksum;
use checksum::{crc32, refresh_crc};

fn raw_options() -> BuildOptions {
    BuildOptions {
        mode: BuildMode::Raw,
        ..BuildOptions::default()
    }
}

fn phrase_options() -> BuildOptions {
    BuildOptions {
        mode: BuildMode::Phrase,
        ..BuildOptions::default()
    }
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn set_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn set_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn set_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

#[derive(Debug)]
struct Group {
    entry: usize,
    codec: u8,
    width: usize,
    count: u64,
    first_rank: u64,
    payload: usize,
}

fn groups(bytes: &[u8]) -> Vec<Group> {
    let directory = 64 + u32_at(bytes, 44) as usize;
    let count = u32_at(bytes, 40) as usize;
    let mut payload = directory + count * 16;
    let mut first_rank = 0;
    let mut result = Vec::new();
    for index in 0..count {
        let entry = directory + index * 16;
        let width = u32_at(bytes, entry + 4) as usize;
        let count = u64_at(bytes, entry + 8);
        result.push(Group {
            entry,
            codec: bytes[entry],
            width,
            count,
            first_rank,
            payload,
        });
        first_rank += count;
        payload += width * count as usize;
    }
    result
}

fn phrase_file(symbols: &[&[u8]], codes: &[u8], decoded_bytes: u64, maximum: u32) -> Vec<u8> {
    let mut dictionary = Vec::new();
    for symbol in symbols {
        dictionary.push(symbol.len() as u8);
        dictionary.extend_from_slice(symbol);
    }
    let mut bytes = vec![0; 64];
    bytes[..8].copy_from_slice(b"NAMPACK\0");
    set_u16(&mut bytes, 8, 1);
    set_u32(&mut bytes, 12, 64);
    set_u64(&mut bytes, 16, 1);
    set_u64(&mut bytes, 24, decoded_bytes);
    set_u32(&mut bytes, 32, maximum);
    set_u16(&mut bytes, 36, symbols.len() as u16);
    set_u32(&mut bytes, 40, 1);
    set_u32(&mut bytes, 44, dictionary.len() as u32);
    set_u64(&mut bytes, 48, codes.len() as u64);
    bytes.extend(dictionary);
    bytes.extend_from_slice(&[1, 0, 0, 0]);
    bytes.extend_from_slice(&(codes.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&1_u64.to_le_bytes());
    bytes.extend_from_slice(codes);
    refresh_crc(&mut bytes);
    bytes
}

fn synthetic_values() -> Vec<String> {
    (0..128)
        .map(|index| format!("aaaaaaaaaaaaaaaaaaaaaaaa{index:08b}bbbbbbbbbbbbbbbbbbbbbbbb"))
        .collect()
}

fn next_random(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[test]
fn crc_reference_matches_standard_check_vector() {
    assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
}

#[test]
fn legacy_v1_fixture_remains_readable_and_rebuild_uses_v2() {
    let fixture = include_bytes!("../data/given_names_2024.npk");
    let source = include_str!("../data/ssa_2024_given_names.txt");
    let table = Table::open(fixture).unwrap();
    assert_eq!(table.version(), 1);
    assert_eq!(u32_at(fixture, 56), 0x3a28_f80f);
    let report = table.validate_all(true).unwrap();
    assert_eq!(report.row_count, 29_329);
    assert_eq!(report.decoded_bytes, 178_667);
    let decoded: BTreeSet<_> = (0..table.len())
        .map(|rank| table.get(rank).unwrap())
        .collect();
    assert!(decoded.iter().map(String::as_str).eq(source.lines()));

    let rebuilt = build(source.lines(), &phrase_options()).unwrap();
    assert_eq!(&rebuilt.bytes[..8], &fixture[..8]);
    assert_eq!(u16_at(&rebuilt.bytes, 8), 2);
    assert_ne!(u32_at(&rebuilt.bytes, 56), u32_at(fixture, 56));
    let table = Table::open(&rebuilt.bytes).unwrap();
    assert_eq!(table.version(), 2);
    table.validate_all(true).unwrap();
    let rebuilt_values: BTreeSet<_> = (0..table.len())
        .map(|rank| table.get(rank).unwrap())
        .collect();
    assert_eq!(rebuilt_values, decoded);
}

#[test]
fn all_ranks_roundtrip_exact_unicode_and_whitespace() {
    let mut values = synthetic_values();
    values.extend(
        [
            "",
            "山田 太郎",
            "佐藤 花子",
            "Élodie",
            "ÉLODIE",
            "E\u{301}lodie",
            "Zoë",
            "Мария",
            "李",
            "محمد",
            "𝄞 音",
            "👩🏽‍💻",
            " leading",
            "trailing ",
            "two  spaces",
            " \t ",
            "A\0B",
            "line\nbreak",
            "\r\n",
            "trailing CR\r",
            "\u{feff}leading BOM",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    let expected: BTreeSet<_> = values.iter().cloned().collect();
    for mode in [BuildMode::Raw, BuildMode::Phrase] {
        let options = BuildOptions {
            mode,
            ..BuildOptions::default()
        };
        let packed = build(&values, &options).unwrap();
        let table = Table::open(&packed.bytes).unwrap();
        assert_eq!(table.version(), 2);
        assert_eq!(table.len() as usize, expected.len());
        let mut decoded = BTreeSet::new();
        let mut scratch = Vec::with_capacity(table.max_value_bytes());
        let capacity = scratch.capacity();
        for rank in 0..table.len() {
            let value = table.get_into(rank, &mut scratch).unwrap().to_owned();
            assert_eq!(table.get(rank).unwrap(), value);
            if let Some(raw) = table.get_raw(rank).unwrap() {
                assert_eq!(raw, value);
            }
            assert!(decoded.insert(value), "duplicate output at rank {rank}");
        }
        assert_eq!(scratch.capacity(), capacity);
        assert_eq!(decoded, expected);
        table.validate_all(true).unwrap();
        table.verify_checksum().unwrap();
    }
}

#[test]
fn every_group_boundary_addresses_the_right_record() {
    let mut values = synthetic_values();
    values.extend(["", "x", "ab", "世", "世界", "ééééé"].map(str::to_owned));
    let packed = build(&values, &phrase_options()).unwrap();
    let table = Table::open(&packed.bytes).unwrap();
    let expected: BTreeSet<_> = values.iter().cloned().collect();
    let parsed = groups(&packed.bytes);
    assert!(!parsed.is_empty());
    for group in parsed {
        for rank in [group.first_rank, group.first_rank + group.count - 1] {
            let decoded = table.get(rank).unwrap();
            assert!(expected.contains(&decoded));
            if group.codec == 0 {
                let offset = group.payload + (rank - group.first_rank) as usize * group.width;
                assert_eq!(
                    decoded.as_bytes(),
                    &packed.bytes[offset..offset + group.width]
                );
            }
        }
    }
    assert!(matches!(
        table.get(table.len()),
        Err(Error::OutOfBounds { .. })
    ));
    assert!(matches!(
        table.get(u64::MAX),
        Err(Error::OutOfBounds { .. })
    ));
}

#[test]
fn compact_mode_never_exceeds_raw_file_for_these_corpora() {
    for values in [
        vec!["Alice".to_owned()],
        vec!["山田 太郎".to_owned(), "Élodie".to_owned()],
        synthetic_values(),
    ] {
        let compact = build(&values, &BuildOptions::default()).unwrap();
        let raw = build(&values, &raw_options()).unwrap();
        assert!(compact.bytes.len() <= raw.bytes.len());
    }
    let tiny = build(["x"], &BuildOptions::default()).unwrap();
    assert_eq!(
        u16_at(&tiny.bytes, 36),
        0,
        "tiny corpus should use raw fallback"
    );
}

#[test]
fn duplicate_policy_is_explicit_and_exact_byte_based() {
    assert!(matches!(
        build(["Alice", "Alice"], &BuildOptions::default()),
        Err(Error::DuplicateValue(_))
    ));
    let options = BuildOptions {
        duplicate_policy: DuplicatePolicy::Deduplicate,
        ..BuildOptions::default()
    };
    assert!(matches!(
        build(["", ""], &BuildOptions::default()),
        Err(Error::DuplicateValue(value)) if value.is_empty()
    ));
    let packed = build(
        ["Alice", "Alice", "", "Élodie", "", "E\u{301}lodie"],
        &options,
    )
    .unwrap();
    assert_eq!(packed.stats.row_count, 4);
    assert_eq!(packed.stats.input_rows, 6);
    assert_eq!(packed.stats.duplicate_rows, 2);
    let table = Table::open(&packed.bytes).unwrap();
    table.validate_all(true).unwrap();
    let decoded: BTreeSet<_> = (0..table.len())
        .map(|rank| table.get(rank).unwrap())
        .collect();
    assert_eq!(
        decoded,
        ["", "Alice", "Élodie", "E\u{301}lodie"]
            .map(str::to_owned)
            .into_iter()
            .collect()
    );
}

#[test]
fn empty_table_and_empty_value_have_distinct_contracts() {
    let reader_options = OpenOptions {
        limits: Limits {
            max_value_bytes: 0,
            ..Limits::default()
        },
        ..OpenOptions::default()
    };
    for mode in [BuildMode::Raw, BuildMode::Compact] {
        let options = BuildOptions {
            mode,
            max_value_bytes: 0,
            ..BuildOptions::default()
        };
        let packed = build(std::iter::empty::<&str>(), &options).unwrap();
        assert_eq!(packed.bytes.len(), 64);
        let table = Table::open_with_options(&packed.bytes, &reader_options).unwrap();
        assert!(table.is_empty());
        assert_eq!(table.max_value_bytes(), 0);
        assert_eq!(table.validate_all(true).unwrap().decoded_bytes, 0);
        let mut scratch = Vec::new();
        let mut unexpected_rng = || panic!("empty table sampling must not consume randomness");
        assert!(matches!(
            table.sample_into(&mut unexpected_rng, &mut scratch),
            Err(Error::EmptyTable)
        ));

        let packed = build([""], &options).unwrap();
        assert_eq!(packed.bytes.len(), 80);
        assert_eq!(u64_at(&packed.bytes, 48), 0);
        let directory = groups(&packed.bytes);
        let [group] = directory.as_slice() else {
            panic!("expected a single zero-width group");
        };
        assert_eq!((group.codec, group.width, group.count), (0, 0, 1));
        let table = Table::open_with_options(&packed.bytes, &reader_options).unwrap();
        assert!(!table.is_empty());
        assert_eq!(table.len(), 1);
        assert_eq!(table.max_value_bytes(), 0);
        let report = table.validate_all(true).unwrap();
        assert_eq!((report.row_count, report.decoded_bytes), (1, 0));
        assert_eq!(table.get_raw(0).unwrap(), Some(""));
        assert_eq!(table.get_into(0, &mut scratch).unwrap(), "");
        assert_eq!(table.sample_into(&mut || 0, &mut scratch).unwrap(), "");
        assert_eq!(scratch.capacity(), 0);

        assert!(matches!(
            build(["x"], &options),
            Err(Error::LimitExceeded(_))
        ));
        let nonempty = build(["x"], &raw_options()).unwrap();
        assert!(matches!(
            Table::open_with_options(&nonempty.bytes, &reader_options),
            Err(Error::LimitExceeded(_))
        ));
    }
}

#[test]
fn builder_input_limits_count_bytes_and_input_rows() {
    let options = BuildOptions {
        max_value_bytes: 3,
        ..BuildOptions::default()
    };
    assert!(build(["世界"], &options).is_err());
    let options = BuildOptions {
        max_input_rows: 1,
        duplicate_policy: DuplicatePolicy::Deduplicate,
        ..BuildOptions::default()
    };
    assert!(build(["A", "A"], &options).is_err());
    let options = BuildOptions {
        max_input_bytes: 3,
        duplicate_policy: DuplicatePolicy::Deduplicate,
        ..BuildOptions::default()
    };
    assert!(build(["AB", "AB"], &options).is_err());
}

#[test]
fn long_values_roundtrip_through_raw_fallback_under_configured_limits() {
    let long = "é".repeat(4_096);
    let mut values = synthetic_values();
    values.extend(["".to_owned(), long.clone()]);
    assert!(matches!(
        build(&values, &BuildOptions::default()),
        Err(Error::LimitExceeded(_))
    ));

    for mode in [BuildMode::Raw, BuildMode::Phrase] {
        let options = BuildOptions {
            mode,
            max_value_bytes: long.len(),
            ..BuildOptions::default()
        };
        let packed = build(&values, &options).unwrap();
        assert_eq!(packed.stats.max_value_bytes, long.len());
        assert!(matches!(
            Table::open(&packed.bytes),
            Err(Error::LimitExceeded(_))
        ));
        let mut open_options = OpenOptions::default();
        open_options.limits.max_value_bytes = long.len() - 1;
        assert!(matches!(
            Table::open_with_options(&packed.bytes, &open_options),
            Err(Error::LimitExceeded(_))
        ));
        open_options.limits.max_value_bytes = long.len();
        let table = Table::open_with_options(&packed.bytes, &open_options).unwrap();
        table.validate_all(true).unwrap();
        let raw_group = groups(&packed.bytes)
            .into_iter()
            .find(|group| group.codec == 0 && group.width == long.len())
            .expect("long value should use raw storage");
        assert_eq!(raw_group.count, 1);
        assert_eq!(
            table.get_raw(raw_group.first_rank).unwrap(),
            Some(long.as_str())
        );
        if mode == BuildMode::Phrase {
            assert!(packed.stats.compressed_rows > 0);
        }

        let bounded_options = BuildOptions {
            max_value_bytes: long.len() - 1,
            ..options
        };
        assert!(matches!(
            build(&values, &bounded_options),
            Err(Error::LimitExceeded(_))
        ));
    }
}

#[test]
fn all_proper_truncations_and_trailing_data_are_rejected() {
    let packed = build(["", "A", "BB", "日本語"], &raw_options()).unwrap();
    for length in 0..packed.bytes.len() {
        assert!(
            Table::open(&packed.bytes[..length]).is_err(),
            "length {length}"
        );
        assert!(Table::open_structural(&packed.bytes[..length]).is_err());
    }
    let mut trailing = packed.bytes;
    trailing.push(0);
    refresh_crc(&mut trailing);
    assert!(Table::open(&trailing).is_err());
}

#[test]
fn invalid_header_and_directory_fields_reject_even_with_valid_crc() {
    let original = build(["A", "BB", "CCC"], &raw_options()).unwrap().bytes;
    let directory = groups(&original)[0].entry;
    let mut mutations = Vec::new();
    for offset in [0, 10, 38, 60, directory + 1] {
        let mut bytes = original.clone();
        bytes[offset] ^= 1;
        mutations.push(bytes);
    }
    let mut too_many_symbols = original.clone();
    set_u16(&mut too_many_symbols, 36, 257);
    mutations.push(too_many_symbols);
    for (offset, value) in [
        (12, 63),
        (40, u32::MAX),
        (44, u32::MAX),
        (directory + 4, 0),
        (directory + 4, u32::MAX),
    ] {
        let mut bytes = original.clone();
        set_u32(&mut bytes, offset, value);
        mutations.push(bytes);
    }
    for (offset, value) in [
        (16, 0),
        (16, u64::MAX),
        (48, u64::MAX),
        (directory + 8, 0),
        (directory + 8, u64::MAX),
    ] {
        let mut bytes = original.clone();
        set_u64(&mut bytes, offset, value);
        mutations.push(bytes);
    }
    let mut unknown_codec = original.clone();
    unknown_codec[directory] = 255;
    mutations.push(unknown_codec);
    let mut duplicate_bucket = original;
    let first_width = u32_at(&duplicate_bucket, directory + 4);
    set_u32(&mut duplicate_bucket, directory + 16 + 4, first_width);
    mutations.push(duplicate_bucket);
    for (index, mut bytes) in mutations.into_iter().enumerate() {
        refresh_crc(&mut bytes);
        assert!(Table::open(&bytes).is_err(), "mutation {index}");
        assert!(Table::open_structural(&bytes).is_err(), "mutation {index}");
    }
}

#[test]
fn unsupported_versions_and_v1_with_empty_value_are_rejected() {
    let original = build(["", "AB"], &raw_options()).unwrap().bytes;
    for version in [0, 4, u16::MAX] {
        let mut bytes = original.clone();
        set_u16(&mut bytes, 8, version);
        refresh_crc(&mut bytes);
        assert!(matches!(
            Table::open(&bytes),
            Err(Error::UnsupportedVersion(found)) if found == version
        ));
    }

    let mut legacy = original;
    set_u16(&mut legacy, 8, 1);
    refresh_crc(&mut legacy);
    assert!(matches!(Table::open(&legacy), Err(Error::InvalidFormat(_))));
}

#[test]
fn malformed_v2_empty_groups_and_metadata_are_rejected_with_valid_crc() {
    let original = build(["", "AB"], &raw_options()).unwrap().bytes;
    let directory = groups(&original);
    let empty = directory[0].entry;
    let nonempty = directory[1].entry;
    let mut zero_count = original.clone();
    set_u64(&mut zero_count, empty + 8, 0);

    let mut two_empty_rows = original.clone();
    set_u64(&mut two_empty_rows, empty + 8, 2);
    set_u64(&mut two_empty_rows, 16, 3);

    let mut duplicate_empty_group = original.clone();
    set_u32(&mut duplicate_empty_group, nonempty + 4, 0);
    set_u64(&mut duplicate_empty_group, 24, 0);
    set_u32(&mut duplicate_empty_group, 32, 0);
    set_u64(&mut duplicate_empty_group, 48, 0);
    duplicate_empty_group.truncate(directory[1].payload);

    let mut out_of_order = original.clone();
    out_of_order[empty..nonempty].copy_from_slice(&original[nonempty..nonempty + 16]);
    out_of_order[nonempty..nonempty + 16].copy_from_slice(&original[empty..nonempty]);

    let mut zero_width_phrases = phrase_file(&[b"A"], &[], 0, 0);
    set_u16(&mut zero_width_phrases, 8, 2);

    let mut extra_payload = original.clone();
    set_u64(&mut extra_payload, 48, 3);
    extra_payload.push(0);

    let mut incorrect_total = original.clone();
    set_u64(&mut incorrect_total, 24, 3);
    let mut incorrect_maximum = original;
    set_u32(&mut incorrect_maximum, 32, 3);

    for (case, mut bytes) in [
        ("zero empty-group count", zero_count),
        ("two empty values in one group", two_empty_rows),
        ("duplicate empty group", duplicate_empty_group),
        ("empty group after a nonempty group", out_of_order),
        ("zero-width phrase-coded group", zero_width_phrases),
        ("payload not covered by groups", extra_payload),
        ("decoded total does not match raw data", incorrect_total),
        ("decoded maximum does not match raw data", incorrect_maximum),
    ] {
        refresh_crc(&mut bytes);
        assert!(Table::open(&bytes).is_err(), "{case}");
        assert!(Table::open_structural(&bytes).is_err(), "{case}");
    }
}

#[test]
fn v2_group_count_arithmetic_rejects_overflow_with_permissive_limits() {
    let mut bytes = build(["", "AB"], &raw_options()).unwrap().bytes;
    let nonempty = groups(&bytes)[1].entry;
    let options = OpenOptions {
        limits: Limits {
            max_rows: u64::MAX,
            max_decoded_bytes: u64::MAX,
            ..Limits::default()
        },
        ..OpenOptions::default()
    };
    for count in [u64::MAX / 2 + 1, u64::MAX] {
        set_u64(&mut bytes, 16, count.saturating_add(1));
        set_u64(&mut bytes, 24, u64::MAX);
        set_u64(&mut bytes, nonempty + 8, count);
        refresh_crc(&mut bytes);
        assert!(Table::open_with_options(&bytes, &options).is_err());
    }
}

#[test]
fn checksum_verification_and_structural_open_are_distinct() {
    let mut bytes = build(["Alpha", "Bravo"], &raw_options()).unwrap().bytes;
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    assert!(matches!(
        Table::open(&bytes),
        Err(Error::ChecksumMismatch { .. })
    ));
    let table = Table::open_structural(&bytes).unwrap();
    assert!(matches!(
        table.verify_checksum(),
        Err(Error::ChecksumMismatch { .. })
    ));
}

#[test]
fn invalid_utf8_is_reported_when_the_selected_record_is_decoded() {
    let mut bytes = build(["", "Alpha"], &raw_options()).unwrap().bytes;
    let payload = groups(&bytes)[1].payload;
    bytes[payload] = 0xff;
    refresh_crc(&mut bytes);
    let table = Table::open(&bytes).unwrap();
    assert_eq!(table.get_raw(0).unwrap(), Some(""));
    assert!(matches!(table.get(1), Err(Error::InvalidUtf8)));
    assert!(matches!(table.get_raw(1), Err(Error::InvalidUtf8)));
    assert!(matches!(table.validate_all(false), Err(Error::InvalidUtf8)));
    let mut buffer = Vec::new();
    let mut source = [1].into_iter();
    assert!(matches!(
        table.sample_into(&mut || source.next().unwrap(), &mut buffer),
        Err(Error::InvalidUtf8)
    ));
}

#[test]
fn semantic_verification_checks_counts_and_uniqueness_separately() {
    let mut bytes = build(["Alpha", "Bravo"], &raw_options()).unwrap().bytes;
    let group = &groups(&bytes)[0];
    let first = bytes[group.payload..group.payload + group.width].to_vec();
    bytes[group.payload + group.width..group.payload + 2 * group.width].copy_from_slice(&first);
    refresh_crc(&mut bytes);
    let table = Table::open(&bytes).unwrap();
    table.validate_all(false).unwrap();
    assert!(table.validate_all(true).is_err());

    let mut wrong_total = build(["Alpha", "Bravo"], &raw_options()).unwrap().bytes;
    set_u64(&mut wrong_total, 24, 11);
    refresh_crc(&mut wrong_total);
    // Eager rejection is also acceptable.
    if let Ok(table) = Table::open(&wrong_total) {
        assert!(table.validate_all(false).is_err());
    }
}

#[test]
fn structural_open_rejects_impossible_decoded_totals() {
    // One row with maximum length one cannot contain two decoded bytes.
    let too_large = phrase_file(&[b"A"], &[0], 2, 1);
    assert!(Table::open_structural(&too_large).is_err());

    // Every code expands to at least one byte, so four code bytes cannot
    // represent only three decoded bytes, even when other totals agree.
    let mut too_small = phrase_file(&[b"A"], &[0, 0, 0, 0], 3, 2);
    let directory = 64 + u32_at(&too_small, 44) as usize;
    set_u64(&mut too_small, 16, 2);
    set_u32(&mut too_small, directory + 4, 2);
    set_u64(&mut too_small, directory + 8, 2);
    refresh_crc(&mut too_small);
    assert!(Table::open_structural(&too_small).is_err());

    let mut mixed = phrase_file(&[b"AA"], &[0], 2, 2);
    let directory = 64 + u32_at(&mixed, 44) as usize;
    let mut empty_group = [0; 16];
    set_u64(&mut empty_group, 8, 1);
    mixed.splice(directory..directory, empty_group);
    set_u16(&mut mixed, 8, 2);
    set_u64(&mut mixed, 16, 2);
    set_u32(&mut mixed, 40, 2);
    refresh_crc(&mut mixed);
    let table = Table::open(&mixed).unwrap();
    assert_eq!(table.get(0).unwrap(), "");
    assert_eq!(table.get(1).unwrap(), "AA");
    table.validate_all(true).unwrap();

    // The empty rank does not contribute to the maximum possible total.
    set_u64(&mut mixed, 24, 3);
    refresh_crc(&mut mixed);
    assert!(Table::open(&mixed).is_err());
}

#[test]
fn raw_only_totals_and_maximum_match_the_directory_exactly() {
    let original = build(["A", "BB", "CCC"], &raw_options()).unwrap().bytes;
    let mut wrong_total = original.clone();
    set_u64(&mut wrong_total, 24, 7);
    refresh_crc(&mut wrong_total);
    assert!(Table::open_structural(&wrong_total).is_err());

    let mut wrong_maximum = original;
    set_u32(&mut wrong_maximum, 32, 4);
    refresh_crc(&mut wrong_maximum);
    assert!(Table::open_structural(&wrong_maximum).is_err());
}

#[test]
fn dictionary_constraints_and_selected_code_bounds_are_checked() {
    let valid = phrase_file(&[b"A"], &[0], 1, 1);
    assert_eq!(Table::open(&valid).unwrap().get(0).unwrap(), "A");
    for bytes in [
        phrase_file(&[b""], &[0], 1, 1),
        phrase_file(&[b"123456789"], &[0], 9, 9),
        phrase_file(&[b"A", b"A"], &[0], 1, 1),
        phrase_file(&[], &[0], 1, 1),
    ] {
        assert!(Table::open(&bytes).is_err());
    }
    let undefined = phrase_file(&[b"A"], &[255], 1, 1);
    let table = Table::open(&undefined).unwrap();
    assert!(table.get(0).is_err());
    assert!(table.validate_all(false).is_err());
    let invalid_unicode = phrase_file(&[&[0xff]], &[0], 1, 1);
    let table = Table::open(&invalid_unicode).unwrap();
    assert!(matches!(table.get(0), Err(Error::InvalidUtf8)));
    let excessive_expansion = phrase_file(&[b"12345678"], &[0], 1, 1);
    assert!(Table::open(&excessive_expansion).unwrap().get(0).is_err());
}

#[test]
fn learned_phrase_codec_is_exercised_and_corrupt_tokens_fail() {
    let options = BuildOptions {
        max_symbols: 16,
        ..phrase_options()
    };
    let mut packed = build(synthetic_values(), &options).unwrap();
    let group = groups(&packed.bytes)
        .into_iter()
        .find(|group| group.codec == 1)
        .expect("repetitive synthetic corpus should exercise phrase encoding");
    assert!(u16_at(&packed.bytes, 36) <= 16);
    Table::open(&packed.bytes)
        .unwrap()
        .validate_all(true)
        .unwrap();
    packed.bytes[group.payload] = 255;
    refresh_crc(&mut packed.bytes);
    let table = Table::open(&packed.bytes).unwrap();
    assert!(table.get(group.first_rank).is_err());
}

#[test]
fn reader_limits_reject_before_unbounded_work() {
    let bytes = build(["A", "BB", "CCC"], &raw_options()).unwrap().bytes;
    let limits = [
        Limits {
            max_file_bytes: bytes.len() - 1,
            ..Limits::default()
        },
        Limits {
            max_rows: 2,
            ..Limits::default()
        },
        Limits {
            max_value_bytes: 2,
            ..Limits::default()
        },
        Limits {
            max_groups: 1,
            ..Limits::default()
        },
        Limits {
            max_decoded_bytes: 5,
            ..Limits::default()
        },
    ];
    for limits in limits {
        let options = OpenOptions {
            verify_checksum: false,
            limits,
        };
        assert!(Table::open_with_options(&bytes, &options).is_err());
    }
}

#[test]
fn bounded_rng_rejects_low_remainder_and_handles_extreme_bounds() {
    assert!(matches!(
        uniform_below(0, &mut || panic!("unexpected RNG use")),
        Err(Error::EmptyTable)
    ));
    assert_eq!(uniform_below(1, &mut || u64::MAX).unwrap(), 0);
    assert_eq!(uniform_below(64, &mut || u64::MAX).unwrap(), 63);
    let sequence = [0, 5, 6];
    let mut calls = 0;
    let mut rng = || {
        let value = sequence[calls];
        calls += 1;
        value
    };
    assert_eq!(uniform_below(10, &mut rng).unwrap(), 6);
    assert_eq!(calls, 3);
    let mut sequence = [0, u64::MAX].into_iter();
    assert_eq!(
        uniform_below(u64::MAX, &mut || sequence.next().unwrap()).unwrap(),
        0
    );
    for bound in [2, 3, 7, 10, 257, 1 << 63, (1 << 63) + 1, u64::MAX] {
        let mut state = 7;
        for _ in 0..1_000 {
            assert!(uniform_below(bound, &mut || next_random(&mut state)).unwrap() < bound);
        }
    }
}

#[test]
fn sampling_agrees_with_independent_rank_selection() {
    let packed = build(
        ["", "Alice", "Bob", "山田", "Élodie", "Zoë"],
        &raw_options(),
    )
    .unwrap();
    let table = Table::open(&packed.bytes).unwrap();
    let mut state1 = 42;
    let mut state2 = 42;
    let mut buffer = Vec::with_capacity(table.max_value_bytes());
    let buffer_start = buffer.as_ptr();
    let capacity = buffer.capacity();
    let mut sampled_empty = false;
    for _ in 0..1_000 {
        let rank = uniform_below(table.len(), &mut || next_random(&mut state1)).unwrap();
        let value = table
            .sample_into(&mut || next_random(&mut state2), &mut buffer)
            .unwrap();
        assert_eq!(value, table.get(rank).unwrap());
        sampled_empty |= value.is_empty();
    }
    assert!(sampled_empty);
    assert_eq!(buffer.capacity(), capacity);
    assert_eq!(buffer.as_ptr(), buffer_start);
}

#[test]
fn thousands_of_bounded_malformed_inputs_never_panic() {
    let valid = build(["", "A", "BB", "山田", "Élodie", "Zoë"], &raw_options())
        .unwrap()
        .bytes;
    let mut state = 0xfedc_ba98_7654_3210;
    for iteration in 0..4_096 {
        let mut bytes = if iteration % 2 == 0 {
            let length = (next_random(&mut state) % 512) as usize;
            (0..length)
                .map(|_| next_random(&mut state) as u8)
                .collect::<Vec<_>>()
        } else {
            let mut bytes = valid.clone();
            for _ in 0..1 + iteration % 4 {
                let index = (next_random(&mut state) % bytes.len() as u64) as usize;
                bytes[index] ^= (next_random(&mut state) as u8) | 1;
            }
            bytes
        };
        if iteration % 3 == 0 && bytes.len() >= 64 {
            refresh_crc(&mut bytes);
        }
        let outcome = std::panic::catch_unwind(|| {
            let options = OpenOptions {
                verify_checksum: iteration % 3 == 0,
                limits: Limits {
                    max_file_bytes: 8_192,
                    max_rows: 512,
                    max_value_bytes: 256,
                    max_groups: 128,
                    max_decoded_bytes: 65_536,
                },
            };
            if let Ok(table) = Table::open_with_options(&bytes, &options) {
                let mut scratch = Vec::new();
                for rank in [0, table.len() / 2, table.len().saturating_sub(1)] {
                    let _ = table.get_into(rank, &mut scratch);
                }
                let _ = table.validate_all(false);
            }
        });
        assert!(outcome.is_ok(), "malformed input iteration {iteration}");
    }
}
