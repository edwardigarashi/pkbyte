//! Compare serialized codec choices and their prepared readers.
#![forbid(unsafe_code)]

use pkbyte::{
    build, BuildMode, BuildOutput, Limits, OpenOptions, PrepareOptions, PreparedTable, Table,
};
use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::hint::black_box;
use std::mem::{size_of, size_of_val};
use std::path::Path;
use std::time::Instant;

mod codec_workloads;
use codec_workloads::{build_options, load_workloads, next_word, MAX_VALUE_BYTES, MODES, SEED};

const WARM_PHASES: [&str; 2] = ["matched_prepared_retrieval", "local_prepared_sampling"];
const OPEN_PHASES: [&str; 4] = [
    "structural_open",
    "normal_open",
    "full_validation",
    "prepare",
];
const HASH_START: u64 = 0xcbf2_9ce4_8422_2325;
const HASH_MULTIPLIER: u64 = 0x100_0000_01b3;

#[derive(Clone, Copy)]
struct Settings {
    iterations: usize,
    rounds: usize,
    build_rounds: usize,
}

struct BuiltModes {
    outputs: Vec<BuildOutput>,
    elapsed: [Vec<u128>; MODES.len()],
}

fn rank_stream(bound: usize, length: usize) -> Vec<usize> {
    let mut state = SEED;
    let bound = bound as u64;
    let threshold = bound.wrapping_neg() % bound;
    (0..length)
        .map(|_| loop {
            let product = u128::from(next_word(&mut state)) * u128::from(bound);
            if product as u64 >= threshold {
                break (product >> 64) as usize;
            }
        })
        .collect()
}

fn consume(checksum: u64, value: &str) -> u64 {
    let checksum = (checksum ^ value.len() as u64).wrapping_mul(HASH_MULTIPLIER);
    value.bytes().fold(checksum, |state, byte| {
        (state ^ u64::from(byte)).wrapping_mul(HASH_MULTIPLIER)
    })
}

fn summarize<'a>(values: impl IntoIterator<Item = &'a str>) -> (u64, usize) {
    values
        .into_iter()
        .fold((HASH_START, 0), |(hash, bytes), value| {
            (consume(hash, value), bytes + value.len())
        })
}

fn timed<T>(run: impl FnOnce() -> T) -> (u128, T) {
    let start = Instant::now();
    let result = run();
    let elapsed = start.elapsed().as_nanos();
    black_box(&result);
    (elapsed, result)
}

fn median(samples: &[u128]) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let middle = sorted.len() / 2;
    if middle * 2 == sorted.len() {
        (sorted[middle - 1] as f64 + sorted[middle] as f64) / 2.0
    } else {
        sorted[middle] as f64
    }
}

fn build_modes(values: &[String], rounds: usize) -> Result<BuiltModes, pkbyte::Error> {
    let mut samples = std::array::from_fn(|_| Vec::with_capacity(rounds));
    let mut outputs = Vec::new();
    for round in 0..=rounds {
        for offset in 0..MODES.len() {
            let index = (offset + round) % MODES.len();
            let options = build_options(MODES[index].1);
            let (elapsed, packed) = timed(|| build(black_box(values), black_box(&options)));
            let packed = packed?;
            if round == 0 {
                outputs.push(packed);
            } else {
                assert_eq!(
                    packed.bytes, outputs[index].bytes,
                    "nondeterministic {}",
                    MODES[index].0
                );
                samples[index].push(elapsed);
            }
        }
    }
    assert_eq!(
        outputs.last().unwrap().bytes.len(),
        outputs.iter().map(|pack| pack.bytes.len()).min().unwrap()
    );
    Ok(BuiltModes {
        outputs,
        elapsed: samples,
    })
}

struct Trace {
    retrieval_ranks: Vec<u64>,
    sample_summary: (u64, usize),
}

