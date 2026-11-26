# Usage and integration guide

[Back to the project overview](../README.md)

pkbyte builds compact, immutable tables of unique UTF-8 strings and picks
values uniformly with replacement. The Rust library and CLI use the standard
library and require Rust 1.74 or newer.

## Install the CLI

For a release published on crates.io, install the CLI with this command.

```sh
cargo install pkbyte --locked
```

To try the 0.1.1 candidate before publication, run this command from the
source checkout or unpacked `.crate`. The release guide also covers the CLI
archive and its checksums. [Release procedure](RELEASE.md)

```sh
cargo install --path . --locked --offline
```

Use Cargo's installation `bin` directory on your `PATH` to run `pkbyte`.

## Pack a file and pick a value

Put one value per line in `values.txt`. Then run these two commands.

```sh
pkbyte pack values.txt
pkbyte pick values.npk
```

The output path uses `.npk` in place of the input's extension. For a path
without an extension, `pack` appends `.npk`. It reports the path on stderr
and chooses a compact layout automatically. Use `-o` for an input path that
already ends in `.npk`. `pick` writes one JSON string to stdout by default.
Request more picks with `-n` or `--count`. Picks are with replacement, so a
value can appear more than once.

```sh
pkbyte pick values.npk -n 10
pkbyte pick values.npk -n 10 --seed 42
pkbyte info values.npk
pkbyte verify values.npk
pkbyte unpack values.npk
```

Run `pkbyte` for an overview, or `pkbyte pack --help` and
`pkbyte pick --help` for examples and options. `pkbyte --version` prints
the installed CLI version.

## Pick from Rust

For a release published on crates.io, run `cargo add pkbyte` in your
application. Before publication, add the unpacked candidate as a path
dependency. Adjust the path relative to your application's `Cargo.toml`.

```toml
[dependencies]
pkbyte = { path = "../pkbyte-0.1.1" }
```

Run with `values.npk` in your working directory, or give the file example
below a path.

```rust
use pkbyte::Picker;

fn main() -> pkbyte::Result<()> {
    let mut picker = Picker::open("values.npk")?;
    let value = picker.pick()?;
    println!("{value}");
    Ok(())
}
```

Or build a compressed table in memory from borrowed or owned strings.
`from_strings` accepts an iterator of either.

```rust
use pkbyte::Picker;

fn main() -> pkbyte::Result<()> {
    let values = ["Tokyo", "大阪", "", "first line\nsecond line"];
    let mut picker = Picker::from_strings(values)?;
    let value = picker.pick()?;
    println!("{value:?}");
    Ok(())
}
```

[File example](../examples/picker.rs) · [In-memory example](../examples/basic.rs)

## Pick repeatedly

Keep the picker and reuse it across draws. Construction owns the compressed
bytes, verifies every value and exact uniqueness, prepares the reader, and
initializes its RNG. Each pick uses that state again. The default RNG is
automatically seeded, noncryptographic SplitMix64. Calling `set_seed` resets
its stream. `Picker` and the CLI reproduce the same sequence with the same
seed and compiled table bytes under the same sampling implementation.

```rust
use pkbyte::Picker;

fn main() -> pkbyte::Result<()> {
    let mut picker = Picker::open("values.npk")?;
    picker.set_seed(42);
    let mut output = Vec::new();

    for _ in 0..1_000 {
        let value: &str = picker.pick_into(&mut output)?;
        println!("{value:?}");
    }
    Ok(())
}
```

