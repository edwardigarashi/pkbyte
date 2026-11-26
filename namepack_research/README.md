# Namepack: next-generation experiments

Research snapshot, 2026-11-25. These experiments investigate exact uniform
sampling of names with the original Namepack 0.1.0 library. The captured results
and `compile_hotpath/experiment_manifest.json` describe that original source and
layout. The experiments remain separate from the pkbyte library and CLI.

## What compilation can improve

The original measurements already used optimized native Rust with thin LTO and
one code-generation unit. Merely embedding the `.npk` file with `include_bytes!`
does not compress its bytes further. It removes a separate file read; ordinary
reader construction still performs its documented validation and parsing.

A useful dataset compiler would instead validate the complete dataset during a
build, choose its representation, and generate compact static tables and only
the decoder paths needed for that dataset. Generated code, indexes, alphabets,
and dictionaries must all count toward the deployment footprint. Native code
is target-specific; the portable byte representation can remain shared.

The prepared-reader experiment measures the runtime layout such a compiler
could emit, but constructs that layout at startup. It does not yet implement
a complete ahead-of-time name-table generator.

## Measured directions

### Faster access to the same file

`compile_hotpath/src/main.rs` compares the original reader with a prepared
reader using the same 106,230-byte file. It uses dense 8-byte phrase arrays,
fixed-size phrase copies, a small rank-to-group hint directory, reusable output,
and an optional precomputed multiply-high unbiased range reducer.

The experiment uses safe Rust and retains UTF-8 validation of each returned
name. Preparation performs exhaustive decoding before the timing loop. Every
rank is compared with the original reader. The prototype assumes a nonempty
validated table with at most 256 groups; these assumptions need checked public
constructors before production use.

The initial recorded complete-sampling medians are 62.700 ns for the original
modulo sampler, 34.694 ns for prepared phrase decoding with the same modulo
sampler, and 29.877 ns for prepared decoding with the multiply-high sampler.
These include RNG, decoding, output, and a checksum of every returned byte.
The two range reducers produce different valid uniform sequences; checksums
are compared within each reducer's matching-sequence group. The original
decoder with the multiply-high reducer measured 59.535 ns.

See `compile_hotpath/results.json` for the complete batch measurements. These
are medians of batch averages, not individual-request latency percentiles.

### Smaller exact sets

`compression/elias_fano.py` maps names of each byte length to invertible
integers, then stores the sorted unique integer set using Elias–Fano. It
includes position-specific byte alphabets, file framing, CRC-32, low/high bit
arrays, and a select checkpoint index. All 29,329 names are exhaustively
reconstructed and compared as an exact unique set.

Selected complete file sizes:

| Representation | Bytes |
| --- | ---: |
| Original compact namepack | 106,230 |
| Mixed-radix Elias–Fano, checkpoint every 64 names | 71,126 |
| Mixed-radix Elias–Fano, checkpoint every 32 names | 72,966 |
| Power-of-two-radix Elias–Fano, checkpoint every 64 names | 77,190 |
| Prior lexically sorted gzip-9 text reference | 72,985 |

The original table already discards input order, but encodes each name
independently. The new representation explicitly compresses an unordered set.
It stores only the integers that correspond to actual names. Drawing a uniform
rank and selecting that stored integer cannot invent an absent name.

The mixed-radix representation uses tighter integer ranges but needs division
or equivalent arithmetic during reconstruction. The power-of-two variant
spends additional bits to use shifts and masks. Generated per-group constant
divisors could provide another useful tradeoff; that AOT variant is unmeasured.

`compile_hotpath/run_ef.py` runs the native comparison. See its result JSON and
source-hash manifest for the final observations and methodology. This second
comparison prepares matching name/rank sequences outside the timing loop; its
lookup figures must not be confused with complete-sampling figures above.

Final same-name warm retrieval medians:

| Representation | Nanoseconds per name |
| --- | ---: |
| Original reader | 58.128 |
| Prepared phrase reader | 27.133 |
| Mixed-radix EF, checkpoints every 64 | 54.725 |
| Mixed-radix EF, checkpoints every 32, reciprocal arithmetic | 54.609 |
| Power-of-two EF, checkpoints every 64 | 49.169 |