fn match_values(
    table: &Table<'_>,
    prepared: &PreparedTable<'_>,
    values: &[String],
    by_value: &BTreeMap<&str, usize>,
    source_trace: &[usize],
) -> Result<Trace, pkbyte::Error> {
    assert_eq!(table.len() as usize, values.len());
    let mut source_by_rank = Vec::with_capacity(values.len());
    let mut rank_by_source = vec![u64::MAX; values.len()];
    let mut checked_output = Vec::with_capacity(table.max_value_bytes());
    let mut prepared_output = prepared.new_output_buffer()?;
    for rank in 0..table.len() {
        let value = table.get_into(rank, &mut checked_output)?;
        assert_eq!(prepared.get_into(rank, &mut prepared_output)?, value);
        let source_index = by_value[value];
        assert_eq!(rank_by_source[source_index], u64::MAX);
        rank_by_source[source_index] = rank;
        source_by_rank.push(source_index);
    }
    Ok(Trace {
        retrieval_ranks: source_trace
            .iter()
            .map(|&index| rank_by_source[index])
            .collect(),
        sample_summary: summarize(
            source_trace
                .iter()
                .map(|&rank| values[source_by_rank[rank]].as_str()),
        ),
    })
}

struct ReadPath<'a, 'file> {
    checked: &'a Table<'file>,
    prepared: PreparedTable<'a>,
    trace: Trace,
    scratch: Vec<u8>,
}

fn warm_read(
    path: &mut ReadPath<'_, '_>,
    phase: usize,
    expected_retrieval: u64,
) -> Result<u128, pkbyte::Error> {
    let mut state = black_box(SEED);
    let mut next = || next_word(&mut state);
    let reader = &path.prepared;
    let scratch = &mut path.scratch;
    let (elapsed, checksum) = match phase {
        0 => timed(|| {
            path.trace
                .retrieval_ranks
                .iter()
                .try_fold(HASH_START, |hash, &rank| {
                    reader
                        .get_into(rank, scratch)
                        .map(|value| consume(hash, value))
                })
        }),
        _ => timed(|| {
            (0..path.trace.retrieval_ranks.len()).try_fold(HASH_START, |hash, _| {
                reader
                    .sample_into(&mut next, scratch)
                    .map(|value| consume(hash, value))
            })
        }),
    };
    let expected = if phase == 0 {
        expected_retrieval
    } else {
        path.trace.sample_summary.0
    };
    assert_eq!(checksum?, expected);
    Ok(elapsed)
}

fn vector_lookup(values: &[String], ranks: &[usize], scratch: &mut [u8]) -> (u128, u64) {
    timed(|| {
        ranks.iter().fold(HASH_START, |hash, &rank| {
            let source = values[rank].as_bytes();
            let output = &mut scratch[..source.len()];
            output.copy_from_slice(source);
            consume(hash, std::str::from_utf8(output).unwrap())
        })
    })
}

fn setup_time(
    bytes: &[u8],
    table: &Table<'_>,
    structural: &OpenOptions,
    checked: &OpenOptions,
    prepared: &PrepareOptions,
    phase: usize,
) -> Result<u128, pkbyte::Error> {
    Ok(match phase {
        0 => {
            let (time, result) = timed(|| Table::open_with_options(black_box(bytes), structural));
            assert_eq!(result?.len(), table.len());
            time
        }
        1 => {
            let (time, result) = timed(|| Table::open_with_options(black_box(bytes), checked));
            assert_eq!(result?.len(), table.len());
            time
        }
        2 => {
            let (time, result) = timed(|| black_box(table).validate_all(true));
            assert_eq!(result?.row_count, table.len());
            time
        }
        _ => {
            let (time, result) = timed(|| black_box(table).prepare(prepared));
            assert_eq!(result?.len(), table.len());
            time
        }
    })
}

fn print_samples(name: &str, timings: &[u128], iterations: Option<usize>, comma: &str) {
    let (metric, scale) = match iterations {
        Some(iterations) => ("median_ns_per_value", iterations as f64),
        None => ("median_ns", 1.0),
    };
    println!(
        "            \"{name}\": {{\"{metric}\": {:.3}, \"elapsed_ns\": {timings:?}}}{comma}",
        median(timings) / scale,
    );
}

