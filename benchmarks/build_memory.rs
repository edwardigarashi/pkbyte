//! Count build allocation requests in a process separate from the timing runs.

mod allocations;
mod codec_workloads;

use allocations::{track, verify_tracking};
use codec_workloads::{build_options, load_workloads, MODES};
use std::error::Error;
use std::fs;
use std::hint::black_box;
use std::path::Path;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    assert_eq!(
        args.len(),
        4,
        "provide SSA_INPUT BENCHMARK_OUTPUT BUILD_ROUNDS WORKLOADS"
    );
    let benchmark_output = Path::new(&args[1]);
    let rounds = args[2].to_str().unwrap().parse::<usize>()?;
    assert!(rounds > 0);
    let extended = match args[3].to_str() {
        Some("standard") => false,
        Some("extended") => true,
        _ => return Err("workloads must be standard or extended".into()),
    };
    verify_tracking();

    let workloads = load_workloads(Path::new(&args[0]), extended)?;
    println!("{{");
    for (index, (dataset, values)) in workloads.iter().enumerate() {
        let separator = if index == 0 { "" } else { "," };
        println!("{separator}  \"{dataset}\": {{");
        for (index, (label, mode)) in MODES.iter().enumerate() {
            let options = build_options(*mode);
            let expected = fs::read(benchmark_output.join(format!("{dataset}_{label}.npk")))?;
            let mut calls = Vec::with_capacity(rounds);
            let mut requested = Vec::with_capacity(rounds);
            let mut peaks = Vec::with_capacity(rounds);
            let mut live_at_return = Vec::with_capacity(rounds);
            let mut retained = Vec::with_capacity(rounds);
            for _ in 0..rounds {
                let (packed, allocation) =
                    track(|| black_box(pkbyte::build(black_box(values), black_box(&options))));
                let packed = packed?;
                assert!(
                    packed.bytes == expected,
                    "tracked build diverged for {dataset}/{label}"
                );
                assert_eq!(
                    allocation.live_bytes,
                    packed.bytes.capacity() as u64,
                    "live heap at build return must equal returned buffer capacity for {dataset}/{label}"
                );
                calls.push(allocation.calls);
                requested.push(allocation.bytes);
                peaks.push(allocation.peak_live_bytes);
                live_at_return.push(allocation.live_bytes);
                retained.push(packed.bytes.capacity());
            }
            let comma = if index + 1 == MODES.len() { "" } else { "," };
            println!(
                "    \"{label}\": {{\"allocation_calls\": {calls:?}, \"requested_bytes\": {requested:?}, \"peak_live_bytes\": {peaks:?}, \"live_bytes_at_return\": {live_at_return:?}, \"returned_table_capacity_bytes\": {retained:?}}}{comma}"
            );
        }
        print!("  }}");
    }
    println!("\n}}");
    Ok(())
}
