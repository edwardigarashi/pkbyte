#![no_main]
#![forbid(unsafe_code)]

use libfuzzer_sys::fuzz_target;
use pkbyte::{build, BuildMode, BuildOptions, DuplicatePolicy, Error, PrepareOptions, Table};
use pkbyte_fuzz::{open_options, INPUT_VALUE_BYTES, ROWS, VALUE_BYTES};
use std::collections::BTreeSet;

const DECODED_BYTES: usize = 12 * 1024;

fuzz_target!(|input: &[u8]| {
    let Some((&control, mut rest)) = input.split_first() else {
        return;
    };
    let mut values = Vec::new();
    let mut decoded_bytes = 0;
    while values.len() < ROWS {
        let Some((&length, payload)) = rest.split_first() else {
            break;
        };
        let length = (usize::from(length) % (INPUT_VALUE_BYTES + 1)).min(payload.len());
        let value = String::from_utf8_lossy(&payload[..length]).into_owned();
        decoded_bytes += value.len();
        if decoded_bytes > DECODED_BYTES {
            break;
        }
        values.push(value);
        rest = &payload[length..];
    }
    let expected: BTreeSet<_> = values.iter().map(String::as_str).collect();
    let mode = [
        BuildMode::Raw,
        BuildMode::Phrase,
        BuildMode::EliasFanoMixed,
        BuildMode::EliasFanoPowerOfTwo,
        BuildMode::Compact,
    ][usize::from(control & 0x7f) % 5];
    let policy = if control & 0x80 == 0 {
        DuplicatePolicy::Deduplicate
    } else {
        DuplicatePolicy::Reject
    };
    let options = BuildOptions {
        mode,
        duplicate_policy: policy,
        max_value_bytes: VALUE_BYTES,
        max_input_rows: ROWS,
        max_input_bytes: DECODED_BYTES,
        training_values: 24,
        training_bytes: 1_024,
        max_symbols: 16,
    };
    let result = build(&values, &options);
    if policy == DuplicatePolicy::Reject && expected.len() != values.len() {
        assert!(matches!(result, Err(Error::DuplicateValue(_))));
        return;
    }
    let built = result.unwrap();
    let raw = build(
        &values,
        &BuildOptions {
            mode: BuildMode::Raw,
            ..options
        },
    )
    .unwrap();
    assert!(built.bytes.len() <= raw.bytes.len());
    let table = Table::open_with_options(&built.bytes, &open_options(true)).unwrap();
    let prepared = table.prepare(&PrepareOptions::default()).unwrap();
    assert_eq!(table.len(), expected.len() as u64);
    let mut checked = Vec::with_capacity(table.max_value_bytes());
    let mut output = prepared.new_output_buffer().unwrap();
    let mut remaining = expected;
    for rank in 0..table.len() {
        let value = table.get_into(rank, &mut checked).unwrap();
        assert!(remaining.remove(value));
        assert_eq!(prepared.get_into(rank, &mut output).unwrap(), value);
    }
    assert!(remaining.is_empty());
});
