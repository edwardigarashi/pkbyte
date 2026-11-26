use pkbyte::{PrepareOptions, SamplingMethod, Table};

struct Fixture {
    bytes: &'static [u8],
    version: u16,
    codec_rows: [u64; 4],
    ranks: &'static [&'static str],
}

const EF_RANKS: &[&str] = &[
    "",
    "\t",
    "世界\0 frozen fixture /aab",
    "世界\0 frozen fixture /abb",
    "世界\0 frozen fixture /bba",
    "世界\0 frozen fixture /bbc",
    "世界\0 frozen fixture /cac",
    "世界\0 frozen fixture /cca",
    "世界\0 frozen fixture /ccc",
];

const FIXTURES: &[Fixture] = &[
    Fixture {
        bytes: include_bytes!("fixtures/v1-raw.npk"),
        version: 1,
        codec_rows: [5, 0, 0, 0],
        ranks: &[" \t", "Ada", "世", "🦀\0", "line\nend"],
    },
    Fixture {
        bytes: include_bytes!("fixtures/v1-phrase.npk"),
        version: 1,
        codec_rows: [1, 3, 0, 0],
        ranks: &[" \t", "世", "λ", "ABC\0"],
    },
    Fixture {
        bytes: include_bytes!("fixtures/v2-raw.npk"),
        version: 2,
        codec_rows: [6, 0, 0, 0],
        ranks: &["", "\t", "\r\n", "A\0B", "世", "🦀"],
    },
    Fixture {
        bytes: include_bytes!("fixtures/v2-phrase.npk"),
        version: 2,
        codec_rows: [2, 3, 0, 0],
        ranks: &["", " \t", "世", "λ", "ABC\0"],
    },
    Fixture {
        bytes: include_bytes!("fixtures/v3-mixed.npk"),
        version: 3,
        codec_rows: [2, 0, 7, 0],
        ranks: EF_RANKS,
    },
    Fixture {
        bytes: include_bytes!("fixtures/v3-power-of-two.npk"),
        version: 3,
        codec_rows: [2, 0, 0, 7],
        ranks: EF_RANKS,
    },
];

#[test]
fn frozen_wire_formats_keep_exact_values_and_ranks() {
    for fixture in FIXTURES {
        let context = format!(
            "version {}, codec counts {:?}",
            fixture.version, fixture.codec_rows
        );
        let table = Table::open(fixture.bytes).unwrap_or_else(|error| panic!("{context}: {error}"));
        assert_eq!(table.version(), fixture.version, "{context}");
        let report = table.validate_all(true).unwrap();
        assert_eq!(report.row_count, fixture.ranks.len() as u64, "{context}");
        assert_eq!(
            report.decoded_bytes,
            fixture
                .ranks
                .iter()
                .map(|value| value.len() as u64)
                .sum::<u64>(),
            "{context}"
        );
        let stats = table.stats();
        assert_eq!(
            [
                stats.raw_rows,
                stats.compressed_rows,
                stats.ef_mixed_rows,
                stats.ef_power_of_two_rows
            ],
            fixture.codec_rows,
            "{context}"
        );
        let structural = Table::open_structural(fixture.bytes).unwrap();
        let prepared = structural
            .prepare(&PrepareOptions {
                sampling: SamplingMethod::Modulo,
                ..PrepareOptions::default()
            })
            .unwrap();
        let mut checked_output = Vec::with_capacity(table.max_value_bytes());
        let mut prepared_output = prepared.new_output_buffer().unwrap();
        for (index, &expected) in fixture.ranks.iter().enumerate() {
            let rank = index as u64;
            assert_eq!(
                table.get_into(rank, &mut checked_output).unwrap(),
                expected,
                "{context}, rank {rank}"
            );
            assert_eq!(
                table.get_raw(rank).unwrap(),
                (rank < fixture.codec_rows[0]).then_some(expected),
                "{context}, rank {rank}"
            );
            assert_eq!(
                prepared.get_into(rank, &mut prepared_output).unwrap(),
                expected,
                "{context}, rank {rank}"
            );
            // The draw is accepted and has residue rank for the modulo reducer.
            let mut draw = || table.len() + rank;
            assert_eq!(
                prepared
                    .sample_into(&mut draw, &mut prepared_output)
                    .unwrap(),
                expected,
                "{context}, sample rank {rank}"
            );
        }
    }
}
