# Compression experiments for Namepack

These isolated experiments archive the original work on Namepack v0.1.0. Python
is used for construction, exact accounting, and exhaustive verification. The
pkbyte reader uses its own legacy format parser.

Run from any working directory with Python 3.10+:

```sh
python3 /path/to/bundle/namepack_research/compression/analyze.py
python3 /path/to/bundle/namepack_research/compression/elias_fano.py
```

The scripts find the 29,329-name SSA 2024 fixture and its v0.1.0 phrase encoding
in the repository's `data/` directory. Fresh files and reports go to
`namepack_research/out/compression/`. [SSA fixture provenance](../../data/PROVENANCE.md)

## Fixed-bit-length Huffman groups

`analyze.py` trains a canonical Huffman code over the existing phrase-token
frequencies, groups records by exact encoded bit length, and concatenates records
without rounding each record to a byte. Group-level bitstreams are byte-rounded.
Addressing remains `group_bit_start + rank_in_group * bits_per_record`.

The first experiment preserves tokenization; the second reparses each name with
dynamic programming to minimize its bit cost under that same Huffman model.
Raw names remain a candidate. A model built from the old tokenization is not
globally optimal after reparsing; no optimality is claimed.

| Variant | Actual payload bytes | Proposed complete bytes | Groups |
| --- | ---: | ---: | ---: |
| Existing v0.1.0 file | 105,110 | 106,230 | 16 |
| Huffman, existing tokens | 98,451 | 100,723 | 72 |
| Huffman, bit-cost DP reparse | 97,782 | 100,006 | 69 |

Both Huffman payloads were built and every name decoded exactly. The proposed
complete size additionally accounts for the existing 64-byte header, 800-byte
phrase dictionary, 256 one-byte Huffman code lengths, and 16 bytes per group.
Those metadata sections were **not serialized as a complete new container**.
Huffman decoding latency has not been measured. More groups and bit decoding
can offset the storage improvement.

## Mixed-radix integers plus Elias–Fano

`elias_fano.py` implements a different exact-set representation. Names are grouped
by original UTF-8 byte length. Each byte position has its own sorted alphabet.
The position alphabets make a name an invertible mixed-radix integer. The sorted
integers are stored with Elias–Fano low/high bitvectors. A sampled select index
records the location of every 16th, 32nd, 64th, 128th, or 256th set bit.

The group stores its minimum integer separately and subtracts that base before
encoding. Retrieval uses:

```text
rank = unbiased_random_integer(number_of_names)
group, local_rank = locate_count_interval(rank)
upper = select1(group.high_bits, local_rank) - local_rank
value = group.base + (upper << group.low_width) + low_bits[local_rank]
name = inverse_mixed_radix(value, group.position_alphabets)
```

`select1` is zero-based. The mapping between stored names and global ranks is
bijective, so a uniform global rank produces a uniform stored name. No names are
generated outside the input set. The known algorithms are Elias–Fano and invertible
radix encoding; this experiment does not establish a new invention.

This exploits uniqueness and the absence of meaningful row order. For `n` distinct
integers from a universe of size `U`, the set has `binomial(U, n)` possibilities,
while an ordered sequence has extra ordering information. The current phrase
format discards row order semantically but does not encode this set redundancy.

### Actual complete file sizes

All sizes include the entire header, directory, position alphabets, low bits,
high bits, checkpoint index, and bitvector padding. Every produced file was opened
again and all 29,329 names exhaustively recovered, including uniqueness checks.

| Checkpoint interval | Mixed-radix bytes | Power-of-two radix bytes |
| ---: | ---: | ---: |
| 16 | 76,618 | 82,682 |
| 32 | 72,966 | 79,030 |
| 64 | 71,126 | 77,190 |
| 128 | 70,218 | 76,282 |
| 256 | 69,762 | 75,826 |

Mixed radix packs tighter. Power-of-two radix rounds each position's universe to
a power of two, so inverse decoding can use shifts and masks instead of division.
That leaves unused integer codes, but none is stored or sampled. Alphabet tables
contain only actual bytes, and decoding still returns the exact stored names.

