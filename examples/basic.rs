//! Build a picker from an iterator and reuse a buffer across seeded picks.
#![forbid(unsafe_code)]

use pkbyte::Picker;

fn main() -> pkbyte::Result<()> {
    let values = ["", "  spaces  ", "東京", "café", "👩🏽‍💻", "line\nbreak"];
    let strings = values.into_iter().map(str::to_owned);
    let mut picker = Picker::from_strings(strings)?;
    println!(
        "{} values, {} table bytes",
        picker.len(),
        picker.as_bytes().len()
    );

    picker.set_seed(42);
    println!("{:?}", picker.pick()?);
    let mut output = Vec::new();
    for _ in 0..5 {
        let value = picker.pick_into(&mut output)?;
        println!("{value:?}");
    }
    Ok(())
}
