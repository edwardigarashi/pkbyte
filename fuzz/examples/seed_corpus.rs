//! Recreate the small, deterministic seed files used by the fuzz targets.
#![forbid(unsafe_code)]

use pkbyte::{build, BuildMode, BuildOptions, Error, PrepareOptions, Table};
use pkbyte_fuzz::{refresh_crc, INPUT_VALUE_BYTES, ROWS};
use std::fs;
use std::io;
use std::path::Path;

fn save(path: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    fs::write(path.join(name), bytes)
}

fn framed(control: u8, values: &[&str]) -> Vec<u8> {
    let mut bytes = vec![control];
    for value in values {
        assert!(value.len() <= INPUT_VALUE_BYTES);
        bytes.push(value.len() as u8);
        bytes.extend_from_slice(value.as_bytes());
    }
    bytes
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args_os()
        .nth(1)
        .ok_or("pass the seed output directory")?;
    let root = Path::new(&root);
    let reader = root.join("table_read");
    let roundtrip = root.join("compact_roundtrip");
    fs::create_dir_all(&reader)?;
    fs::create_dir_all(&roundtrip)?;

    let options = BuildOptions {
        mode: BuildMode::Raw,
        ..BuildOptions::default()
    };
    let mut legacy = build(["Ada", "\t  ", "世", "line\nend"], &options)?.bytes;
    legacy[8..10].copy_from_slice(&1u16.to_le_bytes());
    refresh_crc(&mut legacy);
    save(&reader, "valid-v1-raw.npk", &legacy)?;

    let raw = build(["", "\0\t", "🦀", "世", "\r\n"], &options)?.bytes;
    save(&reader, "valid-v2-raw.npk", &raw)?;
    let mut truncated = raw[..raw.len() - 1].to_vec();
    refresh_crc(&mut truncated);
    save(&reader, "v2-truncated-valid-crc.npk", &truncated)?;
    let mut bad_utf8 = raw.clone();
    *bad_utf8.last_mut().unwrap() = 0xff;
    refresh_crc(&mut bad_utf8);
    assert!(Table::open(&bad_utf8)?.validate_all(false).is_err());
    save(&reader, "v2-invalid-utf8-valid-crc.npk", &bad_utf8)?;
    let mut bad_flags = raw;
    bad_flags[10] = 1;
    refresh_crc(&mut bad_flags);
    save(&reader, "v2-header-flags-valid-crc.npk", &bad_flags)?;

    let redundant: Vec<_> = (0..ROWS)
        .map(|index| format!("prefix {index:08b} suffix suffix"))
        .collect();
    let redundant: Vec<_> = redundant.iter().map(String::as_str).collect();
    let phrase = build(
        &redundant,
        &BuildOptions {
            mode: BuildMode::Phrase,
            ..BuildOptions::default()
        },
    )?;
    assert!(phrase.stats.compressed_rows > 0);
    save(&reader, "valid-v2-phrase.npk", &phrase.bytes)?;
    let mut bad_total = phrase.bytes.clone();
    bad_total[24..32].copy_from_slice(&(phrase.stats.raw_value_bytes - 1).to_le_bytes());
    refresh_crc(&mut bad_total);
    assert!(Table::open(&bad_total)?.validate_all(false).is_err());
    save(&reader, "v2-decoded-total-valid-crc.npk", &bad_total)?;

    let mut legacy_phrase = phrase.bytes;
    legacy_phrase[8..10].copy_from_slice(&1u16.to_le_bytes());
    refresh_crc(&mut legacy_phrase);
    save(&reader, "valid-v1-phrase.npk", &legacy_phrase)?;

    for (mode, name) in [
        (BuildMode::EliasFanoMixed, "mixed"),
        (BuildMode::EliasFanoPowerOfTwo, "power-of-two"),
    ] {
        let built = build(
            &redundant,
            &BuildOptions {
                mode,
                ..BuildOptions::default()
            },
        )?;
        assert!(built.stats.ef_bytes > 0);
        assert_eq!(built.stats.groups, 1);
        save(&reader, &format!("valid-v3-{name}.npk"), &built.bytes)?;
        let mut malformed = built.bytes;
        // Single group with no dictionary. The EF reserved bytes start 21
        // bytes after the block begins, just after the group directory.
        malformed[64 + 16 + 21] ^= 0x80;
        refresh_crc(&mut malformed);
        save(
            &reader,
            &format!("v3-{name}-reserved-bit-valid-crc.npk"),
            &malformed,
        )?;
    }

    let mixed = build(
        &redundant[1..],
        &BuildOptions {
            mode: BuildMode::EliasFanoMixed,
            ..BuildOptions::default()
        },
    )?;
    assert_eq!(mixed.stats.groups, 1);
    assert_eq!(mixed.stats.ef_mixed_rows, (ROWS - 1) as u64);
    assert_eq!(mixed.stats.dictionary_bytes, 0);
    let duplicate = Table::open(&mixed.bytes)?.get(0)?;
    let mut bytes = mixed.bytes[..64].to_vec();
    bytes[16..24].copy_from_slice(&(ROWS as u64).to_le_bytes());
    bytes[24..32]
        .copy_from_slice(&(mixed.stats.raw_value_bytes + duplicate.len() as u64).to_le_bytes());
    bytes[40..44].copy_from_slice(&2u32.to_le_bytes());
    bytes[48..56].copy_from_slice(&((mixed.stats.ef_bytes + duplicate.len()) as u64).to_le_bytes());
    // Insert a single-row raw group before the original Elias–Fano group.
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&(duplicate.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&1u64.to_le_bytes());
    bytes.extend_from_slice(&mixed.bytes[64..80]);
    bytes.extend_from_slice(duplicate.as_bytes());
    bytes.extend_from_slice(&mixed.bytes[80..]);
    refresh_crc(&mut bytes);
    let table = Table::open(&bytes)?;
    table.validate_all(false)?;
    assert!(matches!(
        table.prepare(&PrepareOptions::default()),
        Err(Error::DuplicateValue(value)) if value == duplicate
    ));
    save(&reader, "v3-cross-codec-duplicate-valid-crc.npk", &bytes)?;

    for (control, name) in [
        (0, "raw"),
        (1, "phrase"),
        (2, "mixed"),
        (3, "power-of-two"),
        (4, "auto"),
    ] {
        save(&roundtrip, name, &framed(control, &redundant))?;
    }
    let long = "世".repeat(25);
    save(
        &roundtrip,
        "deduplicate-empty-unicode-and-long",
        &framed(4, &["", "", "🦀\0", "a\n", "E\u{301}", "É", &long]),
    )?;
    save(
        &roundtrip,
        "reject-duplicate",
        &framed(0x84, &["", "\0", ""]),
    )?;
    Ok(())
}
