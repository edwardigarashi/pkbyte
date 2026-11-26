//! Compare matching seeded pick traces with and without result ownership.

#[cfg(picker_api)]
use pkbyte::Picker;
use pkbyte::{build, BuildOptions, PrepareOptions, SamplingMethod, Table};
use std::hint::black_box;
use std::time::Instant;

const SEED: u64 = 42;

fn split_mix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn batch(iterations: u32, mut pick: impl FnMut(u64) -> u64) -> (u128, u64) {
    let start = Instant::now();
    let mut checksum = 0u64;
    for _ in 0..iterations {
        checksum = pick(checksum);
    }
    (start.elapsed().as_nanos(), black_box(checksum))
}

fn consume(checksum: u64, value: &str) -> u64 {
    black_box(value.as_bytes())
        .iter()
        .fold(checksum ^ value.len() as u64, |hash, &byte| {
            hash.wrapping_mul(0x100_0000_01b3) ^ u64::from(byte)
        })
}

#[derive(Clone, Copy, Debug)]
enum Access {
    Prepared,
    #[cfg(picker_api)]
    PickerReuse,
    #[cfg(picker_api)]
    PickerOwned,
}

impl Access {
    fn measure(self, bytes: &[u8], iterations: u32) -> (u128, u128, u64) {
        match self {
            Self::Prepared => {
                let start = Instant::now();
                let table = Table::open_structural(black_box(bytes)).unwrap();
                let prepared = table
                    .prepare(&PrepareOptions {
                        sampling: SamplingMethod::Modulo,
                        ..PrepareOptions::default()
                    })
                    .unwrap();
                let setup = start.elapsed().as_nanos();
                let mut output = prepared.new_output_buffer().unwrap();
                let mut state = SEED;
                let (elapsed, checksum) = batch(iterations, |checksum| {
                    let value = prepared
                        .sample_into(&mut || split_mix(&mut state), &mut output)
                        .unwrap();
                    consume(checksum, value)
                });
                (setup, elapsed, checksum)
            }
            #[cfg(picker_api)]
            Self::PickerReuse | Self::PickerOwned => {
                let owned = bytes.to_vec();
                let start = Instant::now();
                let mut picker = Picker::from_bytes(black_box(owned)).unwrap();
                let setup = start.elapsed().as_nanos();
                let mut output = Vec::new();
                if matches!(self, Self::PickerReuse) {
                    black_box(picker.pick_into(&mut output).unwrap());
                } else {
                    black_box(picker.pick().unwrap());
                }
                picker.set_seed(SEED);
                let (elapsed, checksum) = if matches!(self, Self::PickerReuse) {
                    batch(iterations, |checksum| {
                        consume(checksum, picker.pick_into(&mut output).unwrap())
                    })
                } else {
                    batch(iterations, |checksum| {
                        consume(checksum, &picker.pick().unwrap())
                    })
                };
                (setup, elapsed, checksum)
            }
        }
    }
}

fn main() {
    let arguments: Vec<_> = std::env::args().collect();
    assert_eq!(
        arguments.len(),
        5,
        "provide V1_TABLE SSA_INPUT ITERATIONS ROUNDS"
    );
    let bytes = std::fs::read(&arguments[1]).unwrap();
    let source = std::fs::read_to_string(&arguments[2]).unwrap();
    let compact = build(source.lines(), &BuildOptions::default()).unwrap();
    assert!(compact.stats.ef_mixed_rows + compact.stats.ef_power_of_two_rows > 0);
    let iterations: u32 = arguments[3].parse().unwrap();
    let rounds: usize = arguments[4].parse().unwrap();
    assert!(iterations > 0 && rounds > 0);
    let modes = [
        Access::Prepared,
        #[cfg(picker_api)]
        Access::PickerReuse,
        #[cfg(picker_api)]
        Access::PickerOwned,
    ];
    println!("workload,mode,round,setup_ns,batch_ns,checksum");
    for (workload, bytes) in [("ssa_v1", bytes), ("ssa_v3", compact.bytes)] {
        let expected = modes[0].measure(&bytes, iterations).2;
        for mode in modes.iter().skip(1) {
            assert_eq!(mode.measure(&bytes, iterations).2, expected);
        }
        for round in 0..rounds {
            for offset in 0..modes.len() {
                let mode = modes[(round + offset) % modes.len()];
                let (setup, elapsed, checksum) = mode.measure(&bytes, iterations);
                assert_eq!(checksum, expected);
                println!("{workload},{mode:?},{round},{setup},{elapsed},{checksum:016x}");
            }
        }
    }
}