fn benchmark(
    name: &str,
    values: &Vec<String>,
    output: &Path,
    settings: Settings,
) -> Result<(), Box<dyn Error>> {
    let by_value: BTreeMap<_, _> = values
        .iter()
        .enumerate()
        .map(|(i, value)| (value.as_str(), i))
        .collect();
    assert_eq!(values.len(), by_value.len());
    let source_trace = rank_stream(values.len(), settings.iterations);
    let matched_summary = summarize(source_trace.iter().map(|&index| values[index].as_str()));
    let BuiltModes {
        outputs: built,
        elapsed: build_samples,
    } = build_modes(values, settings.build_rounds)?;
    let checked = OpenOptions {
        limits: Limits {
            max_value_bytes: MAX_VALUE_BYTES,
            ..Limits::default()
        },
        ..OpenOptions::default()
    };
    let structural = OpenOptions {
        verify_checksum: false,
        ..checked.clone()
    };
    let prep_options = PrepareOptions::default();
    let tables = built
        .iter()
        .map(|pack| Table::open_with_options(&pack.bytes, &checked))
        .collect::<Result<Vec<_>, _>>()?;
    let mut paths = Vec::new();
    for (index, table) in tables.iter().enumerate() {
        let stats = table.stats();
        assert_eq!(
            stats.raw_rows
                + stats.compressed_rows
                + stats.ef_mixed_rows
                + stats.ef_power_of_two_rows,
            table.len()
        );
        if name == "high_entropy"
            && matches!(
                MODES[index].1,
                BuildMode::EliasFanoMixed | BuildMode::EliasFanoPowerOfTwo
            )
        {
            assert_eq!(
                stats.raw_rows,
                table.len(),
                "ineligible EF values must fall back"
            );
        }
        fs::write(
            output.join(format!("{name}_{}.npk", MODES[index].0)),
            &built[index].bytes,
        )?;
        let prepared = table.prepare(&prep_options)?;
        let trace = match_values(table, &prepared, values, &by_value, &source_trace)?;
        let scratch = prepared.new_output_buffer()?;
        paths.push(ReadPath {
            checked: table,
            prepared,
            trace,
            scratch,
        });
    }

    let mut setup_samples: [[Vec<u128>; OPEN_PHASES.len()]; MODES.len()] =
        std::array::from_fn(|_| std::array::from_fn(|_| Vec::with_capacity(settings.rounds)));
    let mut warm_samples: [[Vec<u128>; WARM_PHASES.len()]; MODES.len()] =
        std::array::from_fn(|_| std::array::from_fn(|_| Vec::with_capacity(settings.rounds)));
    let mut vector_samples = Vec::with_capacity(settings.rounds);
    let mut vector_scratch = vec![0; values.iter().map(String::len).max().unwrap()];
    let paths_count = MODES.len() + 1;
    for round in 0..=settings.rounds {
        for offset in 0..paths_count {
            let index = (round + offset) % paths_count;
            if index == MODES.len() {
                let (elapsed, checksum) = vector_lookup(values, &source_trace, &mut vector_scratch);
                assert_eq!(checksum, matched_summary.0);
                if round != 0 {
                    vector_samples.push(elapsed);
                }
                continue;
            }
            for (phase, samples) in setup_samples[index].iter_mut().enumerate() {
                let time = setup_time(
                    &built[index].bytes,
                    &tables[index],
                    &structural,
                    &checked,
                    &prep_options,
                    phase,
                )?;
                if round != 0 {
                    samples.push(time);
                }
            }
            for (phase, samples) in warm_samples[index].iter_mut().enumerate() {
                let time = warm_read(&mut paths[index], phase, matched_summary.0)?;
                if round != 0 {
                    samples.push(time);
                }
            }
        }
    }

    println!("    {{\"dataset\": \"{name}\",");
    println!("      \"rows\": {},", values.len());
    println!(
        "      \"matched_trace_checksum_hex\": \"{:016x}\",",
        matched_summary.0
    );
    println!(
        "      \"matched_trace_output_bytes\": {},",
        matched_summary.1
    );
    println!("      \"loaded_vector\": {{");
    println!("        \"inline_bytes\": {},", size_of_val(values));
    println!(
        "        \"retained_heap_bytes\": {},",
        values.capacity() * size_of::<String>()
            + values.iter().map(String::capacity).sum::<usize>()
    );
    println!("        \"scratch_bytes\": {},", vector_scratch.capacity());
    print_samples(
        "matched_retrieval",
        &vector_samples,
        Some(settings.iterations),
        "",
    );
    println!("      }},");
    println!("      \"modes\": [");
    for (index, path) in paths.iter().enumerate() {
        let mode = MODES[index].0;
        let table = path.checked;
        let prepared = &path.prepared;
        let stats = table.stats();
        println!("        {{\"mode\": \"{mode}\",");
        println!("          \"format_version\": {},", table.version());
        println!("          \"file_bytes\": {},", stats.file_bytes);
        println!("          \"decoded_bytes\": {},", stats.raw_value_bytes);
        println!(
            "          \"dictionary_bytes\": {},",
            stats.dictionary_bytes
        );
        println!("          \"elias_fano_bytes\": {},", stats.ef_bytes);
        println!("          \"raw_rows\": {},", stats.raw_rows);
        println!("          \"phrase_rows\": {},", stats.compressed_rows);
        println!("          \"ef_mixed_rows\": {},", stats.ef_mixed_rows);
        println!(
            "          \"ef_power_of_two_rows\": {},",
            stats.ef_power_of_two_rows
        );
        println!("          \"groups\": {},", stats.groups);
        println!(
            "          \"checked_inline_bytes\": {},",
            size_of_val(table)
        );
        println!(
            "          \"checked_metadata_heap_bytes\": {},",
            table.metadata_heap_bytes()
        );
        println!(
            "          \"prepared_inline_bytes\": {},",
            size_of_val(prepared)
        );
        println!(
            "          \"prepared_additional_heap_bytes\": {},",
            prepared.additional_heap_bytes()
        );
        println!(
            "          \"prepared_hint_bytes\": {},",
            prepared.hint_bytes()
        );
        println!(
            "          \"checked_scratch_capacity_bytes\": {},",
            table.max_value_bytes()
        );
        println!(
            "          \"prepared_scratch_required_bytes\": {},",
            prepared.output_buffer_len()
        );
        println!(
            "          \"local_trace_checksum_hex\": \"{:016x}\",",
            path.trace.sample_summary.0
        );
        println!(
            "          \"local_trace_output_bytes\": {},",
            path.trace.sample_summary.1
        );
        println!("          \"setup\": {{");
        print_samples("build", &build_samples[index], None, ",");
        for (phase, &label) in OPEN_PHASES.iter().enumerate() {
            let comma = if phase + 1 == OPEN_PHASES.len() {
                ""
            } else {
                ","
            };
            print_samples(label, &setup_samples[index][phase], None, comma);
        }
        println!("          }},");
        println!("          \"warm_ns_per_value\": {{");
        for (phase, &label) in WARM_PHASES.iter().enumerate() {
            let comma = if phase + 1 == WARM_PHASES.len() {
                ""
            } else {
                ","
            };
            print_samples(
                label,
                &warm_samples[index][phase],
                Some(settings.iterations),
                comma,
            );
        }
        println!("          }}");
        let comma = if index + 1 == MODES.len() { "" } else { "," };
        println!("        }}{comma}");
    }
    println!("      ]");
    print!("    }}");
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    assert_eq!(
        args.len(),
        6,
        "provide SSA_INPUT OUTPUT_DIR ITERATIONS ROUNDS BUILD_ROUNDS WORKLOADS"
    );
    let settings = Settings {
        iterations: args[2].to_str().unwrap().parse()?,
        rounds: args[3].to_str().unwrap().parse()?,
        build_rounds: args[4].to_str().unwrap().parse()?,
    };
    assert!(settings.iterations > 0 && settings.rounds > 0 && settings.build_rounds > 0);
    let output = Path::new(&args[1]);
    let extended = match args[5].to_str() {
        Some("standard") => false,
        Some("extended") => true,
        _ => return Err("workloads must be standard or extended".into()),
    };
    let workloads = load_workloads(Path::new(&args[0]), extended)?;
    println!("{{");
    println!("  \"iterations_per_warm_batch\": {},", settings.iterations);
    println!("  \"measured_rounds\": {},", settings.rounds);
    println!("  \"measured_build_rounds\": {},", settings.build_rounds);
    println!("  \"unreported_warmup_rounds\": 1,");
    println!("  \"value_byte_limit\": {MAX_VALUE_BYTES},");
    println!("  \"sampling_seed_hex\": \"{SEED:016x}\",");
    println!("  \"datasets\": [");
    for (index, (name, values)) in workloads.iter().enumerate() {
        if *name == "multilingual_and_long" {
            let mut limits = build_options(BuildMode::Compact);
            limits.max_value_bytes = 4_096;
            assert!(matches!(
                build(values, &limits),
                Err(pkbyte::Error::LimitExceeded(_))
            ));
        }
        if index != 0 {
            println!(",");
        }
        benchmark(name, values, output, settings)?;
    }
    println!();
    println!("  ]");
    println!("}}");
    Ok(())
}
