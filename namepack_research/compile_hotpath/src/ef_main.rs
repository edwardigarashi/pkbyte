#![forbid(unsafe_code)]
use pkbyte::{uniform_below, Table};
use std::hint::black_box;
use std::time::Instant;

const FILE: &[u8] = include_bytes!("../../../data/given_names_2024.npk");
const SEED: u64 = 0x5a17e6d9b0432c81;
const H: u64 = 0xcbf29ce484222325;
#[derive(Clone, Copy)]
struct Group { first: u64, end: u64, start: usize, width: usize, codec: u8 }
struct Prepared<'a> {
    payload: &'a [u8],
    bytes: [[u8; 8]; 256],
    lengths: [u8; 256],
    groups: Vec<Group>,
    hints: Vec<u8>,
    n: u64,
    max: usize,
}
impl<'a> Prepared<'a> {
    fn from_validated(file: &'a [u8], table: &Table<'_>) -> Self {
        table.validate_all(false).unwrap();
        let dict_bytes = u32::from_le_bytes(file[44..48].try_into().unwrap()) as usize;
        let count = u16::from_le_bytes(file[36..38].try_into().unwrap()) as usize;
        let group_count = u32::from_le_bytes(file[40..44].try_into().unwrap()) as usize;
        assert!(group_count <= 256);
        let mut bytes = [[0; 8]; 256];
        let mut lengths = [0; 256];
        let mut at = 64;
        for i in 0..count {
            let len = file[at] as usize;
            at += 1;
            bytes[i][..len].copy_from_slice(&file[at..at + len]);
            lengths[i] = len as u8;
            at += len;
        }
        assert_eq!(at, 64 + dict_bytes);
        let mut groups = Vec::new();
        let mut first = 0;
        let mut start = 0;
        for _ in 0..group_count {
            let codec = file[at];
            let width = u32::from_le_bytes(file[at + 4..at + 8].try_into().unwrap()) as usize;
            let rows = u64::from_le_bytes(file[at + 8..at + 16].try_into().unwrap());
            groups.push(Group { first, end: first + rows, start, width, codec });
            first += rows;
            start += rows as usize * width;
            at += 16;
        }
        let n = table.len();
        let hints = (0..=((n - 1) >> 5)).map(|bucket| {
            groups.partition_point(|g| g.end <= bucket << 5) as u8
        }).collect();
        Self { payload: &file[at..], bytes, lengths, groups, hints, n, max: table.max_value_bytes() }
    }
    #[inline]
    fn get<'b, const RADIX: bool, const BULK: bool>(&self, rank: u64, out: &'b mut [u8]) -> &'b str {
        assert!(rank < self.n);
        assert!(out.len() >= self.max + 7);
        let id = if RADIX {
            let mut i = self.hints[(rank >> 5) as usize] as usize;
            while self.groups[i].end <= rank { i += 1; }
            i
        } else {
            self.groups.partition_point(|g| g.end <= rank)
        };
        let g = &self.groups[id];
        let start = g.start + (rank - g.first) as usize * g.width;
        let encoded = &self.payload[start..start + g.width];
        let len = if g.codec == 0 {
            out[..encoded.len()].copy_from_slice(encoded);
            encoded.len()
        } else {
            let mut p = 0;
            for &code in encoded {
                let len = self.lengths[code as usize] as usize;
                if BULK { out[p..p + 8].copy_from_slice(&self.bytes[code as usize]); }
                else { out[p..p + len].copy_from_slice(&self.bytes[code as usize][..len]); }
                p += len;
            }
            p
        };
        std::str::from_utf8(&out[..len]).unwrap()
    }

}
#[inline]
fn mix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut x = *state;
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}
#[inline]
fn consume(mut h: u64, bytes: &[u8]) -> u64 {
    h = (h ^ bytes.len() as u64).wrapping_mul(0x100000001b3);
    for &b in bytes { h = (h ^ b as u64).wrapping_mul(0x100000001b3); }
    h
}
fn median(a: &mut [f64]) -> f64 { a.sort_by(f64::total_cmp); a[a.len() / 2] }

