# pkbyte table format

## Compatibility and versioning

The Cargo package version identifies the Rust package and API. The table
header carries a separate wire version under the original `NAMPACK\0` signature.

| Wire version | Read support | Write support |
| --- | --- | --- |
| 1 | Legacy raw and phrase tables with nonempty values | Read-only |
| 2 | Raw and phrase tables with an optional empty value | Emitted for raw and phrase tables |
| 3 | Version 2 layouts and both Elias–Fano codecs | Emitted when the selected layout contains an Elias–Fano block |

Valid older files retain their interpretation and remain readable within the
configured limits. The reader reports `InvalidMagic` for an unrecognized
signature and `UnsupportedVersion` for an unrecognized version. Unknown
flags, reserved bits and codecs outside the selected version's grammar are
format errors. A change that alters existing byte meanings or adds features
that existing readers cannot interpret requires a new wire version.

To retain an existing table, open and verify it using the current reader. To
rebuild it with the current writer, export exact strings as JSON Lines.

```sh
pkbyte verify old.npk
pkbyte unpack old.npk | pkbyte pack - -o rebuilt.npk --format jsonl
pkbyte verify rebuilt.npk
```

With a fixed writer implementation and build options, the same set of values
produces identical bytes regardless of input order. When deduplication is
enabled, the output depends on the distinct values. Writer changes or
different options may change the bytes and ranks. Keep the compiled table,
RNG algorithm and seed, and range-reduction method fixed for reproducible
seeded sampling. Ranks address values within one compiled table. Every draw
is with replacement, so successive samples may repeat even in a unique table.
[Frozen compatibility fixtures](../tests/fixtures/README.md)

## Container layout

All integers are unsigned and little-endian. Sections occur in order as a
64-byte header, a dictionary, a group directory, and a payload. The sections
cover the entire file with no trailing bytes. Original input order is
discarded. The builder emits unique strings under exact UTF-8 byte equality.

### Header

| Offset | Bytes | Field |
| --- | --- | --- |
| 0 | 8 | ASCII `NAMPACK` followed by NUL |
| 8 | 2 | Version, one of 1, 2, or 3 |
| 10 | 2 | Flags = 0 |
| 12 | 4 | Header length = 64 |
| 16 | 8 | Row count, including an empty value if present |
| 24 | 8 | Total decoded UTF-8 bytes |
| 32 | 4 | Maximum decoded value length, or zero if there are no nonempty values |
| 36 | 2 | Dictionary symbol count, 0 through 256 |
| 38 | 2 | Reserved = 0 |
| 40 | 4 | Group count |
| 44 | 4 | Dictionary section byte length |
| 48 | 8 | Payload byte length |
| 56 | 4 | IEEE CRC-32 over entire file with bytes 56 through 59 zeroed |
| 60 | 4 | Reserved = 0 |

### Dictionary

Each symbol is a one-byte length from 1 through 8 followed by that many raw
bytes. A symbol code is its zero-based position, at most 255. Symbols have
distinct, flat byte expansions. A symbol can contain part of a UTF-8 code
point. Each complete decoded value must be valid UTF-8.

### Group directory and payload

Each 16-byte group entry contains a codec byte, three zero reserved bytes, a
width (`u32`), and a row count (`u64`). Codec 0 stores raw UTF-8 bytes. Codec 1
stores a stream of dictionary codes and requires a dictionary. If no group
uses codec 1, the dictionary is empty. Version 3 also accepts codecs 2 and 3
as described below.

Group counts are positive and sum to the header's row count. Keys are
strictly ordered by codec and then width. Width is at most the declared
maximum decoded length. Group sections appear in directory order and
together consume the declared payload exactly.

For codecs 0 and 1, width is the encoded record length and a group consumes
its width times its count in bytes. For rank r, find the group whose
cumulative count interval contains r. A fixed-width record begins at
`group_start + (r - first_rank) * width` and occupies exactly `width` bytes.
These groups contain consecutive records without delimiters, offsets, or
padding.
A raw-only table's decoded byte total equals its payload length and its
decoded maximum equals its greatest width, or zero if it has no groups.

### Canonical empty table

Each version represents an empty table with the 64-byte header alone. Row
count, decoded total and maximum, symbol and group counts, and section lengths
are zero. The header length is 64 and the CRC-32 is valid. Sampling returns
`EmptyTable`.

### Version 1

All group widths are positive. Every decoded value is nonempty. The decoded
byte total is at least the row count, decoded maximum, and payload length. It
is at most the row count multiplied by the decoded maximum.

### Version 2

