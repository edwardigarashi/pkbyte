# pkbyte benchmarks

## Native comparison

This harness compares the current pkbyte library with reference implementations
on the same exact name sequence.
The optional benchmark uses system SQLite, JSON-C, and zlib through FFI. These
dependencies and its unsafe FFI adapters are separate from the dependency-free,
safe Rust library.

The captured Namepack reference run contains all original measurements and
parameters, including the faster loaded vector, the smaller gzip file, and the
stronger SQLite read-transaction baseline. Fresh runs measure the current source
and record the source hashes for each run. [Historical reference run](results/ssa2024.json)
Generated files under `out/` are reproducible working outputs, not required inputs
to the library or its tests.

## Run

On Linux, with Rust, Python 3, SQLite, JSON-C, and zlib shared libraries installed:

```bash
python3 tools/fetch_ssa.py
python3 benchmarks/run.py
```

The downloaded fixture is already included. The fetch command is necessary only
when regenerating it. The script pins and verifies the upstream archive hash;
see `data/PROVENANCE.md`.

By default the benchmark writes generated comparison files and `results.json`
under `benchmarks/out/ssa2024/`. Specify another input or output location:

```bash
python3 benchmarks/run.py --input my_names.txt --output-dir benchmarks/out/custom
```

Input must contain unique, nonempty UTF-8 names, one per line. CRLF is accepted as
a line terminator. No case conversion or Unicode normalization is performed.
Do not use a directory containing unrelated files as the output location: the
script replaces its own generated `names.*` files and report there.

## What is timed

### Warm retrieval

- pkbyte compact table, already opened.
- The same pkbyte compact table with a prepared reader and initialized scratch.
- pkbyte raw table, already opened.
- Loaded Rust `Vec<Vec<u8>>`, representing an inexpensive memory baseline.
- SQLite with an open read-only connection and a reused prepared
  `SELECT name FROM names WHERE id = ?1` statement. IDs are dense
  `INTEGER PRIMARY KEY` values. This is not `ORDER BY random()`.
- A second SQLite variant keeps one read transaction open throughout each warm
  batch, avoiding per-query autocommit transaction/locking work. Its snapshot is
  acquired before timing and committed after timing so it cannot affect the
  autocommit baseline's process-wide file locks. Both
  connections use the single-thread `NOMUTEX` flag. Report this stronger
  baseline when discussing an optimized SQLite comparison.

All current methods copy complete names into preallocated reusable output
buffers, check UTF-8, and consume every output byte with the same checksum.
The new capture measures the full retrieval operation, including that output
handling.

The seeded uniform rank sequence is generated before timing. Because compact
and raw grouping can reorder records differently, the raw ranks are translated
before timing to obtain the same exact names. No translation lookup occurs in
the timed loop. A complete-byte checksum must match for every method and batch.

One unreported warmup precedes nine measured batches of 100,000 lookups by
default. Method order rotates between batches. Results report median/minimum/
maximum **batch averages** in nanoseconds per lookup. They are not per-request
p50/p95/p99 latency measurements. The report retains every batch's elapsed time.

### Complete warm sampling

The separate `warm_sampling_ns_per_name` section measures the actual random-name
operation: RNG + unbiased range reduction + retrieval + copying + the full-byte
checksum. It compares `Table::sample_into`, prepared modulo sampling, a loaded
vector using `pkbyte::uniform_below`, and the optimized SQLite read-transaction
path using that helper. Each method starts each batch from the same fixed
SplitMix64 state and must reproduce the same complete-byte checksum as the
precomputed rank sequence. Buffer allocation and RNG initialization precede the
timer.

SplitMix64 is a noncryptographic generator chosen for reproducible benchmark
inputs. These measurements do not establish the cost of a cryptographic RNG.
The original retrieval-only measurements remain separately available.

### Startup to first result

This is a fresh reader in the same running process, with an explicitly **warm
OS page cache**. It is not process-launch time or cold-disk I/O. Each path reads
or opens its own file, initializes its required structures, retrieves the same
name, copies it, and consumes all bytes. The timer stops before reader teardown.
A small reusable output buffer is allocated before the timer for all methods.

