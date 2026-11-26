use pkbyte::{PrepareOptions, Table};
use std::io::{self, Write};

mod generated {
    include!(concat!(env!("OUT_DIR"), "/pkbyte_table.rs"));
}

fn sample(next_u64: &mut impl FnMut() -> u64) -> pkbyte::Result<()> {
    let table = Table::open_structural(generated::TABLE_BYTES)?;
    let prepared = table.prepare(&PrepareOptions::default())?;
    let mut output = prepared.new_output_buffer()?;
    let mut stdout = io::stdout().lock();
    for _ in 0..8 {
        writeln!(stdout, "{:?}", prepared.sample_into(next_u64, &mut output)?)?;
    }
    Ok(())
}

fn main() -> pkbyte::Result<()> {
    // SplitMix64 supplies reproducible draws for this demonstration.
    let mut state = 0x42a6_97bf_2cad_9e1d_u64;
    let mut next_u64 = || {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    };
    sample(&mut next_u64)
}
