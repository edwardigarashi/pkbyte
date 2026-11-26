use pkbyte::{
    build, BuildMode, BuildOptions, Error, OpenOptions, PrepareOptions, SamplingMethod, Table,
};
use std::collections::BTreeSet;

#[path = "support/checksum.rs"]
mod checksum;
use checksum::refresh_crc;

fn raw_options() -> BuildOptions {
    BuildOptions {
        mode: BuildMode::Raw,
        ..BuildOptions::default()
    }
}

fn compressible_values() -> Vec<String> {
    (0..128)
        .map(|index| format!("aaaaaaaaaaaaaaaaaaaaaaaa{index:08b}bbbbbbbbbbbbbbbbbbbbbbbb"))
        .collect()
}

fn assert_all_ranks_match(table: &Table<'_>, options: &PrepareOptions) -> usize {
    let prepared = table.prepare(options).unwrap();
    assert_eq!(prepared.len(), table.len());
    assert_eq!(prepared.is_empty(), table.is_empty());
    assert!(prepared.hint_bytes() <= options.max_hint_bytes);
    assert!(prepared.additional_heap_bytes() >= prepared.hint_bytes());

    let output_padding = if table.stats().compressed_rows == 0 {
        0
    } else {
        7
    };
    assert_eq!(
        prepared.output_buffer_len(),
        table.max_value_bytes() + output_padding
    );
    let mut output = prepared.new_output_buffer().unwrap();
    assert_eq!(output.len(), prepared.output_buffer_len());
    let mut checked_output = Vec::with_capacity(table.max_value_bytes());
    for rank in 0..table.len() {
        let expected = table.get_into(rank, &mut checked_output).unwrap();
        assert_eq!(prepared.get_into(rank, &mut output).unwrap(), expected);
    }
    prepared.hint_bytes()
}

#[test]
fn prepared_reads_every_rank_of_the_frozen_v1_fixture() {
    let bytes = include_bytes!("../data/given_names_2024.npk");
    let table = Table::open(bytes).unwrap();
    assert_eq!(table.version(), 1);
    assert_all_ranks_match(&table, &PrepareOptions::default());
}

#[test]
fn prepared_reads_exact_values_in_raw_and_compact_v2_files() {
    let mut values = compressible_values();
    values.extend(
        [
            "",
            " \t ",
            "A\0B",
            "leading\r\nand trailing\n",
            "漢字👩🏽‍💻",
            "Élodie",
            "E\u{301}lodie",
        ]
        .map(str::to_owned),
    );
    let long_value = "世".repeat(2_000);
    values.push(long_value.clone());

    let mut open = OpenOptions::default();
    open.limits.max_value_bytes = long_value.len();
    for mode in [BuildMode::Raw, BuildMode::Phrase] {
        let packed = build(
            &values,
            &BuildOptions {
                mode,
                max_value_bytes: long_value.len(),
                ..BuildOptions::default()
            },
        )
        .unwrap();
        let table = Table::open_with_options(&packed.bytes, &open).unwrap();
        assert_eq!(table.version(), 2);
        assert_eq!(table.stats().compressed_rows > 0, mode == BuildMode::Phrase);
        let options = PrepareOptions::default();
        assert_all_ranks_match(&table, &options);

        let prepared = table.prepare(&options).unwrap();
        let mut output = prepared.new_output_buffer().unwrap();
        let decoded: BTreeSet<_> = (0..prepared.len())
            .map(|rank| prepared.get_into(rank, &mut output).unwrap().to_owned())
            .collect();
        assert_eq!(decoded, values.iter().cloned().collect());
    }
}

#[test]
fn group_search_matches_the_checked_reader_with_small_and_large_hint_budgets() {
    let mut values = vec![String::new()];
    for width in 1..=300 {
        for byte in b'a'..=b'a' + (width % 5) as u8 {
            values.push(char::from(byte).to_string().repeat(width));
        }
    }
    let packed = build(&values, &raw_options()).unwrap();
    let table = Table::open(&packed.bytes).unwrap();
    assert_eq!(table.stats().groups, 301);

    for budget in [0, 3, 4, 7, 8, 64, 4096] {
        let hint_bytes = assert_all_ranks_match(
            &table,
            &PrepareOptions {
                max_hint_bytes: budget,
                ..PrepareOptions::default()
            },
        );
        if budget < 8 {
            assert_eq!(hint_bytes, 0);
        } else {
            assert!(hint_bytes > 0);
            assert_eq!(hint_bytes % std::mem::size_of::<u32>(), 0);
        }
    }
}