- pkbyte normal open includes the full-file CRC check.
- pkbyte structural open skips the CRC pass but retains the structural and
  selected-record validation supplied by the library. It is reported separately,
  because its verification contract differs.
- Raw pkbyte normal open includes its full-file CRC check.
- SQLite opens a connection, prepares a point query, and reads one row.
- JSON reads compact JSON and parses the entire array using native JSON-C.
- Gzip reads and decompresses level-9 gzip text through zlib, validates UTF-8,
  materializes a full line vector, and retrieves one row.

The gzip implementation models a reusable full-list loader. A one-shot streaming
scan to a randomly selected row is another possible baseline and is not measured
here. JSON-C retains its native object graph; gzip uses a Rust vector. These
loader choices and allocation costs are part of the reported startup results.
All pack files are read into byte vectors in this harness; mmap is not measured.

## Verification and interpretation

Before timing, the harness decodes every stored rank of both packs, validates
their semantics and uniqueness, and compares the reconstructed sets with the
source. SQLite uses compact-table rank order; JSON and gzip use lexical order so
whole-stream gzip benefits from adjacent similar names. Their rank translations
are computed outside timing. Every timed batch and startup result is checked
against the expected full-byte checksum.

The report includes exact file sizes, including dictionaries and directories;
source hash; build times; CPU/kernel; compiler; library versions; parameters; and
all timing samples. Reproduction metadata hashes every core Rust source file,
Cargo manifest/lockfile, the harness, the runner, and the compiled artifacts;
the runner refuses a reference result if the source changes during measurement.
Gzip is measured as a real whole-stream compressor, so it may
be substantially smaller than an independently addressable representation.

The included SSA corpus is only 29,329 distinct ASCII given names. Its files and
working set are small. Results are not evidence about Japanese strings,
surnames, millions of names, or memory pressure. The harness does not measure
peak/resident memory, cold storage, per-request tail latency, cryptographic
random-number cost, concurrent access, or WebAssembly. Shared-host scheduling
can introduce noise; avoid concurrent builds or heavy tasks while timing.

## Checked and prepared readers

Run from the repository root with Rust and Python 3 installed. The harness uses
only Rust's standard library and pkbyte.

```sh
python3 benchmarks/run_prepared.py --iterations 500000 --rounds 9
```

The runner saves the generated workload and fresh report to
`benchmarks/out/prepared/`. Use `--output-dir` for a separate run. The captured
report identifies the compiler, host, source, dataset, and binary hashes and
contains every measured batch. [Prepared-reader report](results/prepared.json)

The workloads are the archived SSA 2024 version 1 table and a deterministic
synthetic version 2 table. The latter mixes short strings, several scripts,
controls, an empty value, and long values that exercise the raw fallback.
Before timing, the harness checks every rank against the checked reader and
compares the synthetic table with its exact source set.

- Warm retrieval supplies the same precomputed ranks to checked and prepared
  readers. The prepared configurations use the default hint budget or disable
  hints entirely.
- Complete modulo sampling compares `Table::sample_into` with prepared modulo
  sampling. Both consume the same seeded SplitMix64 stream and return the same
  value sequence. Complete multiply-high sampling compares prepared sampling
  with the checked reader using an equivalent precomputed-threshold
  multiply-high reducer. The reducers use different value sequences from each
  other, so compare within a trace.
- Every warm path copies and UTF-8 checks the result and accumulates the same
  checksum over each length and byte. Scratch allocation precedes timing. One
  unreported warmup round precedes measurement, and the starting method rotates
  across rounds. Reported warm medians are medians of batch averages.
- Preparation timing starts from an already opened table and includes its
  full CRC, decoding, and uniqueness pass. The retained heap count covers the
  additional dictionary and hints. It excludes the borrowed table, allocator
  overhead, temporary validation memory, and per-caller scratch. The report
  also lists the prepared handle's inline size and per-caller scratch sizes.