For mixed radix with checkpoint interval 64, the complete breakdown is 64 bytes
header, 672 bytes directory, 2,194 bytes alphabets, 57,728 bytes low bits, 8,608 bytes
high bits, and 1,860 bytes checkpoints. Select scanned a mean 2.151 64-bit words,
with p99 7 and maximum 14 on this corpus. These are operation counts, not latency.

### Research binary layout

All fixed-width integers are little-endian. The 64-byte header contains:

| Offset | Field |
| ---: | --- |
| 0 | 8-byte magic `NAMESEF` plus NUL |
| 8 | version `u32`, currently 1 |
| 12 | group count `u32` |
| 16 | total row count `u64` |
| 24 | total decoded byte count `u64` |
| 32 | payload byte count `u64`, excluding header/directory |
| 40 | IEEE CRC32 `u32`, over entire file with this field zeroed |
| 44 | 20 reserved zero bytes |

Each 48-byte directory record has Python struct format `<HBBIQIIIIIIQ`:

| Offset | Field |
| ---: | --- |
| 0 | original byte length `u16` |
| 2 | low-bit width `u8` |
| 3 | log2 checkpoint interval `u8` |
| 4 | row count `u32` |
| 8 | base integer `u64` |
| 16 | alphabet section bytes `u32` |
| 20 | low-bit section bytes `u32` |
| 24 | high-bit section bytes `u32` |
| 28 | checkpoint count `u32` |
| 32 | high-bit vector logical bit length `u32` |
| 36 | mode `u32`: 0 mixed radix, 1 power-of-two radix |
| 40 | reserved zero `u64` |

The body follows all directory entries. Each group stores its alphabets, then
low-bit vector, high-bit vector, and checkpoints. Each alphabet is a `u16` length
followed by its bytes. Bitvectors use least-significant-bit-first packing inside
little-endian words, padded to whole 64-bit words. A checkpoint is a `u32` bit
position. A reader derives all group section offsets from this directory.

### Limits and next work

- This prototype requires each encoded name integer to fit in `u64`. It explicitly
  asserts this during construction. Long strings and large multilingual alphabets
  need partitioning, wider integers, or fallback to the existing byte codec.
- Byte alphabets preserve Unicode bytes exactly; results on this short, mostly
  ASCII dataset do not establish compression or timing for other languages.
- This is not an adversarial-input-hardened parser. Before library integration it
  needs checked arithmetic, resource limits, canonical format rules, malformed
  input tests, fuzzing, and verification of all index invariants.
- Periodic select checkpoints alone do not give a universal constant worst-case
  lookup bound. Large gaps can require scanning more words. An adaptive select
  index or bounded-span fallback can improve tail behavior.
- The supplied Python reader is a correctness oracle, not a speed implementation.
  Native Rust measurements, where available, are in the sibling hot-path experiment.
- A useful untested extension is applying the set representation to the existing
  fixed-width phrase-code sequences, combining substring and set redundancy.

## Other measured structure

The exact names form a 58,272-state byte trie. Bottom-up minimization yields a
10,416-state acyclic deterministic automaton with 30,437 arcs. Numbered states can
support exact uniform unranking; all names were recovered in lexical order.
No compact serialized automaton was built. Node/edge/count overhead must be counted
before claiming it will beat either file representation.

## Primary related work

- Vigna, *Quasi-Succinct Indices*:
  https://vigna.di.unimi.it/ftp/papers/QuasiSuccinctIndices.pdf
- Vigna's Rust Elias–Fano implementation:
  https://docs.rs/sux/latest/sux/dict/elias_fano/
- *Compressed String Dictionaries via Data-Aware Subtrie Compaction*:
  https://pages.di.unipi.it/vinciguerra/publication/compressed-string-dictionaries-via-data-aware-subtrie-compaction/compressed-string-dictionaries-via-data-aware-subtrie-compaction.pdf
- Daciuk et al., *Incremental Construction of Minimal Acyclic Finite-State Automata*:
  https://aclanthology.org/J00-1002.pdf
