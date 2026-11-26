# Benchmark data provenance

## SSA 2024 given names

`ssa_2024_given_names.txt` is derived from the US Social Security
Administration's national baby-name archive, selecting `yob2024.txt`.

- Official download: <https://www.ssa.gov/oact/babynames/names.zip>
- Official description and disclosure limits:
  <https://www.ssa.gov/oact/babynames/limits.html>
- Government dataset catalog and license declaration:
  <https://catalog.data.gov/dataset/baby-names-from-social-security-card-applications-national-data>
- Catalog-declared license: [CC0 1.0](https://creativecommons.org/publicdomain/zero/1.0/).
- Retrieved for this prototype on 2026-11-25.

The archive contains one CSV file per birth year. The source columns are name,
sex, and occurrence count. The fixture retains the exact name field, deduplicates
it across sex categories, sorts by UTF-8 bytes, and places one name on each line.
It does not lowercase, capitalize, normalize, weight, synthesize, or combine names.

The selected file contains 32,020 rows, producing 29,329 unique names and a
207,996-byte LF-delimited fixture. The accompanying `.source.json` records exact
archive, member, and fixture SHA-256 hashes and the processing counts. The source
archive SHA-256 is
`cd78e975ed7bb358e018dd62fbe14ced89295e9581c49172ca4eedcb011b3724`.

This is one US given-name corpus. Its source names are ASCII and the SSA omits
some rare names to avoid disclosing very small counts. Results on it do not
establish performance or compression for Japanese names, international names,
surnames, arbitrary string tables, or much larger datasets. Frequencies in the
source are deliberately discarded because this prototype samples distinct names
uniformly.

Regenerate with the Python standard library:

```bash
python3 tools/fetch_ssa.py
```

The download URL changes as the SSA publishes new data. The script checks the
archive hash and refuses an unexpected source unless
`--allow-updated-source` is supplied deliberately. Updating the source also
requires updating this provenance record and any reported benchmark results.
