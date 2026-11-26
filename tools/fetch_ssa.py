#!/usr/bin/env python3
"""Fetch the official SSA national archive and derive a unique-name fixture.

No third-party Python packages are required. The SHA-256 pin refers to the
archive retrieved for this prototype; SSA updates this URL over time. Use
--allow-updated-source only when deliberately refreshing the fixture.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import io
import json
from pathlib import Path
import urllib.request
import zipfile


SOURCE_URL = "https://www.ssa.gov/oact/babynames/names.zip"
EXPECTED_ARCHIVE_SHA256 = (
    "cd78e975ed7bb358e018dd62fbe14ced89295e9581c49172ca4eedcb011b3724"
)
CATALOG_URL = (
    "https://catalog.data.gov/dataset/"
    "baby-names-from-social-security-card-applications-national-data"
)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--year", type=int, default=2024)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--allow-updated-source", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    output = args.output or root / "data" / f"ssa_{args.year}_given_names.txt"

    with urllib.request.urlopen(SOURCE_URL, timeout=60) as response:
        archive_bytes = response.read(32_000_001)
    if len(archive_bytes) > 32_000_000:
        raise SystemExit("Source archive exceeds this script's 32 MB limit.")
    if not archive_bytes.startswith(b"PK"):
        raise SystemExit("The server returned a non-ZIP response; no fixture was written.")
    archive_sha = hashlib.sha256(archive_bytes).hexdigest()
    if archive_sha != EXPECTED_ARCHIVE_SHA256 and not args.allow_updated_source:
        raise SystemExit(
            "The SSA archive has changed. Expected "
            f"{EXPECTED_ARCHIVE_SHA256}; received {archive_sha}. "
            "Review the source before using --allow-updated-source."
        )

    member = f"yob{args.year}.txt"
    with zipfile.ZipFile(io.BytesIO(archive_bytes)) as archive:
        selected_bytes = archive.read(member)
    rows = list(csv.reader(io.StringIO(selected_bytes.decode("utf-8"))))
    names = set()
    for row in rows:
        if len(row) != 3 or row[1] not in {"F", "M"} or not row[2].isdigit():
            raise SystemExit(f"Unexpected source row: {row!r}")
        name = row[0]
        if not name or "\n" in name or "\r" in name:
            raise SystemExit("Source contains an invalid line-delimited name.")
        names.add(name)
    ordered = sorted(names, key=lambda name: name.encode("utf-8"))
    fixture = ("\n".join(ordered) + "\n").encode("utf-8")
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(fixture)
    metadata = {
        "source_url": SOURCE_URL,
        "catalog_url": CATALOG_URL,
        "license_url": "https://creativecommons.org/publicdomain/zero/1.0/",
        "archive_sha256": archive_sha,
        "archive_bytes": len(archive_bytes),
        "selected_member": member,
        "selected_member_sha256": hashlib.sha256(selected_bytes).hexdigest(),
        "source_rows": len(rows),
        "unique_names": len(ordered),
        "fixture_bytes": len(fixture),
        "fixture_sha256": hashlib.sha256(fixture).hexdigest(),
        "all_names_ascii": all(name.isascii() for name in ordered),
        "transform": "Exact string deduplication across sex categories; UTF-8 byte sort; LF after every name; no case conversion or Unicode normalization.",
    }
    metadata_path = output.with_suffix(".source.json")
    metadata_path.write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(output), **metadata}, indent=2))


if __name__ == "__main__":
    main()
