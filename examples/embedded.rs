//! Sample a portable table embedded as part of the application.
#![forbid(unsafe_code)]

use std::io::{self, Write};

use pkbyte::{PrepareOptions, Table};

static TABLE_BYTES: &[u8] = include_bytes!("../data/given_names_2024.npk");

fn print_samples(next_u64: &mut impl FnMut() -> u64) -> pkbyte::Result<()> {
    let table = Table::open_structural(TABLE_BYTES)?;
    let prepared = table.prepare(&PrepareOptions::default())?;
    let mut output = prepared.new_output_buffer()?;
    let mut stdout = io::stdout().lock();
    for _ in 0..5 {
        let value = prepared.sample_into(next_u64, &mut output)?;
        writeln!(stdout, "{value:?}")?;
    }
    Ok(())
}

fn main() -> pkbyte::Result<()> {
    // SplitMix64 is a deterministic demonstration source. The caller can
    // provide its own generator of independent uniform u64 draws.
    let mut state = 42_u64;
    print_samples(&mut || {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    })
}
