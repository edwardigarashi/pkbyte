# Validation record

This record describes executed checks on earlier source snapshots. Use the
repository checks for the current checkout and the release procedure for a
new archive. [Repository checks](../AGENTS.md#local-checks) · [Release procedure](RELEASE.md)

## Release verification

The 0.1.0 and 0.1.1 candidates were exercised locally on Linux x86-64.
Both passed the clean-package workflow with Rust 1.100.0 and 1.74.0. The
workflow unpacked the Cargo 1.74.0 source archive, then ran tests, doctests,
and the Rust and generated-consumer examples. Formatting, strict Clippy,
warning-free rustdoc, the release allocator test, dependency inspection,
and both of the then-configured Cargo feature checks passed. The release
workflow installed the CLI from the unpacked crate and ran the executable
from the musl tarball. ELF inspection confirmed the absence of a dynamic
interpreter or shared library dependencies. The snapshots had no Git revision
metadata. Each artifact set included SHA-256 checksums, a file list, and a
build record. [Release procedure](RELEASE.md#build-and-verify)

| Platform | 0.1.0 outcome | 0.1.1 outcome |
| --- | --- | --- |
| Linux x86-64, Rust 1.100.0 and 1.74.0 | 84 tests and one doctest passed on both toolchains. | 95 tests and two doctests passed on both toolchains. |
| i686 Linux musl | All 84 tests passed. | All 95 tests passed. |
| AArch64 Linux musl under QEMU 10.2.2 | 51 selected tests passed. | 56 selected tests passed. |
| WASI Preview 1 under Wasmtime 38.0.0 | 51 selected tests passed. The generated consumer matched the native output. | 55 selected tests and the generated consumer passed. |
| Bare WebAssembly, big-endian PowerPC64, Windows x86-64 GNU, and macOS AArch64 | Compilation checks passed. The bare WebAssembly generated consumer linked. | Compilation checks passed. The bare WebAssembly generated consumer linked. |

Native Windows and macOS execution remained pending in these records. The
workflow configures native tests on Linux, macOS, and Windows with stable
and Rust 1.74.0, plus portability, style, release, and fuzz jobs. The table
records local checks. The full checkout contains `.github/workflows/ci.yml`.

The 0.1.0 candidate added an explicit package include list, a staged Cargo
consumer, and frozen compatibility fixtures. Its CLI built, inspected,
verified, sampled, exported, and rebuilt a table of nine strings that included
Unicode, an empty value, embedded newlines, NUL, and a trailing CR. Export
preserved the set and rebuilding produced identical bytes. Migration of the
frozen SSA table preserved all 29,329 exact values. The fuzz dependency
license declarations were inspected with Cargo metadata. Third-party runtime
notices accompany the release. [Wire policy](FORMAT.md) · [Third-party notices](THIRD-PARTY.md)

The 0.1.1 candidate tested the two-command first-use flow, Unicode JSON Lines
unpack with a byte-identical rebuild, and the owning picker. The picker tests
covered exact strings, malformed input, seeded replay, and use after unlinking
the original file on native targets. The release allocator test recorded
zero allocation attempts for prepared access and warmed `pick_into`.
The matching SSA benchmark measured about 1.3 ns per pick for the owning
picker's reusable-output path over prepared sampling and about 7.5 ns more
per pick for owned output on the measured host. [Picker benchmarks](../benchmarks/README.md#owning-picker)

Both candidates also ran 1,024 AddressSanitizer executions per fuzz target
without findings. A longer pre-candidate campaign ran 977,182 executions
without findings. See the executed campaign section of `fuzz/README.md` in
the full checkout for commands and results.

## Portability

Run from the repository root with the listed rustup targets installed for
Rust 1.100.0. The Linux commands below use 32-bit binary support and
`qemu-aarch64-static`. The WASI runner uses the Python package
`wasmtime==38.0.0`. [WASI runner](../tools/run_wasi.py)

```sh
sysroot=$(rustc +1.100.0 --print sysroot)
lld="$sysroot/lib/rustlib/x86_64-unknown-linux-gnu/bin/rust-lld"
CARGO_TARGET_I686_UNKNOWN_LINUX_MUSL_LINKER="$lld" \
  cargo +1.100.0 test --target i686-unknown-linux-musl --all-targets --locked --offline
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER="$lld" \
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUNNER=qemu-aarch64-static \
  cargo +1.100.0 test --target aarch64-unknown-linux-musl --lib --test compact --test compatibility --test roundtrip_and_format --test input_limits --test picker --locked --offline
export CARGO_TARGET_WASM32_WASIP1_RUNNER="$PWD/tools/run_wasi.py"
cargo +1.100.0 test --target wasm32-wasip1 --lib --test compact --test compatibility --test roundtrip_and_format --test input_limits --test picker --locked --offline -- --test-threads=1 --skip file_picker_matches_the_established_cli_seeded_sequence
python3 tools/run_generated.py --toolchain 1.100.0 --target wasm32-wasip1
cargo +1.100.0 check --target powerpc64-unknown-linux-gnu --lib --locked --offline
cargo +1.100.0 check --target wasm32-unknown-unknown --lib --locked --offline
python3 tools/run_generated.py --toolchain 1.100.0 --target wasm32-unknown-unknown build
cargo +1.100.0 check --target x86_64-pc-windows-gnu --all-targets --locked --offline
cargo +1.100.0 check --target aarch64-apple-darwin --all-targets --locked --offline
```

## Contract coverage

| Guarantee | Evidence |
| --- | --- |
| Exact UTF-8 and dense ranks | Twenty deterministic cases exercise all five builder modes, both duplicate policies, and every table rank against a `BTreeSet` model. [Model tests](../tests/properties.rs) |
| Uniform sampling with replacement | An exhaustive 8-bit model enumerates every draw for each nonzero bound. A `u128` model checks both 64-bit reductions and random draw counts. Checked and prepared modulo sequences match, an empty table consumes no randomness, and selected decoding errors propagate. [Range reduction](../src/sampling.rs) · [Model tests](../tests/properties.rs) · [Reader tests](../tests/roundtrip_and_format.rs) |
| Resource limits and prepared construction | Tests cover exact and one-under limits, overflow, malformed data with valid CRCs, UTF-8, decoded totals, and duplicates across codecs. [Model tests](../tests/properties.rs) · [Stream tests](../tests/input_limits.rs) · [Reader tests](../tests/roundtrip_and_format.rs) · [Compact tests](../tests/compact.rs) · [Prepared tests](../tests/prepared.rs) |
| Format and codec compatibility | Frozen v1, v2, and v3 fixtures and the archived SSA table exercise exact decoded values. The compact tests check codec constraints, bounded select scans, and raw fallback. [Compatibility tests](../tests/compatibility.rs) · [Compact tests](../tests/compact.rs) |
| Deterministic CLI and embedding | Tests cover lossless JSON Lines, seeded output, pipelines, atomic replacement, determinism across output directories and builder entry points, and protection of existing generated outputs on error. [CLI tests](../tests/cli.rs) · [Build integration](../tests/build_integration.rs) |
| Prepared sharing and warmed allocation | Threads share immutable prepared readers with separate RNGs and output buffers. The release allocator test checks that prepared and warmed picker operations allocate zero times after setup. [Model tests](../tests/properties.rs) · [Allocation test](../tests/prepared_allocations.rs) |

## Earlier verification

All rows describe local Linux x86-64 executions. The feature stages also
passed formatting, strict Clippy, and warning-free rustdoc.

| Snapshot | Tests | Additional results |
| --- | --- | --- |
| Supplied Namepack | 34 tests on Rust 1.91.1. The supplied record also reported 34 on 1.100.0 and 1.74.0. | Both supplied SHA-256 manifests matched before editing. The CLI verified the SSA fixture and the research crate compiled. [Historical benchmark report](BENCHMARKS.md) |
| pkbyte foundation | 35 tests and one doctest on Rust 1.91.1 and 1.74.0. | A v1 rebuild was byte-identical and replayed the original CLI's seeded sequence. An 18,000-byte raw value round-tripped with raised limits. The benchmark harness and standalone research checks completed. |
| General strings and JSON Lines | 44 tests and one doctest on Rust 1.91.1 and 1.74.0. | Python encoded 444 distinct values and decoded 10,000 CLI samples spanning the set. v1 and v2 decoded the same SSA set. The original CLI rejected v2. |
| Prepared reader | 56 tests and one doctest on Rust 1.91.1 and 1.74.0. | The release allocator check recorded zero allocations after setup. Both benchmark workloads agreed on every rank and on the sample checksum. [Prepared benchmark report](../benchmarks/results/prepared.json) |
| Elias–Fano | 70 tests and one doctest on Rust 1.91.1 and 1.74.0. | Tests accepted a 2,048-bit select span and rejected one of 2,049 bits. Both new codecs passed the allocator check. [Codec benchmarks](../benchmarks/README.md#integrated-eliasfano-codecs) |
| Application integration | 79 tests and one doctest on Rust 1.91.1 and 1.74.0. | The generated consumer ran on both toolchains, regenerated after an input edit, and retained its previous output after malformed input. The bare WebAssembly library and consumer compiled and the CLI rebuilt a byte-identical table. [Embedding benchmarks](../benchmarks/README.md#build-time-embedding) |
| Pre-candidate verification | 82 tests and one doctest on Rust 1.100.0 and 1.74.0. | All 82 tests ran on i686. 50 selected tests ran on AArch64 under QEMU and WASI under Wasmtime. The WASI consumer matched native output. The big-endian library and bare WebAssembly library compiled, and the bare WebAssembly consumer linked. |

Pre-candidate verification reproduced a row-limit bug that read 1,048,582
bytes despite a one-row cap. The regression passed on native Linux, i686,
AArch64, and WASI after the reader began detecting extra input before
assembling the second record. The verification also exposed an incorrect
assumption about Cargo's output layout on i686. The generated-consumer test
now locates the actual output directory and checks its bytes.
[Stream limit regression](../tests/input_limits.rs) · [Build integration](../tests/build_integration.rs)

Benchmark reports retain the measured source hashes, workloads, toolchain,
and environment. The pre-candidate matrix measured five workloads and kept
file sizes, build time and heap, preparation, warm sampling, and embedding
footprint as separate results. [Workload matrix](../benchmarks/README.md#verification-workload-matrix)
