# Working on pkbyte

The repository root contains the dependency-free Rust library and the `pkbyte`
CLI. Rust 1.74 is the minimum supported version. Follow the release procedure
for crates.io publication. The crate and tools under `namepack_research/` are
historical experiments and stay outside the default build and runtime paths.

## Local checks

Run from the repository root with an installed Rust toolchain and stable
`rustfmt` and Clippy.

```sh
cargo build --locked --offline
cargo fmt --all -- --check
cargo test --all-targets --locked --offline
cargo test --doc --locked --offline
cargo clippy --all-targets --locked --offline -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked --offline
cargo run --locked --offline --example basic
cargo run --locked --offline --example picker -- data/given_names_2024.npk
cargo run --locked --offline --example embedded
python3 tools/run_generated.py
cargo run --locked --offline --bin pkbyte -- verify data/given_names_2024.npk --unique
```

CI configures native tests and doctests on Linux, macOS, and Windows with stable
and 1.74.0. To check the MSRV locally, rerun the test commands with
`cargo +1.74.0`. For format or reader changes, keep historical files readable
and exercise malformed input, resource limits, and exact decoded values. Use
the standalone fuzz package for coverage-guided checks and record executed
tests separately from compile-only or configured jobs.
[Portability commands and evidence](docs/VALIDATION.md#portability). From the
checkout, see `fuzz/README.md` for the fuzz commands.

On Linux x86-64, `python3 tools/verify_release.py` packages and checks an
unpacked crate plus a Linux CLI archive. See [local release verification](docs/RELEASE.md)
for prerequisites and the platform gates.

## Core invariants

- Store exact UTF-8 bytes. The builder rejects duplicates by default and removes
  them only when explicitly requested. Keep CLI record framing separate from
  string semantics.
- Sample uniformly with replacement over dense, table-local ranks using the
  caller's random source. Propagate selected-record errors without retrying.
- Check untrusted bytes and resource limits before relying on offsets, lengths,
  counts, or decoded values. Structural checks and CRC alone cannot establish
  UTF-8 or uniqueness across the table. Prepared fast paths must derive
  their invariants from checked construction over the same immutable bytes.
- Write version 3 for Elias–Fano and version 2 for raw and phrase layouts.
  Preserve legacy wire interpretations. Use one raw zero-width group for
  an empty value. Assign a new version to
  incompatible changes and report historical benchmark results with their
  original provenance. New performance claims require new measurements.

See [the design](docs/DESIGN.md) and [the format](docs/FORMAT.md) for current
behavior and compatibility details.