The captured run used Rust 1.91.1 on Linux x86-64 with an Intel Core i9-12900K.
Each warm value below is the median batch-average time in nanoseconds.

| Operation | SSA v1 | Synthetic v2 |
| --- | ---: | ---: |
| Checked retrieval | 47.493 | 99.125 |
| Prepared retrieval without hints | 30.042 | 77.375 |
| Prepared retrieval with hints | 21.139 | 69.334 |
| Checked modulo sampling | 50.228 | 101.412 |
| Prepared modulo sampling | 25.349 | 71.264 |
| Checked retrieval with multiply-high sampling | 48.283 | 100.149 |
| Prepared multiply-high sampling | 22.939 | 70.900 |

| Preparation and storage | SSA v1 | Synthetic v2 |
| --- | ---: | ---: |
| Preparation without hints, median ms | 5.487 | 1.574 |
| Preparation with hints, median ms | 5.546 | 1.575 |
| Prepared inline bytes | 96 | 96 |
| Additional retained heap without hints, bytes | 2,304 | 2,304 |
| Additional retained heap with hints, bytes | 5,972 | 4,360 |
| Checked scratch, bytes | 15 | 5,871 |
| Prepared scratch, bytes | 22 | 5,878 |

The measurements apply to these two warm workloads and this host. Cold I/O,
peak memory, latency percentiles, and concurrent access remain unmeasured.

## Integrated Elias–Fano codecs

Run the std-only comparison with Rust and Python 3 installed.

```sh
python3 benchmarks/run_codecs.py --iterations 250000 --rounds 7 --build-rounds 3
```

Fresh files and the report go to `benchmarks/out/codecs/`. Use `--output-dir`
to keep a separate run. The report records all batch times, source and binary
hashes, and each complete serialized file's hash. [Codec report](results/codecs.json)

The harness builds raw, phrase, mixed-radix Elias–Fano, power-of-two
Elias–Fano, and automatic compact tables over three exact sets.

- SSA 2024 uses the included UTF-8 source list.
- The synthetic multilingual set has constrained positional alphabets in
  Japanese, Greek, and Arabic, plus an empty string, controls, emoji, and
  12 long strings that use the raw fallback. Building it with a 4,096-byte
  value limit must fail. The captured builds use an 8,192-byte limit.
- The synthetic high-entropy set has 2,048 printable ASCII values of 48 bytes
  each. The harness checks that the positional alphabet product exceeds `u64`.
  Both Elias–Fano modes must fall back to raw for this fixture.

Every build is checked for determinism. Before timing, checked and prepared
decoding agree at every rank, and every compiled table is compared with the
exact source set. The automatic result must have the smallest complete file
size among the five choices.

The prepared retrieval batches use one precomputed value trace. Rank
translation happens before timing, so all paths return exactly the same
values in the same order. Complete sampling uses the same SplitMix64 seed and
multiply-high reduction over each file's local ranks. Different rank orders
can produce different value sequences. Each sampling path is checked against
its own expected checksum, and the report records total output bytes by trace.
All paths use reusable output buffers, check UTF-8, and checksum each output
length and byte. One unreported warmup precedes the rotated measured rounds.

Setup phases time building, structural opening of resident bytes, opening with
CRC, full semantic and uniqueness validation, and preparation from an already
opened table. Each timing excludes teardown of the returned object. Reported
memory includes inline handles, retained metadata and hints, and per-caller
scratch as separate quantities. Serialized file bytes, temporary validation
allocations, allocator overhead, and peak resident memory have separate scopes.
The experiment covers warm single-threaded runs on the recorded host.

The captured comparison used Rust 1.91.1 and an Intel Core i9-12900K. Warm
times are medians of seven batch averages of 250,000 values each.

