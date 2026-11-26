#!/usr/bin/env python3
"""Render README figures from recorded aggregates; this does not run benchmarks.

Run from any directory: python3 tools/render_readme_charts.py
Development dependency: matplotlib (the version used is in measurements.json).
Both editable SVG and high-resolution PNG outputs are generated.
"""

from __future__ import annotations

import json
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.ticker import FuncFormatter, MultipleLocator


ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "docs" / "assets" / "readme"
DATA = json.loads((ASSETS / "measurements.json").read_text())

INK = "#14293d"
MUTED = "#526576"
TEAL = "#087f78"
BLUE = "#4876a3"
GRAY = "#aab8c4"
GRID = "#e5ebef"
PAPER = "#fbfcfd"

plt.rcParams.update({
    "font.family": "DejaVu Sans",
    "font.size": 12,
    "text.color": INK,
    "axes.labelcolor": MUTED,
    "xtick.color": MUTED,
    "ytick.color": INK,
    "svg.fonttype": "none",
    "svg.hashsalt": "pkbyte-readme-figures-v1",
    "savefig.facecolor": PAPER,
})


def base(title: str, subtitle: str):
    fig = plt.figure(figsize=(12, 5.7), facecolor=PAPER)
    fig.text(0.045, 0.94, "pkbyte  /  RECORDED SSA BENCHMARK", fontsize=10,
             weight="bold", color=TEAL)
    fig.text(0.045, 0.865, title, fontsize=23, weight="bold")
    fig.text(0.045, 0.808, subtitle, fontsize=11.5, color=MUTED)
    ax = fig.add_axes([0.265, 0.265, 0.675, 0.455], facecolor=PAPER)
    ax.set_axisbelow(True)
    ax.grid(axis="x", color=GRID, linewidth=0.8)
    for spine in ax.spines.values():
        spine.set_visible(False)
    ax.tick_params(axis="both", length=0, pad=11)
    return fig, ax


def save(fig, name: str, description: str):
    fig.savefig(ASSETS / f"{name}.svg", metadata={
        "Title": description, "Description": description,
        "Creator": "pkbyte README figure generator", "Date": None,
    })
    fig.savefig(ASSETS / f"{name}.png", dpi=180,
                metadata={"Description": description})
    plt.close(fig)


def storage():
    sizes = DATA["file_sizes_bytes"]
    labels = list(sizes)
    fig, ax = base("29,329 strings. A smaller portable table.",
                   "Complete pkbyte files, including their serialized overhead · lower is better")
    values = [sizes[k] / 1024 for k in labels]
    ax.barh(range(3), values, color=[GRAY, BLUE, TEAL], height=0.52)
    ax.set_yticks(range(3), ["Raw layout", "Phrase layout", "Compact layout"])
    ax.invert_yaxis()
    ax.set_xlim(0, 225)
    ax.xaxis.set_major_locator(MultipleLocator(50))
    ax.set_xlabel("Complete file size (KiB; 1 KiB = 1,024 bytes)", labelpad=13)
    for i, label in enumerate(labels):
        ax.text(values[i] + 4, i, f"{sizes[label]:,} B", va="center",
                fontsize=12, weight="bold", color=TEAL if i == 2 else INK)
    saving = (1 - sizes["Compact"] / sizes["Raw"]) * 100
    fig.text(0.045, 0.125, f"{saving:.1f}% smaller than the raw pkbyte file", fontsize=13,
             weight="bold", color=TEAL)
    fig.text(0.045, 0.065,
             "Source: recorded README values. This chart compares pkbyte layouts, not text, JSON or gzip sizes.",
             fontsize=9.7, color=MUTED)
    save(fig, "storage", "Complete pkbyte table sizes: raw 178,955 bytes; phrase 106,230 bytes; compact 70,868 bytes.")


