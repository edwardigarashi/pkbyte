#!/usr/bin/env python3
"""Build and run the optional native benchmark using local system libraries."""
from __future__ import annotations

import argparse
import ctypes.util
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    root = Path(__file__).resolve().parents[1]
    parser.add_argument("--input", type=Path, default=root / "data/ssa_2024_given_names.txt")
    parser.add_argument("--output-dir", type=Path, default=root / "benchmarks/out/ssa2024")
    parser.add_argument("--iterations", type=int, default=100_000)
    parser.add_argument("--rounds", type=int, default=9)
    parser.add_argument("--startup-rounds", type=int, default=9)
    parser.add_argument("--skip-build", action="store_true", help="Use an already built release library; still compile the harness.")
    args = parser.parse_args()
    if platform.system() != "Linux":
        raise SystemExit("This optional FFI benchmark currently targets Linux. The Rust library has no system-library dependency.")
    if min(args.iterations, args.rounds, args.startup_rounds) <= 0:
        raise SystemExit("Iterations and round counts must be positive.")
    if not args.input.is_file():
        raise SystemExit(f"Missing input {args.input}; run python3 tools/fetch_ssa.py first.")

    libraries = {}
    for name in ("sqlite3", "json-c", "z"):
        soname = ctypes.util.find_library(name)
        if not soname:
            raise SystemExit(f"Optional benchmark requires the system {name} shared library.")
        libraries[name] = soname
    if not shutil.which("cargo") or not shutil.which("rustc"):
        raise SystemExit("cargo and rustc must be on PATH.")

    core_source_hashes = source_hashes(root)
    harness_sha = hashlib.sha256((root / "benchmarks/native.rs").read_bytes()).hexdigest()
    runner_sha = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    input_sha = hashlib.sha256(args.input.read_bytes()).hexdigest()
    if not args.skip_build:
        subprocess.run(["cargo", "build", "--release", "--lib", "--offline", "--locked"], cwd=root, check=True)
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--no-deps", "--format-version", "1", "--offline"], cwd=root
    ))
    target = Path(metadata["target_directory"]) / "release"
    library = target / "libpkbyte.rlib"
    if not library.is_file():
        raise SystemExit(f"Release library not found: {library}")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    executable = args.output_dir.resolve() / "native-benchmark"
    compile_command = [
        "rustc", "--edition=2021", "-O", "-C", "codegen-units=1",
        str(root / "benchmarks/native.rs"),
        "--extern", f"pkbyte={library}",
        "-L", f"dependency={target / 'deps'}",
        "-o", str(executable),
    ]
    for soname in libraries.values():
        # GNU-compatible Linux linkers accept exact SONAMEs even when development
        # symlinks (such as libsqlite3.so) are absent.
        compile_command.extend(["-C", f"link-arg=-l:{soname}"])
    subprocess.run(compile_command, cwd=root, check=True)
    command = [str(executable), str(args.input.resolve()), str(args.output_dir.resolve()),
        str(args.iterations), str(args.rounds), str(args.startup_rounds)]
    start_utc = datetime.now(timezone.utc).isoformat(timespec="seconds")
    completed = subprocess.run(command, cwd=root, check=True, stdout=subprocess.PIPE, text=True)
    result = json.loads(completed.stdout)
    if source_hashes(root) != core_source_hashes:
        raise SystemExit("Core source changed during benchmarking; no reference report was saved. Rerun against one stable revision.")
    if hashlib.sha256((root / "benchmarks/native.rs").read_bytes()).hexdigest() != harness_sha:
        raise SystemExit("Benchmark harness changed during benchmarking; no reference report was saved.")
    if hashlib.sha256(Path(__file__).read_bytes()).hexdigest() != runner_sha or (
        hashlib.sha256(args.input.read_bytes()).hexdigest() != input_sha
    ):
        raise SystemExit("Benchmark runner or input changed during benchmarking; no reference report was saved.")
    result["dataset"]["sha256"] = input_sha
    result["environment"].update({
        "run_started_utc": start_utc,
        "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "kernel": platform.release(),
        "logical_cpus": os.cpu_count(),
        "cpu_model": cpu_model(),
        "system_libraries": libraries,
        "profile": "release library; rustc -O harness; no target-cpu=native",
    })
    result["reproduction"] = {
        "rustup_toolchain": os.environ.get("RUSTUP_TOOLCHAIN"),
        "command": [sys.executable, str(Path(__file__).resolve()), "--input", str(args.input.resolve()),
            "--output-dir", str(args.output_dir.resolve()), "--iterations", str(args.iterations),
            "--rounds", str(args.rounds), "--startup-rounds", str(args.startup_rounds)],
        "harness_sha256": harness_sha,
        "runner_sha256": runner_sha,
        "core_source_files_sha256": core_source_hashes,
        "core_source_manifest_sha256": hashlib.sha256(json.dumps(
            core_source_hashes, sort_keys=True, separators=(",", ":")
        ).encode("utf-8")).hexdigest(),
        "core_source_manifest_hash_format": "SHA-256 of UTF-8 JSON mapping relative paths to content SHA-256 hashes; sorted keys; separators comma and colon; no trailing newline.",
        "release_rlib_sha256": hashlib.sha256(library.read_bytes()).hexdigest(),
        "benchmark_executable_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
        "generated_files_sha256": {
            filename: hashlib.sha256((args.output_dir / filename).read_bytes()).hexdigest()
            for filename in (
                "names.compact.npk", "names.raw.npk", "names.json", "names.txt.gz", "names.sqlite",
            )
        },
    }
    output = args.output_dir / "results.json"
    output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2))
    print(f"Saved {output}", file=sys.stderr)


def cpu_model() -> str:
    try:
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                return line.partition(":")[2].strip()
    except OSError:
        pass
    return platform.processor() or "unknown"


def source_hashes(root: Path) -> dict[str, str]:
    paths = [root / "Cargo.toml", root / "Cargo.lock", *sorted((root / "src").rglob("*.rs"))]
    return {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in paths if path.is_file()}


if __name__ == "__main__":
    main()
