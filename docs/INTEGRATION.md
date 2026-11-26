# Application integration

## Owning picker

`Picker::open` takes a file path and loads it within the default reader
limits. `Picker::from_bytes` takes ownership of compiled `Vec<u8>` bytes.
`Picker::from_strings` builds a compressed table from an iterator of borrowed
or owned strings with the default builder options. Each constructor verifies
the table and prepares it with reusable metadata before the first pick.
`len` and `is_empty` report the number of stored values. `as_bytes` returns
a borrowed view of the complete portable file.

Use `open_with_limits` or `from_bytes_with_limits` to supply reader
`Limits`. Use `from_strings_with_options` to supply separate `BuildOptions`
and `Limits`, including exact deduplication, codec selection, and higher
value limits. To save a new table with the same no-clobber publication as
the CLI, use `build` and pass its `BuildOutput` to
`build_support::write_table`. [Save example](../examples/save_table.rs)

The picker owns its noncryptographic SplitMix64 stream. Construction seeds
that stream once, and `set_seed` resets it for reproducibility.
`Picker` and the CLI use the same modulo range reduction. A fixed seed
reproduces the sequence for the same compiled file bytes and sampling
implementation. Ranks and repeatable sequences can change when a table is
rebuilt.

`pick` produces an owned `String` and generally allocates for the result.
It reuses internal decoding scratch after that scratch is initialized.
`pick_into` writes into a caller-supplied `Vec<u8>` and can grow the
buffer on its first use. It reuses sufficient capacity on later calls.
Use the returned `&str` as the value. The buffer remains sized for decoding
and can contain scratch bytes beyond the returned prefix. Finish using
the borrowed string before passing the buffer back for another pick.
[In-memory picker example](../examples/basic.rs) · [File example](../examples/picker.rs)

## Borrowed readers