mod ef;
fn main() {
    let bytes = black_box(FILE);
    let table = Table::open(bytes).unwrap();
    let prep = Prepared::from_validated(bytes, &table);
    let dir = std::env::args().nth(1).unwrap_or_else(|| "../compression/".to_owned());
    let ef64 = ef::Ef::load(&format!("{dir}ssa2024.ef64"));
    let ef32 = ef::Ef::load(&format!("{dir}ssa2024.ef32"));
    let efb64 = ef::Ef::load(&format!("{dir}ssa2024.efb64"));
    let mut output = Vec::with_capacity(prep.max+7);
    let mut scratch = vec![0; prep.max+7];
    let mut source = std::collections::BTreeMap::new();
    for rank in 0..table.len() {
        source.insert(table.get_into(rank, &mut output).unwrap().as_bytes().to_vec(), rank);
    }
    let mut translation = vec![0; table.len() as usize];
    assert_eq!(ef64.len(), table.len());
    for rank in 0..ef64.len() {
        let original = ef64.get::<false>(rank, &mut scratch).to_owned();
        assert_eq!(original, ef64.get::<true>(rank, &mut scratch));
        assert_eq!(original, ef32.get::<true>(rank, &mut scratch));
        assert_eq!(original, efb64.get::<true>(rank, &mut scratch));
        let old = source.remove(original.as_bytes()).unwrap();
        translation[old as usize] = rank;
    }
    assert!(source.is_empty());
    let iters = 1_000_000usize;
    let rounds = 11usize;
    let mut seed = SEED;
    let ranks: Vec<u64> = (0..iters).map(|_| uniform_below(prep.n, &mut || mix(&mut seed)).unwrap()).collect();
    let ef_ranks: Vec<u64> = ranks.iter().map(|r| translation[*r as usize]).collect();
    let methods = ["original", "prepared_bulk_radix", "ef64_divide", "ef64_reciprocal", "ef32_reciprocal", "efb64_shifts"];
    let mut times = vec![vec![]; methods.len()];
    let mut expected = None;
    for round in 0..=rounds {
        for k in 0..methods.len() {
            let method = (k + round) % methods.len();
            let mut hash = H;
            let timer = Instant::now();
            match method {
                0 => for &r in &ranks { hash = consume(hash, table.get_into(r, &mut output).unwrap().as_bytes()); },
                1 => for &r in &ranks { hash = consume(hash, prep.get::<true, true>(r, &mut scratch).as_bytes()); },
                2 => for &r in &ef_ranks { hash = consume(hash, ef64.get::<false>(r, &mut scratch).as_bytes()); },
                3 => for &r in &ef_ranks { hash = consume(hash, ef64.get::<true>(r, &mut scratch).as_bytes()); },
                4 => for &r in &ef_ranks { hash = consume(hash, ef32.get::<true>(r, &mut scratch).as_bytes()); },
                5 => for &r in &ef_ranks { hash = consume(hash, efb64.get::<true>(r, &mut scratch).as_bytes()); },
                _ => unreachable!(),
            }
            let ns = timer.elapsed().as_secs_f64() * 1e9 / iters as f64;
            black_box(hash);
            if let Some(e) = expected { assert_eq!(hash, e); } else { expected = Some(hash); }
            if round > 0 { times[method].push(ns); }
        }
    }
    println!("{{\"iterations\":{iters},\"rounds\":{rounds},\"rng_timed\":false,\"checksum\":\"{:016x}\",\"file_sizes\":[{},{},{},{}],\"results\":[", expected.unwrap(), FILE.len(), ef64.file_bytes, ef32.file_bytes, efb64.file_bytes);
    for i in 0..methods.len() {
        let median = median(&mut times[i]);
        println!("{{\"method\":\"{}\",\"median_ns\":{median},\"all_ns\":{:?}}}{}", methods[i], times[i], if i+1 == methods.len() { "" } else { "," });
    }
    println!("]}}");
}