| Workload | Codec choice | Complete bytes | Matched retrieval ns/value | Local sampling ns/value |
| --- | --- | ---: | ---: | ---: |
| SSA 2024 | Phrase | 106,230 | 21.850 | 23.929 |
| SSA 2024 | Mixed Elias–Fano | 70,868 | 51.991 | 55.010 |
| SSA 2024 | Power-of-two Elias–Fano | 76,924 | 51.254 | 53.654 |
| Multilingual and long | Phrase | 112,163 | 55.097 | 55.134 |
| Multilingual and long | Mixed Elias–Fano | 80,484 | 130.970 | 131.240 |
| Multilingual and long | Power-of-two Elias–Fano | 81,948 | 99.364 | 101.601 |

Automatic compact mode selected mixed Elias–Fano for SSA and the multilingual
fixture. It selected the 93,809-byte phrase table for the high-entropy set,
where both attempted Elias–Fano modes emitted the 98,384-byte raw table.

| SSA 2024 setup and storage | Raw | Phrase | Mixed Elias–Fano | Power-of-two Elias–Fano |
| --- | ---: | ---: | ---: | ---: |
| Build, median ms | 1.392 | 59.624 | 2.148 | 2.184 |
| Structural open, median µs | 1.542 | 46.941 | 93.944 | 91.154 |
| CRC-checked open, median µs | 292.497 | 209.589 | 202.409 | 210.549 |
| Full validation, median ms | 4.748 | 5.245 | 5.655 | 5.508 |
| Preparation, median ms | 4.720 | 5.252 | 5.705 | 5.481 |
| Checked inline bytes | 216 | 216 | 216 | 216 |
| Checked metadata heap bytes | 560 | 4,736 | 2,096 | 2,096 |
| Prepared inline bytes | 112 | 112 | 112 | 112 |
| Additional prepared heap bytes | 3,668 | 5,972 | 3,668 | 3,668 |
| Hint bytes included in prepared heap | 3,668 | 3,668 | 3,668 | 3,668 |
| Checked scratch bytes | 15 | 15 | 15 | 15 |
| Prepared scratch bytes | 15 | 22 | 15 | 15 |

The full report includes all phases and all codec choices for the three
workloads. [Codec report](results/codecs.json)

## Build-time embedding

Run from the repository root with Rust and Python 3 available.

```sh
python3 benchmarks/run_embedding.py --iterations 500000 --rounds 9
```

The runner compiles a small Rust generator that calls `pkbyte::build_support::generate`
on the included SSA 2024 text source using line input and default build options.
The generator compares each decoded value against the exact input set. The runner
then compiles the benchmark with the generated `pkbyte_table.rs` wrapper and
counts exact occurrences of `pkbyte_table.npk` in the resulting executable. It
requires one match and exactly two generated files. The full executable size
includes Rust runtime code, the harness, and the embedded table. The report
records portable bytes, payload bytes, container overhead, wrapper source bytes,
and the absence of serialized prepared metadata. The Elias–Fano indexes are
part of the portable payload. The runtime rank hints are created during
preparation. Both Rust programs use the standard library and pkbyte. Python
orchestrates the benchmark.

The executable opens the embedded bytes structurally, prepares the checked table
with CRC and exhaustive uniqueness validation, and initializes reusable scratch
storage. Timing records each stage and their sum, with one unreported warmup
before the repeated startup batches. The measurements start with bytes resident
in the executable and exclude process launch, file I/O, and teardown. A separate
single-thread pass counts allocator calls and cumulative requested bytes for each
stage. A reallocation contributes its new size. The allocator wrapper remains
installed during timing with its counter disabled. Retained metadata and scratch
capacity are measured independently of temporary allocations and allocator overhead.

Warm sampling uses the prepared reader with default multiply-high reduction and
a seeded SplitMix64 source. Each batch includes RNG, sampling, UTF-8 validation,
and a checksum of every output length and byte. The checked reader independently
computes the expected checksum for the same rank sequence. A separate tracked
batch requires zero allocation calls and requested bytes. The benchmark writes
all elapsed batches, environment details, and source and artifact hashes under
`benchmarks/out/embedding/`. [Embedding report](results/embedding.json)