Use `Table::open` for a checked view of a compiled file. Use
`Table::open_structural` followed by `Table::prepare` for repeated sampling with
one checksum pass. Either path can borrow bytes from memory, a loaded file, or
`include_bytes!`. Keep the backing bytes and checked table alive for the
prepared reader's lifetime. The same types and exact string contract apply
across all three sources. [Reader design](DESIGN.md#prepared-access)

When your application supplies an RNG for an external file, bound the read
and use the checked and prepared readers over that buffer.

```rust
use std::fs::File;
use std::io::Read;
use std::path::Path;

use pkbyte::{OpenOptions, PrepareOptions, Table};

fn pick_from_file(path: &Path, next_u64: &mut impl FnMut() -> u64) -> pkbyte::Result<()> {
    let options = OpenOptions {
        verify_checksum: false, // prepare verifies the checksum below
        ..OpenOptions::default()
    };
    let read_limit = (options.limits.max_file_bytes as u64).saturating_add(1);
    let mut bytes = Vec::new();
    File::open(path)?.take(read_limit).read_to_end(&mut bytes)?;
    let table = Table::open_with_options(&bytes, &options)?;
    let prepared = table.prepare(&PrepareOptions::default())?;
    let mut output = prepared.new_output_buffer()?;
    for _ in 0..5 {
        let value = prepared.sample_into(next_u64, &mut output)?;
        println!("{value:?}");
    }
    Ok(())
}
```

## Ownership and reuse

`build` and `build_from_reader` return an owned `BuildOutput` containing
portable bytes and build statistics. `Table` borrows a slice of those
bytes, and `PreparedTable` borrows metadata and payload from the checked
table. Keep the underlying slice immutable and both owners alive until
callers finish using the prepared reader.

Opening allocates parsed metadata. Preparation verifies the CRC and every
decoded value, checks exact uniqueness with temporary storage, and can
retain a dense phrase dictionary and budgeted rank hints. After preparation,
retrieval and sampling allocate no memory with sufficient initialized
scratch. Prepared calls use `&mut [u8]`, so pass `new_output_buffer()` or
an initialized buffer of at least `output_buffer_len()` bytes. The returned
`&str` borrows the scratch until its last use. Call `to_owned()` if you
need an independent string. Checked `Table::get_into` takes a reusable
`Vec<u8>` and can grow it to fit. The checked `get` returns an owned string.

Share a prepared reader immutably when its borrowed data remains alive for
all callers. Each caller supplies its own mutable scratch and source of
independent uniform `u64` draws. Sampling is with replacement, so repeated
draws may return the same value. Ranks and seeded sequences refer to a
particular compiled table and range reducer. [File compatibility and determinism](FORMAT.md#compatibility-and-versioning)

## Limits and errors

The following defaults apply to the library and corresponding CLI behavior.
Byte counts refer to bytes rather than characters.

| Scope | Option | Default |
| --- | --- | ---: |
| Builder | `BuildOptions::max_input_rows` before deduplication | 1,000,000 |
| Builder | `BuildOptions::max_input_bytes` | 256 MiB |
| Builder | `BuildOptions::max_value_bytes` | 4,096 |
| Reader | `Limits::max_file_bytes` | 257 MiB |
| Reader | `Limits::max_rows` | 1,000,000 |
| Reader | `Limits::max_decoded_bytes` | 256 MiB |
| Reader | `Limits::max_groups` | 65,536 |
| Reader | `Limits::max_value_bytes` | 4,096 |
| Preparation | `PrepareOptions::max_hint_bytes` | 4,096 |

`BuildOptions` also configures phrase training. The input byte cap counts
each value before deduplication. The text helper applies the same cap to
encoded input including record delimiters, quotes, and escapes. When loading
external table bytes, bound the original file or stream read using the same
`Limits::max_file_bytes` set on `OpenOptions`. Leave room for the header,
directory, and any dictionary or index when setting the file limit. To
store longer values, raise the builder and reader value limits together.
Raw UTF-8 stores values that exceed codec limits.

Build and validation return `Error::DuplicateValue` for exact duplicates.
Text parsing returns `Error::InvalidInput` with a one-based physical line
for a malformed record. Check `Error::LimitExceeded` for reader, builder,
and aggregate stream budgets. Reader errors distinguish an invalid
signature, unsupported version, layout, CRC mismatch, and invalid UTF-8.
Prepared retrieval returns `Error::OutOfBounds` for an invalid rank and
`Error::OutputTooSmall` for short scratch. Sampling a zero-row table
returns `Error::EmptyTable`. Sampling a singleton empty value returns
`Ok("")`. [Typed errors](../src/error.rs)

## Cargo generation

Call `pkbyte::build_support::generate(input, format, &options)` from
`build.rs`. The helper accepts plain lines or JSON Lines. It uses
`build_from_reader` and the standard builder, then opens and verifies the
compiled table, including uniqueness. It tracks the dataset with
`cargo:rerun-if-changed`. Use an input path in UTF-8 with no line breaks
so Cargo can track it. Cargo tracks the Rust build script and its build
dependencies. A consumer using other configuration inputs can add Cargo
rerun directives for those files or environment variables.

One call produces `pkbyte_table.npk` and `pkbyte_table.rs` in Cargo's
`OUT_DIR`. The Rust module contains a static slice referring to the portable
file by relative path. Arbitrary values remain bytes in the table, so
quotes, backslashes, controls, and text resembling Rust source never enter
generated Rust syntax. Contents are deterministic for identical values,
framing, and build options with the same writer. The little-endian file
uses fixed-width integers and the target decodes it with its own checked
conversions and limits.

The portable file includes its own dictionary, group directory, and any
Elias–Fano indexes. The generated wrapper adds a constant declaration.
Preparation on the target still performs full validation and constructs
runtime metadata. It temporarily retains decoded values to check uniqueness.
After setup, prepared retrieval and sampling use the caller's scratch with
zero allocations. [Complete consuming example](../examples/generated/README.md)

## File output

The CLI finishes parsing and building before it opens a temporary output.
It writes to a reserved sibling temporary file, synchronizes its data, and
publishes via a hard link for a new destination or rename for an explicit
`--force` replacement. A hard link fails atomically if the destination
already exists. The filesystem must support the respective operation.
A failed publish leaves any existing destination intact. Temporary files
are removed on ordinary return, with best-effort cleanup on failure.
Crash durability of the directory entry depends on the filesystem because
publication does not synchronize the parent directory.

Cargo generation verifies the complete new table before either file is
published. It publishes the constant module first and replaces the data
last. The same module references either valid revision at the fixed path.
Errors are returned to Cargo. Forced process termination can leave a
temporary file.

The primary commands are `pack`, `pick`, `info`, `verify`, and `unpack`.
Pipelines can use `-` for input and `pack -o -` for binary output. Specify
`--format jsonl` when piping JSON Lines into `pack` because stdin defaults
to plain lines. The legacy `build INPUT OUTPUT` spelling still takes
`-` as its output path. Output to stdout
streams through a buffer and can be partial if writing fails. An early
closed stdout pipe is treated as a successful stop. Table input buffers
are bounded by `--max-file-bytes`; reading a table requires its full
portable bytes. Text builds cap the encoded input, retain one physical
record while reading, and retain the decoded set and encoder working state
while building. Unpack and pick allocate reusable scratch and stream
values rather than retaining a separate output list.

### Troubleshooting

| Problem | Resolution |
| --- | --- |
| Input path is missing or unreadable | Check the path in the diagnostic and pass a readable file or stdin. |
| A line has the wrong framing | Check the physical line reported by the decoder. Use `pack --format lines` for nonempty lines or `pack --format jsonl` for one JSON string per record. |
| Duplicate values | Remove the exact duplicates or pass `pack --deduplicate`. |
| Destination exists | Choose a new path with `-o` or explicitly replace it with `--force`. |
| Unsupported format or damaged table | Check [readable versions](FORMAT.md#compatibility-and-versioning), then rebuild from the original strings or restore a verified copy. |
| An empty table cannot be picked from | Pack at least one value. Use `""` in JSON Lines for a table whose only value is the empty string. |
| A limit is exceeded | Use `pack --help` for input limits, or the reader command's help for file, row, decoded, group, and value limits. |
| A value cannot be written as plain lines | Select `--format jsonl` to preserve the exact value. |
