//! Build a portable table and publish it to a new file.
#![forbid(unsafe_code)]

use pkbyte::{build, build_support::write_table, BuildOptions, Error};

fn main() -> pkbyte::Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or(Error::InvalidOptions("provide an output path after --"))?;
    let packed = build(
        ["Tokyo", "大阪", "", "first line\nsecond line"],
        &BuildOptions::default(),
    )?;
    write_table(path, &packed, false)?;
    Ok(())
}
