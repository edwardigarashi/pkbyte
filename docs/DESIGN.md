# pkbyte design and string contract

## String contract

A compiled table is an immutable set of unique UTF-8 strings. Preserve each
input byte exactly, including embedded newlines, tabs, NUL, whitespace-only
strings, emoji, and multilingual text. Trimming, normalization, case folding,
splitting, and other content transformations require the application to act
before calling the builder. The empty string is a value. An empty table has no
values and sampling returns `EmptyTable`.

Use exact byte equality and reject duplicates by default. Explicit deduplication
keeps one copy of each value. Sample uniformly with replacement using the
application's random source. A rank addresses one value within one compiled
table and can change on rebuild. Apply explicit, configurable resource limits
and support long values through a suitable codec or fallback within those
limits. Once prepared, the table supports repeated allocation-free sampling
when the application provides sufficient reusable output storage.

## Current behavior

| Contract area | Implementation |
| --- | --- |
| Exact strings | The library preserves the bytes of every accepted UTF-8 string. The CLI's JSON Lines mode escapes embedded line delimiters and controls and preserves empty strings. Plain line mode accepts nonempty LF/CRLF-delimited records. Plain sampling rejects values that would be ambiguous in this framing. |
| Empty string and empty table | Versions 2 and 3 represent the empty string as one zero-width raw record. A zero-row table has no records and sampling it returns `EmptyTable`. The reader also accepts version 1, whose records all have positive widths. |
| Unique values | Exact duplicate input returns `DuplicateValue` by default, including a repeated empty string. `DuplicatePolicy::Deduplicate` and `--deduplicate` opt into removal. External files require semantic validation with uniqueness enabled to establish set semantics. |
| Sampling, ranks, and randomness | `sample_into` draws an unbiased rank with replacement from caller-supplied independent uniform `u64` values. Built tables and fully validated unique external tables sample uniformly over their values. A checked external reader alone samples over rows and propagates selected-record decoding errors. Ranks are local to one compiled table. |
| Configurable limits and long values | Build options bound input rows, input bytes, value length, and dictionary training. Open options bound file bytes, rows, groups, and decoded lengths. The per-value limit defaults to 4,096 bytes. All modes store values longer than the phrase eligibility cap as raw UTF-8 when the build and open limits allow them. The CLI exposes per-value, input, row, group, file, and decoded byte limits as applicable. |
| Reusable sampling output | The checked reader reuses a `Vec<u8>` with capacity at least `Table::max_value_bytes()`. The prepared reader takes an initialized slice with length at least `PreparedTable::output_buffer_len()`. Both support allocation-free repeated access with sufficient storage. |

## Public API and ownership

`Picker` owns compressed bytes, a fully validated index, a noncryptographic
SplitMix64 state, and scratch for owned picks. Construction opens or builds
the table once and shares preparation and decoding with `PreparedTable`.
Validated Elias–Fano metadata stores payload offsets, so the owner keeps its
bytes and index without a self-reference. `pick()` returns an owned copy.
`pick_into` grows an external vector on demand and borrows the decoded prefix.
The borrowed `Table` and `PreparedTable` remain available for embedding,
shared immutable state, and caller-controlled random sources.

`build` accepts an iterator of UTF-8 values and returns the owned portable
bytes in `BuildOutput` with encoder statistics. Use `BuildOptions` to choose
the duplicate policy, limits, and encoding mode. `build_support::build_from_reader`
applies LF/CRLF or JSON Lines framing before building the same table. Invalid
framed records report their one-based physical line. [Embedding and Cargo generation](INTEGRATION.md)

`Table::open` borrows a byte slice and allocates parsed metadata. Keep those
bytes alive and immutable. A slice can come from an owned byte buffer,
`include_bytes!`, or a Cargo-generated module. `get_raw` borrows a checked raw
value from the file. `get_into` borrows its decoded value from the caller's
scratch, and `get` returns an owned string. Open with custom `Limits` when the
input needs tighter constraints or the builder used larger limits. The
defaults admit at most 257 MiB of file bytes, one million rows, 65,536 groups,
256 MiB of decoded bytes, and 4,096 bytes per value.

