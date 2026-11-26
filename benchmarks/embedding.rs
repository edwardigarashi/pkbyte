//! Measure a build-generated table embedded as immutable bytes.

use pkbyte::{PrepareOptions, PreparedTable, Table};
use std::error::Error;
use std::hint::black_box;
use std::time::Instant;

mod allocations;
use allocations::{track, verify_tracking, Requests};

mod generated {
    include!(concat!(env!("OUT_DIR"), "/pkbyte_table.rs"));
}

const SEED: u64 = 0x5a17_e6d9_b043_2c81;
const HASH_START: u64 = 0xcbf2_9ce4_8422_2325;
const HASH_MULTIPLIER: u64 = 0x100_0000_01b3;

fn timed<T>(run: impl FnOnce() -> T) -> (T, u128) {
    let start = Instant::now();
    let result = black_box(run());
    (result, start.elapsed().as_nanos())
}

fn next_word(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut word = *state;
    word = (word ^ (word >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    word = (word ^ (word >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    word ^ (word >> 31)
}

fn consume(checksum: u64, value: &str) -> u64 {
    let checksum = (checksum ^ value.len() as u64).wrapping_mul(HASH_MULTIPLIER);
    value.bytes().fold(checksum, |state, byte| {
        (state ^ u64::from(byte)).wrapping_mul(HASH_MULTIPLIER)
    })
}

fn checked_checksum(table: &Table<'_>, iterations: usize) -> u64 {
    let bound = table.len();
    let threshold = bound.wrapping_neg() % bound;
    let mut state = SEED;
    let mut scratch = Vec::with_capacity(table.max_value_bytes());
    (0..iterations).fold(HASH_START, |checksum, _| {
        let rank = loop {
            let product = u128::from(next_word(&mut state)) * u128::from(bound);
            if product as u64 >= threshold {
                break (product >> 64) as u64;
            }
        };
        consume(checksum, table.get_into(rank, &mut scratch).unwrap())
    })
}

fn sample_checksum(
    prepared: &PreparedTable<'_>,
    scratch: &mut [u8],
    next_u64: &mut impl FnMut() -> u64,
    iterations: usize,
) -> u64 {
    (0..iterations).fold(HASH_START, |checksum, _| {
        consume(checksum, prepared.sample_into(next_u64, scratch).unwrap())
    })
}

fn median(samples: &[u128]) -> f64 {
    let mut samples = samples.to_vec();
    samples.sort_unstable();
    let middle = samples.len() / 2;
    if middle * 2 == samples.len() {
        (samples[middle - 1] as f64 + samples[middle] as f64) / 2.0
    } else {
        samples[middle] as f64
    }
}

fn write_stage(name: &str, times: &[u128], allocations: Requests) {
    println!(
        "    \"{name}\": {{\"median_ns\": {:.3}, \"elapsed_ns\": {times:?}, \"allocation_calls\": {}, \"allocation_requested_bytes\": {}, \"allocation_peak_live_bytes\": {}, \"allocation_live_bytes_at_return\": {}}},",
        median(times),
        allocations.calls,
        allocations.bytes,
        allocations.peak_live_bytes,
        allocations.live_bytes,
    );
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let iterations: usize = args.next().ok_or("missing iterations")?.parse()?;
    let rounds: usize = args.next().ok_or("missing rounds")?.parse()?;
    assert!(iterations > 0 && rounds > 0 && args.next().is_none());
    verify_tracking();

    let bytes = black_box(generated::TABLE_BYTES);
    let options = PrepareOptions::default();
    let mut open_times = Vec::with_capacity(rounds);
    let mut prepare_times = Vec::with_capacity(rounds);
    let mut scratch_times = Vec::with_capacity(rounds);
    let mut init_times = Vec::with_capacity(rounds);
    for round in 0..=rounds {
        let (table, open) = timed(|| Table::open_structural(bytes));
        let table = table?;
        let (prepared, preparation) = timed(|| table.prepare(&options));
        let prepared = prepared?;
        let (scratch, initialization) = timed(|| prepared.new_output_buffer());
        black_box(scratch?);
        if round != 0 {
            open_times.push(open);
            prepare_times.push(preparation);
            scratch_times.push(initialization);
            init_times.push(open + preparation + initialization);
        }
    }

    let (table, open_allocations) = track(|| Table::open_structural(bytes));
    let table = table?;
    let (prepared, prep_allocations) = track(|| table.prepare(&options));
    let prepared = prepared?;
    let (scratch, scratch_allocations) = track(|| prepared.new_output_buffer());
    let mut scratch = scratch?;
    assert_eq!(
        open_allocations.live_bytes,
        table.metadata_heap_bytes() as u64
    );
    assert_eq!(
        prep_allocations.live_bytes,
        prepared.additional_heap_bytes() as u64
    );
    assert_eq!(scratch_allocations.live_bytes, scratch.capacity() as u64);
    assert!(!table.is_empty());
    let expected = checked_checksum(&table, iterations);
    let mut warm_times = Vec::with_capacity(rounds);
    for round in 0..=rounds {
        let mut state = black_box(SEED);
        let mut random = || next_word(&mut state);
        let (checksum, elapsed) =
            timed(|| sample_checksum(&prepared, &mut scratch, &mut random, iterations));
        assert_eq!(checksum, expected);
        if round != 0 {
            warm_times.push(elapsed);
        }
    }
    let mut state = black_box(SEED);
    let (checksum, warm_allocations) = track(|| {
        sample_checksum(
            &prepared,
            &mut scratch,
            &mut || next_word(&mut state),
            iterations,
        )
    });
    assert_eq!(checksum, expected);
    assert_eq!(warm_allocations.calls, 0);
    assert_eq!(warm_allocations.bytes, 0);
    assert_eq!(warm_allocations.peak_live_bytes, 0);

    let stats = table.stats();
    println!("{{");
    println!(
        "  \"table\": {{\"version\": {}, \"values\": {}, \"decoded_bytes\": {}, \"portable_bytes\": {}, \"payload_bytes\": {}, \"container_overhead_bytes\": {}, \"dictionary_bytes\": {}, \"groups\": {}, \"raw_rows\": {}, \"phrase_rows\": {}, \"ef_mixed_rows\": {}, \"ef_power_of_two_rows\": {}, \"ef_block_bytes\": {}}},",
        table.version(), table.len(), stats.raw_value_bytes, bytes.len(), stats.payload_bytes,
        bytes.len() - stats.payload_bytes, stats.dictionary_bytes, stats.groups,
        stats.raw_rows, stats.compressed_rows, stats.ef_mixed_rows, stats.ef_power_of_two_rows,
        stats.ef_bytes,
    );
    println!(
        "  \"retained\": {{\"checked_metadata_heap_bytes\": {}, \"prepared_additional_heap_bytes\": {}, \"prepared_hint_bytes\": {}, \"scratch_initialized_bytes\": {}, \"scratch_heap_capacity_bytes\": {}}},",
        table.metadata_heap_bytes(), prepared.additional_heap_bytes(), prepared.hint_bytes(),
        scratch.len(), scratch.capacity(),
    );
    println!("  \"configuration\": {{\"rounds\": {rounds}, \"iterations_per_batch\": {iterations}, \"unreported_warmup_batches\": 1, \"seed_hex\": \"{SEED:016x}\", \"rng\": \"SplitMix64\", \"rank_reduction\": \"multiply high\", \"max_hint_bytes\": {}}},", options.max_hint_bytes);
    println!("  \"startup\": {{");
    write_stage("open_structural", &open_times, open_allocations);
    write_stage(
        "prepare_with_crc_and_uniqueness",
        &prepare_times,
        prep_allocations,
    );
    write_stage("initialize_scratch", &scratch_times, scratch_allocations);
    println!(
        "    \"total\": {{\"median_ns\": {:.3}, \"elapsed_ns\": {init_times:?}}}",
        median(&init_times)
    );
    println!("  }},");
    println!(
        "  \"warm_prepared_sampling\": {{\"median_ns_per_sample\": {:.3}, \"elapsed_ns\": {warm_times:?}, \"checksum_hex\": \"{checksum:016x}\", \"allocation_calls\": {}, \"allocation_requested_bytes\": {}, \"allocation_peak_live_bytes\": {}}}",
        median(&warm_times) / iterations as f64, warm_allocations.calls, warm_allocations.bytes, warm_allocations.peak_live_bytes,
    );
    println!("}}");
    Ok(())
}