The captured Linux x86-64 run used Rust 1.91.1 and an Intel Core i9-12900K.
Default compact encoding stored the 29,329 exact SSA values in a 70,868-byte
version 3 table. The payload occupied 70,580 bytes and the header and
directory occupied 288 bytes. The 95-byte wrapper produced one exact copy of
the table in the 2,231,392-byte benchmark executable. No prepared metadata
was serialized. Checked metadata retained 2,096 heap bytes and preparation
added 3,668 heap bytes for rank hints. Scratch used 15 bytes per caller.

| Initialization stage | Median of nine runs | Allocation calls | Requested bytes |
| --- | ---: | ---: | ---: |
| Structural open of embedded bytes | 92.304 µs | 4 | 3,248 |
| Prepare with CRC and exact uniqueness | 5.194 ms | 33,286 | 1,333,366 |
| Initialize scratch | 49 ns | 1 | 15 |
| Total initialization | 5.288 ms | 33,291 | 1,336,629 |

Warm prepared sampling measured a median of 53.798 ns per sample across nine
batches of 500,000 samples. Every batch matched checksum `eb80fd5ea38e50b5`.
The separately tracked warm batch requested zero allocations. These timings
cover one dataset and one native build on the recorded host. [Embedding report](results/embedding.json)

## Verification workload matrix

The extended codec suite adds 64 and 160,000 exact URL and account-ID values
with deliberately repetitive prefixes to the SSA, multilingual and long, and
high-entropy workloads. The synthetic URL and ID fixtures illustrate this
specific shape of repetition. Each codec table and the loaded `Vec<String>`
receive the same precomputed retrieval trace. The vector copies every result
into reusable scratch, checks UTF-8, and computes the same checksum. Its heap
count includes the allocated vector slots and string capacities.

```sh
RUSTUP_TOOLCHAIN=1.100.0 python3 benchmarks/run_codecs.py --extended-workloads --iterations 200000 --rounds 7 --build-rounds 3 --output-dir benchmarks/out/verification-codecs
RUSTUP_TOOLCHAIN=1.100.0 python3 benchmarks/run.py --iterations 100000 --rounds 9 --startup-rounds 9 --output-dir benchmarks/out/verification-native-ssa
RUSTUP_TOOLCHAIN=1.100.0 python3 benchmarks/run_embedding.py --iterations 500000 --rounds 9 --output-dir benchmarks/out/verification-embedding
```

The codec runner measures build allocations in a separate process after the
timed passes. The tracked call to `pkbyte::build` borrows resident inputs.
Cumulative requested bytes include every allocation request, including the new
size of a reallocation and requests that fail. The peak is the largest sum of
requested sizes of successful Rust allocations still live at an allocator
boundary. Deallocation removes a block's size. Successful reallocation
replaces the old size with the new one, and failed reallocation leaves the old
size live. This measures requested Rust heap created in the scope. Input heap,
allocator overhead, internal allocator overlap during reallocation, stack,
and RSS are outside that scope.

The allocation probe exercises growth, shrinkage, and deallocation with known
sizes. Each measured build must produce the same bytes as the timed build, and
its live heap at return must match the returned table buffer's capacity. The
embedding capture applies the same tracker to each initialization stage and
checks its retained heap separately.

The native run uses matching rank/value traces for the checked and prepared
compact readers, loaded vector, and SQLite with a prepared statement and warm
read transaction. The embedding run measures warm use of the embedded SSA file.
Startup in the codec matrix starts from resident bytes. The separate native
startup comparison includes reads through a warm filesystem cache. The reports
contain the measurement environment, elapsed batches, and source and artifact
hashes.

The captures below used Rust 1.100.0 on Linux x86-64 with an Intel Core
i9-12900K. Codec warm times are medians of seven batch averages. Build time
and heap figures are medians of three measured builds each. [Extended codec report](results/verification-codecs.json)

