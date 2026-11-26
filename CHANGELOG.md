# Changelog

## 0.1.1 local release candidate

- Adds an owning `Picker` for a file, owned bytes, or an iterator of strings.
  It performs validation and preparation once and supports repeatable seeds,
  owned picks, and a reusable output buffer.
- Introduces `pack`, `pick`, `info`, and `unpack` alongside `verify`.
  `pack` infers the destination and recognizes JSON Lines from a
  `.jsonl` extension. `pick` and `unpack` default to lossless JSON Lines.
  Legacy build, sample, inspect, and export retain their options and defaults.
- Adds command-specific help and executable examples for opening a table,
  saving a table, and reusing a buffer for seeded picks.
- Preserves the existing v1, v2, and v3 file interpretation.

## 0.1.0 local release candidate

- Builds compact immutable sets of exact UTF-8 strings. Supports empty
  strings, whitespace, embedded controls, long values, and explicit
  byte-exact deduplication. The default policy rejects duplicate input.
- Reads table format versions 1, 2, and 3. Writes v2 for raw and phrase
  layouts and v3 for tables using Elias–Fano. Automatic mode compares the
  complete candidate file sizes, and ineligible compact groups fall back
  to raw storage.
- Provides checked opening, complete CRC and semantic validation, and
  prepared retrieval and uniform sampling with replacement. Applications
  supply their random source and reuse initialized output storage across draws.
- Provides CLI build, inspect, verify, sample, and export commands. They
  support lossless JSON Lines, bounded input streams, file pipelines, and
  staged publication of a new or explicitly replaced file.
- Supports `include_bytes!` and generation in Cargo's `OUT_DIR`. The
  distributable includes the example files and a runner that stages the
  standalone generated consumer from the local source.
- Records exact-set models, allocation checks, bounded fuzzing, portability
  evidence, and current comparisons for storage, build heap, reader setup,
  and sampling. Historical Namepack measurements retain their provenance.

Registry publication is pending. [Compatibility policy](docs/FORMAT.md) ·
[Release procedure](docs/RELEASE.md) · [Release verification](docs/VALIDATION.md#release-verification)