The mixed-radix EF64 reader with a hand-written reciprocal measured 56.525 ns,
slower than its ordinary division variant in this run. Both new representations
are smaller than the original format; the fastest prepared phrase reader is
still faster than either new set representation. No combined ahead-of-time
compiler/EF result has been measured.

## Research ideas to pursue next

1. Apply sorted-set compression to fixed-width **phrase-code integers** from
   the original format. This may combine substring sharing with set redundancy
   while retaining short phrase expansion. This idea is unmeasured.
2. Partition by length or a small prefix when the integer universe is too large
   or sparse. Keep original phrase/raw formats as fallbacks. Count every extra
   group descriptor, alphabet, and select index.
3. Generate compact per-group decoders with constant radices, bounds, and
   offsets. Specialize a small number of layouts rather than emitting one
   function or match arm for every name.
4. Select a codec per disjoint group under an explicit bytes/latency budget.
   Uniform sampling gives a group probability of `group_rows / total_rows`.
   Weighted average latency is an initial cost model; validate whole-program
   cache interactions and tail behavior separately.
5. Compare exact prefix/suffix incidence factoring and counted acyclic
   automata where regular name families make their metadata worthwhile.

The Huffman experiment is also included. Grouping records by exact encoded
bit length preserves direct addressing even with variable-length symbol
codes. Its 100,006-byte figure is a proposed-container accounting result from
verified payloads; it is not a complete standalone file or a measured native
decoder. The result JSON labels that distinction explicitly.

## Robustness before integration

- Preserve exact UTF-8 bytes and uniqueness; never silently normalize names.
- Require each name to have exactly one rank across all codec partitions.
- Use unbiased bounded randomness and document sampling with replacement.
- Retain a fully checked reader for externally supplied data.
- Add checked integer overflow, empty-table, truncation, malformed select-index,
  and invalid-alphabet handling to the research formats before public use.
- The current integer codec requires an invertible value fitting `u64` and
  finite format fields; it needs partitioning or a fallback for longer names.
- Select checkpoints have input-dependent scan work. Measure p95/p99 and
  cold-cache behavior before claiming bounded production latency.
- Measure multilingual names, surnames, larger corpora, ARM64 and WebAssembly.
- Count serialized bytes, generated code, allocated/resident memory, startup,
  warm latency, and throughput as separate metrics.

The original project's validation report does not automatically validate these
new experimental parsers. The Python Elias–Fano loader checks CRC; the native
research reader consumes producer-verified fixtures and is not a hardened
parser for untrusted input.

## Reproduction

Run these commands from the pkbyte checkout with Python 3.10+ and Rust installed.
The Rust comparisons use the current pkbyte library and the archived fixtures.

```sh
python3 namepack_research/compression/elias_fano.py
python3 namepack_research/compression/analyze.py
cargo run --release --offline --locked --manifest-path namepack_research/compile_hotpath/Cargo.toml
python3 namepack_research/compile_hotpath/run_ef.py
```

The complete-sampling comparison prints its report to standard output. The
Python scripts save new outputs under `namepack_research/out/`. The archived
reports and fixtures remain available in their original locations. This corpus
represents ASCII-dominated US given names. [SSA fixture provenance](../data/PROVENANCE.md)

## Prior art and novelty

These are combinations and workload-specific implementations of known ideas,
not a claim of a previously unpublished compression algorithm.

- Rust embedding and build-time generation:
  <https://doc.rust-lang.org/std/macro.include_bytes.html>
  <https://doc.rust-lang.org/cargo/reference/build-scripts.html>
- FSST: <https://vldb.org/pvldb/vol13/p2649-boncz.pdf>
- OptFSST, including optimal DP parsing:
  <https://arxiv.org/abs/2607.11271>
  <https://github.com/Hedi-Chehaidar/optfsst>
- Lemire's unbiased range reduction:
  <https://arxiv.org/abs/1805.10941>
- Elias–Fano and select:
  <https://vigna.di.unimi.it/ftp/papers/QuasiSuccinctIndices.pdf>
  <https://docs.rs/sux/latest/sux/dict/elias_fano/>

The research opportunity is an exact-sampling dataset compiler that makes
representation choices using measured size and latency, with transparent
fallbacks and robust format validation.
