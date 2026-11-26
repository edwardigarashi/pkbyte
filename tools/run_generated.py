#!/usr/bin/env python3
"""Run the standalone build-script example from a checkout or unpacked crate."""

import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--toolchain", help="rustup toolchain to use")
    parser.add_argument("--target", help="Cargo target for run, build or clippy")
    parser.add_argument("command", choices=("run", "build", "fmt", "clippy"), nargs="?", default="run")
    args = parser.parse_args()
    if args.target and args.command == "fmt":
        parser.error("fmt has no target")

    root = Path(__file__).resolve().parents[1]
    example = root / "examples" / "generated"
    with tempfile.TemporaryDirectory(prefix="pkbyte-generated-") as temporary:
        consumer = Path(temporary)
        for name in ("Cargo.lock.in", "build.rs", "values.jsonl", "src/main.rs"):
            output = consumer / name.removesuffix(".in")
            output.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(example / name, output)
        manifest = (example / "Cargo.toml.in").read_text(encoding="utf-8")
        dependency = 'path = "../.."'
        if manifest.count(dependency) != 2:
            parser.error("expected a relative path for each pkbyte dependency")
        path = json.dumps(str(root), ensure_ascii=False)
        (consumer / "Cargo.toml").write_text(manifest.replace(dependency, f"path = {path}"), encoding="utf-8")

        command = ["cargo"]
        if args.toolchain:
            command.append(f"+{args.toolchain}")
        command += [args.command, "--manifest-path", str(consumer / "Cargo.toml")]
        if args.command == "fmt":
            command += ["--", "--check"]
        else:
            command += ["--locked", "--offline", "--target-dir", str(consumer / "target")]
            if args.target:
                command += ["--target", args.target]
            if args.command == "clippy":
                command += ["--all-targets", "--", "-D", "warnings"]
        return subprocess.run(command, cwd=consumer, check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
