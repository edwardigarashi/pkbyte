# Historical Namepack prototype results

Measured 2026-11-25 using the original Namepack prototype, on one dataset and one Linux x86_64 execution environment. The archived report identifies the source used for those measurements.

## Result

The compact table stores **29,329 unique names in 106,230 bytes**, including all metadata. That is **3.622 bytes per name**. Complete warmed sampling measured **63.793 ns/name**, approximately **5.83 times faster** than the prepared SQLite read-transaction baseline in this run.

The loaded vector is faster, and gzip is smaller. Normal CRC-verified startup is slower than SQLite startup. Those qualifications are part of the result.

## Complete warm sampling

Includes RNG, unbiased bounded-rank generation, retrieval, copying into reusable output storage, and the same complete-byte checksum. Nine batches of 100,000 samples per method, plus one unreported warmup. Values are median/minimum/maximum **batch-average** nanoseconds per name, not per-request latency percentiles. The generator is noncryptographic SplitMix64, initialized before each timed batch.

| Method | Median ns/name | Minimum batch average | Maximum batch average |
| --- | ---: | ---: | ---: |
| Loaded vector | 42.260 | 33.521 | 50.682 |
| Namepack compact sample_into | 63.793 | 61.856 | 67.137 |
| SQLite prepared query, persistent read transaction | 372.196 | 351.498 | 428.767 |

## Complete file sizes

| Representation | Bytes |
| --- | ---: |
| Lexically sorted gzip text, level 9 | 72,985 |
| Namepack compact | 106,230 |
| Namepack raw | 178,955 |
| LF-delimited UTF-8 text | 207,996 |
| Compact JSON | 266,655 |
| SQLite database | 438,272 |

The compact file is 75.76% smaller than SQLite and 48.93% smaller than LF text. It is 45.55% larger than gzip. The raw UTF-8 name bytes without delimiters occupy 178,667 bytes.

The compact format breakdown is 64 header bytes + 800 dictionary bytes + 256 group-directory bytes + 105,110 payload bytes. There are 256 dictionary symbols and 16 groups. 28,849 records are coded; 480 remain raw.

## Reader startup to first result

New reader in the same process with a **warm OS page cache**. Pack files are read into byte vectors. This does not measure cold storage or process launch. Output storage is preallocated for all paths; teardown is outside the timer.

| Loader | Median microseconds |
| --- | ---: |
| Compact, structural opening without CRC | 60.490 |
| SQLite open, prepare and lookup | 80.590 |
| Compact, normal CRC-verified opening | 303.611 |
| Raw, normal CRC-verified opening | 435.137 |
| Gzip read, decompress and materialize full line vector | 1994.024 |
| JSON-C read and parse complete array | 2057.408 |

Structural opening retains bounds and selected-record checks but skips the full-file CRC pass. It is a different verification contract and is not presented as equivalent to normal opening. The gzip baseline is a reusable full-list loader; a one-shot streaming lookup is not measured.

## Retrieval with random ranks prepared in advance

These isolate lookup/copy/checksum work from RNG cost. They must not be confused with the complete sampling results above.

| Method | Median ns/lookup |
| --- | ---: |
| Loaded vector | 34.903 |
| Namepack raw | 37.730 |
| Namepack compact | 59.984 |
| SQLite prepared read transaction | 369.620 |
| SQLite prepared autocommit | 2773.021 |

## Environment and integrity

- CPU exposed by host: AMD EPYC 9V74 80-Core Processor.
- Compiler: rustc 1.100.0 (0ef1bc386 2026-11-09).
- SQLite: 3.46.1; JSON-C: 0.18; zlib: 1.3.1.
- Profile: release library; rustc -O harness; no target-cpu=native.
- Dataset SHA-256: `a9c432e163d94e066fc068020f249b333f4ff1703298ecacc13e579bcbeff396`.
- Complete-byte checksum for every timed batch: `842a4e00e57f849e`.
- Measured core source-manifest SHA-256: `5fdf8cfe03f45b00f6942f488eae282af3628df246df98488a093f055c6a3b5f`.

The benchmark reconstructs every rank of the compact and raw packs, validates their UTF-8 and uniqueness, and compares both exact sets with the input. All measured methods return the same name sequence and verify the same complete-byte checksum. Method order rotates between rounds. SQLite uses dense integer primary keys, a reused prepared statement, a read-only connection, NOMUTEX, and a persistent transaction for the strongest warm comparison. Transaction/snapshot setup is outside the timer. JSON and gzip use lexical order; rank translations are outside timing.

The fixture is 29,329 actual unique given names from the SSA 2024 national file, after exact deduplication across the sex categories. SSA suppresses some low-frequency names. This is an ASCII-dominated US given-name corpus; it is not a Japanese or surname performance dataset.

## Run the comparison on current pkbyte

```bash
python3 benchmarks/run.py --iterations 100000 --rounds 9 --startup-rounds 9
```

The command saves a fresh report in `benchmarks/out/ssa2024/` and checks that source files remain unchanged during measurement. The archived Namepack report retains every timing batch and its original source hashes. [Benchmark setup and methods](../benchmarks/README.md) · [Historical report](../benchmarks/results/ssa2024.json) · [Dataset provenance](../data/PROVENANCE.md)

## Not yet established

- Compression or speed superiority over FSST, OnPair, MARISA or front coding.
- Japanese-name or surname performance.
- Performance at millions of records, on phones, or in a browser.
- Cold-disk access, memory-pressure behavior, or peak/resident memory.
- Per-request tail latency or sustained multi-threaded throughput.
- Previously unpublished scientific novelty or globally optimal compression.

These are the next evidence requirements before selecting a long-term format and making broader performance claims.
