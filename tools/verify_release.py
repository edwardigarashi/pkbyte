#!/usr/bin/env python3
"""Build and verify a local pkbyte crate and Linux CLI archive."""

import argparse
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import platform
import shlex
import shutil
import subprocess
import sys
import tarfile
import tempfile


ROOT = Path(__file__).resolve().parents[1]
CLI_TARGET = "x86_64-unknown-linux-musl"
RELEASE_TOOLCHAIN = "1.100.0"
MSRV = "1.74.0"
EXAMPLE = Path("examples/generated/values.jsonl")
PROJECT_LICENSE = "LICENSE"
NATIVE_NOTICES = ("gcc-9.2.0.COPYING3", "gcc-9.2.0.COPYING.RUNTIME",
                  "llvm-libunwind.LICENSE.txt", "musl-1.2.5.COPYRIGHT")
REQUIRED_FILES = {
    "Cargo.toml", "Cargo.toml.orig", "Cargo.lock", "README.md", "CHANGELOG.md",
    PROJECT_LICENSE,
    "CONTRIBUTING.md", "docs/FORMAT.md", "docs/RELEASE.md", "docs/VALIDATION.md",
    "docs/TECHNICAL-DESIGN.md", "docs/assets/readme/measurements.json",
    "docs/assets/readme/storage.svg", "docs/assets/readme/sampling.svg", "docs/assets/readme/amortization.svg",
    "docs/assets/readme/storage.png", "docs/assets/readme/sampling.png",
    "docs/CLI-ARCHIVE.md", "docs/THIRD-PARTY.md", "data/PROVENANCE.md",
    "data/given_names_2024.npk", "src/lib.rs", "src/bin/pkbyte.rs",
    "examples/basic.rs", "examples/embedded.rs", "examples/picker.rs", "examples/save_table.rs",
    "examples/generated/Cargo.toml.in",
    "examples/generated/Cargo.lock.in", "examples/generated/build.rs",
    "examples/generated/src/main.rs", EXAMPLE.as_posix(), "tools/run_generated.py",
    "tests/build_integration.rs", "tests/picker.rs", *(f"licenses/{name}" for name in NATIVE_NOTICES),
}
EXCLUDED_DIRECTORIES = {".git", ".github", "dist", "fuzz", "namepack_research", "out", "output", "target", "work"}


def run(*command, cwd, env=None, capture=False):
    print(f"[{cwd}] {shlex.join(map(str, command))}", flush=True)
    result = subprocess.run(command, cwd=cwd, env=env, check=True, stdout=subprocess.PIPE if capture else None)
    return result.stdout


