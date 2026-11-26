# Third-party material

The pkbyte library and CLI declare zero Cargo dependencies. The CLI uses the
Rust standard library. The Linux CLI archive includes the standard-library
notice shipped with its Rust 1.100.0 compiler as `COPYRIGHT-library.html`, plus
notices for the toolchain's native runtime components. The Rust build pins
the musl toolchain to musl 1.2.5 and GCC 9.2.0 and includes static libunwind
in the target libraries. [Rust target toolchain source](https://raw.githubusercontent.com/rust-lang/rust/1.100.0/src/ci/docker/scripts/musl-toolchain.sh)

| Notice | Source |
| --- | --- |
| `musl-1.2.5.COPYRIGHT` | [musl 1.2.5](https://git.musl-libc.org/cgit/musl/plain/COPYRIGHT?h=v1.2.5) |
| `llvm-libunwind.LICENSE.txt` | [LLVM submodule pinned by Rust 1.100.0](https://raw.githubusercontent.com/rust-lang/llvm-project/0a5579eb649d3634a9c30ac873300cc2944fa070/libunwind/LICENSE.TXT) |
| `gcc-9.2.0.COPYING3`, `gcc-9.2.0.COPYING.RUNTIME` | [GCC 9.2.0 license](https://raw.githubusercontent.com/gcc-mirror/gcc/releases/gcc-9.2.0/COPYING3) · [Runtime Library Exception](https://raw.githubusercontent.com/gcc-mirror/gcc/releases/gcc-9.2.0/COPYING.RUNTIME) |

The source archive includes the SSA compatibility fixture, its inputs, and
`data/PROVENANCE.md`. The government catalog declares CC0 1.0 for that dataset.
[Government dataset catalog](https://catalog.data.gov/dataset/baby-names-from-social-security-card-applications-national-data)

The benchmark harness can use system SQLite, JSON-C, and zlib. Those libraries
are used only for benchmarking. See `benchmarks/README.md` in the source
archive. The fuzz harness and its dependencies stay in the repository checkout.

