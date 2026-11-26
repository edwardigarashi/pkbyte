#!/usr/bin/env python3
"""Generate, compile, and measure an embedded SSA table using pkbyte."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tempfile


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_hashes(root: Path) -> dict[str, str]:
    paths = [
        "Cargo.toml",
        "Cargo.lock",
        "benchmarks/allocations.rs",
        "benchmarks/embedding.rs",
        "benchmarks/embedding_generate.rs",
        "benchmarks/run_embedding.py",
        "data/ssa_2024_given_names.txt",
        *(path.relative_to(root).as_posix() for path in sorted((root / "src").rglob("*.rs"))),
    ]
    return {path: sha256(root / path) for path in paths}


def cpu_model() -> str:
    try:
        for line in Path("/proc/cpuinfo").read_text().splitlines():
            if line.startswith("model name"):
                return line.partition(":")[2].strip()
    except OSError:
        pass
    return platform.processor() or "unknown"


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iterations", type=int, default=500_000)
    parser.add_argument("--rounds", type=int, default=9)
    parser.add_argument("--output-dir", type=Path, default=root / "benchmarks/out/embedding")
    args = parser.parse_args()
    if min(args.iterations, args.rounds) <= 0:
        parser.error("iterations and rounds must be positive")

    output = args.output_dir.resolve()
    generated = output / "generated"
    generated.mkdir(parents=True, exist_ok=True)
    target = output / "target"
    release = target / "release"
    suffix = ".exe" if os.name == "nt" else ""
    generator = output / f"embedding-generate{suffix}"
    benchmark = output / f"embedding-benchmark{suffix}"
    pack = generated / "pkbyte_table.npk"
    wrapper = generated / "pkbyte_table.rs"
    library = release / "libpkbyte.rlib"
    dataset = "data/ssa_2024_given_names.txt"
    sources = source_hashes(root)
    child_env = {**os.environ, "OUT_DIR": str(generated)}
    subprocess.run([
        "cargo", "build", "--release", "--lib", "--locked", "--offline",
        "--target-dir", str(target),
    ], cwd=root, check=True)
    rustc = [
        "rustc", "--edition=2021", "-O", "-C", "lto=thin", "-C", "codegen-units=1",
        "--extern", f"pkbyte={library}", "-L", f"dependency={release / 'deps'}",
    ]
    subprocess.run([
        *rustc, "benchmarks/embedding_generate.rs", "-o", str(generator),
    ], cwd=root, check=True)
    subprocess.run([str(generator), dataset], cwd=root, env=child_env, check=True)
    generated_paths = {path.name for path in generated.iterdir()}
    if generated_paths != {pack.name, wrapper.name}:
        raise SystemExit(f"Expected only the portable table and wrapper in {generated}.")
    subprocess.run([
        *rustc, "benchmarks/embedding.rs", "-o", str(benchmark),
    ], cwd=root, env=child_env, check=True)

    pack_bytes = pack.read_bytes()
    executable = benchmark.read_bytes()
    if not pack_bytes:
        raise SystemExit("Generated portable table is empty.")
    occurrences = 0
    match = executable.find(pack_bytes)
    while match != -1:
        occurrences += 1
        match = executable.find(pack_bytes, match + 1)
    if occurrences != 1:
        raise SystemExit(f"Expected one exact table byte sequence in the binary, found {occurrences}.")
    artifacts = {
        "release_rlib": library,
        "generator_executable": generator,
        "benchmark_executable": benchmark,
        "portable_table": pack,
        "source_wrapper": wrapper,
    }
    artifact_hashes = {name: sha256(path) for name, path in artifacts.items()}
    start_utc = datetime.now(timezone.utc).isoformat(timespec="seconds")
    with tempfile.TemporaryDirectory() as run_directory:
        completed = subprocess.run(
            [str(benchmark), str(args.iterations), str(args.rounds)],
            cwd=run_directory, check=True, stdout=subprocess.PIPE, text=True,
        )
    if source_hashes(root) != sources or {
        name: sha256(path) for name, path in artifacts.items()
    } != artifact_hashes:
        raise SystemExit("Benchmark inputs changed during the run. The report was discarded.")

    report = json.loads(completed.stdout)
    if report["table"]["portable_bytes"] != len(pack_bytes):
        raise SystemExit("The benchmark reported a different portable table size.")
    report["dataset"] = {"path": dataset, "sha256": sources[dataset], "format": "lines"}
    report["embedding"] = {
        "source_wrapper_bytes": wrapper.stat().st_size,
        "serialized_prepared_metadata_bytes": 0,
        "benchmark_executable_bytes": len(executable),
        "exact_portable_table_occurrences_in_executable": occurrences,
        "generated_files": sorted(generated_paths),
    }
    report["environment"] = {
        "run_started_utc": start_utc,
        "platform": platform.platform(),
        "cpu_architecture": platform.machine(),
        "cpu_model": cpu_model(),
        "logical_cpus": os.cpu_count(),
        "available_logical_cpus": len(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
        "rustc": subprocess.check_output(["rustc", "-vV"], cwd=root, text=True).strip(),
        "cargo": subprocess.check_output(["cargo", "--version"], cwd=root, text=True).strip(),
        "library_build": "Cargo release profile (thin LTO, one code-generation unit)",
        "generator_and_harness_build": "rustc -O -C lto=thin -C codegen-units=1, no stripping option",
    }
    report["reproduction"] = {
        "rustup_toolchain": os.environ.get("RUSTUP_TOOLCHAIN"),
        "command": [
            "python3", "benchmarks/run_embedding.py", "--iterations", str(args.iterations),
            "--rounds", str(args.rounds),
        ],
        "source_sha256": sources,
        "artifact_sha256": artifact_hashes,
    }
    path = output / "results.json"
    path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"Saved {path}", file=sys.stderr)


if __name__ == "__main__":
    main()
