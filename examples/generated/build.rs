use pkbyte::build_support::{generate, InputFormat};
use pkbyte::BuildOptions;

fn main() -> pkbyte::Result<()> {
    generate(
        "values.jsonl",
        InputFormat::JsonLines,
        &BuildOptions::default(),
    )?;
    Ok(())
}
