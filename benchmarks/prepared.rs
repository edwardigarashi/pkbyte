//! Compare prepared and checked sampling against identical rank traces.
#![forbid(unsafe_code)]

use pkbyte::{
    build, uniform_below, BuildOptions, Limits, OpenOptions, PrepareOptions, PreparedTable,
    SamplingMethod, Table,
};
use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::hint::black_box;
use std::mem::size_of_val;
use std::time::Instant;

const SEED: u64 = 0x5a17_e6d9_b043_2c81;
const HASH_START: u64 = 0xcbf2_9ce4_8422_2325;
const HASH_MULTIPLIER: u64 = 0x100_0000_01b3;
const MIXED_MAX_VALUE_BYTES: usize = 8_192;

#[derive(Clone, Copy)]
enum Method {
    CheckedRetrieval,
    PreparedNoHintRetrieval,
    PreparedHintedRetrieval,
    CheckedModulo,
    PreparedModulo,
    CheckedMultiplyHigh,
    PreparedMultiplyHigh,
}

impl Method {
    const ALL: [Self; 7] = [
        Self::CheckedRetrieval,
        Self::PreparedNoHintRetrieval,
        Self::PreparedHintedRetrieval,
        Self::CheckedModulo,
        Self::PreparedModulo,
        Self::CheckedMultiplyHigh,
        Self::PreparedMultiplyHigh,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::CheckedRetrieval => "checked_retrieval",
            Self::PreparedNoHintRetrieval => "prepared_no_hint_retrieval",
            Self::PreparedHintedRetrieval => "prepared_hinted_retrieval",
            Self::CheckedModulo => "checked_modulo_sampling",
            Self::PreparedModulo => "prepared_modulo_sampling",
            Self::CheckedMultiplyHigh => "checked_retrieval_with_multiply_high",
            Self::PreparedMultiplyHigh => "prepared_multiply_high_sampling",
        }
    }

    fn trace(self) -> Trace {
        match self {
            Self::CheckedMultiplyHigh | Self::PreparedMultiplyHigh => Trace::MultiplyHigh,
            _ => Trace::Modulo,
        }
    }
}

#[derive(Clone, Copy)]
enum Trace {
    Modulo,
    MultiplyHigh,
}

impl Trace {
    fn name(self) -> &'static str {
        match self {
            Self::Modulo => "modulo",
            Self::MultiplyHigh => "multiply_high",
        }
    }
}

struct Readers<'table, 'file> {
    checked: &'table Table<'file>,
    no_hint: PreparedTable<'table>,
    hinted_modulo: PreparedTable<'table>,
    hinted_multiply_high: PreparedTable<'table>,
}

