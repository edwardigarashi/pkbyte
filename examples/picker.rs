//! Open a packed table and pick one owned string.
#![forbid(unsafe_code)]

use pkbyte::{Error, Picker};

fn main() -> pkbyte::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or(Error::InvalidOptions("provide a table path after --"))?;
    let mut picker = Picker::open(path)?;
    let value = picker.pick()?;
    println!("{value}");
    Ok(())
}
