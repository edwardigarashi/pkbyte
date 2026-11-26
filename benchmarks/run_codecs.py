#!/usr/bin/env python3
"""Capture comparable storage and prepared-reader measurements for each codec."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import platform
import subprocess
import sys

# Reuse the previous local harness's environment and hash helpers.
sys.dont_write_bytecode = True
from run_prepared import cpu_model, sha256


def input_hashes(root: Path) -> dict[str, str]:
    inputs = [
        root / "Cargo.toml",
        root / "Cargo.lock",
        root / "benchmarks/allocations.rs",
        root / "benchmarks/build_memory.rs",
        root / "benchmarks/codec_workloads.rs",
        root / "benchmarks/codecs.rs",
        root / "benchmarks/run_codecs.py",
        root / "benchmarks/run_prepared.py",
        root / "data/ssa_2024_given_names.txt",
        *sorted((root / "src").rglob("*.rs")),
    ]
    return {path.relative_to(root).as_posix(): sha256(path) for path in inputs}


def main() -> None:
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iterations", type=int, default=250_000)
    parser.add_argument("--rounds", type=int, default=7)
    parser.add_argument("--build-rounds", type=int, default=3)
    parser.add_argument("--extended-workloads", action="store_true", help="Add small and large URL and ID workloads")
    parser.add_argument("--output-dir", type=Path, default=root / "benchmarks/out/codecs")
    args = parser.parse_args()
    if min(args.iterations, args.rounds, args.build_rounds) <= 0:
        parser.error("iterations, rounds, and build rounds must be positive")

    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    target = output / "target"
    release = target / "release"
    library = release / "libpkbyte.rlib"
    suffix = ".exe" if os.name == "nt" else ""
    executable = output / f"codec-benchmark{suffix}"
    allocation_executable = output / f"build-memory{suffix}"
    workload_choice = "extended" if args.extended_workloads else "standard"
    dataset_path = "data/ssa_2024_given_names.txt"
    inputs = input_hashes(root)
    subprocess.run(
        ["cargo", "build", "--release", "--lib", "--locked", "--offline", "--target-dir", str(target)],
        cwd=root, check=True,
    )
    rustc = [
        "rustc", "--edition=2021", "-O", "-C", "lto=thin", "-C", "codegen-units=1",
        "--extern", f"pkbyte={library}", "-L", f"dependency={release / 'deps'}",
    ]
    for source, path in [
        ("benchmarks/codecs.rs", executable), ("benchmarks/build_memory.rs", allocation_executable)
    ]:
        subprocess.run([*rustc, source, "-o", str(path)], cwd=root, check=True)
    artifacts = {
        "release_rlib_sha256": sha256(library),
        "benchmark_executable_sha256": sha256(executable),
        "build_allocation_executable_sha256": sha256(allocation_executable),
    }
    start_utc = datetime.now(timezone.utc).isoformat(timespec="seconds")
    completed = subprocess.run(
        [str(executable), dataset_path, str(output),
         str(args.iterations), str(args.rounds), str(args.build_rounds), workload_choice],
        cwd=root, check=True, stdout=subprocess.PIPE, text=True,
    )
    allocation_output = subprocess.check_output(
        [str(allocation_executable), dataset_path, str(output), str(args.build_rounds), workload_choice],
        cwd=root, text=True,
    )
    if input_hashes(root) != inputs:
        raise SystemExit("Benchmark inputs changed during the run. The report was discarded.")
    if (sha256(library), sha256(executable), sha256(allocation_executable)) != (
        artifacts["release_rlib_sha256"], artifacts["benchmark_executable_sha256"],
        artifacts["build_allocation_executable_sha256"],
    ):
        raise SystemExit("Compiled artifacts changed during the run. The report was discarded.")

    report = json.loads(completed.stdout)
    allocations = json.loads(allocation_output)
    report["build_allocation_method"] = {
        "separate_process": True,
        "scope": "single-thread Rust allocator calls during pkbyte::build over resident source strings, excluding release of the returned table",
        "requested_bytes": "cumulative requested allocation sizes, including the new size of each reallocation and any failed requests",
        "peak_live_bytes": "maximum sum of requested sizes of successful scoped allocations still live at an allocator boundary",
        "reallocation": "success replaces the old size with the new size; failure keeps the old size; allocator-internal overlap is outside scope",
        "excludes": ["pre-existing heap", "allocator overhead", "stack", "RSS", "allocations outside Rust's global allocator"],
        "live_bytes_at_return": "checked against the returned table buffer capacity",
    }
    for dataset in report["datasets"]:
        for mode in dataset["modes"]:
            filename = f"{dataset['dataset']}_{mode['mode']}.npk"
            serialized = output / filename
            assert serialized.stat().st_size == mode["file_bytes"], filename
            mode["file_sha256"] = sha256(serialized)
            mode["build_allocations"] = allocations[dataset["dataset"]][mode["mode"]]
    report["environment"] = {
        "run_started_utc": start_utc,
        "platform": platform.platform(),
        "cpu_architecture": platform.machine(),
        "cpu_model": cpu_model(),
        "logical_cpus": os.cpu_count(),
        "available_logical_cpus": len(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None,
        "rustc": subprocess.check_output(["rustc", "-vV"], cwd=root, text=True).strip(),
        "cargo": subprocess.check_output(["cargo", "--version"], cwd=root, text=True).strip(),
        "library_build": "Cargo release profile",
        "harness_build": "rustc -O -C lto=thin -C codegen-units=1",
    }
    report["reproduction"] = {
        "rustup_toolchain": os.environ.get("RUSTUP_TOOLCHAIN"),
        "command": [
            "python3", "benchmarks/run_codecs.py", "--iterations", str(args.iterations),
            "--rounds", str(args.rounds), "--build-rounds", str(args.build_rounds),
            *(["--extended-workloads"] if args.extended_workloads else []),
        ],
        "input_sha256": inputs,
        **artifacts,
    }
    path = output / "results.json"
    path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"Saved {path}", file=sys.stderr)


if __name__ == "__main__":
    main()
