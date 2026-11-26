//! Common exact input workloads for timing and build allocation measurements.

use pkbyte::{BuildMode, BuildOptions};
use std::fs;
use std::io;
use std::path::Path;

pub(super) const MODES: [(&str, BuildMode); 5] = [
    ("raw", BuildMode::Raw),
    ("phrase", BuildMode::Phrase),
    ("elias_fano_mixed", BuildMode::EliasFanoMixed),
    ("elias_fano_power_of_two", BuildMode::EliasFanoPowerOfTwo),
    ("compact", BuildMode::Compact),
];
pub(super) const MAX_VALUE_BYTES: usize = 8_192;
pub(super) const SEED: u64 = 0x5a17_e6d9_b043_2c81;

pub(super) fn next_word(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut word = *state;
    word = (word ^ (word >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    word = (word ^ (word >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    word ^ (word >> 31)
}

fn multilingual() -> Vec<String> {
    let mut values = ["", " ", "\t", "\n", "\0", "🌏", "\r\n", "é", "e\u{301}"]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for (prefix, alphabet) in [
        ("東京・", ['あ', 'い', 'う', 'え', 'お']),
        ("κείμενο: ", ['α', 'β', 'γ', 'δ', 'ε']),
        ("قراءة / نص: ", ['ا', 'ب', 'ت', 'ث', 'ج']),
    ] {
        for index in 0..2_048 {
            let mut value = prefix.to_owned();
            let mut code = index;
            for _ in 0..5 {
                value.push(alphabet[code % alphabet.len()]);
                code /= alphabet.len();
            }
            value.push_str(" \0\n");
            values.push(value);
        }
    }
    for index in 0..12 {
        values.push(format!(
            "long #{index:02}\0{}",
            "境".repeat(1_800 + index * 64)
        ));
    }
    values
}

fn high_entropy() -> Vec<String> {
    const WIDTH: usize = 48;
    let mut state = SEED;
    let values: Vec<_> = (0..2_048)
        .map(|index| {
            let mut value = format!("{index:04x}");
            for _ in 4..WIDTH {
                value.push(char::from(33 + (next_word(&mut state) % 94) as u8));
            }
            value
        })
        .collect();
    let universe = (0..WIDTH).try_fold(1u64, |product, position| {
        let mut alphabet = [false; 94];
        for value in &values {
            alphabet[usize::from(value.as_bytes()[position] - 33)] = true;
        }
        product.checked_mul(alphabet.into_iter().filter(|&present| present).count() as u64)
    });
    assert!(universe.is_none());
    values
}

fn urls_and_ids(count: usize) -> Vec<String> {
    (0..count)
        .map(|index| {
            if index % 2 == 0 {
                format!("account:{index:08x}")
            } else {
                format!("https://cdn.example.test/v1/assets/{index:08x}/manifest.json")
            }
        })
        .collect()
}

pub(super) fn build_options(mode: BuildMode) -> BuildOptions {
    BuildOptions {
        mode,
        max_value_bytes: MAX_VALUE_BYTES,
        ..BuildOptions::default()
    }
}

pub(super) fn load_workloads(
    ssa_path: &Path,
    extended: bool,
) -> io::Result<Vec<(&'static str, Vec<String>)>> {
    let mut workloads = vec![
        (
            "ssa_2024",
            fs::read_to_string(ssa_path)?
                .lines()
                .map(str::to_owned)
                .collect(),
        ),
        ("multilingual_and_long", multilingual()),
        ("high_entropy", high_entropy()),
    ];
    if extended {
        workloads.push(("small_urls_and_ids", urls_and_ids(64)));
        workloads.push(("large_urls_and_ids", urls_and_ids(160_000)));
    }
    Ok(workloads)
}