#[test]
fn prepared_requires_output_slice_length_and_checks_rank_boundaries() {
    let packed = build(["", "東京"], &raw_options()).unwrap();
    let table = Table::open(&packed.bytes).unwrap();
    let prepared = table.prepare(&PrepareOptions::default()).unwrap();
    let required = prepared.output_buffer_len();
    let mut output = Vec::with_capacity(required + 32);
    output.resize(required - 1, 0);
    let too_short = |result: pkbyte::Result<&str>| {
        assert!(matches!(
            result,
            Err(Error::OutputTooSmall {
                required: needed,
                provided
            }) if needed == required && provided == required - 1
        ));
    };
    too_short(prepared.get_into(0, &mut output));
    too_short(prepared.sample_into(
        &mut || panic!("short output must not read the RNG"),
        &mut output,
    ));

    output.resize(required, 0xff);
    assert_eq!(prepared.get_into(1, &mut output).unwrap(), "東京");
    assert_eq!(prepared.get_into(0, &mut output).unwrap(), "");
    for rank in [prepared.len(), u64::MAX] {
        assert!(matches!(
            prepared.get_into(rank, &mut output),
            Err(Error::OutOfBounds { index, len }) if index == rank && len == prepared.len()
        ));
    }
}

#[test]
fn prepared_distinguishes_an_empty_table_from_an_empty_value() {
    let packed = build(std::iter::empty::<&str>(), &raw_options()).unwrap();
    let table = Table::open(&packed.bytes).unwrap();
    let prepared = table.prepare(&PrepareOptions::default()).unwrap();
    assert!(prepared.is_empty());
    assert_eq!(prepared.len(), 0);
    assert_eq!(prepared.hint_bytes(), 0);
    assert_eq!(prepared.output_buffer_len(), 0);
    let mut output = prepared.new_output_buffer().unwrap();
    assert!(matches!(
        prepared.sample_into(
            &mut || panic!("empty table must not read the RNG"),
            &mut output
        ),
        Err(Error::EmptyTable)
    ));

    let packed = build([""], &raw_options()).unwrap();
    let table = Table::open(&packed.bytes).unwrap();
    let prepared = table.prepare(&PrepareOptions::default()).unwrap();
    assert!(!prepared.is_empty());
    assert_eq!(prepared.len(), 1);
    assert_eq!(prepared.output_buffer_len(), 0);
    assert_eq!(prepared.sample_into(&mut || 0, &mut output).unwrap(), "");
}

fn split_mix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[test]
fn prepared_modulo_sampling_matches_the_checked_sequence() {
    let packed = build(["", "a", "b", "c", "d", "e"], &raw_options()).unwrap();
    let table = Table::open(&packed.bytes).unwrap();
    let prepared = table
        .prepare(&PrepareOptions {
            sampling: SamplingMethod::Modulo,
            ..PrepareOptions::default()
        })
        .unwrap();
    let mut checked_output = Vec::new();
    let mut output = prepared.new_output_buffer().unwrap();

    // For six ranks the checked sampler rejects source values below four.
    let mut draws = [0, 1, 3, 4, u64::MAX].into_iter();
    let mut checked_draws = draws.clone();
    assert_eq!(
        prepared
            .sample_into(&mut || draws.next().unwrap(), &mut output)
            .unwrap(),
        table
            .sample_into(&mut || checked_draws.next().unwrap(), &mut checked_output)
            .unwrap()
    );
    assert_eq!(draws.as_slice(), &[u64::MAX]);
    assert_eq!(draws.as_slice(), checked_draws.as_slice());

    let mut state = 42;
    let mut checked_state = state;
    for _ in 0..2_048 {
        assert_eq!(
            prepared
                .sample_into(&mut || split_mix(&mut state), &mut output)
                .unwrap(),
            table
                .sample_into(&mut || split_mix(&mut checked_state), &mut checked_output)
                .unwrap()
        );
    }
    assert_eq!(state, checked_state);
}

