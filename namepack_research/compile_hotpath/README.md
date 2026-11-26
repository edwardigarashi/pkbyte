# Isolated reader experiments

These isolated prototypes compare experimental readers with the pkbyte library.
Run from this directory with Rust installed. The prototypes read the repository's
legacy `.npk` fixture and the saved Elias–Fano files in `../compression/`.

```bash
cargo run --release --offline --locked
python3 run_ef.py
```

The first command prints a new report to standard output. The second saves its
report under `../out/compile_hotpath/`. The checked-in JSON reports and
`experiment_manifest.json` identify the original Namepack source and runs.

The first command compares complete sampling, including SplitMix64, unbiased
rank generation, record reconstruction, UTF-8 validation, and a complete-byte
checksum. The prepared reader uses the same serialized `.npk` fixture, a
reusable padded output buffer, dense dictionary arrays, and an optional rank
hint directory. Modulo methods share one name trace. Multiply-high methods use
a second shared, uniformly generated trace. The experiment checks both traces.

The second compares retrieval for the same pre-generated name trace across byte phrases and new mixed-radix / Elias-Fano prototypes. RNG and rank translations are outside timing. Every method copies/reconstructs output, validates UTF-8 and checksums every byte. Before timing, every EF record is compared against the existing exact name set. Files and environment are recorded in `experiment_manifest.json`.

Both experiments use eleven rotated batches of one million requests plus an unreported warmup. Results are median batch-average nanoseconds, not individual-request percentiles. There is no cold-I/O, process-startup, concurrency, or multi-device measurement.

The prepared phrase constructor checks every decoded record before timing and
assumes it receives the same immutable bytes as the checked table, a nonempty
table, at most 256 groups, and sufficient padded output. The EF parser expects
producer-verified research files. It lacks complete corruption and resource-limit
checks for external input. Safe Rust bounds checks and per-result UTF-8 checks
remain enabled.

The EF reader has a 2,048-byte within-byte select lookup table, in-memory arrays and a 917-byte rank hint directory on this corpus. These are runtime decoder support, not included in the serialized data sizes. Code/binary footprint must be counted separately for embedded deployments. Periodic select checkpoints may scan multiple words and do not establish worst-case constant-time select.

Measured 2026-11-25 on the same Linux x86-64 environment as the preceding namepack prototype. Toolchain: Rust 1.100.0. Generic release compilation with thin LTO; no CPU-specific instruction target enabled. Results do not establish performance on other name sets, scripts or CPUs, nor scientific novelty of the combination.
