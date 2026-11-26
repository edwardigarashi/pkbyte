# Frozen wire-format fixtures

These small synthetic tables were serialized independently of the Rust builder
from the documented wire fields. Keep the bytes fixed when checking reader
compatibility. [Format specification](../../docs/FORMAT.md)

| Files | Coverage |
| --- | --- |
| `v1-raw.npk`, `v1-phrase.npk` | Raw UTF-8 and independently expanded phrases, including UTF-8 code points split across symbols |
| `v2-raw.npk`, `v2-phrase.npk` | The same codecs with the version 2 zero-width empty value |
| `v3-mixed.npk`, `v3-power-of-two.npk` | Raw values and Elias–Fano values with three-symbol alphabets, so the two radix schemes yield distinct integer streams |

The compatibility test specifies the exact expected value at each rank and
exercises CRC verification, full semantic validation, checked and prepared
retrieval, and modulo sampling. The original Namepack SSA v1 fixture has
separate exact-set coverage. [Compatibility test](../compatibility.rs) ·
[SSA dataset provenance](../../data/PROVENANCE.md)