`pick` returns an owned `String` and generally allocates for the result.
`pick_into` grows the buffer if needed and reuses it without allocation once
it has enough capacity. Use the returned `&str` as the value. The buffer
stays sized for decoding and can contain additional scratch bytes. Consume
the borrowed value before you reuse the buffer, or copy it with `to_owned()`.
For caller-provided randomness,
borrowed bytes, explicit limits, or shared immutable reader state, see the
[advanced library path](#advanced-library-use).

```sh
cargo run --locked --offline --example picker -- values.npk
cargo run --locked --offline --example basic
```

## CLI data formats and compatibility

- `pack` reads plain LF or CRLF lines by default. Each record must be nonempty.
  A final record may omit the line terminator. A single final terminator ends
  the preceding record. Blank records are errors. Whitespace inside a value
  is preserved, including a bare CR at the end of an unterminated record.
- A `.jsonl` extension selects JSON Lines, with a case-insensitive extension
  match. Use `--format lines` or `--format jsonl` to override the choice.
  Stdin defaults to lines and requires `-o`. Empty input packs a zero-row
  table. In JSON Lines, `""` represents an empty value and escaped characters
  can represent embedded controls and line breaks.
- `pick` and `unpack` write JSON Lines by default. Use `--format lines` for
  nonempty values that contain no LF and do not end in CR. An unrepresentable
  value produces an error pointing to JSON Lines. Plain unpack checks the
  complete table before writing. Plain picking can emit earlier draws before
  reaching a value it cannot represent.
- `verify` checks all values and exact uniqueness. `unpack` performs full
  validation and writes each value once in table rank order. A positive pick
  count on an empty table returns an error. A count of zero checks the file
  and emits no values.
- `-o` or `--output` selects a destination. `--force` explicitly replaces a file.
  Duplicate values are rejected by default. `--deduplicate` removes exact
  duplicates. `pkbyte pack --help` lists encoder flags and resource limits.

Pass `-` for stdin or `-o -` for binary table output. Record data and table
bytes go to stdout. File, validation, and format diagnostics go to stderr.
Output framing applies to terminals and pipes. `--force` requires a file
destination. The following pipelines use a POSIX shell.

```sh
pkbyte unpack values.npk | pkbyte pack - --format jsonl -o rebuilt.npk
pkbyte pack values.txt -o - | pkbyte info -
```

Existing scripts can keep using `build INPUT OUTPUT`, `inspect FILE`,
`sample FILE [COUNT]`, and `export FILE`. Those names keep their prior syntax,
defaults, and seeded behavior. Legacy `build` uses plain input unless you
pass `--jsonl`, regardless of the file extension. Legacy `sample` uses plain
output unless you pass `--jsonl`. Legacy `export` still accepts `--plain`.
When migrating a plain-output script, select `--format lines` on
`pick` or `unpack`. [Publication and pipeline behavior](INTEGRATION.md#file-output)

## Build and save a table from Rust

For a table you want to save or distribute, call `build` and publish its
result with `write_table`. The example uses no-clobber publication.

```rust
use pkbyte::{build, build_support::write_table, BuildOptions};

fn main() -> pkbyte::Result<()> {
    let values = ["Tokyo", "大阪", "", "first line\nsecond line"];
    let packed = build(values, &BuildOptions::default())?;
    write_table("from-rust.npk", &packed, false)?;
    Ok(())
}
```

```sh
cargo run --locked --offline --example save_table -- from-rust.npk
```

## Advanced library use

`Picker::from_bytes` takes ownership of a `Vec<u8>` containing a table.
Use `open_with_limits` or `from_bytes_with_limits` to customize reading, or
`from_strings_with_options` to supply both build options and reader limits.
Use the borrowed `Table` and `PreparedTable` APIs when your application
provides its RNG, keeps shared immutable reader state, or reads directly
from embedded bytes. Open with `Table::open_structural` and call `prepare`
for full validation and reusable sampling with one checksum pass. Use
`OpenOptions` for custom reader limits and `PrepareOptions` for the rank
reducer and hint budget. Keep the backing bytes and checked table alive
for prepared access. Each caller uses its own RNG and output slice.
[Ownership, limits, and typed errors](INTEGRATION.md)

Values retain their original UTF-8 bytes, including empty strings,
whitespace, multilingual text, and control characters. A table containing
only `""` returns an empty string on every pick. Picking from a zero-row
table returns `Error::EmptyTable`. A table's ranks identify rows in that
compiled file and can change after rebuilding.

The default build and reader value limit is 4,096 bytes. Raise both limits
to store longer values using the build and reader options. The builder
stores values beyond the codec limits as raw UTF-8. By default
`BuildMode::Compact` compares complete raw, phrase, mixed-radix Elias–Fano,
and power-of-two Elias–Fano files. You can restrict candidates with
`BuildOptions::mode`. The writer emits v2 for raw and phrase tables and v3
when it uses Elias–Fano. The reader also accepts legacy Namepack v1.
[Format policy](FORMAT.md) · [Storage selection and fallback](DESIGN.md#storage-selection)

## Embed an existing table

Distribute the portable `.npk` produced by `build`, or place it beside your
application source and use `static TABLE_BYTES: &[u8] = include_bytes!("table.npk");`.
Open that slice with `Table::open_structural`, call `prepare`, and
allocate a buffer once. Pass an application-supplied RNG into
`prepared.sample_into` on each draw. The complete example embeds the archived
SSA table and propagates startup and sampling errors. [Embedded example](../examples/embedded.rs)

```sh
cargo run --example embedded --locked --offline
```

Embedding copies the compiled file into the executable at compile time.
Initialization still parses structure, verifies CRC, decodes every value,
checks uniqueness, and allocates the reader metadata and scratch. Each
subsequent draw reconstructs one value in that scratch. [Embedding benchmark](../benchmarks/README.md#build-time-embedding)

## Generate a table during a Cargo build

Add pkbyte as both a build dependency and a runtime dependency in the
consumer's Cargo manifest. The standalone example uses local path
dependencies. [Example manifest template](../examples/generated/Cargo.toml.in)

In `build.rs`, call the supported generation helper.

```rust
use pkbyte::build_support::{generate, InputFormat};
use pkbyte::BuildOptions;

fn main() -> pkbyte::Result<()> {
    generate("values.jsonl", InputFormat::JsonLines, &BuildOptions::default())?;
    Ok(())
}
```

In the consumer, include the generated module and prepare its bytes just
as for an existing embedded table.

```rust
mod generated {
    include!(concat!(env!("OUT_DIR"), "/pkbyte_table.rs"));
}
// Open generated::TABLE_BYTES with Table::open_structural, then prepare it.
```

```sh
python3 tools/run_generated.py
```

The example runner uses Python 3.9 or newer to stage the consumer.
The helper tracks the input file for Cargo and writes `pkbyte_table.npk` and a
fixed Rust module next to it in `OUT_DIR`. Cargo recompiles the build script
when its code changes. Use `cargo:rerun-if-changed` or
`cargo:rerun-if-env-changed` for any extra file or environment configuration
your script reads. Generated files contain portable table bytes and a
relative `include_bytes!`. The target constructs and validates prepared
state using its own limits. [Build integration details](INTEGRATION.md)

## Performance tradeoffs

On the included 29,329-value SSA dataset, raw, phrase, and default compact
files occupied 178,955, 106,230, and 70,868 bytes, respectively. The recorded
verification runs used Rust 1.100.0 on Linux x86-64 with an Intel
Core i9-12900K.

| Cost | Recorded SSA result |
| --- | --- |
| Initialization from embedded bytes | 5.699 ms median for structural open, full preparation, and scratch initialization |
| Warm complete sampling on matching value traces | 30.450 ns per value for the loaded vector, 57.561 ns for prepared compact, and 251.036 ns for a prepared SQLite warm read transaction |
| Runtime heap after preparation | 2,096 bytes of checked metadata and 3,668 additional bytes of prepared metadata, plus 15 bytes of scratch per caller |
| Embedded executable footprint | 2,466,136 bytes for the full benchmark executable, including one 70,868-byte table |

The benchmark accounts for the portable bytes separately from the runtime
heap and per-caller scratch. Warm figures are medians of repeated batch
averages and include RNG, retrieval, copying, UTF-8 checking, and a checksum.
Codec choice changes storage and decoding cost. The extended report includes
small and large repetitive URL and ID sets, multilingual strings with long
values, and high-entropy input. [Workloads, methodology, and reports](../benchmarks/README.md#verification-workload-matrix)

The owning-picker benchmark compares matched seeded sequences. It measures
initialization separately and compares reusable and owned results.
[Picker benchmark](../benchmarks/README.md#owning-picker)

## Development and evidence

- [Repository commands and invariants](../AGENTS.md)
- [Contributing and reporting issues](../CONTRIBUTING.md)
- [Changelog](../CHANGELOG.md)
- [Release procedure](RELEASE.md)
- [String contract and reader guarantees](DESIGN.md)
- [File format and versioning](FORMAT.md)
- [Validation record](VALIDATION.md)
- [Application integration](INTEGRATION.md)
- [Current verification benchmarks](../benchmarks/README.md#verification-workload-matrix)
- [Codec benchmark](../benchmarks/README.md#integrated-eliasfano-codecs)
- [Prepared reader benchmark](../benchmarks/README.md#checked-and-prepared-readers)
- [Historical benchmark report](../benchmarks/results/ssa2024.json) and
  [SSA fixture provenance](../data/PROVENANCE.md)

The full source checkout includes the fuzzing guide and executed campaign
record under `fuzz/`, and isolated research under `namepack_research/`.
Historical benchmark measurements describe their original source and
environment. Checksum manifests under `docs/` record the supplied Namepack
snapshots.