fn next_word(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut word = *state;
    word = (word ^ (word >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    word = (word ^ (word >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    word ^ (word >> 31)
}

fn multiply_high(bound: u64, threshold: u64, next: &mut impl FnMut() -> u64) -> u64 {
    loop {
        let product = u128::from(next()) * u128::from(bound);
        if product as u64 >= threshold {
            return (product >> 64) as u64;
        }
    }
}

fn consume(checksum: u64, value: &str) -> u64 {
    let checksum = (checksum ^ value.len() as u64).wrapping_mul(HASH_MULTIPLIER);
    value.bytes().fold(checksum, |state, byte| {
        (state ^ u64::from(byte)).wrapping_mul(HASH_MULTIPLIER)
    })
}

fn measure<T>(input: impl IntoIterator<Item = T>, step: impl FnMut(u64, T) -> u64) -> (u128, u64) {
    let start = Instant::now();
    let checksum = input.into_iter().fold(HASH_START, step);
    let elapsed = start.elapsed().as_nanos();
    (elapsed, black_box(checksum))
}

fn time_method(
    method: Method,
    readers: &Readers<'_, '_>,
    ranks: &[u64],
    checked_scratch: &mut Vec<u8>,
    prepared_scratch: &mut [u8],
) -> (u128, u64) {
    let mut state = black_box(SEED);
    let mut random = || next_word(&mut state);
    let iterations = ranks.len();
    match method {
        Method::CheckedRetrieval => measure(ranks.iter().copied(), |hash, rank| {
            consume(
                hash,
                readers.checked.get_into(rank, checked_scratch).unwrap(),
            )
        }),
        Method::PreparedNoHintRetrieval => measure(ranks.iter().copied(), |hash, rank| {
            consume(
                hash,
                readers.no_hint.get_into(rank, prepared_scratch).unwrap(),
            )
        }),
        Method::PreparedHintedRetrieval => measure(ranks.iter().copied(), |hash, rank| {
            consume(
                hash,
                readers
                    .hinted_modulo
                    .get_into(rank, prepared_scratch)
                    .unwrap(),
            )
        }),
        Method::CheckedModulo => measure(0..iterations, |hash, _| {
            consume(
                hash,
                readers
                    .checked
                    .sample_into(&mut random, checked_scratch)
                    .unwrap(),
            )
        }),
        Method::PreparedModulo => measure(0..iterations, |hash, _| {
            consume(
                hash,
                readers
                    .hinted_modulo
                    .sample_into(&mut random, prepared_scratch)
                    .unwrap(),
            )
        }),
        Method::CheckedMultiplyHigh => {
            let bound = readers.checked.len();
            let threshold = bound.wrapping_neg() % bound;
            measure(0..iterations, |hash, _| {
                let rank = multiply_high(bound, threshold, &mut random);
                consume(
                    hash,
                    readers.checked.get_into(rank, checked_scratch).unwrap(),
                )
            })
        }
        Method::PreparedMultiplyHigh => measure(0..iterations, |hash, _| {
            consume(
                hash,
                readers
                    .hinted_multiply_high
                    .sample_into(&mut random, prepared_scratch)
                    .unwrap(),
            )
        }),
    }
}

fn checksum_for_ranks(table: &Table<'_>, ranks: impl IntoIterator<Item = u64>) -> u64 {
    let mut output = Vec::with_capacity(table.max_value_bytes());
    ranks.into_iter().fold(HASH_START, |checksum, rank| {
        consume(checksum, table.get_into(rank, &mut output).unwrap())
    })
}

fn check_all_values(
    readers: &Readers<'_, '_>,
    expected: Option<&[String]>,
    checked_scratch: &mut Vec<u8>,
    prepared_scratch: &mut [u8],
) {
    let mut expected =
        expected.map(|values| values.iter().map(String::as_str).collect::<BTreeSet<_>>());
    for rank in 0..readers.checked.len() {
        let value = readers.checked.get_into(rank, checked_scratch).unwrap();
        assert_eq!(
            value,
            readers.no_hint.get_into(rank, prepared_scratch).unwrap()
        );
        assert_eq!(
            value,
            readers
                .hinted_modulo
                .get_into(rank, prepared_scratch)
                .unwrap()
        );
        assert_eq!(
            value,
            readers
                .hinted_multiply_high
                .get_into(rank, prepared_scratch)
                .unwrap()
        );
        if let Some(expected) = &mut expected {
            assert!(expected.remove(value));
        }
    }
    if let Some(expected) = expected {
        assert!(expected.is_empty());
    }
}

fn median(samples: &[u128]) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let midpoint = sorted.len() / 2;
    if midpoint * 2 == sorted.len() {
        (sorted[midpoint - 1] as f64 + sorted[midpoint] as f64) / 2.0
    } else {
        sorted[midpoint] as f64
    }
}

fn prepare_times(
    table: &Table<'_>,
    options: [&PrepareOptions; 2],
    rounds: usize,
) -> [Vec<u128>; 2] {
    let mut samples = [Vec::with_capacity(rounds), Vec::with_capacity(rounds)];
    for round in 0..=rounds {
        for offset in 0..options.len() {
            let index = (offset + round) % options.len();
            let start = Instant::now();
            let prepared = black_box(table).prepare(options[index]).unwrap();
            let elapsed = start.elapsed().as_nanos();
            black_box(&prepared);
            if round != 0 {
                samples[index].push(elapsed);
            }
        }
    }
    samples
}

fn benchmark_dataset(
    name: &str,
    table: &Table<'_>,
    expected: Option<&[String]>,
    iterations: usize,
    rounds: usize,
) -> Result<(), Box<dyn Error>> {
    assert!(!table.is_empty());
    table.validate_all(true)?;
    let hinted_multiply_options = PrepareOptions {
        sampling: SamplingMethod::MultiplyHigh,
        ..PrepareOptions::default()
    };
    let hinted_modulo_options = PrepareOptions {
        sampling: SamplingMethod::Modulo,
        ..PrepareOptions::default()
    };
    let no_hint_options = PrepareOptions {
        max_hint_bytes: 0,
        sampling: SamplingMethod::Modulo,
    };
    let preparations = prepare_times(table, [&no_hint_options, &hinted_modulo_options], rounds);
    let readers = Readers {
        checked: table,
        no_hint: table.prepare(&no_hint_options)?,
        hinted_modulo: table.prepare(&hinted_modulo_options)?,
        hinted_multiply_high: table.prepare(&hinted_multiply_options)?,
    };
    let mut checked_scratch = Vec::with_capacity(table.max_value_bytes());
    let mut prepared_scratch = readers.hinted_modulo.new_output_buffer()?;
    check_all_values(
        &readers,
        expected,
        &mut checked_scratch,
        &mut prepared_scratch,
    );

    let bound = table.len();
    let threshold = bound.wrapping_neg() % bound;
    let mut state = SEED;
    let ranks: Vec<u64> = (0..iterations)
        .map(|_| uniform_below(bound, &mut || next_word(&mut state)).unwrap())
        .collect();
    let modulo_checksum = checksum_for_ranks(table, ranks.iter().copied());
    state = SEED;
    let multiply_checksum = checksum_for_ranks(
        table,
        (0..iterations).map(|_| multiply_high(bound, threshold, &mut || next_word(&mut state))),
    );
    let mut batches: [Vec<u128>; Method::ALL.len()] =
        std::array::from_fn(|_| Vec::with_capacity(rounds));
    for round in 0..=rounds {
        for offset in 0..Method::ALL.len() {
            let index = (offset + round) % Method::ALL.len();
            let method = Method::ALL[index];
            let (elapsed, checksum) = time_method(
                method,
                &readers,
                &ranks,
                &mut checked_scratch,
                &mut prepared_scratch,
            );
            let expected_checksum = match method.trace() {
                Trace::Modulo => modulo_checksum,
                Trace::MultiplyHigh => multiply_checksum,
            };
            assert_eq!(checksum, expected_checksum, "{name} {}", method.name());
            if round != 0 {
                batches[index].push(elapsed);
            }
        }
    }

    let stats = table.stats();
    println!("    {{");
    println!("      \"dataset\": \"{name}\",");
    println!("      \"format_version\": {},", table.version());
    println!("      \"rows\": {},", table.len());
    println!("      \"file_bytes\": {},", stats.file_bytes);
    println!("      \"raw_rows\": {},", stats.raw_rows);
    println!("      \"phrase_rows\": {},", stats.compressed_rows);
    println!("      \"groups\": {},", stats.groups);
    println!("      \"decoded_bytes\": {},", stats.raw_value_bytes);
    println!("      \"max_value_bytes\": {},", table.max_value_bytes());
    println!(
        "      \"hint_budget_bytes\": {},",
        hinted_modulo_options.max_hint_bytes
    );
    println!(
        "      \"prepared_inline_bytes\": {},",
        size_of_val(&readers.hinted_modulo)
    );
    println!(
        "      \"checked_scratch_capacity_bytes\": {},",
        checked_scratch.capacity()
    );
    println!(
        "      \"prepared_scratch_required_bytes\": {},",
        readers.hinted_modulo.output_buffer_len()
    );
    println!(
        "      \"prepared_scratch_capacity_bytes\": {},",
        prepared_scratch.capacity()
    );
    println!("      \"checksums_hex\": {{\"modulo\": \"{modulo_checksum:016x}\", \"multiply_high\": \"{multiply_checksum:016x}\"}},");
    println!("      \"preparation\": [");
    for (index, ((mode, prepared), samples)) in [
        ("without_hints", &readers.no_hint),
        ("with_hints", &readers.hinted_modulo),
    ]
    .into_iter()
    .zip(preparations.iter())
    .enumerate()
    {
        let comma = if index == 0 { "," } else { "" };
        println!(
            "        {{\"method\": \"{mode}\", \"hint_bytes\": {}, \"additional_heap_bytes\": {}, \"median_ns\": {:.3}, \"elapsed_ns\": {samples:?}}}{comma}",
            prepared.hint_bytes(),
            prepared.additional_heap_bytes(),
            median(samples),
        );
    }
    println!("      ],");
    println!("      \"warm\": [");
    for (index, (method, samples)) in Method::ALL.into_iter().zip(batches.iter()).enumerate() {
        let comma = if index + 1 == Method::ALL.len() {
            ""
        } else {
            ","
        };
        println!(
            "        {{\"method\": \"{}\", \"trace\": \"{}\", \"median_ns_per_value\": {:.3}, \"elapsed_ns\": {samples:?}}}{comma}",
            method.name(),
            method.trace().name(),
            median(samples) / iterations as f64,
        );
    }
    println!("      ]");
    print!("    }}");
    Ok(())
}

fn mixed_v2_values() -> Vec<String> {
    let mut values = ["", " ", "\t", "\n", "\0", "🌏", "\r\n", "é", "e\u{0301}"]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let prefixes = [
        "directory item",
        "東京都の項目",
        "κατάλογος στοιχείων",
        "البند في الدليل",
        "élément du répertoire",
        "🌏 items",
    ];
    for index in 0..8_192 {
        if index % 16 == 0 {
            values.push(format!("#{:03x}", index / 16));
            continue;
        }
        values.push(format!(
            "{} #{index:04x}\t{}\n",
            prefixes[index % prefixes.len()],
            "資料".repeat(index % 7),
        ));
    }
    for index in 0..16 {
        values.push(format!(
            "long value #{index:02}\0{}",
            "境".repeat(512 + index * 96)
        ));
    }
    values
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    assert_eq!(
        arguments.len(),
        4,
        "provide LEGACY_FIXTURE GENERATED_FIXTURE ITERATIONS ROUNDS"
    );
    let iterations = arguments[2].to_str().unwrap().parse::<usize>()?;
    let rounds = arguments[3].to_str().unwrap().parse::<usize>()?;
    assert!(iterations > 0 && rounds > 0);

    let legacy = fs::read(&arguments[0])?;
    let legacy_table = Table::open(&legacy)?;
    let values = mixed_v2_values();
    let mixed = build(
        &values,
        &BuildOptions {
            max_value_bytes: MIXED_MAX_VALUE_BYTES,
            ..BuildOptions::default()
        },
    )?;
    let mixed_table = Table::open_with_options(
        &mixed.bytes,
        &OpenOptions {
            limits: Limits {
                max_value_bytes: MIXED_MAX_VALUE_BYTES,
                ..Limits::default()
            },
            ..OpenOptions::default()
        },
    )?;
    fs::write(&arguments[1], &mixed.bytes)?;
    println!("{{");
    println!("  \"iterations_per_batch\": {iterations},");
    println!("  \"measured_rounds\": {rounds},");
    println!("  \"unreported_warmup_rounds\": 1,");
    println!("  \"sampling_seed_hex\": \"{SEED:016x}\",");
    println!("  \"datasets\": [");
    benchmark_dataset("ssa_2024_v1", &legacy_table, None, iterations, rounds)?;
    println!(",");
    benchmark_dataset(
        "synthetic_mixed_utf8_v2",
        &mixed_table,
        Some(&values),
        iterations,
        rounds,
    )?;
    println!();
    println!("  ]");
    println!("}}");
    Ok(())
}