| Workload | Values | Compact file bytes | Compact build ms | Build requested bytes | Build peak live bytes | Vector retrieval ns/value | Compact prepared retrieval ns/value |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| SSA 2024 | 29,329 | 70,868 | 60.480 | 16,425,821 | 2,460,944 | 18.863 | 52.712 |
| Multilingual and long | 6,165 | 80,484 | 48.116 | 5,018,647 | 1,493,604 | 48.550 | 132.216 |
| High entropy | 2,048 | 93,809 | 83.000 | 2,725,168 | 954,344 | 39.767 | 66.255 |
| Small URLs and IDs | 64 | 407 | 1.509 | 605,346 | 541,346 | 30.941 | 119.485 |
| Large URLs and IDs | 160,000 | 50,471 | 584.768 | 103,619,266 | 27,362,005 | 67.880 | 125.108 |

On SSA, the native harness measured identical value sequences for all methods
below. Times are median batch averages in nanoseconds per value across nine
warm batches. [Native SSA report](results/verification-native-ssa.json)

| SSA reader | Retrieval | Complete sampling |
| --- | ---: | ---: |
| Loaded vector | 23.656 | 30.450 |
| Checked compact table | 60.897 | 65.264 |
| Prepared compact table | 51.612 | 57.561 |
| SQLite prepared warm read transaction | 245.627 | 251.036 |

All three current captures produced the same 70,868-byte SSA compact table.
The embedding executable contained one exact copy. Its median initialization
was 5.699 ms, and warm prepared sampling measured 51.048 ns per value. The
preparation stage peaked at 1,329,682 bytes of new live heap and retained
3,668 bytes. The separate tracked warm batch requested zero allocations. [Embedding verification report](results/verification-embedding.json)

## Owning picker

The same harness compared the archived SSA v1 table and a rebuilt SSA v3
table containing Elias–Fano blocks. All paths used SplitMix64, modulo
reduction, the same seed, and a checksum of the complete returned values.
Checksums matched across both releases for each table. Setup starts with
resident bytes and includes structural parsing, full validation, and
preparation. The picker measurement also includes its RNG initialization.
File I/O, cloning the input fixture, and initial sizing of output buffers
occur outside the timed batches.

| Table | 0.1.0 prepared | 0.1.1 prepared | `Picker::pick_into` | `Picker::pick` |
| --- | ---: | ---: | ---: | ---: |
| SSA v1 | 25.022 ns | 25.777 ns | 27.149 ns | 34.787 ns |
| SSA v3 | 55.117 ns | 55.965 ns | 57.220 ns | 64.692 ns |

Times are medians of nine batches of 500,000 picks, per pick, using CPU 4
on the Intel Core i9-12900K and Rust 1.100.0. Picker setup medians were
5.309 ms and 5.821 ms for v1 and v3 using the reusable-buffer path.
Borrowed prepared setup took 5.294 ms and 5.747 ms. `pick_into` added
1.372 ns and 1.255 ns to the corresponding prepared sample in this run.
Returning an owned `String` cost a further 7.638 ns and 7.472 ns in the
matching picker paths. The separate release allocation test measured zero
allocation attempts for repeated `pick_into` with sufficient buffer capacity
on the phrase and both Elias–Fano codecs. [Full measurements and hashes](results/picker.json)

For the 0.1.0 baseline, build the library from its unpacked local candidate
and compile the same harness without `--cfg picker_api` against that library.
For the 0.1.1 measurement, run the following from the candidate source.

```sh
cargo +1.100.0 build --release --lib --locked --offline --target-dir target/picker-measure
rustc +1.100.0 --edition=2021 -O -C lto=thin -C codegen-units=1 \
  --cfg picker_api --extern pkbyte=target/picker-measure/release/libpkbyte.rlib \
  -L dependency=target/picker-measure/release/deps benchmarks/picker.rs \
  -o target/picker-measure/picker-benchmark
taskset -c 4 target/picker-measure/picker-benchmark data/given_names_2024.npk data/ssa_2024_given_names.txt 500000 9
```
