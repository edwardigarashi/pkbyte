"""Build and run the isolated EF-vs-phrase reader experiment using local Rust."""
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parent
project = root.parents[1]
output_dir = root.parent / "out" / "compile_hotpath"
output_dir.mkdir(parents=True, exist_ok=True)
target = output_dir / "target"
release = target / "release"
subprocess.run([
    "cargo", "build", "--release", "--lib", "--offline", "--locked",
    "--manifest-path", str(project / "Cargo.toml"), "--target-dir", str(target),
], cwd=project, check=True)
library = release / "libpkbyte.rlib"
executable = output_dir / "ef_benchmark"
subprocess.run([
    "rustc", "--edition=2021", "-O", "-C", "lto=thin", "src/ef_main.rs",
    "--extern", f"pkbyte={library}", "-L", f"dependency={release / 'deps'}",
    "-o", str(executable),
], cwd=root, check=True)
result = subprocess.run([str(executable)], cwd=root, check=True, stdout=subprocess.PIPE, text=True)
report = output_dir / "ef_results.json"
report.write_text(result.stdout, encoding="utf-8")
print(f"Saved {report}")
