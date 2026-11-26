# pkbyte coverage-guided fuzzing

This standalone Cargo package runs libFuzzer with AddressSanitizer and Rust
overflow checks. Dependencies and the lockfile are local to `fuzz/`.

- `table_read` feeds arbitrary bytes to both checked and structural opening.
  It also tests a copy with independently recomputed CRC-32 so mutations can
  reach record validation and preparation. Successful preparation must agree
  with checked retrieval and modulo sampling. The deterministic seed set
  contains v1 and v2 raw and phrase tables, both v3 Elias–Fano codecs,
  truncation, and header and block bit mutations with valid CRC-32. It includes
  valid-CRC seeds for invalid UTF-8, inconsistent decoded totals, and the same
  value in raw and Elias–Fano groups.
- `compact_roundtrip` frames bounded test values from the input and exercises
  raw, phrase, both Elias–Fano variants, and automatic mode with both duplicate
  policies. It compares checked and prepared output at every rank against the
  exact expected set, and the complete candidate size against the raw file.

The reader target caps files at 16 KiB, rows and groups at 128, decoded values
at 288 bytes, and total decoded bytes at 12 KiB. The builder target processes
at most 128 values of 96 input bytes each. It applies UTF-8 replacement only to
form test values from arbitrary fuzzer input. Each generated value is then
compared byte for byte. Phrase training has its own smaller budget. A fixed
fallback RNG word bounds the number of draws in a sampling check.

## Run

Use a C++ compiler with its standard library development files, Rust nightly,
and `cargo-fuzz`. Run these commands from the repository root.

```sh
rustup toolchain install nightly --profile minimal --component rust-src
cargo install cargo-fuzz --version 0.13.2 --locked
mkdir -p fuzz/work/table_read fuzz/work/compact_roundtrip
cargo +nightly fuzz run --sanitizer address table_read fuzz/work/table_read fuzz/seeds/table_read -- -seed=73031 -max_len=16384 -max_total_time=50 -timeout=5 -rss_limit_mb=512 -print_final_stats=1
cargo +nightly fuzz run --sanitizer address compact_roundtrip fuzz/work/compact_roundtrip fuzz/seeds/compact_roundtrip -- -seed=73032 -max_len=16384 -max_total_time=50 -timeout=5 -rss_limit_mb=512 -print_final_stats=1
```

Use empty working corpus directories for a fresh campaign. libFuzzer reads
the versioned seeds and writes discovered inputs into the first corpus directory.
The live corpora and generated artifacts are ignored. The pinned seed controls
mutations. A time-limited run can execute different input counts on another
host. [Executed campaign](#executed-campaign)

Recreate the seed files in a scratch directory and compare them to the
versioned copies.

```sh
cargo +nightly run --locked --manifest-path fuzz/Cargo.toml --example seed_corpus -- fuzz/work/regenerated
diff -r fuzz/seeds fuzz/work/regenerated
```

Replay an artifact by passing its file path after the target name. Turn a
confirmed failure into a minimal regression test for the affected behavior.

## Executed campaign

On 2026-11-25, both targets ran on Linux x86-64 on an Intel Core i9-12900K
with Rust `1.102.0-nightly` (`739adecd4`), `cargo-fuzz` 0.13.2, and locked
`libfuzzer-sys` 0.4.13. The binaries were optimized with AddressSanitizer,
coverage instrumentation, and Rust overflow checks.

| Target | Seed | Seed files | Elapsed | Executions | libFuzzer coverage counters | Peak process RSS |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `table_read` | 73031 | 13 | 51 s | 939,225 | 632 → 856 | 283 MiB |
| `compact_roundtrip` | 73032 | 7 | 51 s | 37,957 | 1,500 → 2,103 | 318 MiB |

Both runs reached their time bound and exited successfully. The 977,182 total
executions produced zero crashes, sanitizer findings, input timeouts, RSS-limit
failures, or failure artifacts. Coverage counters come from libFuzzer's initial
and final `cov` fields. RSS is for each instrumented process, including its
runtime and corpus. Seed regeneration matched all versioned files byte for byte.

Each run started with an empty writable corpus and the checked-in seeds. The two
processes ran concurrently, with a 50-second configured bound per target, a
5-second per-input timeout, a 512 MiB RSS limit per process, and a 16 KiB
input ceiling. The harnesses set tighter limits on rows, decoded sizes, and
phrase training. The host initially lacked a C++ compiler and standard library
headers. The campaign used matching Fedora GCC 16 packages extracted into the
ignored working directory. [Commands and seed regeneration](#run)

An earlier 51-second run per target also finished without a finding, with
951,364 reader executions and 36,100 round-trip executions. The recorded
repeat used an expanded reader corpus with explicit seeds for invalid UTF-8,
incorrect decoded totals, and duplicate values across codecs. The separate CI
smoke job is configured for 1,024 executions per target. Longer campaigns and
additional sanitizer or platform configurations remain follow-up work.
