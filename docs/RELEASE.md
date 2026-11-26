# Release procedure

`0.1.1` is a release candidate. Publication on crates.io is pending. The
local distribution is a Cargo `.crate` source archive and a native
`x86_64-unknown-linux-musl` CLI tarball. The project-owned code and
documentation are available under the MIT license.
[MIT license](../LICENSE) · [Third-party notices](THIRD-PARTY.md)

## Build and verify

Python 3.12 or newer, rustup, the host C linker, and `readelf` are required.
The procedure pins Rust 1.100.0 for the CLI archive and Rust 1.74.0 for
packaging and publishing the source crate and verifying the MSRV. Install
`rustfmt`, Clippy, and the musl standard library for the release toolchain.
Run the following from the repository root on an x86-64 Linux host.

```sh
rustup toolchain install 1.100.0 --profile minimal --component rustfmt --component clippy
rustup toolchain install 1.74.0 --profile minimal
rustup target add --toolchain 1.100.0 x86_64-unknown-linux-musl
python3 tools/verify_release.py
```

The script prints the exact commands and fails on a missing prerequisite or
failed check. It uses an empty temporary
Cargo home and private build directory, creates the source archive with
`cargo +1.74.0 package --locked --offline`, audits its file list, and
unpacks it into a fresh directory. It checks the package metadata, style,
documentation, tests, example applications, and dependency graph from that
source. It then installs the CLI from the unpacked crate, checks its ELF
dependencies, and tests the binary extracted from its own archive with the
toolchain notices. The JSON Lines smoke test checks sample
membership, exact exported values, and byte-identical rebuild. Review the
bundled notices when updating the release toolchain. [Third-party notices](THIRD-PARTY.md)

The script writes `dist/pkbyte-<version>/` after verification, with the `.crate`
and CLI tarball, a package file list, build information, and `SHA256SUMS`.
Use a clean checkout for release work. The build information records the Git
revision and dirty status when available. SHA-256 identifies the files produced
by that run. Archive hashes alone do not establish publisher identity.

```sh
cd dist/pkbyte-0.1.1
sha256sum -c SHA256SUMS
tar -tzf pkbyte-0.1.1.crate
tar -tzf pkbyte-0.1.1-x86_64-unknown-linux-musl.tar.gz
```

## Consume a local candidate

After verifying the checksums, unpack the `.crate` beside a consumer and point
its manifest at the unpacked package. Add the same path under
`[build-dependencies]` if a build script uses pkbyte.

```toml
[dependencies]
pkbyte = { path = "../pkbyte-0.1.1" }
```

With the unpacked package as the working directory, run the Rust examples or
the standalone Cargo consumer.

```sh
cargo run --locked --offline --example basic
cargo run --locked --offline --example picker -- data/given_names_2024.npk
cargo run --locked --offline --example embedded
python3 tools/run_generated.py --toolchain 1.74.0
```

To use the supplied CLI on Linux x86-64, unpack the tarball and run its
executable directly. To install from the unpacked crate on a native Rust host,
use `cargo install --path ./pkbyte-0.1.1 --locked --offline`.

```sh
tar -xzf pkbyte-0.1.1-x86_64-unknown-linux-musl.tar.gz
./pkbyte-0.1.1-x86_64-unknown-linux-musl/pkbyte --version
```

## Publish on crates.io

Commit the candidate in the source repository named in `package.repository`
and record `git rev-parse HEAD`. Push the release revision with the `v0.1.1`
tag to the public repository. Verify the documentation links and image
rendering, then check the platform scope and release notes for this revision.
[Validation record](VALIDATION.md) · [Changelog](../CHANGELOG.md)

Use the clean release revision for packaging and publication. Confirm that
the name and release version are available on crates.io. For the first
publication, sign in to the authorized crates.io account and create an API
token with permission to publish a new crate. Enter it at the `cargo login`
prompt. [Cargo publishing guide](https://doc.rust-lang.org/cargo/reference/publishing.html)

```sh
cargo +1.74.0 package --list --locked --offline
python3 tools/verify_release.py
cargo +1.74.0 publish --dry-run --locked --registry crates-io
cmp target/package/pkbyte-0.1.1.crate dist/pkbyte-0.1.1/pkbyte-0.1.1.crate
cargo +1.74.0 login --registry crates-io
cargo +1.74.0 publish --locked --registry crates-io
```

Compare the registry checksum for `0.1.1` with the verified archive
checksum. In a new consumer project, add the exact registry version and run
the rustdoc `Picker::from_strings` example. Install the CLI in a fresh
installation root and check its version.

```sh
cargo add 'pkbyte@=0.1.1'
cargo install pkbyte --version '=0.1.1' --locked --root ./pkbyte-registry-install
./pkbyte-registry-install/bin/pkbyte --version
```

Check that the versioned docs.rs build succeeds and that the crate's library
documentation opens. Check the rendered README on crates.io against the public
source. Update the repository release status after the registry checks pass.
[Registry version](https://crates.io/crates/pkbyte/0.1.1) · [Documentation build](https://docs.rs/crate/pkbyte/0.1.1/builds) · [Library documentation](https://docs.rs/pkbyte/0.1.1/pkbyte/)
