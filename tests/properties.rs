use pkbyte::{
    build, BuildMode, BuildOptions, DuplicatePolicy, Error, Limits, OpenOptions, PrepareOptions,
    SamplingMethod, Table,
};
use std::collections::BTreeSet;
use std::sync::Barrier;
use std::thread;

const MODES: [BuildMode; 5] = [
    BuildMode::Raw,
    BuildMode::Phrase,
    BuildMode::EliasFanoMixed,
    BuildMode::EliasFanoPowerOfTwo,
    BuildMode::Compact,
];

#[derive(Clone)]
struct TestRng(u64);

impl TestRng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn index(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

fn low_entropy_values() -> impl Iterator<Item = String> {
    (0..128).map(|index| format!("aaaaaaaaaaaaaaaa{index:07b}bbbbbbbbbbbbbbbb"))
}

fn input_for_seed(seed: u64) -> Vec<String> {
    match seed {
        0 => return Vec::new(),
        1 => return vec![String::new()],
        2 => return vec![String::new(), String::new()],
        3 => return vec![String::from("👩🏽‍💻")],
        _ => {}
    }
    let mut rng = TestRng(seed);
    let controls = [
        "",
        "prefix",
        "prefix\0",
        "prefix suffix",
        "\n\t\r\0",
        " \t ",
        "É",
        "é",
        "e\u{301}",
        "東京",
        "👩🏽‍💻",
        "\u{feff}",
    ];
    let mut unique: BTreeSet<String> = controls.into_iter().map(str::to_owned).collect();
    let attempts = 20 + rng.index(36);
    for _ in 0..attempts {
        let length = 1 + rng.index(36);
        let value = match rng.index(3) {
            0 => {
                // Entire ASCII domain, including embedded controls.
                (0..length)
                    .map(|_| char::from(rng.index(128) as u8))
                    .collect()
            }
            1 => {
                // Full Unicode scalar range. Surrogate code points are excluded.
                std::iter::repeat_with(|| rng.index(0x11_0000) as u32)
                    .filter_map(char::from_u32)
                    .take(length)
                    .collect()
            }
            _ => {
                let mut value = String::from("prefix");
                for _ in 0..length {
                    value.push_str(controls[rng.index(controls.len())]);
                }
                value
            }
        };
        unique.insert(value);
    }
    if seed % 3 == 0 {
        unique.extend(low_entropy_values());
    }
    if seed % 4 == 0 {
        unique.insert("世".repeat(1_366 + rng.index(9)));
    }
    let mut values: Vec<_> = unique.into_iter().collect();
    if seed % 2 == 1 {
        let duplicate = values[1 + rng.index(values.len() - 1)].clone();
        values.extend([String::new(), duplicate]);
    }
    permute(&mut values, &mut rng);
    values
}

fn permute(values: &mut [String], rng: &mut TestRng) {
    for end in (1..values.len()).rev() {
        values.swap(end, rng.index(end + 1));
    }
}

fn rank_from_draws(bound: usize, method: SamplingMethod, mut draw: impl FnMut() -> u64) -> usize {
    let n = bound as u128;
    let domain = u128::from(u64::MAX) + 1;
    let excess = domain % n;
    loop {
        let value = u128::from(draw());
        match method {
            SamplingMethod::Modulo if value >= excess => return (value % n) as usize,
            SamplingMethod::MultiplyHigh => {
                let product = value * n;
                if product % domain >= excess {
                    return (product / domain) as usize;
                }
            }
            SamplingMethod::Modulo => {}
        }
    }
}

fn assert_source_limits(input: &[String], options: &BuildOptions, context: &str) {
    for (resource, exceeded) in [
        (
            "value byte length",
            options
                .max_value_bytes
                .checked_sub(1)
                .map(|maximum| BuildOptions {
                    max_value_bytes: maximum,
                    ..options.clone()
                }),
        ),
        (
            "cumulative byte length",
            options
                .max_input_bytes
                .checked_sub(1)
                .map(|maximum| BuildOptions {
                    max_input_bytes: maximum,
                    ..options.clone()
                }),
        ),
        (
            "row count",
            options
                .max_input_rows
                .checked_sub(1)
                .map(|maximum| BuildOptions {
                    max_input_rows: maximum,
                    ..options.clone()
                }),
        ),
    ] {
        if let Some(exceeded) = exceeded {
            assert!(
                matches!(build(input, &exceeded), Err(Error::LimitExceeded(_))),
                "{context}, {resource}, {:?}",
                options.duplicate_policy
            );
        }
    }
}

#[test]
fn generated_inputs_match_a_set_model_across_encodings_and_limits() {
    for seed in 0..20 {
        let input = input_for_seed(seed);
        let expected: BTreeSet<&str> = input.iter().map(String::as_str).collect();
        let decoded_bytes = expected.iter().map(|value| value.len()).sum::<usize>() as u64;
        let max_value_bytes = expected.iter().map(|value| value.len()).max().unwrap_or(0);
        let duplicate_rows = input.len() - expected.len();
        let mut permuted = input.clone();
        permute(&mut permuted, &mut TestRng(seed ^ 0xab52_3fc8_51a1_523e));

        for mode in MODES {
            let context = format!("seed {seed}, {mode:?}");
            let reject = BuildOptions {
                mode,
                max_input_rows: input.len(),
                max_input_bytes: input.iter().map(String::len).sum(),
                max_value_bytes,
                ..BuildOptions::default()
            };
            assert_source_limits(&input, &reject, &context);
            let deduplicate = BuildOptions {
                duplicate_policy: DuplicatePolicy::Deduplicate,
                ..reject.clone()
            };
            assert_source_limits(&input, &deduplicate, &context);
            let packed = build(&input, &deduplicate).unwrap_or_else(|error| {
                panic!("{context}, exact limits and deduplication: {error}")
            });
            let canonical = build(&expected, &reject)
                .unwrap_or_else(|error| panic!("{context}, distinct model input: {error}"));
            assert!(packed.bytes == canonical.bytes, "{context}");
            let reordered = build(&permuted, &deduplicate).unwrap();
            assert!(packed.bytes == reordered.bytes, "{context}, permuted input");

            match build(&input, &reject) {
                Ok(rejected_duplicates) => {
                    assert_eq!(duplicate_rows, 0, "{context}");
                    assert!(packed.bytes == rejected_duplicates.bytes, "{context}");
                }
                Err(Error::DuplicateValue(value)) => {
                    assert!(duplicate_rows > 0, "{context}");
                    assert!(
                        input.iter().filter(|input| **input == value).count() > 1,
                        "{context}"
                    );
                }
                Err(error) => panic!("{context}, reject-duplicates mode: {error}"),
            }
            assert_eq!(packed.stats.input_rows, input.len(), "{context}");
            assert_eq!(packed.stats.duplicate_rows, duplicate_rows, "{context}");
            assert_eq!(packed.stats.row_count, expected.len() as u64, "{context}");
            assert_eq!(packed.stats.raw_value_bytes, decoded_bytes, "{context}");
            assert_eq!(packed.stats.max_value_bytes, max_value_bytes, "{context}");

            let limits = Limits {
                max_file_bytes: packed.bytes.len(),
                max_rows: expected.len() as u64,
                max_value_bytes,
                max_groups: packed.stats.groups,
                max_decoded_bytes: decoded_bytes,
            };
            let checked = Table::open_with_options(
                &packed.bytes,
                &OpenOptions {
                    verify_checksum: true,
                    limits: limits.clone(),
                },
            )
            .unwrap_or_else(|error| panic!("{context}, checked open: {error}"));
            let structural = Table::open_with_options(
                &packed.bytes,
                &OpenOptions {
                    verify_checksum: false,
                    limits,
                },
            )
            .unwrap_or_else(|error| panic!("{context}, structural open: {error}"));
            let report = checked.validate_all(true).unwrap();
            assert!(report.uniqueness_checked, "{context}");
            assert_eq!(report.row_count, expected.len() as u64, "{context}");
            assert_eq!(report.decoded_bytes, decoded_bytes, "{context}");
            assert_eq!(checked.max_value_bytes(), max_value_bytes, "{context}");

            let stats = checked.stats();
            assert_eq!(
                stats.raw_rows
                    + stats.compressed_rows
                    + stats.ef_mixed_rows
                    + stats.ef_power_of_two_rows,
                report.row_count,
                "{context}"
            );
            let rank_values: Vec<_> = (0..checked.len())
                .map(|rank| {
                    let value = checked.get(rank).unwrap();
                    let raw = checked.get_raw(rank).unwrap();
                    assert_eq!(
                        raw,
                        (rank < stats.raw_rows).then_some(value.as_str()),
                        "{context}, rank {rank}"
                    );
                    value
                })
                .collect();
            let actual: BTreeSet<_> = rank_values.iter().map(String::as_str).collect();
            assert_eq!(actual.len(), rank_values.len(), "{context}");
            assert!(actual == expected, "{context}, exact UTF-8 set");
            assert_sampling(&checked, &structural, &rank_values, seed, &context);
        }
    }
}

fn assert_sampling(
    checked: &Table<'_>,
    structural: &Table<'_>,
    rank_values: &[String],
    seed: u64,
    context: &str,
) {
    let mut checked_output = Vec::with_capacity(checked.max_value_bytes());
    for method in [SamplingMethod::Modulo, SamplingMethod::MultiplyHigh] {
        let prepared = structural
            .prepare(&PrepareOptions {
                sampling: method,
                max_hint_bytes: if seed % 2 == 0 { 0 } else { 4096 },
            })
            .unwrap_or_else(|error| panic!("{context}, {method:?} prepare: {error}"));
        let mut output = prepared.new_output_buffer().unwrap();
        assert_eq!(prepared.len(), rank_values.len() as u64, "{context}");
        if rank_values.is_empty() {
            assert!(matches!(
                prepared.sample_into(&mut || panic!("empty table drew randomness"), &mut output),
                Err(Error::EmptyTable)
            ));
            continue;
        }
        for (rank, expected) in rank_values.iter().enumerate() {
            let value = prepared.get_into(rank as u64, &mut output).unwrap();
            assert_eq!(
                value.as_bytes(),
                expected.as_bytes(),
                "{context}, rank {rank}"
            );
        }

        // The initial values cover extremes and trigger rejection for some
        // bounds. Follow them with a reproducible full-width stream.
        let mut rng = TestRng(seed);
        let mut observed_draws = [0, 1, u64::MAX, 1 << 63]
            .into_iter()
            .chain(std::iter::repeat_with(move || rng.next()));
        let mut model_draws = observed_draws.clone();
        let mut checked_draws = observed_draws.clone();
        for sample in 0..64 {
            let rank = rank_from_draws(rank_values.len(), method, || model_draws.next().unwrap());
            let value = prepared
                .sample_into(&mut || observed_draws.next().unwrap(), &mut output)
                .unwrap();
            assert_eq!(
                value, rank_values[rank],
                "{context}, {method:?}, sample {sample}"
            );
            if method == SamplingMethod::Modulo {
                let value = checked
                    .sample_into(&mut || checked_draws.next().unwrap(), &mut checked_output)
                    .unwrap();
                assert_eq!(
                    value, rank_values[rank],
                    "{context}, checked sample {sample}"
                );
            }
        }
        let next = model_draws.next();
        assert_eq!(observed_draws.next(), next, "{context}, {method:?} source");
        if method == SamplingMethod::Modulo {
            assert_eq!(checked_draws.next(), next, "{context}, checked source");
        }
    }
}

#[test]
fn threads_share_prepared_tables_with_independent_random_sources_and_output() {
    let mut values: Vec<_> = low_entropy_values().collect();
    values.extend(["", "東京都", "a\0\n"].map(str::to_owned));
    for mode in MODES {
        let packed = build(
            &values,
            &BuildOptions {
                mode,
                ..BuildOptions::default()
            },
        )
        .unwrap();
        let table = Table::open(&packed.bytes).unwrap();
        let stats = table.stats();
        match mode {
            BuildMode::Raw => assert_eq!(stats.raw_rows, table.len()),
            BuildMode::Phrase => assert!(stats.compressed_rows > 0),
            BuildMode::EliasFanoMixed => assert!(stats.ef_mixed_rows > 0),
            BuildMode::EliasFanoPowerOfTwo => assert!(stats.ef_power_of_two_rows > 0),
            BuildMode::Compact => {}
        }
        let rank_values: Vec<_> = (0..table.len())
            .map(|rank| table.get(rank).unwrap())
            .collect();
        let prepared = table.prepare(&PrepareOptions::default()).unwrap();
        let ready = Barrier::new(4);
        thread::scope(|scope| {
            for worker in 0..4 {
                let (ready, prepared, rank_values) = (&ready, &prepared, &rank_values);
                scope.spawn(move || {
                    ready.wait();
                    let mut output = prepared.new_output_buffer().unwrap();
                    let mut rng = TestRng(0xbe50_9237_6f4c_1d6a ^ worker as u64);
                    let mut reference_rng = rng.clone();
                    for index in 0..512 {
                        let rank = rank_from_draws(
                            rank_values.len(),
                            SamplingMethod::MultiplyHigh,
                            || reference_rng.next(),
                        );
                        let sampled = prepared
                            .sample_into(&mut || rng.next(), &mut output)
                            .unwrap();
                        assert_eq!(
                            sampled, rank_values[rank],
                            "{mode:?}, worker {worker}, sample {index}"
                        );
                        let rank = (worker + index) % rank_values.len();
                        let read = prepared.get_into(rank as u64, &mut output).unwrap();
                        assert_eq!(
                            read, rank_values[rank],
                            "{mode:?}, worker {worker}, rank {rank}"
                        );
                    }
                });
            }
        });
    }
}
