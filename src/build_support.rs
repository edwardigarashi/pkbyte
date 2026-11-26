//! Dataset compilation and Cargo build-script integration.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{BuildOptions, BuildOutput, Error, Result, Table};

/// Record framing for a UTF-8 dataset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputFormat {
    /// Nonempty values separated by LF or CRLF. The final record may omit
    /// its delimiter. All bytes outside the delimiters are preserved.
    Lines,
    /// One JSON string per physical line, including empty strings.
    JsonLines,
}

/// Compile a line-oriented UTF-8 dataset with the regular pkbyte builder.
///
/// `max_input_bytes` limits both the encoded stream and the builder's decoded
/// input. The reader retains one physical line at a time and the builder
/// retains the decoded values. An empty input stream contains zero rows.
///
/// # Errors
///
/// Invalid UTF-8, malformed framing, and overlong records return
/// [`Error::InvalidInput`] with a one-based physical line number. Byte and
/// row limits can return [`Error::LimitExceeded`]. Duplicate handling follows
/// the build options. I/O errors propagate as [`Error::Io`].
pub fn build_from_reader(
    input: impl Read,
    format: InputFormat,
    options: &BuildOptions,
) -> Result<BuildOutput> {
    let maximum = u64::try_from(options.max_input_bytes).unwrap_or(u64::MAX);
    let mut input = BufReader::new(input.take(maximum.saturating_add(1)));
    let mut remaining = options.max_input_bytes;
    let mut line = Vec::new();
    let mut line_number = 0;
    let records = std::iter::from_fn(|| {
        if line_number == options.max_input_rows {
            // Check for excess input without collecting a record that the
            // builder cannot accept, including an unterminated final line.
            loop {
                match input.fill_buf() {
                    Ok([]) => return None,
                    Ok(_) => {
                        let limit = if remaining == 0 {
                            "encoded input limit"
                        } else {
                            "input row count"
                        };
                        return Some(Err(Error::LimitExceeded(limit)));
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Some(Err(error.into())),
                }
            }
        }
        line.clear();
        let read = match input.read_until(b'\n', &mut line) {
            Ok(0) => return None,
            Ok(read) => read,
            Err(error) => return Some(Err(error.into())),
        };
        let Some(left) = remaining.checked_sub(read) else {
            return Some(Err(Error::LimitExceeded("encoded input limit")));
        };
        remaining = left;
        line_number += 1;
        let result = match format {
            InputFormat::Lines => decode_plain_line(&line, options.max_value_bytes),
            InputFormat::JsonLines => crate::text::decode_json_string(
                line.strip_suffix(b"\n").unwrap_or(&line),
                options.max_value_bytes,
            ),
        };
        Some(result.map_err(|reason| Error::InvalidInput {
            line: line_number,
            reason,
        }))
    });
    crate::builder::build_records(records, options, std::convert::identity)
}

fn decode_plain_line(line: &[u8], maximum: usize) -> std::result::Result<String, &'static str> {
    let value = match line.strip_suffix(b"\n") {
        Some(line) => line.strip_suffix(b"\r").unwrap_or(line),
        None => line,
    };
    if value.is_empty() {
        return Err(
            "empty plain record. Use --format jsonl (or --jsonl) to represent empty values",
        );
    }
    if value.len() > maximum {
        return Err("decoded value exceeds the configured byte limit");
    }
    std::str::from_utf8(value)
        .map(str::to_owned)
        .map_err(|_| "expected valid UTF-8")
}

/// Paths of the two files produced by [`generate`].
#[derive(Clone, Debug)]
pub struct GeneratedTable {
    /// Portable pkbyte file containing the complete compiled table.
    pub table_path: PathBuf,
    /// Rust module exporting a static `TABLE_BYTES` slice.
    pub source_path: PathBuf,
}