def checksum(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def package_metadata(cargo, source, environment):
    metadata = json.loads(run(*cargo, "metadata", "--format-version", "1", "--locked", "--offline",
                              cwd=source, env=environment, capture=True))
    packages = metadata["packages"]
    if len(packages) != 1:
        raise ValueError("expected a dependency-free package")
    package = packages[0]
    if (package["name"] != "pkbyte" or package["publish"] != ["crates-io"] or package["dependencies"]
            or package["features"] or package["edition"] != "2021" or package["rust_version"] != "1.74"
            or package["license"] != "MIT"):
        raise ValueError("unexpected package name, registry, dependencies, features, edition, MSRV or license")
    return package


def extract_package(archive, destination, name):
    with tarfile.open(archive, "r:gz") as packed:
        files = set()
        for member in packed.getmembers():
            parts = PurePosixPath(member.name).parts
            if (len(parts) < 2 or parts[0] != name or ".." in parts
                    or EXCLUDED_DIRECTORIES.intersection(parts) or not member.isfile()):
                raise ValueError(f"unexpected package member {member.name}")
            path = PurePosixPath(*parts[1:]).as_posix()
            if path in files:
                raise ValueError(f"duplicate package member {path}")
            files.add(path)
        missing = REQUIRED_FILES - files
        if missing:
            raise ValueError(f"missing package files {sorted(missing)}")
        packed.extractall(destination, filter="data")
    return sorted(files)


def verify_source(source, stable, environment):
    cargo = ("cargo", f"+{stable}")
    locked = ("--locked", "--offline")
    generated = (sys.executable, "tools/run_generated.py")
    run(*cargo, "fmt", "--all", "--", "--check", cwd=source, env=environment)
    run(*generated, "--toolchain", stable, "fmt", cwd=source, env=environment)
    run(*cargo, "clippy", "--all-targets", *locked, "--", "-D", "warnings", cwd=source, env=environment)
    run(*generated, "--toolchain", stable, "clippy", cwd=source, env=environment)
    run(*cargo, "doc", "--no-deps", *locked, cwd=source, env=environment | {"RUSTDOCFLAGS": "-D warnings"})
    run(*cargo, "test", "--release", "--test", "prepared_allocations", *locked, cwd=source, env=environment)
    run(*cargo, "tree", "--edges", "normal,build,dev", *locked, cwd=source, env=environment)
    for toolchain in (stable, MSRV):
        cargo = ("cargo", f"+{toolchain}")
        run(*cargo, "test", "--all-targets", *locked, cwd=source, env=environment)
        run(*cargo, "test", "--doc", *locked, cwd=source, env=environment)
        for example in ("basic", "embedded"):
            run(*cargo, "run", "--example", example, *locked, cwd=source, env=environment)
        run(*cargo, "run", "--example", "picker", *locked, "--", "data/given_names_2024.npk",
            cwd=source, env=environment)
        run(*cargo, "run", "--example", "save_table", *locked, "--", source.parent / f"example-{toolchain}.npk",
            cwd=source, env=environment)
        run(*generated, "--toolchain", toolchain, cwd=source, env=environment)


def verify_cli(binary, source, directory, version, environment):
    def cli(*args, capture=False):
        return run(binary, *args, cwd=directory, env=environment, capture=capture)

    if cli("--version", capture=True).decode().strip() != f"pkbyte {version}":
        raise ValueError("CLI version disagrees with the packaged version")
    (directory / "plain.txt").write_bytes("Tokyo\n大阪\n".encode())
    cli("pack", "plain.txt")
    if json.loads(cli("pick", "plain.npk", capture=True)) not in ("Tokyo", "大阪"):
        raise ValueError("two-command quick start returned an unexpected value")
    data = source / EXAMPLE
    values = [json.loads(line) for line in data.read_bytes().splitlines()]
    if "" not in values or "東京都" not in values or "A\0B" not in values or not any("\n" in v for v in values):
        raise ValueError("CLI fixture must cover empty, multilingual and escaped control values")
    cli("pack", data, "-o", "first.npk")
    cli("info", "first.npk")
    cli("verify", "first.npk", "--unique")
    sampled = cli("pick", "first.npk", "-n", "40", "--seed", "42", capture=True)
    samples = [json.loads(line) for line in sampled.splitlines()]
    if len(samples) != 40 or not set(samples).issubset(values):
        raise ValueError("sampling returned unexpected values")
    exported = cli("unpack", "first.npk", capture=True)
    after = [json.loads(line) for line in exported.splitlines()]
    if len(after) != len(values) or set(after) != set(values):
        raise ValueError("export changed the set of values")
    (directory / "export.jsonl").write_bytes(exported)
    cli("pack", "export.jsonl", "-o", "rebuilt.npk")
    cli("verify", "rebuilt.npk", "--unique")
    if (directory / "first.npk").read_bytes() != (directory / "rebuilt.npk").read_bytes():
        raise ValueError("export and rebuild changed the serialized bytes")


def make_cli_archive(archive, binary, source, rust_notice, stem):
    documents = (source / "docs" / "CLI-ARCHIVE.md", source / "docs" / "THIRD-PARTY.md",
                 source / PROJECT_LICENSE,
                 rust_notice, *(source / "licenses" / name for name in NATIVE_NOTICES))
    with tarfile.open(archive, "w:gz") as packed:
        for path in (binary, *documents):
            contents = path.read_bytes()
            name = "README.md" if path.name == "CLI-ARCHIVE.md" else path.name
            member = tarfile.TarInfo(f"{stem}/{name}")
            member.mode = 0o755 if path == binary else 0o644
            member.size = len(contents)
            packed.addfile(member, io.BytesIO(contents))


def git_info():
    if not shutil.which("git"):
        return "revision: unavailable (Git executable absent)"
    revision = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True, check=False)
    if revision.returncode:
        return "revision: unavailable (source snapshot without Git metadata)"
    status = run("git", "status", "--porcelain", cwd=ROOT, capture=True)
    return f"revision: {revision.stdout.strip()}\nworking tree: {'dirty' if status else 'clean'}"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    if not (sys.platform == "linux" and platform.machine() == "x86_64"):
        parser.error("this distribution workflow requires an x86-64 Linux host")
    if sys.version_info < (3, 12):
        parser.error("Python 3.12 or later is required")

    cargo = ("cargo", f"+{RELEASE_TOOLCHAIN}")
    rustc = ("rustc", f"+{RELEASE_TOOLCHAIN}")
    rust_version = run(*rustc, "-vV", cwd=ROOT, capture=True).decode()
    host = next(line.removeprefix("host: ") for line in rust_version.splitlines() if line.startswith("host: "))
    sysroot = Path(os.fsdecode(run(*rustc, "--print", "sysroot", cwd=ROOT, capture=True).strip()))
    linker = sysroot / "lib" / "rustlib" / host / "bin" / "rust-lld"
    rust_notice = sysroot / "share" / "doc" / "rust" / "COPYRIGHT-library.html"
    if not linker.is_file() or not (sysroot / "lib" / "rustlib" / CLI_TARGET / "lib").is_dir():
        parser.error(f"install the {CLI_TARGET} target and rust-lld for {RELEASE_TOOLCHAIN}")
    if not rust_notice.is_file():
        parser.error(f"the Rust {RELEASE_TOOLCHAIN} standard-library copyright notice is missing")

    with tempfile.TemporaryDirectory(prefix="pkbyte-release-") as temporary:
        work = Path(temporary)
        environment = os.environ | {"CARGO_TARGET_DIR": str(work / "target"), "CARGO_HOME": str(work / "cargo")}
        package = package_metadata(cargo, ROOT, environment)
        version = package["version"]
        name = f"pkbyte-{version}"
        packager = ("cargo", f"+{MSRV}")
        run(*packager, "package", "--locked", "--offline", cwd=ROOT, env=environment)
        crate = work / "target" / "package" / f"{name}.crate"
        files = extract_package(crate, work / "unpacked", name)
        source = work / "unpacked" / name
        if package_metadata(cargo, source, environment)["version"] != version:
            raise ValueError("extracted source has an unexpected version")
        verify_source(source, RELEASE_TOOLCHAIN, environment)

        install = work / "install"
        cli_environment = environment | {"CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER": str(linker)}
        run(*cargo, "install", "--path", source, "--root", install, "--target", CLI_TARGET,
            "--locked", "--offline", cwd=work, env=cli_environment)
        stem = f"{name}-{CLI_TARGET}"
        cli_archive = work / f"{stem}.tar.gz"
        binary = install / "bin" / "pkbyte"
        elf = run("readelf", "-l", "-d", binary, cwd=work, capture=True)
        if b"INTERP" in elf or b"(NEEDED)" in elf:
            raise ValueError("the Linux CLI must run without a dynamic loader or shared libraries")
        make_cli_archive(cli_archive, binary, source, rust_notice, stem)
        with tarfile.open(cli_archive, "r:gz") as packed:
            packed.extractall(work / "cli", filter="data")
        verify_cli(work / "cli" / stem / "pkbyte", source, work, version, environment)

        destination = ROOT / "dist" / name
        destination.mkdir(parents=True, exist_ok=True)
        for artifact in (crate, cli_archive):
            shutil.copyfile(artifact, destination / artifact.name)
        (destination / "PACKAGE-FILES.txt").write_text("\n".join(files) + "\n", encoding="utf-8")
        (destination / "BUILD-INFO.txt").write_text(
            f"package: {name}\n{git_info()}\nplatform: {platform.platform()}\ncli target: {CLI_TARGET}\n"
            + rust_version + run(*cargo, "--version", cwd=ROOT, capture=True).decode()
            + "packager: " + run(*packager, "--version", cwd=ROOT, capture=True).decode(),
            encoding="utf-8",
        )
        (destination / "SHA256SUMS").write_text(
            "".join(f"{checksum(destination / name)}  {name}\n" for name in sorted(
                (crate.name, cli_archive.name, "PACKAGE-FILES.txt", "BUILD-INFO.txt"))), encoding="utf-8",
        )
        print(f"Verified release artifacts in {destination}")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"release verification failed: {error}") from None