An empty value uses codec 0, encoded width zero, and count one. With strict
group ordering this entry occurs at most once, first in the directory, and
addresses rank zero. It contributes one row and zero bytes to the payload
and decoded total. All other groups have positive widths.

A singleton empty value has a 64-byte header and one 16-byte directory entry,
with row and group counts one, decoded total and maximum zero, and no
dictionary or payload. This is distinct from the empty table. For all version
2 tables the decoded byte total is at least the number of nonempty rows,
decoded maximum, and payload length. It is at most the number of nonempty
rows multiplied by the decoded maximum.

### Version 3 and Elias–Fano blocks

Version 3 keeps version 2's empty-value and fixed-width group rules. The reader
accepts v3 layouts that contain only raw or phrase groups. Codec 2
uses mixed radices and codec 3 uses the next power of two at each byte
position. For these codecs, the directory width is the decoded UTF-8 byte
length from 1 through 64. Each group occupies one block of variable length.
The group count gives the integer count. Integers must be below the product
of the position radices, which must fit in `u64`.

The block begins with this 24-byte header.

| Offset | Bytes | Field |
| --- | --- | --- |
| 0 | 8 | Base, the smallest stored integer |
| 8 | 4 | Complete block bytes, including the header |
| 12 | 4 | Alphabet section bytes |
| 16 | 4 | Logical bit count of the high bit vector |
| 20 | 1 | Low field width, 0 through 63 |
| 21 | 3 | Reserved = 0 |

After the header come the alphabets, low fields, high bit vector, and
checkpoints, in that order. Their sizes must cover the block exactly.

- **Alphabets.** One alphabet per value byte position, in reverse position
  order. Each consists of a `u16` count from 1 through 256 and that many bytes
  in ascending order without repeats. A byte's zero-based index is its digit.
  The radix is the count for codec 2 and the next power of two greater than
  or equal to the count for codec 3.
- **Low fields.** Subtract the base from every integer. For count N and the
  greatest difference D, the low width L is
  `floor(log2(max(1, floor((D + 1) / N))))`. Store the lowest L bits of each
  difference in rank order. The logical length is N × L bits and can be zero.
- **High bit vector.** For each zero-based local rank R with difference Dᵣ,
  set bit `(Dᵣ >> L) + R`. The logical length is the last set bit plus one,
  at least N and at most 3 × N. It contains exactly N set bits.
- **Bit ordering.** The low and high arrays are packed into little-endian
  `u64` words. A field starts at its least significant bit and may cross a
  word boundary. Bit zero of each word is its least significant bit. The
  arrays are rounded up to complete words and every padding bit is zero.
- **Checkpoints.** One absolute `u32` bit position for each local rank
  divisible by 64, including rank zero. Each position must equal the
  corresponding set bit. Within each batch of 64 ranks, the last selected
  bit is at most 2,048 bit positions past its checkpoint.

Retrieve an integer by selecting the rank's set bit S, subtracting the rank
from S to recover the high field, appending its low field, and adding the
base. The first difference is zero and the reconstructed integers must
increase strictly. Reconstruct the bytes from the last position to the
first, using remainder and quotient by each radix. The power-of-two
variant uses masking and shifting. Any digit without an alphabet entry is
invalid. Reconstructing a row must consume the entire integer and produce
valid UTF-8.

For version 3, the decoded byte total is at least the sum of each group's
width times count. A codec 1 group's width gives a lower bound because each
dictionary symbol expands to at least one byte. If all groups use fixed
decoded widths, the total and maximum must match the directory exactly.

## Verification

The reader applies the caller's `Limits` to file bytes, row and group counts,
declared decoded bytes, and the largest decoded value. The supplied slice and
its offsets must fit the target. The reader checks section arithmetic and
bounds, rank and payload coverage, and structurally possible decoded metadata. `Table::open`
also verifies CRC-32. `Table::open_structural` skips that checksum pass. Both
check each selected record's codec data, decoded length, and UTF-8 during
retrieval. Opening either way checks Elias–Fano section coverage, radix
products, bit padding, ordinal checkpoints and spans, and integer ordering.
Retrieval checks reconstruction digits and UTF-8. Sampling propagates a
selected record's error without redrawing.

`Table::validate_all(true)` verifies CRC-32, decodes every rank, verifies actual
decoded counts, lengths and exact uniqueness, and returns a report. Passing
`false` omits the uniqueness check. `Table::prepare` requires full validation
with uniqueness and keeps the existing file bytes and ranks. CRC-32 detects
accidental corruption. File authenticity requires a separate mechanism.
[Reader trust boundary](DESIGN.md#reader-trust-boundaries)