`validate_all(true)` checks decoded metadata and uniqueness for an external
table. `prepare` performs the same checks and constructs retained indexes
over the borrowed table for repeated access. Keep the checked table alive
while prepared state is in use. Use independent buffers and random sources
when sharing a prepared table across threads. [Prepared setup and storage](#prepared-access)

The public `Error` variants distinguish invalid build options, duplicates,
resource limits, unsupported or malformed files, checksum failures, invalid
UTF-8, and I/O failures. Rank access can return `OutOfBounds`, and prepared
access can return `OutputTooSmall`. Sampling an empty table returns
`EmptyTable` before asking the application for randomness.

## Storage selection

`BuildMode::Compact` compares complete files in this order. Raw comes first,
followed by the bounded phrase candidates, mixed-radix Elias–Fano, and
power-of-two Elias–Fano. Equal sizes keep the earlier candidate.
`BuildMode::Phrase` compares only raw and the phrase candidates. The two
explicit Elias–Fano modes exclude phrase encoding. `BuildMode::Raw` writes the
raw layout directly.

Both Elias–Fano candidates partition the set by original byte length. Each
partition uses its complete block or stays raw, based on which uses fewer
bytes. The directory costs the same either way. Each Rust block stores only
integers that encode actual input values. Reader limits and validation apply
to all codecs. Building the two radix variants can cost additional time, and
compact mode optimizes serialized size over the evaluated candidates.
Codec choice and ranks can change on rebuild.

### Elias–Fano

The builder computes a byte alphabet for each position. A position's alphabet
size defines its radix, or the radix rounds that size up to a power of two for
shift-and-mask decoding. Every byte becomes a digit. Equal-width strings
map in lexical byte order to strictly increasing integers. The codec keeps
the integer set and its reconstruction information together. Empty values,
singleton groups, widths beyond 64 bytes, overflowing radix products,
excessive checkpoint spans, and blocks without a size saving use the raw
layout in these candidates. The format also permits well-formed singleton
Elias–Fano blocks.

The reader checks the structure of the Elias–Fano arrays during opening.
It validates the exact ordinals of serialized checkpoints and caps the
distance from a checkpoint to any row it covers at 2,048 bits. Lookup
can inspect at most 33 high words and clear at most 63 set bits in the
selected word. Reconstruction uses one alphabet per byte, up to 64 bytes.
The parser checks the alphabets, arrays, and checkpoints in place. It retains
a small descriptor of validated payload offsets per Elias–Fano group. The
caller uses the same reusable output API, and prepared sampling shares
the checked block metadata. Only phrase decoding needs extra scratch padding.
[Version 3 block layout](FORMAT.md#version-3-and-eliasfano-blocks)

## Phrase encoding

1. Observe the byte alphabet of nonempty values up to 4,096 bytes. Keep the
   empty string and longer values raw.
2. Give every observed byte a one-byte symbol code while training.
3. Learn frequent adjacent pairs on a bounded deterministic sample, flattening
   each resulting phrase to at most eight original bytes.
4. Evaluate up to four dictionary checkpoints plus the complete raw candidate.
5. For each fixed dictionary, use dynamic programming to minimize the number of
   emitted symbol bytes for each eligible value. Use the raw bytes unless the encoding
   is strictly shorter.
6. Remove dictionary symbols that no compressed row uses and remap their codes.
7. Group records by codec and encoded length, serialize each candidate completely,
   and select the smallest complete file.

The builder knows the literal byte alphabet, so the dictionary needs no escape
codes. The final dictionary may omit literals used only in raw records. All
dictionary entries are flat expansions. Each value decodes independently.

## Training and fixed-dictionary parsing

Training repeatedly counts realizable non-overlapping adjacent pairs. For a pair
whose flattened expansion has length l and appears f times in the current sample,
the estimated gain is `f - (l + 1)`. Each replacement saves one code byte, and
the new dictionary entry takes `l + 1` bytes. Training uses this estimate to
choose a pair. Complete-file model selection measures the actual byte count.
The pair must fit the maximum phrase length and expand to a distinct byte
sequence. Ties are resolved deterministically.

The pair `aa` can be replaced twice in `aaaaa`. For equal-symbol pairs the
implementation counts runs and uses `floor(run_length / 2)`. Occurrences of a
pair of distinct symbols cannot overlap each other. Training stops when no
positive estimated gain remains or the symbol budget is full.

Given a fixed dictionary D and an input x of n bytes, the parser uses this
recurrence.

```
cost[n] = 0
cost[i] = min(1 + cost[i + len(s)])
          for every dictionary symbol s matching x at position i
```

Each symbol costs one encoded byte, so this minimizes encoded length for that
dictionary. With phrases `abc`, `ab` and `cde`, the optimal parse of `abcde`
uses `ab | cde`.

The builder searches a bounded set of dictionary candidates. A locally shorter
record can create another group or require a symbol whose overhead exceeds its
benefit. Complete-file comparison against the raw candidate accounts for that
overhead among the candidates tried.

## Exact file objective

For a candidate with dictionary bytes D and K groups, the complete file size
uses this expression.

```
64 + D + 16*K + sum(group_payload_bytes)
```

The checksum is in the header. A fixed-stride group consumes its encoded width
times its count. An Elias–Fano group consumes the full block size, including
its alphabets, word padding, and checkpoints.

The raw candidate guarantees the following bound for the returned compact file.

```
compact_file_bytes <= raw_binary_file_bytes
```

The bound applies to pkbyte's complete raw layout.

## Direct addressing and probability

Let C_g be the cumulative number of rows before group g. Select group g for rank r
when `C_g <= r < C_g + n_g`. Use the following address for the encoded record.

```
payload_base_g + (r - C_g) * w_g
```

This address calculation applies to raw and phrase groups. Elias–Fano groups
resolve the local rank through their select index and decode its integer.
The reader decodes the selected record independently. A uniform r in `[0,N)` is
uniform over stored rows because the intervals partition those N ranks exactly.
Each value occurs once in a unique table, which makes the distribution over
values uniform as well.

For a uniform 64-bit source, reject draws below `2^64 mod N`, then take modulo N.
The accepted interval contains a multiple of N source draws. For N > 0 the
acceptance probability is greater than one half. An application source must supply
independent uniform `u64` values for exact uniformity and almost-sure termination.

## Complexity and footprint

The reader's group search is O(log K). Reconstruction is proportional to the
encoded and decoded lengths of the selected value. `get_raw` can borrow raw bytes
as a checked `&str`. Full semantic verification scans every row. The optional
uniqueness check retains a set with memory proportional to the decoded corpus.

The builder retains source values and candidate encodings. The number and size
of training values are bounded, with a 256-symbol ceiling. Minimum-token parsing
checks matching first bytes with scratch space proportional to the value length.
The 4,096-byte eligibility cap bounds this work per value.
Resource limits bound input and declared sizes. Peak process memory also includes
retained values, candidates, training state, and optional uniqueness storage.

## Reader trust boundaries

**External bytes and the checked reader.** `Table` borrows immutable bytes and
checks file and section sizes, counts, directory coverage, dictionaries, and
limits independently of the input's claims. The default open also verifies
CRC-32 for accidental corruption. An explicitly structural open skips the CRC
pass. Both perform selected-record bounds, token, decoded size, and UTF-8
checks. A malformed selected record produces an error and is never resampled.
`validate_all(true)` checks CRC-32 and every decoded value, recomputes the decoded
row count, byte total and maximum, and verifies exact uniqueness. It returns a report.
File authenticity requires separate verification.

**Prepared state.** `Table::prepare` calls `validate_all(true)` on the same
table whose immutable payload and parsed groups the prepared reader borrows.
Structure proves group and rank coverage. Exhaustive validation establishes
dictionary references, exact decoded lengths, UTF-8, and uniqueness. Only
this checked constructor can create the prepared reader. Opening structurally
and immediately preparing runs the CRC pass once. The core forbids unsafe code.

**Research readers.** The prepared phrase prototype validates its benchmark
table's records without checking uniqueness, then reparses separately supplied
bytes. It assumes a nonempty table, at most 256 groups, and padded scratch. The
original Elias–Fano experiments use their own producer-verified artifacts.
The pkbyte library implements the checked version 3 layout.
The Huffman experiments serialize verified payloads with metadata accounted
for separately. These readers and formats live outside the default pkbyte
library and CLI. The research snapshot is under `namepack_research/` in the
checkout.

On-disk fields use fixed-width little-endian integers. The reader borrows an
immutable slice. Applications that supply one through a memory map must ensure
the backing data stays valid and immutable for the reader's lifetime. See the
[format specification](FORMAT.md).

## Prepared access

The prepared reader reuses group offsets or borrowed Elias–Fano descriptors
to address one value. Raw values are copied directly, including the zero-byte
empty value. Phrase
expansion uses 256 eight-byte arrays and 256 one-byte lengths. It copies
eight bytes per phrase and advances by the actual phrase length. Every
phrase contributes at least one logical byte, so seven extra bytes cover
the final copy. `new_output_buffer()` initializes the required length.
An existing slice must have sufficient length. The reader checks UTF-8
on the logical prefix before returning it.

The hint index stores one four-byte group ID for the first rank in each
power-of-two bucket, growing the bucket width to respect the budget.
A lookup first probes that group and, if needed, binary-searches through
the group at the start of the next bucket. The last bucket ends at the end
of the directory. With less than eight bytes of budget or fewer than two
groups, lookup uses the existing full-directory binary search.
Group IDs can represent the on-disk group count even above 256 groups.

Preparation's exhaustive uniqueness check temporarily retains the
decoded values, an ordered set with one entry per value, and one decode
buffer. The prepared reader adds 2,304 retained heap bytes for phrase
tables and at most the requested hint budget. It borrows both the
compressed payload and the checked reader's group metadata. Each caller's
buffer holds the maximum decoded value length plus seven bytes if phrases
are present. These sizes exclude allocator overhead. Independent callers
can share the immutable prepared state with separate buffers and RNGs.

## Prepared range reduction

The prepared sampler computes the rejection threshold `2^64 mod N` once
for nonempty tables. The default takes the high half of the product of
the draw and N, rejecting when the low half is below the threshold.
For each possible high half, the accepted portion of its product interval
has a length divisible by N. Products are spaced by N, so the accepted
draw count is identical for every rank. The implementation uses portable
integer arithmetic. An empty table returns before any draw.

Prepared modulo mode matches checked modulo sampling for the same table
and RNG stream. Multiply-high preserves exact uniformity and can yield a
different seeded sequence. `Picker` and the CLI use the prepared modulo
sampler and preserve the established seeded sequences.

## Prior work

- Philip Gage describes byte-pair substitution. [A New Algorithm for Data Compression (1994)](https://www.derczynski.com/papers/archive/BPE_Gage.pdf)
- Boncz, Neumann and Leis describe static short-symbol tables and independently
  decodable strings. [FSST (2020)](https://ir.cwi.nl/pub/32922/)
- Gargiulo and Venturini describe adjacent-substring learning and independent
  short-string compression. [OnPair (2025)](https://arxiv.org/abs/2508.02280)
- Compact static dictionary with key-ID reverse lookup. [MARISA](https://www.s-yata.jp/marisa-trie/docs/readme.en.html)
- Front coding and other compact dictionary alternatives. [Practical Compressed String Dictionaries](https://rcanovas.github.io/publication/pcsd/)