/// Compile a dataset in a Cargo build script and emit one table in `OUT_DIR`.
///
/// Emits a Cargo change-tracking directive for the input path. Cargo tracks
/// changes to the build script itself. If your build options use additional
/// files or environment variables, emit the corresponding Cargo directives.
/// This function owns `pkbyte_table.npk` and `pkbyte_table.rs` in the output
/// directory and publishes each file atomically.
///
/// Include the generated module with
/// `include!(concat!(env!("OUT_DIR"), "/pkbyte_table.rs"))`. The module embeds
/// the portable bytes with a relative `include_bytes!` path. Open them with
/// [`Table::open_structural`] and call [`Table::prepare`] for checked
/// initialization on the target. Use [`Table::open_with_options`] for
/// target-specific limits.
///
/// # Errors
///
/// Missing `OUT_DIR` or an input path unsuitable for a Cargo change-tracking
/// directive returns [`Error::InvalidOptions`]. Input, validation, and output
/// errors propagate before the build script reports success.
pub fn generate(
    input: impl AsRef<Path>,
    format: InputFormat,
    options: &BuildOptions,
) -> Result<GeneratedTable> {
    const MODULE: &[u8] = b"/// Compiled pkbyte table.\npub static TABLE_BYTES: &[u8] = include_bytes!(\"pkbyte_table.npk\");\n";

    let directory = env::var_os("OUT_DIR").ok_or(Error::InvalidOptions(
        "Cargo OUT_DIR is required for build generation",
    ))?;
    let input = input.as_ref();
    let tracked = input
        .to_str()
        .filter(|path| !path.contains(&['\r', '\n'][..]))
        .ok_or(Error::InvalidOptions(
            "Cargo dataset path must be UTF-8 without line breaks",
        ))?;
    writeln!(io::stdout().lock(), "cargo:rerun-if-changed={tracked}")?;
    let file = File::open(input).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("cannot open dataset {}: {error}", input.display()),
        )
    })?;
    if file.metadata()?.len() > options.max_input_bytes as u64 {
        return Err(Error::LimitExceeded("encoded input limit"));
    }
    let table = build_from_reader(file, format, options)?;
    // Use builder-derived limits to verify valid builds that deliberately
    // exceed the default runtime limits. The target still chooses its limits.
    let verification = crate::OpenOptions {
        verify_checksum: false,
        limits: crate::Limits {
            max_file_bytes: table.bytes.len(),
            max_rows: table.stats.row_count,
            max_value_bytes: table.stats.max_value_bytes,
            max_groups: table.stats.groups,
            max_decoded_bytes: table.stats.raw_value_bytes,
        },
    };
    Table::open_with_options(&table.bytes, &verification)?.validate_all(true)?;

    let directory = PathBuf::from(directory);
    let paths = GeneratedTable {
        table_path: directory.join("pkbyte_table.npk"),
        source_path: directory.join("pkbyte_table.rs"),
    };
    // Publish the constant module first. A failure here leaves the previous
    // table intact, and an existing module can refer to either table revision.
    write_atomic(&paths.source_path, MODULE, true)?;
    write_table(&paths.table_path, &table, true)?;
    Ok(paths)
}

/// Publish an already compiled table from a synchronized sibling temporary file.
///
/// Without replacement, publication uses a hard link and fails if the target
/// exists. With replacement, publication uses rename. Both require filesystem
/// support. The parent directory is left unsynchronized.
pub fn write_table(path: impl AsRef<Path>, table: &BuildOutput, replace: bool) -> io::Result<()> {
    write_atomic(path.as_ref(), &table.bytes, replace)
}

struct TemporaryFile(PathBuf);

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn write_atomic(path: &Path, bytes: &[u8], replace: bool) -> io::Result<()> {
    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
    let destination = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing output filename"))?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    for _ in 0..128 {
        let serial = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        let name = format!(".pkbyte-{}-{serial}.tmp", std::process::id());
        if destination.to_string_lossy().eq_ignore_ascii_case(&name) {
            continue;
        }
        let candidate = parent.join(name);
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let temporary = TemporaryFile(candidate);
        let written = file.write_all(bytes).and_then(|()| file.sync_all());
        drop(file);
        written?;
        if replace {
            fs::rename(&temporary.0, path)?;
        } else {
            fs::hard_link(&temporary.0, path)?;
        }
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "cannot reserve a temporary output file",
    ))
}
