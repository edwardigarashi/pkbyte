//! JSON string records used by the CLI's lossless line framing.

use std::io::{self, Write};

pub(super) fn write_record(output: &mut impl Write, value: &str) -> io::Result<()> {
    output.write_all(b"\"")?;
    let bytes = value.as_bytes();
    let mut start = 0;
    for (index, &byte) in bytes.iter().enumerate() {
        let escape: &[u8] = match byte {
            b'"' => b"\\\"",
            b'\\' => b"\\\\",
            b'\x08' => b"\\b",
            b'\x0c' => b"\\f",
            b'\n' => b"\\n",
            b'\r' => b"\\r",
            b'\t' => b"\\t",
            0..=0x1f => b"",
            _ => continue,
        };
        output.write_all(&bytes[start..index])?;
        if escape.is_empty() {
            write!(output, "\\u{byte:04x}")?;
        } else {
            output.write_all(escape)?;
        }
        start = index + 1;
    }
    output.write_all(&bytes[start..])?;
    output.write_all(b"\"\n")
}
