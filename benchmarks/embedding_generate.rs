//! Generate the embedded benchmark's files through the build-script API.
#![forbid(unsafe_code)]

use pkbyte::build_support::{generate, InputFormat};
use pkbyte::{BuildOptions, Table};
use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let input = PathBuf::from(env::args_os().nth(1).ok_or("missing input path")?);
    let generated = generate(&input, InputFormat::Lines, &BuildOptions::default())?;

    let bytes = fs::read(generated.table_path)?;
    let source = fs::read_to_string(input)?;
    let mut expected: BTreeSet<_> = source.lines().collect();
    let table = Table::open(&bytes)?;
    assert_eq!(table.len(), expected.len() as u64);
    let mut scratch = Vec::with_capacity(table.max_value_bytes());
    for rank in 0..table.len() {
        assert!(expected.remove(table.get_into(rank, &mut scratch)?));
    }
    assert!(expected.is_empty());
    Ok(())
}
