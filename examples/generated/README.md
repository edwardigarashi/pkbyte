# Cargo-generated pkbyte table

Run from the repository root or the unpacked candidate source with Python
3.9 or newer and an installed Rust toolchain.

```sh
python3 tools/run_generated.py
python3 tools/run_generated.py --toolchain 1.74.0
```

The runner stages a disposable consumer from the manifest and lockfile
templates and this directory's source files. The staged application uses
the local pkbyte source as a build and runtime dependency.

The build script reads `values.jsonl` and generates the table and a Rust module in
Cargo's `OUT_DIR`. Cargo runs the build script again when the input changes. The
runtime binary embeds the table and prepares one reader and output buffer for all
draws. The example supplies a reproducible, noncryptographic RNG.
[Application integration](../../docs/INTEGRATION.md)