def sampling():
    times = DATA["warm_sampling_ns_per_value"]
    values = list(times.values())
    fig, ax = base("Fast picks from a compressed table.",
                   "Warm complete sampling · median of repeated batch averages · lower is better")
    ax.barh(range(3), values, color=[GRAY, TEAL, BLUE], height=0.52)
    ax.set_yticks(range(3), ["Loaded vector", "pkbyte\nprepared compact", "SQLite\nprepared transaction"])
    ax.invert_yaxis()
    ax.set_xlim(0, 315)
    ax.xaxis.set_major_locator(MultipleLocator(50))
    ax.set_xlabel("Nanoseconds per value", labelpad=13)
    for i, value in enumerate(values):
        ax.text(value + 5, i, f"{value:.3f} ns", va="center", fontsize=12,
                weight="bold", color=TEAL if i == 1 else INK)
    speed = values[2] / values[1]
    fig.text(0.045, 0.125, f"{speed:.2f}× the measured SQLite sampling rate; the loaded vector remains faster.",
             fontsize=12, weight="bold", color=TEAL)
    fig.text(0.045, 0.068,
             "29,329 SSA values · Rust 1.100.0 · Linux x86-64 · Intel Core i9-12900K",
             fontsize=9.5, color=MUTED)
    fig.text(0.045, 0.031,
             "Includes RNG, retrieval, copying, UTF-8 check and checksum. Initialization is separate (5.699 ms).",
             fontsize=9.5, color=MUTED)
    save(fig, "sampling", "Warm complete sampling: loaded vector 30.450 ns/value; prepared pkbyte compact 57.561; prepared SQLite transaction 251.036. Initialization excluded.")


def amortization():
    startup = DATA["initialization"]["median_ms"] * 1_000_000
    steady = DATA["warm_sampling_ns_per_value"]["Prepared pkbyte compact"]
    q = [10 ** (i / 30) for i in range(181)]
    average = [startup / count + steady for count in q]
    fig = plt.figure(figsize=(12, 6), facecolor=PAPER)
    fig.text(0.045, 0.94, "pkbyte  /  ANALYTICAL MODEL", fontsize=10,
             weight="bold", color=TEAL)
    fig.text(0.045, 0.865, "Open once. Let repeated picks share the setup cost.",
             fontsize=20.5, weight="bold")
    fig.text(0.045, 0.807, "Average cost = 5.699 ms ÷ number of picks + 57.561 ns per pick",
             fontsize=12, color=MUTED)
    ax = fig.add_axes([0.12, 0.25, 0.81, 0.48], facecolor=PAPER)
    ax.set_xscale("log")
    ax.set_yscale("log")
    ax.plot(q, average, color=TEAL, linewidth=2.8)
    ax.axhline(steady, color=BLUE, linestyle=(0, (4, 4)), linewidth=1.3)
    ax.set_xlim(1, 1_000_000)
    ax.set_ylim(35, 15_000_000)
    ax.set_xticks([1, 100, 10_000, 1_000_000], ["1", "100", "10,000", "1,000,000"])
    ax.set_yticks([100, 1_000, 10_000, 100_000, 1_000_000, 10_000_000])
    ax.yaxis.set_major_formatter(FuncFormatter(lambda value, _: f"{value:,.0f}"))
    ax.grid(which="major", color=GRID, linewidth=0.8)
    ax.tick_params(which="both", length=0, pad=8)
    for spine in ax.spines.values():
        spine.set_visible(False)
    ax.set_xlabel("Picks per initialization (log scale)", labelpad=11)
    ax.set_ylabel("Average ns per pick (log scale)", labelpad=12)
    ax.text(1.5, steady * 1.5, "57.561 ns steady-state component", color=BLUE, fontsize=10)
    ax.scatter([1_000_000], [startup / 1_000_000 + steady], color=TEAL, s=40, zorder=3)
    ax.annotate("63.260 ns at 1 million picks", (1_000_000, startup / 1_000_000 + steady),
                xytext=(-210, 25), textcoords="offset points", color=TEAL, fontsize=10,
                arrowprops={"arrowstyle":"-", "color":TEAL})
    fig.text(0.045, 0.11, "Model from two reported benchmark aggregates; the curve is not a measured scaling experiment.",
             fontsize=10.5, color=MUTED)
    fig.text(0.045, 0.06, "Embedded startup only. Excludes process launch, file I/O, terminal output and changing cache conditions.",
             fontsize=9.7, color=MUTED)
    save(fig, "amortization", "Analytical startup amortization model using 5.699 ms embedded initialization and 57.561 ns warm sampling; not measured observations.")


def main():
    storage()
    sampling()
    amortization()
    compact = DATA["file_sizes_bytes"]["Compact"]
    print(json.dumps({
        "generated": [f"{name}.{ext}" for name in ("storage", "sampling", "amortization")
                      for ext in ("svg", "png")],
        "complete_file_bytes_per_value": compact / DATA["dataset"]["unique_values"],
        "raw_size_reduction_percent": (1 - compact / DATA["file_sizes_bytes"]["Raw"]) * 100,
        "sqlite_to_pkbyte_warm_ratio": DATA["warm_sampling_ns_per_value"]["Prepared SQLite transaction"]
                                      / DATA["warm_sampling_ns_per_value"]["Prepared pkbyte compact"],
    }, indent=2))


if __name__ == "__main__":
    main()
