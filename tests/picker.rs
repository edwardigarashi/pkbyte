use pkbyte::{BuildMode, BuildOptions, DuplicatePolicy, Error, Limits, Picker, Table};
use std::path::Path;

#[path = "support/checksum.rs"]
mod checksum;
use checksum::refresh_crc;

#[test]
fn builds_from_borrowed_and_owned_iterators_and_picks_exact_values() -> pkbyte::Result<()> {
    let values = ["Tokyo", "大阪", "", "line\nline\t\0", "  ", "é", "e\u{301}"];
    let mut borrowed = Picker::from_strings(values.iter().copied())?;
    let mut owned = Picker::from_strings(values.into_iter().map(str::to_owned))?;
    assert_eq!(borrowed.len(), values.len() as u64);
    assert_eq!(borrowed.as_bytes(), owned.as_bytes());

    borrowed.set_seed(42);
    owned.set_seed(42);
    let mut buffer = Vec::new();
    for _ in 0..128 {
        let value = borrowed.pick()?;
        assert!(values.contains(&value.as_str()));
        assert_eq!(value, owned.pick_into(&mut buffer)?);
    }
    buffer.clear();
    let capacity = buffer.capacity();
    assert!(values.contains(&owned.pick_into(&mut buffer)?));
    assert_eq!(buffer.capacity(), capacity);
    Ok(())
}

#[test]
fn file_picker_matches_the_established_cli_seeded_sequence() -> pkbyte::Result<()> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/given_names_2024.npk");
    let mut picker = Picker::open(&path)?;
    let mut buffer = Vec::new();
    for _ in 0..2 {
        picker.set_seed(42);
        for expected in ["Dessa", "Xailey", "Sameer", "Shulem", "Rhaelynn"] {
            assert_eq!(picker.pick_into(&mut buffer)?, expected);
        }
    }
    let limits = Limits {
        max_file_bytes: picker.as_bytes().len() - 1,
        ..Limits::default()
    };
    assert!(matches!(
        Picker::open_with_limits(&path, &limits),
        Err(Error::LimitExceeded("file size"))
    ));
    let missing = path.with_file_name("missing.npk");
    let error = Picker::open(&missing).unwrap_err();
    assert!(matches!(error, Error::Io(ref e) if e.kind() == std::io::ErrorKind::NotFound));
    assert!(error.to_string().contains("missing.npk"));
    Ok(())
}

#[test]
fn owning_picker_checks_limits_and_file_integrity_and_retains_the_original_bytes(
) -> pkbyte::Result<()> {
    assert!(matches!(
        Picker::from_bytes(vec![]),
        Err(Error::InvalidFormat("truncated header"))
    ));
    let encoded = Picker::from_strings(["🦀", "  "])?;
    let mut bytes = encoded.as_bytes().to_vec();
    bytes[56] ^= 1;
    assert!(matches!(
        Picker::from_bytes(bytes),
        Err(Error::ChecksumMismatch { .. })
    ));

    let bytes = encoded.as_bytes().to_vec();
    let start = bytes.as_ptr();
    let mut moved = Picker::from_bytes(bytes)?;
    assert_eq!(start, moved.as_bytes().as_ptr());
    assert!(matches!(moved.pick()?.as_str(), "🦀" | "  "));
    assert!(matches!(
        Picker::from_bytes_with_limits(
            moved.as_bytes().to_vec(),
            &Limits {
                max_rows: 1,
                ..Limits::default()
            }
        ),
        Err(Error::LimitExceeded("row count"))
    ));

    // Existing structure and metadata checks permit duplicate raw records.
    // Check that owning construction also enforces exact uniqueness.
    let unique = Picker::from_strings(["A", "B"])?;
    let mut duplicate = unique.as_bytes().to_vec();
    *duplicate.last_mut().unwrap() = b'A';
    refresh_crc(&mut duplicate);
    assert!(Table::open(&duplicate)?.validate_all(false).is_ok());
    assert!(matches!(Picker::from_bytes(duplicate), Err(Error::DuplicateValue(s)) if s == "A"));
    Ok(())
}

#[test]
fn empty_table_empty_value_and_duplicate_policies_are_distinct() -> pkbyte::Result<()> {
    let mut empty = Picker::from_strings(std::iter::empty::<&str>())?;
    assert!(empty.is_empty());
    assert!(matches!(empty.pick(), Err(Error::EmptyTable)));
    assert!(matches!(
        empty.pick_into(&mut vec![]),
        Err(Error::EmptyTable)
    ));
    assert!(matches!(
        Picker::from_strings(["", ""]),
        Err(Error::DuplicateValue(s)) if s.is_empty()
    ));
    let mut singleton = Picker::from_strings_with_options(
        ["", ""],
        &BuildOptions {
            mode: BuildMode::Raw,
            duplicate_policy: DuplicatePolicy::Deduplicate,
            ..BuildOptions::default()
        },
        &Limits::default(),
    )?;
    assert!(!singleton.is_empty());
    assert_eq!(singleton.pick()?, "");
    assert_eq!(singleton.pick_into(&mut vec![])?, "");
    Ok(())
}

#[test]
fn duplicate_diagnostics_escape_controls_and_bound_the_preview() {
    let value = format!("\0\n{}", "🦀".repeat(100));
    let error = Picker::from_strings([&value, &value]).unwrap_err();
    let detail = error.to_string();
    assert!(matches!(error, Error::DuplicateValue(full) if full == value));
    assert!(detail.contains(r"\0\n"));
    assert!(detail.ends_with("(402 bytes)"));
    assert!(detail.len() < 120);
}
