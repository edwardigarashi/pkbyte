# pkbyte CLI for Linux x86-64

Run the `pkbyte` executable from this directory, or place it in a directory on
your `PATH`.

```sh
./pkbyte --version
./pkbyte --help
./pkbyte pack values.jsonl
./pkbyte info values.npk
./pkbyte verify values.npk --unique
./pkbyte pick values.npk -n 10 --seed 42
./pkbyte unpack values.npk > exported.jsonl
```

Use one JSON string per physical input line. Encode embedded line breaks and
controls with JSON escapes. The builder rejects exact duplicates unless you
pass `--deduplicate`. Export visits each stored value once. Sampling draws
with replacement. The CLI accepts table versions 1, 2, and 3.

The source archive has the Rust library, complete user guide, and examples.
The archive includes the MIT license for pkbyte and the notices for its Rust
and native runtime components. [Third-party notices](THIRD-PARTY.md)
