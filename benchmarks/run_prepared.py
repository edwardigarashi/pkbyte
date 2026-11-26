#!/usr/bin/env python3
"""Build and run the std-only prepared-reader benchmark."""

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


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_hashes(root: Path) -> dict[str, str]:
    paths = [
        root / "Cargo.toml",
        root / "Cargo.lock",
        root / "benchmarks/prepared.rs",
        root / "benchmarks/run_prepared.py",
        root / "data/given_names_2024.npk",
        *sorted((root / "src").rglob("*.rs")),
    ]
    return {path.relative_to(root).as_posix(): sha256(path) for path in paths}


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
    parser.add_argument("--output-dir", type=Path, default=root / "benchmarks/out/prepared")
    args = parser.parse_args()
    if min(args.iterations, args.rounds) <= 0:
        parser.error("iterations and rounds must be positive")

    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    target = output / "target"
    release = target / "release"
    library = release / "libpkbyte.rlib"
    executable = output / ("prepared-benchmark.exe" if os.name == "nt" else "prepared-benchmark")
    generated = output / "synthetic_mixed_utf8_v2.npk"
    sources = source_hashes(root)
    cargo_command = [
        "cargo", "build", "--release", "--lib", "--locked", "--offline",
        "--target-dir", str(target),
    ]
    rustc_command = [
        "rustc", "--edition=2021", "-O", "-C", "lto=thin", "-C", "codegen-units=1",
        "benchmarks/prepared.rs", "--extern", f"pkbyte={library}",
        "-L", f"dependency={release / 'deps'}", "-o", str(executable),
    ]
    subprocess.run(cargo_command, cwd=root, check=True)
    subprocess.run(rustc_command, cwd=root, check=True)
    binary_hashes = {
        "release_rlib_sha256": sha256(library),
        "benchmark_executable_sha256": sha256(executable),
    }
    start_utc = datetime.now(timezone.utc).isoformat(timespec="seconds")
    completed = subprocess.run(
        [str(executable), "data/given_names_2024.npk", str(generated),
         str(args.iterations), str(args.rounds)],
        cwd=root, check=True, stdout=subprocess.PIPE, text=True,
    )
    if source_hashes(root) != sources:
        raise SystemExit("Benchmark inputs changed during the run. The report was discarded.")
    if sha256(library) != binary_hashes["release_rlib_sha256"] or (
        sha256(executable) != binary_hashes["benchmark_executable_sha256"]
    ):
        raise SystemExit("Benchmark executables changed during the run. The report was discarded.")

    report = json.loads(completed.stdout)
    report["dataset_sha256"] = {
        "ssa_2024_v1": sources["data/given_names_2024.npk"],
        "synthetic_mixed_utf8_v2": sha256(generated),
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
        "library_build": "Cargo release profile",
        "harness_build": "rustc -O -C lto=thin -C codegen-units=1",
    }
    report["reproduction"] = {
        "command": [
            "python3", "benchmarks/run_prepared.py", "--iterations", str(args.iterations),
            "--rounds", str(args.rounds),
        ],
        "source_sha256": sources,
        **binary_hashes,
    }
    path = output / "results.json"
    path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"Saved {path}", file=sys.stderr)


if __name__ == "__main__":
    main()
