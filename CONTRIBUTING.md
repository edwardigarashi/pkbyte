# Contributing to pkbyte

Use the full source checkout for development. Use Rust 1.74.0 to check the
minimum supported version and a current stable toolchain with rustfmt and
Clippy for style checks. Use Python 3.9 or newer for the consumer runner.
The library and CLI use only the Rust standard library.

## Run the checks

Run the build, tests, style checks, and examples from the repository root.
The repository guidance lists the commands and project invariants.
[Repository guidance](AGENTS.md)

The local checks also include the release allocator test and a generated
consumer run on the minimum Rust version.

```sh
cargo test --release --test prepared_allocations --locked --offline
python3 tools/run_generated.py --toolchain 1.74.0
```

Follow the release procedure to verify an unpacked crate and CLI archive.
[Release procedure](docs/RELEASE.md)

For builder, format, reader, or sampling changes, add a regression that
checks exact stored values and the relevant error or compatibility
boundary. Preserve the committed v1 fixture and v2/v3 reading coverage.
Use the model tests and the standalone fuzz targets for broader coverage.
From the full checkout, see `fuzz/README.md` for commands. [Validation record](docs/VALIDATION.md)

Record the command, toolchain, platform, and outcome for each check you ran.
Distinguish compilation from execution when testing another target. For a
performance change, capture fresh timings, workload parameters, and source
hashes in a new report. Keep the earlier benchmark evidence tied to its
original code. [Benchmark commands and methodology](benchmarks/README.md)

## Report an issue

Use the issue tracker or maintainer contact supplied with your distribution.
Include enough information for a reproducer.

- The pkbyte package version and candidate archive checksum, when available.
- The OS and architecture, `rustc -vV` output, and the exact command or a
  minimal Rust example.
- The expected and observed behavior, including errors and exit status.
- A small shareable dataset or table that reproduces the issue. Include the
  framing, build options, format version, and relevant resource limits.
- For seeded sampling, provide the seed, RNG or CLI command, and range reducer.
  Preserve the compiled table when rank order matters.
- For a fuzz finding, include the target, command, and minimized input. Use
  a private maintainer contact for reports containing sensitive inputs.