#[test]
fn prepared_multiply_high_sampling_rejects_and_samples_with_replacement() {
    let packed = build(["", "a", "b", "c", "d", "e"], &raw_options()).unwrap();
    let table = Table::open(&packed.bytes).unwrap();
    assert!(matches!(
        PrepareOptions::default().sampling,
        SamplingMethod::MultiplyHigh
    ));
    let prepared = table.prepare(&PrepareOptions::default()).unwrap();
    let mut output = prepared.new_output_buffer().unwrap();

    // Multiplying zero yields a rejected low word for six ranks. Multiplying
    // one yields low word six and high word zero.
    let mut draws = [0, 1, u64::MAX].into_iter();
    assert_eq!(
        prepared
            .sample_into(&mut || draws.next().unwrap(), &mut output)
            .unwrap(),
        ""
    );
    assert_eq!(draws.as_slice(), &[u64::MAX]);
    let last = table.get(table.len() - 1).unwrap();
    for _ in 0..2 {
        assert_eq!(
            prepared.sample_into(&mut || u64::MAX, &mut output).unwrap(),
            last
        );
    }
}

fn trailing_short_phrase_file() -> Vec<u8> {
    let dictionary = b"\x08abcdefgh\x01Z";
    let mut bytes = vec![0; 64];
    bytes[..8].copy_from_slice(b"NAMPACK\0");
    bytes[8..10].copy_from_slice(&2_u16.to_le_bytes());
    bytes[12..16].copy_from_slice(&64_u32.to_le_bytes());
    bytes[16..24].copy_from_slice(&1_u64.to_le_bytes());
    bytes[24..32].copy_from_slice(&9_u64.to_le_bytes());
    bytes[32..36].copy_from_slice(&9_u32.to_le_bytes());
    bytes[36..38].copy_from_slice(&2_u16.to_le_bytes());
    bytes[40..44].copy_from_slice(&1_u32.to_le_bytes());
    bytes[44..48].copy_from_slice(&(dictionary.len() as u32).to_le_bytes());
    bytes[48..56].copy_from_slice(&2_u64.to_le_bytes());
    bytes.extend_from_slice(dictionary);
    bytes.extend_from_slice(&[1, 0, 0, 0]);
    bytes.extend_from_slice(&2_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u64.to_le_bytes());
    bytes.extend_from_slice(&[0, 1]);
    refresh_crc(&mut bytes);
    bytes
}

#[test]
fn prepared_phrase_ending_at_the_decoded_maximum_returns_only_the_value() {
    let bytes = trailing_short_phrase_file();
    let table = Table::open(&bytes).unwrap();
    let prepared = table.prepare(&PrepareOptions::default()).unwrap();
    assert_eq!(prepared.output_buffer_len(), 16);
    let mut output = [0xff; 16];
    assert_eq!(prepared.get_into(0, &mut output).unwrap(), "abcdefghZ");
    assert_eq!(
        prepared.sample_into(&mut || 0, &mut output).unwrap(),
        "abcdefghZ"
    );
}

#[test]
fn preparation_checks_crc_utf8_and_uniqueness_of_external_files() {
    let original = build(["A", "B"], &raw_options()).unwrap().bytes;
    let mut bad_crc = original.clone();
    *bad_crc.last_mut().unwrap() = b'C';
    let table = Table::open_structural(&bad_crc).unwrap();
    assert!(matches!(
        table.prepare(&PrepareOptions::default()),
        Err(Error::ChecksumMismatch { .. })
    ));

    let mut bad_utf8 = original.clone();
    *bad_utf8.last_mut().unwrap() = 0xff;
    refresh_crc(&mut bad_utf8);
    let table = Table::open(&bad_utf8).unwrap();
    assert!(matches!(
        table.prepare(&PrepareOptions::default()),
        Err(Error::InvalidUtf8)
    ));

    let mut duplicate = original;
    let last = duplicate.len() - 1;
    duplicate[last] = duplicate[last - 1];
    refresh_crc(&mut duplicate);
    let table = Table::open(&duplicate).unwrap();
    assert!(matches!(
        table.prepare(&PrepareOptions::default()),
        Err(Error::DuplicateValue(value)) if value == "A"
    ));
}

#[test]
fn preparation_rejects_undefined_phrase_codes_with_valid_crc() {
    let mut bytes = trailing_short_phrase_file();
    *bytes.last_mut().unwrap() = 2;
    refresh_crc(&mut bytes);
    let table = Table::open(&bytes).unwrap();
    assert!(matches!(
        table.prepare(&PrepareOptions::default()),
        Err(Error::InvalidFormat(_))
    ));
}
